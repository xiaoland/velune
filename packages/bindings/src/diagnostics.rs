//! Local diagnostics and explicitly approved telemetry share the client lifecycle.
use crate::{BindingError, BindingFailureKind};
#[cfg(feature = "local-runtime")]
use std::path::Path;
use std::{sync::Arc, time::Instant};

#[derive(Clone)]
pub(crate) struct Diagnostics {
    pub(crate) telemetry: Arc<velune_telemetry::ClientTelemetry>,
}
impl Diagnostics {
    #[cfg(any(feature = "local-runtime", feature = "remote"))]
    pub(crate) fn with_telemetry(telemetry: Arc<velune_telemetry::ClientTelemetry>) -> Self {
        Self { telemetry }
    }
    #[cfg(feature = "local-runtime")]
    pub(crate) fn open(home: &Path) -> Result<Self, BindingError> {
        let platform = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "host"
        };
        let identity =
            velune_telemetry::TelemetryIdentity::new(platform, env!("CARGO_PKG_VERSION"))?;
        Ok(Self::with_telemetry(
            velune_telemetry::ClientTelemetry::open(home, identity, true, None)?,
        ))
    }
    pub(crate) fn run<T>(
        &self,
        operation: &'static str,
        body: impl FnOnce() -> Result<T, BindingError>,
    ) -> Result<T, BindingError> {
        self.run_context(operation, None, body)
    }
    pub(crate) fn run_context<T>(
        &self,
        operation: &'static str,
        parent: Option<&velune_telemetry::TraceCarrier>,
        body: impl FnOnce() -> Result<T, BindingError>,
    ) -> Result<T, BindingError> {
        let mut observation = self.telemetry.operation(operation, parent, false);
        if operation == "bindings.remote.connect" && parent.is_none() {
            observation.correlation_gap(velune_telemetry::CorrelationGap::MissingParent);
        }
        let operation_id = observation.operation_id.clone();
        let result = observation.run(|| {
            let started = Instant::now();
            let result = body();
            match result {
                Ok(value) => Ok(value),
                Err(error) => {
                    #[cfg(feature = "remote")]
                    if matches!(error, BindingError::RemoteRequest { .. }) { return Err(error); }
                    let (kind, code, phase, detail) = error.diagnostic_parts();
                    tracing::error!(target: "velune_bindings", event = "operation_failed", code, phase, failure_kind = kind.as_str(), detail = %detail, elapsed_ms = started.elapsed().as_millis() as u64);
                    Err(BindingError::Diagnostic { kind, code, phase, detail, operation_id })
                }
            }
        });
        let outcome = match &result {
            Ok(_) => velune_telemetry::Outcome::Success,
            Err(error) => {
                let kind = match error {
                    BindingError::Diagnostic { kind, .. } => kind.as_str(),
                    #[cfg(feature = "remote")]
                    BindingError::RemoteRequest { kind, .. } => kind.as_str(),
                    _ => "other",
                };
                observation.failure(kind);
                if kind == "outcome_unknown" {
                    velune_telemetry::Outcome::Unknown
                } else {
                    velune_telemetry::Outcome::Failure
                }
            }
        };
        observation.complete(outcome);
        result
    }
}

impl BindingFailureKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
            Self::Io => "io",
            Self::Contract => "contract",
            Self::Closed => "closed",
            Self::Unavailable => "unavailable",
            Self::ProviderImport => "provider_import",
            Self::History => "history",
            Self::RuntimeManagement => "runtime_management",
            Self::Conflict => "conflict",
            Self::Unauthorized => "unauthorized",
            Self::Connection => "connection",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}
