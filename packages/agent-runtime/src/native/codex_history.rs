//! Codex owns logical history across forks, revert rollouts and history bases.
use super::{Error, Result, codex, rpc::RpcClient};
use crate::{conversation::*, history::History};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

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
    })
}

// All pages share the existing history output ceiling. Native cursors are
// opaque; repeating one is a contract failure, not a reason to loop forever.
fn pages(rpc: &mut RpcClient, method: &str, mut params: Value) -> Result<Vec<Value>> {
    let mut entries = Vec::new();
    let mut cursors = HashSet::new();
    let mut bytes = 0usize;
    loop {
        let page = rpc.request(method, &params)?;
        bytes = bytes.saturating_add(
            serde_json::to_vec(&page)
                .map_err(|error| Error::new(format!("Codex 历史分页无效：{error}")))?
                .len(),
        );
        if bytes > 16 * 1024 * 1024 {
            return Err(Error::with_code("Codex 历史超出读取上限", "output_limit"));
        }
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
    let mut rpc = codex::history_client(binary, home)?;
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

pub fn read_codex_history(binary: &Path, home: &Path, id: &str) -> Result<History> {
    let mut rpc = codex::history_client(binary, home)?;
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
    let messages: Vec<Message> = items
        .iter()
        .filter_map(|entry| {
            let mut message = codex::project_item(&entry["item"])?;
            message.timestamp_unix_ms = entry["startedAtMs"]
                .as_i64()
                .or_else(|| entry["completedAtMs"].as_i64());
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
