//! Provider-owned Codex subscription sessions. No Harness auth files are read or written.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use velune_gateway::{CredentialResolutionError, ResolvedCredential};

pub(crate) const ENDPOINT: &str = crate::CODEX_SUBSCRIPTION_ENDPOINT;
const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CALLBACK: &str = "http://localhost:1455/auth/callback";
const LIMIT: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
struct Session {
    generation: u64,
    access_token: String,
    refresh_token: String,
    id_token: String,
    account_id: String,
    expires_at: u64,
    invalidated: bool,
}
#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |v| v.as_secs())
}
fn failure() -> CredentialResolutionError {
    CredentialResolutionError::Unavailable
}
fn directory(home: &Path) -> Result<PathBuf, CredentialResolutionError> {
    let path = home.join("subscription-auth");
    fs::create_dir_all(&path).map_err(|_| failure())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|_| failure())?;
    }
    Ok(path)
}
fn path(home: &Path, id: &str) -> Result<PathBuf, CredentialResolutionError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(CredentialResolutionError::InvalidContract);
    }
    Ok(directory(home)?.join(format!("{id}.json")))
}
fn lock(home: &Path, id: &str) -> Result<(PathBuf, File), CredentialResolutionError> {
    let path = path(home, id)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path.with_extension("lock"))
        .map_err(|_| failure())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(35);
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => return Err(CredentialResolutionError::Timeout),
        }
    }
    Ok((path, file))
}
fn read(path: &Path) -> Result<Session, CredentialResolutionError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| failure())?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() as u64 > LIMIT {
        return Err(CredentialResolutionError::InvalidContract);
    }
    serde_json::from_slice(&bytes).map_err(|_| CredentialResolutionError::InvalidContract)
}
fn write(path: &Path, session: &Session) -> Result<(), CredentialResolutionError> {
    crate::repository::commit(
        path,
        &serde_json::to_vec(session).map_err(|_| CredentialResolutionError::InvalidContract)?,
    )
    .map_err(|_| failure())
}
// Claims returned directly by the TLS-authenticated token endpoint describe this session;
// they are never accepted from callers as proof of authorization.
fn claims(token: &str) -> Option<Value> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token.split('.').nth(1)?).ok()?).ok()
}
fn nonempty(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}
fn merge(
    tokens: Tokens,
    previous: Option<&Session>,
    generation: u64,
) -> Result<Session, CredentialResolutionError> {
    if !nonempty(&tokens.access_token) {
        return Err(CredentialResolutionError::InvalidContract);
    }
    let refresh_token = tokens
        .refresh_token
        .or_else(|| previous.map(|p| p.refresh_token.clone()))
        .filter(|v| nonempty(v))
        .ok_or(CredentialResolutionError::InvalidContract)?;
    let id_token = tokens
        .id_token
        .or_else(|| previous.map(|p| p.id_token.clone()))
        .ok_or(CredentialResolutionError::InvalidContract)?;
    let access = claims(&tokens.access_token).ok_or(CredentialResolutionError::InvalidContract)?;
    let identity = claims(&id_token).ok_or(CredentialResolutionError::InvalidContract)?;
    let subject = identity["sub"]
        .as_str()
        .filter(|v| nonempty(v))
        .ok_or(CredentialResolutionError::InvalidContract)?;
    if previous.is_some_and(|old| {
        claims(&old.id_token)
            .and_then(|v| v["sub"].as_str().map(str::to_owned))
            .as_deref()
            != Some(subject)
    }) {
        return Err(CredentialResolutionError::StaleBinding);
    }
    let account_id = identity["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .or_else(|| access["https://api.openai.com/auth"]["chatgpt_account_id"].as_str())
        .filter(|v| nonempty(v))
        .ok_or(CredentialResolutionError::InvalidContract)?
        .to_owned();
    if previous.is_some_and(|p| p.account_id != account_id) {
        return Err(CredentialResolutionError::StaleBinding);
    }
    let expires_at = access["exp"]
        .as_u64()
        .filter(|expiry| *expiry > now())
        .ok_or(CredentialResolutionError::InvalidContract)?;
    Ok(Session {
        generation,
        access_token: tokens.access_token,
        refresh_token,
        id_token,
        account_id,
        expires_at,
        invalidated: false,
    })
}
fn credential(session: Session) -> ResolvedCredential {
    ResolvedCredential {
        material: velune_gateway::CredentialMaterial::Http {
            authorization: velune_gateway::HttpAuthorization::Bearer(session.access_token),
            account_id: Some(session.account_id.clone()),
        },
        recovery: velune_gateway::CredentialRecovery::Refreshable {
            identity: format!("{}:{}", session.generation, session.account_id),
        },
        request_capabilities: Default::default(),
    }
}
async fn exchange(form: &[(&str, &str)]) -> Result<Tokens, CredentialResolutionError> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| failure())?
        .post(format!("{ISSUER}/oauth/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(form.iter().copied())
                .finish(),
        )
        .send()
        .await
        .map_err(|_| failure())?;
    let status = response.status();
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
        if bytes.len() + chunk.len() > LIMIT as usize {
            return Err(CredentialResolutionError::InvalidContract);
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let code = serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| {
            v["error"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| v["error"]["code"].as_str().map(str::to_owned))
        });
        return Err(
            if status == reqwest::StatusCode::UNAUTHORIZED
                || code.as_deref().is_some_and(|v| {
                    matches!(
                        v,
                        "invalid_grant"
                            | "refresh_token_reused"
                            | "refresh_token_expired"
                            | "refresh_token_invalidated"
                    )
                })
            {
                CredentialResolutionError::AuthenticationExpired
            } else {
                failure()
            },
        );
    }
    serde_json::from_slice(&bytes).map_err(|_| CredentialResolutionError::InvalidContract)
}
fn runtime() -> Result<tokio::runtime::Runtime, CredentialResolutionError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| failure())
}

