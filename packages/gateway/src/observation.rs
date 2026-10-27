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
    telemetry: Option<velune_telemetry::Operation>,
    telemetry_owner: Option<Arc<velune_telemetry::ClientTelemetry>>,
}
impl Observation {
    pub(crate) fn new(event: &'static str, span: Span, stopping: Arc<AtomicBool>) -> Self {
        let dispatcher = span
            .with_subscriber(|(_, dispatch)| dispatch.clone())
            .unwrap_or_else(|| tracing::dispatcher::get_default(Clone::clone));
        Self {
            event,
            span,
            dispatcher,
            stopping,
            started: Instant::now(),
            outcome: None,
            telemetry: None,
            telemetry_owner: None,
        }
    }
    pub(crate) fn finish(&mut self, outcome: &'static str) {
        self.outcome = Some(outcome);
    }
    pub(crate) fn with_telemetry(
        mut self,
        operation: Option<velune_telemetry::Operation>,
        owner: Option<Arc<velune_telemetry::ClientTelemetry>>,
    ) -> Self {
        if let Some(operation) = &operation {
            self.span.record("operation_id", &operation.operation_id);
        }
        self.telemetry = operation;
        self.telemetry_owner = owner;
        self
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
        if let Some(operation) = &mut self.telemetry {
            let result = match outcome {
                "transport_completed" | "completed" => velune_telemetry::Outcome::Success,
                "downstream_closed" | "cancelled" | "gateway_stopped" => {
                    velune_telemetry::Outcome::Cancelled
                }
                _ => velune_telemetry::Outcome::Failure,
            };
            if let Some(owner) = &self.telemetry_owner
                && owner.active_revision() != operation.revision()
            {
                let name = if self.event == "gateway_request_finished" {
                    "gateway.request.result"
                } else {
                    "gateway.attempt.result"
                };
                let mut current = owner.operation(name, operation.carrier().as_ref(), false);
                current.retain_identity_from(operation);
                current.correlation_gap(velune_telemetry::CorrelationGap::RetiredGeneration);
                current.complete(result);
            }
            operation.complete(result);
        }
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
