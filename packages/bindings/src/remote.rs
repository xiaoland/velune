//! Remote assembly owns transport resources; the application owner keeps business work alive.
use crate::*;
use std::{path::Path, sync::Arc, time::Duration};
use velune_application_api::{ApplicationReply, ExecutionContext, Revision};
use velune_remote::{
    Controller, DeviceIdentity, DeviceStore, EndpointConfig, EndpointDescriptor,
    RemoteAccessService, RemoteOperation, RequestId, RequestStatus,
};

#[derive(Clone, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum BindingRelayConfig {
    Disabled,
    /// Development infrastructure is an explicit opt-in, without a production SLA.
    DevelopmentPublic,
    Custom {
        urls: Vec<String>,
    },
}
#[derive(Clone, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum BindingDiscoveryConfig {
    None,
    DevelopmentPublic,
    CustomPkarr { urls: Vec<String> },
}
#[derive(Clone, serde::Serialize, serde::Deserialize, uniffi::Enum)]
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
    pub state_file: String,
    pub device_name: String,
    pub relay: BindingRelayConfig,
    pub discovery: BindingDiscoveryConfig,
    pub path_policy: BindingRemotePathPolicy,
    pub bind_addresses: Vec<String>,
    pub request_timeout_ms: u64,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingRemoteEndpointDescriptor {
    pub version: u16,
    pub endpoint_id: String,
    pub name: String,
    pub incarnation: String,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<String>,
    pub lookup_urls: Vec<String>,
}
impl From<EndpointDescriptor> for BindingRemoteEndpointDescriptor {
    fn from(value: EndpointDescriptor) -> Self {
        Self {
            version: value.version,
            endpoint_id: value.endpoint_id,
            name: value.name,
            incarnation: value.incarnation,
            relay_urls: value.relay_urls,
            lookup_urls: value.lookup_urls,
            direct_addresses: value
                .direct_addresses
                .into_iter()
                .map(|address| address.to_string())
                .collect(),
        }
    }
}
/// A one-time pairing credential, exposed only for the user's explicit transfer.
#[derive(Clone, uniffi::Record)]
pub struct BindingRemoteInvitation {
    pub encoded: String,
    pub expires_at_unix_seconds: u64,
    pub issuer: String,
}
#[derive(Debug, Clone, serde::Deserialize, uniffi::Record)]
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
        code: String,
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
                code: error.code().into(),
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
        RemoteError::ConfigurationConflict => BindingFailureKind::Conflict,
        RemoteError::Timeout | RemoteError::ServerRestarted => BindingFailureKind::OutcomeUnknown,
        RemoteError::PairingUnavailable
        | RemoteError::PendingRequests
        | RemoteError::OperationRequired
        | RemoteError::OperationUsed
        | RemoteError::Configuration
        | RemoteError::InvalidInvitation
        | RemoteError::ExpiredInvitation
        | RemoteError::InvalidRequestId
        | RemoteError::RequestIdConflict => BindingFailureKind::Invalid,
        RemoteError::IdentityStorage | RemoteError::StateStorage => BindingFailureKind::Io,
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
    pub local_discovery: bool,
    pub relay: BindingRelayConfig,
    pub discovery: BindingDiscoveryConfig,
    pub path_policy: BindingRemotePathPolicy,
    pub bind_addresses: Vec<String>,
    pub request_timeout_ms: u64,
}
impl From<&BindingRemoteOptions> for BindingRemoteNetworkOptions {
    fn from(options: &BindingRemoteOptions) -> Self {
        Self {
            local_discovery: true,
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
            local_discovery: self.local_discovery,
            relay_credentials: Vec::new(),
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
                BindingDiscoveryConfig::CustomPkarr { urls } => {
                    velune_remote::DiscoveryConfig::CustomPkarr { urls }
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
    state_file: String,
    #[cfg(feature = "local-runtime")]
    device_name: String,
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
        let store = Arc::new(DeviceStore::open(Path::new(&options.state_file))?);
        let service = Arc::new(runtime.block_on(RemoteAccessService::bind(
            identity,
            config.clone(),
            store,
            handler,
        ))?);
        Ok(Arc::new(Self {
            runtime,
            service,
            #[cfg(feature = "local-runtime")]
            identity_file: options.identity_file,
            #[cfg(feature = "local-runtime")]
            state_file: options.state_file,
            #[cfg(feature = "local-runtime")]
            device_name: options.device_name,
        }))
    }
    #[cfg(feature = "local-runtime")]
    fn reuse(&self, options: &BindingRemoteOptions) -> Result<(), BindingError> {
        if self.identity_file != options.identity_file
            || self.state_file != options.state_file
            || self.device_name != options.device_name
        {
            return Err(BindingError::Invalid {
                detail: "同一应用执行核心只能装配一份设备身份与远控状态存储".into(),
            });
        }
        if self.service.descriptor().is_err() {
            self.runtime.block_on(self.service.retry_network())?;
        }
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
        velune_remote::validate_device_storage_paths(
            Path::new(&options.identity_file),
            Path::new(&options.state_file),
        )?;
        for path in [&options.identity_file, &options.state_file] {
            if !Path::new(path).is_absolute() {
                return Err(BindingError::Invalid {
                    detail: "设备身份和远控状态文件必须使用绝对路径".into(),
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
            issuer: invitation.issuer,
        })
    }
    pub fn begin_pairing(&self, encoded: String) -> Result<BindingPairingStatus, BindingError> {
        let offer = velune_remote::PairingOffer::decode(&encoded)?;
        convert(
            self.owner
                .runtime
                .block_on(self.owner.service.begin_pairing(&offer))?,
        )
    }
    pub fn pairing_status(&self, peer_id: String) -> Result<BindingPairingStatus, BindingError> {
        convert(
            self.owner
                .runtime
                .block_on(self.owner.service.pairing_status(&peer_id))?,
        )
    }
    pub fn cancel_pairing(&self, peer_id: String) -> Result<BindingPairingStatus, BindingError> {
        convert(
            self.owner
                .runtime
                .block_on(self.owner.service.cancel_pairing(&peer_id))?,
        )
    }
    pub fn approve_pairing(
        &self,
        attempt_id: String,
        expected_peer_id: String,
    ) -> Result<BindingPairingStatus, BindingError> {
        convert(
            self.owner
                .service
                .approve_pairing(&attempt_id, &expected_peer_id)?,
        )
    }
    pub fn reject_pairing(
        &self,
        attempt_id: String,
        expected_peer_id: String,
    ) -> Result<BindingPairingStatus, BindingError> {
        convert(
            self.owner
                .service
                .reject_pairing(&attempt_id, &expected_peer_id)?,
        )
    }
    pub fn connect(
        &self,
        peer_id: String,
        view_id: String,
    ) -> Result<Arc<VeluneApplication>, BindingError> {
        let controller = self.owner.service.controller(&peer_id)?;
        self.owner.runtime.block_on(controller.connect())?;
        self.application(controller, view_id)
    }
    pub fn set_controller_activity(
        &self,
        peer_id: String,
        selected: bool,
        foreground: bool,
    ) -> Result<(), BindingError> {
        self.owner
            .service
            .controller(&peer_id)?
            .set_activity(selected, foreground)
            .map_err(Into::into)
    }
    pub fn controller_status(
        &self,
        peer_id: String,
    ) -> Result<BindingControllerStatus, BindingError> {
        convert(self.owner.service.controller(&peer_id)?.status()?)
    }
    pub fn refresh_local_discovery(&self) -> Result<u64, BindingError> {
        Ok(self
            .owner
            .runtime
            .block_on(self.owner.service.refresh_local_discovery())?)
    }
    pub fn read_state(&self) -> Result<BindingRemoteServiceSnapshot, BindingError> {
        convert(self.owner.service.read_state()?)
    }
    pub fn observe_state(&self) -> Arc<VeluneRemoteStateObserver> {
        Arc::new(VeluneRemoteStateObserver {
            runtime: self.owner.runtime.clone(),
            observer: tokio::sync::Mutex::new(self.owner.service.observe_state()),
        })
    }
    /// Persist reset notification before the platform removes its obsolete ordinary-state files.
    pub fn record_legacy_state_reset(&self) -> Result<(), BindingError> {
        self.owner
            .service
            .record_legacy_state_reset()
            .map_err(Into::into)
    }
    pub fn acknowledge_reset(&self, notice_id: String) -> Result<(), BindingError> {
        self.owner
            .service
            .acknowledge_reset(&notice_id)
            .map_err(Into::into)
    }
    pub fn query_request(
        &self,
        peer_id: String,
        request_id: BindingRemoteRequestId,
    ) -> Result<BindingRemoteRequestStatus, BindingError> {
        self.owner
            .runtime
            .block_on(
                self.owner
                    .service
                    .query_request(&peer_id, request_id.into()),
            )?
            .try_into()
    }
    pub fn acknowledge_request(
        &self,
        peer_id: String,
        request_id: BindingRemoteRequestId,
    ) -> Result<(), BindingError> {
        self.owner
            .service
            .acknowledge_request(&peer_id, &request_id.into())
            .map_err(Into::into)
    }
    pub fn update_contact_locator(
        &self,
        expected_peer_id: String,
        update: BindingLocatorUpdate,
    ) -> Result<(), BindingError> {
        let locator: velune_remote::DeviceLocator = convert(update.locator)?;
        self.owner
            .service
            .update_contact_locator(
                &expected_peer_id,
                velune_remote::LocatorUpdate {
                    peer_id: update.peer_id,
                    locator,
                },
            )
            .map_err(Into::into)
    }
    pub fn forget_device(&self, peer_id: String) -> Result<(), BindingError> {
        self.owner
            .service
            .forget_device(&peer_id)
            .map_err(Into::into)
    }
    pub fn disconnect(&self, peer_id: String) -> Result<(), BindingError> {
        self.owner
            .service
            .controller(&peer_id)?
            .disconnect()
            .map_err(Into::into)
    }
    pub fn revoke_control(&self, peer_id: String) -> Result<(), BindingError> {
        self.owner
            .service
            .revoke_control(&peer_id)
            .map_err(Into::into)
    }
    /// Stop being controlled without closing outgoing control or replacing the business incarnation.
    pub fn set_control_ingress(&self, enabled: bool) -> Result<(), BindingError> {
        self.owner
            .service
            .set_control_ingress(enabled)
            .map_err(Into::into)
    }
    pub fn network_state(&self) -> Result<BindingNetworkState, BindingError> {
        convert(self.owner.service.network_state()?)
    }
    pub fn apply_network(
        &self,
        update: BindingNetworkSettingsUpdate,
    ) -> Result<BindingNetworkState, BindingError> {
        let update = velune_remote::NetworkSettingsUpdate {
            expected_revision: update.expected_revision,
            config: update.network.config()?,
            credentials: convert(update.credentials)?,
            restore_lookup_urls: update.restore_lookup_urls,
        };
        convert(
            self.owner
                .runtime
                .block_on(self.owner.service.apply_network(update))?,
        )
    }
    pub fn retry_network(&self) -> Result<BindingNetworkState, BindingError> {
        convert(
            self.owner
                .runtime
                .block_on(self.owner.service.retry_network())?,
        )
    }
    /// Final service shutdown waits for accepted completion bookkeeping; ingress uses its own toggle.
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.owner.close()
    }
}
impl VeluneRemoteEndpoint {
    fn application(
        &self,
        controller: Arc<Controller>,
        view_id: String,
    ) -> Result<Arc<VeluneApplication>, BindingError> {
        if view_id.trim().is_empty() || view_id.len() > 128 {
            return Err(BindingError::Invalid {
                detail: "客户端视图标识必须为 1 至 128 字节".into(),
            });
        }
        let info = controller.connection_info()?;
        let remote = Arc::new(RemoteApplication {
            owner: self.owner.clone(),
            controller,
            view: ClientViewId(view_id),
            state: Mutex::new(RemoteState {
                descriptor: info.descriptor,
                expected: None,
                observed: None,
                user_epoch: 0,
                connection_epoch: info.epoch,
                closed: false,
            }),
        });
        let initial = remote.request_reply(Command::List, None)?;
        remote.state()?.expected = Some(initial.revision);
        Ok(remote_proxy(Backend::Remote(remote)))
    }
}
fn remote_proxy(backend: Backend) -> Arc<VeluneApplication> {
    Arc::new(VeluneApplication {
        application: Mutex::new(Some(backend)),
        diagnostics: diagnostics::Diagnostics::silent(),
        #[cfg(feature = "local-runtime")]
        remote_service: Mutex::new(None),
    })
}
#[derive(uniffi::Object)]
pub struct VeluneRemoteStateObserver {
    runtime: Arc<RemoteRuntime>,
    observer: tokio::sync::Mutex<velune_remote::RemoteStateObserver>,
}
#[uniffi::export]
impl VeluneRemoteStateObserver {
    pub fn next(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<BindingRemoteServiceSnapshot>, BindingError> {
        self.runtime
            .block_on(async {
                self.observer
                    .lock()
                    .await
                    .next(Duration::from_millis(timeout_ms))
                    .await
            })?
            .map(convert)
            .transpose()
    }
}
#[derive(uniffi::Object)]
pub struct VeluneRemoteOperation {
    operation: Arc<RemoteOperation>,
    application: Arc<VeluneApplication>,
}
#[uniffi::export]
impl VeluneRemoteOperation {
    pub fn request_id(&self) -> BindingRemoteRequestId {
        self.operation.request_id().into()
    }
    pub fn application(&self) -> Arc<VeluneApplication> {
        self.application.clone()
    }
    pub fn acknowledge(&self) -> Result<(), BindingError> {
        self.operation.acknowledge().map_err(Into::into)
    }
}

pub(crate) struct RemoteApplication {
    owner: Arc<ServiceOwner>,
    view: ClientViewId,
    controller: Arc<Controller>,
    state: Mutex<RemoteState>,
}
struct RemoteState {
    descriptor: EndpointDescriptor,
    expected: Option<Revision>,
    observed: Option<Revision>,
    // Only explicit editor changes advance this epoch, so old replies cannot replace a new form's base.
    user_epoch: u64,
    connection_epoch: u64,
    closed: bool,
}
struct RequestContext {
    execution: ExecutionContext,
    user_epoch: u64,
    connection_epoch: u64,
}
impl RemoteApplication {
    fn state(&self) -> Result<MutexGuard<'_, RemoteState>, BindingError> {
        self.state.lock().map_err(|_| BindingError::Unavailable)
    }
    fn context(&self) -> Result<RequestContext, BindingError> {
        let info = self.controller.connection_info()?;
        let mut state = self.state()?;
        if state.closed {
            return Err(BindingError::Closed);
        }
        if state.descriptor.incarnation != info.descriptor.incarnation {
            state.expected = None;
            state.user_epoch = state.user_epoch.wrapping_add(1);
        }
        state.descriptor = info.descriptor;
        state.connection_epoch = info.epoch;
        Ok(RequestContext {
            execution: ExecutionContext {
                expected_revision: state.expected,
            },
            user_epoch: state.user_epoch,
            connection_epoch: info.epoch,
        })
    }
    fn observe(
        &self,
        context: &RequestContext,
        revision: Revision,
        mutation: bool,
    ) -> Result<(), BindingError> {
        let current_epoch = self
            .controller
            .connection_info()
            .ok()
            .map(|info| info.epoch);
        let mut state = self.state()?;
        if !state.closed
            && current_epoch == Some(context.connection_epoch)
            && state.connection_epoch == context.connection_epoch
        {
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
    pub(crate) fn request(
        &self,
        command: Command,
        operation: Option<&RemoteOperation>,
    ) -> Result<Response, BindingError> {
        self.request_reply(command, operation)
            .map(|reply| reply.response)
    }
    fn request_reply(
        &self,
        command: Command,
        operation: Option<&RemoteOperation>,
    ) -> Result<ApplicationReply, BindingError> {
        let mutation = command.changes_shared_state();
        let context = self.context()?;
        let result = self.owner.runtime.block_on(async {
            match operation {
                Some(operation) => {
                    operation
                        .execute(self.view.clone(), context.execution.clone(), command)
                        .await
                }
                None => {
                    self.controller
                        .observe(self.view.clone(), context.execution.clone(), command)
                        .await
                }
            }
        });
        match result {
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
        self.state()?.closed = true;
        Ok(())
    }
}
impl VeluneApplication {
    fn remote_backend(&self) -> Result<Arc<RemoteApplication>, BindingError> {
        let state = self.lock()?;
        match state.as_ref().ok_or(BindingError::Closed)? {
            Backend::Remote(remote) | Backend::ScopedRemote { remote, .. } => Ok(remote.clone()),
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
        let reply = self.remote_backend()?.request_reply(Command::List, None)?;
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
            Backend::Remote(remote) | Backend::ScopedRemote { remote, .. } => {
                Ok(remote.state()?.observed.map(|revision| revision.0))
            }
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
            remote.controller.observe_paths()?,
        ))
    }
    pub fn connection_status(&self) -> Result<BindingConnectionStatus, BindingError> {
        let remote = self.remote_backend()?;
        convert(remote.controller.connection_status()?)
    }
    pub fn remote_descriptor(&self) -> Result<BindingRemoteEndpointDescriptor, BindingError> {
        Ok(self.remote_backend()?.state()?.descriptor.clone().into())
    }
    /// Reconnect only to the saved pin and locator; the authenticated Hello supplies the incarnation.
    pub fn reconnect(&self) -> Result<(), BindingError> {
        let remote = self.remote_backend()?;
        remote.owner.runtime.block_on(remote.controller.connect())?;
        remote.context()?;
        Ok(())
    }
    pub fn prepare_remote_operation(&self) -> Result<Arc<VeluneRemoteOperation>, BindingError> {
        let remote = self.remote_backend()?;
        let operation = remote.controller.prepare_operation()?;
        let application = remote_proxy(Backend::ScopedRemote {
            remote,
            operation: operation.clone(),
        });
        Ok(Arc::new(VeluneRemoteOperation {
            operation,
            application,
        }))
    }
    /// Allocate before submission so the ID survives a lost reply.
    pub fn next_request_id(&self) -> Result<BindingRemoteRequestId, BindingError> {
        Ok(self.remote_backend()?.controller.next_request_id()?.into())
    }
    /// A Running response is queried with the same ID; submission never holds the application lock over I/O.
    pub fn submit_request(
        &self,
        request_id: BindingRemoteRequestId,
        command: BindingApplicationCommand,
    ) -> Result<BindingRemoteRequestStatus, BindingError> {
        let remote = self.remote_backend()?;
        let context = remote.context()?;
        let scoped = match self.lock()?.as_ref().ok_or(BindingError::Closed)? {
            Backend::ScopedRemote { operation, .. } => Some(operation.clone()),
            _ => None,
        };
        let request_id: RequestId = request_id.into();
        let command: Command = command.try_into()?;
        let status = remote.owner.runtime.block_on(async {
            if let Some(operation) = scoped {
                if operation.request_id() != request_id {
                    return Err(velune_remote::RemoteError::InvalidRequestId);
                }
                operation
                    .submit(remote.view.clone(), context.execution.clone(), command)
                    .await
            } else {
                remote
                    .controller
                    .submit(
                        request_id,
                        remote.view.clone(),
                        context.execution.clone(),
                        command,
                    )
                    .await
            }
        })?;
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
            .block_on(remote.controller.query(request_id.into()))?;
        if let RequestStatus::Completed(Ok(reply)) = &status {
            remote.observe(&context, reply.revision, false)?;
        }
        status.try_into()
    }
}
