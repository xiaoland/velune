//! Interactive authentication belongs to the selected source adapter. The core
//! retains only UI events; credentials are written by the SDK to its source.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStderr, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
};

/// Non-secret Pi authentication-source coordinates provided by application composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub kind: String,
    pub harness_type_id: String,
    #[serde(default)]
    pub source_instance_id: Option<String>,
    pub provider_id: String,
    pub settings: BTreeMap<String, String>,
}
pub struct Login {
    child: Child,
    input: ChildStdin,
    events: Receiver<Result<Value, String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    finished: bool,
    prompt: Option<String>,
    source: Source,
}

impl Login {
    pub fn source(&self) -> &Source {
        &self.source
    }
    pub fn is_running(&self) -> bool {
        !self.finished
    }
    pub fn start(resources: &Path, home: &Path, source: &Value) -> Result<Self, String> {
        let source: Source = serde_json::from_value(source.clone())
            .map_err(|error| format!("invalid authentication source: {error}"))?;
        let node = source
            .settings
            .get("nodeBinary")
            .filter(|value| Path::new(value).is_absolute())
            .ok_or("authentication requires an absolute Node executable")?;
        let imported = source.settings.contains_key("bindingProtocol");
        let imported_openai_oauth = imported
            && source.provider_id == "openai"
            && source.settings.get("bindingProtocol").map(String::as_str) == Some("responsesV1")
            && source.settings.get("bindingEndpoint").map(String::as_str)
                == Some("https://api.openai.com/v1");
        if imported && !imported_openai_oauth {
            return Err("导入的 provider 不支持交互式登录，请先在 Pi 中完成认证".into());
        }
        let helper = resources.join("pi_auth.mjs");
        if source.kind != "harness" || source.harness_type_id != "pi" || !helper.is_file() {
            return Err("authentication source adapter is unavailable".into());
        }
        let device_id = device_id(home)?;
        let mut child = Command::new(node)
            .arg(helper)
            .arg("--operation")
            .arg("login")
            .arg("--source-json")
            .arg(
                serde_json::to_string(&source)
                    .map_err(|error| format!("invalid authentication source: {error}"))?,
            )
            .arg("--device-id")
            .arg(device_id)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("authentication adapter could not start: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or("authentication input unavailable")?;
        let output = child
            .stdout
            .take()
            .ok_or("authentication events unavailable")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("authentication error output unavailable")?;
        let stderr_state = Arc::new(Mutex::new(Vec::new()));
        let stderr_copy = Arc::clone(&stderr_state);
        std::thread::spawn(move || {
            let mut stderr: ChildStderr = stderr;
            let mut buffer = [0_u8; 4096];
            while let Ok(size) = stderr.read(&mut buffer) {
                if size == 0 {
                    break;
                }
                if let Ok(mut value) = stderr_copy.lock() {
                    let remaining = 65537usize.saturating_sub(value.len());
                    value.extend_from_slice(&buffer[..size.min(remaining)]);
                }
            }
        });
        let (sender, events) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                // Bound each event independently; do not materialize arbitrary child output.
                let mut bytes = Vec::new();
                let read = Read::by_ref(&mut reader)
                    .take(65537)
                    .read_until(b'\n', &mut bytes);
                match read {
                    Ok(0) => break,
                    Ok(_) if bytes.len() <= 65536 && bytes.last() == Some(&b'\n') => {
                        let event = serde_json::from_slice::<Value>(&bytes)
                            .map_err(|error| error.to_string());
                        if sender.send(event).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(format!(
                            "authentication adapter event read failed: {error}"
                        )));
                        break;
                    }
                    Ok(_) => {
                        let _ = sender.send(Err(
                            "authentication adapter event exceeded the 64 KiB record limit".into(),
                        ));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            input,
            events,
            stderr: stderr_state,
            finished: false,
            prompt: None,
            source,
        })
    }
    fn poll(&mut self) -> Result<Value, String> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            let event = event
                .map_err(|detail| format!("invalid authentication adapter event: {detail}"))?;
            match event["type"].as_str() {
                Some("prompt") => self.prompt = event["id"].as_str().map(str::to_owned),
                Some("result") => {
                    self.finished = true;
                    self.prompt = None;
                }
                Some("notify") => {
                    if event["notification"]["kind"] == "prompt_cancelled"
                        && event["notification"]["id"].as_str() == self.prompt.as_deref()
                    {
                        self.prompt = None;
                    }
                }
                _ => return Err("unsupported authentication adapter event".into()),
            }
            events.push(event);
        }
        let exit_status = if !self.finished {
            self.child
                .try_wait()
                .map_err(|error| format!("authentication adapter state unavailable: {error}"))?
        } else {
            None
        };
        if let Some(exit_status) = exit_status {
            // Drain after exit: the reader may still be delivering its final line.
            match self
                .events
                .recv_timeout(std::time::Duration::from_millis(50))
            {
                Ok(Ok(event)) if event["type"] == "result" => {
                    self.finished = true;
                    self.prompt = None;
                    events.push(event);
                }
                Ok(Ok(event)) => events.push(event),
                Ok(Err(detail)) => {
                    self.finished = true;
                    self.prompt = None;
                    let stderr = self.failure_stderr();
                    events.push(json!({"type":"result","ok":false,"error":format!("认证适配器事件失败（{exit_status}）：{detail}；stderr：{stderr}")}));
                }
                Err(error) => {
                    self.finished = true;
                    self.prompt = None;
                    let stderr = self.failure_stderr();
                    events.push(json!({"type":"result","ok":false,"error":format!("认证适配器退出（{exit_status}）：{error}；stderr：{stderr}")}));
                }
            }
        }
        Ok(json!({"running":!self.finished,"events":events}))
    }
    fn failure_stderr(&self) -> String {
        match self.stderr.lock() {
            Ok(value) => {
                let detail = String::from_utf8_lossy(&value).trim().to_owned();
                if value.len() >= 65537 {
                    format!("{detail}；stderr 已截断（64 KiB）")
                } else {
                    detail
                }
            }
            Err(error) => format!("stderr 缓存无法读取：{error}"),
        }
    }
    fn reply(&mut self, payload: &Value) -> Result<Value, String> {
        let id = payload["id"]
            .as_str()
            .ok_or("authentication prompt identity is required")?;
        if self.finished || self.prompt.as_deref() != Some(id) {
            return Err("authentication prompt is no longer active".into());
        }
        let value = payload["value"]
            .as_str()
            .ok_or("authentication answer is required")?;
        if value.len() > 16384 {
            return Err("authentication answer is too large".into());
        }
        serde_json::to_writer(
            &mut self.input,
            &json!({"type":"answer","id":id,"value":value}),
        )
        .map_err(|error| format!("authentication input failed: {error}"))?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|error| format!("authentication input failed: {error}"))?;
        self.prompt = None;
        Ok(json!({"running":true,"events":[]}))
    }
    fn cancel(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.finished = true;
        self.prompt = None;
    }
}
impl Drop for Login {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn handle(
    session: &mut Option<Login>,
    resources: &Path,
    home: &Path,
    payload: &Value,
) -> Result<Value, String> {
    match payload["operation"].as_str().unwrap_or("poll") {
        "start" => {
            if session.as_ref().is_some_and(Login::is_running) {
                return Err("authentication is already running".into());
            }
            let source: Value = serde_json::from_str(
                payload["source"]
                    .as_str()
                    .ok_or("authentication source is required")?,
            )
            .map_err(|error| format!("invalid authentication source: {error}"))?;
            *session = Some(Login::start(resources, home, &source)?);
            Ok(json!({"running":true,"events":[]}))
        }
        "inspect" => {
            let source: Source = serde_json::from_str(
                payload["source"]
                    .as_str()
                    .ok_or("authentication source is required")?,
            )
            .map_err(|error| format!("invalid authentication source: {error}"))?;
            let node = source
                .settings
                .get("nodeBinary")
                .filter(|value| Path::new(value).is_absolute())
                .ok_or("authentication requires an absolute Node executable")?;
            if source.kind != "harness" || source.harness_type_id != "pi" {
                return Err("authentication source adapter is unavailable".into());
            }
            let helper = if source.settings.contains_key("bindingProtocol") {
                resources.join("pi_provider_import.mjs")
            } else {
                resources.join("pi_auth.mjs")
            };
            let output = Command::new(node)
                .arg(helper)
                .arg("--operation")
                .arg("inspect")
                .arg("--provider-id")
                .arg(&source.provider_id)
                .arg("--source-json")
                .arg(
                    serde_json::to_string(&source)
                        .map_err(|error| format!("invalid authentication source: {error}"))?,
                )
                .env_clear()
                .stdin(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(|error| format!("authentication source inspection failed: {error}"))?;
            if output.stdout.len() > 65536 {
                return Err("authentication source inspection output exceeded 64 KiB".into());
            }
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                return Err(if detail.is_empty() {
                    format!(
                        "authentication source inspection failed (exit status: {})",
                        output.status
                    )
                } else {
                    format!(
                        "authentication source inspection failed (exit status: {}): {detail}",
                        output.status
                    )
                });
            }
            let metadata: Value = serde_json::from_slice(&output.stdout)
                .map_err(|error| format!("invalid authentication source metadata: {error}"))?;
            Ok(json!({"metadata":metadata}))
        }
        "poll" => session
            .as_mut()
            .map_or(Ok(json!({"running":false,"events":[]})), Login::poll),
        "reply" => session
            .as_mut()
            .ok_or("authentication is not running")?
            .reply(payload),
        "cancel" => {
            if let Some(login) = session.as_mut() {
                login.cancel();
            }
            Ok(json!({"running":false,"events":[{"type":"result","ok":false,"cancelled":true}]}))
        }
        _ => Err("unsupported authentication operation".into()),
    }
}

// The host identifier is ordinary installation metadata, not an account credential.
fn device_id(home: &Path) -> Result<String, String> {
    let path = home.join("authentication-host-id");
    match std::fs::read_to_string(&path) {
        Ok(id) if id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') => {
            return Ok(id);
        }
        Ok(_) => return Err("invalid authentication host identity".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("authentication host identity unavailable".into()),
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("authentication host identity generation failed: {error}"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let id = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("authentication host identity could not be saved: {error}"))?;
    file.write_all(id.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("authentication host identity could not be saved: {error}"))?;
    Ok(id)
}
