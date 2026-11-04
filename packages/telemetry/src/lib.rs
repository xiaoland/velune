//! Client-local OTLP configuration and revocable SDK generations. No process-global SDK is installed.
mod config;
mod context;
mod health;
mod probe;
mod problems;
pub use problems::DiagnosticProblem;
mod sdk;
mod sdk_diagnostics;
mod transport;
#[cfg(windows)]
mod windows_private;
#[cfg(test)]
mod wire_tests;

use config::Stored;
pub use config::{TelemetryPreference, TelemetryProfile};
pub use context::TraceCarrier;
pub use health::TelemetryExportHealth;
pub use opentelemetry::trace::SpanKind;
use opentelemetry::{KeyValue, trace::TraceContextExt};
pub use probe::TelemetryTestResult;

uniffi::setup_scaffolding!();

/// Ordered so the saved profile stays stable; foreign languages see a dictionary.
pub type HeaderMap = std::collections::BTreeMap<String, String>;
uniffi::custom_type!(HeaderMap, std::collections::HashMap<String, String>, {
    remote,
    lower: |map| map.into_iter().collect(),
    try_lift: |map| Ok(map.into_iter().collect()),
});
use sdk::{Generation, LocalWriter};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("遥测地址必须是无认证、路径或查询参数的 HTTP(S) gRPC 地址")]
    InvalidEndpoint,
    #[error("遥测采样率必须在 0 到 1 之间")]
    InvalidSampling,
    #[error("遥测认证 metadata 格式或大小无效")]
    InvalidHeaders,
    #[error("遥测配置过大")]
    TooLarge,
    #[error("启用遥测需要接收端配置")]
    MissingProfile,
    #[error("遥测配置格式不受支持；原文件已保留")]
    StorageFormat,
    #[error("遥测配置已改变，请重新读取后保存")]
    Conflict,
    #[error("遥测候选配置不存在或已过期")]
    NoPreparation,
    #[error("无法准备遥测传输")]
    Transport,
    #[error("遥测关联上下文无效")]
    InvalidContext,
    #[error("无法创建遥测随机标识")]
    Random,
    #[error("遥测状态不可用")]
    Unavailable,
    #[error("遥测客户端已关闭")]
    Closed,
    #[error("遥测存储失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("遥测文件无法解析")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TelemetryIdentity {
    pub service_name: String,
    pub version: String,
    pub platform: String,
    pub environment: String,
    pub instance_id: String,
    pub run_id: Option<String>,
}
impl TelemetryIdentity {
    pub fn new(platform: &str, version: &str) -> Result<Self> {
        if !matches!(
            platform,
            "macos" | "windows" | "ios" | "android" | "host" | "test"
        ) || version.len() > 128
        {
            return Err(Error::StorageFormat);
        }
        Ok(Self {
            service_name: format!("velune.{platform}"),
            version: version.into(),
            platform: platform.into(),
            environment: "production".into(),
            instance_id: config::random_id()?,
            run_id: None,
        })
    }
}

/// Contains no credentials. An endpoint is validated to contain no URL credentials or query.
#[derive(Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySnapshot {
    pub revision: u64,
    pub preference: TelemetryPreference,
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
    pub execution: Option<TelemetryExecution>,
    pub status_write_failed: bool,
    pub configuration_available: bool,
    pub rust_export: TelemetryExportHealth,
    pub native_export: TelemetryExportHealth,
}

#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryExecution {
    pub active_turns: u64,
    pub observed_unix_ms: u64,
    pub owner_available: bool,
}
pub(crate) type ExecutionState = Arc<Mutex<Option<TelemetryExecution>>>;

struct Candidate {
    id: String,
    stored: Stored,
    generation: Option<Arc<Generation>>,
    created: Instant,
}
struct State {
    stored: Stored,
    active: Option<Arc<Generation>>,
    candidate: Option<Candidate>,
    native_revision: Option<u64>,
    status: &'static str,
    native_export: TelemetryExportHealth,
}
/// Retain one owner for an installation/process, independently of windows and remote selections.
pub struct ClientTelemetry {
    directory: PathBuf,
    identity: TelemetryIdentity,
    development: Option<TelemetryProfile>,
    development_unavailable: bool,
    state: Mutex<State>,
    writer: Option<LocalWriter>,
    local_dispatch: tracing::Dispatch,
    runtime: RuntimeOwner,
    status_write_failed: std::sync::atomic::AtomicBool,
    status_writer: Mutex<()>,
    file_lock: Mutex<Option<std::fs::File>>,
    configuration_available: bool,
    current: Mutex<Option<Arc<Generation>>>,
    shared_profile: Mutex<Option<TelemetryProfile>>,
    sdk_diagnostics: Arc<sdk_diagnostics::Diagnostics>,
    execution: ExecutionState,
}

