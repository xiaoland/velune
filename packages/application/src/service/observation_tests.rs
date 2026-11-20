use super::*;

#[test]
fn observation_lease_expires_at_deadline_and_client_activity_renews_it() {
    let start = Instant::now();
    let mut observations = ViewObservations::default();
    observations.replace(
        BTreeSet::from([("runtime".into(), "conversation".into())]),
        start,
    );
    assert!(observations.active(start + Duration::from_secs(89)));

    let expiry = start + VIEW_OBSERVATION_LEASE;
    assert!(observations.expire(expiry));
    assert!(!observations.active(expiry));
    assert_eq!(observations.conversations.len(), 1);

    // A real request on the same client view restores its desired set.
    observations.renew(expiry);
    assert!(observations.active(expiry));
    assert!(observations.active(expiry + Duration::from_secs(89)));
    assert!(observations.expire(expiry + VIEW_OBSERVATION_LEASE));
}

#[test]
fn empty_observation_replacement_releases_the_view_immediately() {
    let now = Instant::now();
    let mut observations = ViewObservations::default();
    observations.replace(
        BTreeSet::from([("runtime".into(), "conversation".into())]),
        now,
    );
    observations.replace(BTreeSet::new(), now);
    assert!(!observations.active(now));
    assert!(observations.conversations.is_empty());
}
