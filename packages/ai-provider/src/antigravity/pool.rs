//! Retain an ACP process only while its caller resolves a tool request.
use super::{AntigravityConfig, acp, mcp, messages};
use crate::claude_code::{Continuation, Executor};
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;
use velune_ai::{InvalidContract, NativeErrorKind as Error, NativeFailure};

const TOOL_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_ACTIVE: usize = 64;

#[derive(Default)]
struct Sessions {
    slots: Vec<Arc<Slot>>,
    closed: bool,
}

pub struct AntigravityPool {
    sessions: Mutex<Sessions>,
    pub(super) executor: Executor,
}

impl Default for AntigravityPool {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(Sessions::default()),
            executor: Executor::new("velune-antigravity"),
        }
    }
}

impl AntigravityPool {
    pub fn begin_user_turn(&self, session: &str) -> Result<(), InvalidContract> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.closed {
            return Err(InvalidContract("Antigravity pool is closed"));
        }
        if sessions
            .slots
            .iter()
            .any(|slot| slot.session.as_deref() == Some(session) && slot.is_active())
        {
            return Err(InvalidContract("Antigravity session still has active work"));
        }
        sessions
            .slots
            .retain(|slot| slot.session.as_deref() != Some(session));
        Ok(())
    }

    pub fn finish_user_turn(&self, session: &str) {
        self.cancel_session(session);
    }

    pub fn cancel_session(&self, session: &str) {
        self.cancel_where(|slot| slot.session.as_deref() == Some(session));
    }

    pub fn invalidate(&self, binding: &str) {
        self.cancel_where(|slot| slot.binding == binding);
    }

    pub fn shutdown(&self) {
        self.sessions.lock().unwrap().closed = true;
        self.cancel_where(|_| true);
    }

    pub fn continuation(&self, session: &str) -> Continuation {
        let sessions = self.sessions.lock().unwrap();
        let Some(slot) = sessions
            .slots
            .iter()
            .find(|slot| slot.session.as_deref() == Some(session))
        else {
            return Continuation::None;
        };
        match &*slot.state.lock().unwrap() {
            Phase::Parked(parked) => slot.pin(parked),
            Phase::Invalidated => Continuation::Invalidated,
            _ => Continuation::None,
        }
    }

    /// Resolve ownership and binding in one snapshot. Even if a concurrent
    /// request then consumes the call, the router can only dispatch here.
    pub fn tool_result_owner(
        &self,
        session: &str,
        ids: &[String],
    ) -> Result<Continuation, InvalidContract> {
        let Some(id) = ids.iter().find(|id| id.starts_with(mcp::CALL_ID_PREFIX)) else {
            return Ok(Continuation::None);
        };
        if ids.len() == 1 {
            let sessions = self.sessions.lock().unwrap();
            if let Some(slot) = sessions
                .slots
                .iter()
                .find(|slot| slot.session.as_deref() == Some(session))
                && let Phase::Parked(parked) = &*slot.state.lock().unwrap()
                && &parked.call.id == id
            {
                return Ok(slot.pin(parked));
            }
        }
        Err(InvalidContract(
            "Antigravity tool result has no matching active call",
        ))
    }

    fn cancel_where(&self, matches: impl Fn(&Slot) -> bool) {
        let sessions = self.sessions.lock().unwrap();
        for slot in sessions.slots.iter().filter(|slot| matches(slot)) {
            slot.invalidate(None);
        }
    }

    pub(super) fn claim(
        &self,
        config: &AntigravityConfig,
        request: &messages::Request,
    ) -> Result<Lease, NativeFailure> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.closed {
            return Err(NativeFailure::unsent(Error::Cancelled));
        }
        sessions.slots.retain(|slot| {
            let phase = slot.state.lock().unwrap();
            !matches!(*phase, Phase::Finished)
                && (slot.session.is_some() || !matches!(*phase, Phase::Invalidated))
        });
        if let Some(session) = &config.session
            && let Some(slot) = sessions
                .slots
                .iter()
                .find(|slot| slot.session.as_ref() == Some(session))
        {
            let mut state = slot.state.lock().unwrap();
            match &*state {
                Phase::Parked(parked) => {
                    // Do not consume a pending call on a mismatched or malformed result.
                    if slot.binding != config.binding
                        || parked.tools != request.tools
                        || parked.system != request.system
                        || parked.model != request.model
                    {
                        return Err(NativeFailure::sent(Error::InvalidInput, None));
                    }
                    let result = request
                        .tool_result(&parked.call.id)
                        .map_err(|kind| NativeFailure::sent(kind, None))?;
                    let Phase::Parked(parked) = std::mem::replace(&mut *state, Phase::Active)
                    else {
                        unreachable!()
                    };
                    return Ok(Lease {
                        slot: slot.clone(),
                        previous: Some((*parked, result)),
                        submitted: true,
                        settled: false,
                    });
                }
                Phase::Active => return Err(NativeFailure::sent(Error::InvalidInput, None)),
                Phase::Invalidated => return Err(NativeFailure::sent(Error::Cancelled, None)),
                Phase::Finished => {
                    // A just-settled slot can finish while claim holds the sessions lock.
                    // The next request may allocate another slot once this one is pruned.
                    return Err(NativeFailure::unsent(Error::InvalidInput));
                }
            }
        }
        if request.is_tool_result() {
            return Err(NativeFailure::unsent(Error::InvalidInput));
        }
        if sessions
            .slots
            .iter()
            .filter(|slot| slot.is_active())
            .count()
            >= MAX_ACTIVE
        {
            return Err(NativeFailure::unsent(Error::Overloaded));
        }
        let slot = Arc::new(Slot {
            session: config.session.clone(),
            binding: config.binding.clone(),
            state: Mutex::new(Phase::Active),
            stop: watch::channel(false).0,
            handle: self.executor.handle.clone(),
        });
        sessions.slots.push(slot.clone());
        Ok(Lease {
            slot,
            previous: None,
            submitted: false,
            settled: false,
        })
    }
}

