//! Submission admission shared by command execution and its public projection.
//! Draft contents, input methods and requests in flight remain client state.
use super::*;
use crate::conversation::{ControlHandoff, ConversationSnapshot, InputAccess};

#[derive(Clone, Copy, Default)]
pub(super) struct SubmissionState {
    pub(super) turn_running: bool,
    pub(super) cancel_requested: bool,
    pub(super) compacting: bool,
    pub(super) preparing_context: bool,
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
        let handed_off = matches!(
            snapshot.follow_up.control_handoff,
            Some(ControlHandoff::Releasing | ControlHandoff::Released)
        );
        let common = if handed_off {
            Err("会话已申请交还，请先恢复 Velune 控制")
        } else if snapshot.actions.input_access != InputAccess::Editable {
            Err("当前会话不能提交输入")
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
            if state.compacting
                || state.preparing_context
                || state.turn_running
                || snapshot.run_state == RunState::Running
            {
                Err("runtime is busy")
            } else if !snapshot.actions.can_send {
                Err("当前原生上下文不可用，请修复来源后重新打开；未自动重发")
            } else if snapshot
                .pending_interactions
                .iter()
                .any(|interaction| interaction.blocks_turn())
            {
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
        let offer_queue = accept_queue.is_ok()
            && (state.compacting
                || state.preparing_context
                || (state.turn_running && snapshot.run_state == RunState::Running));
        Self {
            send,
            accept_queue,
            offer_queue,
            can_steer: offer_queue
                && !state.compacting
                && !state.preparing_context
                && state.native_can_steer,
        }
    }

    fn apply(&self, snapshot: &mut ConversationSnapshot) {
        snapshot.actions.can_send = self.send.is_ok();
        snapshot.follow_up.can_queue = self.offer_queue;
        snapshot.follow_up.can_steer = self.can_steer;
        snapshot.follow_up.can_resume &= self.send.is_ok();
        snapshot.follow_up.can_compact &= self.send.is_ok();
        let handed_off = matches!(
            snapshot.follow_up.control_handoff,
            Some(ControlHandoff::Releasing | ControlHandoff::Released)
        );
        if snapshot.actions.input_access != InputAccess::Editable || handed_off {
            // Source write access only governs replacement in place. Forks
            // create a different conversation, so retain the completed-turn
            // eligibility already derived by editing.rs.
            for editable in &mut snapshot.follow_up.editable {
                editable.in_place = false;
            }
        }
        if handed_off {
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
                preparing_context: session.send_queue.prepares_context()
                    || session.native_controls.submission_pending()
                    || matches!(session.active_state, ActiveState::Pending(_)),
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
        if self.session.send_queue.preparation_cancellable() {
            snapshot.actions.can_cancel = true;
        }
        if self.session.side.is_some() {
            snapshot.follow_up.can_resume = false;
            snapshot.follow_up.can_compact = false;
        }
    }

    #[cfg(test)]
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
            retained_reply_ids: Vec::new(),
            work_notification_ids: Vec::new(),
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

    fn fork_turn(
        id: &str,
        is_running: bool,
        can_fork: bool,
    ) -> crate::conversation::TranscriptTurn {
        crate::conversation::TranscriptTurn {
            id: id.into(),
            user_message_id: format!("{id}:user"),
            work_message_ids: Vec::new(),
            last_message_id: Some(format!("{id}:reply")),
            reply_start_block: 0,
            duration_ms: None,
            is_running,
            can_fork,
            outcome: None,
            settings: None,
            usage: None,
        }
    }

    #[test]
    fn control_projection_preserves_forks_and_fork_only_edits_without_changing_the_source() {
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
            source.transcript_turns = vec![
                fork_turn("completed", false, true),
                fork_turn("running", true, false),
            ];
            source
                .follow_up
                .editable
                .push(crate::conversation::EditableMessage {
                    revision: None,
                    message_id: "user-message".into(),
                    in_place: true,
                    delete_only: false,
                    delete_following: false,
                });
            let mut delivered = source.clone();
            CoreRuntime::apply_read_submission_admission(&mut delivered);
            assert!(!delivered.actions.can_send);
            assert!(!delivered.follow_up.can_queue && !delivered.follow_up.can_steer);
            assert!(!delivered.follow_up.can_resume && !delivered.follow_up.can_compact);
            assert!(delivered.follow_up.can_fork);
            assert!(delivered.transcript_turns[0].can_fork);
            assert!(!delivered.transcript_turns[1].can_fork);
            assert_eq!(delivered.follow_up.editable.len(), 1);
            assert_eq!(delivered.follow_up.editable[0].message_id, "user-message");
            assert!(!delivered.follow_up.editable[0].in_place);
            assert!(source.actions.can_send && source.follow_up.can_resume);
            assert!(source.follow_up.editable[0].in_place);
            assert!(source.transcript_turns[0].can_fork);
            let wire = super::super::delivery::TranscriptDelivery::new()
                .snapshot(delivered, &std::collections::BTreeSet::new());
            assert!(wire.follow_up.can_fork);
            assert!(wire.transcript_turns[0].can_fork);
            assert!(!wire.transcript_turns[1].can_fork);
        }
        // Explicit control handoff remains a separate operation policy; this
        // change only removes source-write admission from read-only forks.
        for handoff in [ControlHandoff::Releasing, ControlHandoff::Released] {
            let mut view = snapshot(RunState::Idle);
            view.follow_up.control_handoff = Some(handoff);
            view.follow_up.can_fork = true;
            view.transcript_turns
                .push(fork_turn("completed", false, true));
            CoreRuntime::apply_read_submission_admission(&mut view);
            assert!(!view.follow_up.can_fork);
            assert!(!view.transcript_turns[0].can_fork);
        }
    }

    #[test]
    fn async_questions_allow_input_and_dismissal_survives_reload() {
        use crate::conversation::{
            InteractionKind, InteractionQuestion, RuntimeInteraction, RuntimeInteractionReply,
        };
        let mut source = snapshot(RunState::Idle);
        source.pending_interactions.push(RuntimeInteraction {
            id: "codex-async:question".into(),
            kind: InteractionKind::AsyncUserInput {
                delivery: crate::conversation::AsyncQuestionDelivery::ConversationInput,
                questions: vec![InteractionQuestion {
                    id: "0".into(),
                    text: "Question".into(),
                    options: vec![],
                    secret: false,
                    default_text: None,
                    multiline: false,
                    allow_other: true,
                    multiple: false,
                }],
            },
        });
        assert!(
            SubmissionAdmission::resolve(&source, Default::default())
                .send
                .is_ok()
        );
        let mut linked = source.clone();
        linked.conversation.runtime_id = "source-runtime".into();
        linked.context_runtime_id = "execution-runtime".into();
        let request =
            super::super::native_composition::async_turn_request(&linked, None, "answer".into());
        assert_eq!(request.source_runtime_id, "source-runtime");
        assert_eq!(request.runtime_id, "execution-runtime");
        assert_eq!(request.conversation_id, linked.conversation.id);
        let root =
            std::env::temp_dir().join(format!("velune-async-question-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let options = crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        };
        let mut application = crate::Application::open(options.clone()).unwrap();
        let core = &mut application.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "runtime".into(),
            name: "Synthetic".into(),
            type_id: RuntimeKind::Acp,
            gateway_id: String::new(),
            settings: Default::default(),
        }];
        core.session.active_state = ActiveState::History(Box::new(source.clone().into()));
        let failed = core.reply_runtime_interaction(
            "runtime",
            "codex-async:question",
            RuntimeInteractionReply::Answers {
                answers: vec![crate::conversation::InteractionAnswer {
                    question_id: "0".into(),
                    values: vec!["answer".into()],
                }],
            },
        );
        assert!(failed.is_err());
        assert!(
            core.organization
                .conversations
                .iter()
                .all(|mark| mark.resolved_async_questions.is_empty())
        );
        assert_eq!(
            core.current_snapshot().unwrap().pending_interactions.len(),
            1
        );
        assert!(
            core.active_conversations()
                .iter()
                .all(|entry| entry.pending_interactions == 0)
        );
        let mut expired = source.clone();
        let mut rpc = expired.pending_interactions[0].clone();
        rpc.id = "native-request".into();
        if let InteractionKind::AsyncUserInput { delivery, .. } = &mut rpc.kind {
            *delivery = crate::conversation::AsyncQuestionDelivery::NativeReply;
        }
        expired.pending_interactions.push(rpc);
        core.session.active_state = ActiveState::History(Box::new(expired.into()));
        core.invalidate_execution().unwrap();
        assert_eq!(
            core.current_snapshot().unwrap().pending_interactions.len(),
            1
        );
        assert!(
            core.reply_runtime_interaction(
                "runtime",
                "native-request",
                RuntimeInteractionReply::Cancel
            )
            .is_err()
        );
        assert!(core.session.send_queue.is_empty());

        let view = core
            .reply_runtime_interaction(
                "runtime",
                "codex-async:question",
                RuntimeInteractionReply::Cancel,
            )
            .unwrap()
            .snapshot
            .unwrap();
        assert!(view.pending_interactions.is_empty());
        assert!(
            core.active_conversations()
                .iter()
                .all(|entry| entry.pending_interactions == 0)
        );

        // A presentation write failure keeps the explicit dismissal in memory,
        // then retries that write without repeating any user input.
        let mut retry_source = source.clone();
        retry_source.pending_interactions[0].id = "codex-async:retry".into();
        core.session.active_state = ActiveState::History(Box::new(retry_source.into()));
        let marks_path = root.join("conversation-organization.json");
        std::fs::rename(&marks_path, root.join("previous-organization.json")).unwrap();
        std::fs::create_dir(&marks_path).unwrap();
        assert!(
            core.reply_runtime_interaction(
                "runtime",
                "codex-async:retry",
                RuntimeInteractionReply::Cancel
            )
            .is_err()
        );
        assert!(
            core.current_snapshot()
                .unwrap()
                .pending_interactions
                .is_empty()
        );
        assert!(core.async_question_marks_retry_at.is_some());
        std::fs::remove_dir(&marks_path).unwrap();
        core.async_question_marks_retry_at = Some(std::time::Instant::now());
        core.current_snapshot();
        assert!(core.async_question_marks_retry_at.is_none());
        assert!(marks_path.is_file());
        drop(application);
        let mut application = crate::Application::open(options).unwrap();
        application.runtime.session.active_state = ActiveState::History(Box::new(source.into()));
        assert!(
            application
                .runtime
                .current_snapshot()
                .unwrap()
                .pending_interactions
                .is_empty()
        );
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn idle_parent_history_alone_does_not_clear_the_viewed_childs_loading_state() {
        let root =
            std::env::temp_dir().join(format!("velune-child-loading-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        let parent = snapshot(RunState::Idle);
        let mut child = snapshot(RunState::Elsewhere);
        child.conversation.parent_conversation_id = Some(parent.conversation.id.clone());
        child.conversation.id = "runtime:child".into();
        child.actions.can_send = false;
        core.parked.push(session::Session {
            active_state: ActiveState::History(Box::new(parent.into())),
            ..Default::default()
        });
        core.session.active_state = ActiveState::History(Box::new(child.into()));
        core.follow_source();
        let view = core.native_snapshot().unwrap();
        assert_eq!(view.run_state, RunState::Elsewhere);
        assert!(!view.actions.can_send);
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn maintenance_accepts_queue_but_child_contexts_remain_read_only() {
        let idle = snapshot(RunState::Idle);
        let compacting = SubmissionAdmission::resolve(
            &idle,
            SubmissionState {
                compacting: true,
                ..Default::default()
            },
        );
        assert!(compacting.send.is_err() && compacting.accept_queue.is_ok());
        assert!(compacting.offer_queue && !compacting.can_steer);
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
        core.session.active_state = ActiveState::History(Box::new(snapshot(RunState::Idle).into()));
        // A maintenance run accepts input even when its snapshot is idle,
        // but must not start the queued runtime until maintenance settles.
        core.session.compacting = true;
        let queued = core
            .send_queue(
                "runtime:synthetic",
                crate::SendQueueOperation::Add {
                    source_runtime_id: "runtime".into(),
                    target_runtime_id: "runtime".into(),
                    selection: None,
                    turn: crate::ComposedTurn {
                        text: "during compaction".into(),
                        effort: None,
                        permission_mode: None,
                        attachments: vec![],
                    },
                },
            )
            .unwrap()
            .snapshot
            .unwrap();
        assert_eq!(queued.follow_up.queue.len(), 1);
        assert!(queued.follow_up.can_queue);
        assert!(queued.follow_up.paused.is_none());
        let id = queued.follow_up.queue[0].id.clone();
        core.send_queue(
            "runtime:synthetic",
            crate::SendQueueOperation::Edit {
                id: id.clone(),
                text: "changed during compaction".into(),
            },
        )
        .unwrap();
        core.send_queue(
            "runtime:synthetic",
            crate::SendQueueOperation::Remove { id },
        )
        .unwrap();
        core.session.compacting = false;
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

    #[test]
    #[ignore = "manual large-transcript admission timing"]
    fn synthetic_long_send_measurement() {
        let root = std::env::temp_dir().join(format!(
            "velune-send-measurement-{:x}",
            getrandom::u64().unwrap()
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
            settings: std::collections::BTreeMap::from([(
                "binary".into(),
                root.join("nonexistent-synthetic-agent")
                    .to_string_lossy()
                    .into_owned(),
            )]),
        }];
        let mut source = snapshot(RunState::Idle);
        for i in 0..192 {
            source.messages.push(velune_conversation::Message {
                id: format!("m-{i}"),
                completed: true,
                role: if i % 2 == 0 {
                    velune_conversation::MessageRole::User
                } else {
                    velune_conversation::MessageRole::Assistant
                },
                timestamp_unix_ms: None,
                blocks: vec![velune_conversation::MessageBlock::Text {
                    text: "x".repeat(192 * 1024),
                }],
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
            });
        }
        core.session.set_history(source);
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let result = core.send_turn(super::super::conversations::TurnRequest {
                submission_id: None,
                conversation_id: "runtime:synthetic".into(),
                source_runtime_id: "runtime".into(),
                runtime_id: "runtime".into(),
                selection: Some(crate::ModelSelection::Managed {
                    model_record_key: "unsupported".into(),
                }),
                turn: crate::ComposedTurn {
                    text: "hi".into(),
                    effort: None,
                    permission_mode: None,
                    attachments: vec![],
                },
            });
            let err = result.err().expect("ACP cannot take a managed selection");
            assert!(err.to_string().contains("通用 ACP"), "{err}");
            eprintln!("send_admission_ms={}", start.elapsed().as_millis());
        }
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }
}
