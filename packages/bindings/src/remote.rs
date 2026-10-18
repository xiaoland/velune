//! Remote assembly owns transport resources; the application owner keeps business work alive.
use crate::*;
use std::{path::Path, sync::Arc, time::Duration};
use velune_application_api::{ApplicationReply, ExecutionContext, Revision};
use velune_remote::{
    DeviceIdentity, EndpointConfig, EndpointDescriptor, FileTrustStore, RemoteAccessService,
    RemoteClient, RequestId, RequestStatus,
};

#[derive(Clone, uniffi::Enum)]
pub enum BindingRelayConfig {
    Disabled,
    /// Development infrastructure is an explicit opt-in, without a production SLA.
    DevelopmentPublic,
    Custom {
        urls: Vec<String>,
    },
}
#[derive(Clone, uniffi::Enum)]
pub enum BindingDiscoveryConfig {
    None,
    DevelopmentPublic,
    CustomPkarr { url: String },
}
#[derive(Clone, uniffi::Enum)]
pub enum BindingRemotePathPolicy {
    Automatic,
    RelayOnly,
}
#[derive(Clone, Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingConnectionPathKind {
    Direct,
    Relay,
    Other,
}
#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingPathAddressFamily {
    Ipv4,
    Ipv6,
    Other,
}
#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingPathSource {
    Unknown,
}

#[derive(Clone, Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingNetworkChangeReason {
    Available,
    Lost,
    LinkProperties,
    Capabilities,
    Foreground,
}
#[derive(Clone, Serialize, uniffi::Record)]
pub struct BindingPlatformNetworkSnapshot {
    pub sequence: u64,
    pub elapsed_realtime_ms: u64,
    pub reason: BindingNetworkChangeReason,
    pub default_network_id: Option<String>,
    pub networks: Vec<BindingPlatformNetwork>,
}
#[derive(Clone, Serialize, uniffi::Record)]
pub struct BindingPlatformNetwork {
    pub id: String,
    pub wifi: bool,
    pub cellular: bool,
    pub ethernet: bool,
    pub vpn: bool,
    pub internet: bool,
    pub validated: bool,
    pub interface_name: Option<String>,
    pub addresses: Vec<String>,
    pub dns_servers: Vec<String>,
}
impl TryFrom<BindingPlatformNetworkSnapshot> for velune_remote::PlatformNetworkSnapshot {
    type Error = BindingError;
    fn try_from(value: BindingPlatformNetworkSnapshot) -> Result<Self, Self::Error> {
        let snapshot: Self = convert(value)?;
        snapshot.validate()?;
        Ok(snapshot)
    }
}

#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingPathObservation {
    pub status: BindingConnectionStatus,
    pub missed_events: u64,
}

