//! Submission admission shared by command execution and its public projection.
//! Draft contents, input methods and requests in flight remain client state.
use super::*;
use crate::conversation::{ConversationSnapshot, InputAccess};

#[derive(Clone, Copy, Default)]
pub(super) struct SubmissionState {
    pub(super) turn_running: bool,
    pub(super) cancel_requested: bool,
    pub(super) compacting: bool,
    pub(super) side_chat: bool,
    pub(super) native_can_steer: bool,
}

pub(super) struct SubmissionAdmission {
    pub(super) send: Result<(), &'static str>,
    /// Accept an already dispatched queue request even if the turn ended.
    pub(super) accept_queue: Result<(), &'static str>,
    /// Offer queueing as the editor's current action only while a turn runs.
    pub(super) offer_queue: bool,
    pub(super) can_steer: bool,
}

impl SubmissionAdmission {
    /// `snapshot.actions.can_send` is the source/runtime capability here,
    /// before `apply` replaces it with application admission. Never feed an
    /// already delivered snapshot back into this calculation as a source.
    fn resolve(snapshot: &ConversationSnapshot, state: SubmissionState) -> Self {
        let common = if snapshot.actions.input_access != InputAccess::Editable {
            Err("当前会话不能提交输入")
        } else if state.compacting {
            Err("上下文压缩期间不能提交输入")
        } else if snapshot.conversation.parent_conversation_id.is_some() {
            Err("原生 sub-agent 会话由启动它的会话继续，不能直接发送")
        } else if matches!(snapshot.run_state, RunState::Stopping | RunState::Elsewhere)
            || (state.turn_running && state.cancel_requested)
        {
            Err("请等待当前会话的执行结束")
        } else {
            Ok(())
        };
        let send = common.and_then(|()| {
            if state.turn_running || snapshot.run_state == RunState::Running {
                Err("runtime is busy")
            } else if !snapshot.actions.can_send {
                Err("当前原生上下文不可用，请修复来源后重新打开；未自动重发")
            } else if !snapshot.pending_interactions.is_empty() {
                Err("请先处理当前运行时请求")
            } else {
                Ok(())
            }
        });
        let accept_queue = common.and({
            if state.side_chat {
                Err("侧聊不提供发送队列")
            } else {
                Ok(())
            }
        });
        let offer_queue =
            accept_queue.is_ok() && state.turn_running && snapshot.run_state == RunState::Running;
        Self {
            send,
            accept_queue,
            offer_queue,
            can_steer: offer_queue && state.native_can_steer,
        }
    }

    fn apply(&self, snapshot: &mut ConversationSnapshot) {
        snapshot.actions.can_send = self.send.is_ok();
        snapshot.follow_up.can_queue = self.offer_queue;
        snapshot.follow_up.can_steer = self.can_steer;
        snapshot.follow_up.can_resume &= self.send.is_ok();
        snapshot.follow_up.can_compact &= self.send.is_ok();
        if snapshot.actions.input_access != InputAccess::Editable {
            // Editing survives a read-only view: only the in-place
            // replacement is lost; continuing in a fork sends elsewhere.
            for editable in &mut snapshot.follow_up.editable {
                editable.in_place = false;
            }
            snapshot.follow_up.can_fork = false;
            for turn in &mut snapshot.transcript_turns {
                turn.can_fork = false;
            }
        }
    }
}

impl CoreRuntime {
    /// Read raw native/history capability plus this session's live facts.
    /// Call before the final admission projection overwrites `can_send`.
    pub(super) fn submission_admission(
        session: &session::Session,
        snapshot: &ConversationSnapshot,
    ) -> SubmissionAdmission {
        SubmissionAdmission::resolve(
            snapshot,
            SubmissionState {
                turn_running: session.turn_running(),
                cancel_requested: session.cancel_requested,
                compacting: session.compacting,
                side_chat: session.side.is_some(),
                native_can_steer: session
                    .active_state
                    .live()
                    .is_some_and(LiveConversation::can_steer),
            },
        )
    }

    /// The final projection step: no later calculation reads this as a
    /// native capability. Native/session state remains unchanged.
    pub(super) fn apply_submission_admission(&self, snapshot: &mut ConversationSnapshot) {
        let admission = Self::submission_admission(&self.session, snapshot);
        admission.apply(snapshot);
        if self.session.side.is_some() {
            snapshot.follow_up.can_resume = false;
            snapshot.follow_up.can_compact = false;
        }
    }

