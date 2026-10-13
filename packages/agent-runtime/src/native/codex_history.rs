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
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
pub fn list_codex_history_at(
    binary: &Path,
    home: &Path,
    runtime_id: &str,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<Vec<ConversationSummary>> {
    let mut rpc = codex::history_client_at(binary, home, execution, os_home)?;
    let threads = pages(
        &mut rpc,
        "thread/list",
        json!({
            "limit":100,"archived":false,"modelProviders":[],
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
        (crate::execution::ExecutionTarget::Local, Some(path)) => turn_contexts(Path::new(path)),
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
                message.turn_settings = Some(
                    entry["turnId"]
                        .as_str()
                        .and_then(|turn| turns.get(turn))
                        .cloned()
                        .unwrap_or_default(),
                );
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

/// What each turn of a rollout ran with, from Codex's `turn_context` records.
/// Best effort: an unreadable file or record leaves the turns unnamed.
fn turn_contexts(path: &Path) -> HashMap<String, TurnSettings> {
    let Ok(file) = std::fs::File::open(path) else {
        return HashMap::new();
    };
    std::io::BufReader::new(file)
        .lines()
        .map_while(std::result::Result::ok)
        .filter(|line| line.contains("\"turn_context\""))
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter(|record| record["type"] == "turn_context")
        .filter_map(|record| {
            let context = &record["payload"];
            let turn = context["turn_id"].as_str()?.to_owned();
            let text = |value: &Value| value.as_str().filter(|v| !v.is_empty()).map(str::to_owned);
            Some((
                turn,
                TurnSettings {
                    runtime_id: None,
                    model: text(&context["model"]),
                    effort: text(&context["effort"]).or_else(|| {
                        text(&context["collaboration_mode"]["settings"]["reasoning_effort"])
                    }),
                    permission_mode: permission_mode(context),
                },
            ))
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
