//! Same-user pairing control for the service already owned by the native application.
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    task::{JoinHandle, JoinSet},
};
use velune_remote::{PairingOffer, PairingPhase, RemoteError};

pub(super) struct LocalControl {
    task: JoinHandle<()>,
    path: PathBuf,
    inode: u64,
}

impl LocalControl {
    pub(super) fn bind(owner: &ServiceOwner, directory: &Path) -> Result<Self, BindingError> {
        let setup = || -> std::io::Result<(std::os::unix::net::UnixListener, PathBuf, u64)> {
            let directory = directory.join("cli");
            match fs::DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            let metadata = fs::symlink_metadata(&directory)?;
            if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
                return Err(std::io::Error::other(
                    "CLI 目录必须是仅当前用户可访问的目录",
                ));
            }
            let path = directory.join("control.sock");
            if let Ok(existing) = fs::symlink_metadata(&path) {
                if !existing.file_type().is_socket() || existing.uid() != metadata.uid() {
                    return Err(std::io::Error::other("CLI 路径已被占用"));
                }
                match std::os::unix::net::UnixStream::connect(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                        fs::remove_file(&path)?
                    }
                    _ => return Err(std::io::Error::other("另一个 Velune 已提供此 CLI 接口")),
                }
            }
            let listener = std::os::unix::net::UnixListener::bind(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            let inode = fs::symlink_metadata(&path)?.ino();
            Ok((listener, path, inode))
        };
        let (listener, path, inode) = setup().map_err(|error| BindingError::Io {
            detail: error.to_string(),
        })?;
        let _context = owner.runtime.enter();
        let listener = UnixListener::from_std(listener).map_err(|_| BindingError::Unavailable)?;
        let service = Arc::downgrade(&owner.service);
        let task = owner.runtime.spawn(async move {
            let mut requests = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        if requests.len() >= 8 { continue; }
                        let Some(service) = service.upgrade() else { break };
                        requests.spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(65), serve(stream, service)).await;
                        });
                    }
                    _ = requests.join_next(), if !requests.is_empty() => {}
                }
            }
        });
        Ok(Self { task, path, inode })
    }
}

impl Drop for LocalControl {
    fn drop(&mut self) {
        self.task.abort();
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| metadata.ino() == self.inode) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Status,
    Invite,
    Begin {
        invitation: String,
    },
    Approve {
        attempt: String,
        peer: String,
        code: String,
    },
    Revoke {
        peer: String,
    },
    Connect {
        peer: String,
    },
}

async fn execute(
    service: Arc<RemoteAccessService>,
    command: Command,
) -> Result<Value, RemoteError> {
    let value = match command {
        Command::Status => json!(service.read_state()?),
        Command::Invite => {
            // Match the explicit "add controlling device" action in the native app.
            service.set_control_ingress(true)?;
            let offer = service.create_invitation(Duration::from_secs(300))?;
            json!({"invitation": offer.encode()?})
        }
        Command::Begin { invitation } => json!(
            service
                .begin_pairing(&PairingOffer::decode(&invitation)?)
                .await?
        ),
        Command::Approve {
            attempt,
            peer,
            code,
        } => {
            let state = service.read_state()?;
            let expected = state.incoming_pairings.iter().find(|candidate| {
                candidate.attempt_id == attempt
                    && candidate.peer_id == peer
                    && candidate.phase == PairingPhase::AwaitingApproval
            });
            if expected.and_then(|candidate| candidate.verification_code.as_deref())
                != Some(code.as_str())
            {
                return Err(RemoteError::PairingUnavailable);
            }
            json!(service.approve_pairing(&attempt, &peer)?)
        }
        Command::Revoke { peer } => {
            service.revoke_control(&peer)?;
            Value::Null
        }
        Command::Connect { peer } => {
            let controller = service.controller(&peer)?;
            controller.connect().await?;
            json!(controller.status()?)
        }
    };
    Ok(value)
}

async fn serve(mut stream: UnixStream, service: Arc<RemoteAccessService>) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new((&mut stream).take(8193)).read_until(b'\n', &mut bytes),
    )
    .await??;
    let result = if bytes.len() > 8192 || bytes.last() != Some(&b'\n') {
        Err(RemoteError::FrameTooLarge)
    } else {
        match serde_json::from_slice::<Command>(&bytes) {
            Ok(command) => execute(service, command).await,
            Err(_) => Err(RemoteError::Protocol),
        }
    };
    let response = match result {
        Ok(value) => json!({"ok": true, "result": value}),
        Err(error) => {
            json!({"ok": false, "error": {"code": error.code(), "message": error.to_string()}})
        }
    };
    let mut response = serde_json::to_vec(&response)?;
    response.push(b'\n');
    stream.write_all(&response).await?;
    stream.shutdown().await
}
