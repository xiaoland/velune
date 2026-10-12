//! Session-scoped access to an application-owned MCP service. No source configuration is written.
use crate::{
    execution::{ExecutionTarget, LoopbackTunnel},
    native::{Error, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fmt, path::PathBuf, process::Command};

/// A temporary loopback service authorized by the application for this execution only.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    pub endpoint: String,
    #[serde(skip_serializing)]
    pub token: String,
}

impl fmt::Debug for Server {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Server")
            .field("name", &self.name)
            .field("endpoint", &"[redacted]")
            .field("token", &"[redacted]")
            .finish()
    }
}

impl Server {
    pub(crate) fn http(&self) -> Value {
        json!({"type":"http","url":self.endpoint,"headers":{"Authorization":format!("Bearer {}",self.token)}})
    }

    pub(crate) fn acp_http(&self) -> Value {
        json!({"type":"http","name":self.name,"url":self.endpoint,"headers":[{"name":"Authorization","value":format!("Bearer {}",self.token)}]})
    }
}

pub(crate) fn validate(servers: &[Server]) -> Result<()> {
    let mut names = BTreeSet::new();
    for server in servers {
        if server.name.is_empty()
            || server.name.len() > 32
            || !server
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !names.insert(&server.name)
        {
            return Err(Error::new("临时 MCP 服务名称无效或重复"));
        }
        if server.token.is_empty() || !server.token.bytes().all(|byte| (33..=126).contains(&byte)) {
            return Err(Error::new("临时 MCP 服务授权无效"));
        }
        if !server.endpoint.starts_with("http://127.0.0.1:")
            || server
                .endpoint
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || server.endpoint.contains('#')
        {
            return Err(Error::new("临时 MCP 服务必须使用本机 loopback HTTP 地址"));
        }
    }
    Ok(())
}

pub(crate) fn set_environment(command: &mut Command, servers: &[Server]) {
    for (index, server) in servers.iter().enumerate() {
        command.env(token_environment(index), &server.token);
    }
}

pub(crate) fn token_environment(index: usize) -> String {
    format!("VELUNE_MCP_TOKEN_{index}")
}

/// Keep reverse forwarding and relay resources alive until after the Agent exits.
/// The services are optional for the conversation: one that cannot be
/// reached from the execution machine is left out rather than failing it.
pub(crate) struct Prepared {
    pub servers: Vec<Server>,
    target: ExecutionTarget,
    os_home: Option<PathBuf>,
    _tunnels: Vec<LoopbackTunnel>,
    relay: Option<Relay>,
}

impl Prepared {
    pub fn new(
        servers: &[Server],
        target: &ExecutionTarget,
        os_home: Option<&std::path::Path>,
    ) -> Result<Self> {
        validate(servers)?;
        let mut prepared = Self {
            servers: Vec::new(),
            target: target.clone(),
            os_home: os_home.map(std::path::Path::to_path_buf),
            _tunnels: Vec::new(),
            relay: None,
        };
        for server in servers {
            let tunnel = match LoopbackTunnel::open(target, &server.endpoint) {
                Ok(tunnel) => tunnel,
                Err(error) => {
                    tracing::warn!(%error, server = %server.name, "MCP service forwarding failed; running without it");
                    continue;
                }
            };
            let mut server = server.clone();
            server.endpoint = tunnel.endpoint().into();
            prepared.servers.push(server);
            prepared._tunnels.push(tunnel);
        }
        Ok(prepared)
    }

    pub fn acp_servers(&mut self, http: bool) -> Result<Vec<Value>> {
        if http || self.servers.is_empty() {
            return Ok(self.servers.iter().map(Server::acp_http).collect());
        }
        if self.relay.is_none() {
            match Relay::install(&self.target, self.os_home.as_deref()) {
                Ok(relay) => self.relay = Some(relay),
                Err(error) => {
                    tracing::warn!(%error, "MCP stdio relay unavailable; running without MCP services");
                    return Ok(Vec::new());
                }
            }
        }
        let relay = self.relay.as_ref().expect("installed ACP relay");
        Ok(self
            .servers
            .iter()
            .map(|server| {
                json!({"name":server.name,"command":relay.python,"args":[relay.path],"env":[
                    {"name":"VELUNE_MCP_ENDPOINT","value":server.endpoint},
                    {"name":"VELUNE_MCP_TOKEN","value":server.token}
                ]})
            })
            .collect())
    }
}

struct Relay {
    target: ExecutionTarget,
    directory: PathBuf,
    python: PathBuf,
    path: PathBuf,
}

impl Relay {
    fn install(target: &ExecutionTarget, os_home: Option<&std::path::Path>) -> Result<Self> {
        let mut command = Command::new("python3");
        command.args(["-c", "import json,sys,tempfile; print(json.dumps({'python':sys.executable,'directory':tempfile.mkdtemp(prefix='velune-mcp-')}))"]);
        let bytes = target.output(command, None, os_home).map_err(|_| {
            Error::new("仅支持 stdio MCP 的 ACP Agent 需要目标机器上可用的 Python 3")
        })?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| Error::new("MCP 转接准备未返回有效结果"))?;
        let path = |key| -> Result<PathBuf> {
            let path = value[key]
                .as_str()
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| Error::new("MCP 转接路径不是绝对路径"))?;
            Ok(path)
        };
        let directory = path("directory")?;
        let relay = Self {
            path: directory.join("relay.py"),
            directory,
            python: path("python")?,
            target: target.clone(),
        };
        target.write_file_atomic(
            &relay.path,
            include_bytes!("../resources/mcp_stdio_relay.py"),
        )?;
        Ok(relay)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        if let Err(error) = self.target.remove_dir_all(&self.directory) {
            tracing::warn!(%error, "MCP relay temporary directory cleanup failed");
        }
    }
}
