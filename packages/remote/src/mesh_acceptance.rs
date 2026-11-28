//! Synthetic acceptance through the ordinary pairing and Controller entries.
//!
//! Pairing now joins a shared device network; the old A-B-C fixtures constrain
//! physical reachability, rather than imposing one-way control authority. C owns
//! execution and the request ledger while the authenticated operator remains A.
//! Network retirement must not retire that ledger or replay an uncertain effect.
//! Nothing here opens an application HOME,
//! a real session, public discovery, mDNS, or a public relay.
//!
//! The topology is enforced at the source's native dial boundary, for both ALPNs,
//! before pairing starts. Advertising an unreachable locator alone is insufficient:
//! a cache, introduction, or reverse connection could otherwise restore a shortcut.
//! A successful Controller call must also advance every intermediary's actual
//! copy_bidirectional admission counter; a route label or signed chain is no oracle.

use crate::{
    protocol::{Envelope, Reply, Request},
    trust::TrustBundle,
    *,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use velune_application_api::{
    ApplicationReply, ClientViewId, Command, CommandKind, ComposedTurn, ExecutionContext, Response,
    Revision, SnapshotResult, conversation::RuntimeInteractionReply,
};

const WAIT: Duration = Duration::from_secs(30);
const RUNTIME: &str = "synthetic-runtime";
const CONVERSATION: &str = "synthetic-conversation";
const VIEW: &str = "synthetic-mesh-view";
const HELD_MESSAGE: &str = "synthetic-effect-held-across-relay-failure";

fn run(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(120), future)
                .await
                .expect("合成设备网络验收超时");
        });
}

async fn eventually(label: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("等待失败：{label}"));
}

fn config() -> EndpointConfig {
    EndpointConfig {
        relay: RelayConfig::Disabled,
        discovery: DiscoveryConfig::None,
        local_discovery: false,
        bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
        request_timeout: Duration::from_secs(5),
        ..EndpointConfig::default()
    }
}

fn view() -> ClientViewId {
    ClientViewId(VIEW.into())
}

fn execution() -> ExecutionContext {
    ExecutionContext {
        expected_revision: Some(Revision(17)),
    }
}

fn read() -> Command {
    Command::ReadConversation {
        runtime_id: RUNTIME.into(),
        conversation_id: CONVERSATION.into(),
    }
}

fn send(text: &str) -> Command {
    Command::SendCurrentTurn {
        source_runtime_id: RUNTIME.into(),
        conversation_id: CONVERSATION.into(),
        turn: ComposedTurn {
            text: text.into(),
            effort: None,
            permission_mode: None,
            attachments: Vec::new(),
        },
    }
}

fn approve() -> Command {
    Command::ReplyRuntimeInteraction {
        runtime_id: RUNTIME.into(),
        conversation_id: CONVERSATION.into(),
        interaction_id: "synthetic-approval".into(),
        reply: RuntimeInteractionReply::Decision {
            option_id: "allow-once".into(),
        },
    }
}

#[derive(Debug, Clone)]
struct Invocation {
    operator: String,
    view: ClientViewId,
    execution: ExecutionContext,
    kind: CommandKind,
}

/// A synthetic execution owner, rather than an Agent runtime. These typed
/// commands take the production Submit/Query path and preserve their delivery
/// classification. Only the business implementation is replaced.
struct RecordingHandler {
    revision: u64,
    calls: Mutex<Vec<Invocation>>,
    sends: AtomicUsize,
    approvals: AtomicUsize,
    completed_sends: AtomicUsize,
    finish_held_send: tokio::sync::Semaphore,
}

impl RecordingHandler {
    fn new(revision: u64) -> Self {
        Self {
            revision,
            calls: Mutex::new(Vec::new()),
            sends: AtomicUsize::new(0),
            approvals: AtomicUsize::new(0),
            completed_sends: AtomicUsize::new(0),
            finish_held_send: tokio::sync::Semaphore::new(0),
        }
    }

    fn assert_operator(&self, operator: &str, expected: &[CommandKind]) {
        let calls = self.calls.lock().unwrap();
        assert_eq!(
            calls.iter().map(|call| call.kind).collect::<Vec<_>>(),
            expected
        );
        for call in calls.iter() {
            assert_eq!(call.operator, operator, "中转不得替换原始操作者");
            assert_eq!(call.view, view());
            assert_eq!(
                call.execution.expected_revision,
                execution().expected_revision
            );
        }
    }

    fn assert_idle(&self) {
        assert!(self.calls.lock().unwrap().is_empty(), "中转执行业务了");
        assert_eq!(self.sends.load(Ordering::SeqCst), 0);
        assert_eq!(self.approvals.load(Ordering::SeqCst), 0);
    }
}

impl Handler for RecordingHandler {
    fn execute(&self, context: RequestContext, command: Command) -> HandlerFuture<'_> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(Invocation {
                operator: context.peer_id,
                view: context.view,
                execution: context.execution,
                kind: command.kind(),
            });
            let mut snapshot = SnapshotResult {
                submission_receipt: None,
                snapshot: None,
                submitted_message_id: None,
            };
            let response = match command {
                Command::ReadConversation {
                    runtime_id,
                    conversation_id,
                } => {
                    assert_eq!(runtime_id, RUNTIME);
                    assert_eq!(conversation_id, CONVERSATION);
                    Response::ReadConversation(snapshot)
                }
                Command::SendCurrentTurn {
                    source_runtime_id,
                    conversation_id,
                    turn,
                } => {
                    assert_eq!(source_runtime_id, RUNTIME);
                    assert_eq!(conversation_id, CONVERSATION);
                    let number = self.sends.fetch_add(1, Ordering::SeqCst) + 1;
                    if turn.text == HELD_MESSAGE {
                        self.finish_held_send.acquire().await.unwrap().forget();
                    }
                    snapshot.submitted_message_id = Some(format!(
                        "synthetic-owner-{}-message-{number}",
                        self.revision
                    ));
                    self.completed_sends.fetch_add(1, Ordering::SeqCst);
                    Response::SendTurn(snapshot)
                }
                Command::ReplyRuntimeInteraction {
                    runtime_id,
                    conversation_id,
                    interaction_id,
                    reply,
                } => {
                    assert_eq!(runtime_id, RUNTIME);
                    assert_eq!(conversation_id, CONVERSATION);
                    assert_eq!(interaction_id, "synthetic-approval");
                    assert!(
                        matches!(reply, RuntimeInteractionReply::Decision { option_id }
                        if option_id == "allow-once")
                    );
                    self.approvals.fetch_add(1, Ordering::SeqCst);
                    Response::ReplyRuntimeInteraction(snapshot)
                }
                _ => panic!("验收不应触发其他业务命令"),
            };
            Ok(ApplicationReply {
                revision: Revision(self.revision),
                response,
            })
        })
    }
}

struct Device {
    id: String,
    service: Arc<RemoteAccessService>,
    store: Arc<DeviceStore>,
    handler: Arc<RecordingHandler>,
}

