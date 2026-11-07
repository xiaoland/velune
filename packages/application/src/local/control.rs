//! One observation lifetime per physical context, shared by every view and caller.
use super::*;
use crate::conversation::{ConversationSnapshot, InputAccess};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use velune_agent_runtime::control::{Observation, Target};

pub(super) struct ControlWatch {
    target: Target,
    observation: Option<Observation>,
    pending: Option<mpsc::Receiver<Observation>>,
    next: Instant,
}
impl ControlWatch {
    fn new(target: Target) -> Self {
        Self {
            target,
            observation: None,
            pending: None,
            next: Instant::now(),
        }
    }
    fn poll(&mut self) -> InputAccess {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(observation) => {
                    self.observation = Some(observation);
                    self.pending = None;
                    self.next = Instant::now() + Duration::from_secs(2);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.observation = Some(Observation::Unknown);
                    self.pending = None;
                    self.next = Instant::now() + Duration::from_secs(2);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pending.is_none() && Instant::now() >= self.next {
            let target = self.target.clone();
            let (sender, receiver) = mpsc::channel();
            self.pending = Some(receiver);
            std::thread::spawn(move || {
                let _ = sender.send(target.observe());
            });
        }
        match self.observation {
            None => InputAccess::CheckingControl,
            Some(Observation::HeldElsewhere) => InputAccess::ExternalController,
            Some(Observation::Unknown) => InputAccess::ControlUnavailable,
            Some(Observation::Available | Observation::NotReported) => InputAccess::Editable,
        }
    }
}
impl CoreRuntime {
    fn control_target(&self) -> Result<Option<Target>, RuntimeError> {
        if self.session.side.is_some() || self.session.unsubmitted_draft.is_some() {
            return Ok(None);
        }
        let Some(view) = self.session.projected() else {
            return Ok(None);
        };
        self.read_control_target(view, self.session.logical_projection.as_ref())
    }
    pub(super) fn read_control_target(
        &self,
        view: &ConversationSnapshot,
        projection: Option<&continuation::LogicalProjection>,
    ) -> Result<Option<Target>, RuntimeError> {
        let (runtime_id, id) = projection
            .and_then(|p| p.link.segments.last())
            .map(|s| {
                (
                    s.runtime_instance_id.as_str(),
                    s.native_conversation_id.as_str(),
                )
            })
            .unwrap_or((&view.context_runtime_id, &view.conversation.id));
        let runtime = self
            .session
            .execution_configuration
            .as_ref()
            .filter(|configuration| configuration.runtime.id == runtime_id)
            .map(|configuration| &configuration.runtime)
            .map_or_else(|| self.runtime_instance(runtime_id), Ok)?;
        if !velune_agent_runtime::control::reports_control(runtime.type_id) {
            return Ok(None);
        }
        let context = self.execution_context(runtime)?;
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .unwrap_or(id)
            .to_owned();
        Ok(Some(Target {
            execution: context.target,
            os_home: context.os_home,
            kind: runtime.type_id,
            agent_dir: context.agent_dir,
            native_id,
            node_binary: setting_path_optional(runtime, "nodeBinary").unwrap_or_default(),
            resources: context.resources,
        }))
    }
    fn observed_input_access(&mut self) -> InputAccess {
        self.observe_control_target(self.control_target())
    }
    fn observe_control_target(
        &mut self,
        target: Result<Option<Target>, RuntimeError>,
    ) -> InputAccess {
        let target = match target {
            Ok(None) => {
                self.session.control = None;
                return InputAccess::Editable;
            }
            Err(_) => {
                self.session.control = None;
                return InputAccess::ControlUnavailable;
            }
            Ok(Some(target)) => target,
        };
        // Views of this session share its native handle. A different runtime
        // configuration parked here is another native client: do not borrow
        // its model, account, MCP services or approval context implicitly.
        if matches!(&self.session.active_state, ActiveState::Native(native) if native.owns_control(&target))
        {
            self.session.control = None;
            return InputAccess::Editable;
        }
        if self
            .session
            .control
            .as_ref()
            .is_none_or(|watch| watch.target != target)
        {
            self.session.control = Some(ControlWatch::new(target));
        }
        self.session.control.as_mut().expect("control watch").poll()
    }
    pub(super) fn apply_input_access(&mut self, snapshot: &mut ConversationSnapshot) {
        let access = self.observed_input_access();
        Self::decorate_input_access(
            snapshot,
            access,
            matches!(self.session.active_state, ActiveState::History(_)),
        );
    }
    pub(super) fn apply_read_input_access(
        &mut self,
        snapshot: &mut ConversationSnapshot,
        target: Result<Option<Target>, RuntimeError>,
    ) {
        let access = self.observe_control_target(target);
        Self::decorate_input_access(snapshot, access, true);
    }
    fn decorate_input_access(
        snapshot: &mut ConversationSnapshot,
        mut access: InputAccess,
        history: bool,
    ) {
        if snapshot.conversation.parent_conversation_id.is_some() {
            access = InputAccess::ReadOnly;
        } else if snapshot.run_state == RunState::Elsewhere && access == InputAccess::Editable {
            access = InputAccess::ExternalExecution;
        } else if history && !snapshot.actions.can_send && access == InputAccess::Editable {
            access = InputAccess::ReadOnly;
        }
        snapshot.actions.input_access = access;
    }
    pub(super) fn control_held_elsewhere(&mut self) {
        if let Ok(Some(target)) = self.control_target() {
            let mut watch = ControlWatch::new(target);
            watch.observation = Some(Observation::HeldElsewhere);
            watch.next = Instant::now() + Duration::from_secs(2);
            self.session.control = Some(watch);
        }
    }
    /// All writers use the same admission, including queued and scheduled turns.
    pub(super) fn require_input_access(&mut self) -> Result<(), RuntimeError> {
        let Some(snapshot) = self.current_snapshot() else {
            return Ok(());
        };
        Self::check_input_access(snapshot.actions.input_access)
    }
    /// Validate the observation already captured for a command's projection.
    pub(super) fn check_input_access(access: InputAccess) -> Result<(), RuntimeError> {
        let message = match access {
            InputAccess::Editable => return Ok(()),
            InputAccess::CheckingControl => "正在检查会话控制权，请稍候",
            InputAccess::ExternalController => {
                "另一个客户端正在控制此会话；草稿已保留，释放后可继续"
            }
            InputAccess::ControlUnavailable => "暂时无法确认会话控制权，请检查运行时连接",
            InputAccess::ExternalExecution => "此会话正在其他客户端执行，请等待执行结束",
            InputAccess::ReadOnly => "此会话当前只可阅读",
        };
        Err(RuntimeError::invalid(message))
    }
}