/// An observer retains transport evidence, but disposing it does not close business work.
#[derive(uniffi::Object)]
pub struct VelunePathObserver {
    runtime: Arc<RemoteRuntime>,
    observer: tokio::sync::Mutex<velune_remote::PathObserver>,
}
impl VelunePathObserver {
    pub(crate) fn new(
        runtime: Arc<RemoteRuntime>,
        observer: velune_remote::PathObserver,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            observer: tokio::sync::Mutex::new(observer),
        })
    }
}
#[uniffi::export]
impl VelunePathObserver {
    /// Null means no event within the bounded wait, or the stream ended; check status.closed.
    pub fn next(&self, timeout_ms: u64) -> Result<Option<BindingPathObservation>, BindingError> {
        if timeout_ms == 0 || timeout_ms > 5_000 {
            return Err(BindingError::Invalid {
                detail: "路径观察等待必须为 1–5000 毫秒".into(),
            });
        }
        let status = self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(timeout_ms), async {
                self.observer.lock().await.next().await
            })
            .await
            .ok()
            .flatten()
        });
        status.map(convert).transpose()
    }
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingConnectionPath {
    pub path_id: u64,
    pub address_family: BindingPathAddressFamily,
    pub source: BindingPathSource,
    pub kind: BindingConnectionPathKind,
    pub selected: bool,
    pub rtt_microseconds: u64,
    pub transmitted_bytes: u64,
    pub received_bytes: u64,
    pub lost_packets: u64,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingConnectionStatus {
    pub connection_id: u64,
    pub sampled_at_micros: u64,
    pub peer_id: String,
    pub closed: bool,
    pub paths: Vec<BindingConnectionPath>,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingRemoteOptions {
    pub identity_file: String,
    pub trust_file: String,
    pub device_name: String,
    pub relay: BindingRelayConfig,
    pub discovery: BindingDiscoveryConfig,
    pub path_policy: BindingRemotePathPolicy,
    pub bind_addresses: Vec<String>,
    pub request_timeout_ms: u64,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingRemoteEndpointDescriptor {
    pub version: u16,
    pub endpoint_id: String,
    pub name: String,
    pub incarnation: String,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<String>,
}
impl From<EndpointDescriptor> for BindingRemoteEndpointDescriptor {
    fn from(value: EndpointDescriptor) -> Self {
        Self {
            version: value.version,
            endpoint_id: value.endpoint_id,
            name: value.name,
            incarnation: value.incarnation,
            relay_urls: value.relay_urls,
            direct_addresses: value
                .direct_addresses
                .into_iter()
                .map(|address| address.to_string())
                .collect(),
        }
    }
}
impl TryFrom<BindingRemoteEndpointDescriptor> for EndpointDescriptor {
    type Error = BindingError;
    fn try_from(value: BindingRemoteEndpointDescriptor) -> Result<Self, Self::Error> {
        Ok(Self {
            version: value.version,
            endpoint_id: value.endpoint_id,
            name: value.name,
            incarnation: value.incarnation,
            relay_urls: value.relay_urls,
            direct_addresses: parse_addresses(value.direct_addresses)?,
        })
    }
}
/// A one-time pairing credential, exposed only for the user's explicit transfer.
#[derive(Clone, uniffi::Record)]
pub struct BindingRemoteInvitation {
    pub encoded: String,
    pub expires_at_unix_seconds: u64,
    pub endpoint: BindingRemoteEndpointDescriptor,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingTrustedDevice {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, uniffi::Record)]
pub struct BindingRemoteRequestId {
    pub incarnation: String,
    pub client: String,
    pub sequence: u64,
}
impl From<RequestId> for BindingRemoteRequestId {
    fn from(value: RequestId) -> Self {
        Self {
            incarnation: value.incarnation,
            client: value.client,
            sequence: value.sequence,
        }
    }
}
impl From<BindingRemoteRequestId> for RequestId {
    fn from(value: BindingRemoteRequestId) -> Self {
        Self {
            incarnation: value.incarnation,
            client: value.client,
            sequence: value.sequence,
        }
    }
}
#[derive(uniffi::Record)]
pub struct BindingVersionedConfiguration {
    pub configuration: BindingConfigurationSnapshot,
    pub revision: u64,
}
#[derive(uniffi::Record)]
pub struct BindingRemoteFailure {
    pub kind: BindingFailureKind,
    pub detail: String,
    pub code: String,
    pub phase: String,
    pub current_revision: Option<u64>,
}
impl From<Failure> for BindingRemoteFailure {
    fn from(value: Failure) -> Self {
        Self {
            kind: failure_kind(value.kind),
            detail: value.message,
            code: value.code,
            phase: value.phase,
            current_revision: value.current_revision.map(|revision| revision.0),
        }
    }
}
#[derive(uniffi::Enum)]
pub enum BindingRemoteRequestStatus {
    Running,
    Completed {
        revision: u64,
        response: Box<BindingApplicationResponse>,
    },
    Failed {
        failure: BindingRemoteFailure,
    },
    Unknown,
    ResultExpired,
    TransportFailed {
        kind: BindingFailureKind,
        detail: String,
    },
}
impl TryFrom<RequestStatus> for BindingRemoteRequestStatus {
    type Error = BindingError;
    fn try_from(value: RequestStatus) -> Result<Self, Self::Error> {
        Ok(match value {
            RequestStatus::Running => Self::Running,
            RequestStatus::Completed(Ok(reply)) => Self::Completed {
                revision: reply.revision.0,
                response: Box::new(reply.response.try_into()?),
            },
            RequestStatus::Completed(Err(failure)) => Self::Failed {
                failure: failure.into(),
            },
            RequestStatus::Unknown => Self::Unknown,
            RequestStatus::ResultExpired => Self::ResultExpired,
            RequestStatus::Failed(error) => Self::TransportFailed {
                kind: remote_failure_kind(error),
                detail: error.to_string(),
            },
        })
    }
}

pub(crate) fn parse_addresses(
    addresses: Vec<String>,
) -> Result<Vec<std::net::SocketAddr>, BindingError> {
    addresses
        .into_iter()
        .map(|address| {
            address.parse().map_err(|_| BindingError::Invalid {
                detail: "远控绑定或拨号地址无效".into(),
            })
        })
        .collect()
}
fn remote_failure_kind(error: velune_remote::RemoteError) -> BindingFailureKind {
    use velune_remote::RemoteError;
    match error {
        RemoteError::Unauthorized => BindingFailureKind::Unauthorized,
        RemoteError::Timeout | RemoteError::ServerRestarted => BindingFailureKind::OutcomeUnknown,
        RemoteError::Configuration
        | RemoteError::InvalidInvitation
        | RemoteError::ExpiredInvitation
        | RemoteError::InvalidRequestId
        | RemoteError::RequestIdConflict => BindingFailureKind::Invalid,
        RemoteError::IdentityStorage | RemoteError::TrustStorage => BindingFailureKind::Io,
        RemoteError::Closed => BindingFailureKind::Closed,
        RemoteError::Protocol | RemoteError::IncompatibleVersion | RemoteError::FrameTooLarge => {
            BindingFailureKind::Contract
        }
        RemoteError::Randomness
        | RemoteError::Unavailable
        | RemoteError::Capacity
        | RemoteError::NoHandler => BindingFailureKind::Connection,
    }
}
impl From<velune_remote::RemoteError> for BindingError {
    fn from(error: velune_remote::RemoteError) -> Self {
        Self::Diagnostic {
            kind: remote_failure_kind(error),
            detail: error.to_string(),
            code: error.code().into(),
            phase: "remote".into(),
            operation_id: String::new(),
        }
    }
}

/// Runtime leases can finish on a Tokio worker; the final release must not block that worker.
pub(crate) struct RemoteRuntime {
    runtime: Option<tokio::runtime::Runtime>,
}
impl RemoteRuntime {
    pub(crate) fn new() -> Result<Arc<Self>, BindingError> {
        Ok(Arc::new(Self {
            runtime: Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|_| BindingError::Unavailable)?,
            ),
        }))
    }
}
impl std::ops::Deref for RemoteRuntime {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.runtime
            .as_ref()
            .expect("runtime remains present until its final drop")
    }
}
impl Drop for RemoteRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(feature = "local-runtime")]
struct ApplicationHandler {
    application: ApplicationService,
    _runtime: Arc<RemoteRuntime>,
}
#[cfg(feature = "local-runtime")]
impl velune_remote::Handler for ApplicationHandler {
    fn execute(
        &self,
        context: velune_remote::RequestContext,
        command: Command,
    ) -> velune_remote::HandlerFuture<'_> {
        let application = self.application.clone();
        Box::pin(async move {
            let view = ClientViewId(format!("{}:{}", context.peer_id, context.view.0));
            tokio::task::spawn_blocking(move || {
                application.execute_checked(view, context.execution, command)
            })
            .await
            .map_err(|_| {
                Failure::new(
                    velune_application_api::FailureKind::Unavailable,
                    "执行端核心不可用",
                )
            })?
        })
    }
}

