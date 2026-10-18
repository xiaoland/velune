//! Private network configuration is atomic and separate from the safe pending/state file.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayCredential {
    pub url: String,
    pub bearer_token: String,
}
impl std::fmt::Debug for RelayCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RelayCredential([REDACTED])")
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub enum CredentialEdit {
    Keep,
    Set { value: String },
    Remove,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct RelayCredentialEdit {
    pub url: String,
    pub credential: CredentialEdit,
}

#[derive(Clone)]
pub struct NetworkSettingsUpdate {
    pub expected_revision: u64,
    pub config: EndpointConfig,
    /// This complete list replaces the credential table. Omitted destinations lose qualification.
    pub credentials: Vec<RelayCredentialEdit>,
    pub restore_lookup_urls: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct RelayCredentialSummary {
    pub url: String,
    pub configured: bool,
}
#[derive(Clone, Serialize)]
pub struct NetworkSettingsSummary {
    pub revision: u64,
    pub relay: RelayConfig,
    pub discovery: DiscoveryConfig,
    pub local_discovery: bool,
    pub path_policy: PathPolicy,
    pub bind_addresses: Vec<std::net::SocketAddr>,
    pub request_timeout_ms: u64,
    pub credentials: Vec<RelayCredentialSummary>,
    pub withdrawn_lookup_urls: Vec<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkApplicationPhase {
    Unapplied,
    Applying,
    Applied,
    Failed,
}
#[derive(Clone, Serialize)]
pub struct NetworkState {
    pub saved: Option<NetworkSettingsSummary>,
    pub phase: NetworkApplicationPhase,
    pub applied_revision: Option<u64>,
    pub generation: Option<u64>,
    pub error_code: Option<String>,
    pub facilities: Vec<FacilityStatus>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FacilityKind {
    HomeRelay,
    Lookup,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FacilityReachability {
    Unknown,
    Connecting,
    Reachable,
    Unavailable,
    AuthenticationDenied,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct FacilityStatus {
    pub kind: FacilityKind,
    pub url: String,
    pub reachability: FacilityReachability,
}
#[derive(Clone, Copy)]
pub(crate) struct NetworkApplication {
    pub phase: NetworkApplicationPhase,
    pub revision: Option<u64>,
    pub generation: Option<u64>,
    pub error: Option<RemoteError>,
}
impl Default for NetworkApplication {
    fn default() -> Self {
        Self {
            phase: NetworkApplicationPhase::Unapplied,
            revision: None,
            generation: None,
            error: None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredNetworkSettings {
    version: u16,
    pub revision: u64,
    pub config: EndpointConfig,
    pub withdrawn_lookup_urls: std::collections::BTreeSet<String>,
}
pub(crate) struct SettingsStore {
    path: Option<PathBuf>,
    value: Mutex<Option<StoredNetworkSettings>>,
}
impl SettingsStore {
    pub(crate) fn memory() -> Self {
        Self {
            path: None,
            value: Mutex::new(None),
        }
    }
    pub(crate) fn open(state_path: &Path) -> Result<Self> {
        let path = private_settings_path(state_path)?;
        let value = match crate::identity::read_private(&path) {
            Ok(bytes) => {
                let stored: StoredNetworkSettings =
                    serde_json::from_slice(&bytes).map_err(|_| RemoteError::StateStorage)?;
                if stored.version != 1 || stored.revision == 0 {
                    return Err(RemoteError::StateStorage);
                }
                crate::endpoint::validate_config(&stored.config)?;
                if stored.withdrawn_lookup_urls.len() > 1024 {
                    return Err(RemoteError::StateStorage);
                }
                for url in &stored.withdrawn_lookup_urls {
                    crate::endpoint::validate_relay_url(url)?;
                }
                Some(stored)
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(&path)
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                None
            }
            Err(_) => return Err(RemoteError::StateStorage),
        };
        Ok(Self {
            path: Some(path),
            value: Mutex::new(value),
        })
    }
    pub(crate) fn get(&self) -> Result<Option<StoredNetworkSettings>> {
        Ok(self
            .value
            .lock()
            .map_err(|_| RemoteError::StateStorage)?
            .clone())
    }
    pub(crate) fn save(
        &self,
        config: EndpointConfig,
        expected: Option<u64>,
        restore: &[String],
    ) -> Result<StoredNetworkSettings> {
        let mut current = self.value.lock().map_err(|_| RemoteError::StateStorage)?;
        let revision = current.as_ref().map_or(0, |settings| settings.revision);
        if expected.is_some_and(|expected| expected != revision) {
            return Err(RemoteError::ConfigurationConflict);
        }
        let mut withdrawn = current
            .as_ref()
            .map(|settings| settings.withdrawn_lookup_urls.clone())
            .unwrap_or_default();
        let next_urls = configured_lookup_urls(&config.discovery);
        if let Some(previous) = current.as_ref() {
            for url in configured_lookup_urls(&previous.config.discovery) {
                if !next_urls.contains(&url) {
                    withdrawn.insert(url);
                }
            }
        }
        for url in next_urls.iter().chain(restore.iter()) {
            withdrawn.remove(url);
        }
        if withdrawn.len() > 1024 {
            return Err(RemoteError::Capacity);
        }
        if let Some(current) = current.as_ref()
            && current.config == config
            && current.withdrawn_lookup_urls == withdrawn
        {
            return Ok(current.clone());
        }
        let settings = StoredNetworkSettings {
            version: 1,
            revision: revision.checked_add(1).ok_or(RemoteError::Capacity)?,
            config,
            withdrawn_lookup_urls: withdrawn,
        };
        if let Some(path) = &self.path {
            let bytes = serde_json::to_vec(&settings).map_err(|_| RemoteError::StateStorage)?;
            if bytes.len() > 1024 * 1024 {
                return Err(RemoteError::Capacity);
            }
            crate::identity::atomic_write(path, &bytes).map_err(|_| RemoteError::StateStorage)?;
        }
        *current = Some(settings.clone());
        Ok(settings)
    }
    pub(crate) fn prepare(
        &self,
        update: NetworkSettingsUpdate,
    ) -> Result<(EndpointConfig, u64, Vec<String>)> {
        let current = self.get()?;
        let revision = current.as_ref().map_or(0, |settings| settings.revision);
        if revision != update.expected_revision {
            return Err(RemoteError::ConfigurationConflict);
        }
        let mut config = update.config;
        if !config.relay_credentials.is_empty() {
            return Err(RemoteError::Configuration);
        }
        config.relay_credentials.clear();
        let mut seen = std::collections::BTreeSet::new();
        for entry in update.credentials {
            let url = canonical_relay_url(&entry.url)?.to_string();
            if !seen.insert(url.clone()) {
                return Err(RemoteError::Configuration);
            }
            let token = match entry.credential {
                CredentialEdit::Keep => current
                    .as_ref()
                    .and_then(|settings| {
                        settings
                            .config
                            .relay_credentials
                            .iter()
                            .find(|credential| credential.url == url)
                    })
                    .map(|credential| credential.bearer_token.clone())
                    .ok_or(RemoteError::Configuration)?,
                CredentialEdit::Set { value } => value,
                CredentialEdit::Remove => continue,
            };
            config.relay_credentials.push(RelayCredential {
                url,
                bearer_token: token,
            });
        }
        crate::endpoint::validate_config(&config)?;
        if update.restore_lookup_urls.len() > 1024 {
            return Err(RemoteError::Capacity);
        }
        let restore = update
            .restore_lookup_urls
            .iter()
            .map(|url| crate::endpoint::validate_relay_url(url).map(|url| url.to_string()))
            .collect::<Result<_>>()?;
        Ok((config, revision, restore))
    }
}
pub(crate) fn private_settings_path(state_path: &Path) -> Result<PathBuf> {
    let name = state_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(RemoteError::StateStorage)?;
    Ok(state_path.with_file_name(format!("{name}.network-private.json")))
}
impl StoredNetworkSettings {
    pub(crate) fn summary(&self) -> NetworkSettingsSummary {
        NetworkSettingsSummary {
            revision: self.revision,
            withdrawn_lookup_urls: self.withdrawn_lookup_urls.iter().cloned().collect(),
            relay: self.config.relay.clone(),
            discovery: self.config.discovery.clone(),
            local_discovery: self.config.local_discovery,
            path_policy: self.config.path_policy,
            bind_addresses: self.config.bind_addresses.clone(),
            request_timeout_ms: self.config.request_timeout.as_millis() as u64,
            credentials: self
                .config
                .relay_credentials
                .iter()
                .map(|credential| RelayCredentialSummary {
                    url: credential.url.clone(),
                    configured: true,
                })
                .collect(),
        }
    }
}
pub(crate) fn canonical_relay_url(value: &str) -> Result<iroh::RelayUrl> {
    let url = crate::endpoint::validate_relay_url(value)?;
    if url.path() != "/" {
        return Err(RemoteError::Configuration);
    }
    Ok(url)
}
pub(crate) fn normalize_config(mut config: EndpointConfig) -> Result<EndpointConfig> {
    crate::endpoint::validate_config(&config)?;
    if let RelayConfig::Custom { urls } = &mut config.relay {
        *urls = urls
            .iter()
            .map(|url| canonical_relay_url(url).map(|url| url.to_string()))
            .collect::<Result<_>>()?;
    }
    if let DiscoveryConfig::CustomPkarr { urls } = &mut config.discovery {
        *urls = urls
            .iter()
            .map(|url| crate::endpoint::validate_relay_url(url).map(|url| url.to_string()))
            .collect::<Result<_>>()?;
    }
    for credential in &mut config.relay_credentials {
        credential.url = canonical_relay_url(&credential.url)?.to_string();
    }
    Ok(config)
}
impl RemoteAccessService {
    pub fn network_state(&self) -> Result<NetworkState> {
        let saved = self
            .state
            .store
            .network_settings
            .get()?
            .map(|settings| settings.summary());
        let application = *self
            .state
            .network_application
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        Ok(NetworkState {
            saved,
            phase: application.phase,
            applied_revision: application.revision,
            generation: application.generation,
            error_code: application.error.map(|error| error.code().into()),
            facilities: self
                .network()
                .ok()
                .map(|network| network.facility_status())
                .transpose()?
                .unwrap_or_default(),
        })
    }
    pub async fn apply_network(&self, update: NetworkSettingsUpdate) -> Result<NetworkState> {
        let (config, revision, restore) = self.state.store.network_settings.prepare(update)?;
        self.replace_network_expected(config, Some(revision), restore)
            .await
    }
    pub async fn retry_network(&self) -> Result<NetworkState> {
        let saved = self
            .state
            .store
            .network_settings
            .get()?
            .ok_or(RemoteError::Configuration)?;
        self.replace_network_expected(saved.config, Some(saved.revision), Vec::new())
            .await
    }
}

pub(crate) fn configured_lookup_urls(discovery: &DiscoveryConfig) -> Vec<String> {
    match discovery {
        DiscoveryConfig::None => Vec::new(),
        DiscoveryConfig::CustomPkarr { urls } => urls.clone(),
        DiscoveryConfig::DevelopmentPublic => vec![
            iroh::address_lookup::pkarr::N0_DNS_PKARR_RELAY_PROD.into(),
            iroh::address_lookup::pkarr::N0_DNS_PKARR_RELAY_STAGING.into(),
        ],
    }
}
