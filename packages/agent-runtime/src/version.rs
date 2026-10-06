//! Versioned adapter identities. A family is a grouping, never a dispatch key.
use crate::native::{Error, Result, rpc::bounded_output};
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
pub struct RuntimeVariant {
    pub id: &'static str,
    pub family_id: &'static str,
    pub name: &'static str,
    pub version_regex: &'static str,
    pub can_rename_conversations: bool,
    pub can_delete_conversations: bool,
    pub supported_protocols: &'static [RuntimeProtocol],
}
const VARIANTS: &[RuntimeVariant] = &[
    RuntimeVariant {
        id: "pi-1.0.2",
        family_id: "pi",
        name: "Pi Agent 1.0.2",
        version_regex: r"^1\.0\.2$",
        can_rename_conversations: true,
        can_delete_conversations: true,
        supported_protocols: &[
            RuntimeProtocol::ChatCompletionsV1,
            RuntimeProtocol::ResponsesV1,
            RuntimeProtocol::MessagesV1,
        ],
    },
    RuntimeVariant {
        id: "codex-0.159.3",
        family_id: "codex",
        name: "Codex 0.159.3",
        version_regex: r"^0\.159\.3$",
        can_rename_conversations: true,
        can_delete_conversations: true,
        supported_protocols: &[RuntimeProtocol::ResponsesV1],
    },
    RuntimeVariant {
        id: "dsh-acp-0.2.0-rc.2",
        family_id: "deepseek-harness",
        name: "DeepSeek Harness 0.2.0-rc.2",
        version_regex: r"^0\.2\.0-rc\.2$",
        can_rename_conversations: false,
        can_delete_conversations: false,
        supported_protocols: &[
            RuntimeProtocol::ChatCompletionsV1,
            RuntimeProtocol::ResponsesV1,
            RuntimeProtocol::MessagesV1,
        ],
    },
];
pub fn variants() -> &'static [RuntimeVariant] {
    VARIANTS
}
pub fn variant(id: &str) -> Option<&'static RuntimeVariant> {
    VARIANTS.iter().find(|v| v.id == id)
}
pub fn check_version(type_id: &str, binary: &Path, node_binary: Option<&Path>) -> Result<String> {
    let adapter = variant(type_id).ok_or_else(|| Error::new("运行时版本类型不受支持"))?;
    let version = probe_version(adapter.family_id, binary, node_binary)?;
    if !Regex::new(adapter.version_regex)
        .map_err(|error| Error::new(format!("适配器版本规则无效：{error}")))?
        .is_match(&version)
    {
        return Err(Error::new(format!(
            "运行时版本 {version} 不属于所选类型 {}（{}）；请选择对应版本的运行时类型。",
            adapter.name, adapter.version_regex
        )));
    }
    Ok(version)
}

/// Resolve only a Pi installation's public CLI and manifest, including known
/// npm/pnpm literal launchers. No shell evaluation or Agent startup occurs.
pub fn resolve_pi_binary(binary: &Path, node: &Path) -> Result<PathBuf> {
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
    let output = bounded_output(command)?;
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
    if !VARIANTS.iter().any(|v| v.family_id == family_id) {
        return Err(Error::new("未知运行时系列"));
    }
    if !binary.is_absolute() || node_binary.is_some_and(|p| !p.is_absolute()) {
        return Err(Error::new("运行时可执行文件必须为绝对路径"));
    }
    let resolved;
    let binary = if family_id == "pi" {
        resolved = resolve_pi_binary(
            binary,
            node_binary.ok_or_else(|| Error::new("Pi 需要配置 Node 可执行文件"))?,
        )?;
        resolved.as_path()
    } else {
        binary
    };
    let mut command = if family_id == "codex" {
        Command::new(binary)
    } else if let Some(node) = node_binary {
        let mut command = Command::new(node);
        command.arg(binary);
        command
    } else {
        return Err(Error::new("此运行时需要配置 Node 可执行文件"));
    };
    command.arg("--version");
    let bytes = bounded_output(command).map_err(|error| {
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
    VARIANTS.iter().find(|v| {
        v.family_id == family_id && Regex::new(v.version_regex).is_ok_and(|r| r.is_match(version))
    })
}
