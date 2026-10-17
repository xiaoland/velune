//! Bounded platform-helper execution. Source contents and helper output never enter logs.
#[cfg(unix)]
use std::{process::Stdio, time::Duration};
#[cfg(unix)]
use tokio::{io::AsyncReadExt, process::Command};

#[cfg(unix)]
const OUTPUT_LIMIT: u64 = 64 * 1024;
#[cfg(unix)]
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(super) enum CredentialError {
    Unavailable,
    #[cfg(unix)]
    Exit {
        code: Option<i32>,
    },
    #[cfg(unix)]
    Timeout,
    #[cfg(unix)]
    InvalidOutput,
}

#[cfg(unix)]
pub(super) async fn execute(command: Command) -> Result<String, CredentialError> {
    execute_timeout(command, RESOLVE_TIMEOUT).await
}

#[cfg(unix)]
pub(super) async fn execute_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<String, CredentialError> {
    use std::os::unix::process::CommandExt;
    command.as_std_mut().process_group(0);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| CredentialError::Unavailable)?;
    let mut group = HelperGroup(
        child
            .id()
            .and_then(|pid| rustix::process::Pid::from_raw(pid as i32)),
    );
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
        // Kill remaining descendants while the direct child PID is still held.
        group.kill();
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
        group.kill();
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    }
    result
}

#[cfg(unix)]
struct HelperGroup(Option<rustix::process::Pid>);
#[cfg(unix)]
impl HelperGroup {
    fn kill(&mut self) {
        if let Some(pid) = self.0.take() {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
}
#[cfg(unix)]
impl Drop for HelperGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

// A Windows helper needs a Job-backed tree lifetime before it can be enabled.
// Reject instead of advertising cancellation that kills only the parent process.
#[cfg(not(unix))]
pub(super) async fn execute(_command: tokio::process::Command) -> Result<String, CredentialError> {
    Err(CredentialError::Unavailable)
}

#[cfg(not(unix))]
pub(super) async fn execute_timeout(
    _command: tokio::process::Command,
    _timeout: std::time::Duration,
) -> Result<String, CredentialError> {
    Err(CredentialError::Unavailable)
}
