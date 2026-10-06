//! Versioned adapter identities. A family is a grouping, never a dispatch key.
use crate::native::{Error, Result, rpc::bounded_output};
use regex::Regex;
use std::{path::Path, process::Command};
/// Protocols this versioned adapter can carry through the injected LLM gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeProtocol {
    ChatCompletionsV1,
    ResponsesV1,
}
pub struct RuntimeVariant {
    pub id: &'static str,
    pub family_id: &'static str,
    pub name: &'static str,
    pub version_regex: &'static str,
    pub supported_protocols: &'static [RuntimeProtocol],
}
const VARIANTS: &[RuntimeVariant] = &[
    RuntimeVariant {
        id: "pi-1.0.2",
        family_id: "pi",
        name: "Pi Agent 1.0.2",
        version_regex: r"^1\.0\.2$",
        supported_protocols: &[
            RuntimeProtocol::ChatCompletionsV1,
            RuntimeProtocol::ResponsesV1,
        ],
    },
    RuntimeVariant {
        id: "codex-0.159.3",
        family_id: "codex",
        name: "Codex 0.159.3",
        version_regex: r"^0\.159\.3$",
        supported_protocols: &[RuntimeProtocol::ResponsesV1],
    },
    RuntimeVariant {
        id: "dsh-acp-0.2.0-rc.2",
        family_id: "deepseek-harness",
        name: "DeepSeek Harness 0.2.0-rc.2",
        version_regex: r"^0\.2\.0-rc\.2$",
        supported_protocols: &[
            RuntimeProtocol::ChatCompletionsV1,
            RuntimeProtocol::ResponsesV1,
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
    if !binary.is_absolute() || node_binary.is_some_and(|p| !p.is_absolute()) {
        return Err(Error::new("运行时可执行文件必须为绝对路径"));
    }
    let mut command = if adapter.id == "codex-0.159.3" {
        Command::new(binary)
    } else if let Some(node) = node_binary {
        let mut command = Command::new(node);
        command.arg(binary);
        command
    } else {
        return Err(Error::new("此运行时需要配置 Node 可执行文件"));
    };
    command.arg("--version");
    let bytes = bounded_output(command)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::new("运行时版本不是有效文本"))?;
    let extractor = Regex::new(r"\b[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?\b")
        .map_err(|_| Error::new("运行时版本规则无效"))?;
    let versions = extractor
        .find_iter(text)
        .map(|m| m.as_str())
        .collect::<Vec<_>>();
    if versions.len() != 1 {
        return Err(Error::new("无法唯一识别运行时版本"));
    }
    let version = versions[0];
    if !Regex::new(adapter.version_regex)
        .map_err(|_| Error::new("适配器版本规则无效"))?
        .is_match(version)
    {
        return Err(Error::new(format!(
            "运行时版本 {version} 不属于所选类型 {}（{}）；请选择对应版本的运行时类型。",
            adapter.name, adapter.version_regex
        )));
    }
    Ok(version.into())
}