#[derive(Clone, uniffi::Record)]
pub struct BindingRemoteNetworkOptions {
    pub relay: BindingRelayConfig,
    pub discovery: BindingDiscoveryConfig,
    pub path_policy: BindingRemotePathPolicy,
    pub bind_addresses: Vec<String>,
    pub request_timeout_ms: u64,
}
impl From<&BindingRemoteOptions> for BindingRemoteNetworkOptions {
    fn from(options: &BindingRemoteOptions) -> Self {
        Self {
            relay: options.relay.clone(),
            discovery: options.discovery.clone(),
            path_policy: options.path_policy.clone(),
            bind_addresses: options.bind_addresses.clone(),
            request_timeout_ms: options.request_timeout_ms,
        }
    }
}
impl BindingRemoteNetworkOptions {
    fn config(self) -> Result<EndpointConfig, BindingError> {
        Ok(EndpointConfig {
            relay: match self.relay {
                BindingRelayConfig::Disabled => velune_remote::RelayConfig::Disabled,
                BindingRelayConfig::DevelopmentPublic => {
                    velune_remote::RelayConfig::DevelopmentPublic
                }
                BindingRelayConfig::Custom { urls } => velune_remote::RelayConfig::Custom { urls },
            },
            discovery: match self.discovery {
                BindingDiscoveryConfig::None => velune_remote::DiscoveryConfig::None,
                BindingDiscoveryConfig::DevelopmentPublic => {
                    velune_remote::DiscoveryConfig::DevelopmentPublic
                }
                BindingDiscoveryConfig::CustomPkarr { url } => {
                    velune_remote::DiscoveryConfig::CustomPkarr { url }
                }
            },
            path_policy: match self.path_policy {
                BindingRemotePathPolicy::Automatic => velune_remote::PathPolicy::Automatic,
                BindingRemotePathPolicy::RelayOnly => velune_remote::PathPolicy::RelayOnly,
            },
            bind_addresses: parse_addresses(self.bind_addresses)?,
            request_timeout: Duration::from_millis(self.request_timeout_ms),
        })
    }
}
pub(crate) struct ServiceOwner {
    service: Arc<RemoteAccessService>,
    runtime: Arc<RemoteRuntime>,
    #[cfg(feature = "local-runtime")]
    identity_file: String,
    #[cfg(feature = "local-runtime")]
    trust_file: String,
    #[cfg(feature = "local-runtime")]
    device_name: String,
    config: Mutex<Option<EndpointConfig>>,
}
impl ServiceOwner {
    fn bind(
        options: BindingRemoteOptions,
        runtime: Arc<RemoteRuntime>,
        handler: Option<Arc<dyn velune_remote::Handler>>,
    ) -> Result<Arc<Self>, BindingError> {
        let config = BindingRemoteNetworkOptions::from(&options).config()?;
        let identity = DeviceIdentity::load_or_create(
            Path::new(&options.identity_file),
            options.device_name.clone(),
        )?;
        let trust = Arc::new(FileTrustStore::open(Path::new(&options.trust_file))?);
        let service = Arc::new(runtime.block_on(RemoteAccessService::bind(
            identity,
            config.clone(),
            trust,
            handler,
        ))?);
        Ok(Arc::new(Self {
            runtime,
            service,
            #[cfg(feature = "local-runtime")]
            identity_file: options.identity_file,
            #[cfg(feature = "local-runtime")]
            trust_file: options.trust_file,
            #[cfg(feature = "local-runtime")]
            device_name: options.device_name,
            config: Mutex::new(Some(config)),
        }))
    }
    #[cfg(feature = "local-runtime")]
    fn reuse(&self, options: &BindingRemoteOptions) -> Result<(), BindingError> {
        if self.identity_file != options.identity_file
            || self.trust_file != options.trust_file
            || self.device_name != options.device_name
        {
            return Err(BindingError::Invalid {
                detail: "同一应用执行核心只能装配一份设备身份与可信设备存储".into(),
            });
        }
        self.apply_config(BindingRemoteNetworkOptions::from(options).config()?)
    }
    fn apply_config(&self, config: EndpointConfig) -> Result<(), BindingError> {
        let mut applied = self.config.lock().map_err(|_| BindingError::Unavailable)?;
        if applied.as_ref() == Some(&config) && self.service.descriptor().is_ok() {
            return Ok(());
        }
        *applied = None;
        self.runtime
            .block_on(self.service.replace_network(config.clone()))?;
        *applied = Some(config);
        Ok(())
    }
    pub(crate) fn close(&self) -> Result<(), BindingError> {
        self.runtime
            .block_on(self.service.close())
            .map_err(Into::into)
    }
}

