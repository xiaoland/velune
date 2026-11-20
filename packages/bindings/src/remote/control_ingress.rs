//! Loopback-only HTTP ingress owned by the live application. HTTPS terminates at the chosen proxy.
use super::{
    hapi,
    session_control::{SessionControl, SessionGrant},
    session_control_http,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, task::JoinHandle};
use velune_application::ApplicationService;
use velune_application_api::ClientViewId;
use velune_remote::RemoteError;

#[derive(Clone, Copy)]
pub(super) enum Protocol {
    Hapi,
    Canonical,
}

pub(super) struct ControlIngress {
    application: ApplicationService,
    hapi: Mutex<Option<Listener>>,
    canonical: Mutex<Option<Listener>>,
}

pub(super) struct ControlShare {
    #[cfg(target_os = "macos")]
    pub(super) id: String,
    pub(super) address: SocketAddr,
    pub(super) access_token: String,
}

struct Listener {
    id: String,
    control: Arc<SessionControl>,
    task: JoinHandle<()>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.control.revoke();
        self.task.abort();
    }
}

impl ControlIngress {
    pub(super) fn new(application: ApplicationService) -> Self {
        Self {
            application,
            hapi: Mutex::new(None),
            canonical: Mutex::new(None),
        }
    }

    pub(super) async fn start(
        &self,
        protocol: Protocol,
        port: u16,
        grants: Vec<SessionGrant>,
    ) -> Result<ControlShare, RemoteError> {
        let mut bytes = [0; 48];
        getrandom::fill(&mut bytes).map_err(|_| RemoteError::Randomness)?;
        let token = URL_SAFE_NO_PAD.encode(&bytes[..32]);
        let id = URL_SAFE_NO_PAD.encode(&bytes[32..]);
        let view = ClientViewId(format!("external:{id}"));
        let grant_count = grants.len();
        let control = Arc::new(
            SessionControl::new(self.application.clone(), grants, view)
                .map_err(|_| RemoteError::Configuration)?,
        );
        if control
            .list()
            .await
            .map_err(|_| RemoteError::Unavailable)?
            .len()
            != grant_count
        {
            return Err(RemoteError::Configuration);
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| RemoteError::Unavailable)?;
        let address = listener
            .local_addr()
            .map_err(|_| RemoteError::Unavailable)?;
        let mut running = self
            .listener(protocol)
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if running
            .as_ref()
            .is_some_and(|listener| !listener.task.is_finished())
        {
            return Err(RemoteError::ConfigurationConflict);
        }
        let router = match protocol {
            Protocol::Hapi => hapi::router(control.clone(), token.clone(), id.clone()),
            Protocol::Canonical => session_control_http::router(control.clone(), token.clone()),
        };
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::warn!(%error, "External session control listener stopped");
            }
        });
        *running = Some(Listener {
            id: id.clone(),
            control,
            task,
        });
        Ok(ControlShare {
            #[cfg(target_os = "macos")]
            id,
            address,
            access_token: token,
        })
    }

    pub(super) fn stop(&self, protocol: Protocol) -> Result<(), RemoteError> {
        self.stop_matching(protocol, None)
    }

    pub(super) fn stop_matching(
        &self,
        protocol: Protocol,
        id: Option<&str>,
    ) -> Result<(), RemoteError> {
        let mut running = self
            .listener(protocol)
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if running
            .as_ref()
            .is_some_and(|listener| id.is_none_or(|id| id == listener.id))
        {
            running.take();
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub(super) fn active_id(&self, protocol: Protocol) -> Result<Option<String>, RemoteError> {
        let running = self
            .listener(protocol)
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        Ok(running
            .as_ref()
            .filter(|listener| !listener.task.is_finished())
            .map(|listener| listener.id.clone()))
    }

    fn listener(&self, protocol: Protocol) -> &Mutex<Option<Listener>> {
        match protocol {
            Protocol::Hapi => &self.hapi,
            Protocol::Canonical => &self.canonical,
        }
    }
}
