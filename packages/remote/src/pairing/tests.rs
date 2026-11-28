use super::*;

#[test]
fn compact_invitation_preserves_binary_tokens_and_pairing_scope() {
    use base64::Engine as _;

    let mut token = [0x8a; 32];
    token[0] = 0;
    token[12] = 0;
    token[31] = 0;
    let mut offer = PairingOffer {
        version: PROTOCOL_VERSION,
        purpose: "velunePairing".into(),
        permission: PairingPermission::Control,
        issuer: iroh::SecretKey::from_bytes(&[0x33; 32])
            .public()
            .to_string(),
        invite_id: "00112233445566778899aabbccddeeff".into(),
        token,
        expires_at_unix_seconds: 1_800_000_000,
        name: "配对夹具".into(),
        locator: DeviceLocator {
            relay_urls: vec!["https://relay.example/".into()],
            direct_addresses: vec!["127.0.0.1:8080".parse().unwrap()],
            lookup_urls: vec!["https://lookup.example/".into()],
        },
    };
    let url = |bytes: &[u8]| {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        format!("{OFFER_URL_PREFIX}{encoded}")
    };
    // The original Control layout has no permission byte.
    let mut legacy = format!("{}\0{}\0", offer.issuer, offer.invite_id).into_bytes();
    legacy.extend_from_slice(&token);
    let token_separator = legacy.len();
    legacy.push(0);
    legacy.extend_from_slice(offer.name.as_bytes());
    legacy.extend_from_slice(
        b"\0\x01https://relay.example/\0\x01127.0.0.1:8080\0\x01https://lookup.example/\0",
    );
    let locator_terminator = legacy.len();
    legacy.push(0);
    legacy.extend_from_slice(&offer.expires_at_unix_seconds.to_le_bytes());
    let legacy_url = url(&legacy);
    assert_eq!(offer.encode_url().unwrap(), legacy_url);
    let json = offer.encode().unwrap();
    for input in [&legacy_url, &json] {
        let decoded = PairingOffer::decode(input).unwrap();
        assert_eq!(decoded.token, token);
        assert_eq!(decoded.encode().unwrap(), json);
    }

    offer.permission = PairingPermission::TelemetryProfileRead;
    let mut telemetry = legacy.clone();
    telemetry.push(2);
    assert_eq!(offer.encode_url().unwrap(), url(&telemetry));
    let decoded = PairingOffer::decode(&url(&telemetry)).unwrap();
    assert_eq!(decoded.encode().unwrap(), offer.encode().unwrap());

    for separator in [token_separator, locator_terminator] {
        let mut invalid = legacy.clone();
        invalid[separator] = 1;
        assert!(matches!(
            PairingOffer::decode(&url(&invalid)),
            Err(RemoteError::InvalidInvitation)
        ));
    }
    assert!(matches!(
        PairingOffer::decode(&url(&legacy[..legacy.len() - 1])),
        Err(RemoteError::InvalidInvitation)
    ));
    for tail in [&[1][..], &[2, 0][..], &[3][..]] {
        let mut invalid = legacy.clone();
        invalid.extend_from_slice(tail);
        assert!(matches!(
            PairingOffer::decode(&url(&invalid)),
            Err(RemoteError::InvalidInvitation)
        ));
    }

    offer.locator.relay_urls = vec![format!("https://relay.example/{}", "a".repeat(OFFER_LIMIT))];
    assert!(matches!(
        offer.encode_url(),
        Err(RemoteError::FrameTooLarge)
    ));
    assert!(matches!(
        PairingOffer::decode(&format!("{OFFER_URL_PREFIX}{}", "A".repeat(OFFER_LIMIT))),
        Err(RemoteError::FrameTooLarge)
    ));
}

struct PairingTestHandler;
impl crate::Handler for PairingTestHandler {
    fn execute(
        &self,
        _: crate::RequestContext,
        _: velune_application_api::Command,
    ) -> crate::HandlerFuture<'_> {
        Box::pin(async {
            Err(velune_application_api::Failure::new(
                velune_application_api::FailureKind::Unsupported,
                "配对夹具不执行业务",
            ))
        })
    }
}

async fn device(name: &str) -> Arc<RemoteAccessService> {
    device_with_execution(name, true).await
}

