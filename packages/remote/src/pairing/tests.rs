use super::*;

async fn device(name: &str) -> Arc<RemoteAccessService> {
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
        None,
    )
    .await
    .unwrap();
    service.set_control_ingress(true).unwrap();
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
            let source = device("source-phone").await;
            let destination = device("destination-phone").await;
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
                    command: Box::new(velune_application_api::Command::List),
                    trace_context: None
                })
                .await,
                Err(RemoteError::Unauthorized)
            ));
            source.set_control_ingress(true).unwrap();
            source.set_control_ingress(false).unwrap();
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
