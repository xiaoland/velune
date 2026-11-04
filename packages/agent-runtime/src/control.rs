//! Exclusive native control is independent of activity. Adapters report facts;
//! the application owns admission and clients consume semantic access reasons.
use crate::{execution::ExecutionTarget, version::RuntimeKind};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Observation {
    NotReported,
    Available,
    HeldElsewhere,
    Unknown,
}

/// Physical context identity: aliases of one runtime must not lock their owner out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub execution: ExecutionTarget,
    pub os_home: Option<PathBuf>,
    pub kind: RuntimeKind,
    pub agent_dir: PathBuf,
    pub native_id: String,
    pub node_binary: PathBuf,
    pub resources: PathBuf,
}
pub fn reports_control(kind: RuntimeKind) -> bool {
    matches!(
        kind,
        RuntimeKind::Codex | RuntimeKind::Codex166 | RuntimeKind::DeepSeek
    )
}

impl Target {
    pub fn reports_control(&self) -> bool {
        reports_control(self.kind)
    }
    pub fn observe(&self) -> Observation {
        if !self.reports_control() {
            return Observation::NotReported;
        }
        self.try_observe().unwrap_or(Observation::Unknown)
    }
    fn try_observe(&self) -> Option<Observation> {
        let path =
            match self.kind {
                RuntimeKind::Codex | RuntimeKind::Codex166 => {
                    // Native IDs are opaque, never caller-controlled filesystem paths.
                    if self.native_id.is_empty()
                        || !self
                            .native_id
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() || b == b'-')
                    {
                        return None;
                    }
                    self.agent_dir
                        .join("thread-writer-locks")
                        .join(format!("{}.lock", self.native_id))
                }
                RuntimeKind::DeepSeek => {
                    let mut command = crate::process::command(&self.node_binary);
                    command.arg(self.resources.join("huihua_sessions.mjs"));
                    let bytes = self.execution.output(command, Some(serde_json::to_vec(&json!({
                    "operation":"locate", "provider":"deepseek", "home":self.agent_dir,
                    "roots":[self.agent_dir.join("sessions")], "nativeId":self.native_id
                })).ok()?), self.os_home.as_deref()).ok()?;
                    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
                    let source = PathBuf::from(value["path"].as_str()?);
                    source.parent()?.to_owned()
                }
                _ => return Some(Observation::NotReported),
            };
        #[cfg(unix)]
        if self.execution.is_local() {
            let path = if self.kind == RuntimeKind::DeepSeek {
                path.join("session.lock")
            } else {
                path
            };
            return Some(observe_local_lock(&path));
        }
        #[cfg(windows)]
        if self.execution.is_local() {
            return Some(observe_windows(&path, self.kind));
        }
        // Remote observation executes there, never against a controller's filesystem.
        let mut command = self.execution.python();
        command
            .arg("-c")
            .arg(include_str!("../resources/control_probe.py"));
        let bytes = self.execution.output(command, Some(serde_json::to_vec(&json!({
            "kind":if self.kind == RuntimeKind::DeepSeek {"deepseek"} else {"codex"}, "path":path
        })).ok()?), self.os_home.as_deref()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

#[cfg(unix)]
fn observe_local_lock(path: &std::path::Path) -> Observation {
    use std::os::fd::AsRawFd;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Observation::Available,
        Err(_) => return Observation::Unknown,
    };
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        Observation::Available
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            Observation::HeldElsewhere
        } else {
            Observation::Unknown
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn lock_is_independent_of_turn_and_stale_file() {
        use std::os::fd::AsRawFd;
        let path = std::env::temp_dir().join(format!("velune-control-{}", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert_eq!(observe_local_lock(&path), Observation::HeldElsewhere);
        drop(file);
        assert_eq!(observe_local_lock(&path), Observation::Available);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(observe_local_lock(&path), Observation::Available);
    }
}

#[cfg(windows)]
fn observe_windows(path: &std::path::Path, kind: RuntimeKind) -> Observation {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError},
        Storage::FileSystem::{LockFileEx, UnlockFileEx},
        System::{
            IO::OVERLAPPED,
            Threading::{OpenSemaphoreW, ReleaseSemaphore, WaitForSingleObject},
        },
    };
    if kind == RuntimeKind::DeepSeek {
        use sha2::{Digest, Sha256};
        let Ok(path) = std::path::absolute(path) else {
            return Observation::Unknown;
        };
        let name = format!(
            "Local\\dsh-session-lock-{:x}",
            Sha256::digest(
                path.to_string_lossy()
                    .replace('/', "\\")
                    .to_lowercase()
                    .as_bytes()
            )
        );
        let wide = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        unsafe {
            let handle = OpenSemaphoreW(0x00100000 | 0x0002, 0, wide.as_ptr());
            if handle.is_null() {
                return if GetLastError() == 2 {
                    Observation::Available
                } else {
                    Observation::Unknown
                };
            }
            let result = match WaitForSingleObject(handle, 0) {
                0 if ReleaseSemaphore(handle, 1, std::ptr::null_mut()) != 0 => {
                    Observation::Available
                }
                258 => Observation::HeldElsewhere,
                _ => Observation::Unknown,
            };
            CloseHandle(handle);
            return result;
        }
    }
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Observation::Available,
        Err(_) => return Observation::Unknown,
    };
    unsafe {
        let mut overlap: OVERLAPPED = std::mem::zeroed();
        let handle = file.as_raw_handle();
        if LockFileEx(handle, 3, 0, 1, 0, &mut overlap) != 0 {
            UnlockFileEx(handle, 0, 1, 0, &mut overlap);
            Observation::Available
        } else if GetLastError() == 33 {
            Observation::HeldElsewhere
        } else {
            Observation::Unknown
        }
    }
}
