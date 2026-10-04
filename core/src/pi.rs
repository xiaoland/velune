//! Small, credential-blind client for Pi's versioned RPC subprocess.
//!
//! The host owns the child process and JSONL transport, and injects a
//! credential-blind loopback Velune gateway configuration. Pi owns session
//! files, message history, and tools; upstream provider credentials stay in
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
    #[error("Pi credential reference is invalid")]
    InvalidCredentialReference,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Application-owned, non-secret Pi launch configuration. The provider fields
/// describe the injected Velune gateway transport; upstream authentication is
/// resolved by the host gateway and is never passed through this config.
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
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(alias = "credentialRef")]
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(alias = "credentialResolver")]
    #[serde(default)]
    pub credential_resolver: Option<PathBuf>,
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
            &self.sdk_helper,
            &self.extension,
            &self.agent_dir,
            &self.working_dir,
            &self.credential_resolver,
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
        if let Some(reference) = &self.credential_ref
            && (reference.is_empty()
                || reference.chars().any(|character| {
                    character.is_whitespace() || character == '\'' || character == '"'
                }))
        {
            return Err(Error::InvalidCredentialReference);
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

impl Client {
    pub fn spawn(config: Config) -> Result<Self> {
        config.validate()?;
        // Finder-launched apps do not inherit a user's shell PATH. When the
        // UI supplies Node explicitly, run Pi through that exact binary so a
        // shebang such as `#!/usr/bin/env node` cannot select another runtime
        // (or fail to resolve one). An omitted node_binary keeps support for
        // native/synthetic Pi-compatible executables used by tests.
        let mut command = if let Some(node) = &config.node_binary {
            let mut command = Command::new(node);
            command.arg(&config.binary);
            command
        } else {
            Command::new(&config.binary)
        };
        command.arg("--mode").arg("rpc");
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
            command.env(
                "VELUNE_PI_SELECTION_FILE",
                agent_dir.join("velune-selection.json"),
            );
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
        thread::Builder::new()
            .name("velune-pi-rpc-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_record(&mut reader) {
                        Ok(None) => {
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
                                    let _ = sender.send(Incoming::Error(format!(
                                        "invalid Pi RPC JSON record: {error}"
                                    )));
                                }
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(Incoming::Error(error.to_string()));
                            break;
                        }
                    }
                }
            })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    use tempfile::tempdir;

    fn fixture_script(dir: &Path) -> PathBuf {
        let path = dir.join("pi-fixture.sh");
        fs::write(
            &path,
            r##"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  type=$(printf '%s' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
  if [ "$type" = prompt ]; then printf '{"type":"agent_settled"}\n'; fi
done
"##,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn node_wrapper(dir: &Path) -> PathBuf {
        let path = dir.join("node-fixture.sh");
        fs::write(
            &path,
            "#!/bin/sh\ntarget=$1\nshift\nexec \"$target\" \"$@\"\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[test]
    fn preserves_events_and_waits_for_matching_response() {
        let dir = tempdir().unwrap();
        let client = fixture_script(dir.path());
        let mut pi = Client::spawn(Config {
            binary: client,
            node_binary: None,
            sdk_helper: None,
            extension: None,
            agent_dir: None,
            working_dir: None,
            provider: None,
            model: None,
            protocol: None,
            endpoint: None,
            credential_ref: None,
            credential_resolver: None,
            gateway_token: None,
            session_dir: None,
            session: None,
            name: None,
        })
        .unwrap();
        pi.prompt("synthetic").unwrap();
        pi.request(json!({"type":"get_state"})).unwrap();
        let events = pi.poll();
        assert!(events.iter().any(|v| v["type"] == "agent_settled"));
        pi.shutdown().unwrap();
    }

    #[test]
    fn uses_explicit_node_binary_for_cli() {
        let dir = tempdir().unwrap();
        let client = fixture_script(dir.path());
        let node = node_wrapper(dir.path());
        let mut pi = Client::spawn(Config {
            binary: client,
            node_binary: Some(node),
            sdk_helper: None,
            extension: None,
            agent_dir: None,
            working_dir: None,
            provider: None,
            model: None,
            protocol: None,
            endpoint: None,
            credential_ref: None,
            credential_resolver: None,
            gateway_token: None,
            session_dir: None,
            session: None,
            name: None,
        })
        .unwrap();
        pi.prompt("synthetic through explicit node").unwrap();
        pi.shutdown().unwrap();
    }
}
