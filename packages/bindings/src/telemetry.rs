//! Installation-local settings; never routed through the selected application backend.
use crate::{BindingError, BindingFailureKind};
use std::{path::Path, sync::Arc};
use velune_telemetry::{
    TelemetryExportHealth, TelemetryPreference, TelemetryPreparation, TelemetryProfile,
    TelemetrySnapshot, TelemetryTestResult,
};

#[derive(Clone, uniffi::Record)]
pub struct BindingTelemetryOptions {
    pub home_directory: String,
    pub platform: String,
    pub version: String,
    pub development_profile: Option<String>,
    pub run_id: Option<String>,
    pub local_logs: bool,
}
impl From<velune_telemetry::Error> for BindingError {
    fn from(error: velune_telemetry::Error) -> Self {
        if matches!(error, velune_telemetry::Error::Closed) {
            return BindingError::Closed;
        }
        if matches!(error, velune_telemetry::Error::Conflict) {
            return BindingError::Diagnostic {
                kind: BindingFailureKind::Conflict,
                detail: error.to_string(),
                code: "telemetry_revision_conflict".into(),
                phase: "telemetry_config".into(),
                operation_id: String::new(),
            };
        }
        BindingError::Invalid {
            detail: error.to_string(),
        }
    }
}

#[derive(uniffi::Object)]
pub struct VeluneTelemetry {
    pub(crate) owner: Arc<velune_telemetry::ClientTelemetry>,
}
#[uniffi::export]
impl VeluneTelemetry {
    pub fn problems(
        &self,
        scope: String,
    ) -> Result<Vec<velune_telemetry::DiagnosticProblem>, BindingError> {
        Ok(self.owner.problems(&scope)?)
    }
    pub fn record_problem(
        &self,
        scope: String,
        problem: velune_telemetry::DiagnosticProblem,
    ) -> Result<(), BindingError> {
        Ok(self.owner.record_problem(&scope, problem)?)
    }
    pub fn resolve_problems(&self, scope: String, ids: Vec<String>) -> Result<(), BindingError> {
        Ok(self.owner.resolve_problems(&scope, ids)?)
    }
    pub fn dismiss_problems(&self, scope: String, ids: Vec<String>) -> Result<(), BindingError> {
        Ok(self.owner.dismiss_problems(&scope, ids)?)
    }
    #[uniffi::constructor]
    pub fn open(options: BindingTelemetryOptions) -> Result<Arc<Self>, BindingError> {
        let mut identity =
            velune_telemetry::TelemetryIdentity::new(&options.platform, &options.version)?;
        if options.run_id.as_ref().is_some_and(|id| {
            id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        }) {
            return Err(BindingError::Invalid {
                detail: "开发运行编号无效".into(),
            });
        }
        identity.run_id = options.run_id;
        let owner = velune_telemetry::ClientTelemetry::open(
            Path::new(&options.home_directory),
            identity,
            options.local_logs,
            options.development_profile.as_deref().map(Path::new),
        )?;
        Ok(Arc::new(Self { owner }))
    }
    pub fn snapshot(&self) -> Result<TelemetrySnapshot, BindingError> {
        Ok(self.owner.snapshot()?)
    }
    /// Returns credentials only for the local editor/SDK or an independently authorized sharing RPC.
    pub fn profile_for_editing(&self) -> Result<Option<TelemetryProfile>, BindingError> {
        Ok(self.owner.saved_profile()?)
    }
    pub fn prepare_saved(&self) -> Result<TelemetryPreparation, BindingError> {
        Ok(self.owner.prepare_saved()?)
    }
    pub fn prepare(
        &self,
        expected_revision: u64,
        preference: TelemetryPreference,
        profile: Option<TelemetryProfile>,
    ) -> Result<TelemetryPreparation, BindingError> {
        Ok(self.owner.prepare(expected_revision, preference, profile)?)
    }
    pub fn commit(&self, preparation_id: String) -> Result<u64, BindingError> {
        Ok(self.owner.commit(&preparation_id)?)
    }
    pub fn confirm_native(&self, revision: u64, active: bool) -> Result<(), BindingError> {
        Ok(self.owner.confirm_native(revision, active)?)
    }
    pub fn cancel_preparation(&self, preparation_id: String) -> Result<(), BindingError> {
        Ok(self.owner.cancel_preparation(&preparation_id)?)
    }
    pub fn send_test(
        &self,
        profile: TelemetryProfile,
    ) -> Result<TelemetryTestResult, BindingError> {
        Ok(self.owner.send_test(&profile)?)
    }
    pub fn flush(&self) -> bool {
        self.owner.flush()
    }
    pub fn report_native_drops(
        &self,
        revision: u64,
        signal: String,
        count: u64,
    ) -> Result<(), BindingError> {
        Ok(self.owner.report_native_drops(revision, &signal, count)?)
    }
    pub fn report_native_export(
        &self,
        revision: u64,
        health: TelemetryExportHealth,
    ) -> Result<(), BindingError> {
        Ok(self.owner.report_native_export(revision, health)?)
    }
    pub fn shutdown(&self) -> Result<(), BindingError> {
        Ok(self.owner.shutdown()?)
    }
}