async fn device_with_execution(name: &str, executable: bool) -> Arc<RemoteAccessService> {
    let service = RemoteAccessService::bind(
        DeviceIdentity::generate(name.into()).unwrap(),
        EndpointConfig {
            relay: RelayConfig::Disabled,
            discovery: DiscoveryConfig::None,
            local_discovery: false,
            bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
            request_timeout: Duration::from_secs(5),
            ..EndpointConfig::default()
        },
        Arc::new(DeviceStore::memory()),
        executable.then(|| Arc::new(PairingTestHandler) as Arc<dyn crate::Handler>),
    )
    .await
    .unwrap();
    Arc::new(service)
}

async fn wait_phase(controller: &Controller, phase: ControllerPhase) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while controller.status().unwrap().phase != phase {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "连接状态未到达预期值：{}",
            serde_json::to_string(&controller.status().unwrap()).unwrap()
        )
    });
}

fn telemetry(home: &std::path::Path, endpoint: &str) -> Arc<velune_telemetry::ClientTelemetry> {
    let owner = velune_telemetry::ClientTelemetry::open(
        home,
        velune_telemetry::TelemetryIdentity::new("test", "test").unwrap(),
        false,
        None,
    )
    .unwrap();
    let profile = velune_telemetry::TelemetryProfile {
        endpoint: endpoint.into(),
        headers: [("authorization".into(), "SYNTHETIC-SECRET".into())].into(),
        sample_ratio: 0.5,
    };
    let p = owner
        .prepare(0, velune_telemetry::TelemetryPreference::Off, Some(profile))
        .unwrap();
    owner.commit(&p.id).unwrap();
    owner.confirm_native(p.revision, true).unwrap();
    owner
}

#[test]
fn mobile_profile_pairing_has_its_own_consent_ingress_and_revocation() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let source = device_with_execution("source-phone", false).await;
            let destination = device_with_execution("destination-phone", false).await;
            source.set_control_ingress(false).unwrap();
            destination.set_control_ingress(false).unwrap();
            let source_home = tempfile::tempdir().unwrap();
            let destination_home = tempfile::tempdir().unwrap();
            let owner = telemetry(source_home.path(), "http://127.0.0.1:4317");
            let destination_owner = telemetry(destination_home.path(), "http://127.0.0.1:4318");
            source.attach_telemetry(owner.clone()).unwrap();
            destination
                .attach_telemetry(destination_owner.clone())
                .unwrap();
            let source_id = source.descriptor().unwrap().endpoint_id;
            let destination_id = destination.descriptor().unwrap().endpoint_id;
            let offer = source
                .create_telemetry_invitation(Duration::from_secs(60))
                .unwrap();
            let mut tampered = offer.clone();
            tampered.permission = PairingPermission::Control;
            assert!(destination.begin_pairing(&tampered).await.is_err());
            let pending = destination.begin_pairing(&offer).await.unwrap();
            assert_eq!(pending.permission, PairingPermission::TelemetryProfileRead);
            assert_eq!(
                pending.verification_code,
                source.read_state().unwrap().incoming_pairings[0].verification_code
            );
            assert!(matches!(
                destination.read_telemetry_profile(&source_id).await,
                Err(RemoteError::Unauthorized)
            ));
            source
                .approve_pairing(&pending.attempt_id, &destination_id)
                .unwrap();
            destination.pairing_status(&source_id).await.unwrap();
            assert!(source.read_state().unwrap().incoming_grants.is_empty());
            assert!(!destination.read_state().unwrap().contacts[0].control_authorized);
            assert!(matches!(
                destination.controller(&source_id).unwrap().connect().await,
                Err(RemoteError::Unauthorized)
            ));
            let copied = destination
                .read_telemetry_profile(&source_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(copied.headers["authorization"], "SYNTHETIC-SECRET");
            let raw = destination
                .dial_locator(
                    &source_id,
                    offer.name.clone(),
                    &offer.locator,
                    ClientSession::new().unwrap(),
                    true,
                )
                .await
                .unwrap();
            assert!(matches!(
                raw.rpc(Request::Submit {
                    id: raw.next_request_id().unwrap(),
                    view: velune_application_api::ClientViewId("test".into()),
                    execution: Default::default(),
                    command: Box::new(velune_application_api::Command::ReadConfiguration),
                    trace_context: None
                })
                .await,
                Err(RemoteError::Unauthorized)
            ));
            assert_eq!(
                source.set_control_ingress(true),
                Err(RemoteError::NoHandler)
            );
            source.revoke_control(&destination_id).unwrap();
            assert!(
                destination
                    .read_telemetry_profile(&source_id)
                    .await
                    .unwrap()
                    .is_some()
            );
            let state = destination_owner.snapshot().unwrap();
            let candidate = destination_owner
                .prepare(state.revision, state.preference, Some(copied))
                .unwrap();
            destination_owner.commit(&candidate.id).unwrap();
            destination_owner
                .confirm_native(candidate.revision, true)
                .unwrap();
            assert_eq!(
                destination_owner.snapshot().unwrap().preference,
                velune_telemetry::TelemetryPreference::Off
            );
            assert!(
                destination_owner
                    .snapshot()
                    .unwrap()
                    .rust_active_revision
                    .is_none()
            );
            source.revoke_telemetry_profile(&destination_id).unwrap();
            assert!(matches!(
                destination.read_telemetry_profile(&source_id).await,
                Err(RemoteError::Unauthorized)
            ));
            assert!(
                source
                    .approve_pairing(&pending.attempt_id, &destination_id)
                    .is_err()
            );
            // The same two handler-free clients work in the opposite direction with separate consent.
            let reverse = destination
                .create_telemetry_invitation(Duration::from_secs(60))
                .unwrap();
            let pending = source.begin_pairing(&reverse).await.unwrap();
            destination
                .approve_pairing(&pending.attempt_id, &source_id)
                .unwrap();
            source.pairing_status(&destination_id).await.unwrap();
            assert!(
                source
                    .read_telemetry_profile(&destination_id)
                    .await
                    .unwrap()
                    .is_some()
            );
            destination.close().await.unwrap();
            source.close().await.unwrap();
        });
}

