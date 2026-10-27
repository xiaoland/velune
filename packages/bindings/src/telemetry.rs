//! Installation-local settings; never routed through the selected application backend.
use crate::{BindingError, BindingFailureKind, convert};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, sync::Arc};

#[derive(Clone, uniffi::Record)]
pub struct BindingTelemetryOptions {
    pub home_directory: String,
    pub platform: String,
    pub version: String,
    pub development_profile: Option<String>,
    pub run_id: Option<String>,
    pub local_logs: bool,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingTelemetryPreference {
    Unset,
    Off,
    On,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTelemetryProfile {
    pub endpoint: String,
    pub headers: HashMap<String, String>,
    pub sample_ratio: f64,
}
#[derive(Clone, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTelemetrySnapshot {
    pub revision: u64,
    pub preference: BindingTelemetryPreference,
    pub endpoint: Option<String>,
    pub has_authentication: bool,
    pub sample_ratio: f64,
    pub development_override: bool,
    pub rust_active_revision: Option<u64>,
    pub native_active_revision: Option<u64>,
    pub native_applied_revision: Option<u64>,
    pub applied: bool,
    pub status: String,
    pub instance_id: String,
    pub updated_unix_ms: u64,
    pub process_id: u32,
    pub run_id: Option<String>,
    pub execution: Option<BindingTelemetryExecution>,
    pub status_write_failed: bool,
    pub configuration_available: bool,
    pub rust_export: BindingTelemetryExportHealth,
    pub native_export: BindingTelemetryExportHealth,
}
#[derive(Clone, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTelemetryExecution {
    pub active_turns: u64,
    pub observed_unix_ms: u64,
    pub owner_available: bool,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTelemetryExportHealth {
    pub completed_batches: u64,
    pub failed_batches: u64,
    pub last_export_unix_ms: Option<u64>,
    pub last_transport_result: Option<String>,
    pub dropped_records: Option<u64>,
}
#[derive(Clone, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTelemetryIdentity {
    pub service_name: String,
    pub version: String,
    pub platform: String,
    pub environment: String,
    pub instance_id: String,
    pub run_id: Option<String>,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingTelemetryPreparation {
    pub id: String,
    pub revision: u64,
    pub profile: Option<BindingTelemetryProfile>,
    pub identity: BindingTelemetryIdentity,
}
#[derive(Clone, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTelemetryTestResult {
    pub traces: String,
    pub logs: String,
    pub metrics: String,
}

#[derive(Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTraceCarrier {
    pub traceparent: String,
    pub tracestate: String,
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
    #[uniffi::constructor]
    pub fn open(options: BindingTelemetryOptions) -> Result<Arc<Self>, BindingError> {
        let mut identity =
            velune_telemetry::ResourceIdentity::new(&options.platform, &options.version)?;
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
    pub fn snapshot(&self) -> Result<BindingTelemetrySnapshot, BindingError> {
        convert(self.owner.snapshot()?)
    }
    /// Returns credentials only for the local editor/SDK or an independently authorized sharing RPC.
    pub fn profile_for_editing(&self) -> Result<Option<BindingTelemetryProfile>, BindingError> {
        convert(self.owner.saved_profile()?)
    }
    pub fn prepare_saved(&self) -> Result<BindingTelemetryPreparation, BindingError> {
        preparation(self.owner.prepare_saved()?)
    }
    pub fn prepare(
        &self,
        expected_revision: u64,
        preference: BindingTelemetryPreference,
        profile: Option<BindingTelemetryProfile>,
    ) -> Result<BindingTelemetryPreparation, BindingError> {
        preparation(self.owner.prepare(
            expected_revision,
            convert(preference)?,
            convert(profile)?,
        )?)
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
        profile: BindingTelemetryProfile,
    ) -> Result<BindingTelemetryTestResult, BindingError> {
        convert(self.owner.send_test(&convert(profile)?)?)
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
        health: BindingTelemetryExportHealth,
    ) -> Result<(), BindingError> {
        Ok(self
            .owner
            .report_native_export(revision, convert(health)?)?)
    }
    pub fn shutdown(&self) -> Result<(), BindingError> {
        Ok(self.owner.shutdown()?)
    }
}
fn preparation(
    p: velune_telemetry::Preparation,
) -> Result<BindingTelemetryPreparation, BindingError> {
    Ok(BindingTelemetryPreparation {
        id: p.id,
        revision: p.revision,
        profile: convert(p.profile)?,
        identity: convert(p.identity)?,
    })
}
