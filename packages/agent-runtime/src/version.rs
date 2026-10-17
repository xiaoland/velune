//! Versioned adapter identities. A family is a grouping, never a dispatch key.
use crate::native::{Error, Result};
use regex::Regex;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
/// Protocols this versioned adapter can carry through the injected LLM gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeProtocol {
    ChatCompletionsV1,
    ResponsesV1,
    MessagesV1,
}
/// A permission mode a runtime version offers a turn: its own identifier and
/// name, with what it allows.
pub struct PermissionMode {
    pub id: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
}
/// Claude Code 2.1 `--permission-mode` choices, in its own cycle order.
pub(crate) const CLAUDE_PERMISSION_MODES: &[PermissionMode] = &[
    PermissionMode {
        id: "manual",
        label: "Manual",
        detail: "需要权限的操作逐次询问",
    },
    PermissionMode {
        id: "acceptEdits",
        label: "Accept edits",
        detail: "自动接受文件编辑，其他需要权限的操作仍询问",
    },
    PermissionMode {
        id: "plan",
        label: "Plan",
        detail: "只分析并制定计划，不修改文件或执行操作",
    },
    PermissionMode {
        id: "auto",
        label: "Auto",
        detail: "由 Claude Code 的安全分类器决定放行或拦截",
    },
    PermissionMode {
        id: "dontAsk",
        label: "Don't ask",
        detail: "未预先允许的操作直接拒绝，不询问",
    },
    PermissionMode {
        id: "bypassPermissions",
        label: "Bypass permissions",
        detail: "跳过全部权限检查，请谨慎使用",
    },
];
/// Codex 0.159 built-in permission presets and Auto-review, applied per
/// turn as approval policy, sandbox policy and approvals reviewer.
pub(crate) const CODEX_PERMISSION_MODES: &[PermissionMode] = &[
    PermissionMode {
        id: "read-only",
        label: "Read Only",
        detail: "可读取工作区文件；编辑文件或访问网络需要批准",
    },
    PermissionMode {
        id: "workspace",
        label: "Default",
        detail: "可读写工作区并运行命令；访问网络或编辑其他文件需要批准",
    },
    PermissionMode {
        id: "auto-review",
        label: "Auto-review",
        detail: "同 Default，需要批准的操作先由 Codex 自动审查",
    },
    PermissionMode {
        id: "full-access",
        label: "Full Access",
        detail: "无需批准即可编辑工作区外的文件和访问网络，请谨慎使用",
    },
];
/// The versions an adapter carries.
pub enum Versions {
    /// A dedicated adapter's CLI versions: from the one verified with the
    /// actual CLI up to, not including, the next version that may break it
    /// (a semantic minor version; for 0.x the minor too). Prereleases of
    /// `below` are outside.
    Cli {
        minimum: &'static str,
        below: &'static str,
    },
    /// The protocol major version `acp-v1` negotiates.
    AcpProtocol(u64),
}
impl Versions {
    /// Whether a version the CLI reports is in the range.
    pub fn contains(&self, version: &str) -> bool {
        let Self::Cli { minimum, below } = self else {
            return false;
        };
        let (Ok(version), Ok(minimum), Ok(below)) = (
            semver::Version::parse(version),
            semver::Version::parse(minimum),
            semver::Version::parse(below),
        ) else {
            return false;
        };
        version >= minimum
            && (version.major, version.minor, version.patch)
                < (below.major, below.minor, below.patch)
    }
    /// The range as users read it.
    pub fn describe(&self) -> String {
        match self {
            Self::Cli { minimum, below } => format!("≥{minimum}，<{below}"),
            Self::AcpProtocol(major) => format!("协议 {major}"),
        }
    }
}
pub struct RuntimeVariant {
    pub id: &'static str,
    pub family_id: &'static str,
    pub name: &'static str,
    pub versions: Versions,
    pub can_rename_conversations: bool,
    pub can_delete_conversations: bool,
    /// The runtime archives conversations itself; others are archived by Velune.
    pub can_archive_conversations: bool,
    /// Where under the user's home the runtime starts conversations given no
    /// project; they belong to no project.
    pub projectless_directories: &'static [&'static str],
    pub supported_protocols: &'static [RuntimeProtocol],
    /// Empty when the runtime offers no per-turn choice.
    pub permission_modes: &'static [PermissionMode],
}
/// Pi's range, shared with the Pi provider import.
pub const PI_VERSIONS: Versions = Versions::Cli {
    minimum: "1.0.2",
    below: "1.1.0",
};
const VARIANTS: &[RuntimeVariant] = &[
    RuntimeVariant {
        id: "acp-v1",
        family_id: "acp",
        name: "ACP v1",
        versions: Versions::AcpProtocol(1),
        can_rename_conversations: false,
        can_delete_conversations: false,
        can_archive_conversations: false,
        projectless_directories: &[],
        supported_protocols: &[],
        permission_modes: &[],
    },
    RuntimeVariant {
        id: "claude-code-2.1",
        family_id: "claude-code",
        name: "Claude Code 2.1",
        versions: Versions::Cli {
            minimum: "2.1.286",
            below: "2.2.0",
        },
        can_rename_conversations: false,
        can_delete_conversations: false,
        can_archive_conversations: false,
        projectless_directories: &[],
        supported_protocols: &[RuntimeProtocol::MessagesV1],
        permission_modes: CLAUDE_PERMISSION_MODES,
    },
    RuntimeVariant {
        id: "pi-1.0",
        family_id: "pi",
        name: "Pi Agent 1.0",
        versions: PI_VERSIONS,
        can_rename_conversations: true,
        can_delete_conversations: true,
        can_archive_conversations: false,
        projectless_directories: &[],
        supported_protocols: &[
            RuntimeProtocol::ChatCompletionsV1,
            RuntimeProtocol::ResponsesV1,
            RuntimeProtocol::MessagesV1,
        ],
        permission_modes: &[],
    },
    RuntimeVariant {
        id: "codex-0.159",
        family_id: "codex",
        name: "Codex 0.159",
        versions: Versions::Cli {
            minimum: "0.159.3",
            below: "0.160.0",
        },
        can_rename_conversations: true,
        can_delete_conversations: true,
        can_archive_conversations: true,
        // The Codex app's default folder for chats without a project.
        projectless_directories: &["Documents/Codex"],
        supported_protocols: &[RuntimeProtocol::ResponsesV1],
        permission_modes: CODEX_PERMISSION_MODES,
    },
    RuntimeVariant {
        id: "dsh-acp-0.2",
        family_id: "deepseek-harness",
        name: "DeepSeek Harness 0.2",
        versions: Versions::Cli {
            minimum: "0.2.0-rc.2",
            below: "0.3.0",
        },
        can_rename_conversations: false,
        can_delete_conversations: false,
        can_archive_conversations: false,
        projectless_directories: &[],
        supported_protocols: &[
            RuntimeProtocol::ChatCompletionsV1,
            RuntimeProtocol::ResponsesV1,
            RuntimeProtocol::MessagesV1,
        ],
        permission_modes: &[],
    },
];
pub fn variants() -> &'static [RuntimeVariant] {
    VARIANTS
}
pub fn variant(id: &str) -> Option<&'static RuntimeVariant> {
    VARIANTS.iter().find(|v| v.id == id)
}
pub fn check_version(type_id: &str, binary: &Path, node_binary: Option<&Path>) -> Result<String> {
    check_version_at(
        type_id,
        binary,
        node_binary,
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
pub fn check_version_at(
    type_id: &str,
    binary: &Path,
    node_binary: Option<&Path>,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<String> {
    let adapter = variant(type_id).ok_or_else(|| Error::new("运行时版本类型不受支持"))?;
    let version = probe_version_at(adapter.family_id, binary, node_binary, execution, os_home)?;
    if !adapter.versions.contains(&version) {
        return Err(Error::new(format!(
            "运行时版本 {version} 不在 {} 的兼容范围（{}）内；请选择对应版本的运行时类型。",
            adapter.name,
            adapter.versions.describe()
        )));
    }
    Ok(version)
}

/// Resolve only a Pi installation's public CLI and manifest, including known
/// npm/pnpm literal launchers. No shell evaluation or Agent startup occurs.
pub fn resolve_pi_binary(binary: &Path, node: &Path) -> Result<PathBuf> {
    resolve_pi_binary_at(
        binary,
        node,
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
pub fn resolve_pi_binary_at(
    binary: &Path,
    node: &Path,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<PathBuf> {
    if !binary.is_absolute() || !node.is_absolute() {
        return Err(Error::new("Pi 与 Node 入口必须为绝对路径"));
    }
    let script = concat!(
        include_str!("../resources/pi_sdk.mjs"),
        r#"
import {inspect} from "node:util";
try {
  const installation = resolvePiInstallation(process.argv[1]);
  console.log(JSON.stringify({binary:installation.cli}));
} catch (error) {
  console.log(JSON.stringify({error:error?.code ?? "sdk_resolution_failed", detail:error?.stack ?? error?.message ?? String(error), configuredBinary:process.argv[1], path:error?.path ?? null}));
}
"#
    );
    let mut command = Command::new(node);
    command
        .args(["--input-type=module", "--eval", script])
        .arg(binary);
    let output = execution.output(command, None, os_home)?;
    let value: serde_json::Value = serde_json::from_slice(&output)
        .map_err(|error| Error::new(format!("Pi 安装解析未返回有效结果：{error}")))?;
    if let Some(binary) = value["binary"]
        .as_str()
        .filter(|path| Path::new(path).is_absolute())
    {
        return Ok(PathBuf::from(binary));
    }
    let (message, code) = match value["error"].as_str() {
        Some("sdk_entrypoint_missing") => (
            "Pi 安装入口缺失或启动链接已失效，请修复外部安装后重试",
            "entrypoint_missing",
        ),
        Some("sdk_source_not_found") => (
            "无法找到此入口所属的 Pi 安装包，请选择已安装的 Pi CLI",
            "package_missing",
        ),
        Some("sdk_launcher_unsupported") => (
            "无法解析此 Pi 启动脚本，请手动选择安装包中的 CLI",
            "launcher_unsupported",
        ),
        Some("invalid_sdk_manifest") => (
            "Pi 安装包描述或 CLI 关联无效，请检查外部安装",
            "invalid_package",
        ),
        Some(error) => (error, "installation_resolution_failed"),
        None => (
            "无法解析 Pi 安装，请检查入口与 Node 配置",
            "installation_resolution_failed",
        ),
    };
    let detail = value["detail"].as_str().unwrap_or("");
    let configured = value["configuredBinary"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| binary.to_string_lossy().into_owned());
    tracing::warn!(
        runtime_family = "pi",
        code,
        phase = "installation",
        configured_binary = %configured,
        detail = %detail,
        "runtime discovery failed"
    );
    Err(Error::with_code(
        format!("{message}；configuredBinary={configured}；cause={detail}"),
        code,
    ))
}

/// Read only the public CLI version; never start an Agent session.
pub fn probe_version(family_id: &str, binary: &Path, node_binary: Option<&Path>) -> Result<String> {
    probe_version_at(
        family_id,
        binary,
        node_binary,
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
pub fn probe_version_at(
    family_id: &str,
    binary: &Path,
    node_binary: Option<&Path>,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<String> {
    if !VARIANTS.iter().any(|v| v.family_id == family_id) {
        return Err(Error::new("未知运行时系列"));
    }
    if family_id == "acp" {
        return Err(Error::with_code(
            "通用 ACP 通过 acp::Client::connect 的 initialize 握手检查协议版本，不执行 CLI 版本探测",
            "unsupported",
        ));
    }
    if !binary.is_absolute() || node_binary.is_some_and(|p| !p.is_absolute()) {
        return Err(Error::new("运行时可执行文件必须为绝对路径"));
    }
    let resolved;
    let binary = if family_id == "pi" {
        resolved = resolve_pi_binary_at(
            binary,
            node_binary.ok_or_else(|| Error::new("Pi 需要配置 Node 可执行文件"))?,
            execution,
            os_home,
        )?;
        resolved.as_path()
    } else {
        binary
    };
    let mut command = if matches!(family_id, "codex" | "claude-code") {
        Command::new(binary)
    } else if let Some(node) = node_binary {
        let mut command = Command::new(node);
        command.arg(binary);
        command
    } else {
        return Err(Error::new("此运行时需要配置 Node 可执行文件"));
    };
    command.arg("--version");
    let bytes = execution
        .output(command, None, os_home)
        .map_err(Error::from)
        .map_err(|error| {
            tracing::warn!(
                runtime_family = family_id,
                code = error.code(),
                phase = "version",
                "runtime version probe failed"
            );
            if error.code() == "process_exit" {
                Error::with_code(
                    "公开版本命令执行失败，请检查外部安装是否完整及 Node 版本",
                    "version_probe_failed",
                )
            } else {
                error
            }
        })?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| Error::new(format!("运行时版本不是有效文本：{error}")))?;
    let extractor = Regex::new(r"\b[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?\b")
        .map_err(|error| Error::new(format!("运行时版本规则无效：{error}")))?;
    let versions = extractor
        .find_iter(text)
        .map(|m| m.as_str())
        .collect::<Vec<_>>();
    if versions.len() != 1 {
        return Err(Error::new("无法唯一识别运行时版本"));
    }
    let version = versions[0];
    Ok(version.into())
}

pub fn matching_variant(family_id: &str, version: &str) -> Option<&'static RuntimeVariant> {
    VARIANTS
        .iter()
        .find(|v| v.family_id == family_id && v.versions.contains(version))
}