/// The worker owns token rotation through durable commit even if its caller cancels a turn.
pub(crate) async fn resolve(
    home: PathBuf,
    id: String,
    generation: u64,
    rejected: Option<String>,
) -> Result<ResolvedCredential, CredentialResolutionError> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let (path, _lock) = lock(&home, &id)?;
            let mut session = read(&path)?;
            if session.generation != generation {
                return Err(CredentialResolutionError::StaleBinding);
            }
            if session.invalidated {
                return Err(CredentialResolutionError::AuthenticationExpired);
            }
            let refresh = rejected
                .as_ref()
                .is_some_and(|token| *token == session.access_token)
                || session.expires_at <= now().saturating_add(60);
            if !refresh {
                return Ok(credential(session));
            }
            let result = runtime()?.block_on(exchange(&[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
                ("refresh_token", &session.refresh_token),
            ]));
            let tokens = match result {
                Ok(tokens) => tokens,
                Err(CredentialResolutionError::AuthenticationExpired) => {
                    session.invalidated = true;
                    write(&path, &session)?;
                    return Err(CredentialResolutionError::AuthenticationExpired);
                }
                Err(error) => return Err(error),
            };
            let rotated = tokens.refresh_token.clone();
            let replacement = match merge(tokens, Some(&session), generation) {
                Ok(value) => value,
                Err(error) => {
                    if let Some(refresh) = rotated.filter(|v| nonempty(v)) {
                        session.refresh_token = refresh;
                    }
                    session.invalidated = true;
                    write(&path, &session)?;
                    return Err(error);
                }
            };
            write(&path, &replacement)?;
            Ok(credential(replacement))
        })();
        let _ = send.send(result);
    });
    receive.await.map_err(|_| failure())?
}
pub(crate) async fn invalidate(home: PathBuf, id: String, generation: u64, rejected: String) {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let (path, _lock) = lock(&home, &id)?;
            let mut session = read(&path)?;
            if session.generation == generation && session.access_token == rejected {
                session.invalidated = true;
                write(&path, &session)?;
            }
            Ok::<_, CredentialResolutionError>(())
        })();
        if result.is_err() {
            tracing::warn!(event = "subscription_invalidation_persist_failed");
        }
        let _ = send.send(());
    });
    let _ = receive.await;
}
pub(crate) fn status(home: &Path, id: &str, generation: u64) -> (bool, Option<String>) {
    let Some(session) = path(home, id)
        .ok()
        .and_then(|p| read(&p).ok())
        .filter(|s| s.generation == generation)
    else {
        return (false, None);
    };
    let suffix: String = session
        .account_id
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (!session.invalidated, Some(format!("账户 · {suffix}")))
}
pub(crate) fn remove(home: &Path, id: &str) -> Result<(), crate::Error> {
    let (path, _lock) = lock(home, id).map_err(|_| crate::Error::invalid("无法锁定订阅认证"))?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(crate::Error::invalid("无法删除订阅认证")),
    }
}