/// The only credential-bearing output to native SDK assembly and authorized sharing.
#[derive(uniffi::Record)]
pub struct TelemetryPreparation {
    pub id: String,
    pub revision: u64,
    pub profile: Option<TelemetryProfile>,
    pub identity: TelemetryIdentity,
}

impl ClientTelemetry {
    pub fn open(
        home: &Path,
        mut identity: TelemetryIdentity,
        local_logs: bool,
        development_profile: Option<&Path>,
    ) -> Result<Arc<Self>> {
        if !home.is_absolute() {
            return Err(Error::StorageFormat);
        }
        let directory = home.join("telemetry");
        let (stored, configuration_available) =
            match config::read::<Stored>(&directory.join("config.json")) {
                Ok(stored) if stored.validate().is_ok() => (stored, true),
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    (Stored::empty()?, true)
                }
                _ => (Stored::empty()?, false),
            };
        let (development, development_unavailable) = match development_profile
            .map(config::read::<TelemetryProfile>)
            .transpose()
        {
            Ok(profile)
                if profile
                    .as_ref()
                    .is_none_or(|profile| profile.validate().is_ok()) =>
            {
                (profile, false)
            }
            _ => (None, true),
        };
        if development.is_some() {
            identity.environment = "development".into();
        }
        let writer = if local_logs {
            let logs = home.join("logs");
            std::fs::create_dir_all(&logs)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o700))?;
            }
            Some(LocalWriter(Arc::new(Mutex::new(
                tracing_appender::rolling::RollingFileAppender::builder()
                    .rotation(tracing_appender::rolling::Rotation::DAILY)
                    .filename_prefix("velune")
                    .filename_suffix("jsonl")
                    .max_log_files(7)
                    .build(logs)
                    .map_err(|_| Error::Unavailable)?,
            ))))
        } else {
            None
        };
        let local_dispatch = sdk::local_dispatch(writer.clone());
        let shared_profile = Mutex::new(stored.profile.clone());
        let sdk_diagnostics = sdk_diagnostics::Diagnostics::new(
            directory.join("sdk-diagnostics.json"),
            identity.instance_id.clone(),
        );
        let owner = Arc::new(Self {
            directory,
            identity,
            development,
            development_unavailable,
            state: Mutex::new(State {
                stored,
                active: None,
                candidate: None,
                native_revision: None,
                status: if configuration_available {
                    "inactive"
                } else {
                    "configuration_unavailable"
                },
                native_export: TelemetryExportHealth::default(),
            }),
            writer,
            local_dispatch,
            runtime: RuntimeOwner(Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .thread_name("velune-telemetry")
                    .build()?,
            )),
            status_write_failed: std::sync::atomic::AtomicBool::new(false),
            status_writer: Mutex::new(()),
            file_lock: Mutex::new(None),
            configuration_available,
            current: Mutex::new(None),
            shared_profile,
            sdk_diagnostics,
            execution: Arc::new(Mutex::new(None)),
        });
        let weak = Arc::downgrade(&owner);
        owner.runtime.spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(15));
            ticks.tick().await;
            loop {
                ticks.tick().await;
                let Some(owner) = weak.upgrade() else { break };
                if owner.lock().is_ok_and(|state| state.status == "stopped") {
                    break;
                }
                owner.write_status();
            }
        });
        Ok(owner)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| Error::Unavailable)
    }
    fn effective(&self, stored: &Stored) -> Option<TelemetryProfile> {
        match stored.preference {
            TelemetryPreference::Off => None,
            TelemetryPreference::On => stored.profile.clone(),
            TelemetryPreference::Unset => self.development.clone(),
        }
    }
    pub fn snapshot(&self) -> Result<TelemetrySnapshot> {
        let state = self.lock()?;
        let profile = self
            .effective(&state.stored)
            .or_else(|| state.stored.profile.clone());
        let rust_revision = state
            .active
            .as_ref()
            .filter(|_| state.native_revision == Some(state.stored.revision))
            .map(|g| g.revision);
        Ok(TelemetrySnapshot {
            revision: state.stored.revision,
            preference: state.stored.preference,
            endpoint: profile.as_ref().map(|p| p.endpoint.clone()),
            has_authentication: profile.as_ref().is_some_and(|p| !p.headers.is_empty()),
            sample_ratio: profile.as_ref().map_or(1.0, |p| p.sample_ratio),
            development_override: state.stored.preference == TelemetryPreference::Unset
                && self.development.is_some(),
            rust_active_revision: rust_revision,
            native_active_revision: state.active.as_ref().and(state.native_revision),
            native_applied_revision: state.native_revision,
            applied: self.configuration_available
                && !(self.development_unavailable
                    && state.stored.preference == TelemetryPreference::Unset)
                && ((state.stored.revision == 0 && self.effective(&state.stored).is_none())
                    || state.native_revision == Some(state.stored.revision)),
            status: if self.development_unavailable
                && state.stored.preference == TelemetryPreference::Unset
            {
                "development_configuration_unavailable".into()
            } else {
                state.status.into()
            },
            instance_id: self.identity.instance_id.clone(),
            updated_unix_ms: now_ms(),
            process_id: std::process::id(),
            run_id: self.identity.run_id.clone(),
            execution: self
                .execution
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            status_write_failed: self
                .status_write_failed
                .load(std::sync::atomic::Ordering::Relaxed),
            configuration_available: self.configuration_available,
            rust_export: state
                .active
                .as_ref()
                .map(|g| g.health.lock().unwrap_or_else(|e| e.into_inner()).clone())
                .unwrap_or_default(),
            native_export: state.native_export.clone(),
        })
    }
    /// Reading saved profile is explicit because this material includes authentication.
    pub fn saved_profile(&self) -> Result<Option<TelemetryProfile>> {
        if !self.configuration_available {
            return Err(Error::StorageFormat);
        }
        Ok(self
            .shared_profile
            .lock()
            .map_err(|_| Error::Unavailable)?
            .clone())
    }
    /// Explicit, bounded one-shot export. It does not enable the SDK or change saved preferences.
    pub fn send_test(&self, profile: &TelemetryProfile) -> Result<TelemetryTestResult> {
        if self.lock()?.status == "stopped" {
            return Err(Error::Closed);
        }
        self.runtime.block_on(probe::send(profile, &self.identity))
    }
    pub fn prepare_saved(&self) -> Result<TelemetryPreparation> {
        let stored = self.lock()?.stored.clone();
        self.prepare(stored.revision, stored.preference, stored.profile)
    }
    pub fn prepare(
        &self,
        expected_revision: u64,
        preference: TelemetryPreference,
        profile: Option<TelemetryProfile>,
    ) -> Result<TelemetryPreparation> {
        if let Some(profile) = &profile {
            profile.validate()?;
        }
        if !self.configuration_available {
            return Err(Error::StorageFormat);
        }
        self.acquire_file_lock()?;
        let mut state = self.lock()?;
        if state.status == "stopped" {
            return Err(Error::Closed);
        }
        if state.stored.revision != expected_revision {
            return Err(Error::Conflict);
        }
        match config::read::<Stored>(&self.directory.join("config.json")) {
            Ok(stored) if stored.revision != state.stored.revision => return Err(Error::Conflict),
            Ok(stored) => stored.validate()?,
            Err(Error::Io(e))
                if e.kind() == std::io::ErrorKind::NotFound && state.stored.revision == 0 => {}
            Err(e) => return Err(e),
        }
        let mut stored = state.stored.clone();
        stored.revision = stored.revision.checked_add(1).ok_or(Error::Conflict)?;
        stored.preference = preference;
        stored.profile = profile;
        stored.validate()?;
        let effective = self.effective(&stored);
        let _entered = self.runtime.enter();
        let generation = effective
            .as_ref()
            .map(|profile| {
                Generation::prepare(
                    profile,
                    &self.identity,
                    stored.revision,
                    self.writer.clone(),
                    self.sdk_diagnostics.clone(),
                    self.execution.clone(),
                )
                .map(Arc::new)
            })
            .transpose()?;
        let id = config::random_id()?;
        let old = state.candidate.replace(Candidate {
            id: id.clone(),
            stored: stored.clone(),
            generation,
            created: Instant::now(),
        });
        drop(state);
        if let Some(old) = old {
            self.cleanup(old.generation);
        }
        Ok(TelemetryPreparation {
            id,
            revision: stored.revision,
            profile: effective,
            identity: self.identity.clone(),
        })
    }
    /// Persist first. On success native revokes its old transport, activates, then confirms.
    pub fn commit(&self, id: &str) -> Result<u64> {
        let mut state = self.lock()?;
        let candidate = state
            .candidate
            .as_ref()
            .filter(|c| c.id == id && c.created.elapsed() < Duration::from_secs(120))
            .ok_or(Error::NoPreparation)?;
        config::write(&self.directory.join("config.json"), &candidate.stored)?;
        let candidate = state.candidate.take().unwrap();
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let old = state.active.take();
        if let Some(old) = &old {
            old.revoke();
        }
        state.stored = candidate.stored;
        *self
            .shared_profile
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = state.stored.profile.clone();
        state.active = candidate.generation;
        state.native_revision = None;
        state.native_export = TelemetryExportHealth::default();
        state.status = "awaiting_native";
        let revision = state.stored.revision;
        drop(state);
        self.cleanup(old);
        self.write_status();
        Ok(revision)
    }
    pub fn confirm_native(&self, revision: u64, active: bool) -> Result<()> {
        let mut state = self.lock()?;
        if state.status == "stopped" {
            return Err(Error::Closed);
        }
        if state.stored.revision != revision {
            return Err(Error::Conflict);
        }
        let newly_active =
            active && state.active.is_some() && state.native_revision != Some(revision);
        if !active {
            self.current
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            let old = state.active.take();
            if let Some(old) = &old {
                old.revoke();
            }
            state.native_revision = None;
            state.status = "activation_failed";
            drop(state);
            self.cleanup(old);
        } else {
            if self.effective(&state.stored).is_some() && state.active.is_none() {
                return Err(Error::NoPreparation);
            }
            if let Some(generation) = &state.active {
                generation.activate();
            }
            *self.current.lock().unwrap_or_else(|e| e.into_inner()) = state.active.clone();
            state.native_revision = Some(revision);
            state.status = if state.active.is_some() {
                "active"
            } else {
                "disabled"
            };
            drop(state);
        }
        self.write_status();
        if newly_active {
            let mut observation = self.operation("telemetry.activated", None, false);
            observation.complete(Outcome::Success);
        }
        Ok(())
    }
    pub fn cancel_preparation(&self, id: &str) -> Result<()> {
        let mut state = self.lock()?;
        if state.candidate.as_ref().is_some_and(|c| c.id == id) {
            let candidate = state.candidate.take().unwrap();
            drop(state);
            self.cleanup(candidate.generation);
        }
        Ok(())
    }
    /// Fixed local SDK diagnostics; a retired revision never changes the active generation's health.
    pub fn report_native_drops(&self, revision: u64, signal: &str, count: u64) -> Result<()> {
        if revision == 0 || revision > self.lock()?.stored.revision {
            return Err(Error::Conflict);
        }
        let signal = match signal {
            "traces" => "traces",
            "logs" => "logs",
            _ => return Err(Error::StorageFormat),
        };
        self.sdk_diagnostics
            .record(revision, "native", signal, count)
    }
    pub fn report_native_export(&self, revision: u64, health: TelemetryExportHealth) -> Result<()> {
        if health
            .last_transport_result
            .as_deref()
            .is_some_and(|value| !matches!(value, "rpc_completed" | "transport_error"))
        {
            return Err(Error::StorageFormat);
        }
        let mut state = self.lock()?;
        if state.stored.revision != revision {
            return Ok(());
        }
        if health.completed_batches < state.native_export.completed_batches
            || health.failed_batches < state.native_export.failed_batches
        {
            return Ok(());
        }
        let changed = state.native_export.last_transport_result != health.last_transport_result;
        state.native_export = health;
        drop(state);
        if changed {
            self.write_status();
        }
        Ok(())
    }
    pub fn dispatch(&self) -> tracing::Dispatch {
        self.current
            .lock()
            .ok()
            .and_then(|current| current.as_ref().map(|g| g.dispatch.clone()))
            .unwrap_or_else(|| self.local_dispatch.clone())
    }
    /// The application owner publishes a complete count; SDK callbacks never call back into CoreRuntime.
    pub fn observe_execution(&self, active_turns: u64) {
        *self.execution.lock().unwrap_or_else(|e| e.into_inner()) = Some(TelemetryExecution {
            active_turns,
            observed_unix_ms: now_ms(),
            owner_available: true,
        });
    }
    pub fn execution_owner_unavailable(&self) {
        if let Some(snapshot) = self
            .execution
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            snapshot.owner_available = false;
        }
    }
    pub fn active_revision(&self) -> Option<u64> {
        self.current
            .lock()
            .ok()
            .and_then(|current| current.as_ref().map(|g| g.revision))
    }
    pub fn operation(
        &self,
        operation: &'static str,
        parent: Option<&TraceCarrier>,
        remote: bool,
    ) -> Operation {
        self.operation_with_kind(operation, SpanKind::Internal, parent, remote)
    }
    /// Kind describes this operation; `remote_parent` describes only the carrier's origin.
    pub fn operation_with_kind(
        &self,
        operation: &'static str,
        kind: SpanKind,
        parent: Option<&TraceCarrier>,
        remote_parent: bool,
    ) -> Operation {
        let remote = remote_parent;
        let kind = match kind {
            SpanKind::Client => "client",
            SpanKind::Server => "server",
            SpanKind::Producer => "producer",
            SpanKind::Consumer => "consumer",
            SpanKind::Internal => "internal",
        };
        let generation = self.current.lock().ok().and_then(|current| current.clone());
        let dispatch = generation
            .as_ref()
            .map(|g| g.dispatch.clone())
            .unwrap_or_else(|| self.local_dispatch.clone());
        let operation_id = config::random_id().unwrap_or_default();
        let span = tracing::dispatcher::with_default(
            &dispatch,
            // Registry parents cannot outlive their ambient dispatcher across SDK replacement.
            || {
                tracing::info_span!(target: "velune_telemetry", parent: None, "client.operation", otel.name = operation, otel.kind = kind,
                velune.operation = operation, velune.operation.id = operation_id,
                velune.outcome = tracing::field::Empty, velune.request.id = tracing::field::Empty,
                velune.correlation.gap = tracing::field::Empty, error.type = tracing::field::Empty,
                http.response.status_code = tracing::field::Empty, velune.gateway.transport = tracing::field::Empty)
            },
        );
        if let Some(parent) = parent.filter(|p| p.validate().is_ok()) {
            let _ = span.set_parent(parent.extract(remote));
        }
        // Resolve while this provider is alive; retired SDKs may no longer assign a context.
        let carrier = TraceCarrier::from_span(&span);
        let mut observation = Operation {
            span,
            dispatch,
            generation,
            operation,
            operation_id,
            started: Instant::now(),
            completed: false,
            measured: true,
            request_id: None,
            http_status: None,
            transport: None,
            failure_kind: None,
            started_event: false,
            gap: None,
            carrier,
        };
        if remote && parent.is_none() {
            observation.correlation_gap(CorrelationGap::MissingParent);
        }
        observation
    }
    pub fn flush(&self) -> bool {
        self.lock()
            .ok()
            .and_then(|s| s.active.clone())
            .is_none_or(|g| g.flush())
    }
    pub fn shutdown(&self) -> Result<()> {
        let mut state = self.lock()?;
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let active = state.active.take();
        let candidate = state.candidate.take();
        state.native_revision = None;
        state.status = "stopped";
        drop(state);
        if let Some(active) = active {
            active.flush();
            active.shutdown();
        }
        if let Some(candidate) = candidate {
            self.cleanup(candidate.generation);
        }
        self.write_status();
        Ok(())
    }
    fn cleanup(&self, generation: Option<Arc<Generation>>) {
        if let Some(generation) = generation {
            generation.revoke();
            std::thread::spawn(move || generation.shutdown());
        }
    }
    fn write_status(&self) {
        let _guard = self.status_writer.lock().unwrap_or_else(|e| e.into_inner());
        let result = self
            .snapshot()
            .and_then(|snapshot| config::write(&self.directory.join("status.json"), &snapshot));
        self.status_write_failed
            .store(result.is_err(), std::sync::atomic::Ordering::Relaxed);
    }
    fn acquire_file_lock(&self) -> Result<()> {
        let mut held = self.file_lock.lock().map_err(|_| Error::Unavailable)?;
        if held.is_some() {
            return Ok(());
        }
        config::private_directory(&self.directory)?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))?;
            options.mode(0o600);
        }
        let file = options.open(self.directory.join("config.lock"))?;
        file.try_lock().map_err(|_| Error::Conflict)?;
        *held = Some(file);
        Ok(())
    }
}