impl Device {
    fn forwards(&self) -> u64 {
        self.service
            .network()
            .unwrap()
            .mesh
            .forwarded_streams_for_test()
    }
}

/// Deliberately has no synthetic Handler field: this service has no execution
/// owner at all and never enables its default-disabled business ingress.
struct Phone {
    id: String,
    service: Arc<RemoteAccessService>,
    store: Arc<DeviceStore>,
}

impl Phone {
    async fn new(name: &str) -> Self {
        let identity = DeviceIdentity::generate(name.into()).unwrap();
        let id = identity.id();
        let store = Arc::new(DeviceStore::memory());
        let service = Arc::new(
            RemoteAccessService::bind(identity, config(), store.clone(), None)
                .await
                .unwrap(),
        );
        assert!(!service.read_state().unwrap().control_ingress);
        assert!(matches!(
            service.set_control_ingress(true),
            Err(RemoteError::NoHandler)
        ));
        assert!(!service.read_state().unwrap().control_ingress);
        assert_isolated(&service);
        Self { id, service, store }
    }

    fn forwards(&self) -> u64 {
        forwarded(&self.service)
    }
}

fn assert_isolated(service: &RemoteAccessService) {
    let descriptor = service.descriptor().unwrap();
    assert!(descriptor.relay_urls.is_empty());
    assert!(descriptor.lookup_urls.is_empty());
    assert!(!descriptor.direct_addresses.is_empty());
    assert!(
        descriptor
            .direct_addresses
            .iter()
            .all(|address| address.ip().is_loopback())
    );
}

async fn full_device(name: &str, revision: u64) -> Device {
    bind_full_device(
        DeviceIdentity::generate(name.into()).unwrap(),
        Arc::new(DeviceStore::memory()),
        revision,
    )
    .await
}

async fn bind_full_device(
    identity: DeviceIdentity,
    store: Arc<DeviceStore>,
    revision: u64,
) -> Device {
    let id = identity.id();
    let handler = Arc::new(RecordingHandler::new(revision));
    let service = Arc::new(
        RemoteAccessService::bind(identity, config(), store.clone(), Some(handler.clone()))
            .await
            .unwrap(),
    );
    assert_isolated(&service);
    Device {
        id,
        service,
        store,
        handler,
    }
}

async fn persistent_full_device(home: &std::path::Path, name: &str, revision: u64) -> Device {
    bind_full_device(
        DeviceIdentity::load_or_create(&home.join("synthetic-identity.json"), name.into()).unwrap(),
        Arc::new(DeviceStore::open(&home.join("synthetic-members.json")).unwrap()),
        revision,
    )
    .await
}

fn forwarded(service: &RemoteAccessService) -> u64 {
    service.network().unwrap().mesh.forwarded_streams_for_test()
}

struct Chain {
    devices: Vec<Device>,
}

impl Chain {
    async fn new(names: &[&str]) -> Self {
        let mut devices = Vec::new();
        for (index, name) in names.iter().enumerate() {
            devices.push(full_device(name, (index as u64 + 1) * 100).await);
        }
        let chain = Self { devices };
        chain.block_shortcuts();
        chain
    }

    fn block_shortcuts(&self) {
        for (source, device) in self.devices.iter().enumerate() {
            for (target, other) in self.devices.iter().enumerate() {
                if source.abs_diff(target) > 1 {
                    device
                        .service
                        .network()
                        .unwrap()
                        .block_outgoing_for_test(&other.id, true)
                        .unwrap();
                }
            }
        }
    }

    async fn pair(&self) {
        // Only adjacent devices are paired into the shared membership. Discovery
        // of distant members must result from production background exchange.
        for adjacent in self.devices.windows(2) {
            pair_devices(&adjacent[0], &adjacent[1]).await;
        }
    }

    async fn discover(&self, source: usize, target: usize) {
        let (source, target) = (&self.devices[source], &self.devices[target]);
        eventually("自动发现已传递授权的目标", || {
            source
                .service
                .read_state()
                .unwrap()
                .contacts
                .iter()
                .any(|contact| {
                    contact.peer_id == target.id
                        && contact.control_authorized
                        && matches!(contact.confirmation, ContactConfirmation::Confirmed { .. })
                })
        })
        .await;
        eventually("双方本机成员根均已取得互控证明", || {
            source.store.allows_control(&source.id, &target.id).unwrap()
                && target.store.allows_control(&target.id, &source.id).unwrap()
        })
        .await;
        assert_local_root(&source.store, &source.id, &target.id);
        assert_local_root(&target.store, &target.id, &source.id);
        // The same advertised target addresses really cannot be dialed from A.
        assert!(
            matches!(
                source
                    .service
                    .connect_direct_client(&target.id, ClientSession::new().unwrap())
                    .await,
                Err(RemoteError::Unavailable)
            ),
            "拓扑未阻断普通 ALPN 的直达捷径"
        );
    }

    async fn connect_last(&self) -> Arc<Controller> {
        let first = &self.devices[0];
        let last = self.devices.last().unwrap();
        let controller = first.service.controller(&last.id).unwrap();
        tokio::time::timeout(WAIT, controller.connect())
            .await
            .unwrap()
            .unwrap();
        self.assert_route(&controller);
        assert_eq!(controller.descriptor().unwrap().endpoint_id, last.id);
        assert_eq!(
            controller.descriptor().unwrap().incarnation,
            last.service.descriptor().unwrap().incarnation
        );
        controller
    }

    fn assert_route(&self, controller: &Controller) {
        let expected: Vec<_> = self.devices[1..self.devices.len() - 1]
            .iter()
            .map(|device| device.id.clone())
            .collect();
        assert_via(controller, &expected);
    }

    async fn close(&self) {
        for device in &self.devices {
            device.service.close().await.unwrap();
        }
    }
}

async fn pair_devices(operator: &Device, target: &Device) {
    pair_services(&operator.service, &target.service).await;
}

async fn pair_services(operator: &Arc<RemoteAccessService>, target: &Arc<RemoteAccessService>) {
    let operator_id = operator.descriptor().unwrap().endpoint_id;
    let target_id = target.descriptor().unwrap().endpoint_id;
    let offer = target.create_invitation(Duration::from_secs(60)).unwrap();
    let pending = operator.begin_pairing(&offer).await.unwrap();
    assert!(matches!(pending.permission, PairingPermission::Control));
    target
        .approve_pairing(&pending.attempt_id, &operator_id)
        .unwrap();
    let status = operator.pairing_status(&target_id).await.unwrap();
    assert!(matches!(status.phase, PairingPhase::Granted));
    assert!(
        target
            .read_state()
            .unwrap()
            .incoming_grants
            .iter()
            .any(|grant| grant.peer_id == operator_id)
    );
}

fn block_link(left: &Device, right: &Device, blocked: bool) {
    block_services(&left.service, &right.service, blocked);
}

fn block_services(left: &RemoteAccessService, right: &RemoteAccessService, blocked: bool) {
    let left_id = left.descriptor().unwrap().endpoint_id;
    let right_id = right.descriptor().unwrap().endpoint_id;
    left.network()
        .unwrap()
        .block_outgoing_for_test(&right_id, blocked)
        .unwrap();
    right
        .network()
        .unwrap()
        .block_outgoing_for_test(&left_id, blocked)
        .unwrap();
}

