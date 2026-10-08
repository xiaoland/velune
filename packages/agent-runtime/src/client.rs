// Small, credential-blind client for Pi's versioned RPC subprocess.
//
// The host owns the child process and JSONL transport, and injects a
// credential-blind loopback Velune gateway configuration. Pi owns session
// files, message history, and tools; upstream provider credentials stay in
//! the host gateway. This module never reads environment variables or Pi auth
//! files.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

const MAX_RECORD_BYTES: usize = 1024 * 1024;
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
    #[error("Pi RPC record exceeds {MAX_RECORD_BYTES} bytes")]
    RecordTooLarge,
    #[error("Pi RPC child exited before responding")]
    ChildExited,
    #[error("Pi RPC response timed out")]
    Timeout,
    #[error("Pi RPC response failed: {0}")]
    CommandFailed(String),
    #[error("Pi SDK 接入失败: {0}")]
    Sdk(String),
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
    pub runtime_type_id: String,
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
    /// Ephemeral loopback gateway token. It is injected into the child
    /// environment and never serialized into app configuration or snapshots.
    #[serde(skip_serializing)]
    pub gateway_token: Option<String>,
    pub session_dir: Option<PathBuf>,
    pub session: Option<PathBuf>,
    pub name: Option<String>,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if crate::version::variant(&self.runtime_type_id)
            .is_none_or(|variant| variant.family_id != "pi")
        {
            return Err(Error::Sdk("Pi 运行时版本类型不受支持".into()));
        }
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
        if record.len() + count > MAX_RECORD_BYTES {
            reader.consume(count);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Pi RPC record exceeds size limit",
            ));
        }
        record.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(Some(record));
        }
    }
}

/// One long-lived Pi RPC subprocess. Records are retained until the host asks
/// for them, so the UI can poll without losing fast stream events.
pub struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    incoming: Receiver<Incoming>,
    pending: VecDeque<Value>,
    next_id: u64,
    stderr: SharedStderr,
    pub config: Config,
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
        let mut output = Command::new(node);
        output
            .arg(entry)
            .arg("--cli")
            .arg(&self.binary)
            .arg("--runtime-type")
            .arg(&self.runtime_type_id)
            .arg("--validate");
        let output = crate::native::rpc::bounded_process(output, None, MAX_RECORD_BYTES);
        match output {
            Ok(_) => Ok(()),
            Err(error) => Err(Error::Sdk(format!("Pi SDK 入口验证失败：{error}"))),
        }
    }
}

