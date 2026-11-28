//! Binds a gateway provider to an official agy process on one execution target.
use crate::{antigravity_quota, credential_helper::execute_prepared_timeout};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use velune_agent_runtime::{
    execution::{ExecutionTarget, LoopbackTunnel, SshTarget},
    process::command,
};
use velune_ai::{NativeErrorKind, OperationFuture};
use velune_ai_provider::antigravity::{Launch, Process, probe_models};
use velune_gateway::QuotaWindow;

// Do not let an inherited Gemini key or Cloud configuration take precedence
// over the CLI's own login. The CLI inherits its OS login environment.
const REMOVED_ENVIRONMENT: &[&str] = &[
    "AGY_ADC_AUTH",
    "CLOUD_CODE_URL",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_LOCATION",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_QUOTA_PROJECT",
    "GOOGLE_GEMINI_BASE_URL",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GOOGLE_OAUTH_ACCESS_TOKEN",
];

// Only the request's MCP server may authorize tools. Explicit command, file
// and web denies are independent of the CLI's sandbox availability.
const ACP_SETTINGS: &str = r#"{
  "agentMode": "",
  "toolPermission": "request-review",
  "permissions": {
    "allow": ["mcp(velune/*)"],
    "deny": ["command(*)", "read_file(*)", "write_file(*)", "read_url(*)", "execute_url(*)"]
  }
}"#;

const USAGE_SETTINGS: &str = r#"{
  "agentMode": "",
  "disableSlashCommands": false,
  "toolPermission": "request-review",
  "permissions": {
    "deny": ["command(*)", "read_file(*)", "write_file(*)", "read_url(*)", "execute_url(*)", "mcp(*)"]
  }
}"#;
const USAGE_AGENT: &str = "velune-usage";
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(15);
const ACP_ARGS: &[&str] = &[
    "acp",
    "--process-per-session",
    "--sandbox",
    "--disable-telemetry",
];
const USAGE_TIMEOUT: Duration = Duration::from_secs(15);
const SETTINGS_PATH: &str = ".gemini/antigravity-cli/settings.json";

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

