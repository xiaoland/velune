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
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::Duration,
};

const MAX_RECORD_BYTES: usize = 1024 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

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
pub struct Config {
    pub binary: PathBuf,
    #[serde(alias = "nodeBinary")]
    #[serde(default)]
    pub node_binary: Option<PathBuf>,
    #[serde(alias = "sdkHelper")]
    #[serde(default)]
    pub sdk_helper: Option<PathBuf>,
    #[serde(alias = "extension")]
    #[serde(default)]
    pub extension: Option<PathBuf>,
    #[serde(alias = "agentDir")]
    #[serde(default)]
    pub agent_dir: Option<PathBuf>,
    #[serde(alias = "workingDir")]
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// SDK launcher and application-owned catalog/selection, distinct from PI_HOME.
    #[serde(default)]
    pub rpc_entry: Option<PathBuf>,
    #[serde(default)]
    pub models_path: Option<PathBuf>,
    #[serde(default)]
    pub selection_file: Option<PathBuf>,
    /// Ephemeral loopback gateway token. It is injected into the child
    /// environment and never serialized into app configuration or snapshots.
    #[serde(skip_serializing, default)]
    pub gateway_token: Option<String>,
    #[serde(alias = "sessionDir")]
    #[serde(default)]
    pub session_dir: Option<PathBuf>,
    #[serde(default)]
    pub session: Option<PathBuf>,
    #[serde(default)]
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
        let output = Command::new(node)
            .arg(entry)
            .arg("--cli")
            .arg(&self.binary)
            .arg("--validate")
            .env_clear()
            .output()?;
        if !output.status.success() {
            return Err(Error::Sdk(
                "仅支持 Pi Agent 1.0.2 SDK 对应的 CLI 入口，请检查运行时配置".into(),
            ));
        }
        Ok(())
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
        command.arg(entry).arg("--cli").arg(&config.binary);
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
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or(Error::ChildExited)?;
        let stdout = child.stdout.take().ok_or(Error::ChildExited)?;
        let (sender, incoming) = mpsc::channel();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let reader_span = tracing::info_span!("agent_runtime.rpc_reader");
        thread::Builder::new()
            .name("velune-pi-rpc-reader".into())
            .spawn(move || tracing::dispatcher::with_default(&dispatcher, || {
                let _entered=reader_span.enter();
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_record(&mut reader) {
                        Ok(None) => {
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
            config,
        })
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
        self.stdin
            .as_mut()
            .ok_or(Error::ChildExited)?
            .write_all(&bytes)?;
        self.stdin.as_mut().ok_or(Error::ChildExited)?.flush()?;
        Ok(format!("{command_type}\0{id}"))
    }

    fn receive(&mut self, timeout: Duration) -> Result<Value> {
        if let Some(value) = self.pending.pop_front() {
            return Ok(value);
        }
        match self.incoming.recv_timeout(timeout) {
            Ok(Incoming::Record(value)) => Ok(value),
            Ok(Incoming::Error(error)) => Err(Error::CommandFailed(error)),
            Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => Err(Error::ChildExited),
            Err(RecvTimeoutError::Timeout) => Err(Error::Timeout),
        }
    }

    /// Send a command and wait for its matching response. Other records stay
    /// buffered for `poll`, preserving event order across requests.
    pub fn request(&mut self, command: Value) -> Result<Value> {
        let key = self.write_command(command)?;
        let (_, id) = key.split_once('\0').expect("internal command key");
        let mut deferred = VecDeque::new();
        loop {
            let value = self.receive(RESPONSE_TIMEOUT)?;
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

    pub fn state(&mut self) -> Result<(Value, Value)> {
        let state = self.request(json!({"type":"get_state"}))?;
        let messages = self.request(json!({"type":"get_messages"}))?;
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
