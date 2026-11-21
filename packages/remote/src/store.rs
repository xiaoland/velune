//! One atomic ordinary-state file. Identity and business data have independent lifetimes.
use crate::{
    identity::{atomic_write, read_private, validate_name},
    trust::{ExecutionCapability, MembershipProof, TrustBundle, TrustContact, TrustState},
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
    pub fn memory() -> Self {
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
    pub fn membership_proof(&self, peer: &str) -> Result<MembershipProof> {
        self.lock_state()?.trust.membership_proof(peer)
    }
    pub fn verify_membership(&self, peer: &str, bundle: &TrustBundle) -> Result<MembershipProof> {
        bundle.validate()?;
        let state = self.lock_state()?;
        let mut trust = state.trust.clone();
        trust.merge(&bundle.statements)?;
        trust.merge_memberships(&bundle.memberships)?;
        trust.membership_proof(peer)
    }
    /// Revoke trust on this device. Other independent paths can still admit
    /// other members, but no proof may end at or pass through this peer locally.
    pub fn revoke_membership(&self, peer: &str) -> Result<()> {
        parse_peer(peer)?;
        self.change(|state| {
            exclude_member(state, peer);
            Ok(())
        })
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
    pub fn import_contact(
        &self,
        peer: &str,
        name: String,
        locator: DeviceLocator,
        via: &str,
    ) -> Result<()> {
        parse_peer(peer)?;
        validate_name(&name)?;
        locator.validate()?;
        self.change(|state| {
            if !trusted_source(state, via)? {
                return Err(RemoteError::Unauthorized);
            }
            if state.trust.forgotten_contacts.contains(peer) {
                return Err(RemoteError::Unauthorized);
            }
            state.trust.membership_proof(peer)?;
            import_verified_contact(state, peer, name, locator, via)
        })
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
    pub fn trusted_contact_via(&self, peer: &str) -> Result<Option<String>> {
        Ok(self.introducers_for(peer)?.into_iter().next())
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
mod trust_tests {
    use super::*;
    use crate::trust::{MembershipStatement, TrustStatement};

    fn device(name: &str) -> (DeviceIdentity, DeviceStore) {
        let identity = DeviceIdentity::generate(name.into()).unwrap();
        let store = DeviceStore::memory();
        store.initialize_trust(&identity).unwrap();
        store.set_local_execution_capability(true).unwrap();
        (identity, store)
    }
    fn grant_attempt(store: &DeviceStore, peer: &DeviceIdentity, attempt: char) {
        store
            .commit_grant(IncomingControlGrant {
                peer_id: peer.id(),
                name: peer.name().into(),
                attempt_id: attempt.to_string().repeat(32),
            })
            .unwrap();
    }
    fn grant(store: &DeviceStore, peer: &DeviceIdentity, permission: PairingPermission) {
        if permission == PairingPermission::Control {
            grant_attempt(store, peer, 'a');
        } else {
            store
                .commit_profile_grant(IncomingControlGrant {
                    peer_id: peer.id(),
                    name: peer.name().into(),
                    attempt_id: "c".repeat(32),
                })
                .unwrap();
        }
    }
    fn pin_attempt(
        store: &DeviceStore,
        peer: &DeviceIdentity,
        attempt: char,
        permission: PairingPermission,
    ) {
        store
            .save_pin(DeviceContact {
                peer_id: peer.id(),
                name: peer.name().into(),
                locator: DeviceLocator::default(),
                confirmation: ContactConfirmation::Pending {
                    attempt_id: attempt.to_string().repeat(32),
                },
                paused: false,
                locator_source: LocatorSource::PairingOffer,
                control_authorized: false,
                pairing_permission: permission,
            })
            .unwrap();
        store
            .confirm_pin(&peer.id(), &attempt.to_string().repeat(32))
            .unwrap();
    }
    fn pin(store: &DeviceStore, peer: &DeviceIdentity) {
        pin_attempt(store, peer, 'b', PairingPermission::Control);
    }
    fn pair(controller: (&DeviceIdentity, &DeviceStore), target: (&DeviceIdentity, &DeviceStore)) {
        pin(controller.1, target.0);
        grant(target.1, controller.0, PairingPermission::Control);
    }
    fn descriptor(peer: &DeviceIdentity) -> EndpointDescriptor {
        EndpointDescriptor {
            version: PROTOCOL_VERSION,
            endpoint_id: peer.id(),
            name: peer.name().into(),
            incarnation: "f".repeat(32),
            relay_urls: vec![],
            direct_addresses: vec![],
            lookup_urls: vec![],
        }
    }
    fn contact(peer: &DeviceIdentity, can_execute: Option<bool>) -> TrustContact {
        TrustContact {
            peer_id: peer.id(),
            name: peer.name().into(),
            locator: DeviceLocator::default(),
            can_execute,
        }
    }
    fn bundle(stores: &[&DeviceStore]) -> TrustBundle {
        let mut statements = BTreeMap::new();
        let mut memberships = BTreeMap::new();
        for store in stores {
            let b = store.export_trust_bundle(None).unwrap();
            for s in b.statements {
                statements.insert(s.grant_id.clone(), s);
            }
            for s in b.memberships {
                memberships.insert(s.membership_id.clone(), s);
            }
        }
        TrustBundle {
            statements: statements.into_values().collect(),
            memberships: memberships.into_values().collect(),
            ..Default::default()
        }
    }
    fn legacy_v5(state: &StoredState) -> serde_json::Value {
        let mut value = serde_json::to_value(state).unwrap();
        value["version"] = serde_json::json!(5);
        let trust = value["trust"].as_object_mut().unwrap();
        for field in [
            "memberships",
            "direct_members",
            "direct_control_pins",
            "membership_initialized",
            "local_execution_capability",
            "execution_capabilities",
        ] {
            trust.remove(field);
        }
        value
    }

    #[test]
    fn common_phone_admits_computers_bidirectionally_without_becoming_execution_target() {
        let (p, sp) = device("Phone");
        sp.set_local_execution_capability(false).unwrap();
        let (a, sa) = device("A");
        let (c, sc) = device("C");
        pair((&p, &sp), (&a, &sa));
        pair((&p, &sp), (&c, &sc));
        let mut proof = bundle(&[&sp, &sa, &sc]);
        proof.contacts = vec![
            contact(&p, Some(false)),
            contact(&a, Some(true)),
            contact(&c, Some(true)),
        ];
        assert_eq!(
            sa.verify_membership(&c.id(), &proof)
                .unwrap()
                .statements
                .len(),
            2
        );
        assert!(!sa.is_member(&c.id()).unwrap()); // verification is pure
        sa.merge_trust_bundle(&p.id(), &proof).unwrap();
        sc.merge_trust_bundle(&p.id(), &proof).unwrap();
        assert!(sa.allows_control(&a.id(), &c.id()).unwrap());
        assert!(sc.allows_control(&c.id(), &a.id()).unwrap());
        assert!(sa.is_member(&p.id()).unwrap());
        assert!(!sa.contact(&p.id()).unwrap().control_authorized);
        assert!(sa.contact(&c.id()).unwrap().control_authorized);
        assert_eq!(sa.introducers_for(&c.id()).unwrap(), vec![p.id()]);
        assert!(
            !sa.snapshot()
                .unwrap()
                .incoming_grants
                .iter()
                .any(|g| g.peer_id == c.id())
        );
        assert!(
            sa.snapshot()
                .unwrap()
                .members
                .iter()
                .any(|m| m.peer_id == c.id() && m.can_execute == Some(true))
        );
        // Authorization is owned by the target, not another store's projection.
        assert!(!sa.allows_control(&c.id(), &a.id()).unwrap());
    }

    #[test]
    fn stranger_reverse_statement_old_control_and_profile_do_not_admit_members() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (x, sx) = device("X");
        pair((&a, &sa), (&p, &sp));
        grant(&sx, &p, PairingPermission::Control); // X accepts P; P never accepts X
        let bad = bundle(&[&sx]);
        assert!(sa.merge_trust_bundle(&x.id(), &bad).is_err());
        sa.merge_trust_bundle(&p.id(), &bad).unwrap();
        assert!(!sa.is_member(&x.id()).unwrap());
        assert!(
            sa.export_trust_bundle(None)
                .unwrap()
                .memberships
                .iter()
                .all(|s| s.issuer != x.id())
        );
        let (t, st) = device("Telemetry");
        grant(&sa, &t, PairingPermission::TelemetryProfileRead);
        pin_attempt(&st, &a, 'd', PairingPermission::TelemetryProfileRead);
        assert!(!sa.is_member(&t.id()).unwrap());
        assert!(!st.is_member(&a.id()).unwrap());
        let mut legacy = TrustBundle::default();
        legacy.statements.push(
            TrustStatement::issue(&p.key, &x.id(), PairingPermission::Control, 1, false).unwrap(),
        );
        sa.merge_trust_bundle(&p.id(), &legacy).unwrap();
        assert!(!sa.is_member(&x.id()).unwrap());
        assert!(sa.verify_membership(&x.id(), &legacy).is_err());
    }

    #[test]
    fn membership_signature_authenticates_every_field_and_separates_permission_purpose() {
        let (a, sa) = device("A");
        let (b, _) = device("B");
        let (c, _) = device("C");
        pin(&sa, &b);
        let member = sa.export_trust_bundle(None).unwrap().memberships.remove(0);
        member.verify().unwrap();
        for field in 0..9 {
            let mut changed = member.clone();
            match field {
                0 => changed.version += 1,
                1 => changed.issuer = c.id(),
                2 => changed.subject = c.id(),
                3 => changed.membership_id = "0".repeat(64),
                4 => changed.revision += 1,
                5 => changed.revoked = true,
                6 => changed.signature[0] ^= 1,
                7 => changed.revision = 0,
                _ => {
                    changed.signature.pop();
                }
            }
            assert!(changed.verify().is_err(), "field {field}");
        }
        let old =
            TrustStatement::issue(&a.key, &b.id(), PairingPermission::Control, 1, false).unwrap();
        let mut confused = member;
        confused.signature = old.signature;
        assert!(confused.verify().is_err());
    }

    #[test]
    fn revoke_excludes_original_member_and_intermediate_but_preserves_independent_paths() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (q, sq) = device("Q");
        let (c, sc) = device("C");
        pair((&p, &sp), (&a, &sa));
        pair((&p, &sp), (&c, &sc));
        pair((&q, &sq), (&a, &sa));
        pair((&q, &sq), (&c, &sc));
        let old = bundle(&[&sa, &sp, &sq, &sc]);
        sc.merge_trust_bundle(&p.id(), &old).unwrap();
        sc.revoke_grant(&p.id()).unwrap();
        assert!(!sc.is_member(&p.id()).unwrap());
        assert!(sc.is_member(&a.id()).unwrap()); // C -> Q -> A still valid
        assert!(
            sc.membership_proof(&a.id())
                .unwrap()
                .statements
                .iter()
                .all(|s| s.subject != p.id())
        );
        sc.revoke_grant(&a.id()).unwrap(); // A has no direct incoming grant at C
        assert!(!sc.allows_control(&c.id(), &a.id()).unwrap());
        sc.merge_trust_bundle(&q.id(), &old).unwrap();
        assert!(!sc.is_member(&a.id()).unwrap());
        assert!(sc.verify_membership(&a.id(), &old).is_err());
        assert!(sc.is_member(&q.id()).unwrap());
        assert!(
            sc.snapshot()
                .unwrap()
                .members
                .iter()
                .all(|m| m.peer_id != a.id() && m.peer_id != p.id())
        );
        let mut injected = TrustBundle::default();
        injected
            .memberships
            .push(MembershipStatement::issue(&c.key, &a.id(), 99, false).unwrap());
        assert!(sc.merge_trust_bundle(&q.id(), &injected).is_err());
        pin_attempt(&sc, &a, 'e', PairingPermission::Control);
        assert!(sc.is_member(&a.id()).unwrap());
        let revived = sc
            .export_trust_bundle(None)
            .unwrap()
            .memberships
            .into_iter()
            .find(|s| s.issuer == c.id() && s.subject == a.id())
            .unwrap();
        assert_eq!(revived.revision, 2);
        assert!(!revived.revoked);
    }

    #[test]
    fn exclusion_survives_replay_restart_and_old_confirmed_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let (a, _) = device("A");
        let (p, sp) = device("P");
        let c = DeviceIdentity::generate("C".into()).unwrap();
        let sc = DeviceStore::open(&path).unwrap();
        sc.initialize_trust(&c).unwrap();
        pin(&sc, &a);
        grant(&sc, &a, PairingPermission::Control);
        pair((&p, &sp), (&c, &sc));
        grant(&sp, &a, PairingPermission::Control);
        let mut old = bundle(&[&sp, &sc]);
        old.contacts.push(contact(&a, Some(true)));
        sc.revoke_grant(&a.id()).unwrap();
        sc.confirm_pin(&a.id(), &"b".repeat(32)).unwrap();
        assert!(
            sc.pairing_grant(&a.id(), &"a".repeat(32))
                .unwrap()
                .is_none()
        );
        grant(&sc, &a, PairingPermission::Control); // duplicate receipt is not new consent
        assert!(!sc.is_member(&a.id()).unwrap());
        pin_attempt(&sc, &a, 'd', PairingPermission::TelemetryProfileRead);
        assert!(!sc.is_member(&a.id()).unwrap());
        drop(sc);
        let sc = DeviceStore::open(&path).unwrap();
        sc.initialize_trust(&c).unwrap();
        sc.merge_trust_bundle(&p.id(), &old).unwrap();
        assert!(!sc.is_member(&a.id()).unwrap());
        assert!(!sc.contact(&a.id()).unwrap().control_authorized);
        assert!(
            sc.import_contact(&a.id(), "A".into(), DeviceLocator::default(), &p.id())
                .is_err()
        );
        grant_attempt(&sc, &a, 'e');
        assert!(sc.is_member(&a.id()).unwrap());
    }

    #[test]
    fn exclusions_keep_history_without_reporting_control_granted() {
        for action in 0..3 {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("state.json");
            let local = DeviceIdentity::generate("Local".into()).unwrap();
            let (peer, _) = device("Peer");
            let store = DeviceStore::open(&path).unwrap();
            store.initialize_trust(&local).unwrap();
            pin(&store, &peer);
            grant(&store, &peer, PairingPermission::Control);
            grant(&store, &peer, PairingPermission::TelemetryProfileRead);
            assert!(
                store
                    .pairing_grant(&peer.id(), &"a".repeat(32))
                    .unwrap()
                    .is_some()
            );
            match action {
                0 => store.revoke_grant(&peer.id()),
                1 => store.revoke_membership(&peer.id()),
                _ => store.forget_contact(&peer.id()),
            }
            .unwrap();
            assert_eq!(
                store.grant(&peer.id()).unwrap().unwrap().attempt_id,
                "a".repeat(32)
            );
            assert!(
                store
                    .pairing_grant(&peer.id(), &"a".repeat(32))
                    .unwrap()
                    .is_none()
            );
            assert!(
                store
                    .pairing_grant(&peer.id(), &"c".repeat(32))
                    .unwrap()
                    .is_some()
            );
            assert!(store.profile_grant(&peer.id()).unwrap().is_some());
            assert_eq!(
                store.snapshot().unwrap().incoming_grants[0].attempt_id,
                "a".repeat(32)
            );
            grant(&store, &peer, PairingPermission::Control); // repeated old callback cannot re-admit
            assert!(!store.is_member(&peer.id()).unwrap());
            let proof = store.export_trust_bundle(None).unwrap();
            assert!(proof.statements.iter().any(|s| s.subject == peer.id()
                && s.permission == PairingPermission::Control
                && s.revoked));
            assert!(
                proof
                    .memberships
                    .iter()
                    .any(|s| s.subject == peer.id() && s.revoked)
            );
            drop(store);
            let reopened = DeviceStore::open(&path).unwrap();
            reopened.initialize_trust(&local).unwrap();
            assert_eq!(
                reopened.grant(&peer.id()).unwrap().unwrap().attempt_id,
                "a".repeat(32)
            );
            assert!(
                reopened
                    .pairing_grant(&peer.id(), &"a".repeat(32))
                    .unwrap()
                    .is_none()
            );
            assert!(reopened.profile_grant(&peer.id()).unwrap().is_some());
            grant(&reopened, &peer, PairingPermission::Control);
            assert!(!reopened.is_member(&peer.id()).unwrap());
            grant_attempt(&reopened, &peer, 'e');
            assert!(reopened.is_member(&peer.id()).unwrap());
            assert!(
                reopened
                    .pairing_grant(&peer.id(), &"a".repeat(32))
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn revoke_invalidates_inflight_pairing_confirmation_until_a_new_explicit_attempt() {
        let (_, sa) = device("A");
        let (p, _) = device("P");
        sa.save_pin(DeviceContact {
            peer_id: p.id(),
            name: "P".into(),
            locator: DeviceLocator::default(),
            confirmation: ContactConfirmation::Pending {
                attempt_id: "d".repeat(32),
            },
            paused: false,
            locator_source: LocatorSource::PairingOffer,
            control_authorized: false,
            pairing_permission: PairingPermission::Control,
        })
        .unwrap();
        sa.revoke_grant(&p.id()).unwrap();
        assert!(sa.confirm_pin(&p.id(), &"d".repeat(32)).is_err());
        assert!(!sa.is_member(&p.id()).unwrap());
        pin_attempt(&sa, &p, 'e', PairingPermission::Control);
        assert!(sa.is_member(&p.id()).unwrap());
    }

    #[test]
    fn remote_member_tombstone_is_a_durable_high_water_mark() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (c, sc) = device("C");
        pair((&a, &sa), (&p, &sp));
        pair((&p, &sp), (&c, &sc));
        let old = bundle(&[&sp]);
        sa.merge_trust_bundle(&p.id(), &old).unwrap();
        assert!(sa.is_member(&c.id()).unwrap());
        sp.revoke_grant(&c.id()).unwrap();
        sa.merge_trust_bundle(&p.id(), &sp.export_trust_bundle(None).unwrap())
            .unwrap();
        assert!(!sa.is_member(&c.id()).unwrap());
        sa.merge_trust_bundle(&p.id(), &old).unwrap();
        assert!(!sa.is_member(&c.id()).unwrap());
        pin_attempt(&sp, &c, 'e', PairingPermission::Control);
        sa.merge_trust_bundle(&p.id(), &sp.export_trust_bundle(None).unwrap())
            .unwrap();
        assert!(sa.is_member(&c.id()).unwrap());
    }

    #[test]
    fn tombstone_learned_while_issuer_is_disconnected_prevents_later_old_grant_replay() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (c, sc) = device("C");
        let (d, _) = device("D");
        pair((&a, &sa), (&p, &sp));
        pair((&p, &sp), (&c, &sc));
        grant(&sc, &d, PairingPermission::Control);
        let old = sc.export_trust_bundle(None).unwrap();
        sc.revoke_grant(&d.id()).unwrap();
        sp.revoke_grant(&c.id()).unwrap();
        sa.merge_trust_bundle(&p.id(), &bundle(&[&sp, &sc]))
            .unwrap();
        assert!(!sa.is_member(&c.id()).unwrap());
        pin_attempt(&sp, &c, 'e', PairingPermission::Control);
        sa.merge_trust_bundle(&p.id(), &bundle(&[&sp])).unwrap();
        assert!(sa.is_member(&c.id()).unwrap());
        sa.merge_trust_bundle(&c.id(), &old).unwrap();
        assert!(!sa.is_member(&d.id()).unwrap());
    }

    #[test]
    fn capability_self_report_beats_forwarded_hints_and_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let a = DeviceIdentity::generate("A".into()).unwrap();
        let sa = DeviceStore::open(&path).unwrap();
        sa.initialize_trust(&a).unwrap();
        let (p, sp) = device("Phone");
        let (b, sb) = device("B");
        pair((&a, &sa), (&p, &sp));
        pair((&a, &sa), (&b, &sb));
        assert!(sa.contact(&p.id()).unwrap().control_authorized); // old direct target compatibility
        sp.set_local_execution_capability(false).unwrap();
        let self_bundle = sp.export_trust_bundle(Some(&descriptor(&p))).unwrap();
        sa.merge_trust_bundle(&p.id(), &self_bundle).unwrap();
        assert!(sa.is_member(&p.id()).unwrap());
        assert!(!sa.contact(&p.id()).unwrap().control_authorized);
        let mut hint = TrustBundle::default();
        hint.contacts.push(contact(&p, Some(true)));
        sa.merge_trust_bundle(&b.id(), &hint).unwrap();
        assert!(!sa.contact(&p.id()).unwrap().control_authorized);
        assert_eq!(
            sa.snapshot()
                .unwrap()
                .members
                .iter()
                .find(|m| m.peer_id == p.id())
                .unwrap()
                .can_execute,
            Some(false)
        );
        assert!(
            sa.export_trust_bundle(None)
                .unwrap()
                .contacts
                .iter()
                .any(|m| m.peer_id == p.id() && m.can_execute == Some(false))
        );
        drop(sa);
        let sa = DeviceStore::open(&path).unwrap();
        sa.initialize_trust(&a).unwrap();
        sa.merge_trust_bundle(&b.id(), &hint).unwrap();
        assert!(!sa.contact(&p.id()).unwrap().control_authorized);
        sp.set_local_execution_capability(true).unwrap();
        sa.merge_trust_bundle(
            &p.id(),
            &sp.export_trust_bundle(Some(&descriptor(&p))).unwrap(),
        )
        .unwrap();
        assert!(sa.contact(&p.id()).unwrap().control_authorized);
    }

    #[test]
    fn unknown_introduced_target_is_member_but_not_automatic_execution_target() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (c, sc) = device("C");
        pair((&a, &sa), (&p, &sp));
        pair((&p, &sp), (&c, &sc));
        let mut proof = bundle(&[&sp]);
        proof.contacts.push(contact(&c, None));
        sa.merge_trust_bundle(&p.id(), &proof).unwrap();
        assert!(sa.is_member(&c.id()).unwrap());
        assert!(!sa.contact(&c.id()).unwrap().control_authorized);
        assert_eq!(sa.introducers_for(&c.id()).unwrap(), vec![p.id()]);
        sa.merge_trust_bundle(
            &c.id(),
            &sc.export_trust_bundle(Some(&descriptor(&c))).unwrap(),
        )
        .unwrap();
        assert!(sa.contact(&c.id()).unwrap().control_authorized);
    }

    #[test]
    fn member_projection_includes_grant_only_members_without_fabricated_pairing_attempts() {
        let (a, sa) = device("A");
        let (p, _) = device("Phone");
        grant(&sa, &p, PairingPermission::Control);
        let snapshot = sa.snapshot().unwrap();
        assert!(snapshot.contacts.is_empty());
        assert_eq!(snapshot.members.len(), 1);
        assert_eq!(snapshot.members[0].name, "Phone");
        assert_eq!(snapshot.members[0].can_execute, None);
        assert_eq!(snapshot.incoming_grants[0].attempt_id, "a".repeat(32));
        sa.revoke_grant(&p.id()).unwrap();
        assert!(sa.snapshot().unwrap().members.is_empty());
        assert_eq!(
            sa.snapshot().unwrap().incoming_grants[0].attempt_id,
            "a".repeat(32)
        );
        assert!(
            sa.pairing_grant(&p.id(), &"a".repeat(32))
                .unwrap()
                .is_none()
        );
        assert!(!sa.allows_control(&a.id(), &p.id()).unwrap());
    }

    #[test]
    fn forget_exclusion_blocks_reintroduction_but_new_explicit_control_pairing_recovers() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (c, sc) = device("C");
        pair((&a, &sa), (&p, &sp));
        pair((&p, &sp), (&c, &sc));
        let mut proof = bundle(&[&sp]);
        proof.contacts.push(contact(&c, Some(true)));
        sa.merge_trust_bundle(&p.id(), &proof).unwrap();
        sa.forget_contact(&c.id()).unwrap();
        sa.merge_trust_bundle(&p.id(), &proof).unwrap();
        assert!(!sa.is_pinned(&c.id()).unwrap());
        assert!(!sa.is_member(&c.id()).unwrap());
        pin_attempt(&sa, &c, 'e', PairingPermission::TelemetryProfileRead);
        assert!(!sa.is_member(&c.id()).unwrap());
        pin_attempt(&sa, &c, 'f', PairingPermission::Control);
        assert!(sa.is_member(&c.id()).unwrap());
    }

    #[test]
    fn locator_priority_pause_pending_pairing_and_invalid_bundle_are_preserved() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (c, sc) = device("C");
        pair((&a, &sa), (&p, &sp));
        pair((&p, &sp), (&c, &sc));
        sa.save_pin(DeviceContact {
            peer_id: c.id(),
            name: "Pending".into(),
            locator: DeviceLocator::default(),
            confirmation: ContactConfirmation::Pending {
                attempt_id: "d".repeat(32),
            },
            paused: false,
            locator_source: LocatorSource::PairingOffer,
            control_authorized: false,
            pairing_permission: PairingPermission::Control,
        })
        .unwrap();
        let mut proof = bundle(&[&sp]);
        proof.contacts.push(contact(&c, Some(true)));
        sa.merge_trust_bundle(&p.id(), &proof).unwrap();
        assert!(sa.pending_pairing(&c.id()).unwrap().is_some());
        assert!(matches!(
            sa.contact(&c.id()).unwrap().confirmation,
            ContactConfirmation::Confirmed { .. }
        ));
        let locator = DeviceLocator {
            direct_addresses: vec!["127.0.0.1:12345".parse().unwrap()],
            ..Default::default()
        };
        sa.update_locator(&c.id(), "C authenticated".into(), locator.clone())
            .unwrap();
        sa.pause(&c.id(), true).unwrap();
        sa.import_contact(&c.id(), "Stale C".into(), DeviceLocator::default(), &p.id())
            .unwrap();
        assert_eq!(
            sa.contact(&c.id()).unwrap().locator.direct_addresses,
            locator.direct_addresses
        );
        assert_eq!(sa.contact(&c.id()).unwrap().name, "C authenticated");
        assert!(sa.introducers_for(&c.id()).unwrap().is_empty());
        let before = sa.version();
        proof.contacts[0]
            .locator
            .relay_urls
            .push("file:///invalid".into());
        assert!(sa.merge_trust_bundle(&p.id(), &proof).is_err());
        assert_eq!(sa.version(), before);
    }

    #[test]
    fn descriptor_export_duplicate_gossip_and_conflicts_are_atomic() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        pair((&a, &sa), (&p, &sp));
        let proof = sp.export_trust_bundle(Some(&descriptor(&p))).unwrap();
        assert!(
            proof
                .contacts
                .iter()
                .any(|c| c.peer_id == p.id() && c.can_execute == Some(true))
        );
        sa.merge_trust_bundle(&p.id(), &proof).unwrap();
        let version = sa.version();
        sa.merge_trust_bundle(&p.id(), &proof).unwrap();
        assert_eq!(sa.version(), version);
        let mut bad = descriptor(&p);
        bad.endpoint_id = a.id();
        assert!(sp.export_trust_bundle(Some(&bad)).is_err());
        let mut conflict = TrustBundle::default();
        conflict
            .memberships
            .push(MembershipStatement::issue(&p.key, &a.id(), 1, true).unwrap());
        assert!(sa.merge_trust_bundle(&p.id(), &conflict).is_err());
        assert_eq!(sa.version(), version);
        let mut oversized = proof.clone();
        oversized.memberships =
            vec![proof.memberships[0].clone(); crate::trust::MAX_STATEMENTS + 1];
        assert!(matches!(
            sa.merge_trust_bundle(&p.id(), &oversized),
            Err(RemoteError::Capacity)
        ));
        assert_eq!(sa.version(), version);
    }

    #[test]
    fn v5_migration_signs_only_completed_local_direct_control_and_keeps_exact_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let (local, old) = device("Local");
        let (direct, sd) = device("Direct");
        let (introduced, _) = device("Introduced");
        let (profile, _) = device("Profile");
        let (incoming, _) = device("Incoming");
        pair((&local, &old), (&direct, &sd));
        grant(&old, &incoming, PairingPermission::Control);
        pin_attempt(&old, &profile, 'c', PairingPermission::TelemetryProfileRead);
        let edge = TrustStatement::issue(
            &introduced.key,
            &direct.id(),
            PairingPermission::Control,
            1,
            false,
        )
        .unwrap();
        let mut state = old.state.lock().unwrap().clone();
        state.trust.paired_controls.clear(); // first signed remote direct grant consumed the old compatibility pin
        let remote = TrustStatement::issue(
            &direct.key,
            &local.id(),
            PairingPermission::Control,
            1,
            false,
        )
        .unwrap();
        state
            .trust
            .statements
            .insert(remote.grant_id.clone(), remote);
        state.contacts.insert(
            introduced.id(),
            DeviceContact {
                peer_id: introduced.id(),
                name: "Introduced".into(),
                locator: DeviceLocator::default(),
                confirmation: ContactConfirmation::Confirmed {
                    attempt_id: edge.grant_id[..32].into(),
                },
                paused: false,
                locator_source: LocatorSource::AuthenticatedPeer,
                control_authorized: true,
                pairing_permission: PairingPermission::Control,
            },
        );
        state.trust.introduced_via.insert(
            introduced.id(),
            std::collections::BTreeSet::from([direct.id()]),
        );
        state.trust.statements.insert(edge.grant_id.clone(), edge);
        let bytes = serde_json::to_vec(&legacy_v5(&state)).unwrap();
        atomic_write(&path, &bytes).unwrap();
        let store = DeviceStore::open(&path).unwrap();
        assert_eq!(
            std::fs::read(path.with_file_name("state.before-v5.json")).unwrap(),
            bytes
        );
        store.initialize_trust(&local).unwrap();
        let proof = store.export_trust_bundle(None).unwrap();
        assert_eq!(proof.memberships.len(), 2);
        assert!(
            proof
                .memberships
                .iter()
                .all(|s| s.issuer == local.id() && s.revision == 1 && !s.revoked)
        );
        assert!(store.is_member(&direct.id()).unwrap());
        assert!(store.is_member(&incoming.id()).unwrap());
        assert!(!store.is_member(&introduced.id()).unwrap());
        assert!(!store.is_member(&profile.id()).unwrap());
        assert!(!store.contact(&introduced.id()).unwrap().control_authorized);
        assert!(store.contact(&direct.id()).unwrap().control_authorized);
        drop(store);
        let reopened = DeviceStore::open(&path).unwrap();
        reopened.initialize_trust(&local).unwrap();
        assert_eq!(
            reopened.export_trust_bundle(None).unwrap().memberships,
            proof.memberships
        );
    }

    #[test]
    fn ambiguous_v5_control_then_telemetry_history_is_preserved_and_explicit_repair_recovers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let (a, sa) = device("A");
        let (b, sb) = device("B");
        pair((&a, &sa), (&b, &sb));
        sa.merge_trust_bundle(
            &b.id(),
            &sb.export_trust_bundle(Some(&descriptor(&b))).unwrap(),
        )
        .unwrap();
        pin_attempt(&sa, &b, 'c', PairingPermission::TelemetryProfileRead);
        let mut state = sa.state.lock().unwrap().clone();
        state.trust.paired_controls.clear();
        let bytes = serde_json::to_vec(&legacy_v5(&state)).unwrap();
        atomic_write(&path, &bytes).unwrap();
        let store = DeviceStore::open(&path).unwrap();
        store.initialize_trust(&a).unwrap();
        assert!(!store.is_member(&b.id()).unwrap());
        assert_eq!(
            store.contact(&b.id()).unwrap().pairing_permission,
            PairingPermission::TelemetryProfileRead
        );
        assert!(
            store
                .export_trust_bundle(None)
                .unwrap()
                .statements
                .iter()
                .any(|s| s.issuer == b.id())
        );
        assert_eq!(
            std::fs::read(path.with_file_name("state.before-v5.json")).unwrap(),
            bytes
        );
        pin_attempt(&store, &b, 'd', PairingPermission::Control);
        assert!(store.is_member(&b.id()).unwrap());
        assert!(store.contact(&b.id()).unwrap().control_authorized);
    }

    #[test]
    fn v5_revoke_and_forget_survive_migration_and_cannot_be_reversed_by_legacy_statements() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        let (c, sc) = device("C");
        pair((&a, &sa), (&p, &sp));
        pair((&p, &sp), (&c, &sc));
        grant(&sa, &c, PairingPermission::Control);
        sa.revoke_grant(&c.id()).unwrap();
        let mut state = sa.state.lock().unwrap().clone();
        state.grants.remove(&c.id());
        state.trust.forgotten_contacts.clear(); // v5 Control revoke had tombstone but no forget exclusion
        let bytes = serde_json::to_vec(&legacy_v5(&state)).unwrap();
        atomic_write(&path, &bytes).unwrap();
        let store = DeviceStore::open(&path).unwrap();
        store.initialize_trust(&a).unwrap();
        store.merge_trust_bundle(&p.id(), &bundle(&[&sp])).unwrap();
        assert!(!store.is_member(&c.id()).unwrap());
        assert!(store.is_member(&p.id()).unwrap());
    }

    #[test]
    fn v5_forgotten_peer_with_incoming_grant_is_migrated_without_restoring_membership() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let (a, sa) = device("A");
        let (p, _) = device("P");
        grant(&sa, &p, PairingPermission::Control);
        let mut state = sa.state.lock().unwrap().clone();
        state.trust.forgotten_contacts.insert(p.id());
        // v5 forget removed discovery, but left the incoming grant and its signature active.
        let bytes = serde_json::to_vec(&legacy_v5(&state)).unwrap();
        atomic_write(&path, &bytes).unwrap();
        let store = DeviceStore::open(&path).unwrap();
        store.initialize_trust(&a).unwrap();
        assert!(!store.is_member(&p.id()).unwrap());
        assert!(store.snapshot().unwrap().members.is_empty());
        assert_eq!(
            std::fs::read(path.with_file_name("state.before-v5.json")).unwrap(),
            bytes
        );
        drop(store);
        let store = DeviceStore::open(&path).unwrap();
        store.initialize_trust(&a).unwrap();
        assert!(!store.is_member(&p.id()).unwrap());
    }

    #[test]
    fn unknown_or_corrupt_v5_v6_never_rewrites_source_or_creates_backup() {
        let (a, sa) = device("A");
        let (p, _) = device("P");
        grant(&sa, &p, PairingPermission::Control);
        let current = serde_json::to_value(sa.state.lock().unwrap().clone()).unwrap();
        let legacy = legacy_v5(&sa.state.lock().unwrap());
        let mut unknown = legacy.clone();
        unknown["trust"]["membership_initialized"] = serde_json::json!(true);
        let mut corrupt = current.clone();
        let signature_byte = &mut corrupt["trust"]["memberships"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["signature"][0];
        *signature_byte = serde_json::json!(signature_byte.as_u64().unwrap() ^ 1);
        let mut inconsistent = current.clone();
        inconsistent["trust"]["direct_members"] = serde_json::json!([]);
        let mut future = current;
        future["version"] = serde_json::json!(7);
        for json in [unknown, corrupt, inconsistent, future] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("state.json");
            let bytes = serde_json::to_vec(&json).unwrap();
            atomic_write(&path, &bytes).unwrap();
            assert!(
                DeviceStore::open(&path).is_err(),
                "invalid state for {}",
                a.id()
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn foreign_membership_flood_does_not_consume_local_consent_capacity() {
        let (a, sa) = device("A");
        let (p, sp) = device("P");
        pair((&a, &sa), (&p, &sp));
        let mut foreign = TrustBundle::default();
        for _ in 0..=crate::trust::MAX_FOREIGN_STATEMENTS {
            let peer = iroh::SecretKey::generate().public().to_string();
            foreign
                .memberships
                .push(MembershipStatement::issue(&p.key, &peer, 1, false).unwrap());
        }
        assert_eq!(
            sa.merge_trust_bundle(&p.id(), &foreign),
            Err(RemoteError::Capacity)
        );
        assert_eq!(sa.export_trust_bundle(None).unwrap().memberships.len(), 1);
        let (c, _) = device("C");
        grant(&sa, &c, PairingPermission::Control);
        assert!(sa.is_member(&c.id()).unwrap());
    }

    #[test]
    fn every_statement_field_and_signature_is_authenticated() {
        let (a, _) = device("A");
        let (b, sb) = device("B");
        let (c, _) = device("C");
        grant(&sb, &a, PairingPermission::Control);
        let statement = sb.export_trust_bundle(None).unwrap().statements.remove(0);
        statement.verify().unwrap();
        for field in 0..9 {
            let mut changed = statement.clone();
            match field {
                0 => changed.version += 1,
                1 => changed.issuer = c.id(),
                2 => changed.subject = c.id(),
                3 => changed.permission = PairingPermission::TelemetryProfileRead,
                4 => changed.grant_id = "0".repeat(64),
                5 => changed.revision += 1,
                6 => changed.revoked = true,
                7 => changed.signature[0] ^= 1,
                _ => {
                    changed.signature.pop();
                }
            }
            assert!(changed.verify().is_err(), "field {field}");
        }
        assert_eq!(statement.issuer, b.id());
    }

    #[test]
    fn pending_operation_blocks_forgetting_without_creating_an_exclusion() {
        let (_, sa) = device("A");
        let (c, _) = device("C");
        pin(&sa, &c);
        sa.record_pending(serde_json::from_value(serde_json::json!({
            "peer_id":c.id(), "request_id":{"incarnation":"a".repeat(32),"client":"b".repeat(32),"sequence":1},
            "view":"synthetic-view", "execution":{"expected_revision":null}, "kind":"readComposerOwner",
            "runtime_id":null,"conversation_id":null,"status":"recorded"
        })).unwrap()).unwrap();
        assert_eq!(
            sa.forget_contact(&c.id()),
            Err(RemoteError::PendingRequests)
        );
        assert!(sa.is_pinned(&c.id()).unwrap());
        assert!(
            !sa.state
                .lock()
                .unwrap()
                .trust
                .forgotten_contacts
                .contains(&c.id())
        );
    }

    #[test]
    fn v4_migration_preserves_bytes_and_signs_only_local_incoming_decisions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let local = DeviceIdentity::generate("Local".into()).unwrap();
        let (a, _) = device("A");
        let (b, _) = device("B");
        let old = DeviceStore::memory();
        pin(&old, &a);
        grant(&old, &a, PairingPermission::Control);
        grant(&old, &b, PairingPermission::TelemetryProfileRead);
        let mut json = serde_json::to_value(old.state.lock().unwrap().clone()).unwrap();
        json["version"] = serde_json::json!(4);
        json.as_object_mut().unwrap().remove("trust");
        let bytes = serde_json::to_vec(&json).unwrap();
        atomic_write(&path, &bytes).unwrap();
        let migrated = DeviceStore::open(&path).unwrap();
        assert_eq!(
            std::fs::read(path.with_file_name("state.before-v4.json")).unwrap(),
            bytes
        );
        assert!(migrated.export_trust_bundle(None).is_err());
        let mut ordinary = serde_json::to_value(migrated.state.lock().unwrap().clone()).unwrap();
        ordinary.as_object_mut().unwrap().remove("trust");
        ordinary["version"] = serde_json::json!(4);
        assert_eq!(ordinary, json);
        migrated.initialize_trust(&local).unwrap();
        let bundle = migrated.export_trust_bundle(None).unwrap();
        assert_eq!(bundle.statements.len(), 2);
        assert!(
            bundle
                .statements
                .iter()
                .all(|s| s.issuer == local.id() && s.revision == 1 && !s.revoked)
        );
        assert!(migrated.is_member(&a.id()).unwrap());
        assert!(!migrated.is_member(&b.id()).unwrap());
        assert!(migrated.contact(&a.id()).unwrap().control_authorized); // local pin, not a forgeable proof
        drop(migrated);
        let reopened = DeviceStore::open(&path).unwrap();
        assert_eq!(
            reopened.initialize_trust(&a),
            Err(RemoteError::PeerMismatch)
        );
        reopened.initialize_trust(&local).unwrap();
        assert_eq!(
            reopened.export_trust_bundle(None).unwrap().statements,
            bundle.statements
        );
    }

    #[test]
    fn failed_revocation_write_disables_old_authority_until_store_is_reopened() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        let path = original.join("state.json");
        let local = DeviceIdentity::generate("Local".into()).unwrap();
        let (a, _) = device("A");
        let store = DeviceStore::open(&path).unwrap();
        store.initialize_trust(&local).unwrap();
        grant(&store, &a, PairingPermission::Control);
        let saved = directory.path().join("saved");
        std::fs::rename(&original, &saved).unwrap();
        assert_eq!(store.revoke_grant(&a.id()), Err(RemoteError::StateStorage));
        assert_eq!(
            store.allows_control(&local.id(), &a.id()),
            Err(RemoteError::StateStorage)
        );
        assert!(matches!(
            store.grant(&a.id()),
            Err(RemoteError::StateStorage)
        ));
        assert!(matches!(
            store.profile_grant(&a.id()),
            Err(RemoteError::StateStorage)
        ));
        assert!(store.export_trust_bundle(None).is_err());
        assert!(store.initialize_trust(&local).is_err());
        let reopened = DeviceStore::open(&saved.join("state.json")).unwrap();
        reopened.initialize_trust(&local).unwrap();
        reopened.revoke_grant(&a.id()).unwrap();
        assert!(!reopened.allows_control(&local.id(), &a.id()).unwrap());
    }

    #[test]
    fn v4_backup_failure_preserves_source_and_bundle_capacity_rejects_before_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let mut json = serde_json::to_value(StoredState::default()).unwrap();
        json["version"] = serde_json::json!(4);
        json.as_object_mut().unwrap().remove("trust");
        let bytes = serde_json::to_vec(&json).unwrap();
        atomic_write(&path, &bytes).unwrap();
        std::fs::create_dir(path.with_file_name("state.before-v4.json")).unwrap();
        std::fs::create_dir(
            path.with_file_name(format!("state.before-v4-{:x}.json", Sha256::digest(&bytes))),
        )
        .unwrap();
        assert!(DeviceStore::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        let (a, sa) = device("A");
        let (b, sb) = device("B");
        pin(&sa, &b);
        grant(&sb, &a, PairingPermission::Control);
        let mut proof = sb.export_trust_bundle(None).unwrap();
        proof.statements = vec![proof.statements[0].clone(); crate::trust::MAX_STATEMENTS + 1];
        assert_eq!(
            sa.merge_trust_bundle(&b.id(), &proof),
            Err(RemoteError::Capacity)
        );
        assert!(sa.export_trust_bundle(None).unwrap().statements.is_empty());
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    fn state_path() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote-state.json");
        (directory, path)
    }

    fn empty_v3_state() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version":3,
            "contacts":{},
            "grants":{},
            "pending":[],
            "reset_notice":null,
            "legacy_reset_recorded":false
        }))
        .unwrap()
    }

    #[test]
    fn unknown_future_version_is_not_reset_or_overwritten() {
        let (_directory, path) = state_path();
        let bytes = br#"{"version":99,"synthetic":"preserve"}"#;
        atomic_write(&path, bytes).unwrap();
        assert!(DeviceStore::open(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn malformed_or_unknown_state_is_rejected_without_rewrite() {
        for bytes in [
            br#"{"version":4,"contacts":[]"#.as_slice(),
            br#"{"version":4,"contacts":{},"future":"preserve"}"#,
        ] {
            let (_directory, path) = state_path();
            atomic_write(&path, bytes).unwrap();
            assert!(DeviceStore::open(&path).is_err());
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn v3_with_v4_fields_is_rejected_without_overwrite_or_backup() {
        let mut profile_grant =
            serde_json::from_slice::<serde_json::Value>(&empty_v3_state()).unwrap();
        profile_grant["profile_grants"] = serde_json::json!({"synthetic": {}});

        let mut pairing_contact =
            serde_json::from_slice::<serde_json::Value>(&empty_v3_state()).unwrap();
        pairing_contact["contacts"] = serde_json::json!({
            "synthetic": {
                "control_authorized": false,
                "pairing_permission": "telemetryProfileRead"
            }
        });

        for value in [profile_grant, pairing_contact] {
            let (_directory, path) = state_path();
            let source = serde_json::to_vec(&value).unwrap();
            atomic_write(&path, &source).unwrap();

            assert!(DeviceStore::open(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), source);
            assert!(
                !path
                    .with_file_name("remote-state.before-beta-v3.json")
                    .exists()
            );
        }
    }

    #[test]
    fn interrupted_backup_files_do_not_block_v3_migration_retry() {
        let (_directory, path) = state_path();
        let source = empty_v3_state();
        atomic_write(&path, &source).unwrap();

        let previous_backup = path.with_file_name("remote-state.before-beta-v3.json");
        atomic_write(&previous_backup, b"partial backup").unwrap();
        let abandoned_temporary = path.with_file_name(".remote-state-backup-interrupted.tmp");
        std::fs::write(&abandoned_temporary, b"partial temporary backup").unwrap();

        drop(DeviceStore::open(&path).unwrap());

        assert_eq!(std::fs::read(&previous_backup).unwrap(), b"partial backup");
        assert_eq!(
            std::fs::read(&abandoned_temporary).unwrap(),
            b"partial temporary backup"
        );
        let digest = Sha256::digest(&source);
        let mut suffix = String::new();
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut suffix, "{byte:02x}").unwrap();
        }
        let complete_backup =
            path.with_file_name(format!("remote-state.before-beta-v3-{suffix}.json"));
        assert_eq!(std::fs::read(complete_backup).unwrap(), source);
        let migrated: StoredState = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(migrated.version, 6);
    }

    #[test]
    fn known_v3_state_is_backed_up_and_migrated() {
        let (_directory, path) = state_path();
        let peer = crate::identity::DeviceIdentity::generate("Synthetic Peer".into())
            .unwrap()
            .id();
        let incarnation = "1".repeat(32);
        let source = serde_json::to_vec(&serde_json::json!({
            "version":3,
            "contacts":{
                peer.clone(): {
                    "peer_id":peer,
                    "name":"Synthetic Peer",
                    "locator":{"relay_urls":[],"direct_addresses":[],"lookup_urls":[]},
                    "confirmation":{"state":"confirmed","attempt_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                    "paused":false,
                    "locator_source":"pairingOffer"
                }
            },
            "grants":{
                peer.clone():{"peer_id":peer.clone(),"name":"Synthetic Peer","attempt_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
            },
            "pending":[{
                "peer_id":peer.clone(),
                "request_id":{"incarnation":incarnation,"client":"cccccccccccccccccccccccccccccccc","sequence":1},
                "view":"synthetic-view","execution":{"expected_revision":null},"kind":"readComposerOwner",
                "runtime_id":null,"conversation_id":null,"status":"recorded"
            }],
            "reset_notice":{"id":"dddddddddddddddddddddddddddddddd"},
            "legacy_reset_recorded":true
        }))
        .unwrap();
        atomic_write(&path, &source).unwrap();
        let store = DeviceStore::open(&path).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.contacts.len(), 1);
        assert!(snapshot.contacts[0].control_authorized);
        assert_eq!(
            snapshot.contacts[0].pairing_permission,
            PairingPermission::Control
        );
        assert_eq!(snapshot.incoming_grants.len(), 1);
        assert_eq!(snapshot.pending.len(), 1);
        assert_eq!(
            snapshot.reset_notice.as_ref().unwrap().id,
            "dddddddddddddddddddddddddddddddd"
        );
        drop(store);
        assert_eq!(
            std::fs::read(path.with_file_name("remote-state.before-beta-v3.json")).unwrap(),
            source
        );
        let migrated_bytes = std::fs::read(&path).unwrap();
        let migrated: StoredState = serde_json::from_slice(&migrated_bytes).unwrap();
        assert_eq!(migrated.version, 6);
        assert_eq!(migrated.contacts.len(), 1);
        assert_eq!(migrated.grants.len(), 1);
        assert_eq!(migrated.pending.len(), 1);
        assert!(migrated.pending_pairings.is_empty());
        assert!(migrated.profile_grants.is_empty());
        drop(DeviceStore::open(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), migrated_bytes);
    }

    #[test]
    fn current_state_is_read_without_rewrite_and_missing_state_initializes() {
        let (_directory, path) = state_path();
        let bytes = serde_json::to_vec(&StoredState::default()).unwrap();
        atomic_write(&path, &bytes).unwrap();
        let store = DeviceStore::open(&path).unwrap();
        assert!(store.snapshot().unwrap().contacts.is_empty());
        drop(store);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        let (_directory, path) = state_path();
        let store = DeviceStore::open(&path).unwrap();
        assert!(store.snapshot().unwrap().contacts.is_empty());
        let initialized: StoredState =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(initialized.version, 6);
    }
}