fn assert_local_root(store: &DeviceStore, target: &str, operator: &str) {
    let proof = store.membership_proof(operator).unwrap();
    assert_eq!(proof.root, target);
    assert_eq!(proof.member, operator);
    let chain = &proof.statements;
    assert!(!chain.is_empty());
    assert_eq!(
        chain[0].issuer, target,
        "互控资格必须根植于目标本机正常配对形成的签名，不能只导入远端 grant"
    );
    assert!(chain.iter().all(|statement| !statement.revoked));
    assert_eq!(chain.last().unwrap().subject, operator);
    for adjacent in chain.windows(2) {
        assert_eq!(adjacent[0].subject, adjacent[1].issuer);
    }
    for statement in chain {
        statement.verify().unwrap();
    }
}

fn assert_phone_member(device: &Device, phone: &Phone) {
    assert!(device.store.is_member(&phone.id).unwrap());
    assert!(!device.store.contact(&phone.id).unwrap().control_authorized);
    let stored = device.store.snapshot().unwrap();
    let member = stored
        .members
        .iter()
        .find(|member| member.peer_id == phone.id)
        .expect("不可执行的手机仍须进入可见、可撤销的本机成员投影");
    assert_eq!(member.can_execute, Some(false));
    assert!(
        device
            .service
            .read_state()
            .unwrap()
            .members
            .iter()
            .any(|member| member.peer_id == phone.id && member.can_execute == Some(false))
    );
}

fn assert_via(controller: &Controller, expected: &[String]) {
    let status = controller.status().unwrap();
    assert!(
        matches!(status.phase, ControllerPhase::Connected),
        "{}",
        serde_json::to_string(&status).unwrap()
    );
    assert_eq!(status.via_device_ids, expected, "必须报告实际的完整中转链");
}

async fn next_path(observer: &mut PathObserver) -> PathObservation {
    tokio::time::timeout(WAIT, observer.next())
        .await
        .expect("路径观察未返回状态")
        .expect("路径观察应先返回快照")
}

fn assert_routed_path(observation: &PathObservation, target: &str, firsthop_id: u64) {
    assert_eq!(
        observation.status.peer_id, target,
        "路径观察必须归属逻辑目标"
    );
    assert_eq!(
        observation.status.connection_id, firsthop_id,
        "中转路径统计仍来源于物理首跳"
    );
    assert!(
        !observation.status.paths.is_empty(),
        "不能用空路径绕过类型检查"
    );
    assert!(
        observation
            .status
            .paths
            .iter()
            .all(|path| path.kind == PathKind::Other),
        "A 到 B 的物理直连不能被呈现为 A 到 C 的直连"
    );
}

async fn exercise_business(chain: &Chain, controller: &Controller) {
    let target = chain.devices.last().unwrap();
    let intermediaries = &chain.devices[1..chain.devices.len() - 1];
    let relays: Vec<_> = intermediaries
        .iter()
        .map(|device| &device.service)
        .collect();
    let via: Vec<_> = intermediaries
        .iter()
        .map(|device| device.id.clone())
        .collect();
    exercise_requests(controller, target, &relays, &via).await;
    target
        .handler
        .assert_operator(&chain.devices[0].id, &business_kinds());
    assert_eq!(target.handler.sends.load(Ordering::SeqCst), 1);
    assert_eq!(target.handler.approvals.load(Ordering::SeqCst), 1);
    for device in &chain.devices[..chain.devices.len() - 1] {
        device.handler.assert_idle();
    }
    assert!(
        chain.devices[0]
            .store
            .snapshot()
            .unwrap()
            .pending
            .is_empty()
    );
    assert_eq!(
        chain.devices[0]
            .service
            .read_state()
            .unwrap()
            .contacts
            .iter()
            .filter(|contact| contact.peer_id == target.id)
            .count(),
        1,
        "反复交换介绍不得复制目标身份"
    );
}

fn business_kinds() -> [CommandKind; 3] {
    [
        CommandKind::ReadConversation,
        CommandKind::SendTurn,
        CommandKind::ReplyRuntimeInteraction,
    ]
}

#[test]
fn adjacent_alpha_envelopes_keep_reads_and_single_writes_over_direct_and_routed_links() {
    run(async {
        let chain = Chain::new(&["compat-client", "compat-relay", "compat-host"]).await;
        chain.pair().await;
        chain.discover(0, 2).await;
        let source = &chain.devices[0];
        for host in &chain.devices[1..] {
            let mut legacy_client = source
                .service
                .connect_client(&host.id, ClientSession::new().unwrap())
                .await
                .unwrap();
            assert_eq!(
                legacy_client.wire_version,
                crate::protocol::APPLICATION_WIRE_VERSION
            );
            // The already released client always sends v3 request envelopes.
            legacy_client.wire_version = PROTOCOL_VERSION;
            assert!(matches!(
                legacy_client.rpc(Request::Hello).await.unwrap(),
                Reply::Hello(_)
            ));
            assert!(matches!(
                legacy_client
                    .call(view(), execution(), read())
                    .await
                    .unwrap()
                    .response,
                Response::ReadConversation(_)
            ));
            legacy_client
                .call(view(), execution(), send("legacy client"))
                .await
                .unwrap();
            assert_eq!(host.handler.sends.load(Ordering::SeqCst), 1);

            // A released host rejects the v4 Hello before dispatch.
            host.service
                .network()
                .unwrap()
                .legacy_wire_only
                .store(true, Ordering::Release);
            let modern_client = source
                .service
                .connect_client(&host.id, ClientSession::new().unwrap())
                .await
                .unwrap();
            assert_eq!(modern_client.wire_version, PROTOCOL_VERSION);
            assert_eq!(
                modern_client.via_device_ids(),
                if host.id == chain.devices[1].id {
                    Vec::new()
                } else {
                    vec![chain.devices[1].id.clone()]
                }
            );
            assert!(matches!(
                modern_client
                    .call(view(), execution(), read())
                    .await
                    .unwrap()
                    .response,
                Response::ReadConversation(_)
            ));
            modern_client
                .call(view(), execution(), send("modern client"))
                .await
                .unwrap();
            assert_eq!(host.handler.sends.load(Ordering::SeqCst), 2);
        }
    });
}

