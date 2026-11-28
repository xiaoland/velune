//! One atomic ordinary-state file. Identity and business data have independent lifetimes.
use crate::{
    identity::{atomic_write, read_private, validate_name},
    trust::{ExecutionCapability, TrustBundle, TrustContact, TrustState},
    *,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
#[serde(tag = "state", rename_all = "camelCase", deny_unknown_fields)]
pub enum ContactConfirmation {
    Pending { attempt_id: String },
    Confirmed { attempt_id: String },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum LocatorSource {
    PairingOffer,
    AuthenticatedPeer,
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

/// Current local membership, distinct from historical direct pairing grants.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct DeviceMember {
    pub peer_id: String,
    pub name: String,
    pub can_execute: Option<bool>,
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
    trust: TrustState,
}
impl Default for StoredState {
    fn default() -> Self {
        Self {
            version: 6,
            contacts: BTreeMap::new(),
            grants: BTreeMap::new(),
            profile_grants: BTreeMap::new(),
            pending_pairings: BTreeMap::new(),
            pending: Vec::new(),
            reset_notice: None,
            legacy_reset_recorded: false,
            trust: TrustState::default(),
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
    pub members: Vec<DeviceMember>,
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
    signing_key: Mutex<Option<iroh::SecretKey>>,
    write_failed: AtomicBool,
    changed: tokio::sync::watch::Sender<u64>,
}
impl DeviceStore {
    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self {
            network_settings: crate::settings::SettingsStore::memory(),
            path: None,
            state: Mutex::new(StoredState::default()),
            signing_key: Mutex::new(None),
            write_failed: AtomicBool::new(false),
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
                if version == Some(3) {
                    // v3 already contains pins and control grants. Preserve those
                    // decisions; v4 adds separate profile grants and pairing state.
                    validate_v3_shape(&json)?;
                    let contacts = json
                        .get_mut("contacts")
                        .and_then(serde_json::Value::as_object_mut)
                        .ok_or(RemoteError::StateStorage)?;
                    for contact in contacts.values_mut() {
                        let contact = contact.as_object_mut().ok_or(RemoteError::StateStorage)?;
                        contact.insert("control_authorized".into(), serde_json::Value::Bool(true));
                        contact.insert(
                            "pairing_permission".into(),
                            serde_json::Value::String("control".into()),
                        );
                    }
                    json["version"] = serde_json::json!(4);
                    json["profile_grants"] = serde_json::json!({});
                    json["pending_pairings"] = serde_json::json!({});
                } else if version != Some(4) && version != Some(5) && version != Some(6) {
                    return Err(RemoteError::StateStorage);
                }
                let migrate = version != Some(6);
                if version == Some(3) || version == Some(4) {
                    // No v4 field is optional. Reject unknown old-schema fields,
                    // including a v5 trust block masquerading as v4.
                    if json.get("trust").is_some() {
                        return Err(RemoteError::StateStorage);
                    }
                    json["trust"] = serde_json::to_value(TrustState::default())
                        .map_err(|_| RemoteError::StateStorage)?;
                    json["version"] = serde_json::json!(5);
                }
                if migrate {
                    upgrade_membership_schema(&mut json, version.unwrap() as u16)?;
                }
                let mut state: StoredState =
                    serde_json::from_value(json).map_err(|_| RemoteError::StateStorage)?;
                // Validate all legacy facts before deriving any new consent.
                validate_state(&state)?;
                if migrate {
                    migrate_direct_members(&mut state, version.unwrap() as u16)?;
                }
                validate_state(&state)?;
                if migrate {
                    backup_state(path, &bytes, version.unwrap() as u16)?;
                }
                (state, migrate)
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
            signing_key: Mutex::new(None),
            write_failed: AtomicBool::new(false),
            changed: tokio::sync::watch::channel(1).0,
        };
        if write {
            let bytes =
                serde_json::to_vec(&*store.state.lock().map_err(|_| RemoteError::StateStorage)?)
                    .map_err(|_| RemoteError::StateStorage)?;
            if bytes.len() > MAX_STATE_BYTES {
                return Err(RemoteError::Capacity);
            }
            atomic_write(path, &bytes).map_err(|_| RemoteError::StateStorage)?;
        }
        Ok(store)
    }
    pub fn snapshot(&self) -> Result<DeviceStoreSnapshot> {
        let state = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        self.ensure_healthy()?;
        Ok(DeviceStoreSnapshot {
            members: member_projection(&state)?,
            contacts: state.contacts.values().cloned().collect(),
            incoming_grants: state.grants.values().cloned().collect(),
            telemetry_profile_grants: state.profile_grants.values().cloned().collect(),
            pending: state.pending.clone(),
            reset_notice: state.reset_notice.clone(),
        })
    }
    /// Bind the host's existing identity before serving or changing grants. Only
    /// this key signs local decisions; no secret is serialized into ordinary state.
    pub fn bind_trust_identity(&self, key: &iroh::SecretKey) -> Result<()> {
        let mut signer = self
            .signing_key
            .lock()
            .map_err(|_| RemoteError::IdentityStorage)?;
        if signer
            .as_ref()
            .is_some_and(|old| old.public() != key.public())
        {
            return Err(RemoteError::PeerMismatch);
        }
        let issuer = key.public().to_string();
        self.change_signed(Some(key), |state| {
            if let Some(existing) = &state.trust.issuer {
                if existing != &issuer {
                    return Err(RemoteError::PeerMismatch);
                }
            } else {
                state.trust.issuer = Some(issuer);
            }
            state.trust.membership_initialized = true;
            Ok(())
        })?;
        *signer = Some(key.clone());
        Ok(())
    }
    pub fn initialize_trust(&self, identity: &DeviceIdentity) -> Result<()> {
        self.bind_trust_identity(&identity.key)
    }
    /// Includes tombstones. Supplying the live local descriptor advertises this
    /// endpoint too; incarnation is deliberately not a persisted locator fact.
    pub fn export_trust_bundle(&self, local: Option<&EndpointDescriptor>) -> Result<TrustBundle> {
        let state = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        self.ensure_healthy()?;
        let issuer = state
            .trust
            .issuer
            .as_deref()
            .ok_or(RemoteError::IdentityStorage)?;
        let mut bundle = TrustBundle {
            statements: state.trust.statements.values().cloned().collect(),
            memberships: state.trust.memberships.values().cloned().collect(),
            contacts: state
                .contacts
                .values()
                .filter(|contact| {
                    state.trust.is_member(&contact.peer_id).unwrap_or(false)
                        && matches!(contact.confirmation, ContactConfirmation::Confirmed { .. })
                })
                .map(|contact| TrustContact {
                    peer_id: contact.peer_id.clone(),
                    name: contact.name.clone(),
                    locator: contact.locator.clone(),
                    can_execute: execution_hint(&state, &contact.peer_id),
                })
                .collect(),
            ..TrustBundle::default()
        };
        if let Some(local) = local {
            local.validate(issuer)?;
            bundle.contacts.push(TrustContact {
                peer_id: issuer.into(),
                name: local.name.clone(),
                can_execute: state.trust.local_execution_capability,
                locator: DeviceLocator {
                    relay_urls: local.relay_urls.clone(),
                    direct_addresses: local.direct_addresses.clone(),
                    lookup_urls: local.lookup_urls.clone(),
                },
            });
        }
        bundle.validate()?;
        Ok(bundle)
    }
    /// `authenticated_peer` must come from the authenticated transport, never
    /// from a bundle field. A first transitive operator may prove its authority
    /// using the bundle, rooted in this store's current local grants.
    pub fn merge_trust_bundle(&self, authenticated_peer: &str, bundle: &TrustBundle) -> Result<()> {
        parse_peer(authenticated_peer)?;
        bundle.validate()?;
        self.change(|state| {
            let mut merged = state.trust.clone();
            merged.merge(&bundle.statements)?;
            merged.merge_memberships(&bundle.memberships)?;
            // Source can first prove transitive admission in this exchange, but
            // only from installed local roots, never a forwarded local signature.
            if !merged.is_member(authenticated_peer)? {
                return Err(RemoteError::Unauthorized);
            }
            merged.retain_network(&state.trust)?;
            state.trust = merged;
            for contact in &bundle.contacts {
                if state.trust.is_member(&contact.peer_id)? {
                    import_verified_contact(
                        state,
                        &contact.peer_id,
                        contact.name.clone(),
                        contact.locator.clone(),
                        authenticated_peer,
                    )?;
                    if let Some(can_execute) = contact.can_execute {
                        let self_reported = contact.peer_id == authenticated_peer;
                        let old = state.trust.execution_capabilities.get(&contact.peer_id);
                        if self_reported || old.is_none_or(|old| !old.self_reported) {
                            state.trust.execution_capabilities.insert(
                                contact.peer_id.clone(),
                                ExecutionCapability {
                                    can_execute,
                                    self_reported,
                                },
                            );
                        }
                    }
                }
            }
            refresh_contact_authority(state)?;
            Ok(())
        })
    }
    pub fn set_local_execution_capability(&self, can_execute: bool) -> Result<()> {
        self.change(|state| {
            state.trust.local_execution_capability = Some(can_execute);
            Ok(())
        })
    }
    pub fn is_member(&self, peer: &str) -> Result<bool> {
        self.lock_state()?.trust.is_member(peer)
    }
    #[cfg(test)]
    pub(crate) fn membership_proof(&self, peer: &str) -> Result<crate::trust::MembershipProof> {
        self.lock_state()?.trust.membership_proof(peer)
    }
    pub fn allows_control(&self, target: &str, operator: &str) -> Result<bool> {
        parse_peer(target)?;
        parse_peer(operator)?;
        let state = self.lock_state()?;
        // Only the target's own store can authorize execution at that target.
        if target != issuer_owned(&state)? {
            return Ok(false);
        }
        state.trust.is_member(operator)
    }
    pub fn introducers_for(&self, peer: &str) -> Result<Vec<String>> {
        parse_peer(peer)?;
        let state = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        self.ensure_healthy()?;
        let Some(contact) = state.contacts.get(peer) else {
            return Ok(Vec::new());
        };
        if !state.trust.is_member(peer)? || contact.paused {
            return Ok(Vec::new());
        }
        let mut sources = Vec::new();
        if let Some(introducers) = state.trust.introduced_via.get(peer) {
            for source in introducers {
                if source != peer && trusted_source(&state, source)? {
                    sources.push(source.clone());
                }
            }
        }
        Ok(sources)
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
        let key = self
            .signing_key
            .lock()
            .map_err(|_| RemoteError::IdentityStorage)?;
        self.change_signed(key.as_ref(), change)
    }
    fn change_signed<T>(
        &self,
        key: Option<&iroh::SecretKey>,
        change: impl FnOnce(&mut StoredState) -> Result<T>,
    ) -> Result<T> {
        let mut current = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        self.ensure_healthy()?;
        if current.trust.issuer.is_some() && key.is_none() {
            return Err(RemoteError::IdentityStorage);
        }
        let mut next = current.clone();
        let value = change(&mut next)?;
        if let Some(key) = key {
            reconcile_local_trust(&mut next, key)?;
        }
        validate_state(&next)?;
        let bytes = serde_json::to_vec(&next).map_err(|_| RemoteError::StateStorage)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(RemoteError::Capacity);
        }
        if serde_json::to_vec(&*current).map_err(|_| RemoteError::StateStorage)? == bytes
            && self.path.as_ref().is_none_or(|path| path.exists())
        {
            return Ok(value);
        }
        if let Some(path) = &self.path
            && atomic_write(path, &bytes).is_err()
        {
            // rename may have succeeded before directory fsync failed. Do
            // not keep serving from a possibly older in-memory grant set.
            self.write_failed.store(true, Ordering::Release);
            return Err(RemoteError::StateStorage);
        }
        *current = next;
        drop(current);
        self.touch();
        Ok(value)
    }
    fn ensure_healthy(&self) -> Result<()> {
        if self.write_failed.load(Ordering::Acquire) {
            Err(RemoteError::StateStorage)
        } else {
            Ok(())
        }
    }
    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, StoredState>> {
        let state = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        self.ensure_healthy()?;
        Ok(state)
    }
    pub(crate) fn contact(&self, peer: &str) -> Result<DeviceContact> {
        self.lock_state()?
            .contacts
            .get(peer)
            .cloned()
            .ok_or(RemoteError::Unauthorized)
    }
    pub(crate) fn is_pinned(&self, peer: &str) -> Result<bool> {
        Ok(self.lock_state()?.contacts.contains_key(peer))
    }
    pub(crate) fn grant(&self, peer: &str) -> Result<Option<IncomingControlGrant>> {
        Ok(self.lock_state()?.grants.get(peer).cloned())
    }
    pub(crate) fn profile_grant(&self, peer: &str) -> Result<Option<IncomingControlGrant>> {
        Ok(self.lock_state()?.profile_grants.get(peer).cloned())
    }
    pub(crate) fn pairing_grant(
        &self,
        peer: &str,
        attempt: &str,
    ) -> Result<Option<(PairingPermission, IncomingControlGrant)>> {
        let state = self.state.lock().map_err(|_| RemoteError::StateStorage)?;
        self.ensure_healthy()?;
        Ok(state
            .grants
            .get(peer)
            // The record is retained as a replay fence, not an active grant.
            // Returning it here would report a revoked pairing as Granted.
            .filter(|g| g.attempt_id == attempt && !state.trust.forgotten_contacts.contains(peer))
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
            state.trust.introduced_locators.remove(&contact.peer_id);
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
                        if state.trust.forgotten_contacts.contains(peer) {
                            return Err(RemoteError::PairingUnavailable);
                        }
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
                state.trust.direct_members.insert(peer.into());
                state.trust.direct_control_pins.insert(peer.into());
                state.trust.forgotten_contacts.remove(peer);
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
            // A repeated completed callback cannot clear a later exclusion.
            if state
                .grants
                .get(&grant.peer_id)
                .is_some_and(|old| old.attempt_id == grant.attempt_id)
            {
                return Ok(());
            }
            state.trust.direct_members.insert(grant.peer_id.clone());
            state.trust.forgotten_contacts.remove(&grant.peer_id);
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
            state.trust.introduced_locators.remove(peer);
            Ok(())
        })
    }
    pub(crate) fn revoke_grant(&self, peer: &str) -> Result<()> {
        self.change(|state| {
            parse_peer(peer)?;
            exclude_member(state, peer);
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
            exclude_member(state, peer);
            state.trust.introduced_via.remove(peer);
            state.trust.introduced_locators.remove(peer);
            state.trust.execution_capabilities.remove(peer);
            for sources in state.trust.introduced_via.values_mut() {
                sources.remove(peer);
            }
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
}
fn issuer_owned(state: &StoredState) -> Result<&str> {
    state
        .trust
        .issuer
        .as_deref()
        .ok_or(RemoteError::IdentityStorage)
}

fn trusted_source(state: &StoredState, source: &str) -> Result<bool> {
    if state
        .contacts
        .get(source)
        .is_some_and(|contact| contact.paused)
    {
        return Ok(false);
    }
    state.trust.is_member(source)
}
fn exclude_member(state: &mut StoredState, peer: &str) {
    state.trust.direct_members.remove(peer);
    state.trust.direct_control_pins.remove(peer);
    state.trust.paired_controls.remove(peer);
    // A confirmation already in flight before revoke is not fresh consent.
    state.pending_pairings.remove(peer);
    state.trust.forgotten_contacts.insert(peer.into());
}
fn execution_hint(state: &StoredState, peer: &str) -> Option<bool> {
    state
        .trust
        .execution_capabilities
        .get(peer)
        .map(|c| c.can_execute)
        .or_else(|| {
            state
                .trust
                .direct_control_pins
                .contains(peer)
                .then_some(true)
        })
}
fn member_projection(state: &StoredState) -> Result<Vec<DeviceMember>> {
    if state.trust.issuer.is_none() {
        return Ok(Vec::new());
    }
    let candidates: std::collections::BTreeSet<_> = state
        .trust
        .memberships
        .values()
        .map(|s| s.subject.clone())
        .collect();
    let mut members = Vec::new();
    for peer in candidates {
        if !state.trust.is_member(&peer)? {
            continue;
        }
        let name = state
            .contacts
            .get(&peer)
            .map(|c| c.name.clone())
            .or_else(|| state.grants.get(&peer).map(|g| g.name.clone()))
            .unwrap_or_else(|| peer.clone());
        members.push(DeviceMember {
            can_execute: execution_hint(state, &peer),
            peer_id: peer,
            name,
        });
    }
    Ok(members)
}

fn import_verified_contact(
    state: &mut StoredState,
    peer: &str,
    name: String,
    locator: DeviceLocator,
    via: &str,
) -> Result<()> {
    if peer == issuer_owned(state)? || state.trust.forgotten_contacts.contains(peer) {
        return Err(RemoteError::Unauthorized);
    }
    parse_peer(via)?;
    let attempt_id =
        state.trust.membership_proof(peer)?.statements[0].membership_id[..32].to_string();
    if let Some(existing) = state.contacts.get_mut(peer) {
        // A third-party introduction cannot overwrite a locator obtained from
        // a direct invitation or target-authenticated descriptor.
        if peer == via || state.trust.introduced_locators.contains(peer) {
            existing.name = name;
            existing.locator = locator;
            existing.locator_source = LocatorSource::AuthenticatedPeer;
        }
        if matches!(existing.confirmation, ContactConfirmation::Pending { .. }) {
            existing.confirmation = ContactConfirmation::Confirmed { attempt_id };
        }
        existing.control_authorized = true;
    } else {
        state.contacts.insert(
            peer.into(),
            DeviceContact {
                peer_id: peer.into(),
                name,
                locator,
                confirmation: ContactConfirmation::Confirmed { attempt_id },
                paused: false,
                locator_source: LocatorSource::AuthenticatedPeer,
                control_authorized: true,
                pairing_permission: PairingPermission::Control,
            },
        );
        if peer != via {
            state.trust.introduced_locators.insert(peer.into());
        }
    }
    if peer == via {
        state.trust.introduced_locators.remove(peer);
    }
    state
        .trust
        .introduced_via
        .entry(peer.into())
        .or_default()
        .insert(via.into());
    Ok(())
}

fn refresh_contact_authority(state: &mut StoredState) -> Result<()> {
    let targets: std::collections::BTreeSet<_> = state
        .contacts
        .keys()
        .filter(|peer| {
            state.trust.is_member(peer).unwrap_or(false)
                && execution_hint(state, peer) == Some(true)
        })
        .cloned()
        .collect();
    for contact in state.contacts.values_mut() {
        contact.control_authorized = targets.contains(&contact.peer_id);
    }
    Ok(())
}

fn reconcile_local_trust(state: &mut StoredState, key: &iroh::SecretKey) -> Result<()> {
    let issuer = issuer_owned(state)?.to_string();
    if issuer != key.public().to_string() {
        return Err(RemoteError::PeerMismatch);
    }
    // Keep every local tombstone, even after a contact is forgotten. A later
    // explicit grant advances the same relation's revision rather than resetting it.
    let previous: Vec<_> = state
        .trust
        .statements
        .values()
        .filter(|edge| edge.issuer == issuer)
        .map(|edge| (edge.subject.clone(), edge.permission))
        .collect();
    for (subject, permission) in previous {
        let grants = match permission {
            PairingPermission::Control => &state.grants,
            PairingPermission::TelemetryProfileRead => &state.profile_grants,
        };
        state.trust.set_local(
            key,
            &subject,
            permission,
            !grants.contains_key(&subject)
                || (permission == PairingPermission::Control
                    && state.trust.forgotten_contacts.contains(&subject)),
        )?;
    }
    for (permission, grants) in [
        (PairingPermission::Control, &state.grants),
        (
            PairingPermission::TelemetryProfileRead,
            &state.profile_grants,
        ),
    ] {
        for subject in grants.keys() {
            let revoked = permission == PairingPermission::Control
                && state.trust.forgotten_contacts.contains(subject);
            state.trust.set_local(key, subject, permission, revoked)?;
        }
    }
    let subjects: std::collections::BTreeSet<_> = state
        .trust
        .memberships
        .values()
        .filter(|s| s.issuer == issuer)
        .map(|s| s.subject.clone())
        .chain(state.trust.direct_members.iter().cloned())
        .chain(state.trust.forgotten_contacts.iter().cloned())
        .collect();
    for subject in subjects {
        let revoked = !state.trust.direct_members.contains(&subject);
        state.trust.set_local_membership(key, &subject, revoked)?;
    }
    refresh_contact_authority(state)
}

/// The old trust object is decoded strictly before introducing v6 fields.
fn upgrade_membership_schema(json: &mut serde_json::Value, source_version: u16) -> Result<()> {
    let trust = json
        .get_mut("trust")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(RemoteError::StateStorage)?;
    let old_fields = [
        "issuer",
        "statements",
        "paired_controls",
        "introduced_via",
        "introduced_locators",
        "forgotten_contacts",
    ];
    // v3/v4 just received a complete default TrustState from our own converter.
    // Their original format was checked before that insertion.
    if source_version == 5 {
        if trust.len() != old_fields.len()
            || trust.keys().any(|k| !old_fields.contains(&k.as_str()))
        {
            return Err(RemoteError::StateStorage);
        }
        let defaults =
            serde_json::to_value(TrustState::default()).map_err(|_| RemoteError::StateStorage)?;
        for (key, value) in defaults.as_object().ok_or(RemoteError::StateStorage)? {
            if !old_fields.contains(&key.as_str()) {
                trust.insert(key.clone(), value.clone());
            }
        }
    }
    json["version"] = serde_json::json!(6);
    Ok(())
}
fn migrate_direct_members(state: &mut StoredState, source_version: u16) -> Result<()> {
    // v5 introductions used the first legacy edge's deterministic grant ID as
    // their synthetic confirmation ID. They are not completed direct pairings,
    // even after a later authenticated Hello replaced their locator.
    for contact in state.contacts.values() {
        let ContactConfirmation::Confirmed { attempt_id } = &contact.confirmation else {
            continue;
        };
        let synthetic = state.trust.statements.values().any(|edge| {
            edge.issuer == contact.peer_id
                && edge.permission == PairingPermission::Control
                && edge.grant_id.starts_with(attempt_id)
        });
        // v5 overwrote the last pairing permission on a later Telemetry
        // confirmation, and could consume paired_controls. Without that direct
        // provenance or an incoming grant, a true former Control pin is then
        // indistinguishable from a Telemetry contact authorized by introduction.
        // Preserve its contact/history, but require fresh explicit Control
        // pairing rather than minting an unprovable local acceptance.
        let direct = if source_version < 5 {
            contact.control_authorized
        } else {
            state.trust.paired_controls.contains(&contact.peer_id)
                || (contact.pairing_permission == PairingPermission::Control && !synthetic)
        };
        if direct {
            state.trust.direct_members.insert(contact.peer_id.clone());
            state
                .trust
                .direct_control_pins
                .insert(contact.peer_id.clone());
        }
    }
    state
        .trust
        .direct_members
        .extend(state.grants.keys().cloned());
    // A previously issued local revoke remains a local rejection after upgrade.
    state.trust.forgotten_contacts.extend(
        state
            .trust
            .statements
            .values()
            .filter(|edge| {
                Some(edge.issuer.as_str()) == state.trust.issuer.as_deref()
                    && edge.permission == PairingPermission::Control
                    && edge.revoked
            })
            .map(|edge| edge.subject.clone()),
    );
    let exclusions = state.trust.forgotten_contacts.clone();
    for peer in exclusions {
        exclude_member(state, &peer);
        if let Some(contact) = state.contacts.get_mut(&peer) {
            contact.control_authorized = false;
        }
    }
    Ok(())
}

fn validate_v3_shape(value: &serde_json::Value) -> Result<()> {
    fn only_keys(value: &serde_json::Value, allowed: &[&str]) -> bool {
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
    }

    if !only_keys(
        value,
        &[
            "version",
            "contacts",
            "grants",
            "pending",
            "reset_notice",
            "legacy_reset_recorded",
        ],
    ) {
        return Err(RemoteError::StateStorage);
    }
    let contacts = value
        .get("contacts")
        .and_then(serde_json::Value::as_object)
        .ok_or(RemoteError::StateStorage)?;
    if contacts.values().any(|contact| {
        !only_keys(
            contact,
            &[
                "peer_id",
                "name",
                "locator",
                "confirmation",
                "paused",
                "locator_source",
            ],
        ) || !contact.get("locator").is_some_and(|locator| {
            only_keys(locator, &["relay_urls", "direct_addresses", "lookup_urls"])
        })
    }) {
        return Err(RemoteError::StateStorage);
    }
    Ok(())
}

fn backup_state(path: &Path, bytes: &[u8], version: u16) -> Result<()> {
    let prefix = if version == 3 {
        "remote-state.before-beta-v3".to_string()
    } else {
        format!(
            "{}.before-v{version}",
            path.file_stem()
                .ok_or(RemoteError::StateStorage)?
                .to_string_lossy()
        )
    };
    let backup = path.with_file_name(format!("{prefix}.json"));
    if matches!(read_private(&backup), Ok(existing) if existing == bytes) {
        sync_backup_directory(&backup).map_err(|_| RemoteError::StateStorage)?;
        return Ok(());
    }
    // Older builds could leave a partially written file at the fixed backup
    // path. Keep it intact and publish this source under a content-derived name.
    let backup = if backup.exists() {
        let digest = Sha256::digest(bytes);
        let mut name = format!("{prefix}-");
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut name, "{byte:02x}").map_err(|_| RemoteError::StateStorage)?;
        }
        name.push_str(".json");
        path.with_file_name(name)
    } else {
        backup
    };
    if matches!(read_private(&backup), Ok(existing) if existing == bytes) {
        sync_backup_directory(&backup).map_err(|_| RemoteError::StateStorage)?;
        return Ok(());
    }

    match crate::identity::publish_new_private(&backup, bytes) {
        Ok(()) => {
            if read_private(&backup).map_err(|_| RemoteError::StateStorage)? != bytes {
                return Err(RemoteError::StateStorage);
            }
            Ok(())
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                && matches!(read_private(&backup), Ok(existing) if existing == bytes) =>
        {
            sync_backup_directory(&backup).map_err(|_| RemoteError::StateStorage)
        }
        Err(_) => Err(RemoteError::StateStorage),
    }
}

#[cfg(unix)]
fn sync_backup_directory(backup: &Path) -> std::io::Result<()> {
    let parent = backup
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_backup_directory(_backup: &Path) -> std::io::Result<()> {
    Ok(())
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
    if state.version != 6 {
        return Err(RemoteError::StateStorage);
    }
    state.trust.validate()?;
    if state.trust.membership_initialized {
        let issuer = issuer_owned(state)?;
        for contact in state.contacts.values() {
            let authorized = state.trust.is_member(&contact.peer_id)?
                && execution_hint(state, &contact.peer_id) == Some(true);
            if contact.control_authorized != authorized {
                return Err(RemoteError::StateStorage);
            }
        }
        for edge in state
            .trust
            .memberships
            .values()
            .filter(|s| s.issuer == issuer)
        {
            if edge.revoked == state.trust.direct_members.contains(&edge.subject) {
                return Err(RemoteError::StateStorage);
            }
        }
        for peer in &state.trust.direct_members {
            if !state
                .trust
                .memberships
                .values()
                .any(|s| s.issuer == issuer && &s.subject == peer && !s.revoked)
            {
                return Err(RemoteError::StateStorage);
            }
        }
    } else if !state.trust.memberships.is_empty() {
        return Err(RemoteError::StateStorage);
    }
    if state
        .trust
        .direct_control_pins
        .iter()
        .any(|p| !state.contacts.contains_key(p) || !state.trust.direct_members.contains(p))
    {
        return Err(RemoteError::StateStorage);
    }
    if state
        .trust
        .paired_controls
        .iter()
        .any(|peer| !state.contacts.contains_key(peer))
        || state.trust.introduced_locators.iter().any(|peer| {
            !state.contacts.contains_key(peer) || !state.trust.introduced_via.contains_key(peer)
        })
        || state
            .trust
            .introduced_via
            .keys()
            .any(|peer| !state.contacts.contains_key(peer))
        || state.trust.forgotten_contacts.iter().any(|peer| {
            state
                .contacts
                .get(peer)
                .is_some_and(|contact| contact.control_authorized)
        })
    {
        return Err(RemoteError::StateStorage);
    }
    if let Some(issuer) = &state.trust.issuer {
        for edge in state
            .trust
            .statements
            .values()
            .filter(|edge| &edge.issuer == issuer)
        {
            let grants = match edge.permission {
                PairingPermission::Control => &state.grants,
                PairingPermission::TelemetryProfileRead => &state.profile_grants,
            };
            let active = grants.contains_key(&edge.subject)
                && !(edge.permission == PairingPermission::Control
                    && state.trust.membership_initialized
                    && state.trust.forgotten_contacts.contains(&edge.subject));
            if edge.revoked == active {
                return Err(RemoteError::StateStorage);
            }
        }
        for (permission, grants) in [
            (PairingPermission::Control, &state.grants),
            (
                PairingPermission::TelemetryProfileRead,
                &state.profile_grants,
            ),
        ] {
            for subject in grants.keys() {
                let revoked = permission == PairingPermission::Control
                    && state.trust.membership_initialized
                    && state.trust.forgotten_contacts.contains(subject);
                if state
                    .trust
                    .statement(issuer, subject, permission)?
                    .is_none_or(|edge| edge.revoked != revoked)
                {
                    return Err(RemoteError::StateStorage);
                }
            }
        }
    }
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
mod trust_tests;

#[cfg(test)]
mod version_tests;
