//! Metadata-only gateway lifecycle observations; never inspect protocol payloads.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tracing::{Dispatch, Span};

pub(crate) struct Observation {
    event: &'static str,
    span: Span,
    dispatcher: Dispatch,
    stopping: Arc<AtomicBool>,
    started: Instant,
    outcome: Option<&'static str>,
}
impl Observation {
    pub(crate) fn new(event: &'static str, span: Span, stopping: Arc<AtomicBool>) -> Self {
        Self {
            event,
            span,
            dispatcher: tracing::dispatcher::get_default(Clone::clone),
            stopping,
            started: Instant::now(),
            outcome: None,
        }
    }
    pub(crate) fn finish(&mut self, outcome: &'static str) {
        self.outcome = Some(outcome);
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        let outcome = self.outcome.unwrap_or_else(|| {
            if self.stopping.load(Ordering::Acquire) {
                "gateway_stopped"
            } else {
                "downstream_closed"
            }
        });
        // Response bodies and aborted futures may drop outside the application
        // dispatcher. Keep both the sink and request correlation through teardown.
        tracing::dispatcher::with_default(&self.dispatcher, || {
            self.span.in_scope(|| {
                tracing::info!(
                    event = self.event,
                    outcome,
                    elapsed_ms = self.started.elapsed().as_millis() as u64
                );
            });
        });
    }
}