async fn exercise_requests(
    controller: &Controller,
    target: &Device,
    relays: &[&Arc<RemoteAccessService>],
    via: &[String],
) {
    for (command, kind) in [
        (read(), CommandKind::ReadConversation),
        (send("synthetic-message"), CommandKind::SendTurn),
        (approve(), CommandKind::ReplyRuntimeInteraction),
    ] {
        let before: Vec<_> = relays.iter().map(|relay| forwarded(relay)).collect();
        let reply = controller
            .request(view(), execution(), command)
            .await
            .unwrap_or_else(|error| panic!("目标业务调用失败：{error:?}"));
        assert_eq!(reply.revision, Revision(target.handler.revision));
        match (kind, reply.response) {
            (CommandKind::ReadConversation, Response::ReadConversation(_)) => {}
            (CommandKind::SendTurn, Response::SendTurn(snapshot)) => {
                assert_eq!(
                    snapshot.submitted_message_id,
                    Some(format!(
                        "synthetic-owner-{}-message-1",
                        target.handler.revision
                    ))
                );
            }
            (CommandKind::ReplyRuntimeInteraction, Response::ReplyRuntimeInteraction(_)) => {}
            _ => panic!("业务响应必须由最终目标按原命令返回"),
        }
        assert_via(controller, via);
        for (relay, previous) in relays.iter().zip(before) {
            assert!(
                forwarded(relay) > previous,
                "本次业务调用未进入中转 {} 的真实字节通道",
                relay.descriptor().unwrap().endpoint_id
            );
        }
    }
}

async fn discover_member(source: &Arc<RemoteAccessService>, target: &str) {
    eventually(
        "标准配对成员自动发现目标并取得控制资格",
        || {
            source.read_state().unwrap().contacts.iter().any(|contact| {
                contact.peer_id == target
                    && contact.control_authorized
                    && matches!(contact.confirmation, ContactConfirmation::Confirmed { .. })
            })
        },
    )
    .await;
}

async fn connect_member(
    source: &Arc<RemoteAccessService>,
    target: &Device,
    via: &[String],
) -> Arc<Controller> {
    let controller = source.controller(&target.id).unwrap();
    tokio::time::timeout(WAIT, controller.connect())
        .await
        .unwrap()
        .unwrap();
    assert_via(&controller, via);
    assert_eq!(controller.descriptor().unwrap().endpoint_id, target.id);
    assert_eq!(
        controller.descriptor().unwrap().incarnation,
        target.service.descriptor().unwrap().incarnation
    );
    controller
}

#[test]
fn phone_hub_without_handler_enables_mutual_control_and_disabled_full_member_still_routes() {
    run(async {
        let p = Phone::new("synthetic-phone-P").await;
        let a = full_device("synthetic-full-A", 100).await;
        let c = full_device("synthetic-full-C", 300).await;
        let d = full_device("synthetic-new-full-D", 400).await;
        // Initially only P-A and P-C can communicate. Every other pair is
        // blocked before any pairing, including D until its later introduction.
        for (left, right) in [
            (&a.service, &c.service),
            (&a.service, &d.service),
            (&p.service, &d.service),
            (&c.service, &d.service),
        ] {
            block_services(left, right, true);
        }
        pair_services(&p.service, &a.service).await;
        pair_services(&p.service, &c.service).await;
        discover_member(&a.service, &c.id).await;
        discover_member(&c.service, &a.id).await;
        assert_local_root(&a.store, &a.id, &c.id);
        assert_local_root(&c.store, &c.id, &a.id);
        eventually(
            "手机成员可中转但不被标为业务执行目标",
            || {
                [&a.store, &c.store].iter().all(|store| {
                    store.is_member(&p.id).unwrap()
                        && !store.contact(&p.id).unwrap().control_authorized
                })
            },
        )
        .await;
        assert_phone_member(&a, &p);
        assert_phone_member(&c, &p);
        assert!(!p.service.read_state().unwrap().control_ingress);
        for (source, target) in [(&a.service, &c.id), (&c.service, &a.id)] {
            assert!(
                matches!(
                    source
                        .connect_direct_client(target, ClientSession::new().unwrap())
                        .await,
                    Err(RemoteError::Unavailable)
                ),
                "A/C 的普通入口不得偷偷直达"
            );
        }
        let a_to_c = connect_member(&a.service, &c, std::slice::from_ref(&p.id)).await;
        exercise_requests(&a_to_c, &c, &[&p.service], std::slice::from_ref(&p.id)).await;
        let c_to_a = connect_member(&c.service, &a, std::slice::from_ref(&p.id)).await;
        exercise_requests(&c_to_a, &a, &[&p.service], std::slice::from_ref(&p.id)).await;
        c.handler.assert_operator(&a.id, &business_kinds());
        a.handler.assert_operator(&c.id, &business_kinds());
        assert_eq!(a.handler.sends.load(Ordering::SeqCst), 1);
        assert_eq!(a.handler.approvals.load(Ordering::SeqCst), 1);
        assert_eq!(c.handler.sends.load(Ordering::SeqCst), 1);
        assert_eq!(c.handler.approvals.load(Ordering::SeqCst), 1);
        assert!(p.forwards() > 0);
        assert!(!p.service.read_state().unwrap().control_ingress);
        assert!(
            p.store.snapshot().unwrap().pending.is_empty(),
            "手机不能把过路副作用变成自己的请求"
        );

        a_to_c.set_activity(false, true).unwrap();
        c_to_a.set_activity(false, true).unwrap();
        let old_member_proof = a
            .store
            .export_trust_bundle(Some(&a.service.descriptor().unwrap()))
            .unwrap();
        c.service.revoke_control(&p.id).unwrap();
        assert!(!c.store.is_member(&p.id).unwrap());
        assert!(
            !c.store.is_member(&a.id).unwrap(),
            "C 的唯一成员根经过 P，排除 P 后不能继续把 A 当成员"
        );
        assert!(!c.store.allows_control(&c.id, &a.id).unwrap());
        assert!(matches!(
            c.store.merge_trust_bundle(&p.id, &old_member_proof),
            Err(RemoteError::Unauthorized)
        ));
        // Only a fresh explicit pairing clears the local P exclusion. It must
        // restore the ordinary member root, without importing test-only grants.
        pair_services(&p.service, &c.service).await;
        eventually("新明确配对恢复经 P 的成员根", || {
            c.store.is_member(&p.id).unwrap()
                && c.store.is_member(&a.id).unwrap()
                && c.store.allows_control(&c.id, &a.id).unwrap()
        })
        .await;
        assert_local_root(&c.store, &c.id, &a.id);
        a.service.set_control_ingress(false).unwrap();
        // A now sits between the handler-free phone and a newly introduced D.
        // The test requires fresh exchange after ingress closes, not just use
        // of an already cached target/route. It does not re-enable A's ingress.
        block_services(&p.service, &c.service, true);
        block_services(&a.service, &c.service, false);
        block_services(&c.service, &d.service, false);
        pair_devices(&c, &d).await;
        discover_member(&p.service, &d.id).await;
        let p_to_d = connect_member(&p.service, &d, &[a.id.clone(), c.id.clone()]).await;
        exercise_requests(
            &p_to_d,
            &d,
            &[&a.service, &c.service],
            &[a.id.clone(), c.id.clone()],
        )
        .await;
        d.handler.assert_operator(&p.id, &business_kinds());
        a.handler.assert_operator(&c.id, &business_kinds());
        c.handler.assert_operator(&a.id, &business_kinds());
        assert!(!a.service.read_state().unwrap().control_ingress);
        assert!(!p.service.read_state().unwrap().control_ingress);
        assert!(p.store.snapshot().unwrap().pending.is_empty());
        p_to_d.disconnect().unwrap();
        for service in [&p.service, &a.service, &c.service, &d.service] {
            service.close().await.unwrap();
        }
    });
}