pub struct Operation {
    span: tracing::Span,
    dispatch: tracing::Dispatch,
    generation: Option<Arc<Generation>>,
    operation: &'static str,
    pub operation_id: String,
    started: Instant,
    completed: bool,
    measured: bool,
    request_id: Option<String>,
    http_status: Option<u16>,
    transport: Option<&'static str>,
    failure_kind: Option<String>,
    started_event: bool,
    gap: Option<CorrelationGap>,
    carrier: Option<TraceCarrier>,
}
impl Operation {
    pub fn run<T>(&self, body: impl FnOnce() -> T) -> T {
        tracing::dispatcher::with_default(&self.dispatch, || self.span.in_scope(body))
    }
    pub fn carrier(&self) -> Option<TraceCarrier> {
        self.carrier.clone()
    }
    pub fn revision(&self) -> Option<u64> {
        self.generation.as_ref().map(|g| g.revision)
    }
    pub fn dispatch(&self) -> tracing::Dispatch {
        self.dispatch.clone()
    }
    pub fn correlation_gap(&mut self, gap: CorrelationGap) {
        self.gap = Some(gap);
        self.span.record("velune.correlation.gap", gap.as_str());
    }
    /// Only the bounded diagnostic taxonomy crosses the telemetry boundary.
    pub fn failure(&mut self, kind: &str) {
        let kind = match kind {
            "invalid" | "unsupported" | "io" | "contract" | "closed" | "unavailable"
            | "provider_import" | "history" | "runtime_management" | "conflict"
            | "unauthorized" | "connection" | "timeout" => kind,
            code if code
                .parse::<u16>()
                .is_ok_and(|value| (400..=599).contains(&value)) =>
            {
                code
            }
            _ => "other",
        };
        self.failure_kind = Some(match kind.parse::<u16>() {
            Ok(status) => status.to_string(),
            Err(_) => kind.to_owned(),
        });
    }
    pub fn http_response(&mut self, status: u16) {
        if (100..=599).contains(&status) {
            self.http_status = Some(status);
            self.span
                .record("http.response.status_code", i64::from(status));
        }
    }
    pub fn gateway_transport(&mut self, transport: &'static str) {
        assert!(matches!(transport, "http" | "cli"));
        self.transport = Some(transport);
        self.span.record("velune.gateway.transport", transport);
    }
    /// A result report preserves work identity and duration, without counting new work.
    pub fn retain_identity_from(&mut self, original: &Self) {
        self.operation = original.operation;
        self.operation_id = original.operation_id.clone();
        self.started = original.started;
        self.started_event = original.started_event;
        self.request_id = original.request_id.clone();
        self.failure_kind = original.failure_kind.clone();
        self.measured = false;
        self.span.record("velune.operation", self.operation);
        self.span.record("velune.operation.id", &self.operation_id);
        if let Some(id) = &self.request_id {
            self.span.record("velune.request.id", id);
        }
        if let Some(status) = original.http_status {
            self.http_response(status);
        }
        if let Some(transport) = original.transport {
            self.gateway_transport(transport);
        }
    }
    /// Emitted only after the operation's actual work is accepted, before the first drain.
    pub fn execution_started(&mut self) {
        if self.started_event || self.completed {
            return;
        }
        self.started_event = true;
        self.run(|| {
            tracing::event!(name: "velune.execution.turn.started", target: "velune_telemetry", tracing::Level::INFO,
            velune.operation = self.operation, velune.operation.id = self.operation_id,
            velune.request.id = self.request_id.as_deref())
        });
    }
    pub fn execution_has_started(&self) -> bool {
        self.started_event
    }
    pub fn execution_finished(&mut self, outcome: Outcome) {
        self.finish(outcome, true);
    }
    pub fn request_identity(&mut self, incarnation: &str, client: &str, sequence: u64) {
        if [incarnation, client]
            .iter()
            .all(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
            && sequence > 0
        {
            let id = format!("{incarnation}/{client}/{sequence}");
            self.span.record("velune.request.id", &id);
            self.request_id = Some(id);
        }
    }
    pub fn instrument<F: std::future::Future>(
        &self,
        future: F,
    ) -> impl std::future::Future<Output = F::Output> + use<F> {
        use tracing::{Instrument, instrument::WithSubscriber};
        future
            .instrument(self.span.clone())
            .with_subscriber(self.dispatch.clone())
    }
    pub fn complete(&mut self, outcome: Outcome) {
        self.finish(outcome, false);
    }
    fn finish(&mut self, outcome: Outcome, execution: bool) {
        if self.completed {
            return;
        }
        self.completed = true;
        self.span.record("velune.outcome", outcome.as_str());
        if matches!(outcome, Outcome::Failure) {
            if self.failure_kind.is_none() {
                self.failure("other");
            }
            self.span.record("error.type", self.failure_kind.as_deref());
            self.span
                .set_status(opentelemetry::trace::Status::error(""));
        } else {
            self.failure_kind = None;
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        let quiet = matches!(
            self.operation,
            "bindings.snapshot"
                | "bindings.snapshot_update"
                | "bindings.authentication_poll"
                | "remote.query"
                | "remote.query_request"
        );
        if execution || !quiet || !matches!(outcome, Outcome::Success) {
            macro_rules! emit {
                ($name:literal, $level:expr) => {
                    tracing::event!(name: $name, target: "velune_telemetry", $level,
                        velune.operation = self.operation, velune.operation.id = self.operation_id,
                        velune.outcome = outcome.as_str(), error.type = self.failure_kind.as_deref(),
                        velune.correlation.gap = self.gap.map(|gap| gap.as_str()),
                        velune.request.id = self.request_id.as_deref(), velune.operation.duration = elapsed,
                        http.response.status_code = self.http_status.map(i64::from),
                        velune.gateway.transport = self.transport)
                };
            }
            self.run(|| {
                match (
                    execution,
                    self.started_event,
                    matches!(outcome, Outcome::Failure),
                ) {
                    (true, true, true) => {
                        emit!("velune.execution.turn.finished", tracing::Level::ERROR)
                    }
                    (true, true, false) => {
                        emit!("velune.execution.turn.finished", tracing::Level::INFO)
                    }
                    (true, false, true) => {
                        emit!("velune.execution.turn.not_started", tracing::Level::ERROR)
                    }
                    (true, false, false) => {
                        emit!("velune.execution.turn.not_started", tracing::Level::INFO)
                    }
                    (false, _, true) => emit!("velune.operation.completed", tracing::Level::ERROR),
                    (false, _, false) => emit!("velune.operation.completed", tracing::Level::INFO),
                }
            });
        }
        if self.measured
            && let Some(generation) = &self.generation
        {
            let mut labels = vec![
                KeyValue::new("velune.operation", self.operation),
                KeyValue::new("velune.outcome", outcome.as_str()),
            ];
            if let Some(kind) = &self.failure_kind {
                labels.push(KeyValue::new("error.type", kind.clone()));
            }
            generation.operations.add(1, &labels);
            generation.duration.record(elapsed, &labels);
        }
        // Local diagnostic readers may retain tracing handles. End the SDK span now,
        // independently of the registry's last-reference lifetime; SDK end is idempotent.
        self.span.context().span().end();
        self.span = tracing::Span::none();
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        if !self.completed {
            self.complete(Outcome::Unknown);
        }
    }
}
#[derive(Clone, Copy)]
pub enum Outcome {
    Success,
    Failure,
    Cancelled,
    Unknown,
}
#[derive(Clone, Copy)]
pub enum CorrelationGap {
    MissingParent,
    RetiredGeneration,
}
impl CorrelationGap {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingParent => "missing_parent",
            Self::RetiredGeneration => "retired_generation",
        }
    }
}
impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

