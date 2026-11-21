use super::*;

fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

fn peers(count: usize) -> Vec<String> {
    (0..count)
        .map(|_| {
            crate::DeviceIdentity::generate("Synthetic mesh device".into())
                .unwrap()
                .id()
        })
        .collect()
}

#[test]
fn path_allows_eight_hops_and_rejects_a_ninth() {
    let ids = peers(10);
    let path = enter_path(&ids[8], &ids[7], &ids[8], ids[..8].to_vec()).unwrap();
    assert_eq!(path, ids[..9]);
    validate_ready(&path, &ids[..1], &ids[1], &ids[8]).unwrap();
    validate_ready(&path, &ids[..8], &ids[8], &ids[8]).unwrap();
    assert_eq!(
        enter_path(&ids[9], &ids[8], &ids[9], path).unwrap_err(),
        RemoteError::Protocol
    );
    assert_eq!(
        validate_ready(&ids, &ids[..1], &ids[1], &ids[9]).unwrap_err(),
        RemoteError::Protocol
    );
}

#[test]
fn tunnel_path_binds_the_immediate_peer_and_rejects_loops() {
    let ids = peers(5);
    for visited in [
        vec![],
        vec![ids[0].clone()], // Adjacent peer is missing.
        vec![
            ids[0].clone(),
            ids[1].clone(),
            ids[0].clone(),
            ids[1].clone(),
        ],
        vec![ids[2].clone(), ids[1].clone()], // This relay is already in the path.
        vec![ids[3].clone(), ids[1].clone()], // Destination has already been visited.
        vec!["not-a-peer".into(), ids[1].clone()],
    ] {
        assert!(enter_path(&ids[2], &ids[1], &ids[3], visited).is_err());
    }
    assert_eq!(
        enter_path(&ids[2], &ids[1], &ids[3], ids[..2].to_vec()).unwrap(),
        ids[..3]
    );
}

#[test]
fn ready_cannot_change_prefix_next_peer_destination_or_repeat_a_hop() {
    let ids = peers(5);
    let good = ids[..4].to_vec();
    validate_ready(&good, &ids[..1], &ids[1], &ids[3]).unwrap();
    assert!(validate_ready(&good, &ids[..2], &ids[2], &ids[3]).is_ok());
    let mut wrong_origin = good.clone();
    wrong_origin[0] = ids[4].clone();
    assert!(validate_ready(&wrong_origin, &ids[..1], &ids[1], &ids[3]).is_err());
    assert!(validate_ready(&good, &ids[..1], &ids[2], &ids[3]).is_err());
    assert!(validate_ready(&good, &ids[..1], &ids[1], &ids[4]).is_err());
    assert!(validate_ready(&ids[..1], &ids[..1], &ids[1], &ids[3]).is_err());
    let repeated = vec![
        ids[0].clone(),
        ids[1].clone(),
        ids[0].clone(),
        ids[3].clone(),
    ];
    assert!(validate_ready(&repeated, &ids[..1], &ids[1], &ids[3]).is_err());
}

#[test]
fn mesh_only_accepts_control_business_requests() {
    assert!(allowed_request(&Request::Hello).is_ok());
    let query = Request::Query {
        id: crate::RequestId {
            incarnation: "a".repeat(32),
            client: "b".repeat(32),
            sequence: 1,
        },
        trace_context: None,
    };
    assert!(allowed_request(&query).is_ok());
    let submit = Request::Submit {
        id: crate::RequestId {
            incarnation: "a".repeat(32),
            client: "b".repeat(32),
            sequence: 2,
        },
        view: velune_application_api::ClientViewId("synthetic-view".into()),
        execution: Default::default(),
        command: Box::new(velune_application_api::Command::ReadConfiguration),
        trace_context: None,
    };
    assert!(allowed_request(&submit).is_ok());
    for request in [
        Request::PairBegin {
            invite_id: "a".repeat(32),
            token: [0; 32],
            name: "Synthetic peer".into(),
            permission: crate::PairingPermission::Control,
        },
        Request::PairReady {
            attempt_id: "a".repeat(32),
        },
        Request::PairStatus {
            attempt_id: "a".repeat(32),
        },
        Request::PairCancel {
            attempt_id: "a".repeat(32),
        },
        Request::ReadTelemetryProfile,
    ] {
        assert_eq!(allowed_request(&request), Err(RemoteError::Unauthorized));
    }
}

