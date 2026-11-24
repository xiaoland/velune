//! Binds a gateway provider to an official agy process on one execution target.
use std::{
    fs,
    path::{Path, PathBuf},
};
use velune_agent_runtime::{
    execution::{ExecutionTarget, LoopbackTunnel, SshTarget},
    process::command,
};
use velune_ai::{NativeErrorKind, OperationFuture};
use velune_ai_provider::antigravity::{Launch, Process};

// Do not let an inherited Gemini key or Cloud configuration take precedence
// over the CLI's own login. The CLI inherits its OS login environment.
const REMOVED_ENVIRONMENT: &[&str] = &[
    "AGY_ADC_AUTH",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_LOCATION",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_GEMINI_BASE_URL",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GOOGLE_OAUTH_ACCESS_TOKEN",
];

// Only the request's MCP server may authorize tools. Explicit command, file
// and web denies are independent of the CLI's sandbox availability.
const SETTINGS: &str = r#"{
  "agentMode": "",
  "toolPermission": "request-review",
  "permissions": {
    "allow": ["mcp(velune/*)"],
    "deny": ["command(*)", "read_file(*)", "write_file(*)", "read_url(*)", "execute_url(*)"]
  }
}"#;

pub(crate) const SSH_UNSUPPORTED: &str = "Windows 上的 Antigravity 订阅只支持本机执行";

pub(crate) fn validate_target(binary: &str, destination: Option<&str>) -> Result<(), &'static str> {
    let absolute = if destination.is_some() {
        binary.starts_with('/')
    } else {
        Path::new(binary).is_absolute()
    };
    if !absolute || binary.chars().any(char::is_control) {
        return Err("Antigravity CLI 可执行文件必须是执行环境中的绝对路径");
    }
    if let Some(destination) = destination {
        let invalid_component = |part: &str| part.is_empty() || part.starts_with('-');
        let invalid_address = match destination.split_once('@') {
            Some((user, host)) => {
                invalid_component(user) || invalid_component(host) || host.contains('@')
            }
            None => invalid_component(destination),
        };
        if invalid_address
            || destination.contains(['/', '\\'])
            || destination
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err("Antigravity SSH 目标须为 OpenSSH 主机别名或 user@host");
        }
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct Launcher {
    binary: String,
    destination: Option<String>,
}

impl Launcher {
    pub(crate) fn new(binary: String, destination: Option<String>) -> Self {
        Self {
            binary,
            destination,
        }
    }

    fn prepare_process(self, loopback_mcp: &str) -> Result<Process, NativeErrorKind> {
        validate_target(&self.binary, self.destination.as_deref())
            .map_err(|_| NativeErrorKind::InvalidInput)?;
        if cfg!(windows) && self.destination.is_some() {
            return Err(NativeErrorKind::Unsupported);
        }
        let target = match self.destination {
            Some(destination) => ExecutionTarget::Ssh(SshTarget {
                binary: ssh_binary().ok_or(NativeErrorKind::Transport)?,
                destination,
                port: None,
            }),
            None => ExecutionTarget::Local,
        };
        let mut workspace =
            Workspace::create(target.clone()).map_err(|_| NativeErrorKind::Transport)?;
        let cwd = workspace
            .path
            .to_str()
            .ok_or(NativeErrorKind::InvalidInput)?
            .to_owned();
        let mut child = command(&self.binary);
        child
            .current_dir(&workspace.path)
            .args([
                "acp",
                "--process-per-session",
                "--sandbox",
                "--disable-telemetry",
                "--gemini-dir",
            ])
            .arg(workspace.path.join(".gemini"))
            .arg("--log-file")
            .arg(workspace.path.join("acp.log"));
        for name in REMOVED_ENVIRONMENT {
            child.env_remove(name);
        }
        let prepared = target
            .prepare(child, None)
            .map_err(|_| NativeErrorKind::Transport)?;
        let tunnel =
            LoopbackTunnel::open(&target, loopback_mcp).map_err(|_| NativeErrorKind::Transport)?;
        let mcp_endpoint = tunnel.endpoint().to_owned();
        workspace.tunnel = Some(tunnel);
        Ok(Process {
            command: prepared.command,
            input_prefix: prepared.input_prefix,
            cwd,
            mcp_endpoint,
            guard: Some(Box::new(workspace)),
        })
    }
}

