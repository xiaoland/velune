// Small, credential-blind client for Pi's versioned RPC subprocess.
//
// The host owns the child process and JSONL transport, and injects a
// optional loopback Velune gateway configuration. Pi owns session files,
// message history, tools, and native provider authentication. This Rust client
// never reads environment variables or Pi auth files.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

mod port;
pub use port::*;

const MAX_STDERR_BYTES: usize = 1024 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);
// Pi may load an existing session, extensions, and project resources before
// answering the readiness RPC. Keep startup more patient without extending
// ordinary prompt, selection, and cancellation commands.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Pi binary path is required")]
    MissingBinary,
    #[error("Pi binary path must be absolute: {0}")]
    RelativeBinary(PathBuf),
    #[error("Pi RPC command must be a JSON object")]
    InvalidCommand,
    #[error("Pi RPC command type is required")]
    MissingCommandType,
    #[error("Pi RPC child exited before responding")]
    ChildExited,
    #[error("Pi RPC response timed out: {0}")]
    Timeout(String),
    #[error("Pi operation not admitted: native scope transition busy")]
    ScopeTransitionBusy,
    #[error("Pi RPC response failed: {0}")]
    CommandFailed(String),
    #[error("Pi RPC transport ended without a confirmed result: {0}")]
    Transport(String),
    #[error("Pi SDK 接入失败: {0}")]
    Sdk(String),
    #[error("会话执行尚未准备")]
    NotStarted,
    #[error(transparent)]
    Execution(#[from] crate::execution::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Application-owned Pi subprocess paths and session selection.
/// Upstream authentication is not part of the execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub execution: crate::execution::ExecutionTarget,
    pub os_home: Option<PathBuf>,
    #[serde(skip)]
    pub mcp_servers: Vec<crate::mcp::Server>,
    pub binary: PathBuf,
    pub node_binary: Option<PathBuf>,
    pub sdk_helper: Option<PathBuf>,
    pub extension: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    pub provider: Option<String>,
    pub model: Option<String>,
    /// SDK launcher and application-owned catalog/selection, distinct from PI_HOME.
    pub rpc_entry: Option<PathBuf>,
    pub models_path: Option<PathBuf>,
    pub selection_file: Option<PathBuf>,
    /// Per-selected-model reserve tokens, for this SDK process only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_compaction_reserves: BTreeMap<String, u32>,
    /// Ephemeral loopback gateway token. It is injected into the child
    /// environment and never serialized into app configuration or snapshots.
    #[serde(skip_serializing)]
    pub gateway_token: Option<String>,
    pub session_dir: Option<PathBuf>,
    pub session: Option<PathBuf>,
    /// Run `session` as a side chat: a copy held in memory, never saved.
    #[serde(default)]
    pub side: bool,
    pub name: Option<String>,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.binary.as_os_str().is_empty() {
            return Err(Error::MissingBinary);
        }
        if !self.binary.is_absolute() {
            return Err(Error::RelativeBinary(self.binary.clone()));
        }
        for path in [
            &self.node_binary,
            &self.rpc_entry,
            &self.models_path,
            &self.selection_file,
            &self.sdk_helper,
            &self.extension,
            &self.agent_dir,
            &self.working_dir,
            &self.session_dir,
            &self.session,
        ]
        .into_iter()
        .flatten()
        {
            if !path.is_absolute() {
                return Err(Error::RelativeBinary(path.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
enum Incoming {
    Record(Value),
    Error(String),
    Closed,
}

struct StderrCapture {
    bytes: Vec<u8>,
    truncated: bool,
    done: bool,
}

type SharedStderr = Arc<(Mutex<StderrCapture>, Condvar)>;

fn read_record(reader: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
    let mut record = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if record.is_empty() {
                None
            } else {
                Some(record)
            });
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(buffer.len(), |index| index + 1);
        record.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(Some(record));
        }
    }
}

/// Levels Pi 1.0.2 `set_thinking_level` accepts.
pub const PI_THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// One long-lived Pi RPC subprocess. Records are retained until the host asks
/// for them, so the UI can poll without losing fast stream events.
pub struct Client {
    port: Option<port::Port>,
    child: crate::process_tree::ProcessTree,
    stdin: Option<crate::transport::PipeWriter>,
    incoming: Receiver<Incoming>,
    pending: crate::transport::DeferredRecords,
    prepared_snapshot: Option<Value>,
    next_id: u64,
    stderr: SharedStderr,
    terminal_error: Option<String>,
    terminal_reported: bool,
    pub config: Config,
    _mcp: crate::mcp::Prepared,
}

impl Config {
    /// Verify that the selected CLI belongs to the supported SDK before starting services.
    pub fn validate_sdk(&self) -> Result<()> {
        self.validate()?;
        let node = self
            .node_binary
            .as_ref()
            .ok_or_else(|| Error::Sdk("请配置所选运行时的 Node 可执行文件".into()))?;
        let entry = self
            .rpc_entry
            .as_ref()
            .ok_or_else(|| Error::Sdk("SDK 入口不可用".into()))?;
        let mut output = crate::process_tree::command(node);
        output
            .arg(entry)
            .arg("--cli")
            .arg(&self.binary)
            .arg("--runtime-type")
            .arg(crate::version::RuntimeKind::Pi.id())
            .arg("--validate");
        let output = self.execution.output(output, None, self.os_home.as_deref());
        match output {
            Ok(_) => Ok(()),
            Err(error) => Err(Error::Sdk(format!("Pi SDK 入口验证失败：{error}"))),
        }
    }
}

impl Client {
    /// Resource preparation, process startup and the transport prefix perform IO.
    /// Call before installing this connection in the nonblocking session owner.
    pub fn spawn(mut config: Config) -> Result<Self> {
        config.validate()?;
        let mcp = crate::mcp::Prepared::new(
            &config.mcp_servers,
            &config.execution,
            config.os_home.as_deref(),
        )
        .map_err(|error| Error::Sdk(error.to_string()))?;
        config.mcp_servers = mcp.servers.clone();
        let node = config
            .node_binary
            .as_ref()
            .ok_or_else(|| Error::Sdk("请配置所选运行时的 Node 可执行文件".into()))?;
        let entry = config
            .rpc_entry
            .as_ref()
            .ok_or_else(|| Error::Sdk("SDK 入口不可用".into()))?;
        let mut command = crate::process_tree::command(node);
        command
            .arg(entry)
            .arg("--cli")
            .arg(&config.binary)
            .arg("--runtime-type")
            .arg(crate::version::RuntimeKind::Pi.id());
        for (name, path) in [
            ("--models-path", &config.models_path),
            ("--selection-file", &config.selection_file),
            ("--agent-dir", &config.agent_dir),
        ] {
            if let Some(path) = path {
                command.arg(name).arg(path);
            }
        }
        if let Some(value) = &config.provider {
            command.arg("--provider").arg(value);
        }
        if let Some(value) = &config.model {
            command.arg("--model").arg(value);
        }
        if let Some(value) = &config.session_dir {
            command.arg("--session-dir").arg(value);
        }
        if let Some(value) = &config.session {
            command.arg("--session").arg(value);
        }
        if config.side {
            command.arg("--side");
        }
        if let Some(value) = &config.name {
            command.arg("--name").arg(value);
        }
        if let Some(extension) = &config.extension {
            command.arg("--extension").arg(extension);
        }
        if let Some(dir) = &config.working_dir {
            command.current_dir(dir);
        }
        if let Some(agent_dir) = &config.agent_dir {
            command.env("PI_CODING_AGENT_DIR", agent_dir);
        }
        if let Some(token) = &config.gateway_token {
            command.env("VELUNE_GATEWAY_TOKEN", token);
        }
        let mcp_servers: serde_json::Map<String, Value> = config
            .mcp_servers
            .iter()
            .map(|server| (server.name.clone(), server.http()))
            .collect();
        command.env("VELUNE_MCP_SERVERS", Value::Object(mcp_servers).to_string());
        command.env(
            "VELUNE_PI_COMPACTION_RESERVES",
            serde_json::to_string(&config.model_compaction_reserves)?,
        );
        let prepared = config
            .execution
            .prepare(command, config.os_home.as_deref())?;
        let mut command = prepared.command;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::process_tree::ProcessTree::spawn(&mut command)?;
        let stdin =
            crate::transport::PipeWriter::new(child.take_stdin().ok_or(Error::ChildExited)?)?;
        stdin.write(prepared.input_prefix, Instant::now() + STARTUP_TIMEOUT)?;
        let stdout = child.take_stdout().ok_or(Error::ChildExited)?;
        let stderr_output = child.take_stderr().ok_or(Error::ChildExited)?;
        let stderr: SharedStderr = Arc::new((
            Mutex::new(StderrCapture {
                bytes: Vec::new(),
                truncated: false,
                done: false,
            }),
            Condvar::new(),
        ));
        let stderr_state = Arc::clone(&stderr);
        thread::Builder::new()
            .name("velune-pi-rpc-stderr".into())
            .spawn(move || {
                let mut output = stderr_output;
                let mut chunk = [0_u8; 8192];
                loop {
                    match std::io::Read::read(&mut output, &mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(size) => {
                            let (state, _) = &*stderr_state;
                            if let Ok(mut state) = state.lock() {
                                let remaining = MAX_STDERR_BYTES.saturating_sub(state.bytes.len());
                                if remaining != 0 {
                                    let retained = size.min(remaining);
                                    state.bytes.extend_from_slice(&chunk[..retained]);
                                    if retained != size {
                                        state.truncated = true;
                                    }
                                } else {
                                    state.truncated = true;
                                }
                            }
                        }
                    }
                }
                let (state, ready) = &*stderr_state;
                if let Ok(mut state) = state.lock() {
                    state.done = true;
                    ready.notify_all();
                }
            })?;
        let (sender, incoming) = mpsc::sync_channel(64);
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        // The long-lived reader must not keep the startup operation open.
        let reader_span = tracing::info_span!(parent: None, "agent_runtime.rpc_reader");
        let stderr_for_reader = Arc::clone(&stderr);
        thread::Builder::new()
            .name("velune-pi-rpc-reader".into())
            .spawn(move || tracing::dispatcher::with_default(&dispatcher, || {
                let _entered=reader_span.enter();
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_record(&mut reader) {
                        Ok(None) => {
                            let (state, ready) = &*stderr_for_reader;
                            if let Ok(state) = state.lock() {
                                let _ = ready.wait_timeout_while(
                                    state,
                                    Duration::from_millis(100),
                                    |state| !state.done,
                                );
                            }
                            tracing::debug!(code="rpc_eof","Pi RPC reader closed");
                            let _ = sender.send(Incoming::Closed);
                            break;
                        }
                        Ok(Some(mut line)) => {
                            if line.last() == Some(&b'\n') {
                                line.pop();
                            }
                            if line.last() == Some(&b'\r') {
                                line.pop();
                            }
                            match serde_json::from_slice::<Value>(&line) {
                                Ok(value) => {
                                    if sender.send(Incoming::Record(value)).is_err() {
                                        break;
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(code="rpc_invalid_json","Pi RPC reader rejected invalid JSON");
                                    let _ = sender.send(Incoming::Error(format!("invalid Pi RPC JSON record: {error}")));
                                    break;
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(code="rpc_read_failed",io_kind=?error.kind(),"Pi RPC reader I/O failed");
                            let _ = sender.send(Incoming::Error(format!("Pi RPC read failed: {error}")));
                            break;
                        }
                    }
                }
            }))?;
        Ok(Self {
            port: None,
            child,
            stdin: Some(stdin),
            incoming,
            pending: Default::default(),
            prepared_snapshot: None,
            next_id: 1,
            stderr,
            terminal_error: None,
            terminal_reported: false,
            config,
            _mcp: mcp,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_reader_failure_batch(&mut self, records: Vec<Value>) {
        let (sender, incoming) = mpsc::sync_channel(64);
        for record in records {
            sender.send(Incoming::Record(record)).unwrap();
        }
        sender
            .send(Incoming::Error("synthetic reader failure".into()))
            .unwrap();
        self.incoming = incoming;
    }

    fn stderr_detail(&self, wait_for_eof: bool) -> String {
        let (state, ready) = &*self.stderr;
        let mut state = match state.lock() {
            Ok(state) => state,
            Err(_) => return String::new(),
        };
        if wait_for_eof && !state.done {
            let (updated, _) = ready
                .wait_timeout_while(state, Duration::from_millis(100), |state| !state.done)
                .unwrap_or_else(|error| error.into_inner());
            state = updated;
        }
        let mut detail = String::from_utf8_lossy(&state.bytes).trim().to_owned();
        if state.truncated {
            detail.push_str("\n[Pi RPC stderr 已截断：诊断缓存达到上限]");
        }
        detail
    }

    fn child_exit_error(&mut self) -> Error {
        let detail = self.stderr_detail(true);
        let status = self
            .child
            .try_wait()
            .ok()
            .flatten()
            .map(|status| format!("；进程状态：{status}"))
            .unwrap_or_default();
        if detail.is_empty() && status.is_empty() {
            Error::ChildExited
        } else if detail.is_empty() {
            Error::Transport(format!("Pi RPC child exited before responding{status}"))
        } else {
            Error::Transport(format!(
                "Pi RPC child exited before responding{status}；原始错误：{detail}"
            ))
        }
    }

    fn terminate(&mut self, cause: Option<String>) -> Error {
        if let Some(error) = &self.terminal_error {
            return Error::Transport(error.clone());
        }
        self.stdin.take();
        self.child.stop();
        let exit = match self.child_exit_error() {
            Error::Transport(detail) => detail,
            error => error.to_string(),
        };
        let detail = cause.map_or_else(|| exit.clone(), |cause| format!("{cause}；{exit}"));
        self.terminal_error = Some(detail.clone());
        Error::Transport(detail)
    }

    fn timeout_error(&mut self, method: &str) -> Error {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => format!("进程状态：{status}"),
            Ok(None) => "Pi RPC 子进程仍在运行".into(),
            Err(error) => format!("进程状态不可用：{error}"),
        };
        let detail = self.stderr_detail(false);
        if detail.is_empty() {
            Error::Timeout(format!("{method}；{status}"))
        } else {
            Error::Timeout(format!("{method}；{status}；原始错误：{detail}"))
        }
    }

    fn write_command_until(&mut self, mut command: Value, deadline: Instant) -> Result<String> {
        let class = crate::transport::WriteClass::Ordinary;
        if self.port.is_some() {
            return Err(Error::Transport(
                "Pi typed port enabled; synchronous mutations disabled".into(),
            ));
        }
        if let Some(error) = &self.terminal_error {
            return Err(Error::Transport(error.clone()));
        }
        let object = command.as_object_mut().ok_or(Error::InvalidCommand)?;
        let command_type = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(Error::MissingCommandType)?
            .to_string();
        if !object.contains_key("id") {
            let next = self
                .next_id
                .checked_add(1)
                .ok_or_else(|| Error::CommandFailed("Pi RPC request sequence exhausted".into()))?;
            object.insert(
                "id".into(),
                Value::String(format!("velune-{}", self.next_id)),
            );
            self.next_id = next;
        }
        let id = object["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or(Error::InvalidCommand)?
            .to_owned();
        let input = self.stdin.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Pi RPC input closed")
        })?;
        let frame = input.encode_json(
            &command,
            class,
            crate::transport::writer_policy_limit(),
            deadline,
        )?;
        let receipt = input
            .try_enqueue(frame, class, deadline)
            .map_err(|rejected| Error::Io(rejected.into_error()))?;
        if let Err(error) = receipt.wait() {
            return Err(self.terminate(Some(format!("Pi RPC command send unconfirmed: {error}"))));
        }
        Ok(format!("{command_type}\0{id}"))
    }

    fn receive(&mut self, method: &str, deadline: Instant) -> Result<Value> {
        if let Some(error) = &self.terminal_error {
            return Err(Error::Transport(error.clone()));
        }
        if Instant::now() >= deadline {
            let error = self.timeout_error(method);
            self.terminate(Some(error.to_string()));
            return Err(error);
        }
        if let Some(value) = self.pending.pop() {
            return Ok(value);
        }
        match self
            .incoming
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Incoming::Record(value)) => Ok(value),
            Ok(Incoming::Error(error)) => Err(self.terminate(Some(error))),
            Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => Err(self.terminate(None)),
            Err(RecvTimeoutError::Timeout) => {
                let error = self.timeout_error(method);
                self.terminate(Some(error.to_string()));
                Err(error)
            }
        }
    }

    /// Send a command and wait for its matching response. Other records stay
    /// buffered for `poll`, preserving event order across requests.
    fn request_with_timeout(&mut self, command: Value, timeout: Duration) -> Result<Value> {
        // Only fixed protocol commands enter diagnostics. Arbitrary caller
        // fields and unknown command names do not become log labels.
        let method = match command["type"].as_str() {
            Some(method @ ("get_available_models" | "get_state" | "prompt")) => method.to_owned(),
            _ => "command".to_owned(),
        };
        let started = Instant::now();
        let outcome = self.request_response(command, &method, timeout);
        let elapsed_ms = started.elapsed().as_millis();
        if let Err(error) = &outcome {
            let failure_kind = match error {
                Error::Timeout(_) => "timeout",
                Error::Transport(_) | Error::ChildExited | Error::Io(_) => "transport",
                _ => "command_failed",
            };
            tracing::warn!(target: "velune_agent_runtime", event = "native_rpc_failed",
                phase = "control_rpc", runtime_family = "pi", method, elapsed_ms, failure_kind);
        } else {
            tracing::debug!(target: "velune_agent_runtime", event = "native_rpc_completed",
                phase = "control_rpc", runtime_family = "pi", method, elapsed_ms);
        }
        outcome
    }

    fn request_response(
        &mut self,
        command: Value,
        method: &str,
        timeout: Duration,
    ) -> Result<Value> {
        let projection_sync = command["type"] == "prompt"
            && command["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("/velune-projection-sync "));
        if projection_sync {
            self.prepared_snapshot = None;
        }
        let deadline = Instant::now() + timeout;
        let key = self.write_command_until(command, deadline)?;
        let (_, id) = key.split_once('\0').expect("internal command key");
        let mut deferred = crate::transport::DeferredRecords::default();
        let result = loop {
            let value = match self.receive(method, deadline) {
                Ok(value) => value,
                Err(error) => break Err(error),
            };
            if projection_sync
                && value["type"] == "extension_ui_request"
                && value["method"] == "setStatus"
                && value["statusKey"] == "velune.session-metadata"
                && let Some(frame) = value["statusText"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                && frame["requestId"].as_str() == Some(id)
            {
                // This is the current RPC's result body, not an unrelated UI
                // event. Large native histories must not consume the bounded
                // queue reserved for deferred events and subsequent appends.
                if self.prepared_snapshot.is_none() {
                    if let Err(error) = Self::validate_projection_frame(&frame, id) {
                        break Err(error);
                    }
                    let header = json!({"type":"extension_ui_request","method":"setStatus",
                        "statusKey":"velune.session-metadata","statusText":json!({
                            "snapshotHeaderOnly":true,"snapshotRequestId":id,"sessionFile":frame["sessionFile"],"cwd":frame["cwd"],
                            "name":frame["name"],"createdUnixMs":frame["createdUnixMs"]
                        }).to_string()});
                    if let Err(error) = deferred.push(header) {
                        break Err(self.terminate(Some(error.to_string())));
                    }
                    self.prepared_snapshot = Some(value);
                }
                continue;
            }
            let matches = value.get("type").and_then(Value::as_str) == Some("response")
                && value.get("id").and_then(Value::as_str) == Some(id);
            if matches {
                if value.get("success") == Some(&Value::Bool(false)) {
                    break Err(Error::CommandFailed(
                        value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Pi command failed")
                            .to_string(),
                    ));
                }
                break Ok(value);
            }
            if let Err(error) = deferred.push(value) {
                break Err(self.terminate(Some(error.to_string())));
            }
        };
        if let Err(error) = deferred.append(&mut self.pending) {
            return Err(self.terminate(Some(error.to_string())));
        }
        self.pending = deferred;
        if result.is_err() && projection_sync {
            self.prepared_snapshot = None;
            // A failed RPC returns no body. Withdraw only its synthetic header,
            // preserving unrelated native events and real extension diagnostics.
            let mut retained = crate::transport::DeferredRecords::default();
            for event in self.pending.drain() {
                let own_header = event["type"] == "extension_ui_request"
                    && event["method"] == "setStatus"
                    && event["statusKey"] == "velune.session-metadata"
                    && event["statusText"]
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                        .is_some_and(|frame| {
                            frame["snapshotHeaderOnly"] == true
                                && frame["snapshotRequestId"].as_str() == Some(id)
                        });
                if !own_header {
                    retained
                        .push(event)
                        .map_err(|error| self.terminate(Some(error.to_string())))?;
                }
            }
            self.pending = retained;
        }
        result
    }

    pub fn request(&mut self, command: Value) -> Result<Value> {
        self.request_with_timeout(command, RESPONSE_TIMEOUT)
    }

    /// Pi takes its fixed thinking levels for models marked `reasoning`.
    pub fn models(&mut self) -> Result<Vec<crate::native::ModelOption>> {
        let result = self.request(json!({"type":"get_available_models"}))?;
        result["data"]["models"]
            .as_array()
            .ok_or_else(|| Error::Sdk("Pi 模型目录格式无效".into()))?
            .iter()
            .filter(|m| !matches!(m["provider"].as_str(), Some("velune" | "velune-gateway")))
            .map(|model| {
                let id = model["id"]
                    .as_str()
                    .ok_or_else(|| Error::Sdk("Pi 模型身份为空".into()))?;
                Ok(crate::native::ModelOption {
                    model: crate::native::RuntimeModel {
                        provider_id: model["provider"].as_str().map(str::to_owned),
                        id: id.into(),
                        name: model["name"].as_str().unwrap_or(id).into(),
                    },
                    reasoning_efforts: if model["reasoning"] == true {
                        PI_THINKING_LEVELS.map(str::to_owned).to_vec()
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect()
    }
    pub fn poll(&mut self) -> Vec<Value> {
        if self.port.is_some() {
            return vec![]; // typed owner must consume ordered poll_pi evidence
        }
        let mut records = self.pending.drain_batch(64);
        if let Some(error) = &self.terminal_error {
            if !std::mem::replace(&mut self.terminal_reported, true) {
                records.push(json!({"type":"velune_error","error":error}));
            }
            return records;
        }
        for _ in records.len()..64 {
            let Ok(item) = self.incoming.try_recv() else {
                break;
            };
            match item {
                Incoming::Record(value) => records.push(value),
                Incoming::Error(error) => {
                    let error = self.terminate(Some(error));
                    self.terminal_reported = true;
                    records.push(json!({"type":"velune_error","error":error.to_string()}));
                    break;
                }
                Incoming::Closed => {
                    let error = self.terminate(None);
                    self.terminal_reported = true;
                    records.push(json!({"type":"velune_error","error":error.to_string()}));
                    break;
                }
            }
        }
        records
    }

    /// Blocking readiness/history preparation before handing the client to PiSession.
    /// Disabled once the typed port is enabled.
    /// The live owner installs the atomic ProjectionSync frame in event order.
    pub fn state(&mut self) -> Result<(Value, Value)> {
        let mut state = self.request_with_timeout(json!({"type":"get_state"}), STARTUP_TIMEOUT)?;
        let request_id = format!("velune-snapshot-{}", self.next_id);
        let metadata = self.request_with_timeout(
            json!({"id":request_id,"type":"prompt","message":format!("/velune-projection-sync {request_id}")}),
            STARTUP_TIMEOUT,
        )?;
        if metadata["data"]["disposition"] != "handled" {
            return Err(Error::Sdk("原生会话元数据查询未被扩展处理".into()));
        }
        let metadata_event = self
            .prepared_snapshot
            .take()
            .ok_or_else(|| Error::Sdk("原生会话投影缺少同源快照".into()))?;
        let messages = Self::projection_snapshot(&metadata_event, &request_id)?;
        // Session navigation may occur after get_state. Display identity must
        // come from the same frame as the returned history, never that old state.
        state["data"]["sessionFile"] = messages["nativeSession"]["sessionFile"].clone();
        state["data"]["sessionName"] = messages["nativeSession"]["name"].clone();
        Ok((state, messages))
    }

    /// Decode one self-contained native projection. No message list obtained
    /// by another RPC is allowed to borrow these source identities.
    pub(crate) fn projection_snapshot(metadata_event: &Value, request_id: &str) -> Result<Value> {
        let metadata_text = metadata_event["statusText"]
            .as_str()
            .ok_or_else(|| Error::Sdk("原生会话来源身份 statusText 必须为字符串".into()))?;
        let mut provenance: Value = serde_json::from_str(metadata_text)
            .map_err(|error| Error::Sdk(format!("原生会话来源身份 JSON 无效: {error}")))?;
        Self::validate_projection_frame(&provenance, request_id)?;
        let messages = provenance["messages"].take();
        let confirmations = provenance["identityConfirmations"].take();
        Ok(json!({
            "data":{"messages":messages},
            "identityConfirmations":confirmations,
            "nativeSession":{
                "sessionFile":provenance["sessionFile"],"cwd":provenance["cwd"],
                "name":provenance["name"],"createdUnixMs":provenance["createdUnixMs"]
            }
        }))
    }

    fn validate_projection_frame(provenance: &Value, request_id: &str) -> Result<()> {
        if provenance["requestId"].as_str() != Some(request_id) {
            return Err(Error::Sdk("原生会话投影不属于当前快照请求".into()));
        }
        if provenance["snapshotVersion"] != 1 {
            return Err(Error::Sdk(
                "原生会话投影缺少同源快照，请更新运行时桥接组件".into(),
            ));
        }
        let _: Vec<crate::conversation::MessageIdentityConfirmation> =
            serde_json::from_value(provenance["identityConfirmations"].clone())
                .map_err(|error| Error::Sdk(format!("原生会话身份确认格式无效: {error}")))?;
        let identities = provenance["messageIds"]
            .as_array()
            .ok_or_else(|| Error::Sdk("原生会话消息身份格式无效".into()))?;
        let native_messages = provenance["messages"]
            .as_array()
            .ok_or_else(|| Error::Sdk("原生会话消息格式无效".into()))?;
        if identities.len() != native_messages.len() {
            return Err(Error::Sdk(format!(
                "原生会话快照与来源身份不一致: messages={}, identities={}",
                native_messages.len(),
                identities.len()
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (message, identity) in native_messages.iter().zip(identities) {
            let id = identity
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| Error::Sdk("原生会话消息身份为空".into()))?;
            if message["id"].as_str() != Some(id) || !seen.insert(id) {
                return Err(Error::Sdk("原生会话消息身份不属于同一快照或重复".into()));
            }
            if !message.is_object() {
                return Err(Error::Sdk("原生会话消息格式无效".into()));
            }
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.cleanup_blocking()
    }

    /// Explicit blocking graceful-exit/process-tree cleanup, never an owner pump.
    pub fn cleanup_blocking(&mut self) -> Result<()> {
        self.stdin.take();
        for _ in 0..20 {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        self.child.stop();
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod snapshot_tests;
