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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedConfig {
    pub(crate) schema_version: u32,
    pub(crate) gateways: Vec<GatewayConfig>,
    pub(crate) runtime_instances: Vec<RuntimeInstance>,
    pub(crate) model_templates: Vec<crate::config::ModelTemplate>,
    #[serde(default = "default_conversation_browser_group_limit")]
    pub(crate) conversation_browser_group_limit: u32,
}

pub(crate) fn default_conversation_browser_group_limit() -> u32 {
    20
}

impl Default for PersistedConfig {
    fn default() -> Self {
        Self {
            schema_version: 0,
            gateways: Vec::new(),
            runtime_instances: Vec::new(),
            model_templates: Vec::new(),
            conversation_browser_group_limit: default_conversation_browser_group_limit(),
        }
    }
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
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&persisted_path, fs::Permissions::from_mode(0o600))?;
                }
                let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
                let version = raw["schemaVersion"].as_u64().ok_or_else(|| {
                    RuntimeError::invalid("configuration schema version is required")
                })?;
                if version < 7 {
                    reset = true;
                    PersistedConfig {
                        schema_version: 7,
                        ..PersistedConfig::default()
                    }
                } else {
                    if version > 7 {
                        return Err(RuntimeError::invalid(
                            "配置版本高于本应用支持的 schema 7；请使用支持该版本的应用。",
                        ));
                    }
                    let value: PersistedConfig = serde_json::from_value(raw)?;
                    if value.conversation_browser_group_limit == 0 {
                        return Err(RuntimeError::invalid("每组会话加载数量必须大于零"));
                    }
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
                    let mut template_ids = std::collections::BTreeSet::new();
                    for template in &value.model_templates {
                        if template.id.is_empty()
                            || template.name.trim().is_empty()
                            || !template_ids.insert(&template.id)
                        {
                            return Err(RuntimeError::invalid("invalid model template identity"));
                        }
                        crate::provider_configuration::validate_template(template)?;
                    }
                    value
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PersistedConfig {
                schema_version: 7,
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
        let path = self.home.join("generic-config.json");
        let bytes = serde_json::to_vec_pretty(config)?;
        let temporary = path.with_extension("json.tmp");
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
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

    #[cfg(feature = "local-runtime")]
    pub(crate) fn load_conversation_links(
        &self,
    ) -> Result<Vec<crate::config::ConversationLink>, RuntimeError> {
        let path = self.home.join("conversation-links.json");
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value["schemaVersion"].as_u64() != Some(1) {
            return Err(RuntimeError::invalid("conversation links schema version"));
        }
        let links: Vec<crate::config::ConversationLink> =
            serde_json::from_value(value["links"].clone())?;
        let mut ids = std::collections::BTreeSet::new();
        let mut native_ids = std::collections::BTreeSet::new();
        let valid_digest =
            |digest: &str| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
        for link in &links {
            if link.id.is_empty()
                || !ids.insert(&link.id)
                || link.segments.len() < 2
                || link
                    .segments
                    .first()
                    .is_none_or(|s| s.native_conversation_id != link.id)
            {
                return Err(RuntimeError::invalid("关联会话身份或段顺序无效"));
            }
            for (index, segment) in link.segments.iter().enumerate() {
                if segment.runtime_instance_id.is_empty()
                    || segment.runtime_type_id.is_empty()
                    || !segment
                        .native_conversation_id
                        .starts_with(&format!("{}:", segment.runtime_instance_id))
                    || !native_ids.insert(&segment.native_conversation_id)
                    || (index + 1 == link.segments.len()) != segment.cutoff.is_none()
                    || segment
                        .cutoff
                        .as_ref()
                        .is_some_and(|c| !valid_digest(&c.prefix_digest))
                    || segment.handoff.as_ref().is_some_and(|h| {
                        !valid_digest(&h.marker) || !valid_digest(&h.payload_digest)
                    })
                {
                    return Err(RuntimeError::invalid(
                        "关联会话原生引用、截止位置或交接定位无效",
                    ));
                }
            }
        }
        Ok(links)
    }

    #[cfg(feature = "local-runtime")]
    pub(crate) fn store_conversation_links(
        &self,
        links: &[crate::config::ConversationLink],
    ) -> Result<(), RuntimeError> {
        let path = self.home.join("conversation-links.json");
        let temporary = path.with_extension("json.tmp");
        let value = serde_json::json!({"schemaVersion":1,"links":links});
        let bytes = serde_json::to_vec_pretty(&value)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, path)?;
        Ok(())
    }
}
