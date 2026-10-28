//! Local LLM ingress. Same-protocol traffic remains native; cross-protocol
//! routing uses request-scoped best-effort translation. A model route fails
//! over to its next member only before anything has reached the caller.
mod attempts;
mod credentials;
mod failover;
mod ingress;
mod routes;
mod wire;

pub use credentials::{
    CredentialMaterial, CredentialRecovery, CredentialResolutionError, CredentialResolver,
    CredentialTarget, HttpAuthorization, RequestCapabilities, RequestProfile, ResolvedCredential,
};
pub(crate) use wire::{WireEvent, send_json};

use crate::{
    analytics::SharedAnalyticsSink,
    config::{ExecutionBackend, GatewayConfig},
    scheduling::{QuotaSource, SchedulingState, SessionIdentity, UnknownQuotaSource},
};
use ingress::Ingress;
use reqwest::Client;
use routes::{Route, build_routes};
use std::{
    collections::BTreeMap,
    net::TcpListener,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::oneshot;
use velune_ai_provider::{HttpClient, claude_code::ClaudeCodePool};

#[derive(Debug)]
pub struct GatewayError(pub(crate) &'static str);
impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for GatewayError {}

#[derive(Clone, Default)]
pub(crate) struct TelemetryContext {
    owner: Option<Arc<velune_telemetry::ClientTelemetry>>,
    parent: Option<velune_telemetry::TraceCarrier>,
    parent_revision: Option<u64>,
}
impl TelemetryContext {
    pub(super) fn operation(
        &self,
        name: &'static str,
        kind: velune_telemetry::SpanKind,
    ) -> Option<velune_telemetry::Operation> {
        let owner = self.owner.as_ref()?;
        let mut operation = owner.operation_with_kind(name, kind, self.parent.as_ref(), false);
        if self.parent.is_none() {
            operation.correlation_gap(velune_telemetry::CorrelationGap::MissingParent);
        } else if self.parent_revision != operation.revision() {
            operation.correlation_gap(velune_telemetry::CorrelationGap::RetiredGeneration);
        }
        Some(operation)
    }
}

/// Shared execution resources, independently owned from any individual Runner.
#[derive(Clone, Default)]
pub struct ExecutionBackends {
    pub claude_code: Arc<ClaudeCodePool>,
    bindings: Arc<std::sync::Mutex<BTreeMap<(String, String), String>>>,
}
impl ExecutionBackends {
    pub(super) fn session_key(gateway_id: &str, session: &SessionIdentity) -> String {
        serde_json::to_string(&(gateway_id, &session.namespace, &session.id))
            .expect("session tuple")
    }
    /// Begin an explicitly new user turn; pending or running work must be cancelled first.
    pub fn begin_user_turn(
        &self,
        gateway_id: &str,
        session: &SessionIdentity,
    ) -> Result<(), velune_ai::InvalidContract> {
        self.claude_code
            .begin_user_turn(&Self::session_key(gateway_id, session))
    }
    /// Cancel session execution, including tools waiting after the HTTP response ended.
    pub fn cancel_session(&self, gateway_id: &str, session: &SessionIdentity) {
        self.claude_code
            .cancel_session(&Self::session_key(gateway_id, session));
    }
    /// Settle an outer turn, cancelling unfinished execution while preserving idle sessions.
    pub fn finish_user_turn(&self, gateway_id: &str, session: &SessionIdentity) {
        self.claude_code
            .finish_user_turn(&Self::session_key(gateway_id, session));
    }
    pub fn invalidate_provider(&self, gateway_id: &str, provider_id: &str) {
        if let Some(binding) = self
            .bindings
            .lock()
            .expect("backend bindings")
            .remove(&(gateway_id.into(), provider_id.into()))
        {
            self.claude_code.invalidate(&binding);
        }
    }
    pub fn shutdown(&self) {
        self.claude_code.shutdown();
        self.bindings.lock().expect("backend bindings").clear();
    }
    fn register(&self, gateway: &str, provider: &str, epoch: u64) -> String {
        let binding = serde_json::to_string(&(gateway, provider, epoch)).expect("binding tuple");
        let mut bindings = self.bindings.lock().expect("backend bindings");
        if let Some(previous) = bindings.insert((gateway.into(), provider.into()), binding.clone())
            && previous != binding
        {
            self.claude_code.invalidate(&previous);
        }
        binding
    }
}

pub struct Runner {
    telemetry: Arc<RwLock<TelemetryContext>>,
    config: GatewayConfig,
    routes: Arc<RwLock<BTreeMap<String, Route>>>,
    endpoint: String,
    token: String,
    stopping: Arc<AtomicBool>,
    stop: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}
impl Runner {
    /// Bind an actual session/turn owner. Requests capture this value once at admission.
    pub fn set_telemetry(
        &self,
        owner: Option<Arc<velune_telemetry::ClientTelemetry>>,
        parent: Option<velune_telemetry::TraceCarrier>,
        revision: Option<u64>,
    ) {
        *self.telemetry.write().unwrap_or_else(|e| e.into_inner()) = TelemetryContext {
            owner,
            parent,
            parent_revision: revision,
        };
    }
    /// Replace only runtime aliases. In-flight calls retain their captured target.
    /// Target configurations, credentials and the ingress token stay unchanged.
    pub fn replace_aliases(
        &self,
        aliases: BTreeMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let next = build_routes(&self.config, &aliases)?;
        *self
            .routes
            .write()
            .map_err(|_| GatewayError("gateway alias lock"))? = next;
        Ok(())
    }
    pub fn start(
        config: GatewayConfig,
        credential_resolver: Arc<dyn CredentialResolver>,
        aliases: BTreeMap<String, String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::start_with_analytics(
            config,
            credential_resolver,
            aliases,
            None,
            SchedulingState::default(),
        )
    }
    pub fn start_with_analytics(
        config: GatewayConfig,
        credential_resolver: Arc<dyn CredentialResolver>,
        aliases: BTreeMap<String, String>,
        analytics: Option<SharedAnalyticsSink>,
        health: SchedulingState,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::start_with_scheduling(
            config,
            credential_resolver,
            aliases,
            analytics,
            health,
            Arc::new(UnknownQuotaSource),
            None,
            ExecutionBackends::default(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn start_with_scheduling(
        config: GatewayConfig,
        credential_resolver: Arc<dyn CredentialResolver>,
        aliases: BTreeMap<String, String>,
        analytics: Option<SharedAnalyticsSink>,
        health: SchedulingState,
        quotas: Arc<dyn QuotaSource>,
        session: Option<SessionIdentity>,
        backends: ExecutionBackends,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if session.as_ref().is_some_and(|session| {
            session.namespace.trim().is_empty() || session.id.trim().is_empty()
        }) {
            return Err(Box::new(GatewayError(
                "session namespace and identity must be nonempty",
            )));
        }
        config.validate().map_err(GatewayError)?;
        let routes = Arc::new(RwLock::new(build_routes(&config, &aliases)?));
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|_| GatewayError("gateway token"))?;
        let token = random
            .iter()
            .map(|value| format!("{value:02x}"))
            .collect::<String>();
        let client = HttpClient::new(Client::builder().connect_timeout(Duration::from_secs(15)))?;
        let mut epoch = [0u8; 8];
        getrandom::fill(&mut epoch).map_err(|_| GatewayError("gateway request identity"))?;
        let stopping = Arc::new(AtomicBool::new(false));
        let provider_epochs: BTreeMap<_, _> = config
            .providers
            .iter()
            .map(|provider| (provider.id.clone(), health.epoch(&config.id, &provider.id)))
            .collect();
        let backend_bindings = config
            .providers
            .iter()
            .filter(|provider| {
                matches!(
                    provider.execution_backend,
                    ExecutionBackend::ClaudeCli { .. }
                )
            })
            .map(|provider| {
                (
                    provider.id.clone(),
                    backends.register(&config.id, &provider.id, provider_epochs[&provider.id]),
                )
            })
            .collect();
        let telemetry = Arc::new(RwLock::new(TelemetryContext::default()));
        let state = Arc::new(Ingress {
            telemetry: telemetry.clone(),
            routes: routes.clone(),
            token: token.clone(),
            resolver: credential_resolver,
            client,
            request_epoch: u64::from_le_bytes(epoch),
            next_request: AtomicU64::new(1),
            stopping: stopping.clone(),
            analytics,
            provider_epochs,
            backend_bindings,
            backends,
            gateway_id: config.id.clone(),
            health,
            quotas,
            session,
        });
        let (stop, stopped) = oneshot::channel();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        let handle = thread::Builder::new().name("velune-ai-gateway".into()).spawn(move || {
            tracing::dispatcher::with_default(&dispatcher, || {
                let _entered = parent.enter();
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime, Err(_) => { tracing::error!(event="gateway_executor_failed"); return; }
                };
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) { Ok(value)=>value,Err(_)=>return };
                    let router = ingress::router(state);
                    tracing::info!(event="gateway_started");
                    // Dropping the server stops admission; dropping this executor aborts
                    // connection/dispatch tasks, including their kill-on-drop helpers.
                    tokio::select! { result = axum::serve(listener,router) => { if result.is_err() { tracing::warn!(event="gateway_listener_failed"); } }, _ = stopped => {} }
                    tracing::info!(event="gateway_stopped");
                });
            });
        })?;
        Ok(Self {
            telemetry,
            config,
            routes,
            endpoint: format!("http://127.0.0.1:{}/v1", address.port()),
            token,
            stopping,
            stop: Some(stop),
            handle: Some(handle),
        })
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
