use super::{
    layout::{Format, Layout},
    *,
};
use std::path::Path;

fn skill_declaration(layout: &Layout, key: &ResourceKey) -> bool {
    let Format::SkillTree { flat_markdown } = layout.format else {
        return false;
    };
    // DSH treats each root Markdown file as a declaration. Pi only opts such a file in
    // when it has a description; a plain README remains an editable asset there.
    Path::new(&key.path)
        .file_name()
        .is_some_and(|name| name == "SKILL.md")
        || flat_markdown && !key.path.contains('/') && key.path.ends_with(".md")
}

pub(super) fn needs_text(layout: &Layout, key: &ResourceKey) -> bool {
    !matches!(layout.format, Format::SkillTree { .. }) || skill_declaration(layout, key)
}

pub(super) fn content(layout: &Layout, key: &ResourceKey, bytes: &[u8]) -> Result<()> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Error::new(ErrorCode::Invalid, "资源超过 16 MiB 上限"));
    }
    let frontmatter =
        matches!(layout.format, Format::MarkdownAgents) || skill_declaration(layout, key);
    if matches!(layout.format, Format::SkillTree { .. }) && !frontmatter {
        return Ok(());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::new(ErrorCode::Invalid, "此原生声明必须为 UTF-8 文本"))?;
    if frontmatter {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lines = text.lines();
        if lines.next() != Some("---") {
            return Err(Error::new(
                ErrorCode::Invalid,
                "原生定义缺少 YAML frontmatter",
            ));
        }
        let mut header = String::new();
        let mut closed = false;
        for line in lines {
            if line == "---" {
                closed = true;
                break;
            }
            header.push_str(line);
            header.push('\n');
        }
        if !closed {
            return Err(Error::new(
                ErrorCode::Invalid,
                "原生定义的 YAML frontmatter 未闭合",
            ));
        }
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&header)
            .map_err(|_| Error::new(ErrorCode::Invalid, "原生定义的 YAML frontmatter 无效"))?;
        if !value.is_mapping() {
            return Err(Error::new(
                ErrorCode::Invalid,
                "原生定义的 YAML frontmatter 必须是对象",
            ));
        }
        // Skill loaders differ on optional names and descriptions; source editing must not impose
        // a stricter shared schema than the native Harness. Agent definitions require both fields.
        if matches!(layout.format, Format::MarkdownAgents) {
            for field in ["name", "description"] {
                if value
                    .get(field)
                    .and_then(|value| value.as_str())
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(Error::new(
                        ErrorCode::Invalid,
                        format!("原生定义需要非空 {field}"),
                    ));
                }
            }
        }
    } else if matches!(layout.format, Format::TomlAgents) {
        let value = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| Error::new(ErrorCode::Invalid, "原生 Agent TOML 无效"))?;
        for field in ["name", "description", "developer_instructions"] {
            if value
                .get(field)
                .and_then(|value| value.as_str())
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(Error::new(
                    ErrorCode::Invalid,
                    format!("原生 Agent 定义需要非空 {field}"),
                ));
            }
        }
    }
    Ok(())
}