#[test]
fn existing_control_consent_does_not_share_credentials() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let source = device("source").await;
            let destination = device("destination").await;
            let home = tempfile::tempdir().unwrap();
            source
                .attach_telemetry(telemetry(home.path(), "http://127.0.0.1:4317"))
                .unwrap();
            let source_id = source.descriptor().unwrap().endpoint_id;
            let destination_id = destination.descriptor().unwrap().endpoint_id;
            let pending = destination
                .begin_pairing(&source.create_invitation(Duration::from_secs(60)).unwrap())
                .await
                .unwrap();
            source
                .approve_pairing(&pending.attempt_id, &destination_id)
                .unwrap();
            destination.pairing_status(&source_id).await.unwrap();
            assert!(matches!(
                destination.read_telemetry_profile(&source_id).await,
                Err(RemoteError::Unauthorized)
            ));
            source.allow_telemetry_profile(&destination_id).unwrap();
            source.set_control_ingress(false).unwrap();
            assert!(
                destination
                    .read_telemetry_profile(&source_id)
                    .await
                    .unwrap()
                    .is_some()
            );
            destination.close().await.unwrap();
            source.close().await.unwrap();
        });
}

#[test]
fn rejected_additional_scope_keeps_confirmed_control_and_original_request_identity() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let host = device("host").await;
            let phone = device("phone").await;
            let home = tempfile::tempdir().unwrap();
            host.attach_telemetry(telemetry(home.path(), "http://127.0.0.1:4317"))
                .unwrap();
            let host_id = host.descriptor().unwrap().endpoint_id;
            let phone_id = phone.descriptor().unwrap().endpoint_id;
            let control = phone
                .begin_pairing(&host.create_invitation(Duration::from_secs(60)).unwrap())
                .await
                .unwrap();
            host.approve_pairing(&control.attempt_id, &phone_id)
                .unwrap();
            phone.pairing_status(&host_id).await.unwrap();
            let controller = phone.controller(&host_id).unwrap();
            controller.connect().await.unwrap();
            let original = controller.client().unwrap().next_request_id().unwrap();
            let sharing = phone
                .begin_pairing(
                    &host
                        .create_telemetry_invitation(Duration::from_secs(60))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(matches!(
                phone.read_state().unwrap().contacts[0].confirmation,
                ContactConfirmation::Confirmed { .. }
            ));
            host.reject_pairing(&sharing.attempt_id, &phone_id).unwrap();
            phone.pairing_status(&host_id).await.unwrap();
            controller.client().unwrap().close();
            controller.connect().await.unwrap();
            assert!(matches!(
                controller.query(original.clone()).await.unwrap(),
                RequestStatus::Unknown
            ));
            assert_eq!(original.incarnation, host.descriptor().unwrap().incarnation);
            assert!(matches!(
                phone.read_telemetry_profile(&host_id).await,
                Err(RemoteError::Unauthorized)
            ));
            phone.close().await.unwrap();
            host.close().await.unwrap();
        });
}

