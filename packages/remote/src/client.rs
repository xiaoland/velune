use crate::{
    endpoint::NetworkGeneration,
    paths::ConnectionSlot,
    protocol::{Envelope, Reply, Request},
    service::ServiceState,
    wire::{read_frame, write_frame},
    *,
};
use std::{
    sync::{Arc, Weak, atomic::Ordering},
    time::{Duration, Instant},
};
use velune_application_api::{ApplicationReply, ClientViewId, Command, ExecutionContext, Failure};

pub struct RemoteClient {
    pub(crate) connection: Arc<ConnectionSlot>,
    pub(crate) network: Weak<NetworkGeneration>,
    pub(crate) service: Arc<ServiceState>,
    pub(crate) registration: u64,
    pub(crate) descriptor: EndpointDescriptor,
    pub(crate) session: ClientSession,
    pub(crate) authorized: bool,
    pub(crate) timeout: Duration,
}
impl RemoteClient {
    pub fn descriptor(&self) -> &EndpointDescriptor {
        &self.descriptor
    }
    pub fn peer_id(&self) -> String {
        self.connection.status().peer_id
    }
    pub fn session(&self) -> ClientSession {
        self.session.clone()
    }
    pub fn connection_status(&self) -> ConnectionStatus {
        self.connection.status()
    }
    pub fn observe_paths(&self) -> PathObserver {
        PathObserver::from_slot(self.connection.clone())
    }
    /// Allocate before transmitting so a lost reply can be reconciled by ID.
    pub fn next_request_id(&self) -> Result<RequestId> {
        let sequence = self
            .session
            .next
            .try_update(Ordering::AcqRel, Ordering::Acquire, |sequence| {
                sequence.checked_add(1)
            })
            .map_err(|_| RemoteError::Capacity)?;
        Ok(RequestId {
            incarnation: self.descriptor.incarnation.clone(),
            client: self.session.id.clone(),
            sequence,
        })
    }
    /// A bounded sequence window permits concurrent streams; no operation is retried implicitly.
    pub async fn submit(
        &self,
        id: RequestId,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> Result<RequestStatus> {
        id.validate(&self.descriptor.incarnation)?;
        match self
            .rpc(Request::Submit {
                id,
                view,
                execution,
                command: Box::new(command),
            })
            .await?
        {
            Reply::Status(status) => Ok(status),
            _ => Err(RemoteError::Protocol),
        }
    }
    pub async fn query(&self, id: RequestId) -> Result<RequestStatus> {
        match self.rpc(Request::Query { id }).await? {
            Reply::Status(status) => Ok(status),
            _ => Err(RemoteError::Protocol),
        }
    }
    pub async fn call(
        &self,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> std::result::Result<ApplicationReply, CallError> {
        let id = self.next_request_id().map_err(|error| CallError {
            request_id: None,
            failure: CallFailure::Transport(error),
        })?;
        let result = async {
            let mut status = self
                .submit(id.clone(), view, execution, command)
                .await
                .map_err(CallFailure::Transport)?;
            let deadline = Instant::now() + self.timeout;
            loop {
                match status {
                    RequestStatus::Completed(result) => {
                        return result
                            .map(|reply| *reply)
                            .map_err(|failure| CallFailure::Business(Box::new(failure)));
                    }
                    RequestStatus::Failed(error) => return Err(CallFailure::Transport(error)),
                    RequestStatus::Unknown | RequestStatus::ResultExpired => {
                        return Err(CallFailure::ResultUnavailable);
                    }
                    RequestStatus::Running => {}
                }
                if Instant::now() >= deadline {
                    return Err(CallFailure::Transport(RemoteError::Timeout));
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
                status = self
                    .query(id.clone())
                    .await
                    .map_err(CallFailure::Transport)?;
            }
        }
        .await;
        result.map_err(|failure| CallError {
            request_id: Some(id),
            failure,
        })
    }
    pub fn close(&self) {
        self.connection.close(b"client closed");
    }
    pub(crate) async fn rpc(&self, request: Request) -> Result<Reply> {
        if self.authorized && !self.service.trust.is_trusted(&self.peer_id())? {
            return Err(RemoteError::Unauthorized);
        }
        let network = self.network.upgrade().ok_or(RemoteError::Closed)?;
        let _operation = network.operation().await?;
        let connection = self.connection.native().ok_or(RemoteError::Closed)?;
        tokio::select! {
            biased;
            _=network.cancelled()=>Err(RemoteError::Closed),
            result=tokio::time::timeout(self.timeout,async {
                let (mut send,mut recv)=connection.open_bi().await.map_err(|_|RemoteError::Unavailable)?;
                write_frame(&mut send,Envelope {version:PROTOCOL_VERSION,body:request}).await?;
                let reply:Envelope<Reply>=read_frame(&mut recv,MAX_FRAME_BYTES).await?;
                if reply.version!=PROTOCOL_VERSION {return Err(RemoteError::IncompatibleVersion);}
                match reply.body {Reply::Error(error)=>Err(error),reply=>Ok(reply)}
            })=>result.map_err(|_|RemoteError::Timeout)?,
        }
    }
}
impl Drop for RemoteClient {
    fn drop(&mut self) {
        self.close();
        if let Some(network) = self.network.upgrade() {
            network.remove(self.registration);
        }
    }
}

pub enum CallFailure {
    Transport(RemoteError),
    Business(Box<Failure>),
    ResultUnavailable,
}
/// The request ID is retained on uncertain outcomes; callers query it instead of resending a mutation.
pub struct CallError {
    pub request_id: Option<RequestId>,
    pub failure: CallFailure,
}
impl std::fmt::Debug for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallError")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.failure {
            CallFailure::Transport(error) => error.fmt(f),
            CallFailure::Business(_) => f.write_str("远程业务请求未成功"),
            CallFailure::ResultUnavailable => f.write_str("请求结果已不可用；不得自动重新提交"),
        }
    }
}
impl std::error::Error for CallError {}
