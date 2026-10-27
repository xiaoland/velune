use super::*;
use opentelemetry_proto::tonic::collector::{
    logs::v1::{
        ExportLogsServiceRequest, ExportLogsServiceResponse,
        logs_service_server::{LogsService, LogsServiceServer},
    },
    metrics::v1::{
        ExportMetricsServiceRequest, ExportMetricsServiceResponse,
        metrics_service_server::{MetricsService, MetricsServiceServer},
    },
    trace::v1::{
        ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
        trace_service_server::{TraceService, TraceServiceServer},
    },
};

#[derive(Default)]
struct Received {
    traces: Vec<ExportTraceServiceRequest>,
    logs: Vec<ExportLogsServiceRequest>,
    metrics: Vec<ExportMetricsServiceRequest>,
    headers: Vec<tonic::metadata::MetadataMap>,
}
#[derive(Clone, Default)]
struct Receiver {
    received: Arc<Mutex<Received>>,
    partial: bool,
    stalled: bool,
}
#[tonic::async_trait]
impl TraceService for Receiver {
    async fn export(
        &self,
        request: tonic::Request<ExportTraceServiceRequest>,
    ) -> std::result::Result<tonic::Response<ExportTraceServiceResponse>, tonic::Status> {
        if self.stalled {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        let mut received = self.received.lock().unwrap();
        received.headers.push(request.metadata().clone());
        received.traces.push(request.into_inner());
        Ok(tonic::Response::new(ExportTraceServiceResponse {
            partial_success: self.partial.then(|| ExportTracePartialSuccess {
                rejected_spans: 1,
                error_message: "synthetic rejection".into(),
            }),
        }))
    }
}
#[tonic::async_trait]
impl LogsService for Receiver {
    async fn export(
        &self,
        request: tonic::Request<ExportLogsServiceRequest>,
    ) -> std::result::Result<tonic::Response<ExportLogsServiceResponse>, tonic::Status> {
        if self.stalled {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        let mut received = self.received.lock().unwrap();
        received.headers.push(request.metadata().clone());
        received.logs.push(request.into_inner());
        Ok(tonic::Response::new(ExportLogsServiceResponse::default()))
    }
}
#[tonic::async_trait]
impl MetricsService for Receiver {
    async fn export(
        &self,
        request: tonic::Request<ExportMetricsServiceRequest>,
    ) -> std::result::Result<tonic::Response<ExportMetricsServiceResponse>, tonic::Status> {
        if self.stalled {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        let mut received = self.received.lock().unwrap();
        received.headers.push(request.metadata().clone());
        received.metrics.push(request.into_inner());
        Ok(tonic::Response::new(ExportMetricsServiceResponse::default()))
    }
}
struct Server {
    receiver: Receiver,
    endpoint: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn open(partial: bool) -> Self {
        Self::with_stall(partial, false)
    }
    fn with_stall(partial: bool, stalled: bool) -> Self {
        let receiver = Receiver {
            partial,
            stalled,
            ..Default::default()
        };
        let services = receiver.clone();
        let (port_tx, port_rx) = std::sync::mpsc::sync_channel(1);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let join = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    port_tx.send(listener.local_addr().unwrap().port()).unwrap();
                    tonic::transport::Server::builder()
                        .add_service(TraceServiceServer::new(services.clone()))
                        .add_service(LogsServiceServer::new(services.clone()))
                        .add_service(MetricsServiceServer::new(services))
                        .serve_with_incoming_shutdown(
                            tokio_stream::wrappers::TcpListenerStream::new(listener),
                            async {
                                let _ = stopped.await;
                            },
                        )
                        .await
                        .unwrap();
                });
        });
        Self {
            receiver,
            endpoint: format!("http://127.0.0.1:{}", port_rx.recv().unwrap()),
            stop: Some(stop),
            join: Some(join),
        }
    }
    fn profile(&self) -> Profile {
        Profile {
            endpoint: self.endpoint.clone(),
            headers: [("x-velune-synthetic".into(), "allowed".into())].into(),
            sample_ratio: 1.0,
        }
    }
    fn operations(&self) -> Vec<String> {
        let received = self.receiver.received.lock().unwrap();
        received
            .traces
            .iter()
            .flat_map(|request| &request.resource_spans)
            .flat_map(|rs| &rs.scope_spans)
            .flat_map(|ss| &ss.spans)
            .flat_map(|span| &span.attributes)
            .filter(|a| a.key == "operation")
            .filter_map(|a| a.value.as_ref())
            .filter_map(|v| match &v.value {
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(v)) => {
                    Some(v.clone())
                }
                _ => None,
            })
            .filter(|name| name != "telemetry.activated")
            .collect()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        let _ = self.join.take().unwrap().join();
    }
}
fn client(home: &Path) -> Arc<ClientTelemetry> {
    ClientTelemetry::open(
        home,
        ResourceIdentity::new("test", "synthetic").unwrap(),
        false,
        None,
    )
    .unwrap()
}
fn activate(owner: &ClientTelemetry, server: &Server) {
    let p = owner
        .prepare(
            owner.snapshot().unwrap().revision,
            Preference::On,
            Some(server.profile()),
        )
        .unwrap();
    owner.commit(&p.id).unwrap();
    owner.confirm_native(p.revision, true).unwrap();
}
fn operation(owner: &ClientTelemetry, name: &'static str) {
    let mut op = owner.operation(name, None, false);
    op.run(|| {
    tracing::error!(target: "velune_unapproved", prompt = "SENSITIVE-CANARY", path = "/synthetic/private", event = "unapproved");
    tracing::error!(target: "velune_telemetry", prompt = "SENSITIVE-CANARY", event = "unapproved_field");
});
    op.complete(Outcome::Failure);
}

