//! xURL addresses identify native conversations, never an execution instance.
use super::organization::ListedSource;
use super::*;
use crate::{AgentUriResolution, HistoryFailure};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AgentUri {
    pub provider: String,
    pub session: Option<String>,
    pub child: Option<String>,
    pub query: Option<String>,
    pub limit: usize,
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub(super) fn provider(kind: RuntimeKind) -> Option<&'static str> {
    match kind {
        RuntimeKind::Codex | RuntimeKind::Codex166 => Some("codex"),
        RuntimeKind::ClaudeCode => Some("claude"),
        RuntimeKind::Pi => Some("pi"),
        RuntimeKind::DeepSeek | RuntimeKind::Acp => None,
    }
}

impl AgentUri {
    pub(super) fn parse(input: &str) -> Result<Self, RuntimeError> {
        let target = input
            .strip_prefix("agents://")
            .ok_or_else(|| RuntimeError::invalid("需要 agents:// 会话地址"))?;
        if input.len() > 4096 || input.chars().any(char::is_control) || target.contains('#') {
            return Err(RuntimeError::invalid("agents:// 地址无效"));
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let parts: Vec<_> = path.split('/').collect();
        if !matches!(parts.first(), Some(&"codex" | &"claude" | &"pi")) {
            return Err(RuntimeError::Unsupported("Velune 地址入口目前支持 codex、claude、pi；其他 Harness、路径与角色查询请使用官方 xurl CLI".into()));
        }
        if parts.len() > 3 || parts.iter().any(|part| part.is_empty()) {
            return Err(RuntimeError::invalid(
                "会话地址应为 agents://Harness/会话ID[/子会话ID]",
            ));
        }
        let mut result = Self {
            provider: parts[0].into(),
            session: None,
            child: None,
            query: None,
            limit: 10,
        };
        if let Some(id) = parts.get(1) {
            if !uuid(id) {
                return Err(RuntimeError::invalid(
                    "会话 ID 必须是原生 UUID；角色查询请使用 xurl CLI",
                ));
            }
            result.session = Some(id.to_ascii_lowercase());
        }
        if let Some(child) = parts.get(2) {
            let valid = match result.provider.as_str() {
                "codex" => uuid(child),
                "claude" => {
                    child.len() <= 128
                        && child
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                }
                _ => false,
            };
            if !valid {
                return Err(RuntimeError::invalid(
                    "此子会话地址不受支持；Pi 条目地址请使用 xurl CLI",
                ));
            }
            result.child = Some(if result.provider == "codex" {
                child.to_ascii_lowercase()
            } else {
                child.strip_prefix("agent-").unwrap_or(child).to_string()
            });
            if result.child.as_deref() == Some("") {
                return Err(RuntimeError::invalid("子会话 ID 不能为空"));
            }
        }
        if !query.is_empty() && result.session.is_some() {
            return Err(RuntimeError::invalid("打开会话不接受查询或 CLI 执行参数"));
        }
        // form_urlencoded is permissive about malformed escapes; reject them at ingress.
        let bytes = query.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'%'
                && (i + 2 >= bytes.len()
                    || !bytes[i + 1].is_ascii_hexdigit()
                    || !bytes[i + 2].is_ascii_hexdigit())
            {
                return Err(RuntimeError::invalid("地址查询的百分号编码无效"));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if !seen.insert(key.to_string()) {
                return Err(RuntimeError::invalid("地址包含重复查询参数"));
            }
            match key.as_ref() {
                "q" => result.query = Some(value.to_lowercase()),
                "limit" => {
                    result.limit = value
                        .parse::<usize>()
                        .ok()
                        .filter(|n| (1..=100).contains(n))
                        .ok_or_else(|| RuntimeError::invalid("limit 必须在 1 到 100 之间"))?
                }
                _ => {
                    return Err(RuntimeError::invalid(
                        "Velune 查询只接受 q（标题）和 limit；完整搜索请使用 xurl CLI",
                    ));
                }
            }
        }
        Ok(result)
    }
}

impl CoreRuntime {
    pub(super) fn agents_uri(&self, summary: &ConversationSummary) -> Option<String> {
        // A linked history contains several native segments; one URI cannot represent it.
        if self
            .conversation_links
            .iter()
            .any(|link| link.id == summary.id)
        {
            return None;
        }
        let runtime = self.runtime_instance(&summary.runtime_id).ok()?;
        let family = provider(runtime.type_id)?;
        let prefix = format!("{}:", runtime.id);
        let native = summary.id.strip_prefix(&prefix)?;
        let id = if family == "pi" {
            self.indexed_session_id(runtime, &summary.id)?
        } else if family == "codex" {
            match summary
                .parent_conversation_id
                .as_deref()
                .and_then(|id| id.strip_prefix(&prefix))
            {
                Some(parent) => format!("{parent}/{native}"),
                None => native.into(),
            }
        } else {
            native.replace("/agent-", "/")
        };
        let uri = format!("agents://{family}/{id}");
        AgentUri::parse(&uri).ok().map(|_| uri)
    }