#[test]
fn automatic_discovery_and_controller_business_really_traverse_a_b_c() {
    run(async {
        let chain = Chain::new(&["synthetic-A", "synthetic-B", "synthetic-C"]).await;
        chain.pair().await;
        chain.discover(0, 2).await;
        let controller = chain.connect_last().await;
        let (a, b, c) = (&chain.devices[0], &chain.devices[1], &chain.devices[2]);
        let original_client = controller.client().unwrap();
        let firsthop = original_client.connection.clone();
        let physical = firsthop.status();
        assert_eq!(physical.peer_id, b.id);
        assert!(!physical.closed);
        assert!(
            physical
                .paths
                .iter()
                .any(|path| path.selected && path.kind == PathKind::Direct)
        );
        let mut observer = controller.observe_paths().unwrap();
        let initial = next_path(&mut observer).await;
        assert_routed_path(&initial, &c.id, physical.connection_id);
        assert!(!initial.status.closed);
        exercise_business(&chain, &controller).await;

        // Wait from an already-live observation, so closing this logical route
        // must wake next() even though the shared native A->B path stays alive.
        let observed_target = c.id.clone();
        let ending = tokio::spawn(async move {
            loop {
                let observation = next_path(&mut observer).await;
                assert_routed_path(&observation, &observed_target, physical.connection_id);
                if observation.status.closed {
                    assert!(observer.next().await.is_none());
                    break;
                }
            }
        });
        tokio::task::yield_now().await;
        controller.disconnect().unwrap();
        drop(original_client);
        tokio::time::timeout(WAIT, ending).await.unwrap().unwrap();
        assert!(
            !firsthop.status().closed,
            "结束 C 代理或其路径观察误关了共享首跳"
        );
        assert!(firsthop.native().is_some());

        // A second target uses that exact cached physical connection. Its Hello
        // is authenticated by B and does not invoke any synthetic business.
        let other_target = crate::mesh::connect(
            a.service.network().unwrap(),
            &b.id,
            ClientSession::new().unwrap(),
        )
        .await
        .unwrap();
        assert!(
            Arc::ptr_eq(&other_target.connection, &firsthop),
            "其他目标应仍可复用原首跳，不能靠重拨掩盖误关闭"
        );
        assert_eq!(other_target.descriptor().endpoint_id, b.id);
        assert!(!other_target.connection_status().closed);
        other_target.close();
        drop(other_target);
        assert!(!firsthop.status().closed);

        // Resume the same C Controller while its direct source path is still
        // blocked, then restore reachability without another connect() call.
        controller.connect().await.unwrap();
        assert_via(&controller, std::slice::from_ref(&b.id));
        assert!(Arc::ptr_eq(
            &controller.client().unwrap().connection,
            &firsthop
        ));
        let incarnation = controller.descriptor().unwrap().incarnation;
        let mut state_observer = a.service.observe_state();
        let baseline = state_observer
            .next(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let baseline_controller = baseline
            .controllers
            .iter()
            .find(|status| status.peer_id == c.id)
            .unwrap();
        assert_eq!(
            baseline_controller.via_device_ids,
            std::slice::from_ref(&b.id)
        );
        let before_upgrade = a.service.read_state().unwrap();
        assert!(before_upgrade.version >= baseline.version);
        block_link(a, c, false);

        // Consume the public snapshot stream, not just Controller::status():
        // the ten-second probe can keep phase=Connected while replacing the
        // route, and the UI needs a new version containing its new epoch/via.
        let published = tokio::time::timeout(WAIT, async {
            loop {
                if let Some(snapshot) = state_observer.next(Duration::from_secs(5)).await.unwrap()
                    && snapshot.controllers.iter().any(|status| {
                        status.peer_id == c.id
                            && status.phase == ControllerPhase::Connected
                            && status.connection_epoch > baseline_controller.connection_epoch
                            && status.via_device_ids.is_empty()
                    })
                {
                    assert!(
                        snapshot.version > before_upgrade.version,
                        "直达升级必须推进公开快照版本，不能只改变 Controller 内部字段"
                    );
                    break snapshot;
                }
            }
        })
        .await
        .expect("状态观察者未收到 C 从 B 中转升级直达的版本/epoch/via 变化");
        let current = a.service.read_state().unwrap();
        assert!(current.version >= published.version && current.version > before_upgrade.version);
        let current_controller = current
            .controllers
            .iter()
            .find(|status| status.peer_id == c.id)
            .unwrap();
        assert!(current_controller.via_device_ids.is_empty());
        assert!(current_controller.connection_epoch > baseline_controller.connection_epoch);
        assert_eq!(
            current_controller.intent_revision,
            baseline_controller.intent_revision
        );
        assert_via(&controller, &[]);
        assert!(Arc::ptr_eq(
            &controller,
            &a.service.controller(&c.id).unwrap()
        ));
        assert_eq!(controller.descriptor().unwrap().incarnation, incarnation);
        let direct = controller.client().unwrap();
        assert_eq!(direct.connection.status().peer_id, c.id);
        assert!(!Arc::ptr_eq(&direct.connection, &firsthop));
        let mut direct_paths = controller.observe_paths().unwrap();
        let observation = next_path(&mut direct_paths).await;
        assert_eq!(observation.status.peer_id, c.id);
        assert!(!observation.status.closed);
        assert!(
            observation
                .status
                .paths
                .iter()
                .any(|path| path.selected && path.kind == PathKind::Direct)
        );
        let before_direct_read = b.forwards();
        let reply = controller
            .request(view(), execution(), read())
            .await
            .unwrap();
        assert_eq!(reply.revision, Revision(c.handler.revision));
        assert!(matches!(reply.response, Response::ReadConversation(_)));
        assert_eq!(
            b.forwards(),
            before_direct_read,
            "升级后的读取不应再由 B 转送"
        );
        assert!(!firsthop.status().closed, "升级直达不得关闭共享的 A-B 首跳");
        c.handler.assert_operator(
            &a.id,
            &[
                CommandKind::ReadConversation,
                CommandKind::SendTurn,
                CommandKind::ReplyRuntimeInteraction,
                CommandKind::ReadConversation,
            ],
        );
        a.handler.assert_idle();
        b.handler.assert_idle();
        controller.disconnect().unwrap();
        drop(direct_paths);
        drop(direct);
        assert!(!firsthop.status().closed);
        chain.close().await;
        assert!(firsthop.status().closed, "首跳应由所属网络代际最终关闭");
    });
}

#[test]
fn controller_business_traverses_both_relays_in_a_b_c_d() {
    run(async {
        let chain = Chain::new(&["synthetic-A", "synthetic-B", "synthetic-C", "synthetic-D"]).await;
        chain.pair().await;
        chain.discover(0, 3).await;
        chain.discover(1, 3).await;
        let controller = chain.connect_last().await;
        exercise_business(&chain, &controller).await;
        chain.close().await;
    });
}

#[test]
fn existing_controller_automatically_fails_over_from_b_to_d_for_same_c_request() {
    run(async {
        let topology =
            Chain::new(&["synthetic-A", "synthetic-B", "synthetic-C", "synthetic-D"]).await;
        let (a, b, c, d) = (
            &topology.devices[0],
            &topology.devices[1],
            &topology.devices[2],
            &topology.devices[3],
        );
        // Establish B as the actual first hop before introducing the alternative:
        // choosing a random preferred neighbor would not exercise B's failure.
        pair_devices(a, b).await;
        pair_devices(b, c).await;
        topology.discover(0, 2).await;
        let controller = a.service.controller(&c.id).unwrap();
        tokio::time::timeout(WAIT, controller.connect())
            .await
            .unwrap()
            .unwrap();
        assert_via(&controller, std::slice::from_ref(&b.id));

        // The resulting diamond is A-B-C and A-D-C. A-C and B-D remain
        // source-blocked in both directions, with all public facilities off.
        block_link(a, d, false);
        pair_devices(a, d).await;
        pair_devices(d, c).await;
        eventually("A 经自动交换取得 D 对 C 的介绍", || {
            a.store.introducers_for(&c.id).unwrap().contains(&d.id)
        })
        .await;
        assert_via(&controller, std::slice::from_ref(&b.id));
        let initial = controller.status().unwrap();
        let incarnation = controller.descriptor().unwrap().incarnation;
        let old_firsthop = controller.client().unwrap().connection.clone();
        assert_eq!(old_firsthop.status().peer_id, b.id);
        let d_before = d.forwards();

        let caller = controller.clone();
        let request = tokio::spawn(async move {
            caller
                .request(view(), execution(), send(HELD_MESSAGE))
                .await
        });
        eventually("C 已在 B 路径接纳原副作用", || {
            c.handler.sends.load(Ordering::SeqCst) == 1
        })
        .await;
        let pending = a.store.snapshot().unwrap().pending;
        assert_eq!(pending.len(), 1);
        let original_id = pending[0].request_id.clone();
        assert_eq!(pending[0].peer_id, c.id);
        assert_eq!(original_id.incarnation, incarnation);
        b.service.network().unwrap().close().await.unwrap();
        let error = tokio::time::timeout(WAIT, request)
            .await
            .unwrap()
            .unwrap()
            .err()
            .expect("完成被暂挂时不得伪造副作用成功");
        assert_eq!(error.request_id.as_ref(), Some(&original_id));
        assert!(matches!(error.failure, CallFailure::Transport(_)));

        // Keep the same Controller active. Its own recovery must select D,
        // rather than relying on connect(), a fresh proxy, or a replayed send.
        eventually("现有 Controller 自动把首跳从 B 切换为 D", || {
            let status = controller.status().unwrap();
            status.phase == ControllerPhase::Connected
                && status.connection_epoch > initial.connection_epoch
                && status.via_device_ids == [d.id.clone()]
                && d.forwards() > d_before
        })
        .await;
        assert!(old_firsthop.status().closed);
        assert!(Arc::ptr_eq(
            &controller,
            &a.service.controller(&c.id).unwrap()
        ));
        assert_eq!(controller.peer_id(), c.id);
        assert_eq!(controller.descriptor().unwrap().incarnation, incarnation);
        assert_eq!(
            controller.status().unwrap().intent_revision,
            initial.intent_revision
        );
        let pending = a.store.snapshot().unwrap().pending;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_id, original_id);
        assert_eq!(pending[0].peer_id, c.id);
        assert_eq!(c.handler.sends.load(Ordering::SeqCst), 1);

        c.handler.finish_held_send.add_permits(1);
        eventually("D 路径自动查询并完成同一个未决请求", || {
            a.store.snapshot().unwrap().pending.iter().any(|operation| {
                operation.request_id == original_id
                    && operation.peer_id == c.id
                    && operation.status == PendingStatus::Completed
            })
        })
        .await;
        let before_query = d.forwards();
        assert_completed_send(
            controller.query(original_id.clone()).await.unwrap(),
            c.handler.revision,
        );
        assert!(
            d.forwards() > before_query,
            "切换后的原结果查询必须实际经过 D"
        );
        assert_via(&controller, std::slice::from_ref(&d.id));
        let d_firsthop = controller.client().unwrap().connection.clone();
        assert_eq!(d_firsthop.status().peer_id, d.id);
        assert!(!Arc::ptr_eq(&d_firsthop, &old_firsthop));
        assert!(!d_firsthop.status().closed);
        c.handler.assert_operator(&a.id, &[CommandKind::SendTurn]);
        assert_eq!(c.handler.sends.load(Ordering::SeqCst), 1);
        assert_eq!(c.handler.completed_sends.load(Ordering::SeqCst), 1);
        a.handler.assert_idle();
        b.handler.assert_idle();
        d.handler.assert_idle();
        let pending = a.store.snapshot().unwrap().pending;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_id, original_id);
        assert_eq!(pending[0].peer_id, c.id);
        controller.acknowledge(&original_id).unwrap();
        assert!(a.store.snapshot().unwrap().pending.is_empty());
        topology.close().await;
    });
}