impl Drop for AntigravityPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Slot {
    session: Option<String>,
    binding: String,
    state: Mutex<Phase>,
    stop: watch::Sender<bool>,
    handle: tokio::runtime::Handle,
}

enum Phase {
    Active,
    Parked(Box<Parked>),
    Finished,
    Invalidated,
}

pub(super) struct Parked {
    pub run: acp::Run,
    pub prompt_id: u64,
    pub call: mcp::Call,
    pub tools: mcp::Tools,
    pub system: String,
    pub model: String,
}

impl Slot {
    fn pin(&self, parked: &Parked) -> Continuation {
        Continuation::Pinned {
            binding: self.binding.clone(),
            model: Some(parked.model.clone()),
        }
    }

    fn is_active(&self) -> bool {
        matches!(
            *self.state.lock().unwrap(),
            Phase::Active | Phase::Parked(_)
        )
    }

    fn invalidate(&self, only_call: Option<&str>) {
        let previous = {
            let mut state = self.state.lock().unwrap();
            match &*state {
                Phase::Active if only_call.is_none() => {}
                Phase::Parked(parked) if only_call.is_none_or(|id| id == parked.call.id) => {}
                _ => return,
            }
            self.stop.send_replace(true);
            std::mem::replace(&mut *state, Phase::Invalidated)
        };
        if let Phase::Parked(parked) = previous {
            self.handle.spawn(async move {
                let mut parked = parked;
                parked.run.cancel().await;
            });
        }
    }

    fn finish(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        if !matches!(*state, Phase::Active) {
            return false;
        }
        *state = Phase::Finished;
        true
    }
}

pub(super) struct Lease {
    slot: Arc<Slot>,
    pub previous: Option<(Parked, Value)>,
    pub submitted: bool,
    settled: bool,
}

impl Lease {
    pub fn stop(&self) -> watch::Receiver<bool> {
        self.slot.stop.subscribe()
    }

    pub fn park(&mut self, pending: Parked) -> Result<(), Error> {
        let mut ended = pending.run.ended.clone();
        let id = pending.call.id.clone();
        {
            let mut state = self.slot.state.lock().unwrap();
            if !matches!(*state, Phase::Active) {
                return Err(Error::Cancelled);
            }
            if *ended.borrow() || pending.call.reply.is_closed() {
                return Err(Error::Transport);
            }
            *state = Phase::Parked(Box::new(pending));
        }
        let slot = Arc::downgrade(&self.slot);
        let mut cancel = self.slot.stop.subscribe();
        self.slot.handle.spawn(async move {
            tokio::select! {
                _ = ended.wait_for(|closed| *closed) => {}
                _ = tokio::time::sleep(TOOL_TIMEOUT) => {}
                _ = cancel.wait_for(|cancelled| *cancelled) => return,
            }
            if let Some(slot) = slot.upgrade() {
                slot.invalidate(Some(&id));
            }
        });
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), Error> {
        if !self.slot.finish() {
            return Err(Error::Cancelled);
        }
        self.settled = true;
        Ok(())
    }

    pub fn keep_parked(&mut self) {
        self.settled = true;
    }

    pub fn failure(&self, kind: Error) -> NativeFailure {
        if self.submitted {
            NativeFailure::sent(kind, None)
        } else {
            NativeFailure::unsent(kind)
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if !self.settled {
            if self.submitted {
                self.slot.invalidate(None);
            } else {
                self.slot.finish();
            }
        }
    }
}
