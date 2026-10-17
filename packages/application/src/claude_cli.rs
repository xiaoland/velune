//! The unmodified CLI owns authentication and secrets in an application-owned directory.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

pub(crate) fn directory(home: &Path, id: &str) -> PathBuf {
    home.join("claude-cli").join(id)
}
pub(crate) fn command(binary: &str, directory: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command
        .env_clear()
        .env("HOME", directory)
        .env("CLAUDE_CONFIG_DIR", directory)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("DISABLE_AUTOUPDATER", "1");
    command
}
#[derive(Clone)]
pub(crate) struct Status {
    pub configured: bool,
    pub label: Option<String>,
    pub identity: Option<String>,
}
pub(crate) async fn status(binary: &str, dir: &Path) -> Result<Status, ()> {
    let mut cmd = command(binary, dir);
    cmd.args(["auth", "status"]);
    let output = crate::credential_helper::execute(cmd)
        .await
        .map_err(|_| ())?;
    let value: Value = serde_json::from_str(&output).map_err(|_| ())?;
    let reported = value["configDirectory"].as_str().ok_or(())?;
    if std::fs::canonicalize(reported).map_err(|_| ())?
        != std::fs::canonicalize(dir).map_err(|_| ())?
    {
        return Err(());
    }
    let configured = value["loggedIn"].as_bool().ok_or(())?
        && matches!(value["authMethod"].as_str(), Some("claude.ai"));
    let stable = value["accountUuid"]
        .as_str()
        .or_else(|| value["userId"].as_str())
        .filter(|s| !s.trim().is_empty() && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .or_else(|| {
            value["email"]
                .as_str()
                .filter(|email| !email.trim().is_empty() && !email.chars().any(char::is_control))
                .map(|email| {
                    format!(
                        "{}\0{}",
                        email.trim().to_lowercase(),
                        value["orgId"].as_str().unwrap_or("")
                    )
                })
        });
    let identity = stable.map(|subject| {
        format!(
            "{:x}",
            Sha256::digest(
                format!("{}\0{}", subject, value["orgId"].as_str().unwrap_or("")).as_bytes()
            )
        )
    });
    let label = identity
        .as_ref()
        .map(|id| format!("账户 · {}", &id[id.len() - 8..]));
    Ok(Status {
        configured,
        label,
        identity,
    })
}
pub(crate) async fn logout(binary: &str, dir: &Path) -> Result<(), ()> {
    let mut cmd = command(binary, dir);
    cmd.args(["auth", "logout"]);
    crate::credential_helper::execute(cmd)
        .await
        .map(|_| ())
        .map_err(|_| ())
}
pub(crate) async fn usage(binary: &str, dir: &Path) -> Result<String, ()> {
    let mut cmd = command(binary, dir);
    // /usage is the CLI's built-in command; never retry it as a model prompt.
    cmd.args([
        "-p",
        "/usage",
        "--output-format",
        "json",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
        "--settings",
        "{\"disableAllHooks\":true}",
    ]);
    let output = crate::credential_helper::execute(cmd)
        .await
        .map_err(|_| ())?;
    let value: Value = serde_json::from_str(&output).map_err(|_| ())?;
    let result = match &value {
        Value::Array(items) => items
            .iter()
            .rev()
            .find(|v| v["type"] == "result")
            .ok_or(())?,
        Value::Object(_) => &value,
        _ => return Err(()),
    };
    if result["type"] != "result" || result["is_error"].as_bool() != Some(false) {
        return Err(());
    }
    result["result"].as_str().map(str::to_owned).ok_or(())
}
pub(crate) fn remove(home: &Path, id: &str) -> std::io::Result<()> {
    match std::fs::remove_dir_all(directory(home, id)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
pub(crate) struct Login {
    id: String,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Status, ()>>,
    running: bool,
    finalize: Option<mpsc::Sender<bool>>,
    status: Option<Status>,
}
impl Login {
    pub(crate) fn start(home: PathBuf, binary: String) -> Result<Self, crate::Error> {
        let id = crate::new_record_key();
        let dir = directory(&home, &id);
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, result) = mpsc::channel();
        let (finalize, finalization) = mpsc::channel();
        let spawned=std::thread::Builder::new().name("velune-cli-login".into()).spawn(move || {
            let runtime=match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime)=>runtime,
                Err(_)=> { let _=sender.send(Err(())); let _=std::fs::remove_dir_all(&dir); return; }
            };
            let outcome=runtime.block_on(async {
                let operation=async {
                    let mut cmd=command(&binary,&dir); cmd.args(["auth","login"]);
                    crate::credential_helper::execute_timeout(cmd,Duration::from_secs(300)).await.map_err(|_| ())?;
                    let result=status(&binary,&dir).await?;
                    if result.configured && result.identity.is_some() { Ok(result) } else { Err(()) }
                };
                tokio::pin!(operation);
                loop { tokio::select! {
                    result=&mut operation => return result,
                    _=tokio::time::sleep(Duration::from_millis(50)) => if worker_cancel.load(Ordering::Acquire) { return Err(()); }
                } }
            });
            let successful=outcome.is_ok();
            let delivered=sender.send(outcome).is_ok();
            // No subprocess remains while waiting for the configuration commit decision.
            let keep=successful && delivered && finalization.recv().unwrap_or(false);
            if !keep {
                if runtime.block_on(logout(&binary,&dir)).is_err() { tracing::warn!(event="claude_staged_logout_failed"); }
                if std::fs::remove_dir_all(&dir).is_err() { tracing::warn!(event="claude_staged_cleanup_failed"); }
            }
        });
        if let Err(error) = spawned {
            let _ = remove(&home, &id);
            return Err(error.into());
        }
        Ok(Self {
            id,
            cancel,
            result,
            running: true,
            finalize: Some(finalize),
            status: None,
        })
    }
    pub(crate) fn poll(&mut self) -> Value {
        let mut events = Vec::new();
        if self.running {
            match self.result.try_recv() {
                Ok(result) => {
                    self.running = false;
                    let cancelled = self.cancel.load(Ordering::Acquire);
                    let ok = result.is_ok() && !cancelled;
                    self.status = if ok { result.ok() } else { None };
                    events.push(json!({"type":"result","ok":ok,"error":if ok {None::<&str>} else {Some("Claude Code 登录未完成；请检查 CLI 与订阅状态")},"cancelled":self.cancel.load(Ordering::Acquire)}));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.running = false;
                    events.push(
                        json!({"type":"result","ok":false,"error":"Claude Code 登录进程已结束"}),
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        json!({"running":self.running,"events":events})
    }
    pub(crate) fn credential_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
    pub(crate) fn commit(&mut self) {
        if let Some(finalize) = self.finalize.take() {
            let _ = finalize.send(true);
        }
    }
    pub(crate) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(finalize) = self.finalize.take() {
            let _ = finalize.send(false);
        }
    }
    pub(crate) fn is_running(&self) -> bool {
        self.running
    }
}
impl Drop for Login {
    fn drop(&mut self) {
        self.cancel();
    }
}