#[test]
fn unique_relay_failure_recovers_original_target_request_without_reexecuting_effect() {
    run(async {
        let chain = Chain::new(&["synthetic-A", "synthetic-B", "synthetic-C"]).await;
        chain.pair().await;
        chain.discover(0, 2).await;
        let (a, b, c) = (&chain.devices[0], &chain.devices[1], &chain.devices[2]);
        let controller = chain.connect_last().await;
        let original_epoch = controller.status().unwrap().connection_epoch;
        let original_incarnation = controller.descriptor().unwrap().incarnation;
        let relay_addresses = b.service.descriptor().unwrap().direct_addresses;
        let caller = controller.clone();
        let request = tokio::spawn(async move {
            caller
                .request(view(), execution(), send(HELD_MESSAGE))
                .await
        });
        eventually("C 已接纳并执行一次副作用", || {
            c.handler.sends.load(Ordering::SeqCst) == 1
        })
        .await;
        let pending = a.store.snapshot().unwrap().pending;
        assert_eq!(pending.len(), 1);
        let original_id = pending[0].request_id.clone();
        assert_eq!(pending[0].peer_id, c.id);
        assert_eq!(original_id.incarnation, original_incarnation);
        assert_eq!(c.handler.completed_sends.load(Ordering::SeqCst), 0);

        // Failure injection retires only B's native network. C's application
        // owner and request ledger keep running; no service is recreated.
        b.service.network().unwrap().close().await.unwrap();
        let error = tokio::time::timeout(WAIT, request)
            .await
            .unwrap()
            .unwrap()
            .err()
            .expect("唯一中转失效时不得伪造已完成响应");
        assert_eq!(error.request_id.as_ref(), Some(&original_id));
        assert!(matches!(error.failure, CallFailure::Transport(_)));
        assert_eq!(controller.peer_id(), c.id);
        assert!(
            controller
                .request(view(), execution(), read())
                .await
                .is_err(),
            "唯一中转失效后，A 不应仍可通过隐藏直连访问 C"
        );
        assert_eq!(
            a.store.snapshot().unwrap().pending[0].request_id,
            original_id
        );

        c.handler.finish_held_send.add_permits(1);
        eventually("断路期间 C 继续完成原业务", || {
            c.handler.completed_sends.load(Ordering::SeqCst) == 1
        })
        .await;
        // Keep B's synthetic address stable: with discovery deliberately off,
        // changing its port would test locator replacement instead of recovery
        // of a temporarily unavailable intermediary.
        let mut restored = config();
        restored.bind_addresses = relay_addresses;
        let before = b.service.network_state().unwrap().saved.unwrap();
        let applied = b
            .service
            .apply_network(NetworkSettingsUpdate {
                expected_revision: before.revision,
                config: restored,
                credentials: before
                    .credentials
                    .iter()
                    .map(|credential| RelayCredentialEdit {
                        url: credential.url.clone(),
                        credential: CredentialEdit::Keep,
                    })
                    .collect(),
                restore_lookup_urls: Vec::new(),
            })
            .await
            .unwrap();
        assert!(matches!(applied.phase, NetworkApplicationPhase::Applied));
        assert_eq!(applied.applied_revision, Some(before.revision + 1));
        assert_eq!(applied.saved.unwrap().revision, before.revision + 1);
        chain.block_shortcuts();
        // No explicit reconnect, fresh Controller, or resubmission: its existing
        // recovery loop must reconnect and query the original pending ID.
        eventually("原 Controller 自动恢复并查询原请求", || {
            let status = controller.status().unwrap();
            status.phase == ControllerPhase::Connected
                && status.connection_epoch > original_epoch
                && a.store.snapshot().unwrap().pending.iter().any(|operation| {
                    operation.request_id == original_id
                        && operation.peer_id == c.id
                        && operation.status == PendingStatus::Completed
                })
        })
        .await;
        chain.assert_route(&controller);
        assert_eq!(
            controller.descriptor().unwrap().incarnation,
            original_incarnation
        );
        for _ in 0..2 {
            let before = b.forwards();
            let status = controller.query(original_id.clone()).await.unwrap();
            assert_completed_send(status, c.handler.revision);
            assert!(b.forwards() > before, "原结果查询也必须经过恢复后的 B");
        }
        // Even an explicit duplicate of the *same* accepted request cannot
        // execute again. This uses the existing authenticated client only to
        // supply a fixed ID; ordinary callers above use Controller::request.
        let duplicate = controller
            .client()
            .unwrap()
            .submit(original_id.clone(), view(), execution(), send(HELD_MESSAGE))
            .await
            .unwrap();
        assert_completed_send(duplicate, c.handler.revision);
        c.handler.assert_operator(&a.id, &[CommandKind::SendTurn]);
        assert_eq!(c.handler.sends.load(Ordering::SeqCst), 1);
        assert_eq!(c.handler.completed_sends.load(Ordering::SeqCst), 1);
        a.handler.assert_idle();
        b.handler.assert_idle();
        controller.acknowledge(&original_id).unwrap();
        assert!(a.store.snapshot().unwrap().pending.is_empty());
        chain.close().await;
    });
}

