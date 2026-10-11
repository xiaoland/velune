//! Scoped delegation: the broker holds handles; each worker owns one child runtime.
use super::*;
use crate::{ChildSnapshot, ChildState, ChildStatus, CollaborationLimits, CollaborationTarget};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use velune_conversation::{
    ConversationSnapshot, MessageBlock, MessageRole, RuntimeInteractionReply,
};
mod child;
mod mcp;
use child::{CapturedTarget, ChildRuntime};

struct ChildView {
    status: ChildStatus,
    snapshot: Option<ConversationSnapshot>,
}
struct Child {
    view: Mutex<ChildView>,
    changed: Condvar,
    sender: mpsc::Sender<Command>,
    worker: Mutex<Option<JoinHandle<()>>>,
    stop_requested: AtomicBool,
}
enum Command {
    Send(String),
    Cancel,
    Close,
    Reply(
        u64,
        String,
        RuntimeInteractionReply,
        mpsc::Sender<Result<(), RuntimeError>>,
    ),
}
struct Registry {
    closed: bool,
    next: u64,
    children: BTreeMap<String, Arc<Child>>,
}
pub(super) struct Coordinator {
    targets: BTreeMap<String, Arc<CapturedTarget>>,
    limits: CollaborationLimits,
    epoch: u128,
    registry: Mutex<Registry>,
}

fn unavailable() -> RuntimeError {
    RuntimeError::invalid("协作状态不可用")
}
fn running(state: &ChildState) -> bool {
    matches!(
        state,
        ChildState::Starting
            | ChildState::Running
            | ChildState::WaitingForApproval
            | ChildState::Cancelling
    )
}

