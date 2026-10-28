use crate::{
    Result, TelemetryIdentity, TelemetryProfile,
    transport::{self, Permission},
};
use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::{ExportLogsServiceRequest, logs_service_client::LogsServiceClient},
        metrics::v1::{ExportMetricsServiceRequest, metrics_service_client::MetricsServiceClient},
        trace::v1::{ExportTraceServiceRequest, trace_service_client::TraceServiceClient},
    },
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
    metrics::v1::{
        Gauge, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric, number_data_point,
    },
    resource::v1::Resource,
    trace::v1::{ResourceSpans, ScopeSpans, Span},
};

#[derive(Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryTestResult {
    pub traces: String,
    pub logs: String,
    pub metrics: String,
}
fn attribute(key: &str, value: String) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value)),
        }),
        ..Default::default()
    }
}
fn outcome<T>(
    response: std::result::Result<tonic::Response<T>, tonic::Status>,
    rejected: impl FnOnce(T) -> bool,
) -> String {
    match response {
        Ok(response) => {
            if rejected(response.into_inner()) {
                "partially_rejected"
            } else {
                "accepted"
            }
        }
        Err(error) => match error.code() {
            tonic::Code::Unauthenticated => "unauthenticated",
            tonic::Code::PermissionDenied => "permission_denied",
            tonic::Code::DeadlineExceeded => "timeout",
            tonic::Code::Unavailable => "unavailable",
            _ => "rejected",
        },
    }
    .into()
}
pub(crate) async fn send(
    profile: &TelemetryProfile,
    identity: &TelemetryIdentity,
) -> Result<TelemetryTestResult> {
    profile.validate()?;
    let permission = Permission::default();
    permission.enable();
    let channel = transport::channel(profile, permission.clone())?;
    let metadata = transport::metadata(profile)?;
    let request = |body| {
        let mut request = tonic::Request::new(body);
        *request.metadata_mut() = metadata.clone();
        request.set_timeout(std::time::Duration::from_secs(3));
        request
    };
    let mut attributes = vec![
        attribute("service.name", identity.service_name.clone()),
        attribute("service.instance.id", identity.instance_id.clone()),
        attribute("service.version", identity.version.clone()),
        attribute("velune.platform", identity.platform.clone()),
        attribute("deployment.environment.name", identity.environment.clone()),
    ];
    if let Some(run_id) = &identity.run_id {
        attributes.push(attribute("velune.run.id", run_id.clone()));
    }
    let resource = Some(Resource {
        attributes,
        ..Default::default()
    });
    let scope = Some(InstrumentationScope {
        name: "velune.telemetry.test".into(),
        ..Default::default()
    });
    let trace_id = crate::config::random_id()?;
    let bytes = |id: &str| {
        (0..id.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&id[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    let trace_id = bytes(&trace_id);
    let span_id = bytes(&crate::config::random_id()?[..16]);
    let now = crate::now_ms() * 1_000_000;
    let traces = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: resource.clone(),
            scope_spans: vec![ScopeSpans {
                scope: scope.clone(),
                spans: vec![Span {
                    name: "telemetry.test".into(),
                    trace_id: trace_id.clone(),
                    span_id: span_id.clone(),
                    start_time_unix_nano: now,
                    end_time_unix_nano: now + 1,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let traces = outcome(
        TraceServiceClient::new(channel.clone())
            .export(request(traces))
            .await,
        |r| {
            r.partial_success
                .is_some_and(|p| p.rejected_spans != 0 || !p.error_message.is_empty())
        },
    );
    let mut log_request = tonic::Request::new(ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: resource.clone(),
            scope_logs: vec![ScopeLogs {
                scope: scope.clone(),
                log_records: vec![LogRecord {
                    time_unix_nano: now,
                    observed_time_unix_nano: now,
                    trace_id,
                    span_id,
                    severity_number: 9,
                    body: Some(AnyValue {
                        value: Some(any_value::Value::StringValue("telemetry.test".into())),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    });
    *log_request.metadata_mut() = metadata.clone();
    log_request.set_timeout(std::time::Duration::from_secs(3));
    let logs = outcome(
        LogsServiceClient::new(channel.clone())
            .export(log_request)
            .await,
        |r| {
            r.partial_success
                .is_some_and(|p| p.rejected_log_records != 0 || !p.error_message.is_empty())
        },
    );
    let mut metric_request = tonic::Request::new(ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource,
            scope_metrics: vec![ScopeMetrics {
                scope,
                metrics: vec![Metric {
                    name: "velune.telemetry.test".into(),
                    data: Some(metric::Data::Gauge(Gauge {
                        data_points: vec![NumberDataPoint {
                            time_unix_nano: now,
                            value: Some(number_data_point::Value::AsInt(1)),
                            ..Default::default()
                        }],
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    });
    *metric_request.metadata_mut() = metadata;
    metric_request.set_timeout(std::time::Duration::from_secs(3));
    let metrics = outcome(
        MetricsServiceClient::new(channel)
            .export(metric_request)
            .await,
        |r| {
            r.partial_success
                .is_some_and(|p| p.rejected_data_points != 0 || !p.error_message.is_empty())
        },
    );
    permission.revoke();
    Ok(TelemetryTestResult {
        traces,
        logs,
        metrics,
    })
}
