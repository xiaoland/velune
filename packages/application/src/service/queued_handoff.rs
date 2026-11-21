//! One owner command accepts the queued input before any model work can poll.
//! The ordinary session queue remains the only owner of pending messages.
use super::*;
use crate::local::{PreparationState, TurnRequest};

impl Owner {
    pub(super) fn queue_handoff_summary(
        &mut self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        mut command: Command,
        reply: CommandReply,
        completion: mpsc::Sender<Job>,
    ) {
        if context.is_some() {
            let mut failure =
                Failure::new(FailureKind::Unsupported, "当前远程控制连接尚不支持摘要接续");
            failure.code = "portable_summary_unsupported_remote".into();
            let _ = reply.send(Err(failure));
            return;
        }
        if let Err(error) = self
            .application
            .resolve_input_attachments(&mut command, &view, false)
        {
            let _ = reply.send(Err(error));
            return;
        }
        let (request, retry, summary_model, effort) = match command {
            Command::QueueHandoffSummary {
                submission_id,
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection,
                turn,
                summary_model,
                effort,
            } => {
                if !self
                    .application
                    .runtime
                    .handoff_queue_is_empty(&conversation_id)
                {
                    let _ = reply.send(Err(Failure::new(
                        FailureKind::Conflict,
                        "已有排队消息，请处理现有队列后再开始运行时切换",
                    )));
                    return;
                }
                (
                    TurnRequest {
                        submission_id,
                        source_runtime_id,
                        conversation_id,
                        runtime_id: target_runtime_id,
                        selection,
                        turn,
                    },
                    None,
                    summary_model,
                    effort,
                )
            }
            Command::RetryQueuedHandoffSummary {
                source_runtime_id,
                conversation_id,
                queued_message_id,
                summary_model,
                effort,
            } => {
                if let Err(error) = self
                    .application
                    .runtime
                    .require_queued_handoff_retry(&conversation_id, &queued_message_id)
                {
                    let _ = reply.send(Err(Failure::from(error)));
                    return;
                }
                let request = match self
                    .application
                    .runtime
                    .queued_handoff_turn(&conversation_id, &queued_message_id)
                {
                    Ok(request) if request.source_runtime_id == source_runtime_id => request,
                    Ok(_) => {
                        let _ = reply.send(Err(Failure::new(
                            FailureKind::Conflict,
                            "排队会话来源已改变",
                        )));
                        return;
                    }
                    Err(error) => {
                        let _ = reply.send(Err(Failure::from(error)));
                        return;
                    }
                };
                (request, Some(queued_message_id), summary_model, effort)
            }
            _ => {
                let _ = reply.send(Err(Failure::new(
                    FailureKind::Contract,
                    "queued handoff command mismatch",
                )));
                return;
            }
        };
        // This existing initializer validates/reserves work and only schedules
        // creation of its inert native state machine. Native model calls start
        // when the owner polls the installed work, after this command returns.
        let (accepted, result) = mpsc::channel();
        self.begin_handoff_summary(BeginHandoffSummaryRequest {
            source_runtime_id: request.source_runtime_id.clone(),
            conversation_id: request.conversation_id.clone(),
            target_runtime_id: request.runtime_id.clone(),
            summary_model,
            effort,
            remote: false,
            reply: accepted,
            completion: completion.clone(),
        });
        let work_id = match result.try_recv() {
            Ok(Ok(ApplicationReply {
                response: Response::BeginHandoffSummary(start),
                ..
            })) => start.work_id,
            Ok(Err(error)) => {
                let _ = reply.send(Err(error));
                return;
            }
            _ => {
                let _ = reply.send(Err(Failure::new(
                    FailureKind::Contract,
                    "summary reservation did not reply",
                )));
                return;
            }
        };
        let request_conversation_id = request.conversation_id.clone();
        let queued = self
            .with_view(&view, false, |application| {
                application.runtime.serve(
                    Focus::Conversation(request.conversation_id.clone()),
                    Login::Allows,
                    |core| core.begin_queued_handoff(request, &work_id, retry.as_deref()),
                )
            })
            .map_err(Failure::from);
        let (queued_message_id, result) = match queued {
            Ok(value) => value,
            Err(error) => {
                let (cancelled, _) = mpsc::channel();
                self.cancel_handoff_summary(work_id, false, cancelled);
                let _ = reply.send(Err(error));
                return;
            }
        };
        let cancellation = Arc::new(AtomicBool::new(false));
        if let Ok(mut index) = self.queued_handoff_cancellations.lock() {
            index.insert(
                work_id.clone(),
                QueuedHandoffCancellation {
                    conversation_id: request_conversation_id.clone(),
                    queued_message_id: queued_message_id.clone(),
                    flag: cancellation.clone(),
                },
            );
        }
        if let Some(work) = self.handoff_summary_works.get_mut(&work_id) {
            work.queued = Some(QueuedHandoffBinding {
                queued_message_id: queued_message_id.clone(),
                view: ClientViewId(format!("queued-handoff:{work_id}")),
                cancellation,
                accepted_execution_id: None,
            });
        }
        self.revision.0 = self.revision.0.wrapping_add(1);
        let started = velune_application_api::QueuedHandoffSummaryStart {
            work_id,
            queued_message_id,
            result,
        };
        let response = if retry.is_some() {
            Response::RetryQueuedHandoffSummary(started)
        } else {
            Response::QueueHandoffSummary(started)
        };
        let _ = reply.send(self.response_for(false, response));
    }