impl Client {
    pub fn spawn(config: Config) -> Result<Self> {
        config.validate()?;
        let node = config
            .node_binary
            .as_ref()
            .ok_or_else(|| Error::Sdk("请配置所选运行时的 Node 可执行文件".into()))?;
        let entry = config
            .rpc_entry
            .as_ref()
            .ok_or_else(|| Error::Sdk("SDK 入口不可用".into()))?;
        let mut command = Command::new(node);
        command
            .arg(entry)
            .arg("--cli")
            .arg(&config.binary)
            .arg("--runtime-type")
            .arg(&config.runtime_type_id);
        for (name, path) in [
            ("--models-path", &config.models_path),
            ("--selection-file", &config.selection_file),
            ("--agent-dir", &config.agent_dir),
        ] {
            command.arg(name).arg(
                path.as_ref()
                    .ok_or_else(|| Error::Sdk("SDK 装配路径不完整".into()))?,
            );
        }
        command.env_clear();
        for key in [
            "PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "SHELL",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
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
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().ok_or(Error::ChildExited)?;
        let stdout = child.stdout.take().ok_or(Error::ChildExited)?;
        let stderr_output = child.stderr.take().ok_or(Error::ChildExited)?;
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
                                let remaining = MAX_RECORD_BYTES.saturating_sub(state.bytes.len());
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
        let (sender, incoming) = mpsc::channel();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let reader_span = tracing::info_span!("agent_runtime.rpc_reader");
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
                            if let Ok(mut state) = state.lock() {
                                while !state.done {
                                    state = ready
                                        .wait(state)
                                        .unwrap_or_else(|error| error.into_inner());
                                }
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
                                Err(_) => {
                                    tracing::warn!(code="rpc_invalid_json","Pi RPC reader rejected invalid JSON");
                                    let _ = sender.send(Incoming::Error("invalid Pi RPC JSON record".into()));
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(code="rpc_read_failed",io_kind=?error.kind(),"Pi RPC reader I/O failed");
                            let _ = sender.send(Incoming::Error(format!("Pi RPC read failed: {:?}",error.kind())));
                            break;
                        }
                    }
                }
            }))?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            incoming,
            pending: VecDeque::new(),
            next_id: 1,
            stderr,
            config,
        })
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
            detail.push_str("\n[Pi RPC stderr 已截断：本地缓存上限 1 MiB]");
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
            Error::CommandFailed(format!("Pi RPC child exited before responding{status}"))
        } else {
            Error::CommandFailed(format!(
                "Pi RPC child exited before responding{status}；原始错误：{detail}"
            ))
        }
    }

    fn io_error_with_stderr(&self, context: &str, error: std::io::Error) -> Error {
        let detail = self.stderr_detail(true);
        if detail.is_empty() {
            Error::Io(error)
        } else {
            Error::CommandFailed(format!("{context}：{error}；原始错误：{detail}"))
        }
    }

    fn timeout_error(&mut self) -> Error {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => format!("进程状态：{status}"),
            Ok(None) => "Pi RPC 子进程仍在运行".into(),
            Err(error) => format!("进程状态不可用：{error}"),
        };
        let detail = self.stderr_detail(false);
        if detail.is_empty() {
            Error::CommandFailed(format!("Pi RPC response timed out；{status}"))
        } else {
            Error::CommandFailed(format!(
                "Pi RPC response timed out；{status}；原始错误：{detail}"
            ))
        }
    }

    fn write_command(&mut self, mut command: Value) -> Result<String> {
        let object = command.as_object_mut().ok_or(Error::InvalidCommand)?;
        let command_type = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(Error::MissingCommandType)?
            .to_string();
        let id = object
            .entry("id")
            .or_insert_with(|| {
                let id = format!("velune-{}", self.next_id);
                self.next_id += 1;
                Value::String(id)
            })
            .as_str()
            .ok_or(Error::InvalidCommand)?
            .to_string();
        let mut bytes = serde_json::to_vec(&command)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(Error::RecordTooLarge);
        }
        bytes.push(b'\n');
        if self.stdin.is_none() {
            return Err(self.child_exit_error());
        }
        let result = {
            let stdin = self.stdin.as_mut().expect("checked Pi RPC stdin");
            stdin.write_all(&bytes).and_then(|_| stdin.flush())
        };
        result.map_err(|error| self.io_error_with_stderr("Pi RPC command send failed", error))?;
        Ok(format!("{command_type}\0{id}"))
    }

    fn receive(&mut self, timeout: Duration) -> Result<Value> {
        if let Some(value) = self.pending.pop_front() {
            return Ok(value);
        }
        match self.incoming.recv_timeout(timeout) {
            Ok(Incoming::Record(value)) => Ok(value),
            Ok(Incoming::Error(error)) => {
                let stderr = self.stderr_detail(false);
                if stderr.is_empty() {
                    Err(Error::CommandFailed(error))
                } else {
                    Err(Error::CommandFailed(format!("{error}；原始错误：{stderr}")))
                }
            }
            Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => {
                Err(self.child_exit_error())
            }
            Err(RecvTimeoutError::Timeout) => Err(self.timeout_error()),
        }
    }

    /// Send a command and wait for its matching response. Other records stay
    /// buffered for `poll`, preserving event order across requests.
    fn request_with_timeout(&mut self, command: Value, timeout: Duration) -> Result<Value> {
        let key = self.write_command(command)?;
        let (_, id) = key.split_once('\0').expect("internal command key");
        let mut deferred = VecDeque::new();
        loop {
            let value = self.receive(timeout)?;
            let matches = value.get("type").and_then(Value::as_str) == Some("response")
                && value.get("id").and_then(Value::as_str) == Some(id);
            if matches {
                deferred.append(&mut self.pending);
                self.pending = deferred;
                if value.get("success") == Some(&Value::Bool(false)) {
                    return Err(Error::CommandFailed(
                        value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Pi command failed")
                            .to_string(),
                    ));
                }
                return Ok(value);
            }
            deferred.push_back(value);
        }
    }

    pub fn request(&mut self, command: Value) -> Result<Value> {
        self.request_with_timeout(command, RESPONSE_TIMEOUT)
    }

    pub fn prompt(&mut self, message: &str) -> Result<Value> {
        self.request(json!({"type":"prompt","message":message}))
    }

    pub fn sync_virtual_selection(&mut self) -> Result<Value> {
        self.prompt("/velune-sync-selection")
    }

    pub fn cancel(&mut self) -> Result<Vec<Value>> {
        // Pi documents clear_queue before abort for interactive cancellation.
        Ok(vec![
            self.request(json!({"type":"clear_queue"}))?,
            self.request(json!({"type":"abort"}))?,
        ])
    }

    pub fn poll(&mut self) -> Vec<Value> {
        let mut records = Vec::new();
        while let Some(record) = self.pending.pop_front() {
            records.push(record);
        }
        while let Ok(item) = self.incoming.try_recv() {
            match item {
                Incoming::Record(value) => records.push(value),
                Incoming::Error(error) => records.push(json!({
                    "type":"velune_error","error":error
                })),
                Incoming::Closed => records.push(json!({"type":"velune_closed"})),
            }
        }
        records
    }

    /// Events encountered before the last RPC response. Unlike poll, this does
    /// not consume newer reader records after the authoritative message barrier.
    pub fn take_buffered_events(&mut self) -> Vec<Value> {
        self.pending.drain(..).collect()
    }

    pub fn state(&mut self) -> Result<(Value, Value)> {
        let metadata = self.request_with_timeout(
            json!({"type":"prompt","message":"/velune-projection-sync"}),
            STARTUP_TIMEOUT,
        )?;
        if metadata["data"]["disposition"] != "handled" {
            return Err(Error::Sdk("原生会话元数据查询未被扩展处理".into()));
        }
        let state = self.request_with_timeout(json!({"type":"get_state"}), STARTUP_TIMEOUT)?;
        let mut messages =
            self.request_with_timeout(json!({"type":"get_messages"}), STARTUP_TIMEOUT)?;
        let metadata_event = self
            .pending
            .iter()
            .rev()
            .find(|event| {
                event["type"] == "extension_ui_request"
                    && event["method"] == "setStatus"
                    && event["statusKey"] == "velune.session-metadata"
            })
            .ok_or_else(|| Error::Sdk("原生会话投影缺少来源身份".into()))?;
        let metadata_text = metadata_event["statusText"]
            .as_str()
            .ok_or_else(|| Error::Sdk("原生会话来源身份 statusText 必须为字符串".into()))?;
        let provenance: Value = serde_json::from_str(metadata_text)
            .map_err(|error| Error::Sdk(format!("原生会话来源身份 JSON 无效: {error}")))?;
        let _: Vec<crate::conversation::MessageIdentityConfirmation> =
            serde_json::from_value(provenance["identityConfirmations"].clone())
                .map_err(|error| Error::Sdk(format!("原生会话身份确认格式无效: {error}")))?;
        let identities = provenance["messageIds"]
            .as_array()
            .ok_or_else(|| Error::Sdk("原生会话消息身份格式无效".into()))?;
        let native_messages = messages["data"]["messages"]
            .as_array_mut()
            .ok_or_else(|| Error::Sdk("原生会话消息格式无效".into()))?;
        if identities.len() != native_messages.len() {
            return Err(Error::Sdk(format!(
                "原生会话快照与来源身份不一致: messages={}, identities={}",
                native_messages.len(),
                identities.len()
            )));
        }
        for (message, identity) in native_messages.iter_mut().zip(identities) {
            let id = identity
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| Error::Sdk("原生会话消息身份为空".into()))?;
            let object = message
                .as_object_mut()
                .ok_or_else(|| Error::Sdk("原生会话消息格式无效".into()))?;
            object.insert("id".into(), Value::String(id.into()));
        }
        messages["identityConfirmations"] = provenance["identityConfirmations"].clone();
        Ok((state, messages))
    }

    pub fn switch_session(&mut self, path: &Path) -> Result<Value> {
        self.request(json!({"type":"switch_session","sessionPath":path}))
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.stdin.take();
        for _ in 0..20 {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