impl Launch for Launcher {
    fn prepare(&self, loopback_mcp: String) -> OperationFuture<Result<Process, NativeErrorKind>> {
        let launcher = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || launcher.prepare_process(&loopback_mcp))
                .await
                .map_err(|_| NativeErrorKind::Transport)?
        })
    }
}

fn ssh_binary() -> Option<PathBuf> {
    std::iter::once(PathBuf::from("/usr/bin/ssh"))
        .chain(
            std::env::var_os("PATH")
                .iter()
                .flat_map(std::env::split_paths)
                .map(|path| path.join("ssh")),
        )
        .find(|path| path.is_absolute() && path.is_file())
}

struct Workspace {
    target: ExecutionTarget,
    path: PathBuf,
    tunnel: Option<LoopbackTunnel>,
}

impl Workspace {
    fn create(target: ExecutionTarget) -> Result<Self, ()> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| ())?;
        let nonce = u128::from_ne_bytes(bytes);
        let name = format!("velune-agy-{nonce:032x}");
        if target.is_local() {
            let path = std::env::temp_dir().join(name);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&path)
                    .map_err(|_| ())?;
            }
            #[cfg(not(unix))]
            fs::create_dir(&path).map_err(|_| ())?;
            let workspace = Self {
                target,
                path,
                tunnel: None,
            };
            let settings = workspace.path.join(".gemini").join("antigravity-cli");
            fs::create_dir_all(&settings).map_err(|_| ())?;
            fs::write(settings.join("settings.json"), SETTINGS).map_err(|_| ())?;
            return Ok(workspace);
        }

        let path = PathBuf::from(format!("/tmp/{name}"));
        let mut mkdir = command("python3");
        mkdir
            .args([
                "-c",
                r#"
import os, shutil, sys
path = sys.argv[1]
os.mkdir(path, 0o700)
try:
    settings = os.path.join(path, ".gemini", "antigravity-cli")
    os.makedirs(settings, mode=0o700)
    with open(os.path.join(settings, "settings.json"), "x", encoding="utf-8") as file:
        file.write(sys.argv[2])
except BaseException:
    shutil.rmtree(path, ignore_errors=True)
    raise
"#,
            ])
            .arg(&path)
            .arg(SETTINGS);
        target.output(mkdir, None, None).map_err(|_| ())?;
        Ok(Self {
            target,
            path,
            tunnel: None,
        })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if self.target.is_local() {
            let _ = fs::remove_dir_all(&self.path);
            return;
        }
        let target = self.target.clone();
        let path = self.path.clone();
        let tunnel = self.tunnel.take();
        // Remote transport has a bounded timeout and must not block async
        // cancellation or the final response.
        let _ = std::thread::Builder::new().spawn(move || {
            drop(tunnel);
            let _ = target.remove_dir_all(&path);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_plans_a_private_cli_process_without_accessing_login() {
        let executable = std::env::temp_dir().join("synthetic agy fixture");
        let endpoint = "http://127.0.0.1:3210/mcp";
        let process = Launcher::new(executable.to_str().unwrap().into(), None)
            .prepare_process(endpoint)
            .map_err(|e| format!("{e:?}"))
            .unwrap();
        let cwd = PathBuf::from(&process.cwd);
        assert_eq!(process.command.get_program(), executable);
        assert_eq!(process.command.get_current_dir(), Some(cwd.as_path()));
        let args = process.command.get_args().collect::<Vec<_>>();
        assert_eq!(
            &args[..5],
            &[
                "acp",
                "--process-per-session",
                "--sandbox",
                "--disable-telemetry",
                "--gemini-dir"
            ]
        );
        assert_eq!(args[5], cwd.join(".gemini"));
        assert_eq!(
            process.command.get_envs().collect::<Vec<_>>(),
            REMOVED_ENVIRONMENT
                .iter()
                .map(|name| (std::ffi::OsStr::new(name), None))
                .collect::<Vec<_>>()
        );
        assert_eq!(process.mcp_endpoint, endpoint);
        assert!(process.input_prefix.is_empty());
        let settings: serde_json::Value = serde_json::from_slice(
            &fs::read(
                cwd.join(".gemini")
                    .join("antigravity-cli")
                    .join("settings.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(settings["agentMode"], "");
        assert_eq!(
            settings["permissions"]["allow"],
            serde_json::json!(["mcp(velune/*)"])
        );
        assert!(
            settings["permissions"]["deny"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("command(*)"))
        );
        drop(process);
        assert!(!cwd.exists());
    }
}
