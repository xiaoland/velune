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
    command.current_dir(directory).env_clear();
    // macOS resolves the login keychain through the OS user's HOME and identity.
    // CLAUDE_CONFIG_DIR, rather than a synthetic HOME, isolates CLI credentials.
    for key in ["HOME", "USER", "LOGNAME", "PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("CLAUDE_CONFIG_DIR", directory)
        .env("DISABLE_AUTOUPDATER", "1");
    command
}
#[derive(Clone)]
pub(crate) struct LoginFailure {
    phase: &'static str,
    reason: &'static str,
    exit_code: Option<i32>,
}
impl LoginFailure {
    fn new(phase: &'static str, reason: &'static str) -> Self {
        Self {
            phase,
            reason,
            exit_code: None,
        }
    }
    fn process(phase: &'static str, error: crate::credential_helper::CredentialError) -> Self {
        use crate::credential_helper::CredentialError;
        match error {
            CredentialError::Unavailable => Self::new(phase, "process_unavailable"),
            #[cfg(unix)]
            CredentialError::Timeout => Self::new(phase, "timeout"),
            #[cfg(unix)]
            CredentialError::InvalidOutput => Self::new(phase, "output_invalid"),
            #[cfg(unix)]
            CredentialError::Exit { code } => Self {
                phase,
                reason: "process_exit",
                exit_code: code,
            },
        }
    }
    fn message(&self) -> String {
        let exit = self
            .exit_code
            .map_or_else(|| "none".to_owned(), |value| value.to_string());
        format!(
            "Claude Code 认证未完成 [phase={};reason={};exit_code={exit}]",
            self.phase, self.reason
        )
    }
}
#[derive(Clone)]
pub(crate) struct Status {
    pub configured: bool,
    pub label: Option<String>,
    pub identity: Option<String>,
    failure: Option<LoginFailure>,
}
pub(crate) async fn status(binary: &str, dir: &Path) -> Result<Status, LoginFailure> {
    let mut cmd = command(binary, dir);
    cmd.args(["auth", "status"]);
    let output = crate::credential_helper::execute(cmd)
        .await
        .map_err(|error| LoginFailure::process("status", error))?;
    let value: Value =
        serde_json::from_str(&output).map_err(|_| LoginFailure::new("status", "json_invalid"))?;
    let reported = value["configDirectory"]
        .as_str()
        .ok_or_else(|| LoginFailure::new("status", "config_directory_missing"))?;
    if std::fs::canonicalize(reported)
        .map_err(|_| LoginFailure::new("status", "reported_directory_unavailable"))?
        != std::fs::canonicalize(dir)
            .map_err(|_| LoginFailure::new("status", "owned_directory_unavailable"))?
    {
        return Err(LoginFailure::new("status", "config_directory_mismatch"));
    }
    let logged_in = value["loggedIn"]
        .as_bool()
        .ok_or_else(|| LoginFailure::new("status", "logged_in_missing"))?;
    let subscription = matches!(value["authMethod"].as_str(), Some("claude.ai"));
    let configured = logged_in && subscription;
    let failure = if !logged_in {
        Some(LoginFailure::new("status", "not_logged_in"))
    } else if !subscription {
        Some(LoginFailure::new(
            "status",
            "authentication_method_unexpected",
        ))
    } else {
        None
    };
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
        failure,
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
    result: mpsc::Receiver<Result<Status, LoginFailure>>,
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
                Err(_)=> { let _=sender.send(Err(LoginFailure::new("worker","runtime_unavailable"))); let _=std::fs::remove_dir_all(&dir); return; }
            };
            let outcome=runtime.block_on(async {
                let operation=async {
                    let mut cmd=command(&binary,&dir); cmd.args(["auth","login"]);
                    crate::credential_helper::execute_timeout(cmd,Duration::from_secs(300)).await.map_err(|error|LoginFailure::process("login",error))?;
                    let result=status(&binary,&dir).await?;
                    if !result.configured { return Err(result.failure.unwrap_or_else(||LoginFailure::new("status","not_configured"))); }
                    if result.identity.is_none() { return Err(LoginFailure::new("status","identity_missing")); }
                    Ok(result)
                };
                tokio::pin!(operation);
                loop { tokio::select! {
                    result=&mut operation => return result,
                    _=tokio::time::sleep(Duration::from_millis(50)) => if worker_cancel.load(Ordering::Acquire) { return Err(LoginFailure::new("login","cancelled")); }
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
                    let error = if cancelled {
                        Some(LoginFailure::new("login", "cancelled").message())
                    } else {
                        result.as_ref().err().map(LoginFailure::message)
                    };
                    self.status = if ok { result.ok() } else { None };
                    events
                        .push(json!({"type":"result","ok":ok,"error":error,"cancelled":cancelled}));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.running = false;
                    events.push(
                        json!({"type":"result","ok":false,"error":LoginFailure::new("worker","channel_disconnected").message()}),
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