#[test]
fn real_grpc_three_signals_revision_switch_and_retired_queue() {
    let first = Server::open(false);
    let second = Server::open(false);
    let home = tempfile::tempdir().unwrap();
    let owner = client(home.path());
    activate(&owner, &first);
    operation(&owner, "before");
    assert!(owner.flush());
    operation(&owner, "discard");
    let parent = owner.operation("old_parent", None, false);
    let carrier = parent.carrier();
    let mut in_flight = parent.run(|| owner.operation("old_in_flight", carrier.as_ref(), false));
    let original_carrier = in_flight.carrier().unwrap().traceparent;
    drop(parent);
    activate(&owner, &second);
    assert_eq!(in_flight.carrier().unwrap().traceparent, original_carrier);
    in_flight.complete(Outcome::Success);
    tracing::dispatcher::with_default(&owner.dispatch(), || drop(in_flight));
    operation(&owner, "after");
    assert!(owner.flush());
    owner.shutdown().unwrap();
    assert_eq!(first.operations(), ["before"]);
    assert_eq!(second.operations(), ["after"]);
    for server in [&first, &second] {
        let received = server.receiver.received.lock().unwrap();
        assert!(
            !received.traces.is_empty()
                && !received.logs.is_empty()
                && !received.metrics.is_empty()
        );
        for headers in &received.headers {
            assert_eq!(headers.get("x-velune-synthetic").unwrap(), "allowed");
            assert!(headers.get("x-probe-env").is_none());
        }
        let data = format!(
            "{:?}{:?}{:?}",
            received.traces, received.logs, received.metrics
        );
        assert!(!data.contains("SENSITIVE-CANARY") && !data.contains("/synthetic/private"));
    }
}

#[test]
fn explicit_probe_reports_partial_rejection_without_enabling_collection() {
    let server = Server::open(true);
    let home = tempfile::tempdir().unwrap();
    let mut identity = ResourceIdentity::new("test", "synthetic").unwrap();
    identity.run_id = Some("synthetic-test-run".into());
    let owner = ClientTelemetry::open(home.path(), identity, false, None).unwrap();
    let result = owner.send_test(&server.profile()).unwrap();
    assert_eq!(result.traces, "partially_rejected");
    assert_eq!(result.logs, "accepted");
    assert_eq!(result.metrics, "accepted");
    assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
    assert!(!home.path().join("telemetry/config.json").exists());
    let received = server.receiver.received.lock().unwrap();
    let scope = &received.traces[0].resource_spans[0].scope_spans[0].scope;
    assert_eq!(scope.as_ref().unwrap().name, "velune.telemetry.test");
    assert!(
        format!("{:?}", received.traces[0].resource_spans[0].resource)
            .contains("synthetic-test-run")
    );
    assert!(
        format!("{:?}", received.logs[0].resource_logs[0].resource).contains("synthetic-test-run")
    );
    assert!(
        format!("{:?}", received.metrics[0].resource_metrics[0].resource)
            .contains("synthetic-test-run")
    );
}