#[test]
fn re_pairing_recovers_the_existing_controller() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let host = device("host").await;
            let phone = device("phone").await;
            let host_id = host.descriptor().unwrap().endpoint_id;
            let phone_id = phone.descriptor().unwrap().endpoint_id;
            let offer = host.create_invitation(Duration::from_secs(60)).unwrap();
            let first = phone.begin_pairing(&offer).await.unwrap();
            host.approve_pairing(&first.attempt_id, &phone_id).unwrap();
            phone.pairing_status(&host_id).await.unwrap();
            let controller = phone.controller(&host_id).unwrap();
            controller.connect().await.unwrap();
            let epoch = controller.status().unwrap().connection_epoch;

            host.revoke_control(&phone_id).unwrap();
            wait_phase(&controller, ControllerPhase::Blocked).await;
            assert_eq!(
                controller.status().unwrap().error_code.as_deref(),
                Some("unauthorized")
            );
            assert!(matches!(
                host.approve_pairing(&first.attempt_id, &phone_id),
                Err(RemoteError::PairingUnavailable)
            ));

            let offer = host.create_invitation(Duration::from_secs(60)).unwrap();
            let second = phone.begin_pairing(&offer).await.unwrap();
            assert_ne!(first.attempt_id, second.attempt_id);
            for phase in [
                PairingPhase::AwaitingReady,
                PairingPhase::AwaitingApproval,
                PairingPhase::Granted,
                PairingPhase::Rejected,
                PairingPhase::Expired,
                PairingPhase::Disconnected,
            ] {
                assert!(matches!(
                    phone.finish_pairing_status(
                        &host_id,
                        PairingWireState {
                            permission: PairingPermission::Control,
                            attempt_id: first.attempt_id.clone(),
                            phase,
                            challenge: None,
                            expires_at_unix_seconds: 0,
                        }
                    ),
                    Err(RemoteError::PairingUnavailable)
                ));
            }
            let current = phone.read_state().unwrap();
            assert_eq!(current.outgoing_pairings[0].attempt_id, second.attempt_id);
            assert_eq!(
                current.outgoing_pairings[0].phase,
                PairingPhase::AwaitingApproval
            );
            assert!(matches!(
                controller.connect().await,
                Err(RemoteError::Unauthorized)
            ));
            wait_phase(&controller, ControllerPhase::Blocked).await;
            let revision = controller.status().unwrap().intent_revision;
            host.approve_pairing(&second.attempt_id, &phone_id).unwrap();
            phone.pairing_status(&host_id).await.unwrap();
            assert!(controller.status().unwrap().intent_revision > revision);
            assert!(Arc::ptr_eq(
                &controller,
                &phone.controller(&host_id).unwrap()
            ));
            wait_phase(&controller, ControllerPhase::Connected).await;
            assert!(controller.status().unwrap().connection_epoch > epoch);
            assert!(matches!(
                controller
                    .query(controller.next_request_id().unwrap())
                    .await,
                Ok(RequestStatus::Unknown)
            ));

            for (selected, foreground, paused) in [
                (true, true, true),
                (false, true, false),
                (true, false, false),
            ] {
                host.revoke_control(&phone_id).unwrap();
                wait_phase(&controller, ControllerPhase::Blocked).await;
                if paused {
                    controller.disconnect().unwrap();
                } else {
                    controller.set_activity(selected, foreground).unwrap();
                }
                let epoch = controller.status().unwrap().connection_epoch;
                let offer = host.create_invitation(Duration::from_secs(60)).unwrap();
                let attempt = phone.begin_pairing(&offer).await.unwrap();
                assert_eq!(phone.read_state().unwrap().contacts[0].paused, paused);
                host.approve_pairing(&attempt.attempt_id, &phone_id)
                    .unwrap();
                phone.pairing_status(&host_id).await.unwrap();
                let status = controller.status().unwrap();
                assert_eq!(
                    (status.selected, status.foreground, status.paused),
                    (selected, foreground, paused)
                );
                assert!(status.error_code.is_none());
                assert!(
                    status.phase
                        == if paused || !selected {
                            ControllerPhase::Disconnected
                        } else {
                            ControllerPhase::Suspended
                        }
                );
                phone.pairing_status(&host_id).await.unwrap();
                tokio::task::yield_now().await;
                assert_eq!(
                    controller.status().unwrap().intent_revision,
                    status.intent_revision
                );
                assert_eq!(controller.status().unwrap().connection_epoch, epoch);
                assert!(controller.connection_info().is_err());
                if paused {
                    controller.connect().await.unwrap();
                } else {
                    controller.set_activity(true, true).unwrap();
                }
                wait_phase(&controller, ControllerPhase::Connected).await;
            }
            phone.close().await.unwrap();
            host.close().await.unwrap();
        });
}
