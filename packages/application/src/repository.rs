//! Application-owned configuration file locking and atomic commits.
use crate::{
    Error as RuntimeError,
    config::{GatewayConfig, RuntimeInstance},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedConfig {
    pub(crate) schema_version: u32,
    #[serde(default)]
    pub(crate) gateways: Vec<GatewayConfig>,
    #[serde(default)]
    pub(crate) runtime_instances: Vec<RuntimeInstance>,
    #[serde(default)]
    pub(crate) authentication_bindings:
        Vec<crate::authentication_resources::AuthenticationResource>,
}

pub(crate) struct Repository {
    home: PathBuf,
    home_lock: File,
}
impl Repository {
    pub(crate) fn open(home: &Path) -> Result<(Self, PersistedConfig), RuntimeError> {
        let lock_path = home.join("runtime.lock");
        let home_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            home_lock.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        match home_lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(RuntimeError::invalid("runtime home is already open"));
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let persisted_path = home.join("generic-config.json");
        let mut migrated = false;
        let persisted = match fs::read(&persisted_path) {
            Ok(bytes) => {
                let mut raw: serde_json::Value = serde_json::from_slice(&bytes)?;
                if raw["schemaVersion"] == 2 {
                    migrate_schema_two(&mut raw)?;
                    migrated = true;
                }
                let value: PersistedConfig = serde_json::from_value(raw)?;
                if value.schema_version != 3 {
                    return Err(RuntimeError::invalid("unsupported configuration schema"));
                }
                crate::authentication_resources::AuthenticationManager {
                    resources: value.authentication_bindings.clone(),
                }
                .validate(&value.gateways)?;
                for gateway in &value.gateways {
                    gateway.validate().map_err(RuntimeError::invalid)?;
                }
                for runtime in &value.runtime_instances {
                    if runtime.id.is_empty()
                        || runtime.name.is_empty()
                        || runtime.type_id.is_empty()
                        || !value
                            .gateways
                            .iter()
                            .any(|item| item.id == runtime.gateway_id)
                    {
                        return Err(RuntimeError::invalid("invalid persisted runtime instance"));
                    }
                }
                value
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PersistedConfig {
                schema_version: 3,
                ..PersistedConfig::default()
            },
            Err(error) => return Err(error.into()),
        };
        let repository = Self {
            home: home.to_owned(),
            home_lock,
        };
        if migrated {
            repository.store(&persisted)?;
        }
        Ok((repository, persisted))
    }
    pub(crate) fn unlock(&self) -> Result<(), RuntimeError> {
        self.home_lock.unlock()?;
        Ok(())
    }
    pub(crate) fn store(&self, config: &PersistedConfig) -> Result<(), RuntimeError> {
        crate::authentication_resources::AuthenticationManager {
            resources: config.authentication_bindings.clone(),
        }
        .validate(&config.gateways)?;
        let path = self.home.join("generic-config.json");
        let bytes = serde_json::to_vec_pretty(config)?;
        let temporary = path.with_extension("json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, &path)?;
        Ok(())
    }
}

// Migration only transforms ordinary configuration. It never opens a source
// directory, credential store, session, or secret reference.
fn migrate_schema_two(raw: &mut serde_json::Value) -> Result<(), RuntimeError> {
    use crate::authentication_resources::*;
    let mut resources = Vec::new();
    let gateways = raw["gateways"]
        .as_array_mut()
        .ok_or_else(|| RuntimeError::invalid("invalid legacy gateways"))?;
    for (gateway_index, gateway) in gateways.iter_mut().enumerate() {
        let providers = gateway["providers"]
            .as_array_mut()
            .ok_or_else(|| RuntimeError::invalid("invalid legacy providers"))?;
        for (provider_index, provider) in providers.iter_mut().enumerate() {
            let object = provider
                .as_object_mut()
                .ok_or_else(|| RuntimeError::invalid("invalid legacy provider"))?;
            let source = object
                .remove("credentialSource")
                .filter(|value| !value.is_null());
            let reference = object
                .remove("credentialRef")
                .filter(|value| !value.is_null());
            let generation = object
                .remove("credentialGeneration")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            if source.is_some() && reference.is_some() {
                return Err(RuntimeError::invalid(
                    "legacy provider has conflicting authentication",
                ));
            }
            let locator = if let Some(source) = source {
                AuthenticationLocator::RuntimeProvider {
                    source: serde_json::from_value(source)?,
                }
            } else if let Some(reference) = reference {
                AuthenticationLocator::Keychain {
                    reference: reference
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            RuntimeError::invalid("invalid legacy credential reference")
                        })?
                        .into(),
                    owns_secret: false,
                }
            } else {
                object.insert("authenticationId".into(), serde_json::Value::Null);
                continue;
            };
            let (method, configured) = match &locator {
                AuthenticationLocator::Keychain { .. } => (AuthenticationMethod::ApiKey, true),
                AuthenticationLocator::RuntimeProvider { source } => {
                    match source.settings.get("credentialKind").map(String::as_str) {
                        Some("oauth") => (AuthenticationMethod::OAuth, true),
                        Some("literal_api_key" | "stored_environment") => {
                            (AuthenticationMethod::ApiKey, true)
                        }
                        _ => (AuthenticationMethod::Unconfigured, false),
                    }
                }
            };
            let id = format!("legacy_auth_{gateway_index}_{provider_index}");
            let resource =
                AuthenticationResource {
                    id: id.clone(),
                    name: object
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("认证资源")
                        .into(),
                    method,
                    configured,
                    protocol: serde_json::from_value(object.get("protocol").cloned().ok_or_else(
                        || RuntimeError::invalid("invalid legacy provider protocol"),
                    )?)?,
                    endpoint: object
                        .get("endpoint")
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| RuntimeError::invalid("invalid legacy provider endpoint"))?
                        .into(),
                    generation,
                    locator,
                };
            resources.push(resource);
            object.insert("authenticationId".into(), serde_json::Value::String(id));
        }
    }
    raw["schemaVersion"] = serde_json::json!(3);
    raw["authenticationBindings"] = serde_json::to_value(resources)?;
    Ok(())
}