#[derive(Clone, PartialEq, Eq)]
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

    fn target(&self) -> Result<ExecutionTarget, NativeErrorKind> {
        validate_target(&self.binary, self.destination.as_deref())
            .map_err(|_| NativeErrorKind::InvalidInput)?;
        if cfg!(windows) && self.destination.is_some() {
            return Err(NativeErrorKind::Unsupported);
        }
        Ok(match &self.destination {
            Some(destination) => ExecutionTarget::Ssh(SshTarget {
                binary: ssh_binary().ok_or(NativeErrorKind::Transport)?,
                destination: destination.clone(),
                port: None,
            }),
            None => ExecutionTarget::Local,
        })
    }

    fn prepare_process(self, loopback_mcp: &str) -> Result<Process, NativeErrorKind> {
        let target = self.target()?;
        let mut workspace =
            Workspace::create(target.clone(), &[(SETTINGS_PATH, ACP_SETTINGS, 0o600)])
                .map_err(|_| NativeErrorKind::Transport)?;
        let cwd = workspace
            .path
            .to_str()
            .ok_or(NativeErrorKind::InvalidInput)?
            .to_owned();
        let mut child = command(&self.binary);
        child.args(ACP_ARGS);
        configure(&mut child, &workspace.path, "acp.log");
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
    pub(crate) async fn quota_windows(&self, ttl_ms: i64) -> Result<Vec<QuotaWindow>, ()> {
        let target = self.target().map_err(|_| ())?;
        let workspace = tokio::task::spawn_blocking(move || Workspace::create(target, QUOTA_FILES))
            .await
            .map_err(|_| ())??;
        let version = workspace
            .target
            .prepare_helper(self.quota_command(&workspace, &["--version"])?)
            .map_err(|_| ())?;
        let version = execute_prepared_timeout(version, VERSION_TIMEOUT)
            .await
            .map_err(|_| ())?;
        if !antigravity_quota::supported_version(&version) {
            return Err(());
        }

        // A successful session/new is the preflight for account-backed
        // /usage. Authentication and prompts are never requested over ACP.
        let acp = workspace
            .target
            .prepare(self.quota_command(&workspace, ACP_ARGS)?, None)
            .map_err(|_| ())?;
        let models = tokio::time::timeout(
            CATALOG_TIMEOUT,
            probe_models(Process {
                command: acp.command,
                input_prefix: acp.input_prefix,
                cwd: workspace.path.to_str().ok_or(())?.to_owned(),
                mcp_endpoint: String::new(),
                guard: None,
            }),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;

        let usage = self.quota_command(
            &workspace,
            &[
                "-p",
                "/usage",
                "--output-format",
                "json",
                "--agent",
                USAGE_AGENT,
                "--print-timeout",
                "12s",
            ],
        )?;
        let usage = workspace.target.prepare_helper(usage).map_err(|_| ())?;
        let output = execute_prepared_timeout(usage, USAGE_TIMEOUT)
            .await
            .map_err(|_| ())?;
        antigravity_quota::parse(&output, &models, crate::local::unix_ms_now(), ttl_ms)
    }

    fn quota_command(&self, workspace: &Workspace, args: &[&str]) -> Result<Command, ()> {
        let mut child = if workspace.target.is_local() {
            let inherited = std::env::var_os("PATH").unwrap_or_default();
            let path = std::env::join_paths(
                std::iter::once(workspace.path.clone()).chain(std::env::split_paths(&inherited)),
            )
            .map_err(|_| ())?;
            let mut child = command(&self.binary);
            child.env("PATH", path);
            child
        } else {
            // Preserve the target login's PATH. The argument list goes through
            // the existing SSH bootstrap, and Python replaces itself with agy.
            let mut child = command("python3");
            child
                .args([
                    "-c",
                    r#"
import os, sys
os.environ["PATH"] = os.getcwd() + os.pathsep + os.environ.get("PATH", os.defpath)
os.execv(sys.argv[1], sys.argv[1:])
"#,
                ])
                .arg(&self.binary);
            child
        };
        child.args(args);
        configure(&mut child, &workspace.path, "usage.log");
        child
            .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
            .env("BROWSER", workspace.path.join(BROWSER_GUARD));
        Ok(child)
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

fn configure(child: &mut Command, cwd: &Path, log_name: &str) {
    child
        .current_dir(cwd)
        .arg("--gemini-dir")
        .arg(cwd.join(".gemini"))
        .arg("--log-file")
        .arg(cwd.join(log_name));
    for name in REMOVED_ENVIRONMENT {
        child.env_remove(name);
    }
}

#[cfg(unix)]
const BROWSER_GUARD: &str = "open";
#[cfg(windows)]
const BROWSER_GUARD: &str = "open.cmd";

const QUOTA_FILES: &[(&str, &str, u32)] = &[
    (SETTINGS_PATH, USAGE_SETTINGS, 0o600),
    (
        ".agents/agents/velune-usage/agent.md",
        "---\nname: velune-usage\ndescription: Read CLI quota\nmainAgent: true\nsubagent: false\ntools: []\nmcpServers: []\n---\nRead the CLI quota report only.\n",
        0o600,
    ),
    #[cfg(unix)]
    (BROWSER_GUARD, "#!/bin/sh\nexit 1\n", 0o700),
    #[cfg(unix)]
    ("xdg-open", "#!/bin/sh\nexit 1\n", 0o700),
    #[cfg(windows)]
    (BROWSER_GUARD, "@exit /b 1\r\n", 0o600),
];

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
    fn create(target: ExecutionTarget, files: &[(&str, &str, u32)]) -> Result<Self, ()> {
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
            for (relative, content, mode) in files {
                use std::io::Write;
                let path = workspace.path.join(relative);
                fs::create_dir_all(path.parent().ok_or(())?).map_err(|_| ())?;
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(*mode);
                }
                #[cfg(not(unix))]
                let _ = mode;
                options
                    .open(path)
                    .and_then(|mut file| file.write_all(content.as_bytes()))
                    .map_err(|_| ())?;
            }
            return Ok(workspace);
        }

        let path = PathBuf::from(format!("/tmp/{name}"));
        let mut mkdir = command("python3");
        mkdir
            .args([
                "-c",
                r#"
import json, os, shutil, sys
path = sys.argv[1]
os.mkdir(path, 0o700)
try:
    for relative, content, mode in json.loads(sys.argv[2]):
        file_path = os.path.join(path, relative)
        os.makedirs(os.path.dirname(file_path), exist_ok=True)
        fd = os.open(file_path, os.O_CREAT | os.O_WRONLY | os.O_EXCL, mode)
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            file.write(content)
except BaseException:
    shutil.rmtree(path, ignore_errors=True)
    raise
"#,
            ])
            .arg(&path)
            .arg(serde_json::to_string(files).map_err(|_| ())?);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn quota_checks_exact_cli_then_preflights_acp_in_the_same_isolation() {
        use serde_json::{Value, json};

        const SCRIPT: &str = include_str!("../tests/fixtures/antigravity-quota/fake-agy.sh");
        const REPORT: &str = include_str!("../tests/fixtures/antigravity-quota/usage.json");
        const MODELS: &str = include_str!("../tests/fixtures/antigravity-quota/models.json");
        let fixture = Workspace::create(
            ExecutionTarget::Local,
            &[
                ("agy", SCRIPT, 0o700),
                ("version", "1.5.9\n", 0o600),
                ("report", REPORT, 0o600),
            ],
        )
        .unwrap();
        let legacy: Value = serde_json::from_str(MODELS).unwrap();
        let reply = |message: Value| {
            fs::write(fixture.path.join("session"), format!("{message}\n")).unwrap()
        };
        reply(json!({"jsonrpc":"2.0","id":2,"result":legacy}));
        let launcher = Launcher::new(fixture.path.join("agy").to_str().unwrap().to_owned(), None);
        let windows = launcher.quota_windows(60_000).await.unwrap();
        assert_eq!(windows.len(), 5);
        let argon = windows
            .iter()
            .find(|window| window.id == "gemini-argon-wtu-weekly")
            .unwrap();
        assert_eq!(
            argon.model_ids.as_deref().unwrap(),
            ["gemini-4-argon-high", "gemini-4-argon-medium"]
        );
        assert_eq!(
            fs::read_to_string(fixture.path.join("calls")).unwrap(),
            "--version\nacp\nsession/close\n-p\n"
        );
        let runs = fs::read_to_string(fixture.path.join("workspaces")).unwrap();
        let mut paths = runs.lines();
        let cwd = paths.next().unwrap();
        assert_eq!(paths.collect::<Vec<_>>(), [cwd, cwd]);
        assert_ne!(Path::new(cwd), fixture.path);
        assert!(!Path::new(cwd).exists());

        let modern = json!({
            "jsonrpc":"2.0","id":2,"result":{
                "sessionId":"fixture-session",
                "configOptions":[{
                    "id":"model","category":"model","type":"select","currentValue":"gemini-4-hydrogen-medium",
                    "options":[{"group":"gemini","name":"Gemini","options":[
                        {"value":"gemini-4-hydrogen-medium","name":"Gemini 4 Hydrogen (Medium)"},
                        {"value":"gemini-4-hydrogen-xhigh","name":"Gemini 4 Hydrogen (XHigh)"}
                    ]}]
                }]
            }
        });
        reply(modern);
        let windows = launcher.quota_windows(60_000).await.unwrap();
        assert_eq!(
            windows[0].model_ids.as_deref().unwrap(),
            ["gemini-4-hydrogen-medium", "gemini-4-hydrogen-xhigh"]
        );
        assert!(windows[4].model_ids.as_deref().unwrap().is_empty());

        reply(json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":"fixture-session"}}));
        let windows = launcher.quota_windows(60_000).await.unwrap();
        assert!(windows.iter().all(|window| {
            window
                .model_ids
                .as_deref()
                .is_some_and(|ids| ids.is_empty())
        }));

        let calls_path = fixture.path.join("calls");
        for version in [
            "1.1.11",
            "1.4.9",
            "1.5.9-beta",
            "agy 1.5.9",
            "1.5",
            "invalid",
        ] {
            fs::write(fixture.path.join("version"), version).unwrap();
            fs::write(&calls_path, "").unwrap();
            assert!(launcher.quota_windows(60_000).await.is_err());
            assert_eq!(fs::read_to_string(&calls_path).unwrap(), "--version\n");
        }
        fs::write(fixture.path.join("version"), "1.5.0").unwrap();
        fs::write(&calls_path, "").unwrap();
        reply(
            json!({"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}}),
        );
        assert!(launcher.quota_windows(60_000).await.is_err());
        assert_eq!(fs::read_to_string(&calls_path).unwrap(), "--version\nacp\n");

        fs::write(&calls_path, "").unwrap();
        reply(json!({"jsonrpc":"2.0","id":2,"result":legacy}));
        // A valid prefix alone is insufficient when the complete payload is oversized.
        fs::write(
            fixture.path.join("report"),
            format!("{REPORT}{}", " ".repeat(70_000)),
        )
        .unwrap();
        assert!(launcher.quota_windows(60_000).await.is_err());
        assert_eq!(
            fs::read_to_string(&calls_path).unwrap(),
            "--version\nacp\nsession/close\n-p\n"
        );
    }

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
            [
                "AGY_ADC_AUTH",
                "CLOUD_CODE_URL",
                "GEMINI_API_KEY",
                "GOOGLE_API_KEY",
                "GOOGLE_APPLICATION_CREDENTIALS",
                "GOOGLE_CLOUD_LOCATION",
                "GOOGLE_CLOUD_PROJECT",
                "GOOGLE_CLOUD_QUOTA_PROJECT",
                "GOOGLE_GEMINI_BASE_URL",
                "GOOGLE_GENAI_USE_VERTEXAI",
                "GOOGLE_OAUTH_ACCESS_TOKEN",
            ]
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
