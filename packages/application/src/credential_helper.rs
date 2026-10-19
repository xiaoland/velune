//! Bounded platform-helper execution. Source contents and helper output never enter logs.
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

const OUTPUT_LIMIT: u64 = 64 * 1024;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(super) enum CredentialError {
    Unavailable,
    Exit { code: Option<i32> },
    Timeout,
    InvalidOutput,
}

pub(super) async fn execute(command: Command) -> Result<String, CredentialError> {
    execute_timeout(command, RESOLVE_TIMEOUT).await
}

pub(super) async fn execute_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<String, CredentialError> {
    tree::prepare(&mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| CredentialError::Unavailable)?;
    let mut tree = match tree::Tree::containing(&child) {
        Ok(tree) => tree,
        Err(_) => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            return Err(CredentialError::Unavailable);
        }
    };
    let mut stdout = child
        .stdout
        .take()
        .ok_or(CredentialError::Unavailable)?
        .take(OUTPUT_LIMIT + 1);
    let operation = async {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| CredentialError::InvalidOutput)?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(CredentialError::InvalidOutput);
        }
        // Kill remaining descendants while the direct child is still held.
        tree.kill();
        let status = child
            .wait()
            .await
            .map_err(|_| CredentialError::Unavailable)?;
        if !status.success() {
            return Err(CredentialError::Exit {
                code: status.code(),
            });
        }
        String::from_utf8(bytes).map_err(|_| CredentialError::InvalidOutput)
    };
    let result = match tokio::time::timeout(timeout, operation).await {
        Ok(result) => result,
        Err(_) => Err(CredentialError::Timeout),
    };
    if result.is_err() {
        tree.kill();
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    }
    result
}

/// The helper and every process it starts, killed together.
#[cfg(unix)]
mod tree {
    use std::io;
    use tokio::process::{Child, Command};

    pub(super) fn prepare(command: &mut Command) {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }

    pub(super) struct Tree(Option<rustix::process::Pid>);

    impl Tree {
        pub(super) fn containing(child: &Child) -> io::Result<Self> {
            Ok(Self(child.id().and_then(|pid| {
                rustix::process::Pid::from_raw(pid as i32)
            })))
        }

        pub(super) fn kill(&mut self) {
            if let Some(pid) = self.0.take() {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            self.kill();
        }
    }
}

/// The helper and every process it starts, held in a kill-on-close Job Object.
#[cfg(windows)]
mod tree {
    #![allow(unsafe_code)]
    use std::{io, ptr};
    use tokio::process::{Child, Command};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
    };

    pub(super) fn prepare(command: &mut Command) {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
        // Callers clear the environment; Windows programs need these to load
        // system libraries and initialize networking.
        for key in ["SystemRoot", "windir"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }

    pub(super) struct Tree(Option<HANDLE>);

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for Tree {}

    impl Tree {
        pub(super) fn containing(child: &Child) -> io::Result<Self> {
            let process = child
                .raw_handle()
                .ok_or_else(|| io::Error::other("the helper already exited"))?;
            // SAFETY: both pointers may be null: no name, default security.
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let tree = Self(Some(handle));
            // SAFETY: an all-zero value is valid for this plain C struct.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: the pointer and size describe `limits`, which outlives the call.
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the process handle stays open while `child` is borrowed.
            if unsafe { AssignProcessToJobObject(handle, process) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(tree)
        }

        pub(super) fn kill(&mut self) {
            if let Some(handle) = self.0.take() {
                // SAFETY: the handle is open and is closed exactly once here.
                unsafe {
                    TerminateJobObject(handle, 1);
                    CloseHandle(handle);
                }
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            self.kill();
        }
    }
}