fn assert_completed_send(status: RequestStatus, owner_revision: u64) {
    let RequestStatus::Completed(Ok(reply)) = status else {
        panic!(
            "应取回 C 的原请求结果：{}",
            serde_json::to_string(&status).unwrap()
        );
    };
    assert_eq!(reply.revision, Revision(owner_revision));
    let Response::SendTurn(snapshot) = reply.response else {
        panic!("查询必须返回原发送结果");
    };
    assert_eq!(
        snapshot.submitted_message_id,
        Some(format!("synthetic-owner-{owner_revision}-message-1"))
    );
}

#[test]
fn target_revocation_survives_member_alternate_path_stale_proof_and_restart() {
    run(async {
        let home = tempfile::tempdir().unwrap();
        let p = Phone::new("synthetic-primary-phone-P").await;
        let q = Phone::new("synthetic-alternative-phone-Q").await;
        let a = full_device("synthetic-full-A", 100).await;
        let mut c = persistent_full_device(home.path(), "synthetic-full-C", 300).await;
        block_services(&a.service, &c.service, true);
        block_services(&p.service, &q.service, true);
        pair_services(&p.service, &a.service).await;
        pair_services(&p.service, &c.service).await;
        discover_member(&a.service, &c.id).await;
        let controller = connect_member(&a.service, &c, std::slice::from_ref(&p.id)).await;
        assert_local_root(&c.store, &c.id, &a.id);
        let original_id = controller.next_request_id().unwrap();
        let refused_id = controller.next_request_id().unwrap();
        controller
            .client()
            .unwrap()
            .submit(original_id.clone(), view(), execution(), read())
            .await
            .unwrap();
        eventually("C 的原读取已执行", || {
            c.handler.calls.lock().unwrap().len() == 1
        })
        .await;
        // Q is a second independently paired member path, available before C
        // refuses A. Freeze the signed proof while A really is still allowed.
        pair_services(&q.service, &a.service).await;
        pair_services(&q.service, &c.service).await;
        eventually("A/C 均已取得 Q 的独立成员根", || {
            a.store.introducers_for(&c.id).unwrap().contains(&q.id)
                && c.store
                    .export_trust_bundle(None)
                    .unwrap()
                    .memberships
                    .iter()
                    .any(|statement| {
                        statement.issuer == q.id && statement.subject == a.id && !statement.revoked
                    })
        })
        .await;
        let old = a
            .store
            .export_trust_bundle(Some(&a.service.descriptor().unwrap()))
            .unwrap();
        controller.set_activity(false, true).unwrap();
        c.service.revoke_control(&p.id).unwrap();
        assert!(!c.store.is_member(&p.id).unwrap());
        assert!(
            c.store.is_member(&a.id).unwrap(),
            "排除 P 不能切断经独立正常根 Q 加入的 A"
        );
        let independent = c.store.membership_proof(&a.id).unwrap();
        assert!(
            independent
                .statements
                .iter()
                .all(|statement| statement.issuer != p.id && statement.subject != p.id),
            "验成员链不得经过被本机排除的 P"
        );
        assert_local_root(&c.store, &c.id, &a.id);
        let before = q.forwards();
        let hello = hostile_replay(&a, &q, &c, &old, Request::Hello).await;
        assert!(matches!(hello, Reply::Hello(descriptor) if descriptor.endpoint_id == c.id));
        assert!(q.forwards() > before, "替代路径必须在撤销前实际可用");

        // C revokes its own trust in the original A, excluding that identity as
        // a chain endpoint or intermediate. Other devices keep their independent
        // membership roots; neither an alternate path nor a replay clears C's
        // exclusion. This is not a promise of removing A from the whole network.
        c.service.revoke_control(&a.id).unwrap();
        assert!(!c.store.allows_control(&c.id, &a.id).unwrap());
        assert!(!c.store.is_member(&a.id).unwrap());
        assert!(c.store.allows_control(&c.id, &q.id).unwrap());
        assert!(
            a.store.allows_control(&a.id, &c.id).unwrap(),
            "C 的拒绝不是反向控制撤销"
        );
        c.store.merge_trust_bundle(&q.id, &old).unwrap();
        assert!(!c.store.is_member(&a.id).unwrap());
        block_services(&a.service, &p.service, true);
        let before = q.forwards();
        for request in [
            Request::Query {
                id: original_id.clone(),
                trace_context: None,
            },
            Request::Submit {
                id: refused_id,
                view: view(),
                execution: execution(),
                command: Box::new(send("must-not-execute-after-target-revocation")),
                trace_context: None,
            },
        ] {
            let refused = hostile_replay(&a, &q, &c, &old, request).await;
            assert!(
                matches!(refused, Reply::Error(RemoteError::Unauthorized)),
                "原首跳被阻断后，C 仍须亲自拒绝替代路径携带的 A 和旧证明"
            );
        }
        assert!(q.forwards() > before);
        c.handler
            .assert_operator(&a.id, &[CommandKind::ReadConversation]);
        assert_eq!(c.handler.sends.load(Ordering::SeqCst), 0);

        let stable_id = c.id.clone();
        let old_incarnation = c.service.descriptor().unwrap().incarnation;
        c.service.close().await.unwrap();
        // A fresh service, identity load, and DeviceStore::open read only the
        // synthetic private directory. No old in-memory store is transplanted.
        c = persistent_full_device(home.path(), "synthetic-full-C", 300).await;
        assert_eq!(c.id, stable_id);
        assert_ne!(c.service.descriptor().unwrap().incarnation, old_incarnation);
        block_services(&a.service, &c.service, true);
        assert!(!c.store.is_member(&a.id).unwrap());
        assert!(!c.store.allows_control(&c.id, &a.id).unwrap());
        assert!(c.store.allows_control(&c.id, &q.id).unwrap());
        let current_addresses = c.service.descriptor().unwrap().direct_addresses;
        eventually("重启后的 C 经成员交换更新 Q 的位置", || {
            q.store.contact(&c.id).unwrap().locator.direct_addresses == current_addresses
        })
        .await;
        let still_allowed = connect_member(&q.service, &c, &[]).await;
        let reply = still_allowed
            .request(view(), execution(), read())
            .await
            .unwrap();
        assert_eq!(reply.revision, Revision(c.handler.revision));
        assert!(matches!(reply.response, Response::ReadConversation(_)));
        let before = q.forwards();
        for request in [
            Request::Query {
                id: original_id,
                trace_context: None,
            },
            Request::Submit {
                id: RequestId {
                    incarnation: c.service.descriptor().unwrap().incarnation,
                    client: ClientSession::new().unwrap().id,
                    sequence: 1,
                },
                view: view(),
                execution: execution(),
                command: Box::new(send("must-not-execute-after-target-restart")),
                trace_context: None,
            },
        ] {
            let refused = hostile_replay(&a, &q, &c, &old, request).await;
            assert!(
                matches!(refused, Reply::Error(RemoteError::Unauthorized)),
                "C 重启后不能用旧证明恢复 A 的查询或副作用资格"
            );
        }
        assert!(q.forwards() > before);
        c.handler
            .assert_operator(&q.id, &[CommandKind::ReadConversation]);
        assert_eq!(c.handler.sends.load(Ordering::SeqCst), 0);
        assert!(!c.store.allows_control(&c.id, &a.id).unwrap());
        assert!(!p.service.read_state().unwrap().control_ingress);
        assert!(!q.service.read_state().unwrap().control_ingress);
        a.handler.assert_idle();
        for service in [&p.service, &q.service, &a.service, &c.service] {
            service.close().await.unwrap();
        }
    });
}