struct RuntimeOwner(Option<tokio::runtime::Runtime>);
impl std::ops::Deref for RuntimeOwner {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}
impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}
impl Drop for ClientTelemetry {
    fn drop(&mut self) {
        self.current
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.take() {
            active.revoke();
            std::thread::spawn(move || active.shutdown());
        }
        if let Some(candidate) = state.candidate.take().and_then(|c| c.generation) {
            candidate.revoke();
            std::thread::spawn(move || candidate.shutdown());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open(home: &Path, dev: Option<&Path>) -> Arc<ClientTelemetry> {
        ClientTelemetry::open(
            home,
            TelemetryIdentity::new("test", "test").unwrap(),
            false,
            dev,
        )
        .unwrap()
    }
    fn profile() -> TelemetryProfile {
        TelemetryProfile {
            endpoint: "http://127.0.0.1:1".into(),
            headers: Default::default(),
            sample_ratio: 1.0,
        }
    }
    #[test]
    fn default_is_inert_off_wins_and_revisions_are_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open(dir.path(), None);
        assert!(!dir.path().join("telemetry").exists());
        assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
        let pending = owner
            .prepare(0, TelemetryPreference::On, Some(profile()))
            .unwrap();
        assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
        assert!(!dir.path().join("telemetry/config.json").exists());
        assert!(matches!(
            owner.prepare(9, TelemetryPreference::Off, None),
            Err(Error::Conflict)
        ));
        let rev = owner.commit(&pending.id).unwrap();
        assert!(!owner.snapshot().unwrap().applied);
        owner.confirm_native(rev, true).unwrap();
        assert!(owner.snapshot().unwrap().applied);
        let off = owner
            .prepare(rev, TelemetryPreference::Off, Some(profile()))
            .unwrap();
        owner.commit(&off.id).unwrap();
        assert!(!owner.snapshot().unwrap().applied);
        owner.confirm_native(off.revision, true).unwrap();
        assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
        let dev = dir.path().join("dev.json");
        config::write(&dev, &profile()).unwrap();
        drop(owner);
        let reopened = open(dir.path(), Some(&dev));
        assert!(reopened.prepare_saved().unwrap().profile.is_none());
    }
    #[test]
    fn failed_write_preserves_active_revision_and_profile() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open(dir.path(), None);
        let pending = owner
            .prepare(0, TelemetryPreference::On, Some(profile()))
            .unwrap();
        std::fs::create_dir_all(dir.path().join("telemetry/config.json")).unwrap();
        assert!(owner.commit(&pending.id).is_err());
        assert_eq!(owner.snapshot().unwrap().revision, 0);
        assert!(owner.saved_profile().unwrap().is_none());
        owner.cancel_preparation(&pending.id).unwrap();
    }
    #[test]
    fn status_failure_does_not_turn_a_committed_change_into_a_save_failure() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open(dir.path(), None);
        let pending = owner
            .prepare(0, TelemetryPreference::On, Some(profile()))
            .unwrap();
        std::fs::create_dir_all(dir.path().join("telemetry/status.json")).unwrap();
        let revision = owner.commit(&pending.id).unwrap();
        assert_eq!(owner.snapshot().unwrap().revision, revision);
        owner.confirm_native(revision, true).unwrap();
        assert!(owner.snapshot().unwrap().applied);
        assert!(owner.snapshot().unwrap().status_write_failed);
    }
    #[test]
    fn failed_activation_cannot_be_confirmed_and_second_owner_cannot_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let first = open(dir.path(), None);
        let second = open(dir.path(), None);
        let pending = first
            .prepare(0, TelemetryPreference::On, Some(profile()))
            .unwrap();
        first.commit(&pending.id).unwrap();
        first.confirm_native(pending.revision, false).unwrap();
        assert!(first.confirm_native(pending.revision, true).is_err());
        assert!(!first.snapshot().unwrap().applied);
        assert!(matches!(
            second.prepare(0, TelemetryPreference::Off, None),
            Err(Error::Conflict)
        ));
    }
    #[test]
    fn configuration_io_lock_does_not_block_business_observation_or_profile_read() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open(dir.path(), None);
        let candidate = owner
            .prepare(0, TelemetryPreference::On, Some(profile()))
            .unwrap();
        owner.commit(&candidate.id).unwrap();
        owner.confirm_native(candidate.revision, true).unwrap();
        let guard = owner.state.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let copy = owner.clone();
        let thread = std::thread::spawn(move || {
            let mut op = copy.operation("list", None, false);
            op.complete(Outcome::Success);
            let _ = copy.saved_profile().unwrap();
            tx.send(()).unwrap();
        });
        let result = rx.recv_timeout(Duration::from_millis(200));
        drop(guard);
        thread.join().unwrap();
        assert!(
            result.is_ok(),
            "application telemetry must not wait for configuration storage"
        );
    }
    #[test]
    fn invalid_development_profile_does_not_prevent_business_startup_or_explicit_off() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open(dir.path(), Some(&dir.path().join("missing-profile")));
        assert_eq!(
            owner.snapshot().unwrap().status,
            "development_configuration_unavailable"
        );
        assert!(!owner.snapshot().unwrap().applied);
        let mut operation = owner.operation("list", None, false);
        operation.complete(Outcome::Success);
        let off = owner.prepare(0, TelemetryPreference::Off, None).unwrap();
        owner.commit(&off.id).unwrap();
        owner.confirm_native(off.revision, true).unwrap();
        assert!(owner.snapshot().unwrap().applied);
        assert_eq!(owner.snapshot().unwrap().status, "disabled");
    }
    #[test]
    fn delayed_native_work_cannot_reactivate_a_shutdown_owner() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open(dir.path(), None);
        let pending = owner
            .prepare(0, TelemetryPreference::On, Some(profile()))
            .unwrap();
        owner.commit(&pending.id).unwrap();
        owner.confirm_native(pending.revision, true).unwrap();
        owner.shutdown().unwrap();
        assert!(matches!(owner.prepare_saved(), Err(Error::Closed)));
        assert!(matches!(
            owner.confirm_native(pending.revision, true),
            Err(Error::Closed)
        ));
        assert!(matches!(owner.send_test(&profile()), Err(Error::Closed)));
        assert_eq!(owner.snapshot().unwrap().status, "stopped");
        owner.shutdown().unwrap();
    }
    #[test]
    fn future_storage_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("telemetry/config.json");
        config::write(&path, &serde_json::json!({"version": 99})).unwrap();
        let before = std::fs::read(&path).unwrap();
        let owner = open_result(dir.path()).unwrap();
        assert!(!owner.snapshot().unwrap().configuration_available);
        assert!(owner.prepare(0, TelemetryPreference::Off, None).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    fn open_result(path: &Path) -> Result<Arc<ClientTelemetry>> {
        ClientTelemetry::open(path, TelemetryIdentity::new("test", "test")?, false, None)
    }
}