#[derive(uniffi::Object)]
pub struct VeluneRemoteEndpoint {
    owner: Arc<ServiceOwner>,
}
#[uniffi::export]
impl VeluneRemoteEndpoint {
    /// Local applications cache one service/runtime for their actual business owner, not a window.
    #[uniffi::constructor]
    pub fn open(
        options: BindingRemoteOptions,
        application: Option<Arc<VeluneApplication>>,
    ) -> Result<Arc<Self>, BindingError> {
        for path in [&options.identity_file, &options.trust_file] {
            if !Path::new(path).is_absolute() {
                return Err(BindingError::Invalid {
                    detail: "设备身份和可信设备文件必须使用绝对路径".into(),
                });
            }
        }
        #[cfg(feature = "local-runtime")]
        if let Some(application) = application.as_ref() {
            let local = match application.lock()?.as_ref().ok_or(BindingError::Closed)? {
                Backend::Local(service) => service.clone(),
                _ => {
                    return Err(BindingError::Unsupported {
                        detail: "远控入口需要本设备的执行核心".into(),
                    });
                }
            };
            let mut cached = application
                .remote_service
                .lock()
                .map_err(|_| BindingError::Unavailable)?;
            if let Some(owner) = cached.as_ref() {
                owner.reuse(&options)?;
                return Ok(Arc::new(Self {
                    owner: owner.clone(),
                }));
            }
            let runtime = RemoteRuntime::new()?;
            let handler = Arc::new(ApplicationHandler {
                application: local,
                _runtime: runtime.clone(),
            });
            let owner = ServiceOwner::bind(options, runtime, Some(handler))?;
            *cached = Some(owner.clone());
            return Ok(Arc::new(Self { owner }));
        }
        if application.is_some() {
            return Err(BindingError::Unsupported {
                detail: "此构建未链接本机执行核心".into(),
            });
        }
        let owner = ServiceOwner::bind(options, RemoteRuntime::new()?, None)?;
        Ok(Arc::new(Self { owner }))
    }
    /// Platform network and foreground notifications can prompt native path recovery.
    pub fn network_change(&self) {
        self.owner
            .runtime
            .block_on(self.owner.service.network_change());
    }
    /// A returned sequence confirms notification only, not successful path recovery.
    pub fn report_network_change(
        &self,
        snapshot: BindingPlatformNetworkSnapshot,
    ) -> Result<u64, BindingError> {
        Ok(self.owner.runtime.block_on(
            self.owner
                .service
                .report_network_change(snapshot.try_into()?),
        )?)
    }