    pub(super) fn apply_read_submission_admission(snapshot: &mut ConversationSnapshot) {
        SubmissionAdmission::resolve(snapshot, SubmissionState::default()).apply(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{ConversationActions, ConversationSummary};

    fn snapshot(run_state: RunState) -> ConversationSnapshot {
        ConversationSnapshot {
            revision: 1,
            context_runtime_id: "runtime".into(),
            conversation: ConversationSummary {
                id: "runtime:synthetic".into(),
                title: crate::conversation::ConversationTitle::Untitled,
                runtime_id: "runtime".into(),
                cwd: None,
                updated_at_unix_ms: None,
                created_at_unix_ms: None,
                parent_conversation_id: None,
                native_read_state: None,
                can_rename: false,
                can_delete: false,
            },
            resource_id: None,
            model_record_key: None,
            run_state,
            messages: Vec::new(),
            executions: Default::default(),
            transcript_turns: Vec::new(),
            transcript_items: Vec::new(),
            transcript_outline: Vec::new(),
            transcript_message_identities: Vec::new(),
            message_identity_confirmations: Vec::new(),
            pending_interactions: Vec::new(),
            actions: ConversationActions {
                input_access: InputAccess::Editable,
                can_send: true,
                can_cancel: false,
                can_switch: true,
            },
            follow_up: Default::default(),
            sub_agents: Vec::new(),
            collaborators: Vec::new(),
        }
    }

    #[test]
    fn late_queue_acceptance_does_not_offer_queue_in_an_idle_editor() {
        for run_state in [RunState::Idle, RunState::Failed] {
            // Cancellation is retained until the next turn; it must not lock
            // the stopped session against a later explicit send or resume.
            let admission = SubmissionAdmission::resolve(
                &snapshot(run_state),
                SubmissionState {
                    cancel_requested: true,
                    ..Default::default()
                },
            );
            assert!(admission.send.is_ok());
            assert!(admission.accept_queue.is_ok());
            assert!(!admission.offer_queue);
            assert!(!admission.can_steer);
        }
    }

    #[test]
    fn retry_waiting_accepts_queue_but_preserves_native_steer_eligibility() {
        let snapshot = snapshot(RunState::Running);
        let mut state = SubmissionState {
            turn_running: true,
            ..Default::default()
        };
        let waiting = SubmissionAdmission::resolve(&snapshot, state);
        assert!(waiting.send.is_err());
        assert!(waiting.accept_queue.is_ok());
        assert!(waiting.offer_queue);
        assert!(!waiting.can_steer);
        state.native_can_steer = true;
        assert!(SubmissionAdmission::resolve(&snapshot, state).can_steer);
        state.cancel_requested = true;
        let stopping = SubmissionAdmission::resolve(&snapshot, state);
        assert!(stopping.accept_queue.is_err());
        assert!(!stopping.offer_queue && !stopping.can_steer);
    }

    #[test]
    fn submission_restrictions_apply_to_both_projection_and_queue_acceptance() {
        for access in [
            InputAccess::CheckingControl,
            InputAccess::ExternalController,
            InputAccess::ControlUnavailable,
            InputAccess::ExternalExecution,
            InputAccess::ReadOnly,
        ] {
            let mut snapshot = snapshot(RunState::Running);
            snapshot.actions.input_access = access;
            let admission = SubmissionAdmission::resolve(
                &snapshot,
                SubmissionState {
                    turn_running: true,
                    native_can_steer: true,
                    ..Default::default()
                },
            );
            assert!(admission.send.is_err() && admission.accept_queue.is_err());
            assert!(!admission.offer_queue && !admission.can_steer);
        }
        for run_state in [RunState::Stopping, RunState::Elsewhere] {
            let admission = SubmissionAdmission::resolve(&snapshot(run_state), Default::default());
            assert!(admission.send.is_err() && admission.accept_queue.is_err());
            assert!(!admission.offer_queue && !admission.can_steer);
        }
    }

    #[test]
    fn side_chats_keep_sending_without_gaining_queue_or_steer() {
        let mut state = SubmissionState {
            side_chat: true,
            native_can_steer: true,
            ..Default::default()
        };
        let idle = SubmissionAdmission::resolve(&snapshot(RunState::Idle), state);
        assert!(idle.send.is_ok());
        assert!(idle.accept_queue.is_err());
        state.turn_running = true;
        let running = SubmissionAdmission::resolve(&snapshot(RunState::Running), state);
        assert!(!running.offer_queue && !running.can_steer);
    }

    #[test]
    fn pending_interactions_block_a_new_turn_without_rejecting_the_running_queue() {
        let mut view = snapshot(RunState::Idle);
        view.pending_interactions
            .push(crate::conversation::RuntimeInteraction {
                id: "approval".into(),
                kind: crate::conversation::InteractionKind::Approval {
                    title: "Synthetic approval".into(),
                    detail: String::new(),
                    options: Vec::new(),
                },
            });
        view.follow_up.can_resume = true;
        CoreRuntime::apply_read_submission_admission(&mut view);
        assert!(!view.actions.can_send && !view.follow_up.can_resume);
        view.run_state = RunState::Running;
        view.actions.can_send = false; // The runtime's running source capability.
        let running = SubmissionAdmission::resolve(
            &view,
            SubmissionState {
                turn_running: true,
                ..Default::default()
            },
        );
        assert!(running.accept_queue.is_ok() && running.offer_queue);
    }

    #[test]
    fn control_projection_preserves_fork_only_edits_without_changing_the_source() {
        for access in [
            InputAccess::CheckingControl,
            InputAccess::ExternalController,
            InputAccess::ControlUnavailable,
            InputAccess::ExternalExecution,
            InputAccess::ReadOnly,
        ] {
            let mut source = snapshot(RunState::Idle);
            source.actions.input_access = access;
            source.follow_up.can_resume = true;
            source.follow_up.can_compact = true;
            source.follow_up.can_fork = true;
            source
                .follow_up
                .editable
                .push(crate::conversation::EditableMessage {
                    message_id: "user-message".into(),
                    in_place: true,
                });
            let mut delivered = source.clone();
            CoreRuntime::apply_read_submission_admission(&mut delivered);
            assert!(!delivered.actions.can_send);
            assert!(!delivered.follow_up.can_queue && !delivered.follow_up.can_steer);
            assert!(!delivered.follow_up.can_resume && !delivered.follow_up.can_compact);
            assert!(!delivered.follow_up.can_fork);
            assert_eq!(delivered.follow_up.editable.len(), 1);
            assert_eq!(delivered.follow_up.editable[0].message_id, "user-message");
            assert!(!delivered.follow_up.editable[0].in_place);
            assert!(source.actions.can_send && source.follow_up.can_resume);
            assert!(source.follow_up.editable[0].in_place);
        }
    }

    #[test]
    fn maintenance_and_child_contexts_do_not_gain_submission_actions() {
        let idle = snapshot(RunState::Idle);
        let compacting = SubmissionAdmission::resolve(
            &idle,
            SubmissionState {
                compacting: true,
                ..Default::default()
            },
        );
        assert!(compacting.send.is_err() && compacting.accept_queue.is_err());
        let mut child = idle;
        child.conversation.parent_conversation_id = Some("parent".into());
        let admission = SubmissionAdmission::resolve(&child, Default::default());
        assert!(admission.send.is_err() && admission.accept_queue.is_err());
    }

    #[test]
    fn late_queue_command_keeps_the_stopped_conversations_pause() {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-queue-admission-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "runtime".into(),
            name: "Synthetic ACP".into(),
            type_id: RuntimeKind::Acp,
            gateway_id: String::new(),
            settings: Default::default(),
        }];
        core.session.active_state = ActiveState::History(Box::new(snapshot(RunState::Idle)));
        core.session.cancel_requested = true;
        core.follow_finished_turn(crate::TurnOutcome::Cancelled);
        let result = core
            .send_queue(
                "runtime:synthetic",
                crate::SendQueueOperation::Add {
                    source_runtime_id: "runtime".into(),
                    target_runtime_id: "runtime".into(),
                    selection: None,
                    turn: crate::ComposedTurn {
                        text: "queued before stop completed".into(),
                        effort: None,
                        permission_mode: None,
                        attachments: Vec::new(),
                    },
                },
            )
            .unwrap();
        let view = result.snapshot.unwrap();
        assert_eq!(view.follow_up.queue.len(), 1);
        assert_eq!(
            view.follow_up.paused,
            Some(crate::conversation::QueuePause::TurnStopped)
        );
        assert!(!view.follow_up.can_queue);
        assert!(view.actions.can_send);
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }
}