impl Coordinator {
    fn new(targets: Vec<CapturedTarget>, limits: CollaborationLimits) -> Arc<Self> {
        Arc::new(Self {
            targets: targets
                .into_iter()
                .map(|t| (t.target.id.clone(), Arc::new(t)))
                .collect(),
            limits,
            epoch: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos(),
            registry: Mutex::new(Registry {
                closed: false,
                next: 0,
                children: BTreeMap::new(),
            }),
        })
    }
    pub(super) fn agents(&self) -> Vec<CollaborationTarget> {
        self.targets
            .values()
            .map(|target| target.target.clone())
            .collect()
    }
    pub(super) fn spawn(
        self: &Arc<Self>,
        target_id: &str,
        prompt: &str,
    ) -> Result<ChildStatus, RuntimeError> {
        validate_prompt(prompt)?;
        let target = self
            .targets
            .get(target_id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("委派目标未获授权"))?;
        let mut registry = self.registry.lock().map_err(|_| unavailable())?;
        if registry.closed {
            return Err(RuntimeError::invalid("协作范围已关闭"));
        }
        let live = registry
            .children
            .values()
            .filter(|child| {
                child
                    .view
                    .lock()
                    .is_ok_and(|v| v.status.state != ChildState::Closed)
            })
            .count();
        if live >= self.limits.max_children as usize {
            return Err(RuntimeError::invalid("子任务数量已达到授权上限"));
        }
        registry.next += 1;
        let id = format!("child-{:x}-{}", self.epoch, registry.next);
        let status = ChildStatus {
            id: id.clone(),
            target_id: target_id.into(),
            runtime_id: target.target.runtime_id.clone(),
            conversation_id: None,
            revision: 1,
            state: ChildState::Starting,
            turn: 1,
            result: None,
            error: None,
            pending_interaction_count: 0,
        };
        let (sender, receiver) = mpsc::channel();
        let child = Arc::new(Child {
            view: Mutex::new(ChildView {
                status: status.clone(),
                snapshot: None,
            }),
            changed: Condvar::new(),
            sender,
            worker: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
        });
        let worker_child = child.clone();
        let timeout = Duration::from_millis(self.limits.turn_timeout_ms);
        let text = prompt.to_owned();
        let worker = thread::Builder::new()
            .name("velune-child-agent".into())
            .spawn(move || run_child(worker_child, receiver, target, id, text, timeout))?;
        *child.worker.lock().map_err(|_| unavailable())? = Some(worker);
        registry.children.insert(status.id.clone(), child);
        Ok(status)
    }
    fn child(&self, id: &str) -> Result<Arc<Child>, RuntimeError> {
        let registry = self.registry.lock().map_err(|_| unavailable())?;
        if registry.closed {
            return Err(RuntimeError::invalid("协作范围已关闭"));
        }
        registry
            .children
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("子任务不属于当前协作范围"))
    }
    pub(super) fn status(&self, id: &str) -> Result<ChildStatus, RuntimeError> {
        Ok(self
            .child(id)?
            .view
            .lock()
            .map_err(|_| unavailable())?
            .status
            .clone())
    }
    fn all(&self) -> Result<Vec<ChildStatus>, RuntimeError> {
        self.registry
            .lock()
            .map_err(|_| unavailable())?
            .children
            .values()
            .map(|child| Ok(child.view.lock().map_err(|_| unavailable())?.status.clone()))
            .collect()
    }
    pub(super) fn send(&self, id: &str, prompt: &str) -> Result<ChildStatus, RuntimeError> {
        validate_prompt(prompt)?;
        let child = self.child(id)?;
        let mut view = child.view.lock().map_err(|_| unavailable())?;
        if !matches!(view.status.state, ChildState::Idle | ChildState::Cancelled) {
            return Err(RuntimeError::invalid("子任务尚未空闲或已不可发送；未排队"));
        }
        view.status.state = ChildState::Starting;
        view.status.turn += 1;
        view.status.revision += 1;
        view.status.result = None;
        view.status.error = None;
        child.stop_requested.store(false, Ordering::Release);
        child
            .sender
            .send(Command::Send(prompt.into()))
            .map_err(|_| unavailable())?;
        child.changed.notify_all();
        Ok(view.status.clone())
    }
    pub(super) fn wait(
        &self,
        id: &str,
        after_revision: u64,
        timeout_ms: u64,
    ) -> Result<ChildStatus, RuntimeError> {
        if timeout_ms > 30_000 {
            return Err(RuntimeError::invalid("单次协作等待不能超过 30 秒"));
        }
        let child = self.child(id)?;
        let view = child.view.lock().map_err(|_| unavailable())?;
        let (view, _) = child
            .changed
            .wait_timeout_while(view, Duration::from_millis(timeout_ms), |v| {
                v.status.revision <= after_revision
                    && matches!(
                        v.status.state,
                        ChildState::Starting | ChildState::Running | ChildState::Cancelling
                    )
            })
            .map_err(|_| unavailable())?;
        Ok(view.status.clone())
    }
    pub(super) fn cancel(&self, id: &str) -> Result<ChildStatus, RuntimeError> {
        let child = self.child(id)?;
        let mut view = child.view.lock().map_err(|_| unavailable())?;
        if running(&view.status.state) {
            child.stop_requested.store(true, Ordering::Release);
            view.status.state = ChildState::Cancelling;
            view.status.revision += 1;
            child
                .sender
                .send(Command::Cancel)
                .map_err(|_| unavailable())?;
            child.changed.notify_all();
        }
        Ok(view.status.clone())
    }
    pub(super) fn close(&self, id: &str) -> Result<ChildStatus, RuntimeError> {
        let child = self.child(id)?;
        child.stop_requested.store(true, Ordering::Release);
        child.sender.send(Command::Close).ok();
        if let Some(worker) = child.worker.lock().map_err(|_| unavailable())?.take() {
            worker.join().map_err(|_| unavailable())?;
        }
        let status = child.view.lock().map_err(|_| unavailable())?.status.clone();
        Ok(status)
    }
    fn snapshot(
        &self,
        id: &str,
    ) -> Result<(ChildStatus, Option<ConversationSnapshot>), RuntimeError> {
        let child = self.child(id)?;
        let view = child.view.lock().map_err(|_| unavailable())?;
        Ok((view.status.clone(), view.snapshot.clone()))
    }
    fn reply(
        &self,
        id: &str,
        turn: u64,
        interaction_id: &str,
        reply: RuntimeInteractionReply,
    ) -> Result<ChildStatus, RuntimeError> {
        let child = self.child(id)?;
        {
            let view = child.view.lock().map_err(|_| unavailable())?;
            if view.status.state != ChildState::WaitingForApproval
                || child.stop_requested.load(Ordering::Acquire)
                || view.status.turn != turn
                || !view.snapshot.as_ref().is_some_and(|s| {
                    s.pending_interactions
                        .iter()
                        .any(|i| i.id == interaction_id)
                })
            {
                return Err(RuntimeError::invalid("子任务审批已失效或不属于当前轮次"));
            }
        }
        let (sender, receiver) = mpsc::channel();
        child
            .sender
            .send(Command::Reply(turn, interaction_id.into(), reply, sender))
            .map_err(|_| unavailable())?;
        receiver
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| unavailable())??;
        self.status(id)
    }
    fn busy(&self) -> bool {
        self.all()
            .map_or(true, |states| states.iter().any(|s| running(&s.state)))
    }
    fn cancel_all(&self) {
        if let Ok(states) = self.all() {
            for state in states {
                let _ = self.cancel(&state.id);
            }
        }
    }
    fn shutdown(&self) {
        let children = if let Ok(mut registry) = self.registry.lock() {
            registry.closed = true;
            registry.children.values().cloned().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for child in &children {
            child.stop_requested.store(true, Ordering::Release);
            let _ = child.sender.send(Command::Close);
        }
        for child in children {
            if let Ok(mut worker) = child.worker.lock()
                && let Some(worker) = worker.take()
            {
                let _ = worker.join();
            }
        }
    }
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn validate_prompt(prompt: &str) -> Result<(), RuntimeError> {
    if prompt.trim().is_empty() || prompt.len() > 128 * 1024 {
        return Err(RuntimeError::invalid("子任务消息必须非空且不超过 128 KiB"));
    }
    Ok(())
}

fn publish(
    child: &Child,
    runtime: &ChildRuntime,
    state: ChildState,
    result: Option<String>,
    error: Option<String>,
) {
    if let Ok(mut view) = child.view.lock() {
        let mut snapshot = runtime.snapshot();
        if !running(&state)
            && let Some(snapshot) = &mut snapshot
        {
            snapshot.pending_interactions.clear();
            snapshot.actions.can_cancel = false;
            if matches!(
                state,
                ChildState::Failed | ChildState::TimedOut | ChildState::Closed
            ) {
                snapshot.actions.can_send = false;
            }
            if matches!(state, ChildState::Failed | ChildState::TimedOut) {
                snapshot.run_state = RunState::Failed;
            }
        }
        let mut next = view.status.clone();
        next.state = state;
        next.result = result;
        next.error = error;
        next.conversation_id = snapshot.as_ref().map(|s| s.conversation.id.clone());
        next.pending_interaction_count = snapshot
            .as_ref()
            .map_or(0, |s| s.pending_interactions.len() as u32);
        if next != view.status || snapshot != view.snapshot {
            next.revision += 1;
            view.status = next;
            view.snapshot = snapshot;
            child.changed.notify_all();
        }
    }
}
fn final_text(runtime: &ChildRuntime) -> Option<String> {
    let snapshot = runtime.snapshot()?;
    snapshot
        .messages
        .iter()
        .rev()
        .take_while(|m| m.role != MessageRole::User)
        .find(|m| m.role == MessageRole::Assistant)
        .map(|m| {
            m.blocks
                .iter()
                .filter_map(|b| {
                    if let MessageBlock::Text { text } = b {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
}
fn run_child(
    child: Arc<Child>,
    receiver: mpsc::Receiver<Command>,
    target: Arc<CapturedTarget>,
    id: String,
    prompt: String,
    timeout: Duration,
) {
    let mut runtime = ChildRuntime::new(target, &id);
    let mut active = true;
    let mut deadline = Instant::now() + timeout;
    let mut cancelled = false;
    let mut cancel_deadline = None;
    let mut failure = match runtime.send(&prompt, || {
        !child.stop_requested.load(Ordering::Acquire) && Instant::now() < deadline
    }) {
        Ok(sent) => {
            cancelled = !sent && child.stop_requested.load(Ordering::Acquire);
            None
        }
        Err(error) => Some(error.to_string()),
    };
    loop {
        if active && failure.is_none() {
            failure = runtime.poll().err().map(|e| e.to_string());
        }
        if let Some(error) = failure.take() {
            let _ = runtime.shutdown();
            publish(&child, &runtime, ChildState::Failed, None, Some(error));
            active = false;
        }
        if active {
            if Instant::now() >= deadline {
                let _ = runtime.cancel();
                let _ = runtime.shutdown();
                publish(
                    &child,
                    &runtime,
                    ChildState::TimedOut,
                    None,
                    Some("子任务轮次已超过授权期限；未重发".into()),
                );
                active = false;
            } else if cancel_deadline.is_some_and(|end| Instant::now() >= end) {
                let _ = runtime.shutdown();
                publish(
                    &child,
                    &runtime,
                    ChildState::Failed,
                    None,
                    Some("子任务取消未确认，已关闭执行进程；结果未知".into()),
                );
                active = false;
            } else if !runtime.busy() {
                match runtime.settle() {
                    Ok(()) => {
                        let failed = runtime.snapshot().is_some_and(|s| {
                            s.run_state == RunState::Failed || !s.actions.can_send
                        });
                        let state = if failed {
                            ChildState::Failed
                        } else if cancelled {
                            ChildState::Cancelled
                        } else {
                            ChildState::Idle
                        };
                        publish(
                            &child,
                            &runtime,
                            state,
                            if failed || cancelled {
                                None
                            } else {
                                final_text(&runtime)
                            },
                            if failed {
                                Some("子任务执行失败；未自动重发".into())
                            } else {
                                None
                            },
                        );
                    }
                    Err(error) => {
                        let _ = runtime.shutdown();
                        publish(
                            &child,
                            &runtime,
                            ChildState::Failed,
                            None,
                            Some(error.to_string()),
                        );
                    }
                }
                active = false;
            } else {
                let state = if cancelled {
                    ChildState::Cancelling
                } else if runtime
                    .snapshot()
                    .is_some_and(|s| !s.pending_interactions.is_empty())
                {
                    ChildState::WaitingForApproval
                } else {
                    ChildState::Running
                };
                publish(&child, &runtime, state, None, None);
            }
        }
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(Command::Send(prompt)) => {
                deadline = Instant::now() + timeout;
                cancelled = false;
                cancel_deadline = None;
                active = true;
                failure = match runtime.send(&prompt, || {
                    !child.stop_requested.load(Ordering::Acquire) && Instant::now() < deadline
                }) {
                    Ok(sent) => {
                        cancelled = !sent && child.stop_requested.load(Ordering::Acquire);
                        None
                    }
                    Err(error) => Some(error.to_string()),
                };
            }
            Ok(Command::Cancel) => {
                if active {
                    cancelled = true;
                    cancel_deadline = Some(Instant::now() + Duration::from_secs(5));
                    failure = runtime.cancel().err().map(|e| e.to_string());
                }
            }
            Ok(Command::Reply(turn, id, reply, response)) => {
                // Admission and consumption are separate: cancellation or a deadline
                // can invalidate a host answer while it is waiting in the channel.
                let valid = active
                    && !cancelled
                    && !child.stop_requested.load(Ordering::Acquire)
                    && Instant::now() < deadline
                    && child.view.lock().is_ok_and(|view| {
                        view.status.turn == turn
                            && view.status.state == ChildState::WaitingForApproval
                    })
                    && runtime.snapshot().is_some_and(|snapshot| {
                        snapshot.pending_interactions.iter().any(|i| i.id == id)
                    });
                let result = if valid {
                    runtime.reply(&id, reply)
                } else {
                    Err(RuntimeError::invalid("子任务审批在执行前已失效"))
                };
                let _ = response.send(result);
            }
            Ok(Command::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if active {
                    let _ = runtime.cancel();
                }
                let error = runtime.shutdown().err().map(|e| e.to_string());
                publish(&child, &runtime, ChildState::Closed, None, error);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

pub(super) struct Scope {
    pub(super) root_id: Option<String>,
    coordinator: Arc<Coordinator>,
    server: mcp::Server,
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.coordinator.shutdown();
    }
}

impl CoreRuntime {
    pub(super) fn collaboration_servers(&self) -> Vec<velune_agent_runtime::mcp::Server> {
        self.collaboration
            .as_ref()
            .map(|scope| {
                vec![velune_agent_runtime::mcp::Server {
                    name: format!("velune_{:016x}", scope.coordinator.epoch as u64),
                    endpoint: scope.server.endpoint().into(),
                    token: scope.server.token().into(),
                }]
            })
            .unwrap_or_default()
    }
    pub(super) fn collaboration_busy(&self) -> bool {
        self.collaboration
            .as_ref()
            .is_some_and(|scope| scope.coordinator.busy())
    }
    pub(super) fn close_collaboration(&mut self) {
        self.collaboration = None;
    }
    pub(super) fn cancel_children(&self) {
        if let Some(scope) = &self.collaboration {
            scope.coordinator.cancel_all();
        }
    }
    pub(super) fn create_collaborative(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() || self.collaboration_busy() {
            return Err(RuntimeError::invalid("父运行时或子任务正在执行"));
        }
        let targets: Vec<CollaborationTarget> =
            serde_json::from_value(request["payload"]["targets"].clone())?;
        let limits: CollaborationLimits =
            serde_json::from_value(request["payload"]["limits"].clone())?;
        if targets.is_empty()
            || targets.len() > 64
            || limits.max_children == 0
            || limits.max_children > 32
            || limits.turn_timeout_ms == 0
            || limits.turn_timeout_ms > 86_400_000
        {
            return Err(RuntimeError::invalid("协作授权范围或限制无效"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut captured = Vec::new();
        for target in targets {
            if target.id.is_empty()
                || target.id.len() > 128
                || target.id.chars().any(char::is_control)
                || !ids.insert(target.id.clone())
            {
                return Err(RuntimeError::invalid("协作目标标识必须非空且唯一"));
            }
            captured.push(self.capture_child(target)?);
        }
        self.close_collaboration();
        let coordinator = Coordinator::new(captured, limits);
        let server = mcp::Server::start(coordinator.clone())?;
        self.collaboration = Some(Scope {
            root_id: None,
            coordinator,
            server,
        });
        match self.create_conversation(request) {
            Ok(result) => {
                let id = self.current_snapshot().map(|s| s.conversation.id);
                self.collaboration.as_mut().expect("scope").root_id = id;
                Ok(result)
            }
            Err(error) => {
                self.close_collaboration();
                Err(error)
            }
        }
    }
    pub(super) fn collaboration_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let action = request["action"].as_str().unwrap_or_default();
        if action == "closeCollaboration" {
            if self.busy() {
                return Err(RuntimeError::invalid("请先取消父运行时轮次"));
            }
            self.close_collaboration();
            return Ok(Value::Null);
        }
        let scope = self
            .collaboration
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("没有活动协作范围"))?;
        let child_id = request["payload"]["childID"].as_str().unwrap_or_default();
        match action {
            "collaborationChildren" => Ok(serde_json::to_value(scope.coordinator.all()?)?),
            "childSnapshot" => {
                let (status, snapshot) = scope.coordinator.snapshot(child_id)?;
                Ok(serde_json::to_value(ChildSnapshot {
                    status,
                    snapshot: snapshot.map(|snapshot| self.delivery.snapshot(snapshot)),
                })?)
            }
            "replyChildInteraction" => {
                let turn = request["payload"]["turn"]
                    .as_u64()
                    .ok_or_else(|| RuntimeError::invalid("子任务轮次无效"))?;
                let id = request["payload"]["interactionID"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("子任务审批标识无效"))?;
                let reply = serde_json::from_value(request["payload"]["reply"].clone())?;
                Ok(serde_json::to_value(
                    scope.coordinator.reply(child_id, turn, id, reply)?,
                )?)
            }
            _ => Err(RuntimeError::invalid("协作操作无效")),
        }
    }
}