    pub fn descriptor(&self) -> Result<BindingRemoteEndpointDescriptor, BindingError> {
        Ok(self.owner.service.descriptor()?.into())
    }
    pub fn create_invitation(
        &self,
        lifetime_seconds: u64,
    ) -> Result<BindingRemoteInvitation, BindingError> {
        let invitation = self
            .owner
            .service
            .create_invitation(Duration::from_secs(lifetime_seconds))?;
        Ok(BindingRemoteInvitation {
            encoded: invitation.encode()?,
            expires_at_unix_seconds: invitation.expires_at_unix_seconds,
            endpoint: invitation.endpoint.into(),
        })
    }
    pub fn pair(
        &self,
        invitation: String,
        view_id: String,
    ) -> Result<Arc<VeluneApplication>, BindingError> {
        let invitation = velune_remote::Invitation::decode(&invitation)?;
        let client = self
            .owner
            .runtime
            .block_on(self.owner.service.pair(&invitation))?;
        self.application(client, view_id)
    }
    pub fn connect(
        &self,
        descriptor: BindingRemoteEndpointDescriptor,
        view_id: String,
    ) -> Result<Arc<VeluneApplication>, BindingError> {
        let descriptor = descriptor.try_into()?;
        let client = self
            .owner
            .runtime
            .block_on(self.owner.service.connect(&descriptor))?;
        self.application(client, view_id)
    }
    pub fn trusted_devices(&self) -> Result<Vec<BindingTrustedDevice>, BindingError> {
        Ok(self
            .owner
            .service
            .trusted_devices()?
            .into_iter()
            .map(|device| BindingTrustedDevice {
                id: device.id,
                name: device.name,
            })
            .collect())
    }
    pub fn revoke(&self, device_id: String) -> Result<(), BindingError> {
        self.owner.service.revoke(&device_id).map_err(Into::into)
    }
    /// Stop being controlled without closing outgoing control or replacing the business incarnation.
    pub fn set_control_ingress(&self, enabled: bool) -> Result<(), BindingError> {
        self.owner
            .service
            .set_control_ingress(enabled)
            .map_err(Into::into)
    }
    /// Fully retire one network generation before binding the next, preserving accepted requests.
    pub fn replace_network(
        &self,
        options: BindingRemoteNetworkOptions,
    ) -> Result<(), BindingError> {
        self.owner.apply_config(options.config()?)
    }
    /// Final service shutdown waits for accepted completion bookkeeping; ingress uses its own toggle.
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.owner.close()
    }
}
impl VeluneRemoteEndpoint {
    fn application(
        &self,
        client: RemoteClient,
        view_id: String,
    ) -> Result<Arc<VeluneApplication>, BindingError> {
        if view_id.trim().is_empty() || view_id.len() > 128 {
            client.close();
            return Err(BindingError::Invalid {
                detail: "客户端视图标识必须为 1 至 128 字节".into(),
            });
        }
        let descriptor = client.descriptor().clone();
        let remote = Arc::new(RemoteApplication {
            owner: self.owner.clone(),
            view: ClientViewId(view_id),
            state: Mutex::new(RemoteState {
                client: Arc::new(client),
                descriptor,
                expected: None,
                observed: None,
                user_epoch: 0,
                connection_epoch: 0,
                closed: false,
            }),
        });
        let initial = remote.request_reply(Command::List)?;
        remote.state()?.expected = Some(initial.revision);
        Ok(Arc::new(VeluneApplication {
            application: Mutex::new(Some(Backend::Remote(remote))),
            diagnostics: diagnostics::Diagnostics::silent(),
            #[cfg(feature = "local-runtime")]
            remote_service: Mutex::new(None),
        }))
    }
}

