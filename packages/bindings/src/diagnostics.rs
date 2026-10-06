//! Per-application subscriber assembly. Domain units emit events without owning sinks.
use crate::{BindingError, BindingFailureKind};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tracing::Dispatch;
use tracing_subscriber::{Layer, filter::filter_fn, layer::SubscriberExt};

pub(crate) struct Diagnostics {
    dispatch: Dispatch,
}

impl Diagnostics {
    pub(crate) fn open(home: &Path) -> Result<Self, BindingError> {
        let directory = home.join("logs");
        std::fs::create_dir_all(&directory).map_err(|error| BindingError::Io {
            detail: format!("无法创建本地诊断日志目录 {}：{error}", directory.display()),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| BindingError::Io {
                    detail: format!("无法设置诊断日志目录权限：{error}"),
                },
            )?;
        }
        let writer = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("velune")
            .filename_suffix("jsonl")
            .max_log_files(7)
            .build(directory)
            .map_err(|error| BindingError::Io {
                detail: format!("无法打开本地诊断日志：{error}"),
            })?;
        // Only Velune's explicitly selected fields enter this sink; dependency
        // spans are not subscribed by this application-owned layer.
        let layer = tracing_subscriber::fmt::layer()
            .json()
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(filter_fn(|metadata| {
                metadata.target().starts_with("velune_")
                    && *metadata.level() <= tracing::Level::INFO
            }));
        // Future exporters belong alongside this layer, not inside domain packages.
        let subscriber = tracing_subscriber::registry().with(layer);
        Ok(Self {
            dispatch: Dispatch::new(subscriber),
        })
    }

    pub(crate) fn run<T>(
        &self,
        operation: &'static str,
        body: impl FnOnce() -> Result<T, BindingError>,
    ) -> Result<T, BindingError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let operation_id = format!(
            "{epoch:x}-{:x}-{:x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        tracing::dispatcher::with_default(&self.dispatch, || {
            let span = tracing::info_span!("application_operation", operation, operation_id);
            let _entered = span.enter();
            let started = Instant::now();
            let result = body();
            match result {
                Ok(value) => {
                    // Snapshot/auth polling produces no routine log volume.
                    if !matches!(operation, "snapshot" | "authentication_poll") {
                        tracing::info!(
                            event = "operation_completed",
                            elapsed_ms = started.elapsed().as_millis() as u64
                        );
                    }
                    Ok(value)
                }
                Err(error) => {
                    let (kind, code, phase, detail) = error.diagnostic_parts();
                    tracing::error!(
                        event = "operation_failed",
                        code,
                        phase,
                        detail = detail.as_str(),
                        failure_kind = kind.as_str(),
                        elapsed_ms = started.elapsed().as_millis() as u64
                    );
                    Err(BindingError::Diagnostic {
                        kind,
                        code,
                        phase,
                        detail,
                        operation_id,
                    })
                }
            }
        })
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
        }
    }
}
