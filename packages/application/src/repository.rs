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
    #[serde(default = "default_conversation_browser_total_limit")]
    pub(crate) conversation_browser_total_limit: u32,
    #[serde(default)]
    pub(crate) outline_presentation: crate::config::OutlinePresentation,
    #[serde(default)]
    pub(crate) conversation_browser_preferences: crate::config::ConversationBrowserPreferences,
    #[serde(default = "crate::config::default_status_line")]
    pub(crate) status_line: Vec<crate::config::StatusLineItem>,
}

pub(crate) fn default_conversation_browser_group_limit() -> u32 {
    20
}
pub(crate) fn default_conversation_browser_total_limit() -> u32 {
    200
}

impl Default for PersistedConfig {
    fn default() -> Self {
        Self {
            schema_version: 0,
            gateways: Vec::new(),
            runtime_instances: Vec::new(),
            model_templates: Vec::new(),
            conversation_browser_group_limit: default_conversation_browser_group_limit(),
            conversation_browser_total_limit: default_conversation_browser_total_limit(),
            outline_presentation: Default::default(),
            conversation_browser_preferences: Default::default(),
            status_line: crate::config::default_status_line(),
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
                if version < 8 {
                    reset = true;
                    PersistedConfig {
                        schema_version: 8,
                        ..PersistedConfig::default()
                    }
                } else {
                    if version > 8 {
                        return Err(RuntimeError::invalid(
                            "配置版本高于本应用支持的 schema 8；请使用支持该版本的应用。",
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
                schema_version: 8,
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
        commit(
            &self.home.join("generic-config.json"),
            &serde_json::to_vec_pretty(config)?,
        )
    }

    /// Composer drafts live beside the configuration, not in it: they change
    /// while typing and survive an ordinary configuration reset.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn load_composer_drafts(
        &self,
    ) -> Result<std::collections::BTreeMap<String, String>, RuntimeError> {
        let bytes = match fs::read(self.home.join("composer-drafts.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Default::default());
            }
            Err(error) => return Err(error.into()),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value["schemaVersion"].as_u64() != Some(1) {
            return Err(RuntimeError::invalid("composer drafts schema version"));
        }
        let drafts: Vec<crate::config::ComposerDraft> =
            serde_json::from_value(value["drafts"].clone())?;
        let mut result = std::collections::BTreeMap::new();
        for draft in drafts {
            if draft.conversation_id.is_empty()
                || draft.text.is_empty()
                || result.insert(draft.conversation_id, draft.text).is_some()
            {
                return Err(RuntimeError::invalid("composer draft identity"));
            }
        }
        Ok(result)
    }

    #[cfg(feature = "local-runtime")]
    pub(crate) fn store_composer_drafts(
        &self,
        drafts: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), RuntimeError> {
        let drafts: Vec<_> = drafts
            .iter()
            .map(|(id, text)| crate::config::ComposerDraft {
                conversation_id: id.clone(),
                text: text.clone(),
            })
            .collect();
        let value = serde_json::json!({"schemaVersion":1,"drafts":drafts});
        commit(
            &self.home.join("composer-drafts.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
    }

    /// Reusable resource templates (with their secrets) and the managed bindings
    /// to runtimes are application files of their own, not part of the configuration.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn load_resource_library(
        &self,
    ) -> Result<(Vec<crate::ResourceTemplate>, Vec<crate::ResourceBinding>), RuntimeError> {
        let read = |name: &str, field: &str| -> Result<serde_json::Value, RuntimeError> {
            let bytes = match fs::read(self.home.join(name)) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(serde_json::Value::Array(Vec::new()));
                }
                Err(error) => return Err(error.into()),
            };
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            if value["schemaVersion"].as_u64() != Some(1) {
                return Err(RuntimeError::invalid("resource library schema version"));
            }
            Ok(value[field].clone())
        };
        let templates: Vec<crate::ResourceTemplate> =
            serde_json::from_value(read("resource-templates.json", "templates")?)?;
        let bindings: Vec<crate::ResourceBinding> =
            serde_json::from_value(read("resource-bindings.json", "bindings")?)?;
        let mut ids = std::collections::BTreeSet::new();
        if templates
            .iter()
            .any(|template| template.id.is_empty() || !ids.insert(template.id.as_str()))
        {
            return Err(RuntimeError::invalid("resource template identity"));
        }
        if bindings
            .iter()
            .any(|binding| !ids.contains(binding.template_id.as_str()))
        {
            return Err(RuntimeError::invalid("resource binding template"));
        }
        Ok((templates, bindings))
    }

    #[cfg(feature = "local-runtime")]
    pub(crate) fn store_resource_templates(
        &self,
        templates: &[crate::ResourceTemplate],
    ) -> Result<(), RuntimeError> {
        let value = serde_json::json!({"schemaVersion":1,"templates":templates});
        commit(
            &self.home.join("resource-templates.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
    }

    #[cfg(feature = "local-runtime")]
    pub(crate) fn store_resource_bindings(
        &self,
        bindings: &[crate::ResourceBinding],
    ) -> Result<(), RuntimeError> {
        let value = serde_json::json!({"schemaVersion":1,"bindings":bindings});
        commit(
            &self.home.join("resource-bindings.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
    }

    /// Velune's list organization; a missing file starts read state now.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn load_conversation_organization(
        &self,
    ) -> Result<crate::config::ConversationOrganization, RuntimeError> {
        let bytes = match fs::read(self.home.join("conversation-organization.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(crate::config::ConversationOrganization {
                    read_baseline_unix_ms: crate::local::unix_ms_now(),
                    ..Default::default()
                });
            }
            Err(error) => return Err(error.into()),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value["schemaVersion"].as_u64() != Some(1) {
            return Err(RuntimeError::invalid(
                "conversation organization schema version",
            ));
        }
        let organization: crate::config::ConversationOrganization =
            serde_json::from_value(value["organization"].clone())?;
        let mut ids = std::collections::BTreeSet::new();
        let mut paths = std::collections::BTreeSet::new();
        if organization.conversations.iter().any(|marks| {
            marks.conversation_id.is_empty()
                || marks.is_empty()
                || !ids.insert(&marks.conversation_id)
        }) || organization.project_aliases.iter().any(|alias| {
            alias.path.is_empty() || alias.alias.trim().is_empty() || !paths.insert(&alias.path)
        }) {
            return Err(RuntimeError::invalid("会话组织记录的会话或项目身份无效"));
        }
        Ok(organization)
    }

    #[cfg(feature = "local-runtime")]
    pub(crate) fn store_conversation_organization(
        &self,
        organization: &crate::config::ConversationOrganization,
    ) -> Result<(), RuntimeError> {
        let value = serde_json::json!({"schemaVersion":1,"organization":organization});
        commit(
            &self.home.join("conversation-organization.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
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
            // Replacing an unsubmitted draft preserves its public ID as an alias
            // for the first real target, including after later handoffs.
            if link.id.is_empty()
                || !ids.insert(&link.id)
                || link.segments.is_empty()
                || link.segments.first().is_none_or(|s| {
                    s.handoff.is_some()
                        || (link.segments.len() == 1 && s.native_conversation_id == link.id)
                })
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
        let value = serde_json::json!({"schemaVersion":1,"links":links});
        commit(
            &self.home.join("conversation-links.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
    }
}

/// Private (0600) atomic replacement of one application file.
pub(crate) fn commit(path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
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
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}