    pub(super) fn advance_queued_handoffs(&mut self) {
        let work_ids: Vec<_> = self
            .handoff_summary_works
            .iter()
            .filter(|(_, work)| work.queued.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in work_ids {
            if self
                .handoff_summary_works
                .get(&id)
                .is_some_and(|work| work.delivery_accepted)
            {
                continue;
            }
            let state = self.application.runtime.queued_preparation_state(&id);
            if state.is_none() || state == Some(PreparationState::Cancelled) {
                let should_cancel = self.handoff_summary_works.get(&id).is_some_and(|work| {
                    !work.cancel_requested
                        && matches!(
                            work.status,
                            velune_application_api::HandoffSummaryStatus::Running
                                | velune_application_api::HandoffSummaryStatus::Ready { .. }
                        )
                });
                if should_cancel {
                    let (reply, _) = mpsc::channel();
                    self.cancel_handoff_summary(id.clone(), false, reply);
                }
                if state.is_none()
                    && let Some(work) = self.handoff_summary_works.get_mut(&id)
                {
                    work.queued = None;
                }
                continue;
            }
            let Some(work) = self.handoff_summary_works.get(&id) else {
                continue;
            };
            let projected = match work.status {
                velune_application_api::HandoffSummaryStatus::Running => PreparationState::Running,
                velune_application_api::HandoffSummaryStatus::Cancelling => {
                    PreparationState::Cancelling
                }
                velune_application_api::HandoffSummaryStatus::Ready { .. } => {
                    PreparationState::Ready
                }
                velune_application_api::HandoffSummaryStatus::Failed { .. } => {
                    PreparationState::Failed
                }
                velune_application_api::HandoffSummaryStatus::Cancelled => {
                    PreparationState::Cancelled
                }
            };
            self.application
                .runtime
                .sync_queued_preparation(&id, projected);
            if work.send_pending || work.delivery_accepted || projected != PreparationState::Ready {
                continue;
            }
            let Some(token) = work.prepared_token.clone() else {
                continue;
            };
            let Some(binding) = &work.queued else {
                continue;
            };
            let view = binding.view.clone();
            let completion = self.owner_sender.clone();
            let cancellation = binding.cancellation.clone();
            let Some(request) = self.application.runtime.queued_prepared_candidate(&id) else {
                continue;
            };
            if self
                .application
                .runtime
                .queued_handoff_turn(&request.conversation_id, &binding.queued_message_id)
                .is_err()
            {
                self.application.runtime.pause_queued_preparation(&id);
                continue;
            }
            let command = Command::SendPreparedHandoff {
                source_runtime_id: request.source_runtime_id,
                conversation_id: request.conversation_id,
                target_runtime_id: request.runtime_id,
                selection: request.selection,
                turn: request.turn,
                prepared_token: token,
            };
            let (reply, result) = mpsc::channel();
            self.send_prepared_handoff(view, None, command, Some(cancellation), reply, completion);
            if result.try_recv().is_ok() {
                self.application.runtime.pause_queued_preparation(&id);
                if self
                    .handoff_summary_works
                    .get(&id)
                    .is_some_and(|work| work.bound_request_digest.is_none())
                {
                    self.application.runtime.unbind_queued_preparation(&id);
                }
            }
        }
        if let Ok(mut index) = self.queued_handoff_cancellations.lock() {
            index.retain(|id, _| {
                self.handoff_summary_works
                    .get(id)
                    .is_some_and(|work| work.queued.is_some())
            });
        }
    }
}
