//! Purpose-separated device-network admission and legacy directional permissions.
//! Only an admitted issuer can admit another member. Transport still authenticates
//! the original operator and destination; membership never signs business requests.
use crate::{DeviceLocator, PairingPermission, RemoteError, Result, store::parse_peer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const VERSION: u16 = 1;
const MEMBERSHIP_DOMAIN: &[u8] = b"velune.device-membership.v1\0";
const DOMAIN: &[u8] = b"velune.directional-trust.v1\0";
pub(crate) const MAX_STATEMENTS: usize = 1024;
pub(crate) const MAX_FOREIGN_STATEMENTS: usize = MAX_STATEMENTS / 2;
const MAX_LOCAL_STATEMENTS: usize = MAX_STATEMENTS - MAX_FOREIGN_STATEMENTS;
const MAX_BUNDLE_BYTES: usize = 768 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustStatement {
    pub version: u16,
    /// The controlled device whose public key verifies this statement.
    pub issuer: String,
    pub subject: String,
    pub permission: PairingPermission,
    /// Stable across re-pairing and revocation of this directional permission.
    pub grant_id: String,
    pub revision: u64,
    pub revoked: bool,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustContact {
    pub peer_id: String,
    pub name: String,
    pub locator: DeviceLocator,
    /// An availability hint, not authority. Authenticated self reports take precedence.
    #[serde(default)]
    pub can_execute: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustBundle {
    pub version: u16,
    pub statements: Vec<TrustStatement>,
    pub memberships: Vec<MembershipStatement>,
    pub contacts: Vec<TrustContact>,
}
impl Default for TrustBundle {
    fn default() -> Self {
        Self {
            version: 2,
            statements: Vec::new(),
            memberships: Vec::new(),
            contacts: Vec::new(),
        }
    }
}
impl TrustBundle {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.version != 2 {
            return Err(RemoteError::IncompatibleVersion);
        }
        if self.statements.len() > MAX_STATEMENTS
            || self.memberships.len() > MAX_STATEMENTS
            || self.contacts.len() > 257
        {
            return Err(RemoteError::Capacity);
        }
        if serde_json::to_vec(self)
            .map_err(|_| RemoteError::Protocol)?
            .len()
            > MAX_BUNDLE_BYTES
        {
            return Err(RemoteError::FrameTooLarge);
        }
        for statement in &self.statements {
            statement.verify()?;
        }
        for statement in &self.memberships {
            statement.verify()?;
        }
        let mut contacts = BTreeSet::new();
        for contact in &self.contacts {
            parse_peer(&contact.peer_id)?;
            crate::identity::validate_name(&contact.name)?;
            contact.locator.validate()?;
            if !contacts.insert(&contact.peer_id) {
                return Err(RemoteError::Protocol);
            }
        }
        Ok(())
    }
}

fn permission_code(permission: PairingPermission) -> u8 {
    match permission {
        PairingPermission::Control => 1,
        PairingPermission::TelemetryProfileRead => 2,
    }
}
fn relation_id(issuer: &str, subject: &str, permission: PairingPermission) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    hash.update(parse_peer(issuer)?.as_bytes());
    hash.update(parse_peer(subject)?.as_bytes());
    hash.update([permission_code(permission)]);
    Ok(format!("{:x}", hash.finalize()))
}
impl TrustStatement {
    pub(crate) fn issue(
        key: &iroh::SecretKey,
        subject: &str,
        permission: PairingPermission,
        revision: u64,
        revoked: bool,
    ) -> Result<Self> {
        let issuer = key.public().to_string();
        let mut statement = Self {
            version: VERSION,
            grant_id: relation_id(&issuer, subject, permission)?,
            issuer,
            subject: subject.into(),
            permission,
            revision,
            revoked,
            signature: Vec::new(),
        };
        statement.signature = key.sign(&statement.signing_bytes()?).to_bytes().to_vec();
        Ok(statement)
    }
    fn signing_bytes(&self) -> Result<Vec<u8>> {
        if self.version != VERSION {
            return Err(RemoteError::IncompatibleVersion);
        }
        if self.revision == 0
            || self.issuer == self.subject
            || self.grant_id != relation_id(&self.issuer, &self.subject, self.permission)?
        {
            return Err(RemoteError::Protocol);
        }
        // Fixed-width fields and explicit domain separation, independent of JSON encoding.
        let mut bytes = DOMAIN.to_vec();
        bytes.extend(self.version.to_be_bytes());
        bytes.extend(parse_peer(&self.issuer)?.as_bytes());
        bytes.extend(parse_peer(&self.subject)?.as_bytes());
        bytes.push(permission_code(self.permission));
        bytes.extend(self.grant_id.as_bytes());
        bytes.extend(self.revision.to_be_bytes());
        bytes.push(u8::from(self.revoked));
        Ok(bytes)
    }
    pub fn verify(&self) -> Result<()> {
        let signature = iroh::Signature::try_from(self.signature.as_slice())
            .map_err(|_| RemoteError::Protocol)?;
        parse_peer(&self.issuer)?
            .verify(&self.signing_bytes()?, &signature)
            .map_err(|_| RemoteError::Unauthorized)
    }
}

/// A directed acceptance into the issuer's device network, independent of Control
/// or TelemetryProfileRead grants. Its stable identity survives revoke/re-pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipStatement {
    pub version: u16,
    pub issuer: String,
    pub subject: String,
    pub membership_id: String,
    pub revision: u64,
    pub revoked: bool,
    pub signature: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipProof {
    pub root: String,
    pub member: String,
    pub statements: Vec<MembershipStatement>,
}
fn membership_id(issuer: &str, subject: &str) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(MEMBERSHIP_DOMAIN);
    hash.update(parse_peer(issuer)?.as_bytes());
    hash.update(parse_peer(subject)?.as_bytes());
    Ok(format!("{:x}", hash.finalize()))
}
impl MembershipStatement {
    pub(crate) fn issue(
        key: &iroh::SecretKey,
        subject: &str,
        revision: u64,
        revoked: bool,
    ) -> Result<Self> {
        let issuer = key.public().to_string();
        let mut statement = Self {
            version: VERSION,
            membership_id: membership_id(&issuer, subject)?,
            issuer,
            subject: subject.into(),
            revision,
            revoked,
            signature: Vec::new(),
        };
        statement.signature = key.sign(&statement.signing_bytes()?).to_bytes().to_vec();
        Ok(statement)
    }
    fn signing_bytes(&self) -> Result<Vec<u8>> {
        if self.version != VERSION {
            return Err(RemoteError::IncompatibleVersion);
        }
        if self.revision == 0
            || self.issuer == self.subject
            || self.membership_id != membership_id(&self.issuer, &self.subject)?
        {
            return Err(RemoteError::Protocol);
        }
        let mut bytes = MEMBERSHIP_DOMAIN.to_vec();
        bytes.extend(self.version.to_be_bytes());
        bytes.extend(parse_peer(&self.issuer)?.as_bytes());
        bytes.extend(parse_peer(&self.subject)?.as_bytes());
        bytes.extend(self.membership_id.as_bytes());
        bytes.extend(self.revision.to_be_bytes());
        bytes.push(u8::from(self.revoked));
        Ok(bytes)
    }
    pub fn verify(&self) -> Result<()> {
        let signature = iroh::Signature::try_from(self.signature.as_slice())
            .map_err(|_| RemoteError::Protocol)?;
        parse_peer(&self.issuer)?
            .verify(&self.signing_bytes()?, &signature)
            .map_err(|_| RemoteError::Unauthorized)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionCapability {
    pub can_execute: bool,
    pub self_reported: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrustState {
    pub issuer: Option<String>,
    pub memberships: BTreeMap<String, MembershipStatement>,
    /// Local consent is authoritative; never populated by wire assertions.
    pub direct_members: BTreeSet<String>,
    /// Completed direct outgoing Control pins retain the legacy target hint.
    pub direct_control_pins: BTreeSet<String>,
    pub membership_initialized: bool,
    pub local_execution_capability: Option<bool>,
    pub execution_capabilities: BTreeMap<String, ExecutionCapability>,
    pub statements: BTreeMap<String, TrustStatement>,
    /// v5 explicit pairing provenance retained for migration/recovery; v6 uses
    /// direct_members and direct_control_pins and never consumes this as authority.
    pub paired_controls: BTreeSet<String>,
    /// Authenticated introduction sources are routing hints, never authority.
    pub introduced_via: BTreeMap<String, BTreeSet<String>>,
    pub introduced_locators: BTreeSet<String>,
    /// Local discovery exclusions. Only a new explicit pairing confirmation
    /// clears one; neither a valid grant nor another introduction does so.
    pub forgotten_contacts: BTreeSet<String>,
}
impl TrustState {
    pub fn validate(&self) -> Result<()> {
        if self.memberships.len() > MAX_STATEMENTS
            || self.direct_members.len() > 512
            || self.direct_control_pins.len() > 256
            || self.execution_capabilities.len() > 512
        {
            return Err(RemoteError::Capacity);
        }
        for peer in self
            .direct_members
            .iter()
            .chain(&self.direct_control_pins)
            .chain(self.execution_capabilities.keys())
        {
            parse_peer(peer)?;
            if self.issuer.as_ref() == Some(peer) {
                return Err(RemoteError::StateStorage);
            }
        }
        if self
            .direct_members
            .iter()
            .any(|p| self.forgotten_contacts.contains(p))
        {
            return Err(RemoteError::StateStorage);
        }
        for (id, edge) in &self.memberships {
            if id != &edge.membership_id {
                return Err(RemoteError::StateStorage);
            }
            edge.verify()?;
        }
        if let Some(local) = &self.issuer {
            let local_count = self
                .memberships
                .values()
                .filter(|s| &s.issuer == local)
                .count();
            if local_count > MAX_LOCAL_STATEMENTS
                || self.memberships.len() - local_count > MAX_FOREIGN_STATEMENTS
            {
                return Err(RemoteError::Capacity);
            }
        } else if self.membership_initialized || !self.memberships.is_empty() {
            return Err(RemoteError::StateStorage);
        }
        if self.statements.len() > MAX_STATEMENTS
            || self.paired_controls.len() > 256
            || self.introduced_via.len() > 256
            || self.introduced_locators.len() > 256
            || self.forgotten_contacts.len() > MAX_STATEMENTS
            || self
                .introduced_via
                .values()
                .any(|sources| sources.len() > 256)
        {
            return Err(RemoteError::Capacity);
        }
        if let Some(issuer) = &self.issuer {
            parse_peer(issuer)?;
            let local_count = self
                .statements
                .values()
                .filter(|edge| &edge.issuer == issuer)
                .count();
            // Propagation cannot consume the slots reserved for local consent
            // and its tombstones. Both histories are bounded independently.
            if local_count > MAX_LOCAL_STATEMENTS
                || self.statements.len() - local_count > MAX_FOREIGN_STATEMENTS
            {
                return Err(RemoteError::Capacity);
            }
        } else if !self.statements.is_empty()
            || !self.paired_controls.is_empty()
            || !self.introduced_via.is_empty()
            || !self.introduced_locators.is_empty()
        {
            return Err(RemoteError::StateStorage);
        }
        for (id, statement) in &self.statements {
            if id != &statement.grant_id {
                return Err(RemoteError::StateStorage);
            }
            statement.verify()?;
        }
        for peer in &self.paired_controls {
            parse_peer(peer)?;
        }
        for (peer, sources) in &self.introduced_via {
            parse_peer(peer)?;
            for source in sources {
                parse_peer(source)?;
            }
        }
        for peer in &self.introduced_locators {
            parse_peer(peer)?;
        }
        for peer in &self.forgotten_contacts {
            parse_peer(peer)?;
        }
        Ok(())
    }
    pub fn statement(
        &self,
        issuer: &str,
        subject: &str,
        permission: PairingPermission,
    ) -> Result<Option<&TrustStatement>> {
        Ok(self
            .statements
            .get(&relation_id(issuer, subject, permission)?))
    }
    pub fn set_local(
        &mut self,
        key: &iroh::SecretKey,
        subject: &str,
        permission: PairingPermission,
        revoked: bool,
    ) -> Result<()> {
        let previous = self.statement(&key.public().to_string(), subject, permission)?;
        if previous.is_some_and(|old| old.revoked == revoked) {
            return Ok(());
        }
        let revision = previous.map_or(Ok(1), |old| {
            old.revision.checked_add(1).ok_or(RemoteError::Capacity)
        })?;
        let statement = TrustStatement::issue(key, subject, permission, revision, revoked)?;
        self.statements
            .insert(statement.grant_id.clone(), statement);
        Ok(())
    }
    /// Authenticate every statement before calling this. Local statements cannot
    /// be installed from the wire: the current local grant is the authority.
    pub fn merge(&mut self, statements: &[TrustStatement]) -> Result<()> {
        for statement in statements {
            let old = self.statements.get(&statement.grant_id);
            if self.issuer.as_deref() == Some(&statement.issuer)
                && old.is_none_or(|old| statement.revision > old.revision)
            {
                return Err(RemoteError::Unauthorized);
            }
            match old {
                Some(old) if old.revision > statement.revision => continue,
                Some(old) if old.revision == statement.revision => {
                    if old != statement {
                        return Err(RemoteError::Protocol);
                    }
                    continue;
                }
                _ => {
                    self.statements
                        .insert(statement.grant_id.clone(), statement.clone());
                }
            }
        }
        Ok(())
    }
    pub fn set_local_membership(
        &mut self,
        key: &iroh::SecretKey,
        peer: &str,
        revoked: bool,
    ) -> Result<()> {
        let id = membership_id(&key.public().to_string(), peer)?;
        let previous = self.memberships.get(&id);
        if previous.is_some_and(|s| s.revoked == revoked) {
            return Ok(());
        }
        let revision = previous.map_or(Ok(1), |s| {
            s.revision.checked_add(1).ok_or(RemoteError::Capacity)
        })?;
        let statement = MembershipStatement::issue(key, peer, revision, revoked)?;
        self.memberships.insert(id, statement);
        Ok(())
    }
    /// Local roots cannot be restored or advanced by a forwarded historical copy.
    pub fn merge_memberships(&mut self, statements: &[MembershipStatement]) -> Result<()> {
        for statement in statements {
            let old = self.memberships.get(&statement.membership_id);
            if self.issuer.as_deref() == Some(&statement.issuer)
                && old.is_none_or(|s| statement.revision > s.revision)
            {
                return Err(RemoteError::Unauthorized);
            }
            match old {
                Some(old) if old.revision > statement.revision => continue,
                Some(old) if old.revision == statement.revision => {
                    if old != statement {
                        return Err(RemoteError::Protocol);
                    }
                }
                _ => {
                    self.memberships
                        .insert(statement.membership_id.clone(), statement.clone());
                }
            }
        }
        Ok(())
    }
    /// Admission only follows issuer -> subject from our current local roots.
    /// Exclusions apply to endpoints and intermediate nodes alike.
    pub fn membership_proof(&self, member: &str) -> Result<MembershipProof> {
        parse_peer(member)?;
        let root = self.issuer.as_deref().ok_or(RemoteError::IdentityStorage)?;
        if root == member || self.forgotten_contacts.contains(member) {
            return Err(RemoteError::Unauthorized);
        }
        let mut queue = VecDeque::from([root]);
        let mut previous = BTreeMap::<&str, &MembershipStatement>::new();
        let mut visited = BTreeSet::from([root]);
        while let Some(issuer) = queue.pop_front() {
            for edge in self
                .memberships
                .values()
                .filter(|s| !s.revoked && s.issuer == issuer)
            {
                if self.forgotten_contacts.contains(&edge.subject) || !visited.insert(&edge.subject)
                {
                    continue;
                }
                previous.insert(&edge.subject, edge);
                if edge.subject == member {
                    let mut path = Vec::new();
                    let mut current = member;
                    while current != root {
                        let edge = previous[current];
                        path.push(edge.clone());
                        current = &edge.issuer;
                    }
                    path.reverse();
                    return Ok(MembershipProof {
                        root: root.into(),
                        member: member.into(),
                        statements: path,
                    });
                }
                queue.push_back(&edge.subject);
            }
        }
        Err(RemoteError::Unauthorized)
    }
    pub fn is_member(&self, member: &str) -> Result<bool> {
        match self.membership_proof(member) {
            Ok(_) => Ok(true),
            Err(RemoteError::Unauthorized) => Ok(false),
            Err(e) => Err(e),
        }
    }
    /// Storage relevance follows directed historical membership, including
    /// tombstones, so a revoke learned before an old grant is never discarded.
    /// This does not admit members: membership_proof follows only active edges
    /// and always applies local exclusions. Legacy Control grants play no role.
    pub fn retain_network(&mut self, previous: &Self) -> Result<()> {
        let root = self.issuer.as_deref().ok_or(RemoteError::IdentityStorage)?;
        let mut next = BTreeMap::<&str, Vec<&str>>::new();
        for edge in self.memberships.values() {
            next.entry(&edge.issuer).or_default().push(&edge.subject);
        }
        let mut relevant = BTreeSet::from([root]);
        let mut queue = VecDeque::from([root]);
        while let Some(issuer) = queue.pop_front() {
            for subject in next.get(issuer).into_iter().flatten() {
                if relevant.insert(subject) {
                    queue.push_back(subject);
                }
            }
        }
        let relevant: BTreeSet<String> = relevant.into_iter().map(str::to_string).collect();
        self.memberships
            .retain(|id, s| previous.memberships.contains_key(id) || relevant.contains(&s.issuer));
        self.statements
            .retain(|id, s| previous.statements.contains_key(id) || relevant.contains(&s.issuer));
        self.validate()
    }
}
