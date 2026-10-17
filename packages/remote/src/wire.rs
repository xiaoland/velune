use crate::{MAX_FRAME_BYTES, RemoteError, Result};
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Serialize, de::DeserializeOwned};
use std::io::Write;

struct FrameBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for FrameBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let needed = self.bytes.len() + bytes.len();
        if needed > self.bytes.capacity() {
            let capacity = needed
                .max(self.bytes.capacity().saturating_mul(2))
                .min(self.limit);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(crate) fn encode<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>> {
    let mut buffer = FrameBuffer {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut buffer, value) {
        Ok(()) => Ok(buffer.bytes),
        Err(_) if buffer.exceeded => Err(RemoteError::FrameTooLarge),
        Err(_) => Err(RemoteError::Protocol),
    }
}
pub(crate) async fn write_frame<T: Serialize + Send + 'static>(
    send: &mut SendStream,
    value: T,
) -> Result<()> {
    // Large base64 payloads must not monopolize the runtime handling approval/cancel streams.
    let bytes = tokio::task::spawn_blocking(move || encode(&value, MAX_FRAME_BYTES))
        .await
        .map_err(|_| RemoteError::Protocol)??;
    send.write_all(&bytes)
        .await
        .map_err(|_| RemoteError::Unavailable)?;
    send.finish().map_err(|_| RemoteError::Unavailable)?;
    Ok(())
}
pub(crate) async fn read_frame<T: DeserializeOwned + Send + 'static>(
    recv: &mut RecvStream,
    limit: usize,
) -> Result<T> {
    let bytes = recv
        .read_to_end(limit)
        .await
        .map_err(|_| RemoteError::Protocol)?;
    tokio::task::spawn_blocking(move || {
        serde_json::from_slice(&bytes).map_err(|_| RemoteError::Protocol)
    })
    .await
    .map_err(|_| RemoteError::Protocol)?
}