pub(crate) struct RemoteApplication {
    owner: Arc<ServiceOwner>,
    view: ClientViewId,
    state: Mutex<RemoteState>,
}
struct RemoteState {
    client: Arc<RemoteClient>,
    descriptor: EndpointDescriptor,
    expected: Option<Revision>,
    observed: Option<Revision>,
    // Only explicit editor changes advance this epoch, so old replies cannot replace a new form's base.
    user_epoch: u64,
    connection_epoch: u64,
    closed: bool,
}
struct RequestContext {
    client: Arc<RemoteClient>,
    execution: ExecutionContext,
    user_epoch: u64,
    connection_epoch: u64,
}
impl RemoteApplication {
    fn state(&self) -> Result<MutexGuard<'_, RemoteState>, BindingError> {
        self.state.lock().map_err(|_| BindingError::Unavailable)
    }
    fn context(&self) -> Result<RequestContext, BindingError> {
        let state = self.state()?;
        if state.closed {
            return Err(BindingError::Closed);
        }
        Ok(RequestContext {
            client: state.client.clone(),
            execution: ExecutionContext {
                expected_revision: state.expected,
            },
            user_epoch: state.user_epoch,
            connection_epoch: state.connection_epoch,
        })
    }
    fn observe(
        &self,
        context: &RequestContext,
        revision: Revision,
        mutation: bool,
    ) -> Result<(), BindingError> {
        let mut state = self.state()?;
        if !state.closed && state.connection_epoch == context.connection_epoch {
            if state.observed.is_none_or(|current| revision.0 > current.0) {
                state.observed = Some(revision);
            }
            if mutation
                && state.user_epoch == context.user_epoch
                && state.expected.is_none_or(|current| revision.0 > current.0)
            {
                state.expected = Some(revision);
            }
        }
        Ok(())
    }
    pub(crate) fn request(&self, command: Command) -> Result<Response, BindingError> {
        self.request_reply(command).map(|reply| reply.response)
    }
    fn request_reply(&self, command: Command) -> Result<ApplicationReply, BindingError> {
        let mutation = command.changes_shared_state();
        let context = self.context()?;
        match self.owner.runtime.block_on(context.client.call(
            self.view.clone(),
            context.execution.clone(),
            command,
        )) {
            Ok(reply) => {
                self.observe(&context, reply.revision, mutation)?;
                Ok(reply)
            }
            Err(error) => match error.failure {
                velune_remote::CallFailure::Business(failure) => {
                    if let Some(revision) = failure.current_revision {
                        self.observe(&context, revision, false)?;
                    }
                    Err((*failure).into())
                }
                velune_remote::CallFailure::Transport(failure) => {
                    Err(BindingError::RemoteRequest {
                        kind: remote_failure_kind(failure),
                        detail: failure.to_string(),
                        code: failure.code().into(),
                        request_id: error.request_id.map(Into::into),
                    })
                }
                velune_remote::CallFailure::ResultUnavailable => Err(BindingError::RemoteRequest {
                    kind: BindingFailureKind::OutcomeUnknown,
                    detail: "请求结果已不可用；不得自动重发".into(),
                    code: "result_unavailable".into(),
                    request_id: error.request_id.map(Into::into),
                }),
            },
        }
    }
    pub(crate) fn close(&self) -> Result<(), BindingError> {
        let mut state = self.state()?;
        state.closed = true;
        state.client.close();
        Ok(())
    }
}
impl VeluneApplication {
    fn remote_backend(&self) -> Result<Arc<RemoteApplication>, BindingError> {
        let state = self.lock()?;
        match state.as_ref().ok_or(BindingError::Closed)? {
            Backend::Remote(remote) => Ok(remote.clone()),
            #[cfg(feature = "local-runtime")]
            Backend::Local(_) => Err(BindingError::Unsupported {
                detail: "此操作需要远控连接".into(),
            }),
        }
    }
}

