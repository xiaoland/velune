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
    pub(crate) gateways: Vec<GatewayConfig>,
    pub(crate) runtime_instances: Vec<RuntimeInstance>,
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

        let mut reset = false;
        let persisted = match fs::read(&persisted_path) {
            Ok(bytes) => {
                let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
                let version = raw["schemaVersion"].as_u64().ok_or_else(|| {
                    RuntimeError::invalid("configuration schema version is required")
                })?;
                if version < 4 {
                    reset = true;
                    PersistedConfig {
                        schema_version: 4,
                        ..PersistedConfig::default()
                    }
                } else {
                    if version > 4 {
                        return Err(RuntimeError::invalid(
                            "配置版本高于本应用支持的 schema 4；请使用支持该版本的应用。",
                        ));
                    }
                    let value: PersistedConfig = serde_json::from_value(raw)?;
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
                            return Err(RuntimeError::invalid(
                                "invalid persisted runtime instance",
                            ));
                        }
                    }
                    value
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PersistedConfig {
                schema_version: 4,
                ..PersistedConfig::default()
            },
            Err(error) => return Err(error.into()),
        };
        let repository = Self {
            home: home.to_owned(),
            home_lock,
        };
        if reset {
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
