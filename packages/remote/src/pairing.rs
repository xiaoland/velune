//! Directional, locally approved pairing bound to a single authenticated TLS connection.
use crate::{
    identity::{random_bytes, random_id, validate_name},
    paths::{ConnectionSlot, ObservedConnection},
    protocol::{Reply, Request},
    service::ServiceState,
    *,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const EXPORTER_LABEL: &[u8] = b"EXPORTER-Velune-Pairing-v2";
const OFFER_LIMIT: usize = 1024;
const FOLLOW_INTERVAL: Duration = Duration::from_secs(2);
const FOLLOW_LIMIT: Duration = Duration::from_secs(600);

/// Only explicit encode exposes the single-use invitation secret. This value must not be logged.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingOffer {
    version: u16,
    purpose: String,
    pub issuer: String,
    pub invite_id: String,
    token: [u8; 32],
    pub expires_at_unix_seconds: u64,
    pub name: String,
    pub locator: DeviceLocator,
}
impl PairingOffer {
    pub fn encode(&self) -> Result<String> {
        let encoded = serde_json::to_string(self).map_err(|_| RemoteError::InvalidInvitation)?;
        if encoded.len() > OFFER_LIMIT {
            return Err(RemoteError::FrameTooLarge);
        }
        Ok(encoded)
    }
    pub fn decode(encoded: &str) -> Result<Self> {
        if encoded.len() > OFFER_LIMIT {
            return Err(RemoteError::FrameTooLarge);
        }
        let offer: Self =
            serde_json::from_str(encoded).map_err(|_| RemoteError::InvalidInvitation)?;
        if offer.version != PROTOCOL_VERSION || offer.purpose != "velunePairing" {
            return Err(RemoteError::IncompatibleVersion);
        }
        crate::store::parse_peer(&offer.issuer)?;
        crate::protocol::validate_id(&offer.invite_id)?;
        validate_name(&offer.name)?;
        offer.locator.validate()?;
        if offer.locator.relay_urls.len() > 1 || offer.locator.direct_addresses.len() > 2 {
            return Err(RemoteError::InvalidInvitation);
        }
        Ok(offer)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PairingPhase {
    AwaitingReady,
    AwaitingApproval,
    Granted,
    Rejected,
    Expired,
    Disconnected,
}
/// Verification codes are derived locally; they are never accepted from a peer's response.
#[derive(Clone, Serialize, Deserialize)]
pub struct PairingStatus {
    pub attempt_id: String,
    pub peer_id: String,
    pub name: String,
    pub phase: PairingPhase,
    pub verification_code: Option<String>,
    pub expires_at_unix_seconds: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PairingWireState {
    pub(crate) attempt_id: String,
    pub(crate) phase: PairingPhase,
    pub(crate) challenge: Option<[u8; 32]>,
    pub(crate) expires_at_unix_seconds: u64,
}
struct Window {
    offer: PairingOffer,
    deadline: Instant,
    challenges: u8,
}
struct Candidate {
    status: PairingStatus,
    challenge: [u8; 32],
    deadline: Instant,
    generation: u64,
    connection_id: u64,
    connection: Weak<ConnectionSlot>,
}
#[derive(Default)]
pub(crate) struct PairingRegistry {
    window: Option<Window>,
    candidates: BTreeMap<String, Candidate>,
}
impl PairingRegistry {
    pub(crate) fn revoke(&mut self, peer: &str) {
        self.candidates
            .retain(|_, candidate| candidate.status.peer_id != peer);
    }
    pub(crate) fn invalidate(&mut self, phase: PairingPhase) {
        self.window = None;
        for candidate in self.candidates.values_mut() {
            if matches!(
                candidate.status.phase,
                PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
            ) {
                candidate.status.phase = phase;
            }
        }
    }
    fn refresh(&mut self) -> bool {
        let mut changed = false;
        let now = Instant::now();
        for candidate in self.candidates.values_mut() {
            if matches!(
                candidate.status.phase,
                PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
            ) {
                if candidate.deadline <= now {
                    candidate.status.phase = PairingPhase::Expired;
                    changed = true;
                } else if candidate
                    .connection
                    .upgrade()
                    .is_none_or(|slot| slot.status().closed)
                {
                    candidate.status.phase = PairingPhase::Disconnected;
                    changed = true;
                }
            }
        }
        changed
    }
    pub(crate) fn statuses(&mut self) -> (Vec<PairingStatus>, bool) {
        let changed = self.refresh();
        (
            self.candidates
                .values()
                .map(|candidate| candidate.status.clone())
                .collect(),
            changed,
        )
    }
    pub(crate) fn disconnected(&mut self, connection_id: u64) {
        for candidate in self
            .candidates
            .values_mut()
            .filter(|candidate| candidate.connection_id == connection_id)
        {
            if matches!(
                candidate.status.phase,
                PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
            ) {
                candidate.status.phase = PairingPhase::Disconnected;
            }
        }
    }
}
pub(crate) struct OutgoingPairing {
    pub(crate) client: Arc<RemoteClient>,
    pub(crate) status: PairingStatus,
    invite_id: String,
}

fn unix_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| RemoteError::Unavailable)
}
fn raw_id(id: &str) -> Result<[u8; 16]> {
    crate::protocol::validate_id(id)?;
    let mut bytes = [0; 16];
    for (index, pair) in id.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        bytes[index] = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| RemoteError::Protocol)?,
            16,
        )
        .map_err(|_| RemoteError::Protocol)?;
    }
    Ok(bytes)
}
fn verification_code(
    connection: &ObservedConnection,
    issuer: &str,
    controller: &str,
    invite: &str,
    attempt: &str,
    challenge: [u8; 32],
) -> Result<String> {
    let issuer = crate::store::parse_peer(issuer)?;
    let controller = crate::store::parse_peer(controller)?;
    let mut context = [0u8; 131];
    context[0..2].copy_from_slice(&2u16.to_be_bytes());
    context[2..34].copy_from_slice(issuer.as_bytes());
    context[34..66].copy_from_slice(controller.as_bytes());
    context[66..82].copy_from_slice(&raw_id(invite)?);
    context[82..98].copy_from_slice(&raw_id(attempt)?);
    context[98..130].copy_from_slice(&challenge);
    context[130] = 1;
    let mut output = [0; 32];
    connection
        .export_keying_material(&mut output, EXPORTER_LABEL, &context)
        .map_err(|_| RemoteError::Protocol)?;
    Ok(format!(
        "{:08}",
        u64::from_be_bytes(output[..8].try_into().map_err(|_| RemoteError::Protocol)?)
            % 100_000_000
    ))
}
impl ServiceState {
    pub(crate) fn pairing_disconnected(&self, id: u64) {
        if let Ok(mut access) = self.access() {
            access.pairing.disconnected(id);
            self.store.touch();
        }
    }
    pub(crate) fn dispatch_pairing(
        &self,
        generation: u64,
        connection: &ObservedConnection,
        slot: &Arc<ConnectionSlot>,
        request: Request,
    ) -> Reply {
        self.try_dispatch_pairing(generation, connection, slot, request)
            .unwrap_or_else(Reply::Error)
    }
    fn try_dispatch_pairing(
        &self,
        generation: u64,
        connection: &ObservedConnection,
        slot: &Arc<ConnectionSlot>,
        request: Request,
    ) -> Result<Reply> {
        let peer = connection.remote_id().to_string();
        let mut access = self.access()?;
        Self::check_generation(&access, generation)?;
        if !access.ingress {
            return Err(RemoteError::Closed);
        }
        if access.pairing.refresh() {
            self.store.touch();
        }
        if let Request::PairBegin {
            invite_id,
            token,
            name,
        } = request
        {
            validate_name(&name)?;
            if let Some(candidate) = access.pairing.candidates.values().find(|candidate| {
                candidate.connection_id == slot.status().connection_id
                    && matches!(
                        candidate.status.phase,
                        PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
                    )
            }) {
                return Ok(Reply::Pairing(PairingWireState {
                    attempt_id: candidate.status.attempt_id.clone(),
                    phase: candidate.status.phase,
                    challenge: Some(candidate.challenge),
                    expires_at_unix_seconds: candidate.status.expires_at_unix_seconds,
                }));
            }
            let window = access
                .pairing
                .window
                .as_mut()
                .ok_or(RemoteError::InvalidInvitation)?;
            if window.deadline <= Instant::now() {
                return Err(RemoteError::ExpiredInvitation);
            }
            if window.offer.invite_id != invite_id
                || window
                    .offer
                    .token
                    .iter()
                    .zip(token)
                    .fold(0u8, |difference, (left, right)| difference | (left ^ right))
                    != 0
            {
                return Err(RemoteError::InvalidInvitation);
            }
            if window.challenges >= 5 {
                return Err(RemoteError::Capacity);
            }
            window.challenges += 1;
            let attempt_id = random_id()?;
            let challenge = random_bytes()?;
            let status = PairingStatus {
                verification_code: Some(verification_code(
                    connection,
                    &self.identity.id(),
                    &peer,
                    &invite_id,
                    &attempt_id,
                    challenge,
                )?),
                attempt_id: attempt_id.clone(),
                peer_id: peer,
                name,
                phase: PairingPhase::AwaitingReady,
                expires_at_unix_seconds: window.offer.expires_at_unix_seconds,
            };
            let response = PairingWireState {
                attempt_id: attempt_id.clone(),
                phase: status.phase,
                challenge: Some(challenge),
                expires_at_unix_seconds: status.expires_at_unix_seconds,
            };
            let deadline = window.deadline;
            access.pairing.candidates.insert(
                attempt_id,
                Candidate {
                    status,
                    challenge,
                    deadline,
                    generation,
                    connection_id: slot.status().connection_id,
                    connection: Arc::downgrade(slot),
                },
            );
            self.store.touch();
            return Ok(Reply::Pairing(response));
        }
        let (attempt_id, ready, cancel) = match request {
            Request::PairReady { attempt_id } => (attempt_id, true, false),
            Request::PairStatus { attempt_id } => (attempt_id, false, false),
            Request::PairCancel { attempt_id } => (attempt_id, false, true),
            _ => return Err(RemoteError::Protocol),
        };
        crate::protocol::validate_id(&attempt_id)?;
        if let Some(grant) = self.store.grant(&peer)?
            && grant.attempt_id == attempt_id
        {
            return Ok(Reply::Pairing(PairingWireState {
                attempt_id,
                phase: PairingPhase::Granted,
                challenge: None,
                expires_at_unix_seconds: 0,
            }));
        }
        let candidate = access
            .pairing
            .candidates
            .get_mut(&attempt_id)
            .ok_or(RemoteError::PairingUnavailable)?;
        if candidate.status.peer_id != peer {
            return Err(RemoteError::PairingUnavailable);
        }
        if candidate.status.phase == PairingPhase::Granted {
            return Err(RemoteError::PairingUnavailable);
        }
        if matches!(
            candidate.status.phase,
            PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
        ) && (candidate.generation != generation
            || candidate.connection_id != slot.status().connection_id)
        {
            candidate.status.phase = PairingPhase::Disconnected;
        }
        if ready && candidate.status.phase == PairingPhase::AwaitingReady {
            candidate.status.phase = PairingPhase::AwaitingApproval;
        }
        if cancel
            && matches!(
                candidate.status.phase,
                PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
            )
        {
            candidate.status.phase = PairingPhase::Rejected;
        }
        self.store.touch();
        Ok(Reply::Pairing(PairingWireState {
            attempt_id,
            phase: candidate.status.phase,
            challenge: None,
            expires_at_unix_seconds: candidate.status.expires_at_unix_seconds,
        }))
    }
}
impl RemoteAccessService {
    pub fn create_invitation(&self, lifetime: Duration) -> Result<PairingOffer> {
        if lifetime.is_zero() || lifetime > Duration::from_secs(600) {
            return Err(RemoteError::Configuration);
        }
        let mut access = self.state.access()?;
        if access.closed || !access.ingress {
            return Err(RemoteError::Closed);
        }
        let endpoint = self.descriptor()?;
        let offer = PairingOffer {
            version: PROTOCOL_VERSION,
            purpose: "velunePairing".into(),
            issuer: endpoint.endpoint_id,
            invite_id: random_id()?,
            token: random_bytes()?,
            expires_at_unix_seconds: unix_seconds()? + lifetime.as_secs(),
            name: endpoint.name,
            locator: DeviceLocator {
                relay_urls: endpoint.relay_urls.into_iter().take(1).collect(),
                direct_addresses: endpoint.direct_addresses.into_iter().take(2).collect(),
                lookup_urls: endpoint.lookup_urls.into_iter().take(1).collect(),
            },
        };
        offer.encode()?;
        access.pairing.invalidate(PairingPhase::Expired);
        access.pairing.candidates.clear();
        access.pairing.window = Some(Window {
            offer: offer.clone(),
            deadline: Instant::now() + lifetime,
            challenges: 0,
        });
        self.state.store.touch();
        Ok(offer)
    }
    pub fn approve_pairing(&self, attempt: &str, peer: &str) -> Result<PairingStatus> {
        let mut access = self.state.access()?;
        if access.closed || !access.ingress {
            return Err(RemoteError::Closed);
        }
        if let Some(grant) = self.state.store.grant(peer)?
            && grant.attempt_id == attempt
        {
            return Ok(PairingStatus {
                attempt_id: attempt.into(),
                peer_id: peer.into(),
                name: grant.name,
                phase: PairingPhase::Granted,
                verification_code: access
                    .pairing
                    .candidates
                    .get(attempt)
                    .and_then(|candidate| candidate.status.verification_code.clone()),
                expires_at_unix_seconds: 0,
            });
        }
        if access.pairing.refresh() {
            self.state.store.touch();
        }
        let candidate = access
            .pairing
            .candidates
            .get(attempt)
            .ok_or(RemoteError::PairingUnavailable)?;
        if candidate.status.peer_id != peer
            || candidate.status.phase != PairingPhase::AwaitingApproval
            || access.generation != Some(candidate.generation)
        {
            return Err(RemoteError::PairingUnavailable);
        }
        self.state.store.commit_grant(IncomingControlGrant {
            peer_id: peer.into(),
            name: candidate.status.name.clone(),
            attempt_id: attempt.into(),
        })?;
        access.pairing.invalidate(PairingPhase::Expired);
        let candidate = access
            .pairing
            .candidates
            .get_mut(attempt)
            .ok_or(RemoteError::PairingUnavailable)?;
        candidate.status.phase = PairingPhase::Granted;
        self.state.store.touch();
        Ok(candidate.status.clone())
    }
    pub fn reject_pairing(&self, attempt: &str, peer: &str) -> Result<PairingStatus> {
        let mut access = self.state.access()?;
        if let Some(grant) = self.state.store.grant(peer)?
            && grant.attempt_id == attempt
        {
            return Ok(PairingStatus {
                attempt_id: attempt.into(),
                peer_id: peer.into(),
                name: grant.name,
                phase: PairingPhase::Granted,
                verification_code: access
                    .pairing
                    .candidates
                    .get(attempt)
                    .and_then(|candidate| candidate.status.verification_code.clone()),
                expires_at_unix_seconds: 0,
            });
        }
        if access.pairing.refresh() {
            self.state.store.touch();
        }
        let candidate = access
            .pairing
            .candidates
            .get_mut(attempt)
            .ok_or(RemoteError::PairingUnavailable)?;
        if candidate.status.peer_id != peer || candidate.status.phase == PairingPhase::Granted {
            return Err(RemoteError::PairingUnavailable);
        }
        if matches!(
            candidate.status.phase,
            PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
        ) {
            candidate.status.phase = PairingPhase::Rejected;
        }
        self.state.store.touch();
        Ok(candidate.status.clone())
    }
    /// Begins pairing with the device that issued `offer`, then follows the
    /// attempt until that device answers (see `follow_pairing`).
    pub async fn begin_pairing(self: &Arc<Self>, offer: &PairingOffer) -> Result<PairingStatus> {
        let status = self.begin_pairing_attempt(offer).await?;
        if matches!(
            status.phase,
            PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
        ) {
            self.follow_pairing(status.peer_id.clone());
        }
        Ok(status)
    }
    /// The issuing device answers on its own screen and never calls back, so
    /// the pairing device asks until the answer arrives, which confirms the
    /// contact for state observers, or until the attempt cannot be used. Only a
    /// weak reference is held between attempts, so closing the service ends it.
    /// Needs a Tokio runtime context.
    pub fn follow_pairing(self: &Arc<Self>, peer: String) {
        let service = Arc::downgrade(self);
        tokio::spawn(async move {
            let deadline = Instant::now() + FOLLOW_LIMIT;
            while Instant::now() < deadline {
                tokio::time::sleep(FOLLOW_INTERVAL).await;
                let Some(service) = service.upgrade() else {
                    return;
                };
                match service.pairing_status(&peer).await {
                    Ok(status)
                        if matches!(
                            status.phase,
                            PairingPhase::AwaitingReady | PairingPhase::AwaitingApproval
                        ) => {}
                    Ok(_)
                    | Err(
                        RemoteError::Closed
                        | RemoteError::PairingUnavailable
                        | RemoteError::Unauthorized,
                    ) => return,
                    // The device may be briefly unreachable; ask again.
                    Err(_) => {}
                }
            }
        });
    }
    /// Resumes following the pairings this device began and has not seen
    /// answered, as after a restart. Needs a Tokio runtime context.
    pub fn follow_unanswered_pairings(self: &Arc<Self>) -> Result<()> {
        for contact in self.state.store.snapshot()?.contacts {
            if matches!(contact.confirmation, ContactConfirmation::Pending { .. }) {
                self.follow_pairing(contact.peer_id);
            }
        }
        Ok(())
    }
    async fn begin_pairing_attempt(&self, offer: &PairingOffer) -> Result<PairingStatus> {
        let _begin = self.pairing_begin.lock().await;
        let offer = PairingOffer::decode(&offer.encode()?)?;
        if offer.issuer == self.state.identity.id() {
            return Err(RemoteError::InvalidInvitation);
        }
        if let Some(existing) = self
            .state
            .outgoing
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .get(&offer.issuer)
            && existing.invite_id == offer.invite_id
            && !existing.client.connection_status().closed
            && matches!(
                existing.status.phase,
                PairingPhase::AwaitingReady
                    | PairingPhase::AwaitingApproval
                    | PairingPhase::Granted
            )
        {
            return Ok(existing.status.clone());
        }
        let client = Arc::new(
            self.dial_locator(
                &offer.issuer,
                offer.name.clone(),
                &offer.locator,
                ClientSession::new()?,
                false,
            )
            .await?,
        );
        let Reply::Pairing(challenge) = client
            .rpc(Request::PairBegin {
                invite_id: offer.invite_id.clone(),
                token: offer.token,
                name: self.state.identity.name.clone(),
            })
            .await?
        else {
            return Err(RemoteError::Protocol);
        };
        let native = client.connection.native().ok_or(RemoteError::Closed)?;
        let code = verification_code(
            &native,
            &offer.issuer,
            &self.state.identity.id(),
            &offer.invite_id,
            &challenge.attempt_id,
            challenge.challenge.ok_or(RemoteError::Protocol)?,
        )?;
        drop(native);
        let status = PairingStatus {
            attempt_id: challenge.attempt_id.clone(),
            peer_id: offer.issuer.clone(),
            name: offer.name.clone(),
            phase: challenge.phase,
            verification_code: Some(code),
            expires_at_unix_seconds: challenge.expires_at_unix_seconds,
        };
        {
            let _access = self.state.access()?;
            self.state.store.save_pin(DeviceContact {
                peer_id: offer.issuer.clone(),
                name: offer.name,
                locator: offer.locator,
                confirmation: ContactConfirmation::Pending {
                    attempt_id: challenge.attempt_id.clone(),
                },
                paused: false,
                locator_source: LocatorSource::PairingOffer,
            })?;
            self.state
                .outgoing
                .lock()
                .map_err(|_| RemoteError::Unavailable)?
                .insert(
                    offer.issuer.clone(),
                    OutgoingPairing {
                        client: client.clone(),
                        status,
                        invite_id: offer.invite_id.clone(),
                    },
                );
        }
        let Reply::Pairing(ready) = client
            .rpc(Request::PairReady {
                attempt_id: challenge.attempt_id,
            })
            .await?
        else {
            return Err(RemoteError::Protocol);
        };
        self.finish_pairing_status(&offer.issuer, ready)
    }
    fn finish_pairing_status(&self, peer: &str, wire: PairingWireState) -> Result<PairingStatus> {
        let _access = self.state.access()?;
        let mut outgoing = self
            .state
            .outgoing
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        let contact = self.state.store.contact(peer)?;
        if wire.phase == PairingPhase::Granted {
            self.state.store.confirm_pin(peer, &wire.attempt_id)?;
        }
        let status = PairingStatus {
            attempt_id: wire.attempt_id,
            peer_id: peer.into(),
            name: contact.name,
            phase: wire.phase,
            verification_code: outgoing
                .get(peer)
                .and_then(|pairing| pairing.status.verification_code.clone()),
            expires_at_unix_seconds: wire.expires_at_unix_seconds,
        };
        if let Some(pairing) = outgoing.get_mut(peer) {
            pairing.status = status.clone();
        }
        self.state.store.touch();
        Ok(status)
    }
    pub async fn pairing_status(&self, peer: &str) -> Result<PairingStatus> {
        self.outgoing_pairing_request(peer, false).await
    }
    pub async fn cancel_pairing(&self, peer: &str) -> Result<PairingStatus> {
        self.outgoing_pairing_request(peer, true).await
    }
    async fn outgoing_pairing_request(&self, peer: &str, cancel: bool) -> Result<PairingStatus> {
        let contact = self.state.store.contact(peer)?;
        let active = self
            .state
            .outgoing
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .get(peer)
            .map(|pairing| {
                (
                    pairing.client.clone(),
                    pairing.status.attempt_id.clone(),
                    pairing.status.phase,
                )
            });
        let (client, attempt_id, phase) = if let Some((client, attempt, phase)) =
            active.filter(|(client, _, _)| !client.connection_status().closed)
        {
            (client, attempt, phase)
        } else {
            let attempt = match &contact.confirmation {
                ContactConfirmation::Pending { attempt_id: id } => id.clone(),
                ContactConfirmation::Confirmed { attempt_id: id } => id.clone(),
            };
            (
                Arc::new(
                    self.dial_locator(
                        peer,
                        contact.name.clone(),
                        &contact.locator,
                        ClientSession::new()?,
                        false,
                    )
                    .await?,
                ),
                attempt,
                PairingPhase::Disconnected,
            )
        };
        let request = if cancel {
            Request::PairCancel { attempt_id }
        } else if phase == PairingPhase::AwaitingReady {
            Request::PairReady { attempt_id }
        } else {
            Request::PairStatus { attempt_id }
        };
        let Reply::Pairing(status) = client.rpc(request).await? else {
            return Err(RemoteError::Protocol);
        };
        self.finish_pairing_status(peer, status)
    }
}