#[uniffi::export]
impl VeluneApplication {
    /// A form must retain the revision of the same configuration it displays.
    pub fn list_with_revision(&self) -> Result<BindingVersionedConfiguration, BindingError> {
        let reply = self.remote_backend()?.request_reply(Command::List)?;
        let Response::List(configuration) = reply.response else {
            return Err(BindingError::Contract {
                detail: "应用返回了不匹配的配置结果".into(),
            });
        };
        Ok(BindingVersionedConfiguration {
            configuration: convert(configuration)?,
            revision: reply.revision.0,
        })
    }
    /// This observation does not automatically replace an editor's write base.
    pub fn observed_revision(&self) -> Result<Option<u64>, BindingError> {
        let state = self.lock()?;
        match state.as_ref().ok_or(BindingError::Closed)? {
            Backend::Remote(remote) => Ok(remote.state()?.observed.map(|revision| revision.0)),
            #[cfg(feature = "local-runtime")]
            Backend::Local(_) => Ok(None),
        }
    }
    /// Retain the form's revision; conflicts must never trigger an automatic resend.
    pub fn expect_revision(&self, revision: u64) -> Result<(), BindingError> {
        let remote = self.remote_backend()?;
        let mut state = remote.state()?;
        if state.closed {
            return Err(BindingError::Closed);
        }
        state.expected = Some(Revision(revision));
        state.user_epoch = state.user_epoch.wrapping_add(1);
        Ok(())
    }
    pub fn observe_paths(&self) -> Result<Arc<VelunePathObserver>, BindingError> {
        let remote = self.remote_backend()?;
        Ok(VelunePathObserver::new(
            remote.owner.runtime.clone(),
            remote.context()?.client.observe_paths(),
        ))
    }
    pub fn connection_status(&self) -> Result<BindingConnectionStatus, BindingError> {
        let remote = self.remote_backend()?;
        convert(remote.context()?.client.connection_status())
    }
    pub fn remote_descriptor(&self) -> Result<BindingRemoteEndpointDescriptor, BindingError> {
        Ok(self.remote_backend()?.state()?.descriptor.clone().into())
    }
    /// Reconnection preserves numbering and cannot replace the execution device or resurrect a closed object.
    pub fn reconnect(
        &self,
        descriptor: BindingRemoteEndpointDescriptor,
    ) -> Result<(), BindingError> {
        let remote = self.remote_backend()?;
        let descriptor: EndpointDescriptor = descriptor.try_into()?;
        let context = remote.context()?;
        if descriptor.endpoint_id != remote.state()?.descriptor.endpoint_id {
            return Err(BindingError::Invalid {
                detail: "重连不能更换执行设备，请建立独立连接".into(),
            });
        }
        let client = remote.owner.runtime.block_on(
            remote
                .owner
                .service
                .connect_with_session(&descriptor, context.client.session()),
        )?;
        let descriptor = client.descriptor().clone();
        let mut state = remote.state()?;
        if state.closed {
            client.close();
            return Err(BindingError::Closed);
        }
        if state.connection_epoch != context.connection_epoch {
            client.close();
            return Err(BindingError::Unavailable);
        }
        if descriptor.incarnation != state.descriptor.incarnation {
            state.expected = None;
            state.user_epoch = state.user_epoch.wrapping_add(1);
        }
        state.client.close();
        state.client = Arc::new(client);
        state.descriptor = descriptor;
        state.observed = None;
        state.connection_epoch = state.connection_epoch.wrapping_add(1);
        Ok(())
    }
    /// Allocate before submission so the ID survives a lost reply.
    pub fn next_request_id(&self) -> Result<BindingRemoteRequestId, BindingError> {
        Ok(self
            .remote_backend()?
            .context()?
            .client
            .next_request_id()?
            .into())
    }
    /// A Running response is queried with the same ID; submission never holds the application lock over I/O.
    pub fn submit_request(
        &self,
        request_id: BindingRemoteRequestId,
        command: BindingApplicationCommand,
    ) -> Result<BindingRemoteRequestStatus, BindingError> {
        let remote = self.remote_backend()?;
        let context = remote.context()?;
        let status = remote.owner.runtime.block_on(context.client.submit(
            request_id.into(),
            remote.view.clone(),
            context.execution.clone(),
            command.try_into()?,
        ))?;
        if let RequestStatus::Completed(Ok(reply)) = &status {
            remote.observe(&context, reply.revision, false)?;
        }
        status.try_into()
    }
    /// Querying an existing request never resubmits it and does not wait on another business call.
    pub fn query_request(
        &self,
        request_id: BindingRemoteRequestId,
    ) -> Result<BindingRemoteRequestStatus, BindingError> {
        let remote = self.remote_backend()?;
        let context = remote.context()?;
        let status = remote
            .owner
            .runtime
            .block_on(context.client.query(request_id.into()))?;
        if let RequestStatus::Completed(Ok(reply)) = &status {
            remote.observe(&context, reply.revision, false)?;
        }
        status.try_into()
    }
}
