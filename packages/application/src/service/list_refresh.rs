use super::*;

const MAX_LIST_WAITERS: usize = 128;

pub(super) struct PendingListRefresh {
    revision: Revision,
    retried: bool,
    waiters: Vec<ListWaiter>,
}

struct ListWaiter {
    view: ClientViewId,
    view_created: u64,
    remote: bool,
    reply: CommandReply,
}

impl Owner {
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
            revision: self.revision,
            retried,
            waiters: vec![waiter],
        });
    }

    pub(super) fn finish_list_refresh(
        &mut self,
        loaded: LoadedListRefresh,
        completion: mpsc::Sender<Job>,
    ) {
        let pending = self
            .pending_list_refresh
            .take()
            .expect("history worker returned the pending list scan");
        // Commands can change native archive membership or replace runtime
        // settings while the worker is reading those sources.
        let installed = pending.revision == self.revision
            && self
                .application
                .runtime
                .install_list_refresh(loaded)
                .is_ok();
        for waiter in pending.waiters {
            let expired = self
                .views
                .get(&waiter.view)
                .is_none_or(|view| view.created != waiter.view_created);
            let result = if expired {
                Err(Failure::new(FailureKind::Conflict, "列表请求的视图已关闭"))
            } else if installed {
                self.read_refreshed_list(&waiter)
            } else if !pending.retried {
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

fn list_refresh_busy() -> Failure {
    Failure::new(FailureKind::Unavailable, "会话列表读取繁忙，请稍后重试")
}
