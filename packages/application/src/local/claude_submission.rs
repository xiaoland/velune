//! Exact CLI acceptance for a composed Claude input, advanced by the application owner.
use super::*;
use execution_workers::{ExecutionCommandKind, ExecutionSuccess, ExecutionTicket};
use velune_agent_runtime::native::ClaudeComposedAcceptance;

pub(super) struct ClaudeSubmission {
    application: ExecutionTicket,
    message_id: String,
}

impl CoreRuntime {
    /// None means this send uses the runtime's existing compatibility path.
    pub(super) fn try_start_claude_composed(
        &mut self,
        input: &pi::TurnInput,
    ) -> Option<Result<(), velune_agent_runtime::LiveError>> {
        if !matches!(&self.session.active_state, ActiveState::Native(native)
            if native.supports_claude_composed_acceptance())
        {
            return None;
        }
        let application = match self.execution_control_ticket(ExecutionCommandKind::Send) {
            Ok(Some(ticket)) => ticket,
            Ok(None) => return None,
            Err(error) => return Some(Err(error.into())),
        };
        let ActiveState::Native(native) = &mut self.session.active_state else {
            unreachable!("checked supported Claude session");
        };
        Some(
            native
                .send_claude_composed(input)
                .map(|message_id| {
                    self.session.claude_submission = Some(ClaudeSubmission {
                        application,
                        message_id,
                    });
                    self.mark_execution_started(application);
                })
                .map_err(Into::into),
        )
    }

    pub(super) fn advance_claude_submission(&mut self) {
        let Some(submission) = self.session.claude_submission.take() else {
            return;
        };
        let result = match &mut self.session.active_state {
            ActiveState::Native(native) => match native.take_claude_composed_acceptance() {
                None => {
                    self.session.claude_submission = Some(submission);
                    return;
                }
                Some(ClaudeComposedAcceptance::Accepted { message_id })
                    if message_id == submission.message_id =>
                {
                    Ok(ExecutionSuccess {
                        submitted_native_message_id: Some(message_id),
                    })
                }
                Some(ClaudeComposedAcceptance::Rejected { message_id, detail })
                    if message_id == submission.message_id =>
                {
                    Err(RuntimeError::Invalid(format!(
                        "Claude Code 明确拒绝输入：{detail}"
                    )))
                }
                Some(ClaudeComposedAcceptance::Indeterminate { message_id, detail })
                    if message_id == submission.message_id =>
                {
                    Err(RuntimeError::operation_indeterminate(detail))
                }
                Some(_) => Err(RuntimeError::operation_indeterminate(
                    "Claude Code 输入回执身份与原提交不一致",
                )),
            },
            _ => Err(RuntimeError::operation_indeterminate(
                "Claude Code 确认输入前，原运行时连接已改变",
            )),
        };
        if result.is_ok()
            && let Some(id) = self.session.conversation_id().map(str::to_owned)
        {
            self.restore_archived(&id);
        }
        self.execution_results
            .insert(submission.application, result);
    }
}
