//! One atomic ordinary-state file. Identity and business data have independent lifetimes.
use crate::{
    identity::{atomic_write, random_id, read_private, validate_name},
    *,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};
use velune_application_api::{CommandKind, ExecutionContext};

const MAX_CONTACTS: usize = 256;
const MAX_PENDING: usize = 256;
const MAX_STATE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct DeviceLocator {
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<std::net::SocketAddr>,
    pub lookup_urls: Vec<String>,
}
impl DeviceLocator {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.relay_urls.len() > 16
            || self.direct_addresses.len() > 32
            || self.lookup_urls.len() > 4
        {
            return Err(RemoteError::Configuration);
        }
        for url in self.relay_urls.iter().chain(self.lookup_urls.iter()) {
            crate::endpoint::validate_relay_url(url)?;
        }
        Ok(())
    }
    pub(crate) fn address(&self, peer: &str) -> Result<iroh::EndpointAddr> {
        self.validate()?;
        let mut address = iroh::EndpointAddr::new(parse_peer(peer)?);
        for url in &self.relay_urls {
            address = address.with_relay_url(url.parse().map_err(|_| RemoteError::Configuration)?);
        }
        for ip in &self.direct_addresses {
            address = address.with_ip_addr(*ip);
        }
        Ok(address)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ContactConfirmation {
    Pending { attempt_id: String },
    Confirmed { attempt_id: String },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum LocatorSource {
    PairingOffer,
    AuthenticatedPeer,
    ExplicitImport,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct LocatorUpdate {
    pub peer_id: String,
    pub locator: DeviceLocator,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct DeviceContact {
    pub peer_id: String,
    pub name: String,
    pub locator: DeviceLocator,
    pub confirmation: ContactConfirmation,
    pub paused: bool,
    pub locator_source: LocatorSource,
    pub control_authorized: bool,
    pub pairing_permission: PairingPermission,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct IncomingControlGrant {
    pub peer_id: String,
    pub name: String,
    pub attempt_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum PendingStatus {
    Recorded,
    Running,
    Completed,
    BusinessFailed,
    TransportFailed,
    Unknown,
    ResultExpired,
    ServerRestarted,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct PendingOperation {
    pub peer_id: String,
    pub request_id: RequestId,
    /// The originating `ClientViewId`; a plain string because uniffi-bindgen-cs
    /// does not alias another crate's custom types.
    pub view: String,
    pub execution: ExecutionContext,
    pub kind: CommandKind,
    pub runtime_id: Option<String>,
    pub conversation_id: Option<String>,
    pub status: PendingStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationResetNotice {
    pub id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredState {
    version: u16,
    contacts: BTreeMap<String, DeviceContact>,
    grants: BTreeMap<String, IncomingControlGrant>,
    profile_grants: BTreeMap<String, IncomingControlGrant>,
    pending_pairings: BTreeMap<String, PendingPairing>,
    pending: Vec<PendingOperation>,
    reset_notice: Option<ConfigurationResetNotice>,
    legacy_reset_recorded: bool,
}
impl Default for StoredState {
    fn default() -> Self {
        Self {
            version: 4,
            contacts: BTreeMap::new(),
            grants: BTreeMap::new(),
            profile_grants: BTreeMap::new(),
            pending_pairings: BTreeMap::new(),
            pending: Vec::new(),
            reset_notice: None,
            legacy_reset_recorded: false,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingPairing {
    pub attempt_id: String,
    pub permission: PairingPermission,
}
#[derive(Clone)]
pub struct DeviceStoreSnapshot {
    pub contacts: Vec<DeviceContact>,
    pub incoming_grants: Vec<IncomingControlGrant>,
    pub telemetry_profile_grants: Vec<IncomingControlGrant>,
    pub pending: Vec<PendingOperation>,
    pub reset_notice: Option<ConfigurationResetNotice>,
}
/// A service shares one store instance. It never stores command bodies or completed response payloads.
pub struct DeviceStore {
    pub(crate) network_settings: crate::settings::SettingsStore,
    path: Option<PathBuf>,
    state: Mutex<StoredState>,
    changed: tokio::sync::watch::Sender<u64>,
}
impl DeviceStore {
    pub fn memory() -> Self {
        Self {
            network_settings: crate::settings::SettingsStore::memory(),
            path: None,
            state: Mutex::new(StoredState::default()),
            changed: tokio::sync::watch::channel(1).0,
        }
    }
    pub fn open(path: &Path) -> Result<Self> {
        let (state, write) = match read_private(path) {
            Ok(bytes) => {
                let mut json: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|_| RemoteError::StateStorage)?;
                // An independent identity is never an obsolete ordinary-state schema.
                if json.get("secret_key").is_some() {
                    return Err(RemoteError::StateStorage);
                }
                let version = json.get("version").and_then(serde_json::Value::as_u64);
                if version.is_some_and(|v| v > 4) {
                    return Err(RemoteError::StateStorage);
                }
                if version == Some(3) {
                    // v3 control grants remain control-only; never infer consent to share credentials.
                    json["version"] = serde_json::json!(4);
                    json["profile_grants"] = serde_json::json!({});
                    json["pending_pairings"] = serde_json::json!({});
                    if let Some(contacts) = json.get_mut("contacts").and_then(|v| v.as_object_mut())
                    {
                        for contact in contacts.values_mut() {
                            let contact =
                                contact.as_object_mut().ok_or(RemoteError::StateStorage)?;
                            contact.insert("control_authorized".into(), serde_json::json!(true));
                            contact
                                .insert("pairing_permission".into(), serde_json::json!("control"));
                        }
                    }
                    (
                        serde_json::from_value(json).map_err(|_| RemoteError::StateStorage)?,
                        true,
                    )
                } else if version != Some(4) {
                    (
                        StoredState {
                            reset_notice: Some(ConfigurationResetNotice { id: random_id()? }),
                            legacy_reset_recorded: true,
                            ..Default::default()
                        },
                        true,
                    )
                } else {
                    (
                        serde_json::from_value(json).map_err(|_| RemoteError::StateStorage)?,
                        false,
                    )
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (StoredState::default(), true)
            }
            Err(_) => return Err(RemoteError::StateStorage),
        };
        validate_state(&state)?;
        let store = Self {
            network_settings: crate::settings::SettingsStore::open(path)?,
            path: Some(path.to_owned()),
            state: Mutex::new(state),
            changed: tokio::sync::watch::channel(1).0,
        };
        if write {
            store.change(|_| Ok(()))?;
        }
        Ok(store)
    }
    pub fn snapshot(&self) -> Result<DeviceStoreSnapshot> {
        let state = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        Ok(DeviceStoreSnapshot {
            contacts: state.contacts.values().cloned().collect(),
            incoming_grants: state.grants.values().cloned().collect(),
            telemetry_profile_grants: state.profile_grants.values().cloned().collect(),
            pending: state.pending.clone(),
            reset_notice: state.reset_notice.clone(),
        })
    }
    pub(crate) fn version(&self) -> u64 {
        *self.changed.borrow()
    }
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub(crate) fn touch(&self) {
        self.changed
            .send_modify(|version| *version = version.wrapping_add(1));
    }
    fn change<T>(&self, change: impl FnOnce(&mut StoredState) -> Result<T>) -> Result<T> {
        let mut current = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        let mut next = current.clone();
        let value = change(&mut next)?;
        validate_state(&next)?;
        let bytes = serde_json::to_vec(&next).map_err(|_| RemoteError::StateStorage)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(RemoteError::Capacity);
        }
        if let Some(path) = &self.path {
            atomic_write(path, &bytes).map_err(|_| RemoteError::StateStorage)?;
        }
        *current = next;
        drop(current);
        self.touch();
        Ok(value)
    }
    pub(crate) fn contact(&self, peer: &str) -> Result<DeviceContact> {
        self.state
            .lock()
            .map_err(|_| RemoteError::StateStorage)?
            .contacts
            .get(peer)
            .cloned()
            .ok_or(RemoteError::Unauthorized)
    }
    pub(crate) fn is_pinned(&self, peer: &str) -> Result<bool> {
        Ok(self
            .state
            .lock()
            .map_err(|_| RemoteError::StateStorage)?
            .contacts
            .contains_key(peer))
    }
    pub(crate) fn grant(&self, peer: &str) -> Result<Option<IncomingControlGrant>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| RemoteError::StateStorage)?
            .grants
            .get(peer)
            .cloned())
    }
    pub(crate) fn profile_grant(&self, peer: &str) -> Result<Option<IncomingControlGrant>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| RemoteError::StateStorage)?
            .profile_grants
            .get(peer)
            .cloned())
    }
    pub(crate) fn pairing_grant(
        &self,
        peer: &str,
        attempt: &str,
    ) -> Result<Option<(PairingPermission, IncomingControlGrant)>> {
        let state = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        Ok(state
            .grants
            .get(peer)
            .filter(|g| g.attempt_id == attempt)
            .map(|g| (PairingPermission::Control, g.clone()))
            .or_else(|| {
                state
                    .profile_grants
                    .get(peer)
                    .filter(|g| g.attempt_id == attempt)
                    .map(|g| (PairingPermission::TelemetryProfileRead, g.clone()))
            }))
    }
    pub(crate) fn commit_profile_grant(&self, grant: IncomingControlGrant) -> Result<()> {
        self.change(|state| {
            state.profile_grants.insert(grant.peer_id.clone(), grant);
            Ok(())
        })
    }
    pub(crate) fn revoke_profile_grant(&self, peer: &str) -> Result<()> {
        self.change(|state| {
            state.profile_grants.remove(peer);
            Ok(())
        })
    }
    pub(crate) fn save_pin(&self, mut contact: DeviceContact) -> Result<()> {
        self.change(|state| {
            let attempt_id = match &contact.confirmation {
                ContactConfirmation::Pending { attempt_id } => attempt_id.clone(),
                _ => return Err(RemoteError::PairingUnavailable),
            };
            state.pending_pairings.insert(
                contact.peer_id.clone(),
                PendingPairing {
                    attempt_id,
                    permission: contact.pairing_permission,
                },
            );
            if let Some(existing) = state.contacts.get(&contact.peer_id) {
                contact.paused = existing.paused;
                contact.control_authorized |= existing.control_authorized;
                if matches!(existing.confirmation, ContactConfirmation::Confirmed { .. }) {
                    contact.confirmation = existing.confirmation.clone();
                    contact.pairing_permission = existing.pairing_permission;
                }
            }
            state.contacts.insert(contact.peer_id.clone(), contact);
            Ok(())
        })
    }
    pub(crate) fn confirm_pin(&self, peer: &str, attempt: &str) -> Result<()> {
        self.change(|state| {
            let pending = state
                .pending_pairings
                .get(peer)
                .filter(|pending| pending.attempt_id == attempt)
                .cloned();
            let contact = state
                .contacts
                .get_mut(peer)
                .ok_or(RemoteError::Unauthorized)?;
            let permission = if let Some(pending) = pending {
                pending.permission
            } else {
                match &contact.confirmation {
                    ContactConfirmation::Pending { attempt_id: id } if id == attempt => {
                        contact.pairing_permission
                    }
                    ContactConfirmation::Confirmed { attempt_id: id } if id == attempt => {
                        return Ok(());
                    }
                    _ => return Err(RemoteError::PairingUnavailable),
                }
            };
            if permission == PairingPermission::Control {
                contact.control_authorized = true;
            }
            contact.pairing_permission = permission;
            contact.confirmation = ContactConfirmation::Confirmed {
                attempt_id: attempt.into(),
            };
            state.pending_pairings.remove(peer);
            Ok(())
        })
    }
    pub(crate) fn pending_pairing(&self, peer: &str) -> Result<Option<PendingPairing>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| RemoteError::StateStorage)?
            .pending_pairings
            .get(peer)
            .cloned())
    }
    pub(crate) fn pending_pairing_peers(&self) -> Result<Vec<String>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| RemoteError::StateStorage)?
            .pending_pairings
            .keys()
            .cloned()
            .collect())
    }
    pub(crate) fn finish_ungranted_pairing(&self, peer: &str, attempt: &str) -> Result<()> {
        self.change(|state| {
            if state
                .pending_pairings
                .get(peer)
                .is_some_and(|p| p.attempt_id == attempt)
            {
                state.pending_pairings.remove(peer);
            }
            Ok(())
        })
    }
    pub(crate) fn commit_grant(&self, grant: IncomingControlGrant) -> Result<()> {
        self.change(|state| {
            state.grants.insert(grant.peer_id.clone(), grant);
            Ok(())
        })
    }
    pub(crate) fn update_locator(
        &self,
        peer: &str,
        name: String,
        locator: DeviceLocator,
    ) -> Result<()> {
        self.update_locator_from(peer, name, locator, LocatorSource::AuthenticatedPeer)
    }
    pub(crate) fn update_locator_from(
        &self,
        peer: &str,
        name: String,
        locator: DeviceLocator,
        source: LocatorSource,
    ) -> Result<()> {
        locator.validate()?;
        self.change(|state| {
            let contact = state
                .contacts
                .get_mut(peer)
                .ok_or(RemoteError::Unauthorized)?;
            contact.name = name;
            contact.locator = locator;
            contact.locator_source = source;
            Ok(())
        })
    }
    pub(crate) fn record_legacy_reset(&self) -> Result<()> {
        self.change(|state| {
            if !state.legacy_reset_recorded {
                state.reset_notice = Some(ConfigurationResetNotice { id: random_id()? });
                state.legacy_reset_recorded = true;
            }
            Ok(())
        })
    }
    pub(crate) fn revoke_grant(&self, peer: &str) -> Result<()> {
        self.change(|state| {
            state.grants.remove(peer);
            Ok(())
        })
    }
    pub(crate) fn forget_contact(&self, peer: &str) -> Result<()> {
        self.change(|state| {
            if state.pending.iter().any(|op| op.peer_id == peer) {
                return Err(RemoteError::PendingRequests);
            }
            state.contacts.remove(peer);
            state.pending_pairings.remove(peer);
            Ok(())
        })
    }
    pub(crate) fn pause(&self, peer: &str, paused: bool) -> Result<()> {
        self.change(|state| {
            state
                .contacts
                .get_mut(peer)
                .ok_or(RemoteError::Unauthorized)?
                .paused = paused;
            Ok(())
        })
    }
    pub(crate) fn record_pending(&self, operation: PendingOperation) -> Result<()> {
        self.change(|state| {
            if let Some(existing) = state.pending.iter().find(|existing| {
                existing.peer_id == operation.peer_id && existing.request_id == operation.request_id
            }) {
                if existing.kind != operation.kind
                    || existing.view != operation.view
                    || existing.execution.expected_revision != operation.execution.expected_revision
                {
                    return Err(RemoteError::RequestIdConflict);
                }
                return Ok(());
            }
            if state.pending.len() >= MAX_PENDING {
                return Err(RemoteError::Capacity);
            }
            state.pending.push(operation);
            Ok(())
        })
    }
    pub(crate) fn update_pending(
        &self,
        peer: &str,
        id: &RequestId,
        status: PendingStatus,
    ) -> Result<()> {
        self.change(|state| {
            let pending = state
                .pending
                .iter_mut()
                .find(|operation| operation.peer_id == peer && operation.request_id == *id)
                .ok_or(RemoteError::InvalidRequestId)?;
            pending.status = status;
            Ok(())
        })
    }
    pub(crate) fn acknowledge(&self, peer: &str, id: &RequestId) -> Result<()> {
        self.change(|state| {
            state
                .pending
                .retain(|operation| operation.peer_id != peer || operation.request_id != *id);
            Ok(())
        })
    }
    pub(crate) fn acknowledge_reset(&self, id: &str) -> Result<()> {
        self.change(|state| {
            if state
                .reset_notice
                .as_ref()
                .is_some_and(|notice| notice.id == id)
            {
                state.reset_notice = None;
            }
            Ok(())
        })
    }
}
pub(crate) fn parse_peer(peer: &str) -> Result<iroh::EndpointId> {
    {
        let key: iroh::EndpointId = peer.parse().map_err(|_| RemoteError::Configuration)?;
        if key.to_string() != peer {
            return Err(RemoteError::Configuration);
        }
        Ok(key)
    }
}
fn validate_state(state: &StoredState) -> Result<()> {
    if state.contacts.len() > MAX_CONTACTS
        || state.grants.len() > MAX_CONTACTS
        || state.profile_grants.len() > MAX_CONTACTS
        || state.pending_pairings.len() > MAX_CONTACTS
        || state.pending.len() > MAX_PENDING
    {
        return Err(RemoteError::Capacity);
    }
    for (id, contact) in &state.contacts {
        if id != &contact.peer_id {
            return Err(RemoteError::StateStorage);
        }
        parse_peer(id)?;
        validate_name(&contact.name)?;
        contact.locator.validate()?;
        match &contact.confirmation {
            ContactConfirmation::Pending { attempt_id: id }
            | ContactConfirmation::Confirmed { attempt_id: id } => {
                crate::protocol::validate_id(id)?
            }
        }
    }
    for (id, grant) in state.grants.iter().chain(&state.profile_grants) {
        if id != &grant.peer_id {
            return Err(RemoteError::StateStorage);
        }
        parse_peer(id)?;
        validate_name(&grant.name)?;
        crate::protocol::validate_id(&grant.attempt_id)?;
    }
    for (peer, pending) in &state.pending_pairings {
        if !state.contacts.contains_key(peer) {
            return Err(RemoteError::StateStorage);
        }
        crate::protocol::validate_id(&pending.attempt_id)?;
    }
    for pending in &state.pending {
        parse_peer(&pending.peer_id)?;
        pending
            .request_id
            .validate(&pending.request_id.incarnation)?;
        if pending.view.is_empty() || pending.view.len() > 128 {
            return Err(RemoteError::StateStorage);
        }
    }
    if let Some(notice) = &state.reset_notice {
        crate::protocol::validate_id(&notice.id)?;
    }
    Ok(())
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn v3_preserves_contacts_grants_pending_and_notice_without_sharing_consent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote-state.json");
        let peer = DeviceIdentity::generate("synthetic".into()).unwrap().id();
        let attempt = random_id().unwrap();
        let mut old = serde_json::to_value(StoredState::default()).unwrap();
        old["version"] = serde_json::json!(3);
        old.as_object_mut().unwrap().remove("profile_grants");
        old["contacts"][&peer] = serde_json::json!({ "peer_id": peer, "name": "synthetic", "locator": {"relay_urls": [], "direct_addresses": [], "lookup_urls": []}, "confirmation": {"state":"confirmed", "attempt_id":attempt}, "paused":true, "locator_source":"pairingOffer" });
        old["grants"][&peer] =
            serde_json::json!({"peer_id":peer,"name":"synthetic","attempt_id":attempt});
        old["pending"] = serde_json::json!([{"peer_id":peer,"request_id":{"incarnation":random_id().unwrap(),"client":random_id().unwrap(),"sequence":1},"view":"synthetic","execution":{"expected_revision":null},"kind":"List","runtime_id":null,"conversation_id":null,"status":"unknown"}]);
        // Use the authoritative enum encoding instead of hand-maintaining its wire representation.
        old["pending"][0]["kind"] = serde_json::to_value(CommandKind::ReadConfiguration).unwrap();
        old["reset_notice"] = serde_json::json!({"id":random_id().unwrap()});
        atomic_write(&path, &serde_json::to_vec(&old).unwrap()).unwrap();
        let store = DeviceStore::open(&path).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.contacts.len(), 1);
        assert!(snapshot.contacts[0].paused && snapshot.contacts[0].control_authorized);
        assert_eq!(snapshot.incoming_grants[0].attempt_id, attempt);
        assert!(snapshot.telemetry_profile_grants.is_empty());
        assert_eq!(snapshot.pending[0].request_id.sequence, 1);
        assert!(matches!(snapshot.pending[0].status, PendingStatus::Unknown));
        assert_eq!(
            serde_json::to_value(snapshot.reset_notice).unwrap(),
            old["reset_notice"]
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()["version"],
            4
        );
    }
    #[test]
    fn unknown_future_version_is_not_reset_or_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote-state.json");
        let bytes = br#"{"version":99,"synthetic":"preserve"}"#;
        atomic_write(&path, bytes).unwrap();
        assert!(DeviceStore::open(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn damaged_v3_contact_returns_error_without_panicking_or_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote-state.json");
        let mut old = serde_json::to_value(StoredState::default()).unwrap();
        old["version"] = serde_json::json!(3);
        old["contacts"]["synthetic"] = serde_json::json!(42);
        let bytes = serde_json::to_vec(&old).unwrap();
        atomic_write(&path, &bytes).unwrap();
        assert!(DeviceStore::open(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
