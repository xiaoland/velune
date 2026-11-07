use super::*;
use std::path::Path;
use velune_conversation::RuntimeKind;

#[derive(Clone, Copy)]
pub(super) enum Format {
    Instructions,
    SkillTree { flat_markdown: bool },
    TomlAgents,
    MarkdownAgents,
    JsonMcp,
    TomlMcp,
}

pub(super) struct Layout {
    pub root: PathBuf,
    pub prefix: String,
    pub candidates: Vec<&'static str>,
    pub format: Format,
}
impl Layout {
    pub fn source_path(&self, path: &str) -> String {
        if self.prefix.is_empty() {
            path.into()
        } else if path.is_empty() {
            self.prefix.clone()
        } else {
            format!("{}/{path}", self.prefix)
        }
    }
    pub fn relative_path<'a>(&self, path: &'a str) -> &'a str {
        if self.prefix.is_empty() {
            path
        } else {
            path.strip_prefix(&self.prefix)
                .unwrap_or(path)
                .trim_start_matches('/')
        }
    }
    pub fn is_mcp(&self) -> bool {
        matches!(self.format, Format::JsonMcp | Format::TomlMcp)
    }
}

pub(super) fn unsupported(config: &Config, kind: ResourceKind) -> Option<&'static str> {
    unsupported_variant(config.runtime, kind)
}

pub(super) fn unsupported_variant(
    runtime: RuntimeKind,
    kind: ResourceKind,
) -> Option<&'static str> {
    match (runtime, kind) {
        (RuntimeKind::Acp, _) => {
            Some("ACP v1 未声明原生持久资源管理协议，无法从启动命令推断资源位置")
        }
        (RuntimeKind::Pi, ResourceKind::NativeAgentDefinitions) => {
            Some("Pi 1.0.2 没有内置子 Agent 定义格式；扩展提供的定义不在当前管理范围")
        }
        (
            RuntimeKind::DeepSeek,
            ResourceKind::McpServers | ResourceKind::NativeAgentDefinitions,
        ) => Some("DSH 支持此运行能力，但当前管理适配未支持其 Cordis 插件声明与 patch 语义"),
        (
            RuntimeKind::Codex166
            | RuntimeKind::ClaudeCode
            | RuntimeKind::Pi
            | RuntimeKind::DeepSeek,
            _,
        ) => None,
    }
}

pub(super) fn resolve(config: &Config, kind: ResourceKind, scope: ResourceScope) -> Result<Layout> {
    if let Some(reason) = unsupported(config, kind) {
        return Err(Error::new(ErrorCode::Unsupported, reason));
    }
    config
        .execution
        .validate()
        .map_err(|_| Error::new(ErrorCode::Invalid, "运行时执行目标无效"))?;
    let root = match scope {
        ResourceScope::User => config.agent_dir.as_deref(),
        ResourceScope::Project => config.cwd.as_deref(),
    }
    .filter(|path| path.is_absolute())
    .ok_or_else(|| {
        Error::new(
            ErrorCode::Invalid,
            "此资源范围需要目标机器上的绝对原生配置目录或项目目录",
        )
    })?
    .to_path_buf();
    let family = config.runtime;
    let user = scope == ResourceScope::User;
    let (prefix, candidates, format) = match kind {
        ResourceKind::UserInstructions => {
            let candidates = match family {
                RuntimeKind::ClaudeCode if user => vec!["CLAUDE.md"],
                RuntimeKind::ClaudeCode => {
                    vec!["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"]
                }
                RuntimeKind::Codex166 => {
                    vec!["AGENTS.md", "AGENTS.override.md"]
                }
                RuntimeKind::Pi => vec![
                    "AGENTS.md",
                    "AGENTS.override.md",
                    "AGENTS.MD",
                    "CLAUDE.md",
                    "CLAUDE.MD",
                ],
                RuntimeKind::DeepSeek | RuntimeKind::Acp if user => vec!["AGENTS.md"],
                RuntimeKind::DeepSeek | RuntimeKind::Acp => vec![
                    "AGENTS.md",
                    "CLAUDE.md",
                    "AGENTS.local.md",
                    "CLAUDE.local.md",
                ],
            };
            ("", candidates, Format::Instructions)
        }
        ResourceKind::Skills => {
            let prefix = if user {
                "skills"
            } else {
                match family {
                    RuntimeKind::Codex166 => ".agents/skills",
                    RuntimeKind::ClaudeCode => ".claude/skills",
                    RuntimeKind::Pi => ".pi/skills",
                    RuntimeKind::DeepSeek | RuntimeKind::Acp => ".dsh/skills",
                }
            };
            (
                prefix,
                vec![],
                Format::SkillTree {
                    flat_markdown: family == RuntimeKind::DeepSeek,
                },
            )
        }
        ResourceKind::NativeAgentDefinitions => {
            let prefix = if user {
                "agents"
            } else if matches!(family, RuntimeKind::Codex166) {
                ".codex/agents"
            } else {
                ".claude/agents"
            };
            let format = if matches!(family, RuntimeKind::Codex166) {
                Format::TomlAgents
            } else {
                Format::MarkdownAgents
            };
            (prefix, vec![], format)
        }
        ResourceKind::McpServers => {
            let path = match (family, user) {
                (RuntimeKind::Codex166, true) => "config.toml",
                (RuntimeKind::Codex166, false) => ".codex/config.toml",
                (RuntimeKind::ClaudeCode, true) => ".claude.json",
                (RuntimeKind::ClaudeCode, false) => ".mcp.json",
                (RuntimeKind::Pi | RuntimeKind::DeepSeek | RuntimeKind::Acp, true) => "mcp.json",
                (RuntimeKind::Pi | RuntimeKind::DeepSeek | RuntimeKind::Acp, false) => {
                    ".pi/mcp.json"
                }
            };
            (
                path,
                vec![],
                if matches!(family, RuntimeKind::Codex166) {
                    Format::TomlMcp
                } else {
                    Format::JsonMcp
                },
            )
        }
    };
    Ok(Layout {
        root,
        prefix: prefix.into(),
        candidates,
        format,
    })
}

pub(super) fn validate_path(layout: &Layout, key: &ResourceKey) -> Result<()> {
    let path = Path::new(&key.path);
    if key.path.is_empty()
        || key.path.contains(['\0', '\\'])
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(Error::new(
            ErrorCode::Invalid,
            "资源标识必须为范围内的相对路径",
        ));
    }
    let accepted = match layout.format {
        Format::Instructions => layout
            .candidates
            .iter()
            .any(|candidate| *candidate == key.path),
        Format::TomlAgents => path.extension().is_some_and(|value| value == "toml"),
        Format::MarkdownAgents => path.extension().is_some_and(|value| value == "md"),
        Format::SkillTree { .. } => true,
        Format::JsonMcp | Format::TomlMcp => true,
    };
    if !accepted {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "该路径不属于此类原生资源的可管理入口",
        ));
    }
    Ok(())
}
