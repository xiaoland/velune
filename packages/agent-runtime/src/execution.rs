//! Local and SSH execution with target-owned paths and stdin-only launch data.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const BOOTSTRAP: &str = include_str!("../resources/remote_bootstrap.py");
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(String);
pub type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "ssh",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExecutionTarget {
    #[default]
    Local,
    Ssh(SshTarget),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SshTarget {
    pub binary: PathBuf,
    /// An OpenSSH host alias or user@host; credentials remain with OpenSSH.
    pub destination: String,
    pub port: Option<u16>,
}

pub struct PreparedCommand {
    pub command: Command,
    /// Write this before the runtime's first stdin record. Never log these bytes.
    pub input_prefix: Vec<u8>,
}

impl ExecutionTarget {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    pub fn validate(&self) -> Result<()> {
        if let Self::Ssh(ssh) = self {
            absolute(&ssh.binary)?;
            if ssh.destination.is_empty()
                || ssh.destination.starts_with('-')
                || ssh
                    .destination
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
                || ssh.port == Some(0)
            {
                return Err(Error("SSH 目标或端口无效".into()));
            }
        }
        Ok(())
    }

    /// Stdio is configured by the caller after preparation. Command must inherit
    /// its target environment; env_clear cannot be represented across SSH.
    pub fn prepare(&self, command: Command, os_home: Option<&Path>) -> Result<PreparedCommand> {
        self.prepare_lifetime(command, os_home, true)
    }

    fn prepare_lifetime(
        &self,
        mut command: Command,
        os_home: Option<&Path>,
        interactive: bool,
    ) -> Result<PreparedCommand> {
        self.validate()?;
        if let Some(home) = os_home {
            absolute(home)?;
            command.env("HOME", home);
        }
        let Self::Ssh(ssh) = self else {
            return Ok(PreparedCommand {
                command,
                input_prefix: Vec::new(),
            });
        };
        let argv = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(utf8)
            .collect::<Result<Vec<_>>>()?;
        let environment = command
            .get_envs()
            .map(|(name, value)| Ok((utf8(name)?, value.map(utf8).transpose()?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let cwd = command
            .get_current_dir()
            .map(|path| {
                absolute(path)?;
                utf8(path.as_os_str())
            })
            .transpose()?;
        let request = json!({"operation":"launch", "argv":argv, "environment":environment, "cwd":cwd, "interactive":interactive});
        ssh.prepare_request(&request)
    }

    /// Run a short-lived helper using the same bounded process transport as adapters.
    pub fn output(
        &self,
        command: Command,
        input: Option<Vec<u8>>,
        os_home: Option<&Path>,
    ) -> Result<Vec<u8>> {
        let prepared = self.prepare_lifetime(command, os_home, false)?;
        run_prepared(prepared, input)
    }

    pub fn create_dir_all(&self, path: &Path) -> Result<()> {
        absolute(path)?;
        match self {
            Self::Local => {
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder.create(path)?;
            }
            Self::Ssh(ssh) => {
                ssh.request(&json!({"operation":"mkdir", "path":path}))?;
            }
        }
        Ok(())
    }

    /// Remove a caller-owned temporary application directory; missing paths are already removed.
    pub fn remove_dir_all(&self, path: &Path) -> Result<()> {
        absolute(path)?;
        match self {
            Self::Local => match fs::remove_dir_all(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            },
            Self::Ssh(ssh) => ssh
                .request(&json!({"operation":"remove_directory", "path":path}))
                .map(|_| ()),
        }
    }

    /// Read an application-owned runtime projection on its execution machine.
    pub fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        absolute(path)?;
        match self {
            Self::Local => fs::read(path).map_err(Into::into),
            Self::Ssh(ssh) => ssh.request(&json!({"operation":"read", "path":path})),
        }
    }

    pub fn write_file_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        absolute(path)?;
        match self {
            Self::Local => write_local(path, bytes),
            Self::Ssh(ssh) => ssh
                .request(
                    &json!({"operation":"write", "files":[{"path":path,"content":hex(bytes)}]}),
                )
                .map(|_| ()),
        }
    }

    pub fn is_directory(&self, path: &Path) -> Result<bool> {
        self.path_query(path, "is_directory")
    }

    pub fn path_exists(&self, path: &Path) -> Result<bool> {
        self.path_query(path, "exists")
    }

    fn path_query(&self, path: &Path, operation: &str) -> Result<bool> {
        absolute(path)?;
        match self {
            Self::Local => match fs::metadata(path) {
                Ok(metadata) => Ok(operation == "exists" || metadata.is_dir()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.into()),
            },
            Self::Ssh(ssh) => {
                serde_json::from_slice(&ssh.request(&json!({"operation":operation,"path":path}))?)
                    .map_err(|_| Error("远程路径检查返回无效结果".into()))
            }
        }
    }
}

impl SshTarget {
    fn command(&self) -> Result<Command> {
        ExecutionTarget::Ssh(self.clone()).validate()?;
        let mut command = Command::new(&self.binary);
        command.args([
            "-T",
            "-a",
            "-x",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "PermitLocalCommand=no",
        ]);
        if let Some(port) = self.port {
            command.arg("-p").arg(port.to_string());
        }
        Ok(command)
    }

    fn prepare_request(&self, request: &Value) -> Result<PreparedCommand> {
        let mut command = self.command()?;
        // Only fixed application code is placed in the remote shell command.
        command
            .args(["-o", "ClearAllForwardings=yes"])
            .arg(&self.destination)
            .arg(format!("python3 -c {}", shell_quote(BOOTSTRAP)));
        let request = serde_json::to_vec(request).map_err(|_| Error("远程请求编码失败".into()))?;
        let mut input_prefix = format!("{}\n", request.len()).into_bytes();
        input_prefix.extend(request);
        Ok(PreparedCommand {
            command,
            input_prefix,
        })
    }

    fn request(&self, request: &Value) -> Result<Vec<u8>> {
        run_prepared(self.prepare_request(request)?, None)
    }
}

fn run_prepared(prepared: PreparedCommand, input: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let mut bytes = prepared.input_prefix;
    if let Some(input) = input {
        bytes.extend(input);
    }
    crate::native::rpc::bounded_process(
        prepared.command,
        (!bytes.is_empty()).then_some(bytes),
        32768,
    )
    .map_err(|error| Error(error.to_string()))
}

/// Install only application helpers and their bundled parser dependencies.
/// Neither directory may refer to a user runtime HOME or authentication store.
pub fn install_resources(target: &ExecutionTarget, local: &Path, remote: &Path) -> Result<()> {
    absolute(local)?;
    absolute(remote)?;
    let names = [
        "pi_sdk.mjs",
        "pi_sessions.mjs",
        "pi_rpc.mjs",
        "pi_virtual_model.mjs",
        "pi_auth.mjs",
        "pi_provider_import.mjs",
        "huihua_sessions.mjs",
    ];
    let mut files = Vec::new();
    for name in names {
        collect_resource(&local.join(name), &remote.join(name), &mut files)?;
    }
    let dependencies = local.join("node_modules");
    if dependencies.exists() {
        collect_resource(&dependencies, &remote.join("node_modules"), &mut files)?;
    }
    match target {
        ExecutionTarget::Ssh(ssh) => {
            ssh.request(&json!({"operation":"write", "files":files}))?;
        }
        ExecutionTarget::Local => {
            return Err(Error("本地运行时直接使用应用资源，无需远程安装".into()));
        }
    }
    Ok(())
}

fn collect_resource(local: &Path, remote: &Path, files: &mut Vec<Value>) -> Result<()> {
    let metadata = fs::symlink_metadata(local)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(local)? {
            let entry = entry?;
            collect_resource(&entry.path(), &remote.join(entry.file_name()), files)?;
        }
    } else if metadata.is_file() {
        files.push(json!({"path":remote, "content":hex(&fs::read(local)?)}));
    } else {
        return Err(Error("运行时资源包必须仅包含普通文件与目录".into()));
    }
    Ok(())
}

/// Holds the forwarding connection for as long as a remote runtime can use it.
pub struct LoopbackTunnel {
    endpoint: String,
    master: Option<Child>,
    directory: Option<PathBuf>,
}

impl LoopbackTunnel {
    pub fn open(target: &ExecutionTarget, endpoint: &str) -> Result<Self> {
        target.validate()?;
        let (port, suffix) = loopback_address(endpoint)?;
        let ExecutionTarget::Ssh(ssh) = target else {
            return Ok(Self {
                endpoint: endpoint.into(),
                master: None,
                directory: None,
            });
        };
        let directory = temporary_directory()?;
        let socket = directory.join("s");
        let mut command = ssh.command()?;
        command
            .args([
                "-M",
                "-N",
                "-o",
                "ClearAllForwardings=yes",
                "-o",
                "ControlPersist=no",
                "-o",
                "ExitOnForwardFailure=yes",
                "-S",
            ])
            .arg(&socket)
            .arg(&ssh.destination)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(error.into());
            }
        };
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&diagnostic);
        let mut stderr = child.stderr.take().expect("SSH stderr is piped");
        thread::spawn(move || {
            let mut chunk = [0; 4096];
            while let Ok(count) = stderr.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                if let Ok(mut retained) = capture.lock() {
                    let count = count.min(32768_usize.saturating_sub(retained.len()));
                    retained.extend_from_slice(&chunk[..count]);
                }
            }
        });
        let mut tunnel = Self {
            endpoint: String::new(),
            master: Some(child),
            directory: Some(directory),
        };
        let deadline = Instant::now() + TIMEOUT;
        while !socket.exists() {
            if tunnel
                .master
                .as_mut()
                .expect("SSH master is present")
                .try_wait()?
                .is_some()
            {
                return Err(ssh_connection_error(
                    "SSH loopback 连接未能建立",
                    &diagnostic,
                ));
            }
            if Instant::now() >= deadline {
                return Err(ssh_connection_error("SSH loopback 连接超时", &diagnostic));
            }
            thread::sleep(Duration::from_millis(20));
        }
        let mut forward = Command::new(&ssh.binary);
        forward.args(["-F", "none"]);
        forward
            .arg("-S")
            .arg(&socket)
            .args(["-O", "forward", "-R"])
            .arg(format!("127.0.0.1:0:127.0.0.1:{port}"))
            .arg(&ssh.destination);
        let bytes = crate::native::rpc::bounded_process(forward, None, 32768)
            .map_err(|error| Error(format!("SSH loopback 转发失败：{error}")))?;
        let remote_port = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|value| value.trim().parse::<u16>().ok())
            .filter(|value| *value != 0)
            .ok_or_else(|| Error("SSH 未返回有效的远程 loopback 端口".into()))?;
        ssh.request(&json!({"operation":"verify_loopback", "port":remote_port}))?;
        tunnel.endpoint = format!("http://127.0.0.1:{remote_port}{suffix}");
        Ok(tunnel)
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for LoopbackTunnel {
    fn drop(&mut self) {
        if let Some(child) = &mut self.master {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(directory) = &self.directory {
            let _ = fs::remove_dir_all(directory);
        }
    }
}

fn ssh_connection_error(message: &str, diagnostic: &Mutex<Vec<u8>>) -> Error {
    let detail = diagnostic
        .lock()
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .unwrap_or_default();
    Error(if detail.is_empty() {
        message.into()
    } else {
        format!("{message}：{detail}")
    })
}

fn loopback_address(endpoint: &str) -> Result<(u16, &str)> {
    let address = endpoint
        .strip_prefix("http://127.0.0.1:")
        .ok_or_else(|| Error("运行时服务必须为 HTTP IPv4 loopback 地址".into()))?;
    let (port, suffix) = address
        .split_once('/')
        .map_or((address, ""), |(port, _)| (port, &address[port.len()..]));
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| Error("运行时 loopback 服务端口无效".into()))?;
    Ok((port, suffix))
}

fn temporary_directory() -> Result<PathBuf> {
    // Unix control sockets have a short path limit; use a private directory in /tmp.
    #[cfg(unix)]
    let root = PathBuf::from("/tmp");
    #[cfg(not(unix))]
    let root = std::env::temp_dir();
    let directory = root.join(format!("velune-ssh-{}", unique_name()?));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    Ok(directory)
}

fn write_local(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error("写入路径缺少父目录".into()))?;
    ExecutionTarget::Local.create_dir_all(parent)?;
    let temporary = parent.join(format!(".velune-{}", unique_name()?));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Into::into)
}

fn unique_name() -> Result<String> {
    let mut bytes = [0; 12];
    getrandom::fill(&mut bytes).map_err(|_| Error("临时路径随机源不可用".into()))?;
    Ok(hex(&bytes))
}
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 15) as usize] as char);
    }
    value
}
fn absolute(path: &Path) -> Result<()> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(Error("运行时路径必须为绝对路径".into()))
    }
}
fn utf8(value: &OsStr) -> Result<&str> {
    value
        .to_str()
        .ok_or_else(|| Error("SSH 运行时参数必须为 UTF-8".into()))
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
