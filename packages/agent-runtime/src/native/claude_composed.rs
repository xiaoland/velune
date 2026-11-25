//! Acceptance of an ordinary composed input by the Claude stream-json child.
use super::*;
use crate::transport::{WriteOutcome, WriteReceipt};
use std::time::Instant;

#[derive(Debug, PartialEq, Eq)]
pub enum ClaudeComposedAcceptance {
    Accepted { message_id: String },
    Rejected { message_id: String, detail: String },
    Indeterminate { message_id: String, detail: String },
}

pub(super) struct PendingPrompt {
    message: Message,
    write: Option<WriteReceipt>,
    deadline: Instant,
    reported_indeterminate: bool,
}

impl PendingPrompt {
    pub(super) fn new(message: Message, write: WriteReceipt, deadline: Instant) -> Self {
        Self {
            message,
            write: Some(write),
            deadline,
            reported_indeterminate: false,
        }
    }

    fn write_outcome(&self) -> Option<WriteOutcome> {
        self.write.as_ref().and_then(WriteReceipt::try_outcome)
    }
}

impl NativeSession {
    /// Ordinary Claude stream-json sessions identify the starting input,
    /// verified from the supported version floor (2.1.286).
    pub fn supports_claude_composed_acceptance(&self) -> bool {
        self.config.kind == NativeKind::ClaudeCode && !self.side && !self.claude.commands.enabled
    }

    /// Starts the ordinary turn and returns its native user UUID after local
    /// writer admission. Errors at this boundary prove the input was not sent.
    /// The caller advances the session with poll and consumes the one receipt.
    pub fn send_claude_composed(&mut self, input: &crate::TurnInput) -> Result<String> {
        if !self.supports_claude_composed_acceptance() {
            return Err(Error::new("Claude Code 不支持此发送确认路径"));
        }
        if self.claude.composed_prompt.is_some() || self.claude.composed_acceptance.is_some() {
            return Err(Error::with_code(
                "Claude Code 上一次发送确认尚未处理",
                "operation_pending",
            ));
        }
        self.start_turn(input, false, true)?;
        Ok(self
            .claude
            .composed_prompt
            .as_ref()
            .expect("composed turn stages its user message")
            .message
            .id
            .clone())
    }

    pub fn take_claude_composed_acceptance(&mut self) -> Option<ClaudeComposedAcceptance> {
        self.claude.composed_acceptance.take()
    }

    pub(super) fn claude_observe_composed_acceptance(&mut self, record: &Value) {
        if record["type"] != "command_lifecycle"
            // A queued command cannot yet be targeted by the native interrupt.
            || record["state"] != "started"
            || record["session_id"].as_str() != self.native_id.as_deref()
        {
            return;
        }
        let Some(prompt) = self
            .claude
            .composed_prompt
            .take_if(|prompt| record["command_uuid"].as_str() == Some(prompt.message.id.as_str()))
        else {
            return;
        };
        if !prompt.reported_indeterminate {
            self.claude.composed_acceptance = Some(ClaudeComposedAcceptance::Accepted {
                message_id: prompt.message.id.clone(),
            });
        }
        // Use the staged row so native history and the live transcript share
        // an identity, while the live row keeps the input's attachment paths.
        self.message(prompt.message);
    }

    pub(super) fn claude_poll_composed_send(&mut self) {
        let Some(prompt) = &mut self.claude.composed_prompt else {
            return;
        };
        match prompt.write_outcome() {
            Some(WriteOutcome::NotWritten(error)) => {
                let prompt = self
                    .claude
                    .composed_prompt
                    .take()
                    .expect("pending Claude input");
                if !prompt.reported_indeterminate {
                    self.claude.composed_acceptance = Some(ClaudeComposedAcceptance::Rejected {
                        message_id: prompt.message.id,
                        detail: error.to_string(),
                    });
                }
                self.claude.foreground = None;
                self.settle(RunState::Idle);
                return;
            }
            Some(WriteOutcome::Written(_)) => prompt.write = None,
            Some(WriteOutcome::Indeterminate(error)) => {
                prompt.write = None;
                self.claude_report_indeterminate_composed_send(error.to_string());
                return;
            }
            Some(WriteOutcome::ObservationTimeout { .. }) | None => {}
        }
        if Instant::now() >= prompt.deadline {
            self.claude_report_indeterminate_composed_send("等待 Claude Code 接收确认超时".into());
        }
    }

    fn claude_report_indeterminate_composed_send(&mut self, detail: String) {
        if let Some(prompt) = &mut self.claude.composed_prompt
            && !prompt.reported_indeterminate
        {
            prompt.reported_indeterminate = true;
            self.claude.composed_acceptance = Some(ClaudeComposedAcceptance::Indeterminate {
                message_id: prompt.message.id.clone(),
                detail,
            });
        }
    }

    pub(super) fn claude_finish_unconfirmed_composed_send(&mut self) {
        let Some(prompt) = self.claude.composed_prompt.take() else {
            return;
        };
        if prompt.reported_indeterminate {
            return;
        }
        self.claude.composed_acceptance = Some(match prompt.write_outcome() {
            Some(WriteOutcome::NotWritten(error)) => ClaudeComposedAcceptance::Rejected {
                message_id: prompt.message.id,
                detail: error.to_string(),
            },
            _ => ClaudeComposedAcceptance::Indeterminate {
                message_id: prompt.message.id,
                detail: "Claude Code 已结束当前输入，未收到接收确认".into(),
            },
        });
    }
}
