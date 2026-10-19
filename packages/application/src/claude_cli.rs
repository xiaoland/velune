//! The unmodified CLI owns authentication and secrets: in an application-owned
//! directory, or in a login the user already has that a provider only references.
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
/// `config_dir` is the login's `CLAUDE_CONFIG_DIR`; `None` runs on the user's
/// own default login, which setting the variable would hide.
pub(crate) fn command(binary: &str, config_dir: Option<&Path>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command
        .current_dir(config_dir.map_or_else(std::env::temp_dir, Path::to_path_buf))
        .env_clear();
    // macOS resolves the login keychain through the OS user's HOME and identity.
    // CLAUDE_CONFIG_DIR, rather than a synthetic HOME, isolates CLI credentials.
    for key in ["HOME", "USER", "LOGNAME", "PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    if let Some(dir) = config_dir {
        command.env("CLAUDE_CONFIG_DIR", dir);
    }
    command.env("DISABLE_AUTOUPDATER", "1");
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
            CredentialError::Timeout => Self::new(phase, "timeout"),
            CredentialError::InvalidOutput => Self::new(phase, "output_invalid"),
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
pub(crate) async fn status(
    binary: &str,
    config_dir: Option<&Path>,
) -> Result<Status, LoginFailure> {
    let expected = config_dir
        .map(Path::to_path_buf)
        .or_else(velune_agent_runtime::native::claude_user_config_directory)
        .ok_or_else(|| LoginFailure::new("status", "home_unavailable"))?;
    let mut cmd = command(binary, config_dir);
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
        != std::fs::canonicalize(&expected)
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
    // The tail tells logins apart; platforms add their own wording.
    let label = identity.as_ref().map(|id| id[id.len() - 8..].to_owned());
    Ok(Status {
        configured,
        label,
        identity,
        failure,
    })
}
/// Only for application-owned directories; a referenced login is never signed out.
pub(crate) async fn logout(binary: &str, dir: &Path) -> Result<(), ()> {
    let mut cmd = command(binary, Some(dir));
    cmd.args(["auth", "logout"]);
    crate::credential_helper::execute(cmd)
        .await
        .map(|_| ())
        .map_err(|_| ())
}
pub(crate) async fn usage(binary: &str, config_dir: Option<&Path>) -> Result<String, ()> {
    let mut cmd = command(binary, config_dir);
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
/// The `CLAUDE_CONFIG_DIR` of a referenced login: `None` for the user's
/// default, else a local Claude Code runtime instance's own directory.
pub(crate) fn linked_directory(
    login: &crate::provider_authentication::LinkedClaudeLogin,
    runtimes: &[crate::config::RuntimeInstance],
) -> Result<Option<String>, crate::Error> {
    use crate::provider_authentication::LinkedClaudeLogin;
    let LinkedClaudeLogin::Runtime { instance_id } = login else {
        return Ok(None);
    };
    let runtime = runtimes
        .iter()
        .find(|runtime| &runtime.id == instance_id && runtime.type_id == "claude-code-2.1")
        .ok_or_else(|| crate::Error::invalid("所选 Claude Code 运行时已不存在"))?;
    if runtime.execution()? != crate::RuntimeExecution::default() {
        return Err(crate::Error::invalid(
            "只能沿用使用本机 HOME 的 Claude Code 运行时的登录",
        ));
    }
    let directory = runtime
        .settings
        .get("agentDir")
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| crate::Error::invalid("请先为该运行时配置绝对路径的运行时目录"))?;
    Ok(
        velune_agent_runtime::native::claude_login_directory(directory)
            .map(|dir| dir.to_string_lossy().into_owned()),
    )
}
/// Reads a referenced login's status into its material; an unavailable CLI or
/// login leaves it unconfigured rather than failing the caller.
pub(crate) fn refresh_linked(
    material: &mut crate::provider_authentication::AuthenticationMaterial,
    binary: &str,
) -> Result<(), crate::Error> {
    let crate::provider_authentication::AuthenticationMaterial::ClaudeCliLinked {
        config_dir,
        configured,
        account_label,
        identity,
        ..
    } = material
    else {
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let status = runtime
        .block_on(status(binary, config_dir.as_deref().map(Path::new)))
        .ok();
    *configured = status.as_ref().is_some_and(|status| status.configured);
    *account_label = status.as_ref().and_then(|status| status.label.clone());
    *identity = status.and_then(|status| status.identity);
    Ok(())
}
/// One Claude account backs one provider, signed in here or referenced.
pub(crate) fn reject_duplicate_account(
    gateways: &[crate::config::GatewayConfig],
    provider_id: &str,
) -> Result<(), crate::Error> {
    use crate::provider_authentication::AuthenticationMaterial;
    let identity = |p: &crate::config::ProviderDefinition| match &p.authentication.material {
        AuthenticationMaterial::ClaudeCli { identity, .. }
        | AuthenticationMaterial::ClaudeCliLinked { identity, .. } => identity.clone(),
        _ => None,
    };
    let mut providers = gateways.iter().flat_map(|g| &g.providers);
    let mine = providers
        .clone()
        .find(|p| p.id == provider_id)
        .and_then(identity);
    if mine.is_some() && providers.any(|other| other.id != provider_id && identity(other) == mine) {
        return Err(crate::Error::invalid(
            "此 Claude 账户已绑定另一个提供商；请使用该提供商的模型。",
        ));
    }
    Ok(())
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
                    let mut cmd=command(&binary,Some(&dir)); cmd.args(["auth","login"]);
                    crate::credential_helper::execute_timeout(cmd,Duration::from_secs(300)).await.map_err(|error|LoginFailure::process("login",error))?;
                    let result=status(&binary,Some(&dir)).await?;
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
