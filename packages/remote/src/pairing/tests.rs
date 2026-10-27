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

fn telemetry(home: &std::path::Path, endpoint: &str) -> Arc<velune_telemetry::ClientTelemetry> {
    let owner = velune_telemetry::ClientTelemetry::open(
        home,
        velune_telemetry::ResourceIdentity::new("test", "test").unwrap(),
        false,
        None,
    )
    .unwrap();
    let profile = velune_telemetry::Profile {
        endpoint: endpoint.into(),
        headers: [("authorization".into(), "SYNTHETIC-SECRET".into())].into(),
        sample_ratio: 0.5,
    };
    let p = owner
        .prepare(0, velune_telemetry::Preference::Off, Some(profile))
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
                velune_telemetry::Preference::Off
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