#[test]
fn prefixed_control_frame_does_not_consume_following_tunnel_bytes() {
    run(async {
        let (mut sender, mut receiver) = tokio::io::duplex(4096);
        let ids = peers(2);
        write_prefixed(
            &mut sender,
            MeshRequest::Tunnel {
                target: ids[1].clone(),
                visited: vec![ids[0].clone()],
            },
            MAX_CONTROL_FRAME,
        )
        .await
        .unwrap();
        // An opaque TLS-like record immediately follows the bootstrap frame.
        let sentinel = [0x16, 0x03, 0x03, 0x00, 0x01, 0xfe];
        sender.write_all(&sentinel).await.unwrap();
        let request: MeshRequest = read_prefixed(&mut receiver, MAX_CONTROL_FRAME)
            .await
            .unwrap();
        match request {
            MeshRequest::Tunnel { target, visited } => {
                assert_eq!(target, ids[1]);
                assert_eq!(visited, ids[..1]);
            }
            _ => panic!("wrong bootstrap request"),
        }
        let mut actual = [0; 6];
        receiver.read_exact(&mut actual).await.unwrap();
        assert_eq!(actual, sentinel);
        // Length delimiting does not close the stream after reading a frame.
        write_prefixed(
            &mut sender,
            MeshReply::Ready { path: ids.clone() },
            MAX_CONTROL_FRAME,
        )
        .await
        .unwrap();
        let reply: MeshReply = read_prefixed(&mut receiver, MAX_CONTROL_FRAME)
            .await
            .unwrap();
        assert!(matches!(reply, MeshReply::Ready { path } if path == ids));
    });
}

#[test]
fn oversized_length_is_rejected_without_waiting_for_its_body() {
    run(async {
        for limit in [MAX_CONTROL_FRAME, MAX_FRAME_BYTES] {
            let (mut sender, mut receiver) = tokio::io::duplex(8);
            sender.write_u32((limit + 1) as u32).await.unwrap();
            let error = tokio::time::timeout(
                Duration::from_secs(1),
                read_prefixed::<_, serde_json::Value>(&mut receiver, limit),
            )
            .await
            .expect("must reject the header without waiting for body")
            .unwrap_err();
            assert_eq!(error, RemoteError::FrameTooLarge);
        }
    });
}

#[test]
fn malformed_zero_and_truncated_prefixed_frames_are_distinct() {
    run(async {
        let (mut sender, mut receiver) = tokio::io::duplex(64);
        sender.write_u32(0).await.unwrap();
        assert_eq!(
            read_prefixed::<_, serde_json::Value>(&mut receiver, 64)
                .await
                .unwrap_err(),
            RemoteError::Protocol
        );
        sender.write_u32(1).await.unwrap();
        sender.write_all(b"{").await.unwrap();
        assert_eq!(
            read_prefixed::<_, serde_json::Value>(&mut receiver, 64)
                .await
                .unwrap_err(),
            RemoteError::Protocol
        );
        sender.write_u32(8).await.unwrap();
        sender.write_all(b"{\"").await.unwrap();
        drop(sender);
        assert_eq!(
            read_prefixed::<_, serde_json::Value>(&mut receiver, 64)
                .await
                .unwrap_err(),
            RemoteError::Unavailable
        );
    });
}

#[test]
fn oversized_output_is_rejected_before_writing_a_header() {
    run(async {
        let (mut sender, mut receiver) = tokio::io::duplex(64);
        assert_eq!(
            write_prefixed(&mut sender, "x".repeat(65), 64).await,
            Err(RemoteError::FrameTooLarge)
        );
        // A subsequent valid frame remains readable, so no partial header escaped.
        write_prefixed(&mut sender, "small", 64).await.unwrap();
        assert_eq!(
            read_prefixed::<_, String>(&mut receiver, 64).await.unwrap(),
            "small"
        );
    });
}

