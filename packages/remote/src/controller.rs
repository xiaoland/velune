//! One per-peer connection intent, recovery task and write-before-send gate.
use crate::*;
mod recovery;
pub use recovery::{ControllerPhase, ControllerStatus};
use serde::Serialize;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use velune_application_api::{
    ApplicationReply, ClientViewId, Command, DeliveryClass, ExecutionContext,
};

pub struct Controller {
    service: Weak<RemoteAccessService>,
    peer: String,
    session: ClientSession,
    connection: Mutex<ControllerConnection>,
    dial: tokio::sync::Mutex<()>,
    recovery: recovery::Recovery,
}
struct ControllerConnection {
    client: Option<Arc<RemoteClient>>,
    epoch: u64,
}
#[derive(Clone)]
pub struct ControllerConnectionInfo {
    pub descriptor: EndpointDescriptor,
    pub epoch: u64,
}
impl Controller {
    fn owner(&self) -> Result<Arc<RemoteAccessService>> {
        self.service.upgrade().ok_or(RemoteError::Closed)
    }
    pub fn peer_id(&self) -> &str {
        &self.peer
    }
    pub(crate) fn client(&self) -> Result<Arc<RemoteClient>> {
        self.connection
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .client
            .clone()
            .filter(|client| !client.connection_status().closed)
            .ok_or(RemoteError::Closed)
    }
    pub fn connection_info(&self) -> Result<ControllerConnectionInfo> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        let client = connection
            .client
            .as_ref()
            .filter(|client| !client.connection_status().closed)
            .ok_or(RemoteError::Closed)?;
        Ok(ControllerConnectionInfo {
            descriptor: client.descriptor().clone(),
            epoch: connection.epoch,
        })
    }
    pub fn descriptor(&self) -> Result<EndpointDescriptor> {
        Ok(self.client()?.descriptor().clone())
    }
    pub fn connection_status(&self) -> Result<ConnectionStatus> {
        Ok(self.client()?.connection_status())
    }
    pub fn observe_paths(&self) -> Result<PathObserver> {
        Ok(self.client()?.observe_paths())
    }
    pub fn next_request_id(&self) -> Result<RequestId> {
        self.client()?.next_request_id()
    }
    pub fn prepare_operation(self: &Arc<Self>) -> Result<Arc<RemoteOperation>> {
        Ok(Arc::new(RemoteOperation {
            controller: self.clone(),
            id: self.next_request_id()?,
            used: Mutex::new(false),
        }))
    }
    /// The entry for a command without an explicit scope. An effect is recorded
    /// before it is sent and acknowledged once its outcome is definite; an
    /// unknown outcome stays pending for the user and is never resent.
    pub async fn request(
        &self,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> std::result::Result<ApplicationReply, CallError> {
        if command.delivery_class() == DeliveryClass::Observation {
            return self.observe(view, execution, command).await;
        }
        let id = self
            .next_request_id()
            .map_err(|error| call_error(None, error))?;
        let result = self.call(id.clone(), view, execution, command).await;
        let definite = match &result {
            Ok(_) => true,
            Err(error) => matches!(error.failure, CallFailure::Business(_)),
        };
        if definite {
            // A failed acknowledgement leaves the record pending, shown as
            // unresolved; the caller still receives the outcome.
            let _ = self.acknowledge(&id);
        }
        result
    }
    async fn observe(
        &self,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> std::result::Result<ApplicationReply, CallError> {
        self.client()
            .map_err(|error| call_error(None, error))?
            .call(view, execution, command)
            .await
    }
    /// This is the sole explicit submission gate; no body is ever persisted for replay.
    pub async fn submit(
        &self,
        id: RequestId,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> Result<RequestStatus> {
        let client = self.client()?;
        id.validate(&client.descriptor().incarnation)?;
        let effect = command.delivery_class() == DeliveryClass::Effect;
        if effect {
            let service = self.owner()?;
            let _access = service.state.access()?;
            let (runtime_id, conversation_id) = operation_targets(&command);
            service.state.store.record_pending(PendingOperation {
                peer_id: self.peer.clone(),
                request_id: id.clone(),
                view: view.clone(),
                execution: execution.clone(),
                kind: command.kind(),
                runtime_id,
                conversation_id,
                status: PendingStatus::Recorded,
            })?;
        }
        let result = client.submit(id.clone(), view, execution, command).await;
        if effect {
            self.remember(&id, &result)?;
        }
        result
    }
    pub async fn query(&self, id: RequestId) -> Result<RequestStatus> {
        let result = self.client()?.query(id.clone()).await;
        self.remember(&id, &result)?;
        result
    }
    fn remember(&self, id: &RequestId, result: &Result<RequestStatus>) -> Result<()> {
        let status = match result {
            Ok(RequestStatus::Running) => PendingStatus::Running,
            Ok(RequestStatus::Completed(Ok(_))) => PendingStatus::Completed,
            Ok(RequestStatus::Completed(Err(_))) => PendingStatus::BusinessFailed,
            Ok(RequestStatus::Unknown) => PendingStatus::Unknown,
            Ok(RequestStatus::ResultExpired) => PendingStatus::ResultExpired,
            Ok(RequestStatus::Failed(_)) | Err(_) => PendingStatus::TransportFailed,
        };
        let status = if matches!(result, Err(RemoteError::ServerRestarted)) {
            PendingStatus::ServerRestarted
        } else {
            status
        };
        let service = self.owner()?;
        let _access = service.state.access()?;
        if service
            .state
            .store
            .snapshot()?
            .pending
            .iter()
            .any(|operation| {
                operation.peer_id == self.peer
                    && operation.request_id == *id
                    && operation.status != status
            })
        {
            service.state.store.update_pending(&self.peer, id, status)?;
        }
        Ok(())
    }
    pub fn acknowledge(&self, id: &RequestId) -> Result<()> {
        let service = self.owner()?;
        let _access = service.state.access()?;
        service.state.store.acknowledge(&self.peer, id)
    }
    async fn call(
        &self,
        id: RequestId,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> std::result::Result<ApplicationReply, CallError> {
        let timeout = self
            .client()
            .map_err(|error| call_error(Some(id.clone()), error))?
            .timeout;
        let deadline = Instant::now() + timeout;
        let mut status = self
            .submit(id.clone(), view, execution, command)
            .await
            .map_err(|error| call_error(Some(id.clone()), error))?;
        loop {
            match status {
                RequestStatus::Completed(result) => {
                    return result.map(|reply| *reply).map_err(|failure| CallError {
                        request_id: Some(id),
                        failure: CallFailure::Business(Box::new(failure)),
                    });
                }
                RequestStatus::Failed(error) => return Err(call_error(Some(id), error)),
                RequestStatus::Unknown | RequestStatus::ResultExpired => {
                    return Err(CallError {
                        request_id: Some(id),
                        failure: CallFailure::ResultUnavailable,
                    });
                }
                RequestStatus::Running => {}
            }
            if Instant::now() >= deadline {
                return Err(call_error(Some(id), RemoteError::Timeout));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
            status = self
                .query(id.clone())
                .await
                .map_err(|error| call_error(Some(id.clone()), error))?;
        }
    }
}
fn call_error(request_id: Option<RequestId>, error: RemoteError) -> CallError {
    CallError {
        request_id,
        failure: CallFailure::Transport(error),
    }
}

/// One effect and its exact receipt, shared by a scoped typed facade. Drop never acknowledges it.
pub struct RemoteOperation {
    controller: Arc<Controller>,
    id: RequestId,
    used: Mutex<bool>,
}
impl RemoteOperation {
    pub fn request_id(&self) -> RequestId {
        self.id.clone()
    }
    pub fn peer_id(&self) -> &str {
        self.controller.peer_id()
    }
    fn claim(&self, command: &Command) -> Result<()> {
        if command.delivery_class() == DeliveryClass::Observation {
            return Ok(());
        }
        let mut used = self.used.lock().map_err(|_| RemoteError::Unavailable)?;
        if *used {
            return Err(RemoteError::OperationUsed);
        }
        *used = true;
        Ok(())
    }
    pub async fn submit(
        &self,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> Result<RequestStatus> {
        self.claim(&command)?;
        self.controller
            .submit(self.id.clone(), view, execution, command)
            .await
    }
    pub async fn execute(
        &self,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> std::result::Result<ApplicationReply, CallError> {
        if command.delivery_class() == DeliveryClass::Observation {
            return self.controller.observe(view, execution, command).await;
        }
        self.claim(&command)
            .map_err(|error| call_error(Some(self.id.clone()), error))?;
        self.controller
            .call(self.id.clone(), view, execution, command)
            .await
    }
    pub fn acknowledge(&self) -> Result<()> {
        self.controller.acknowledge(&self.id)
    }
}

#[derive(Clone, Serialize)]
pub struct RemoteServiceSnapshot {
    pub version: u64,
    pub device_id: String,
    pub control_ingress: bool,
    pub descriptor: Option<EndpointDescriptor>,
    pub contacts: Vec<DeviceContact>,
    pub incoming_grants: Vec<IncomingControlGrant>,
    pub incoming_pairings: Vec<PairingStatus>,
    pub outgoing_pairings: Vec<PairingStatus>,
    pub pending: Vec<PendingOperation>,
    pub controllers: Vec<ControllerStatus>,
    pub network: NetworkState,
    pub reset_notice: Option<ConfigurationResetNotice>,
}
pub struct RemoteStateObserver {
    service: Weak<RemoteAccessService>,
    versions: tokio::sync::watch::Receiver<u64>,
    last: Option<u64>,
}
impl RemoteStateObserver {
    pub async fn next(&mut self, timeout: Duration) -> Result<Option<RemoteServiceSnapshot>> {
        if timeout.is_zero() || timeout > Duration::from_secs(5) {
            return Err(RemoteError::Configuration);
        }
        let service = self.service.upgrade().ok_or(RemoteError::Closed)?;
        let state = service.read_state()?;
        if self.last != Some(state.version) {
            self.last = Some(state.version);
            self.versions.borrow_and_update();
            return Ok(Some(state));
        }
        let _ = tokio::time::timeout(timeout, self.versions.changed()).await;
        let state = service.read_state()?;
        if self.last == Some(state.version) {
            return Ok(None);
        }
        self.last = Some(state.version);
        self.versions.borrow_and_update();
        Ok(Some(state))
    }
}
impl RemoteAccessService {
    pub fn controller(self: &Arc<Self>, peer: &str) -> Result<Arc<Controller>> {
        let access = self.state.access()?;
        if access.closed {
            return Err(RemoteError::Closed);
        }
        self.state.store.contact(peer)?;
        let mut controllers = self
            .controllers
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if let Some(controller) = controllers.get(peer) {
            return Ok(controller.clone());
        }
        let controller = Arc::new(Controller {
            service: Arc::downgrade(self),
            peer: peer.into(),
            session: ClientSession::new()?,
            connection: Mutex::new(ControllerConnection {
                client: None,
                epoch: 0,
            }),
            dial: tokio::sync::Mutex::new(()),
            recovery: recovery::Recovery::new(self.state.store.contact(peer)?.paused),
        });
        controllers.insert(peer.into(), controller.clone());
        Ok(controller)
    }
    pub fn read_state(&self) -> Result<RemoteServiceSnapshot> {
        let mut access = self.state.access()?;
        let (before, changed) = access.pairing.statuses();
        if changed {
            self.state.store.touch();
        }
        let stored = self.state.store.snapshot()?;
        let network = self.network_state()?;
        Ok(RemoteServiceSnapshot {
            version: self.state.store.version(),
            network,
            device_id: self.state.identity.id(),
            control_ingress: access.ingress && !access.closed,
            descriptor: self.descriptor().ok(),
            contacts: stored.contacts,
            incoming_grants: stored.incoming_grants,
            incoming_pairings: before,
            outgoing_pairings: self
                .state
                .outgoing
                .lock()
                .map_err(|_| RemoteError::Unavailable)?
                .values()
                .map(|pairing| pairing.status.clone())
                .collect(),
            pending: stored.pending,
            controllers: self
                .controllers
                .lock()
                .map_err(|_| RemoteError::Unavailable)?
                .values()
                .map(|controller| controller.status())
                .collect::<Result<_>>()?,
            reset_notice: stored.reset_notice,
        })
    }
    pub fn observe_state(self: &Arc<Self>) -> RemoteStateObserver {
        RemoteStateObserver {
            service: Arc::downgrade(self),
            versions: self.state.store.subscribe(),
            last: None,
        }
    }
    pub fn record_legacy_state_reset(&self) -> Result<()> {
        let _access = self.state.access()?;
        self.state.store.record_legacy_reset()
    }
    pub async fn query_request(
        self: &Arc<Self>,
        peer: &str,
        id: RequestId,
    ) -> Result<RequestStatus> {
        let controller = self.controller(peer)?;
        controller.connect().await?;
        controller.query(id).await
    }
    pub fn acknowledge_reset(&self, id: &str) -> Result<()> {
        let _access = self.state.access()?;
        self.state.store.acknowledge_reset(id)
    }
    pub fn acknowledge_request(&self, peer: &str, id: &RequestId) -> Result<()> {
        let _access = self.state.access()?;
        self.state.store.acknowledge(peer, id)
    }
    /// Replace routing hints for the already pinned public key without changing either grant.
    pub fn update_contact_locator(&self, expected_peer: &str, update: LocatorUpdate) -> Result<()> {
        crate::store::parse_peer(expected_peer)?;
        if expected_peer != update.peer_id {
            return Err(RemoteError::Unauthorized);
        }
        let _access = self.state.access()?;
        let contact = self.state.store.contact(expected_peer)?;
        self.state.store.update_locator_from(
            expected_peer,
            contact.name,
            update.locator,
            LocatorSource::ExplicitImport,
        )?;
        if let Some(controller) = self
            .controllers
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .get(expected_peer)
        {
            controller.network_changed();
        }
        Ok(())
    }
    pub fn forget_device(&self, peer: &str) -> Result<()> {
        let _access = self.state.access()?;
        self.state.store.forget_contact(peer)?;
        if let Some(controller) = self
            .controllers
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .remove(peer)
            && let Some(client) = controller
                .connection
                .lock()
                .map_err(|_| RemoteError::Unavailable)?
                .client
                .take()
        {
            client.close();
        }
        self.state
            .outgoing
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .remove(peer);
        Ok(())
    }
}
fn operation_targets(command: &Command) -> (Option<String>, Option<String>) {
    match command {
        Command::SendTurn {
            source_runtime_id,
            conversation_id,
            ..
        }
        | Command::SendTurnWithSelection {
            source_runtime_id,
            conversation_id,
            ..
        } => (
            Some(source_runtime_id.clone()),
            Some(conversation_id.clone()),
        ),
        Command::OpenConversation {
            runtime_id,
            conversation_id,
        }
        | Command::RenameConversation {
            runtime_id,
            conversation_id,
            ..
        }
        | Command::DeleteConversation {
            runtime_id,
            conversation_id,
        }
        | Command::SetAcpSessionConfig {
            runtime_id,
            conversation_id,
            ..
        }
        | Command::SendQueue {
            runtime_id,
            conversation_id,
            ..
        }
        | Command::SteerTurn {
            runtime_id,
            conversation_id,
            ..
        }
        | Command::EditMessage {
            runtime_id,
            conversation_id,
            ..
        }
        | Command::Cancel {
            runtime_id,
            conversation_id,
        }
        | Command::StopSubAgents {
            runtime_id,
            conversation_id,
        }
        | Command::ReplyRuntimeInteraction {
            runtime_id,
            conversation_id,
            ..
        } => (Some(runtime_id.clone()), Some(conversation_id.clone())),
        Command::CreateConversation { runtime_id, .. }
        | Command::CreateConversationWithSelection { runtime_id, .. }
        | Command::OpenAcpConversation { runtime_id, .. }
        | Command::ConfigureRuntimeExecution { runtime_id, .. }
        | Command::AdoptRuntimeResource { runtime_id, .. }
        | Command::ResolveResourceBinding { runtime_id, .. }
        | Command::WriteRuntimeResource { runtime_id, .. }
        | Command::DeleteRuntimeResource { runtime_id, .. } => (Some(runtime_id.clone()), None),
        _ => (None, None),
    }
}
