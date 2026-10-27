//! Address interoperation and instructions for the separately bundled upstream CLI.
use super::*;
use sha2::Digest;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::local) struct ResolveUri {
    pub uri: String,
    pub runtime_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::local) struct ReadConversation {
    conversation_id: String,
    runtime_id: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    expected_revision: Option<String>,
}

fn text_page(
    text: &str,
    offset: usize,
    limit: usize,
    expected: Option<&str>,
) -> Result<Value, RuntimeError> {
    if !(1..=64_000).contains(&limit) {
        return Err(RuntimeError::invalid("limit 必须在 1 到 64000 之间"));
    }
    let revision = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
    if expected.is_some_and(|expected| expected != revision) {
        return Err(RuntimeError::invalid("会话内容已变化，请从第一页重新读取"));
    }
    let total = text.chars().count();
    if offset > total {
        return Err(RuntimeError::invalid("offset 超出会话文字长度"));
    }
    let end = offset.saturating_add(limit).min(total);
    Ok(
        json!({"text":text.chars().skip(offset).take(limit).collect::<String>(),
        "offset":offset,"next_offset":(end < total).then_some(end),"total_chars":total,"revision":revision}),
    )
}

impl CoreRuntime {
    pub(super) fn agent_reference(
        &self,
        id: &str,
        runtime_id: Option<&str>,
    ) -> Result<String, RuntimeError> {
        if id.starts_with("agents://") {
            let resolved = self.unique_agent_uri(id, runtime_id)?.id;
            if self
                .conversation_links
                .iter()
                .any(|link| link.id == resolved && link.segments.len() > 1)
            {
                return Err(RuntimeError::invalid(
                    "此原生会话已跨 Harness 接续；请用 resolve_agent_uri 返回的 Velune conversation.id 明确选择逻辑会话，或用官方 xurl 读取原生段",
                ));
            }
            return Ok(resolved);
        }
        if runtime_id.is_some_and(|runtime| !id.starts_with(&format!("{runtime}:"))) {
            return Err(RuntimeError::invalid(
                "conversation_id 与 runtime_id 不匹配",
            ));
        }
        Ok(id.into())
    }

    pub(super) fn read_for_agent(&self, request: ReadConversation) -> Result<Value, RuntimeError> {
        let id = self.agent_reference(&request.conversation_id, request.runtime_id.as_deref())?;
        let view = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.conversation_id() == Some(id.as_str()))
            .and_then(|session| session.projected())
            .cloned();
        let view = match view {
            Some(view) => view,
            None => {
                let found: Option<crate::api::ListedConversation> = serde_json::from_value(
                    self.find_conversation(&json!({"payload":{"conversationID":id}}))?,
                )?;
                let summary = found
                    .ok_or_else(|| RuntimeError::invalid("没有这个会话"))?
                    .conversation;
                self.read_conversation_source(&summary.runtime_id, &summary.id)?
                    .0
            }
        };
        let mut text = String::new();
        for message in &view.messages {
            text.push_str(&format!("\n## {:?}\n", message.role));
            for block in &message.blocks {
                match block {
                    MessageBlock::Text { text: body }
                    | MessageBlock::Reasoning { text: body }
                    | MessageBlock::Notice { text: body }
                    | MessageBlock::ContextCompaction { text: body } => text.push_str(body),
                    MessageBlock::Tool { title, output, .. } => {
                        text.push_str(title);
                        text.push('\n');
                        if let Some(output) = output {
                            text.push_str(output);
                        }
                    }
                    MessageBlock::Image { .. } => text.push_str("[图片：文字读取不包含图片数据]"),
                }
                text.push('\n');
            }
        }
        let mut page = text_page(
            &text,
            request.offset.unwrap_or(0),
            request.limit.unwrap_or(16_000),
            request.expected_revision.as_deref(),
        )?;
        page["conversation_id"] = json!(id);
        page["runtime_id"] = json!(view.conversation.runtime_id);
        page["agents_uri"] = json!(self.agents_uri(&view.conversation));
        Ok(page)
    }

    pub(super) fn xurl_invocation(
        &self,
        runtime: &RuntimeInstance,
        caller: Option<&RuntimeInstance>,
    ) -> Option<Value> {
        if !caller.is_some_and(|caller| same_machine(caller, runtime)) {
            return None;
        }
        let family = super::super::agent_uri::provider(&runtime.type_id)?;
        let (target, home) = self.runtime_location(runtime).ok()?;
        // A local binary is not portable to an SSH target's architecture.
        if !target.is_local()
            || (family == "pi"
                && runtime
                    .settings
                    .get("sessionDir")
                    .is_some_and(|path| !path.is_empty()))
        {
            return None;
        }
        let binary = self
            .options
            .resources_directory
            .join("xurl/bin")
            .join(if cfg!(windows) { "xurl.exe" } else { "xurl" });
        if !binary.is_file() {
            return None;
        }
        let directory = self
            .native_agent_directory(runtime, home.as_deref())
            .ok()??;
        let home = home.or_else(std::env::home_dir)?;
        let (root_key, binary_key) = match family {
            "codex" => ("CODEX_HOME", "XURL_CODEX_BIN"),
            "claude" => ("CLAUDE_CONFIG_DIR", "XURL_CLAUDE_BIN"),
            "pi" => ("PI_CODING_AGENT_DIR", "XURL_PI_BIN"),
            _ => return None,
        };
        let mut environment = BTreeMap::from([
            ("HOME", home.to_string_lossy().into_owned()),
            (root_key, directory.to_string_lossy().into_owned()),
            (binary_key, runtime.settings.get("binary")?.clone()),
        ]);
        // Script entrypoints use /usr/bin/env node, even when launched by xurl.
        if let Some(node) = runtime.settings.get("nodeBinary") {
            let directory = Path::new(node).parent()?;
            let inherited = std::env::var_os("PATH").unwrap_or_default();
            let paths =
                std::iter::once(directory.to_path_buf()).chain(std::env::split_paths(&inherited));
            environment.insert(
                "PATH",
                std::env::join_paths(paths)
                    .ok()?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        Some(
            json!({"executable":binary,"environment":environment,"provider":family,
            "write_mode":"independent_native_cli","instructions":"调用 xurl_instructions；Velune 管理会话优先用 send_message 继续"}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_keep_unicode_and_reject_changed_history() {
        let text = "甲🙂乙\nreply";
        let first = text_page(text, 0, 2, None).unwrap();
        assert_eq!(first["text"], "甲🙂");
        assert_eq!(first["next_offset"], 2);
        let rest = text_page(text, 2, 64, first["revision"].as_str()).unwrap();
        assert_eq!(rest["text"], "乙\nreply");
        assert!(rest["next_offset"].is_null());
        assert!(text_page("changed", 2, 64, first["revision"].as_str()).is_err());
        assert!(text_page(text, 100, 64, None).is_err());
    }
}