/// Hostile wire fixture only: the normal acceptance paths above never organize
/// routes or import proofs themselves. A really dials Q, Q forwards the bytes
/// to C, and TLS authenticates A. The A<->C source block remains installed.
async fn hostile_replay(
    operator: &Device,
    carrier: &Phone,
    target: &Device,
    stale: &TrustBundle,
    request: Request,
) -> Reply {
    tokio::time::timeout(WAIT, async {
        let descriptor = carrier.service.descriptor().unwrap();
        let locator = DeviceLocator {
            relay_urls: descriptor.relay_urls,
            direct_addresses: descriptor.direct_addresses,
            lookup_urls: descriptor.lookup_urls,
        };
        let network = operator.service.network().unwrap();
        let endpoint = network.prepare_peer(&carrier.id, &locator).unwrap();
        let native = endpoint
            .connect(locator.address(&carrier.id).unwrap(), crate::mesh::ALPN)
            .await
            .unwrap();
        assert_eq!(native.remote_id().to_string(), carrier.id);
        let (send, recv) = native.open_bi().await.unwrap();
        let mut io = tokio::io::join(recv, send);
        // Private mesh framing from mesh.rs; this fixture needs an adversarial
        // claimed proof, which the ordinary Controller correctly cannot select.
        write_test_frame(
            &mut io,
            &serde_json::json!({
                "operation": "tunnel", "target": target.id, "visited": [operator.id],
            }),
        )
        .await;
        let ready: serde_json::Value = read_test_frame(&mut io).await;
        assert_eq!(
            ready.get("reply").and_then(|value| value.as_str()),
            Some("ready")
        );
        assert_eq!(
            ready["value"]["path"],
            serde_json::json!([operator.id, carrier.id, target.id]),
            "回放必须实际到达 C 的目标入口"
        );
        let mut tls =
            crate::tunnel_tls::connect(&operator.service.state.identity.key, &target.id, io)
                .await
                .unwrap();
        #[derive(Serialize)]
        struct StaleRequest<'a> {
            trust: &'a TrustBundle,
            request: Request,
        }
        write_test_frame(
            &mut tls,
            &Envelope {
                version: PROTOCOL_VERSION,
                body: StaleRequest {
                    trust: stale,
                    request,
                },
            },
        )
        .await;
        let response: Envelope<Reply> = read_test_frame(&mut tls).await;
        assert_eq!(response.version, PROTOCOL_VERSION);
        let _ = tls.shutdown().await;
        native.close(0u8.into(), b"synthetic stale proof fixture finished");
        response.body
    })
    .await
    .expect("对 C 的旧证明回放必须得到明确拒绝，不能只等连接超时")
}

async fn write_test_frame<W: AsyncWrite + Unpin, T: Serialize>(io: &mut W, value: &T) {
    let bytes = serde_json::to_vec(value).unwrap();
    assert!(bytes.len() <= MAX_FRAME_BYTES);
    io.write_u32(u32::try_from(bytes.len()).unwrap())
        .await
        .unwrap();
    io.write_all(&bytes).await.unwrap();
    io.flush().await.unwrap();
}

async fn read_test_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(io: &mut R) -> T {
    let length = io.read_u32().await.unwrap() as usize;
    assert!((1..=MAX_FRAME_BYTES).contains(&length));
    let mut bytes = vec![0; length];
    io.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
