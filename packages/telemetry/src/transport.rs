use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tonic::{
    metadata::MetadataMap,
    transport::{Channel, Endpoint},
};

use crate::{Error, Profile, Result};

#[derive(Default)]
struct PermissionState {
    allowed: bool,
    waiters: Vec<Weak<Mutex<Option<Waker>>>>,
}

/// The same lock orders revocation against connection admission and every socket write.
#[derive(Clone, Default)]
pub(crate) struct Permission(Arc<Mutex<PermissionState>>);
impl Permission {
    pub fn is_enabled(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).allowed
    }
    pub fn enable(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).allowed = true;
    }
    pub fn revoke(&self) {
        let waiters = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.allowed = false;
            std::mem::take(&mut state.waiters)
        };
        for waiter in waiters.into_iter().filter_map(|w| w.upgrade()) {
            if let Some(waker) = waiter.lock().unwrap_or_else(|e| e.into_inner()).take() {
                waker.wake();
            }
        }
    }
    fn admitted(&self) -> io::Result<()> {
        if self.0.lock().unwrap_or_else(|e| e.into_inner()).allowed {
            Ok(())
        } else {
            Err(revoked())
        }
    }
}
fn revoked() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "telemetry generation revoked",
    )
}

struct GatedIo {
    socket: tokio::net::TcpStream,
    permission: Permission,
    waiter: Arc<Mutex<Option<Waker>>>,
}
impl GatedIo {
    fn new(socket: tokio::net::TcpStream, permission: Permission) -> Self {
        let waiter = Arc::new(Mutex::new(None));
        let mut state = permission.0.lock().unwrap_or_else(|e| e.into_inner());
        state.waiters.retain(|w| w.strong_count() > 0);
        state.waiters.push(Arc::downgrade(&waiter));
        drop(state);
        Self {
            socket,
            permission,
            waiter,
        }
    }
    fn poll<T>(
        &mut self,
        cx: &mut Context<'_>,
        body: impl FnOnce(Pin<&mut tokio::net::TcpStream>, &mut Context<'_>) -> Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        let state = self.permission.0.lock().unwrap_or_else(|e| e.into_inner());
        if !state.allowed {
            return Poll::Ready(Err(revoked()));
        }
        *self.waiter.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
        body(Pin::new(&mut self.socket), cx)
    }
}
impl Drop for GatedIo {
    fn drop(&mut self) {
        self.permission
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiters
            .retain(|w| !w.ptr_eq(&Arc::downgrade(&self.waiter)) && w.strong_count() > 0);
    }
}
impl AsyncRead for GatedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.poll(cx, |socket, cx| socket.poll_read(cx, buf))
    }
}
impl AsyncWrite for GatedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.poll(cx, |socket, cx| socket.poll_write(cx, buf))
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll(cx, |socket, cx| socket.poll_flush(cx))
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}

pub(crate) fn metadata(profile: &Profile) -> Result<MetadataMap> {
    let mut metadata = MetadataMap::new();
    for (key, value) in &profile.headers {
        metadata.insert(
            key.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>()
                .map_err(|_| Error::InvalidHeaders)?,
            value.parse().map_err(|_| Error::InvalidHeaders)?,
        );
    }
    Ok(metadata)
}

pub(crate) fn channel(profile: &Profile, permission: Permission) -> Result<Channel> {
    let url = url::Url::parse(&profile.endpoint).map_err(|_| Error::InvalidEndpoint)?;
    let host = url
        .host_str()
        .ok_or(Error::InvalidEndpoint)?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = url.port_or_known_default().ok_or(Error::InvalidEndpoint)?;
    let mut endpoint = Endpoint::from_shared(profile.endpoint.clone())
        .map_err(|_| Error::InvalidEndpoint)?
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(3));
    if url.scheme() == "https" {
        let verifier = rustls_platform_verifier::Verifier::new(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .map_err(|_| Error::Transport)?;
        endpoint = endpoint
            .tls_config_with_verifier(
                tonic::transport::ClientTlsConfig::new().timeout(Duration::from_secs(3)),
                Arc::new(verifier),
            )
            .map_err(|_| Error::Transport)?;
    }
    Ok(
        endpoint.connect_with_connector_lazy(tower::service_fn(move |_| {
            let permission = permission.clone();
            let host = host.clone();
            async move {
                permission.admitted()?;
                let socket = tokio::time::timeout(
                    Duration::from_secs(3),
                    tokio::net::TcpStream::connect((host.as_str(), port)),
                )
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "telemetry connect timeout")
                })??;
                permission.admitted()?;
                Ok::<_, io::Error>(hyper_util::rt::TokioIo::new(GatedIo::new(
                    socket, permission,
                )))
            }
        })),
    )
}
