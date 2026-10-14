//! Codex owns logical history across forks, revert rollouts and history bases.
use super::{Error, Result, codex, rpc::RpcClient};
use crate::{conversation::*, history::History};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::BufRead,
    path::Path,
};

const SOURCE_KINDS: [&str; 10] = [
    "cli",
    "vscode",
    "exec",
    "appServer",
    "subAgent",
    "subAgentReview",
    "subAgentCompact",
    "subAgentThreadSpawn",
    "subAgentOther",
    "unknown",
];

fn summary(thread: &Value, runtime_id: &str) -> Result<ConversationSummary> {
    let id = thread["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::new("Codex 历史缺少原生会话身份"))?;
    Ok(ConversationSummary {
        id: format!("{runtime_id}:{id}"),
        runtime_id: runtime_id.into(),
        title: conversation_title(thread["name"].as_str(), thread["preview"].as_str()),
        cwd: thread["cwd"].as_str().map(str::to_owned),
        created_at_unix_ms: thread["createdAt"]
            .as_i64()
            .and_then(|v| v.checked_mul(1000)),
        updated_at_unix_ms: thread["updatedAt"]
            .as_i64()
            .and_then(|v| v.checked_mul(1000)),
        can_rename: false,
        can_delete: false,
        // Only a spawned thread names its parent; Codex's other internal
        // sub-agents (review, compaction) are listed as they are.
        parent_conversation_id: thread["parentThreadId"]
            .as_str()
            .or_else(|| thread["source"]["subAgent"]["thread_spawn"]["parent_thread_id"].as_str())
            .map(|parent| format!("{runtime_id}:{parent}")),
    })
}

// Native cursors are opaque; repeating one is a contract failure, not a reason
// to loop forever. The SDK owns page sizes and the adapter does not impose a
// second aggregate history-size ceiling.
fn pages(rpc: &mut RpcClient, method: &str, mut params: Value) -> Result<Vec<Value>> {
    let mut entries = Vec::new();
    let mut cursors = HashSet::new();
    loop {
        let page = rpc.request(method, &params)?;
        let data = page["data"]
            .as_array()
            .ok_or_else(|| Error::new("Codex 历史分页缺少内容"))?;
        entries.extend(data.iter().cloned());
        match page.get("nextCursor") {
            Some(Value::Null) => break,
            Some(Value::String(cursor)) if !cursor.is_empty() && cursors.insert(cursor.clone()) => {
                params["cursor"] = json!(cursor)
            }
            _ => return Err(Error::new("Codex 历史分页游标无效")),
        }
    }
    Ok(entries)
}

pub fn list_codex_history(
    binary: &Path,
    home: &Path,
    runtime_id: &str,
) -> Result<Vec<ConversationSummary>> {
    list_codex_history_at(
        binary,
        home,
        runtime_id,
        false,
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
/// The runtime's threads, either those in use or those it archived.
pub fn list_codex_history_at(
    binary: &Path,
    home: &Path,
    runtime_id: &str,
    archived: bool,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<Vec<ConversationSummary>> {
    let mut rpc = codex::history_client_at(binary, home, execution, os_home)?;
    let threads = pages(
        &mut rpc,
        "thread/list",
        json!({
            "limit":100,"archived":archived,"modelProviders":[],
            "sourceKinds":SOURCE_KINDS,"sortKey":"updated_at","sortDirection":"desc"
        }),
    )?;
    let mut seen = HashSet::new();
    threads
        .iter()
        .map(|thread| {
            let summary = summary(thread, runtime_id)?;
            if !seen.insert(summary.id.clone()) {
                return Err(Error::new("Codex 返回重复逻辑会话"));
            }
            Ok(summary)
        })
        .collect()
}

pub fn read_codex_history(
    binary: &Path,
    home: &Path,
    runtime_id: &str,
    id: &str,
) -> Result<History> {
    read_codex_history_at(
        binary,
        home,
        runtime_id,
        id,
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
pub fn read_codex_history_at(
    binary: &Path,
    home: &Path,
    runtime_id: &str,
    id: &str,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<History> {
    let mut rpc = codex::history_client_at(binary, home, execution, os_home)?;
    let metadata = rpc.request("thread/read", &json!({"threadId":id,"includeTurns":false}))?;
    let thread = &metadata["thread"];
    if thread["id"].as_str() != Some(id) {
        return Err(Error::new("Codex 返回不同的原生会话"));
    }
    let history_mode = thread["historyMode"]
        .as_str()
        .filter(|mode| matches!(*mode, "legacy" | "paginated"))
        .ok_or_else(|| Error::new("Codex 返回未知的历史契约"))?;
    let title = conversation_title(thread["name"].as_str(), thread["preview"].as_str());
    let items = pages(
        &mut rpc,
        "thread/items/list",
        json!({"threadId":id,"limit":50,"sortDirection":"asc"}),
    )?;
    // The rollout is a file on the executing machine; only a local one is read.
    let turns = match (execution, thread["path"].as_str()) {
        (crate::execution::ExecutionTarget::Local, Some(path)) => rollout_turns(Path::new(path)),
        _ => HashMap::new(),
    };
    let messages: Vec<Message> = items
        .iter()
        .filter_map(|entry| {
            let mut message = codex::project_item(&entry["item"], runtime_id)?;
            message.timestamp_unix_ms = entry["startedAtMs"]
                .as_i64()
                .or_else(|| entry["completedAtMs"].as_i64());
            if message.role == MessageRole::User {
                let turn = entry["turnId"].as_str().and_then(|turn| turns.get(turn));
                message.turn_settings = Some(turn.map(|t| t.settings.clone()).unwrap_or_default());
                message.turn_usage = turn.and_then(|t| t.usage.clone());
            }
            Some(message)
        })
        .collect();
    tracing::info!(target:"velune_agent_runtime",event="history_native_resolution",phase="read",runtime_family="codex",native_items=items.len(),projected_messages=messages.len(),history_mode);
    Ok(History {
        native_id: id.into(),
        title,
        cwd: thread["cwd"].as_str().map(str::to_owned),
        messages,
    })
}

/// What a turn of a rollout ran with and used.
#[derive(Default)]
pub(super) struct RolloutTurn {
    pub(super) settings: TurnSettings,
    pub(super) usage: Option<TurnUsage>,
}

/// Each turn of a rollout, from Codex's `turn_context` records, the running
/// totals of its `token_usage_record`s, and the `token_count` event that
/// follows each of those with the model's window and the account's limits.
/// Best effort: an unreadable file or record leaves the turns unnamed.
pub(super) fn rollout_turns(path: &Path) -> HashMap<String, RolloutTurn> {
    let Ok(file) = std::fs::File::open(path) else {
        return HashMap::new();
    };
    let mut turns: HashMap<String, RolloutTurn> = HashMap::new();
    // A `token_count` event names no turn; it belongs to the record before it.
    let mut counted: Option<String> = None;
    let records = std::io::BufReader::new(file)
        .lines()
        .map_while(std::result::Result::ok)
        .filter(|line| {
            [
                "\"turn_context\"",
                "\"token_usage_record\"",
                "\"token_count\"",
            ]
            .iter()
            .any(|kind| line.contains(kind))
        })
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok());
    for record in records {
        let payload = &record["payload"];
        if record["type"] == "event_msg" && payload["type"] == "token_count" {
            if let Some(usage) = counted
                .as_ref()
                .and_then(|turn| turns.get_mut(turn))
                .and_then(|turn| turn.usage.as_mut())
            {
                let info = &payload["info"];
                usage.context_window = info["model_context_window"]
                    .as_u64()
                    .or(usage.context_window);
                let limits = rate_limits(
                    &payload["rate_limits"],
                    "used_percent",
                    "window_minutes",
                    "resets_at",
                );
                if !limits.is_empty() {
                    usage.rate_limits = limits;
                }
            }
            continue;
        }
        let Some(turn) = payload["turn_id"].as_str() else {
            continue;
        };
        let entry = turns.entry(turn.to_owned()).or_default();
        let text = |value: &Value| value.as_str().filter(|v| !v.is_empty()).map(str::to_owned);
        match record["type"].as_str() {
            Some("turn_context") => {
                entry.settings = TurnSettings {
                    runtime_id: None,
                    model: text(&payload["model"]),
                    effort: text(&payload["effort"]).or_else(|| {
                        text(&payload["collaboration_mode"]["settings"]["reasoning_effort"])
                    }),
                    permission_mode: permission_mode(payload),
                };
            }
            // Each record carries the turn's total so far; the last one counts.
            Some("token_usage_record") if payload["turn_token_usage"].is_object() => {
                let mut usage = codex_usage(&payload["turn_token_usage"], ROLLOUT_KEYS);
                usage.context_tokens = context_tokens(&payload["usage"], ROLLOUT_KEYS);
                if let Some(previous) = &entry.usage {
                    usage.context_window = previous.context_window;
                    usage.rate_limits = previous.rate_limits.clone();
                }
                entry.usage = Some(usage);
                counted = Some(turn.to_owned());
            }
            _ => {}
        }
    }
    turns
}

/// Keys of Codex's token counts: input, cached input, cache writes, output, reasoning.
pub(super) type UsageKeys = [&'static str; 5];
pub(super) const ROLLOUT_KEYS: UsageKeys = [
    "input_tokens",
    "cached_input_tokens",
    "cache_write_input_tokens",
    "output_tokens",
    "reasoning_output_tokens",
];
pub(super) const APP_SERVER_KEYS: UsageKeys = [
    "inputTokens",
    "cachedInputTokens",
    "cacheWriteInputTokens",
    "outputTokens",
    "reasoningOutputTokens",
];

/// Codex counts cached input within input, and reasoning within output, as
/// the Responses API does.
pub(super) fn codex_usage(
    usage: &Value,
    [input, cached, cache_write, output, reasoning]: UsageKeys,
) -> TurnUsage {
    TurnUsage {
        input_tokens: usage[input].as_u64(),
        cache_read_tokens: usage[cached].as_u64(),
        cache_write_tokens: usage[cache_write].as_u64(),
        output_tokens: usage[output].as_u64(),
        reasoning_tokens: usage[reasoning].as_u64(),
        ..TurnUsage::default()
    }
}

/// What one request leaves in the context: its input and its output.
pub(super) fn context_tokens(request: &Value, [input, _, _, output, _]: UsageKeys) -> Option<u64> {
    Some(request[input].as_u64()? + request[output].as_u64()?)
}

/// The primary and secondary windows of Codex's account limits; `resets_at` is in seconds.
pub(super) fn rate_limits(
    limits: &Value,
    used: &str,
    window: &str,
    resets: &str,
) -> Vec<RateLimit> {
    ["primary", "secondary"]
        .iter()
        .filter_map(|name| {
            let limit = &limits[name];
            Some(RateLimit {
                used_percent: limit[used].as_f64()?.round().clamp(0.0, 100.0) as u32,
                window_minutes: limit[window].as_u64(),
                resets_at_unix_ms: limit[resets].as_i64().map(|seconds| seconds * 1000),
            })
        })
        .collect()
}

/// The preset Velune offers that a turn's policies amount to, or the policies
/// themselves when they match none.
fn permission_mode(context: &Value) -> Option<String> {
    let approval = context["approval_policy"].as_str()?;
    let sandbox = context["sandbox_policy"]["type"].as_str()?;
    let reviewer = context["approvals_reviewer"].as_str().unwrap_or("user");
    Some(
        match (approval, sandbox, reviewer) {
            ("on-request", "read-only", "user") => "read-only",
            ("on-request", "workspace-write", "user") => "workspace",
            ("on-request", "workspace-write", "auto_review") => "auto-review",
            ("never", "danger-full-access", _) => "full-access",
            _ => return Some(format!("{approval} · {sandbox}")),
        }
        .to_owned(),
    )
}
