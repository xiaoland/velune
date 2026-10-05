//! Bounded platform-helper execution. Source contents and helper output never enter logs.
use std::path::Path;
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
    Timeout,
    #[cfg(unix)]
    InvalidOutput,
    #[cfg(unix)]
    StaleBinding,
}

#[cfg(unix)]
pub(super) async fn resolve(
    path: &Path,
    reference: Option<&str>,
    source: Option<&str>,
    protocol: &str,
    endpoint: &str,
) -> Result<String, CredentialError> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(path);
    command.as_std_mut().process_group(0);
    if let Some(source) = source {
        command.arg("--source-json").arg(source);
    } else {
        command.arg(reference.ok_or(CredentialError::Unavailable)?);
    }
    let mut child = command
        .env_clear()
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
            return Err(CredentialError::Unavailable);
        }
        String::from_utf8(bytes).map_err(|_| CredentialError::InvalidOutput)
    };
    let result = match tokio::time::timeout(RESOLVE_TIMEOUT, operation).await {
        Ok(result) => result,
        Err(_) => Err(CredentialError::Timeout),
    };
    if result.is_err() {
        group.kill();
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    }
    let result = result?;
    if source.is_none() {
        return (!result.trim().is_empty())
            .then(|| result.trim().to_owned())
            .ok_or(CredentialError::InvalidOutput);
    }
    let envelope: serde_json::Value =
        serde_json::from_str(&result).map_err(|_| CredentialError::InvalidOutput)?;
    if envelope["contractVersion"] != 1
        || envelope["capabilities"]["protocol"] != protocol
        || envelope["capabilities"]["endpoint"] != endpoint
    {
        return Err(CredentialError::StaleBinding);
    }
    envelope["bearer"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CredentialError::InvalidOutput)?;
    Ok(result)
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
pub(super) async fn resolve(
    _path: &Path,
    _reference: Option<&str>,
    _source: Option<&str>,
    _protocol: &str,
    _endpoint: &str,
) -> Result<String, CredentialError> {
    Err(CredentialError::Unavailable)
}
