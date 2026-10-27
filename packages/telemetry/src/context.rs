use crate::{Error, Result};
use opentelemetry::{
    Context,
    propagation::TextMapPropagator,
    trace::{SpanContext, TraceContextExt},
};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use serde::{Deserialize, Serialize};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Only W3C trace context crosses a boundary. Baggage and arbitrary headers never do.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TraceCarrier {
    pub traceparent: String,
    pub tracestate: String,
}
impl TraceCarrier {
    pub fn validate(&self) -> Result<()> {
        if self.traceparent.len() != 55
            || self.tracestate.len() > 512
            || self
                .traceparent
                .bytes()
                .any(|b| !b.is_ascii_hexdigit() && b != b'-')
            || self.tracestate.bytes().any(|b| !(32..=126).contains(&b))
        {
            return Err(Error::InvalidContext);
        }
        if !self.extract(true).span().span_context().is_valid() {
            return Err(Error::InvalidContext);
        }
        Ok(())
    }
    pub fn current() -> Option<Self> {
        Self::from_span(&tracing::Span::current())
    }
    pub(crate) fn from_span(span: &tracing::Span) -> Option<Self> {
        let context = span.context();
        if !context.span().span_context().is_valid() {
            return None;
        }
        let mut headers = std::collections::HashMap::new();
        TraceContextPropagator::new().inject_context(&context, &mut headers);
        Some(Self {
            traceparent: headers.remove("traceparent").unwrap_or_default(),
            tracestate: headers.remove("tracestate").unwrap_or_default(),
        })
    }
    pub(crate) fn extract(&self, remote: bool) -> Context {
        let headers = std::collections::HashMap::from([
            ("traceparent".to_owned(), self.traceparent.clone()),
            ("tracestate".to_owned(), self.tracestate.clone()),
        ]);
        let context = TraceContextPropagator::new().extract(&headers);
        if remote {
            return context;
        }
        let span = context.span();
        let sc = span.span_context();
        Context::new().with_remote_span_context(SpanContext::new(
            sc.trace_id(),
            sc.span_id(),
            sc.trace_flags(),
            false,
            sc.trace_state().clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_is_bounded_and_ffi_is_local() {
        let carrier = TraceCarrier {
            traceparent: "00-12345678901234567890123456789012-1234567890123456-01".into(),
            tracestate: String::new(),
        };
        assert!(carrier.validate().is_ok());
        assert!(!carrier.extract(false).span().span_context().is_remote());
        assert!(carrier.extract(true).span().span_context().is_remote());
        let mut invalid = carrier;
        invalid.tracestate = "a".repeat(513);
        assert!(invalid.validate().is_err());
    }
}