#[test]
fn unsampled_failures_still_export_logs_and_metrics() {
    let server = Server::open(false);
    let home = tempfile::tempdir().unwrap();
    let owner = client(home.path());
    let mut profile = server.profile();
    profile.sample_ratio = 0.0;
    let p = owner.prepare(0, Preference::On, Some(profile)).unwrap();
    owner.commit(&p.id).unwrap();
    owner.confirm_native(p.revision, true).unwrap();
    operation(&owner, "failure");
    assert!(owner.flush());
    owner.shutdown().unwrap();
    let received = server.receiver.received.lock().unwrap();
    assert!(received.traces.is_empty());
    assert!(!received.logs.is_empty());
    assert!(!received.metrics.is_empty());
}

#[test]
fn official_sdk_drop_summaries_keep_the_retired_revision_without_private_messages() {
    let server = Server::with_stall(false, true);
    let home = tempfile::tempdir().unwrap();
    let owner = client(home.path());
    activate(&owner, &server);
    let revision = owner.active_revision().unwrap();
    for _ in 0..3000 {
        operation(&owner, "saturated");
    }
    let off = owner.prepare(revision, Preference::Off, None).unwrap();
    owner.commit(&off.id).unwrap();
    owner.confirm_native(off.revision, true).unwrap();
    let path = home.path().join("telemetry/sdk-diagnostics.json");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let data: serde_json::Value = serde_json::from_str(&text).unwrap();
            let records = data["records"].as_array().unwrap();
            if records.len() == 2 {
                for record in records {
                    assert_eq!(record["revision"], revision);
                    assert_eq!(record["scope"], "rust");
                    assert!(record["droppedRecords"].as_u64().unwrap() > 0);
                }
                assert!(!text.contains("CANARY"));
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "SDK did not report the old generation's drops"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
    owner.shutdown().unwrap();
}

#[test]
fn execution_gauge_keeps_the_authoritative_count_across_sdk_generations() {
    use opentelemetry_proto::tonic::metrics::v1::{metric::Data, number_data_point::Value};
    fn values(server: &Server, name: &str) -> Vec<i64> {
        server
            .receiver
            .received
            .lock()
            .unwrap()
            .metrics
            .iter()
            .flat_map(|request| &request.resource_metrics)
            .flat_map(|resource| &resource.scope_metrics)
            .flat_map(|scope| &scope.metrics)
            .filter(|metric| metric.name == name)
            .filter_map(|metric| match &metric.data {
                Some(Data::Gauge(gauge)) => Some(gauge),
                _ => None,
            })
            .flat_map(|gauge| &gauge.data_points)
            .filter_map(|point| match point.value {
                Some(Value::AsInt(value)) => Some(value),
                _ => None,
            })
            .collect()
    }
    let first = Server::open(false);
    let second = Server::open(false);
    let home = tempfile::tempdir().unwrap();
    let owner = client(home.path());
    assert!(owner.snapshot().unwrap().execution.is_none());
    owner.observe_execution(2);
    activate(&owner, &first);
    assert!(owner.flush());
    activate(&owner, &second);
    assert!(owner.flush());
    assert_eq!(values(&first, "velune.execution.active_turns"), [2]);
    assert_eq!(values(&second, "velune.execution.active_turns"), [2]);
    owner.observe_execution(0);
    assert!(owner.flush());
    assert_eq!(values(&second, "velune.execution.active_turns"), [2, 0]);
    owner.execution_owner_unavailable();
    assert!(owner.flush());
    assert!(!owner.snapshot().unwrap().execution.unwrap().owner_available);
    assert_eq!(
        values(&second, "velune.execution.owner_available").last(),
        Some(&0)
    );
    owner.shutdown().unwrap();
}
