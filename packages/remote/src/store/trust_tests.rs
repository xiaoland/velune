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
    let old = TrustStatement::issue(&a.key, &b.id(), PairingPermission::Control, 1, false).unwrap();
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
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let a = DeviceIdentity::generate("A".into()).unwrap();
    let sa = DeviceStore::open(&path).unwrap();
    sa.initialize_trust(&a).unwrap();
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
    drop(sa);
    let sa = DeviceStore::open(&path).unwrap();
    sa.initialize_trust(&a).unwrap();
    assert!(!sa.is_member(&c.id()).unwrap());
    // A forwarded revoke must survive reopening, not merely remain in memory.
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
    oversized.memberships = vec![proof.memberships[0].clone(); crate::trust::MAX_STATEMENTS + 1];
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
