use super::*;

const MAX_LIST_WAITERS: usize = 128;

pub(super) struct PendingListRefresh {
    stage: ListRefreshStage,
    waiters: Vec<ListWaiter>,
}

enum ListRefreshStage {
    Warming,
    Reading { revision: Revision, retried: bool },
}

struct ListWaiter {
    view: ClientViewId,
    view_created: u64,
    remote: bool,
    reply: CommandReply,
}

impl Owner {
    pub(super) fn schedule_codex_index_warmup(&mut self) {
        let Some(warmup) = self.application.runtime.prepare_codex_index_warmup() else {
            return;
        };
        let job = HistoryJob {
            completion: self.owner_sender.clone(),
            id: self.view_clock,
            work: HistoryWork::WarmCodexIndex(warmup),
            queued_at: Instant::now(),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            span: tracing::Span::current(),
        };
        if self.history.jobs.try_send(job).is_ok() {
            // A list waits here while the startup scan occupies a single
            // worker, so the other worker can read a conversation.
            self.pending_list_refresh = Some(PendingListRefresh {
                stage: ListRefreshStage::Warming,
                waiters: Vec::new(),
            });
        }
    }

    pub(super) fn finish_codex_index_warmup(&mut self, completion: mpsc::Sender<Job>) {
        let Some(PendingListRefresh {
            stage: ListRefreshStage::Warming,
            waiters,
        }) = self.pending_list_refresh.take()
        else {
            unreachable!("history worker returned the pending Codex warmup");
        };
        for waiter in waiters {
            self.request_list_refresh(waiter, false, completion.clone());
        }
    }

    pub(super) fn start_list_refresh(
        &mut self,
        view: ClientViewId,
        remote: bool,
        reply: CommandReply,
        completion: mpsc::Sender<Job>,
    ) {
        let view_created = self.views[&view].created;
        self.request_list_refresh(
            ListWaiter {
                view,
                view_created,
                remote,
                reply,
            },
            false,
            completion,
        );
    }

    fn request_list_refresh(
        &mut self,
        waiter: ListWaiter,
        retried: bool,
        completion: mpsc::Sender<Job>,
    ) {
        if self.list_view_expired(&waiter) {
            let _ = waiter.reply.send(Err(list_view_closed()));
            return;
        }
        let request = self.with_view(&waiter.view, waiter.remote, |application| {
            application
                .runtime
                .serve(Focus::None, Login::Allows, |core| {
                    Ok(core.prepare_list_refresh())
                })
        });
        let request = match request {
            Ok(Some(request)) => request,
            Ok(None) => {
                let result = self.read_refreshed_list(&waiter);
                let _ = waiter.reply.send(result);
                return;
            }
            Err(error) => {
                let _ = waiter.reply.send(Err(Failure::from(error)));
                return;
            }
        };
        if let Some(pending) = &mut self.pending_list_refresh {
            if pending.waiters.len() < MAX_LIST_WAITERS {
                pending.waiters.push(waiter);
            } else {
                let _ = waiter.reply.send(Err(list_refresh_busy()));
            }
            return;
        }
        let job = HistoryJob {
            completion,
            id: self.view_clock,
            work: HistoryWork::ListRefresh(request),
            queued_at: Instant::now(),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            span: tracing::Span::current(),
        };
        if self.history.jobs.try_send(job).is_err() {
            let _ = waiter.reply.send(Err(list_refresh_busy()));
            return;
        }
        self.pending_list_refresh = Some(PendingListRefresh {
            stage: ListRefreshStage::Reading {
                revision: self.revision,
                retried,
            },
            waiters: vec![waiter],
        });
    }

    pub(super) fn finish_list_refresh(
        &mut self,
        loaded: LoadedListRefresh,
        completion: mpsc::Sender<Job>,
    ) {
        let Some(PendingListRefresh {
            stage: ListRefreshStage::Reading { revision, retried },
            waiters,
        }) = self.pending_list_refresh.take()
        else {
            unreachable!("history worker returned the pending list scan");
        };
        // Commands can change native archive membership or replace runtime
        // settings while the worker is reading those sources.
        let installed = revision == self.revision
            && self
                .application
                .runtime
                .install_list_refresh(loaded)
                .is_ok();
        for waiter in waiters {
            let result = if self.list_view_expired(&waiter) {
                Err(list_view_closed())
            } else if installed {
                self.read_refreshed_list(&waiter)
            } else if !retried {
                self.request_list_refresh(waiter, true, completion.clone());
                continue;
            } else {
                let mut failure = Failure::new(
                    FailureKind::Conflict,
                    "读取期间会话列表已改变，请刷新后重试",
                );
                failure.current_revision = Some(self.revision);
                Err(failure)
            };
            let _ = waiter.reply.send(result);
        }
    }

    fn list_view_expired(&self, waiter: &ListWaiter) -> bool {
        self.views
            .get(&waiter.view)
            .is_none_or(|view| view.created != waiter.view_created)
    }

    fn read_refreshed_list(&mut self, waiter: &ListWaiter) -> Result<ApplicationReply, Failure> {
        let snapshot = self.with_view(&waiter.view, waiter.remote, |application| {
            application
                .runtime
                .serve(Focus::None, Login::Allows, |core| {
                    if core.shows_archived() {
                        core.list()
                    } else {
                        Ok(core.list_refreshed())
                    }
                })
        })?;
        self.response_for(waiter.remote, Response::ReadConfiguration(snapshot))
    }
}

fn list_view_closed() -> Failure {
    Failure::new(FailureKind::Conflict, "列表请求的视图已关闭")
}

fn list_refresh_busy() -> Failure {
    Failure::new(FailureKind::Unavailable, "会话列表读取繁忙，请稍后重试")
}
