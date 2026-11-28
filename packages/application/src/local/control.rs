//! Native facts are shared across every logical view of a physical context.
use super::*;
use crate::conversation::{ConversationSnapshot, InputAccess};
use velune_agent_runtime::control::Target;

mod observations;
pub(super) use observations::ControlObservations;

impl CoreRuntime {
    pub(super) fn control_target(&self) -> Result<Option<Target>, RuntimeError> {
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
            .execution()
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
        if self.current_handoff_is_released() {
            return InputAccess::Editable;
        }
        self.observe_control_target(self.control_target())
    }
    pub(super) fn observe_control_target(
        &mut self,
        target: Result<Option<Target>, RuntimeError>,
    ) -> InputAccess {
        let target = match target {
            Ok(None) => return InputAccess::Editable,
            Err(_) => return InputAccess::ControlUnavailable,
            Ok(Some(target)) => target,
        };
        // Views of this session share its native handle. A different runtime
        // configuration parked here is another native client: do not borrow
        // its model, account, MCP services or approval context implicitly.
        if match &self.session.active_state {
            ActiveState::Native(native) => native.owns_control(&target),
            ActiveState::Pending(pending) => pending.owns_control(&target),
            _ => false,
        } {
            self.control_observations.suspend(&target);
            return InputAccess::Editable;
        }
        self.control_observations.access(target)
    }

    pub(super) fn apply_input_access(&mut self, snapshot: &mut ConversationSnapshot) {
        let access = self.observed_input_access();
        Self::decorate_input_access(
            snapshot,
            access,
            matches!(self.session.active_state, ActiveState::History(_)),
        );
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
            self.control_observations.held_elsewhere(target);
        }
    }
    /// All writers use the same admission, including queued and scheduled turns.
    pub(super) fn require_input_access(&mut self) -> Result<(), RuntimeError> {
        let Some(snapshot) = self.submission_context() else {
            return Ok(());
        };
        self.check_handoff_input(&snapshot)?;
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