    pub(crate) fn resolve_agent_uri(
        &self,
        input: &str,
        runtime_id: Option<&str>,
    ) -> Result<AgentUriResolution, RuntimeError> {
        let uri = AgentUri::parse(input)?;
        let runtimes: Vec<_> = self
            .runtime_instances
            .iter()
            .filter(|r| r.enabled && provider(r.type_id) == Some(uri.provider.as_str()))
            .filter(|r| runtime_id.is_none_or(|id| r.id == id))
            .collect();
        if runtimes.is_empty() {
            return Err(RuntimeError::invalid(
                "当前设备没有匹配且已启用的运行时实例",
            ));
        }
        let mut result = AgentUriResolution {
            conversations: Vec::new(),
            failures: Vec::new(),
        };
        let places = self.projectless_places();
        for runtime in runtimes {
            match self.resolve_agent_uri_in(&uri, runtime) {
                Ok(sources) => {
                    for source in sources {
                        self.reached
                            .borrow_mut()
                            .insert(source.summary.id.clone(), source.clone());
                        result.conversations.push(self.listed(&source, &places));
                    }
                }
                Err(error) => result.failures.push(HistoryFailure {
                    runtime_id: runtime.id.clone(),
                    detail: error.to_string(),
                }),
            }
        }
        result
            .conversations
            .sort_by_key(|item| std::cmp::Reverse(item.conversation.updated_at_unix_ms));
        if uri.session.is_none() {
            result.conversations.truncate(uri.limit);
        }
        Ok(result)
    }

    fn resolve_agent_uri_in(
        &self,
        uri: &AgentUri,
        runtime: &RuntimeInstance,
    ) -> Result<Vec<ListedSource>, RuntimeError> {
        self.index_history(runtime)?;
        if uri.provider == "codex"
            && let Some(session) = &uri.session
        {
            let native = uri.child.as_ref().unwrap_or(session);
            let id = format!("{}:{native}", runtime.id);
            // thread/read does not distinguish a missing thread from other provider errors.
            // Establish membership through the native lists, including archived children.
            let found = match self.listed_summary(runtime, &id) {
                Some(summary) => Some((summary, false)),
                None => history::list_archived(&self.history_config(runtime)?, &runtime.id)?
                    .into_iter()
                    .find(|summary| summary.id == id)
                    .map(|summary| (summary, true)),
            };
            return Ok(found
                .into_iter()
                .filter(|(summary, _)| {
                    uri.child.is_none()
                        || summary.parent_conversation_id.as_deref()
                            == Some(format!("{}:{session}", runtime.id).as_str())
                })
                .map(|(summary, archived_natively)| ListedSource {
                    summary,
                    archived_natively,
                    provisional: false,
                })
                .collect());
        }
        let entries = self.indexed(runtime);
        let ids: Vec<String> = if let Some(session) = &uri.session {
            if uri.provider == "pi" {
                entries
                    .iter()
                    .filter(|entry| {
                        entry
                            .hint
                            .session_id
                            .as_deref()
                            .is_some_and(|id| id.eq_ignore_ascii_case(session))
                    })
                    .map(|entry| entry.id.clone())
                    .collect()
            } else {
                vec![format!(
                    "{}:{session}{}",
                    runtime.id,
                    uri.child
                        .as_ref()
                        .map(|id| format!("/agent-{id}"))
                        .unwrap_or_default()
                )]
            }
        } else {
            entries.iter().map(|entry| entry.id.clone()).collect()
        };
        self.read_indexed(runtime, &ids)?;
        Ok(ids
            .into_iter()
            .filter_map(|id| self.listed_summary(runtime, &id))
            .filter(|summary| {
                uri.query
                    .as_ref()
                    .is_none_or(|q| summary.title.display_text().to_lowercase().contains(q))
            })
            .map(|summary| ListedSource {
                summary,
                archived_natively: false,
                provisional: false,
            })
            .collect())
    }

    /// Ambiguity and unreadable sources must be resolved before an Agent sends anything.
    pub(super) fn unique_agent_uri(
        &self,
        uri: &str,
        runtime: Option<&str>,
    ) -> Result<ConversationSummary, RuntimeError> {
        if AgentUri::parse(uri)?.session.is_none() {
            return Err(RuntimeError::invalid(
                "需要具体会话地址；先查询并选择一个会话",
            ));
        }
        let found = self.resolve_agent_uri(uri, runtime)?;
        if !found.failures.is_empty() {
            return Err(RuntimeError::context(
                "部分运行时读取失败，无法确认唯一目标；指定 runtime_id 后重试",
                serde_json::to_string(&found.failures)?,
            ));
        }
        match found.conversations.as_slice() {
            [one] => Ok(one.conversation.clone()),
            [] => Err(RuntimeError::invalid("当前设备的配置实例中找不到该会话")),
            _ => Err(RuntimeError::invalid(
                "地址匹配多个会话；指定 runtime_id，或使用查询返回的 conversation_id",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses_do_not_carry_execution_or_filesystem_arguments() {
        let id = "019c871c-b1f9-7f60-9c4f-87ed09f13592";
        assert_eq!(
            AgentUri::parse(&format!("agents://claude/{id}/agent-a12"))
                .unwrap()
                .child
                .as_deref(),
            Some("a12")
        );
        let query = AgentUri::parse("agents://codex?q=hello+world&limit=3").unwrap();
        assert_eq!(query.query.as_deref(), Some("hello world"));
        assert_eq!(query.limit, 3);
        for address in [
            format!("agents://codex/{id}?model=x"),
            format!("agents://claude/{id}/.."),
            "agents://codex/../../file".into(),
            "agents://codex?q=%xx".into(),
            "agents://codex?limit=0".into(),
            "agents://codex?limit=1&limit=2".into(),
            "agents://codex?cd=/tmp".into(),
            "agents://codex#x".into(),
        ] {
            assert!(AgentUri::parse(&address).is_err(), "{address}");
        }
    }
}