#[test]
fn cancelled_queued_blocking_work_does_not_mutate_the_store() {
    struct Dropped(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap()
        .block_on(async {
            let (started, start) = tokio::sync::oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let occupying = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                let _ = blocked.recv();
            });
            start.await.unwrap();
            let (dropped, did_drop) = tokio::sync::oneshot::channel();
            let probe = Dropped(Some(dropped));
            let mutations = Arc::new(AtomicU64::new(0));
            let observed = mutations.clone();
            let mut queued = Box::pin(blocking(move || {
                let _probe = probe;
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
            // Poll while the only blocking worker is occupied, then cancel the
            // stream before this queued store-like mutation can begin.
            std::future::poll_fn(|cx| {
                assert!(queued.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            drop(queued);
            release.send(()).unwrap();
            occupying.await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), did_drop)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(mutations.load(Ordering::SeqCst), 0);
        });
}

#[test]
fn direct_mesh_replies_survive_peer_shutdown_and_reuse_preserves_origin() {
    run(async {
        tokio::time::timeout(Duration::from_secs(20), async {
            let origins = Arc::new(MeshOrigins::default());
            let a = mesh_device("Synthetic A", None).await;
            let b = mesh_device("Synthetic B", Some(origins.clone())).await;
            let a_id = a.descriptor().unwrap().endpoint_id;
            let b_id = b.descriptor().unwrap().endpoint_id;
            pair_members(&a, &b).await;
            let network = a.network().unwrap();
            let session = ClientSession::new().unwrap();
            let mut physical: Option<Arc<ConnectionSlot>> = None;

            for _ in 0..3 {
                // The immediate native peer is also the TLS/business target. Its
                // receive half can close as soon as it has written the reply.
                let client = connect(network.clone(), &b_id, session.clone())
                    .await
                    .unwrap();
                assert_eq!(client.descriptor().endpoint_id, b_id);
                assert!(client.via_device_ids().is_empty());
                if let Some(shared) = &physical {
                    assert!(Arc::ptr_eq(shared, &client.connection));
                } else {
                    physical = Some(client.connection.clone());
                }
                mesh_read(&client).await.unwrap();
                client.close();
                drop(client);
                assert!(!physical.as_ref().unwrap().status().closed);
            }
            assert_eq!(*origins.0.lock().unwrap(), vec![a_id; 3]);
            a.close().await.unwrap();
            b.close().await.unwrap();
        })
        .await
        .unwrap();
    });
}

#[derive(Default)]
struct MeshOrigins(Mutex<Vec<String>>);
impl crate::Handler for MeshOrigins {
    fn execute(
        &self,
        context: crate::RequestContext,
        _command: velune_application_api::Command,
    ) -> crate::HandlerFuture<'_> {
        Box::pin(async move {
            self.0.lock().unwrap().push(context.peer_id);
            Ok(velune_application_api::ApplicationReply {
                revision: velune_application_api::Revision(1),
                response: velune_application_api::Response::ReadConversation(
                    velune_application_api::SnapshotResult {
                        submission_receipt: None,
                        snapshot: None,
                        submitted_message_id: None,
                    },
                ),
            })
        })
    }
}

async fn mesh_device(
    name: &str,
    handler: Option<Arc<dyn crate::Handler>>,
) -> Arc<crate::RemoteAccessService> {
    Arc::new(
        crate::RemoteAccessService::bind(
            crate::DeviceIdentity::generate(name.into()).unwrap(),
            crate::EndpointConfig {
                relay: crate::RelayConfig::Disabled,
                discovery: crate::DiscoveryConfig::None,
                local_discovery: false,
                bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
                request_timeout: Duration::from_secs(5),
                ..crate::EndpointConfig::default()
            },
            Arc::new(crate::DeviceStore::memory()),
            handler,
        )
        .await
        .unwrap(),
    )
}

async fn pair_members(
    controller: &Arc<crate::RemoteAccessService>,
    host: &Arc<crate::RemoteAccessService>,
) {
    let offer = host.create_invitation(Duration::from_secs(60)).unwrap();
    let attempt = controller.begin_pairing(&offer).await.unwrap();
    host.approve_pairing(
        &attempt.attempt_id,
        &controller.descriptor().unwrap().endpoint_id,
    )
    .unwrap();
    controller
        .pairing_status(&host.descriptor().unwrap().endpoint_id)
        .await
        .unwrap();
}

async fn wait_for_member(network: &Arc<NetworkGeneration>, peer: &str) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if member_contact(network, peer).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background member discovery did not reach the target");
}

async fn mesh_read(client: &RemoteClient) -> Result<()> {
    let id = client.next_request_id()?;
    let mut status = client
        .submit(
            id.clone(),
            velune_application_api::ClientViewId("synthetic-member-view".into()),
            Default::default(),
            velune_application_api::Command::ReadConversation {
                runtime_id: "synthetic-runtime".into(),
                conversation_id: "synthetic-conversation".into(),
            },
        )
        .await?;
    while matches!(status, crate::RequestStatus::Running) {
        tokio::task::yield_now().await;
        status = client.query(id.clone()).await?;
    }
    match status {
        crate::RequestStatus::Completed(Ok(_)) => Ok(()),
        crate::RequestStatus::Failed(error) => Err(error),
        _ => panic!("unexpected synthetic business result"),
    }
}

#[test]
fn handlerless_phone_discovers_and_relays_computers_with_business_control_disabled() {
    run(async {
        tokio::time::timeout(Duration::from_secs(40), async {
            let a_calls = Arc::new(MeshOrigins::default());
            let c_calls = Arc::new(MeshOrigins::default());
            let a = mesh_device("Synthetic computer A", Some(a_calls.clone())).await;
            let p = mesh_device("Synthetic phone P", None).await;
            let c = mesh_device("Synthetic computer C", Some(c_calls.clone())).await;
            let a_id = a.descriptor().unwrap().endpoint_id;
            let p_id = p.descriptor().unwrap().endpoint_id;
            let c_id = c.descriptor().unwrap().endpoint_id;
            let a_net = a.network().unwrap();
            let p_net = p.network().unwrap();
            let c_net = c.network().unwrap();
            a_net.block_outgoing_for_test(&c_id, true).unwrap();
            c_net.block_outgoing_for_test(&a_id, true).unwrap();

            // P only initiates pairing. It never opens a business ingress or
            // installs a handler, and A/C never pair with one another.
            pair_members(&p, &a).await;
            pair_members(&p, &c).await;
            assert!(!p.state.access().unwrap().ingress);
            assert!(!p.state.access().unwrap().profile_ingress);
            wait_for_member(&a_net, &c_id).await;
            wait_for_member(&c_net, &a_id).await;
            assert!(a.state.store.allows_control(&a_id, &c_id).unwrap());
            assert!(c.state.store.allows_control(&c_id, &a_id).unwrap());

            // P advertises that it cannot execute business, yet remains a mesh
            // member and usable first hop. Sync authenticates this role itself.
            let pc = connection(&p_net, &c_id).await.unwrap();
            sync_peer(&p_net, &pc).await.unwrap();
            let ap = connection(&a_net, &p_id).await.unwrap();
            sync_peer(&a_net, &ap).await.unwrap();
            assert!(a.state.store.is_member(&p_id).unwrap());
            assert!(p.state.store.is_member(&c_id).unwrap());
            assert_eq!(
                a.state
                    .store
                    .snapshot()
                    .unwrap()
                    .members
                    .into_iter()
                    .find(|member| member.peer_id == p_id)
                    .unwrap()
                    .can_execute,
                Some(false),
            );
            assert!(!a.state.store.contact(&p_id).unwrap().control_authorized);

            let before = p_net.mesh.forwarded_streams_for_test();
            let to_c = connect(a_net.clone(), &c_id, ClientSession::new().unwrap())
                .await
                .unwrap();
            assert_eq!(to_c.via_device_ids(), vec![p_id.clone()]);
            assert!(Arc::ptr_eq(&to_c.connection, &ap.slot));
            mesh_read(&to_c).await.unwrap();
            assert!(p_net.mesh.forwarded_streams_for_test() > before);
            assert_eq!(*c_calls.0.lock().unwrap(), vec![a_id.clone()]);

            let to_a = connect(c_net.clone(), &a_id, ClientSession::new().unwrap())
                .await
                .unwrap();
            assert_eq!(to_a.via_device_ids(), vec![p_id.clone()]);
            mesh_read(&to_a).await.unwrap();
            assert_eq!(*a_calls.0.lock().unwrap(), vec![c_id.clone()]);

            // Disabling execution on the final target is still enforced by its
            // service; the same native first hop and membership remain usable.
            c.set_control_ingress(false).unwrap();
            assert_eq!(mesh_read(&to_c).await, Err(RemoteError::Closed));
            assert!(to_c.route.as_ref().unwrap().is_closed());
            assert!(!ap.slot.status().closed);
            c.set_control_ingress(true).unwrap();
            let again = connect(a_net.clone(), &c_id, ClientSession::new().unwrap())
                .await
                .unwrap();
            assert!(Arc::ptr_eq(&again.connection, &ap.slot));
            mesh_read(&again).await.unwrap();

            // Local removal excludes the actual TLS origin even when a trusted
            // relay still forwards its old membership proof to this target.
            c.revoke_control(&a_id).unwrap();
            assert_eq!(mesh_read(&again).await, Err(RemoteError::Unauthorized));
            assert!(!c.state.store.is_member(&a_id).unwrap());
            assert!(p.state.store.is_member(&a_id).unwrap());
            assert!(a.state.store.is_member(&c_id).unwrap());
            assert_eq!(*c_calls.0.lock().unwrap(), vec![a_id.clone(), a_id]);
            assert!(!ap.slot.status().closed);
            assert!(!p.state.access().unwrap().ingress);
            to_a.close();
            again.close();
            a.close().await.unwrap();
            p.close().await.unwrap();
            c.close().await.unwrap();
        })
        .await
        .unwrap();
    });
}

#[test]
fn telemetry_only_peer_cannot_sync_or_open_a_mesh_tunnel() {
    run(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            let phone = mesh_device("Synthetic member phone", None).await;
            let profile_peer = mesh_device("Synthetic telemetry-only peer", None).await;
            let phone_id = phone.descriptor().unwrap().endpoint_id;
            let profile_id = profile_peer.descriptor().unwrap().endpoint_id;
            let attempt = "a".repeat(32);
            phone
                .state
                .store
                .commit_profile_grant(crate::IncomingControlGrant {
                    peer_id: profile_id.clone(),
                    name: "Synthetic profile peer".into(),
                    attempt_id: attempt.clone(),
                })
                .unwrap();
            profile_peer
                .state
                .store
                .save_pin(DeviceContact {
                    peer_id: phone_id.clone(),
                    name: "Synthetic member phone".into(),
                    locator: locator(&phone.descriptor().unwrap()),
                    confirmation: ContactConfirmation::Pending {
                        attempt_id: attempt.clone(),
                    },
                    paused: false,
                    locator_source: crate::LocatorSource::PairingOffer,
                    control_authorized: false,
                    pairing_permission: crate::PairingPermission::TelemetryProfileRead,
                })
                .unwrap();
            profile_peer
                .state
                .store
                .confirm_pin(&phone_id, &attempt)
                .unwrap();
            assert!(!phone.state.store.is_member(&profile_id).unwrap());
            assert!(!profile_peer.state.store.is_member(&phone_id).unwrap());
            assert!(matches!(
                connection(&profile_peer.network().unwrap(), &phone_id).await,
                Err(RemoteError::Unauthorized)
            ));

            // Exercise server admission too, bypassing the local client's member
            // check with an ordinary authenticated native QUIC connection.
            let native = profile_peer
                .network()
                .unwrap()
                .endpoint()
                .unwrap()
                .connect(
                    locator(&phone.descriptor().unwrap())
                        .address(&phone_id)
                        .unwrap(),
                    ALPN,
                )
                .await
                .unwrap();
            for request in [
                MeshRequest::Sync {
                    bundle: TrustBundle::default(),
                    local_descriptor: Some(profile_peer.descriptor().unwrap()),
                },
                MeshRequest::Tunnel {
                    target: phone_id.clone(),
                    visited: vec![profile_id.clone()],
                },
            ] {
                let (send, recv) = native.open_bi().await.unwrap();
                let mut io = tokio::io::join(recv, send);
                write_prefixed(&mut io, request, MAX_CONTROL_FRAME)
                    .await
                    .unwrap();
                let reply: MeshReply = read_prefixed(&mut io, MAX_CONTROL_FRAME).await.unwrap();
                assert!(matches!(reply, MeshReply::Error(RemoteError::Unauthorized)));
            }
            assert_eq!(
                phone.network().unwrap().mesh.forwarded_streams_for_test(),
                0
            );
            assert!(!phone.state.store.is_member(&profile_id).unwrap());
            native.close(0u8.into(), b"synthetic telemetry boundary complete");
            profile_peer.close().await.unwrap();
            phone.close().await.unwrap();
        })
        .await
        .unwrap();
    });
}