pub(crate) struct Login {
    home: PathBuf,
    credential_id: String,
    committed: bool,
    cancelled: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<(), String>>,
    finished: bool,
    url: Option<String>,
}
impl Login {
    pub(crate) fn start(home: PathBuf, id: String, generation: u64) -> Result<Self, crate::Error> {
        let listener = TcpListener::bind("127.0.0.1:1455").map_err(|_| {
            crate::Error::invalid("登录回调端口 1455 已被占用；请结束其他登录后重试。")
        })?;
        listener.set_nonblocking(true)?;
        let mut random = [0u8; 64];
        getrandom::fill(&mut random).map_err(|_| crate::Error::invalid("无法生成登录随机数"))?;
        let verifier = URL_SAFE_NO_PAD.encode(random);
        use sha2::Digest;
        let challenge = URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
        getrandom::fill(&mut random).map_err(|_| crate::Error::invalid("无法生成登录随机数"))?;
        let state = URL_SAFE_NO_PAD.encode(random);
        let mut url = url::Url::parse(&format!("{ISSUER}/oauth/authorize"))
            .map_err(|_| crate::Error::invalid("登录地址"))?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", CALLBACK),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("state", &state),
            ("originator", "codex_cli_rs"),
        ]);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&cancelled);
        let (send, result) = mpsc::channel();
        let worker_home = home.clone();
        let worker_id = id.clone();
        std::thread::spawn(move || {
            let result = login_worker(
                listener,
                &cancel,
                &state,
                &verifier,
                &worker_home,
                &worker_id,
                generation,
            )
            .map_err(|_| "订阅登录未完成；请重试。".to_owned());
            if send.send(result).is_err() {
                let _ = remove(&worker_home, &worker_id);
            }
        });
        Ok(Self {
            home,
            credential_id: id,
            committed: false,
            cancelled,
            result,
            finished: false,
            url: Some(url.into()),
        })
    }
    pub(crate) fn credential_id(&self) -> &str {
        &self.credential_id
    }
    pub(crate) fn commit(&mut self) {
        self.committed = true;
    }
    pub(crate) fn is_running(&self) -> bool {
        !self.finished
    }
    pub(crate) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub(crate) fn poll(&mut self) -> Value {
        let mut events = Vec::new();
        if let Some(url) = self.url.take() {
            events.push(json!({"type":"notify","notification":{"kind":"url","url":url,"instructions":"在浏览器中选择此提供商对应的 ChatGPT 账户。"}}));
        }
        match self.result.try_recv() {
            Ok(result) => {
                self.finished = true;
                let result = if self.cancelled.load(Ordering::SeqCst) {
                    Err("登录已取消".into())
                } else {
                    result
                };
                events.push(match result { Ok(()) => json!({"type":"result","ok":true}), Err(error) => json!({"type":"result","ok":false,"error":error,"cancelled":self.cancelled.load(Ordering::SeqCst)}) });
            }
            Err(mpsc::TryRecvError::Disconnected) if !self.finished => {
                self.finished = true;
                events.push(json!({"type":"result","ok":false,"error":"登录进程已结束"}));
            }
            _ => {}
        }
        json!({"running":!self.finished,"events":events})
    }
}
impl Drop for Login {
    fn drop(&mut self) {
        self.cancel();
        if !self.committed {
            let _ = remove(&self.home, &self.credential_id);
        }
    }
}
fn login_worker(
    listener: TcpListener,
    cancel: &AtomicBool,
    state: &str,
    verifier: &str,
    home: &Path,
    id: &str,
    generation: u64,
) -> Result<(), CredentialResolutionError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    while std::time::Instant::now() < deadline && !cancel.load(Ordering::SeqCst) {
        let (mut stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(_) => return Err(failure()),
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| failure())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| failure())?;
        let mut request = Vec::new();
        let read_deadline = (std::time::Instant::now() + Duration::from_secs(2)).min(deadline);
        while request.len() < 8192
            && std::time::Instant::now() < read_deadline
            && !cancel.load(Ordering::SeqCst)
        {
            let mut byte = [0u8; 1];
            stream
                .set_read_timeout(Some(
                    read_deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .max(Duration::from_millis(1)),
                ))
                .map_err(|_| failure())?;
            match stream.read(&mut byte) {
                Ok(1) => {}
                _ => break,
            }
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        if !request.ends_with(b"\r\n\r\n") {
            continue;
        }
        let first = std::str::from_utf8(&request)
            .ok()
            .and_then(|s| s.lines().next())
            .unwrap_or("");
        let mut parts = first.split_whitespace();
        let url = if parts.next() == Some("GET") {
            parts
                .next()
                .and_then(|s| url::Url::parse(&format!("http://localhost{s}")).ok())
        } else {
            None
        };
        let Some(url) = url.filter(|u| u.path() == "/auth/callback") else {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            continue;
        };
        let query: std::collections::BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        if query.get("state").map(String::as_str) != Some(state) {
            let _ = stream.write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            continue;
        }
        let code = query
            .get("code")
            .filter(|v| nonempty(v))
            .ok_or_else(failure)?;
        if query.contains_key("error") || cancel.load(Ordering::SeqCst) {
            return Err(failure());
        }
        let tokens = runtime()?.block_on(exchange(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", CALLBACK),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ]))?;
        let session = merge(tokens, None, generation)?;
        let (path, _lock) = lock(home, id)?;
        if cancel.load(Ordering::SeqCst) {
            return Err(failure());
        }
        write(&path, &session)?;
        let body = "登录完成，可关闭此页面并返回 Velune。";
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        return Ok(());
    }
    Err(failure())
}

/// Exact usage purpose: subscription credentials are not a general HTTP credential source.
pub(crate) const USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";
pub(crate) async fn read_usage(
    home: PathBuf,
    id: String,
    generation: u64,
) -> Result<Value, CredentialResolutionError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| failure())?;
    let mut auth = resolve(home.clone(), id.clone(), generation, None).await?;
    for attempt in 0..2 {
        let velune_gateway::CredentialMaterial::Http {
            authorization,
            account_id: Some(account_id),
        } = &auth.material
        else {
            return Err(CredentialResolutionError::UnauthorizedTarget);
        };
        let mut bearer =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", authorization.secret()))
                .map_err(|_| CredentialResolutionError::InvalidContract)?;
        bearer.set_sensitive(true);
        let mut response = client
            .get(USAGE_ENDPOINT)
            .header("authorization", bearer)
            .header("chatgpt-account-id", account_id)
            .header("originator", "codex_cli_rs")
            .header("user-agent", "codex_cli_rs/0.162.0 Velune/0.1")
            .send()
            .await
            .map_err(|_| failure())?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if attempt == 0 {
                auth = resolve(
                    home.clone(),
                    id.clone(),
                    generation,
                    Some(
                        auth.token()
                            .ok_or(CredentialResolutionError::InvalidContract)?
                            .to_owned(),
                    ),
                )
                .await?;
                continue;
            }
            invalidate(
                home,
                id,
                generation,
                auth.token()
                    .ok_or(CredentialResolutionError::InvalidContract)?
                    .to_owned(),
            )
            .await;
            return Err(CredentialResolutionError::AuthenticationExpired);
        }
        if !response.status().is_success() {
            return Err(failure());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
            if bytes.len() + chunk.len() > LIMIT as usize {
                return Err(CredentialResolutionError::InvalidContract);
            }
            bytes.extend_from_slice(&chunk);
        }
        return serde_json::from_slice(&bytes)
            .map_err(|_| CredentialResolutionError::InvalidContract);
    }
    Err(failure())
}

/// The full subscription identity stays private; email is never used for deduplication.
pub(crate) fn same_subscription(home: &Path, left: &str, right: &str) -> bool {
    let identity = |id: &str| -> Option<(String, String)> {
        let session = read(&path(home, id).ok()?).ok()?;
        let subject = claims(&session.id_token)?["sub"]
            .as_str()
            .filter(|v| nonempty(v))?
            .to_owned();
        Some((subject, session.account_id))
    };
    match (identity(left), identity(right)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}
