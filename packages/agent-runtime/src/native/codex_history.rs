//! Codex owns logical history across forks, revert rollouts and history bases.
use super::{Error, ModelOption, Result, RuntimeModel, codex, rpc::RpcClient};
use crate::{conversation::*, creation::Retained, history::History};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub(super) const SOURCE_KINDS: [&str; 10] = [
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
pub(super) fn is_handoff_summary_thread(thread: &Value) -> bool {
    thread["threadSource"]
        .as_str()
        .is_some_and(|source| source.starts_with("velune_handoff_summary:"))
}

pub(super) fn summary(thread: &Value, runtime_id: &str) -> Result<ConversationSummary> {
    let id = thread["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::new("Codex 历史缺少原生会话身份"))?;
    Ok(ConversationSummary {
        native_read_state: read_state(thread.get("readState"), runtime_id, id)?,
        id: format!("{runtime_id}:{id}"),
        runtime_id: runtime_id.into(),
        title: thread_title(thread),
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

/// Stored names and native agent nicknames are authoritative across listing,
/// history and live resume. Internal agents may have no user-message preview.
pub(super) fn thread_title(thread: &Value) -> ConversationTitle {
    let name = thread["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            thread["agentNickname"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
        });
    match conversation_title(name, thread["preview"].as_str()) {
        ConversationTitle::Untitled => conversation_title(thread["agentRole"].as_str(), None),
        title => title,
    }
}

// Native cursors are opaque; repeating one is a contract failure, not a reason
// to loop forever. The SDK owns page sizes and the adapter does not impose a
// second aggregate history-size ceiling.
pub(super) fn pages(
    rpc: &mut RpcClient,
    method: &str,
    mut params: Value,
) -> Result<Retained<Vec<Value>>> {
    let mut entries: Option<Retained<Vec<Value>>> = None;
    let mut cursors = HashSet::new();
    loop {
        let mut page = rpc.request(method, &params)?;
        if matches!(method, "thread/items/list" | "thread/turns/list") {
            page.settle_history_page();
        }
        let (data, next) = page
            .try_map(|mut page| {
                let Some(Value::Array(mut data)) = page.get_mut("data").map(Value::take) else {
                    return Err(Error::new("Codex 历史分页缺少内容"));
                };
                // Move each receipt, rather than cloning a subtree out of its owner.
                for entry in &mut data {
                    if method == "thread/list"
                        && let Some(id) = entry["id"].as_str()
                    {
                        let id = id.to_owned();
                        if let Some(receipt) = page["readStates"].get_mut(&id) {
                            entry["readState"] = receipt.take();
                        }
                    }
                }
                Ok((data, page["nextCursor"].take()))
            })?
            .split_map(|parts| parts);
        entries = Some(match entries {
            None => data,
            Some(previous) => previous.zip(data).map(|(mut previous, data)| {
                previous.extend(data);
                previous
            }),
        });
        match next.as_ref() {
            Value::Null => break,
            Value::String(cursor) if !cursor.is_empty() && cursors.insert(cursor.clone()) => {
                params["cursor"] = json!(cursor)
            }
            _ => return Err(Error::new("Codex 历史分页游标无效")),
        }
    }
    Ok(entries.expect("the first page always owns an aggregate"))
}

/// The runtime's threads, either those in use or those it archived.
pub fn list_codex_history_at(
    kind: crate::version::RuntimeKind,
    binary: &Path,
    home: &Path,
    runtime_id: &str,
    archived: bool,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<Retained<Vec<ConversationSummary>>> {
    let mut rpc = codex::history_client_at(kind, binary, home, execution, os_home)?;
    let threads = pages(
        &mut rpc,
        "thread/list",
        json!({
            "limit":100,"archived":archived,"modelProviders":[],
            "sourceKinds":SOURCE_KINDS,"sortKey":"updated_at","sortDirection":"desc"
        }),
    )?;
    let mut seen = HashSet::new();
    threads.try_map(|threads| {
        threads
            .iter()
            .filter(|thread| !is_handoff_summary_thread(thread))
            .map(|thread| {
                let summary = summary(thread, runtime_id)?;
                if !seen.insert(summary.id.clone()) {
                    return Err(Error::new("Codex 返回重复逻辑会话"));
                }
                Ok(summary)
            })
            .collect()
    })
}

/// One thread as the runtime describes it, and whether it is archived, for a
/// thread no listing in use holds: a native sub-agent Codex archived once it
/// ended. Codex keeps archived rollouts in its `archived_sessions` folder.
pub fn read_codex_summary_at(
    kind: crate::version::RuntimeKind,
    binary: &Path,
    home: &Path,
    runtime_id: &str,
    id: &str,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<Retained<(ConversationSummary, bool)>> {
    let mut rpc = codex::history_client_at(kind, binary, home, execution, os_home)?;
    let metadata = rpc.request("thread/read", &json!({"threadId":id,"includeTurns":false}))?;
    metadata.try_map(|metadata| {
        let thread = &metadata["thread"];
        if thread["id"].as_str() != Some(id) {
            return Err(Error::new("Codex 返回不同的原生会话"));
        }
        let archived = thread["path"].as_str().is_some_and(|path| {
            Path::new(path)
                .components()
                .any(|component| component.as_os_str() == "archived_sessions")
        });
        let mut summary = summary(thread, runtime_id)?;
        summary.native_read_state = read_state(metadata.get("readState"), runtime_id, id)?;
        Ok((summary, archived))
    })
}

pub fn codex_context_provider(
    config: &crate::history::HistoryConfig,
    cwd: &Path,
) -> Result<String> {
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let value = rpc.request("config/read", &json!({"includeLayers":false,"cwd":cwd}))?;
    Ok(value["config"]["model_provider"]
        .as_str()
        .unwrap_or("openai")
        .to_owned())
}

/// Materialize a newly imported rollout's paginated history without starting
/// a user turn. Until resume, the official items/list index can be empty.
pub fn materialize_codex_import(
    config: &crate::history::HistoryConfig,
    id: &str,
    provider: &str,
) -> Result<()> {
    let mut rpc = codex::history_client_at_with_provider(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
        Some(provider),
    )?;
    let params = json!({"threadId":id,"excludeTurns":true,"modelProvider":provider});
    let resumed = rpc.request("thread/resume", &params)?;
    if resumed["thread"]["id"].as_str() != Some(id) {
        return Err(Error::new("Codex 返回不同的导入会话"));
    }
    rpc.shutdown();
    Ok(())
}

pub fn materialize_owned_codex_import(
    config: &crate::history::HistoryConfig,
    id: &str,
    provider: &str,
    ownership: &mut crate::import::ImportOwnership,
) -> Result<()> {
    if !ownership.owns_codex(id) {
        return Err(Error::new("导入副本所有权不属于此 Codex 会话"));
    }
    ownership.verify(config, false)?;
    let materialized = materialize_codex_import(config, id, provider);
    // materialize's RPC child has ended on both success and failure. Capture
    // only its recognized metadata append before any caller attempts cleanup.
    ownership.refresh_after_materialize(config)?;
    materialized
}

pub(crate) fn cleanup_owned_codex_import(
    config: &crate::history::HistoryConfig,
    id: &str,
    canonical_path: &str,
    ownership: &crate::import::ImportOwnership,
) -> Result<()> {
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let metadata = read_metadata(&mut rpc, id)?;
    if metadata["thread"]["path"].as_str() != Some(canonical_path) {
        return Err(Error::new(format!(
            "导入副本路径已改变，保留来源：{}",
            ownership.source_path().display()
        )));
    }
    ownership.verify(config, false)?;
    rpc.request("thread/delete", &json!({"threadId":id}))?;
    rpc.shutdown();
    Ok(())
}

/// Resolve the authoritative rollout through Codex before reading model input.
pub fn export_codex_context(
    config: &crate::history::HistoryConfig,
    id: &str,
) -> Result<Retained<crate::import::Context>> {
    export_codex_context_at(config, id, &[], false, None)
}

/// Exports effective Codex context with an explicitly generated summary.
/// For encrypted deliveries the summary covers the whole effective history.
pub fn export_codex_context_with_summary(
    config: &crate::history::HistoryConfig,
    id: &str,
    summary: &str,
) -> Result<Retained<crate::import::Context>> {
    if summary.trim().is_empty() {
        return Err(Error::with_code(
            "portable summary is empty",
            "invalid_input",
        ));
    }
    export_codex_context_at(config, id, &[], false, Some(summary))
}

pub fn export_codex_context_before(
    config: &crate::history::HistoryConfig,
    id: &str,
    message_id: &str,
) -> Result<Retained<crate::import::Context>> {
    export_codex_context_at(config, id, &[message_id.to_owned()], false, None)
}

/// Effective native input with exactly one uncompressed user prompt omitted.
pub fn export_codex_context_without(
    config: &crate::history::HistoryConfig,
    id: &str,
    message_id: &str,
) -> Result<Retained<crate::import::Context>> {
    export_codex_context_without_many(config, id, &[message_id.to_owned()])
}

pub fn export_codex_context_without_many(
    config: &crate::history::HistoryConfig,
    id: &str,
    message_ids: &[String],
) -> Result<Retained<crate::import::Context>> {
    export_codex_context_at(config, id, message_ids, true, None)
}

/// Checks for encrypted Codex context without a portable representation.
/// It returns no history text or encrypted payload.
pub fn codex_requires_portable_summary(
    config: &crate::history::HistoryConfig,
    id: &str,
) -> Result<bool> {
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let metadata = read_metadata(&mut rpc, id)?;
    let path = metadata["thread"]["path"]
        .as_str()
        .ok_or_else(|| Error::new("Codex 未提供上下文来源路径"))?;
    let bytes = config.execution.read_file(Path::new(path))?;
    let mut command = config.execution.python();
    command
        .args(["-c", include_str!("../../resources/codex_context.py")])
        .arg("--source-path")
        .arg(path)
        .arg("--home")
        .arg(&config.home)
        .arg("--requires-portable-summary");
    let result = config
        .execution
        .output(command, Some(bytes), config.os_home.as_deref())?;
    #[derive(Deserialize)]
    struct Inspection {
        required: bool,
    }
    let inspected: Inspection = serde_json::from_slice(&result)
        .map_err(|error| Error::new(format!("Codex 摘要能力检查返回无效结果：{error}")))?;
    Ok(inspected.required)
}

/// Returns the thread's effective model and provider without reading or
/// returning conversation content. Call on a history worker, not the runtime
/// owner thread.
pub fn codex_thread_model(
    config: &crate::history::HistoryConfig,
    id: &str,
) -> Result<Retained<(String, String)>> {
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let metadata = read_metadata(&mut rpc, id)?;
    metadata.try_map(|metadata| {
        let thread = &metadata["thread"];
        let provider = thread["modelProvider"]
            .as_str()
            .filter(|provider| !provider.is_empty())
            .ok_or_else(|| Error::new("Codex 未返回会话当前 provider"))?;
        let model = thread["model"]
            .as_str()
            .filter(|model| !model.is_empty())
            .ok_or_else(|| Error::new("Codex 未返回会话当前模型"))?;
        Ok((model.to_owned(), provider.to_owned()))
    })
}

/// Lists source-provider Codex models using a provider-scoped app-server
/// connection. The returned catalog cannot inherit the machine's default
/// provider when the source thread uses another account/provider.
pub fn codex_models_for_provider(
    config: &crate::history::HistoryConfig,
    provider: &str,
) -> Result<Retained<Vec<ModelOption>>> {
    if provider.trim().is_empty() {
        return Err(Error::with_code("Codex provider is empty", "invalid_input"));
    }
    #[derive(Deserialize)]
    struct Tier {
        id: String,
        name: String,
    }
    let mut rpc = codex::history_client_at_with_provider(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
        Some(provider),
    )?;
    pages(
        &mut rpc,
        "model/list",
        json!({"limit":100,"includeHidden":false}),
    )?
    .try_map(|models| {
        let mut options = Vec::new();
        for model in &models {
            let id = model["model"]
                .as_str()
                .ok_or_else(|| Error::new("Codex 模型缺少身份"))?;
            let name = model["displayName"].as_str().unwrap_or(id).to_owned();
            let efforts = model["supportedReasoningEfforts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|effort| effort["reasoningEffort"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            options.push(ModelOption {
                model: RuntimeModel {
                    provider_id: Some(provider.to_owned()),
                    id: id.to_owned(),
                    name: name.clone(),
                },
                reasoning_efforts: efforts.clone(),
            });
            let tiers = model
                .get("serviceTiers")
                .cloned()
                .map(serde_json::from_value::<Vec<Tier>>)
                .transpose()
                .map_err(|error| Error::new(format!("Codex 速度档位目录格式无效：{error}")))?
                .unwrap_or_default();
            options.extend(
                tiers
                    .into_iter()
                    .filter(|tier| !tier.id.is_empty())
                    .map(|tier| ModelOption {
                        model: RuntimeModel {
                            provider_id: Some(provider.to_owned()),
                            id: velune_conversation::model_variant::codex_speed_model(id, &tier.id),
                            name: format!("{name}（{}）", tier.name),
                        },
                        reasoning_efforts: efforts.clone(),
                    }),
            );
        }
        Ok(options)
    })
}

/// Result of crash recovery for a portable-summary fork. A missing or changed
/// ownership proof always retains the candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandoffSummaryRecovery {
    Archived,
    NotFound,
    RetainedChanged,
    RetainedUnverified,
}

/// Rechecks the source snapshot immediately before a prepared handoff is
/// committed. It returns false if identity, model/provider, latest idle turn or raw
/// rollout fingerprint changed since summary preparation.
pub fn verify_codex_handoff_summary_source(
    config: &crate::history::HistoryConfig,
    source_native_id: &str,
    source_provider_id: &str,
    source_model_id: &str,
    source_turn_id: &str,
    expected_fingerprint: &str,
) -> Result<bool> {
    if config.kind != crate::version::RuntimeKind::Codex166 || config.provider != "codex" {
        return Err(Error::with_code(
            "handoff summary verification is only supported for Codex",
            "unsupported",
        ));
    }
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let metadata = read_metadata(&mut rpc, source_native_id)?;
    let Some(path) = metadata["thread"]["path"].as_str() else {
        return Ok(false);
    };
    let source_model_base = velune_conversation::model_variant::split_codex_speed(source_model_id)
        .map(|(model, _)| model)
        .unwrap_or_else(|| source_model_id.to_owned());
    if metadata["thread"]["model"].as_str() != Some(source_model_base.as_str())
        || metadata["thread"]["modelProvider"].as_str() != Some(source_provider_id)
    {
        return Ok(false);
    }
    let turns = rpc.request(
        "thread/turns/list",
        &json!({"threadId":source_native_id,"limit":1,"sortDirection":"desc","itemsView":"notLoaded"}),
    )?;
    let Some(turn) = turns["data"].as_array().and_then(|turns| turns.first()) else {
        return Ok(false);
    };
    let turn_id = turn["id"].as_str().or_else(|| turn["turnId"].as_str());
    if turn_id != Some(source_turn_id) || turn["status"].as_str() == Some("inProgress") {
        return Ok(false);
    }
    let fingerprint = config
        .execution
        .read_file(Path::new(path))
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))?;
    Ok(fingerprint == expected_fingerprint)
}

/// Recovers a marked persistent summary fork after its worker was lost.
/// Archival requires an exact marker and source lineage, a caller-persisted
/// rollout fingerprint, and no child threads. Call from a history worker.
pub fn recover_codex_handoff_summary(
    config: &crate::history::HistoryConfig,
    source_native_id: &str,
    work_id: &str,
    expected_temporary_thread_id: Option<&str>,
    expected_temporary_fingerprint: Option<&str>,
) -> Result<HandoffSummaryRecovery> {
    if config.kind != crate::version::RuntimeKind::Codex166 || config.provider != "codex" {
        return Err(Error::with_code(
            "handoff summary recovery is only supported for Codex",
            "unsupported",
        ));
    }
    let marker = format!("velune_handoff_summary:{work_id}");
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let mut candidate_pages = Vec::new();
    let mut candidate_ids = HashSet::new();
    for archived in [false, true] {
        let threads = pages(
            &mut rpc,
            "thread/list",
            json!({
                "limit":100,"archived":archived,"modelProviders":[],
                "sourceKinds":SOURCE_KINDS,"sortKey":"created_at","sortDirection":"desc"
            }),
        )?;
        candidate_pages.push(threads.map(|threads| {
            threads
                .into_iter()
                .filter(|thread| {
                    let id = thread["id"].as_str();
                    thread["threadSource"].as_str() == Some(marker.as_str())
                        && (thread["forkedFromThreadId"].as_str() == Some(source_native_id)
                            || thread["forkedFromId"].as_str() == Some(source_native_id)
                            || thread["forkedFrom"].as_str() == Some(source_native_id))
                        && id.is_some_and(|id| candidate_ids.insert(id.to_owned()))
                })
                .collect::<Vec<_>>()
        }));
    }
    let candidates: Vec<&Value> = candidate_pages
        .iter()
        .flat_map(|page| page.iter())
        .collect();
    if candidates.is_empty() {
        return Ok(HandoffSummaryRecovery::NotFound);
    }
    if candidates.len() != 1 {
        return Ok(HandoffSummaryRecovery::RetainedUnverified);
    }
    let Some(id) = candidates[0]["id"].as_str() else {
        return Ok(HandoffSummaryRecovery::RetainedUnverified);
    };
    if expected_temporary_thread_id.is_some_and(|expected| expected != id) {
        return Ok(HandoffSummaryRecovery::RetainedChanged);
    }
    let Some(expected_fingerprint) = expected_temporary_fingerprint else {
        return Ok(HandoffSummaryRecovery::RetainedUnverified);
    };
    let metadata = read_metadata(&mut rpc, id)?;
    let thread = &metadata["thread"];
    if thread["threadSource"].as_str() != Some(marker.as_str())
        || !(thread["forkedFromThreadId"].as_str() == Some(source_native_id)
            || thread["forkedFromId"].as_str() == Some(source_native_id)
            || thread["forkedFrom"].as_str() == Some(source_native_id))
    {
        return Ok(HandoffSummaryRecovery::RetainedChanged);
    }
    let Some(path) = thread["path"].as_str() else {
        return Ok(HandoffSummaryRecovery::RetainedUnverified);
    };
    let digest = config
        .execution
        .read_file(Path::new(path))
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))?;
    if digest != expected_fingerprint {
        return Ok(HandoffSummaryRecovery::RetainedChanged);
    }
    for archived in [false, true] {
        let threads = pages(
            &mut rpc,
            "thread/list",
            json!({
                "limit":100,"archived":archived,"modelProviders":[],
                "sourceKinds":SOURCE_KINDS,"sortKey":"created_at","sortDirection":"desc"
            }),
        )?;
        if threads.iter().any(|child| {
            child["forkedFromThreadId"].as_str() == Some(id)
                || child["forkedFromId"].as_str() == Some(id)
                || child["parentThreadId"].as_str() == Some(id)
        }) {
            return Ok(HandoffSummaryRecovery::RetainedUnverified);
        }
    }
    rpc.request("thread/archive", &json!({"threadId":id}))?;
    rpc.shutdown();
    Ok(HandoffSummaryRecovery::Archived)
}

fn export_codex_context_at(
    config: &crate::history::HistoryConfig,
    id: &str,
    message_ids: &[String],
    omit_only: bool,
    portable_summary: Option<&str>,
) -> Result<Retained<crate::import::Context>> {
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let metadata = rpc.request("thread/read", &json!({"threadId":id,"includeTurns":false}))?;
    let thread = &metadata["thread"];
    if thread["id"].as_str() != Some(id) {
        return Err(Error::new("Codex 返回不同的原生会话"));
    }
    let path = thread["path"]
        .as_str()
        .ok_or_else(|| Error::new("Codex 未提供上下文来源路径"))?;
    let bytes = config.execution.read_file(Path::new(path))?;
    let mut legacy_retention = None;
    let mut legacy_boundaries = serde_json::Map::new();
    let mut legacy_turns = serde_json::Map::new();
    if thread["historyMode"] == "legacy" {
        let read = rpc.request("thread/read", &json!({"threadId":id,"includeTurns":true}))?;
        let (legacy, retention) = read
            .try_map(|mut read| legacy_history(&mut read["thread"], id))?
            .into_parts();
        legacy_retention = Some(retention);
        let (items, _) = legacy;
        for entry in &items {
            if entry["item"]["type"] == "userMessage"
                && let Some(turn) = entry["turnId"].as_str()
            {
                legacy_turns
                    .entry(turn.to_owned())
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .expect("legacy users")
                    .push(entry["item"].clone());
            }
        }
        for message_id in message_ids {
            let selected = items
                .iter()
                .find(|entry| {
                    entry["item"]["id"] == message_id.as_str()
                        && entry["item"]["type"] == "userMessage"
                })
                .ok_or_else(|| Error::new("Codex 原生用户消息已改变"))?;
            let turn = selected["turnId"]
                .as_str()
                .ok_or_else(|| Error::new("Codex 用户消息缺少所属轮次"))?;
            let users: Vec<&Value> = items
                .iter()
                .filter(|entry| entry["turnId"] == turn && entry["item"]["type"] == "userMessage")
                .map(|entry| &entry["item"])
                .collect();
            let ordinal = users
                .iter()
                .position(|item| item["id"] == message_id.as_str())
                .ok_or_else(|| Error::new("Codex 用户消息未出现在所属轮次"))?;
            legacy_boundaries.insert(
                message_id.clone(),
                json!({"turnId":turn,"ordinal":ordinal,"users":users}),
            );
        }
    }
    let mut command = config.execution.python();
    command
        .args(["-c", include_str!("../../resources/codex_context.py")])
        .arg("--source-path")
        .arg(path)
        .arg("--home")
        .arg(&config.home);
    if omit_only {
        command.arg("--without-message-ids").arg(
            serde_json::to_string(message_ids).map_err(|error| Error::new(error.to_string()))?,
        );
    } else if let Some(message_id) = message_ids.first() {
        command.arg("--before-message-id").arg(message_id);
    }
    let bytes = if !legacy_turns.is_empty() || portable_summary.is_some() {
        command.arg("--request-envelope");
        serde_json::to_vec(&json!({
            "rollout":String::from_utf8(bytes).map_err(|error| Error::new(error.to_string()))?,
            "legacyBoundaries":legacy_boundaries,
            "legacyTurns":legacy_turns,
            "portableSummary":portable_summary,
        }))
        .map_err(|error| Error::new(error.to_string()))?
    } else {
        bytes
    };
    let output = config
        .execution
        .output(command, Some(bytes), config.os_home.as_deref())?;
    // The exporter emits a one-field control envelope for an unavailable
    // portable representation. Regular history starts with `messages`.
    if output.starts_with(b"{\"errorCode\"")
        && let Ok(response) = serde_json::from_slice::<Value>(&output)
        && response["errorCode"].as_str() == Some("portable_summary_required")
    {
        return Err(Error::with_code(
            "Codex encrypted history requires an explicit portable summary before handoff",
            "portable_summary_required",
        ));
    }
    let context = if !omit_only && !message_ids.is_empty() {
        crate::import::Context::parse_before(&output)?
    } else {
        crate::import::Context::parse(&output)?
    };
    let (_, mut retention) = metadata.into_parts();
    if let Some(legacy) = legacy_retention {
        retention = retention.merge(legacy);
    }
    Ok(Retained::from_parts(context, retention))
}

pub fn read_codex_history_at(
    kind: crate::version::RuntimeKind,
    binary: &Path,
    home: &Path,
    runtime_id: &str,
    id: &str,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<Retained<History>> {
    let mut rpc = codex::history_client_at(kind, binary, home, execution, os_home)?;
    let metadata = read_metadata(&mut rpc, id)?;
    let history = read_history(&mut rpc, runtime_id, id, execution, &metadata)?;
    Ok(metadata.zip(history).map(|(_, history)| history))
}

pub(crate) fn read_codex_with_summary_at(
    config: &crate::history::HistoryConfig,
    runtime_id: &str,
    id: &str,
) -> Result<Retained<(ConversationSummary, History)>> {
    let mut rpc = codex::history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let metadata = read_metadata(&mut rpc, id)?;
    let mut summary = summary(&metadata["thread"], runtime_id)?;
    let history = read_history(&mut rpc, runtime_id, id, &config.execution, &metadata)?;
    Ok(metadata.zip(history).map(|(_, history)| {
        summary.native_read_state = history.native_read_state.clone();
        (summary, history)
    }))
}

pub(super) fn read_metadata(rpc: &mut RpcClient, id: &str) -> Result<Retained<Value>> {
    let metadata = rpc.request("thread/read", &json!({"threadId":id,"includeTurns":false}))?;
    if metadata["thread"]["id"].as_str() != Some(id) {
        return Err(Error::new("Codex 返回不同的原生会话"));
    }
    Ok(metadata)
}

/// The borrowed metadata contributes no ownership here. Callers must zip its
/// Retained owner with this result before persisting any projected History.
pub(super) fn read_history(
    rpc: &mut RpcClient,
    runtime_id: &str,
    id: &str,
    execution: &crate::execution::ExecutionTarget,
    metadata: &Value,
) -> Result<Retained<History>> {
    let thread = &metadata["thread"];
    let history_mode = thread["historyMode"]
        .as_str()
        .filter(|mode| matches!(*mode, "legacy" | "paginated"))
        .ok_or_else(|| Error::new("Codex 返回未知的历史契约"))?
        .to_owned();
    let title = thread_title(thread);
    let body = if history_mode == "legacy" {
        // Legacy rollouts explicitly do not implement items/list. thread/read
        // owns their logical turns (including native fork/revert resolution).
        let read = rpc.request("thread/read", &json!({"threadId":id,"includeTurns":true}))?;
        read.try_map(|mut read| legacy_history(&mut read["thread"], id))?
    } else {
        pages(
            rpc,
            "thread/items/list",
            json!({"threadId":id,"limit":50,"sortDirection":"asc"}),
        )?
        .zip(pages(
            rpc,
            "thread/turns/list",
            json!({"threadId":id,"limit":50,"sortDirection":"asc","itemsView":"notLoaded"}),
        )?)
    };
    body.try_map(|(mut items, native_turns)| {
    let thread = &metadata["thread"];
    // Native agent communications are not exposed as ThreadItems. The runtime
    // resolves the source file and valid turns; those boundaries stay authoritative.
    if (thread["parentThreadId"].is_string()
        || thread["source"]["subAgent"]["thread_spawn"]["parent_thread_id"].is_string())
        && let Some(path) = thread["path"].as_str()
        && let Ok(bytes) = execution.read_file(Path::new(path))
    {
        super::codex_messages::supplement(&mut items, &native_turns, &bytes);
    }
    // The rollout is a file on the executing machine; only a local one is read.
    let turns = match (execution, thread["path"].as_str()) {
        (crate::execution::ExecutionTarget::Local, Some(path)) => rollout_turns(Path::new(path)),
        _ => HashMap::new(),
    };
    let (messages, executions) = project_history(&items, &native_turns, &turns, runtime_id)?;
    tracing::info!(target:"velune_agent_runtime",event="history_native_resolution",phase="read",runtime_family="codex",native_items=items.len(),projected_messages=messages.len(),history_mode);
    Ok(History {
        native_id: id.into(),
        title,
        cwd: thread["cwd"].as_str().map(str::to_owned),
        messages,
        pending_interactions: super::codex_questions::pending(&items),
        executions,
        native_read_state: read_state(metadata.get("readState"), runtime_id, id)?,
        can_resume: native_turns
            .last()
            .is_some_and(|turn| matches!(turn["status"].as_str(), Some("interrupted" | "failed"))),
    })
    })
}

fn legacy_history(thread: &mut Value, id: &str) -> Result<(Vec<Value>, Vec<Value>)> {
    if thread["id"].as_str() != Some(id) || thread["historyMode"] != "legacy" {
        return Err(Error::new("Codex 历史契约在读取期间改变，请重新打开"));
    }
    let Value::Array(mut turns) = thread["turns"].take() else {
        return Err(Error::new("Codex 未返回 legacy 轮次"));
    };
    let mut items = Vec::new();
    for turn in &mut turns {
        let turn_id = turn["id"]
            .as_str()
            .ok_or_else(|| Error::new("Codex legacy 轮次缺少身份"))?
            .to_owned();
        let started = turn["startedAt"]
            .as_i64()
            .and_then(|at| at.checked_mul(1000));
        let completed = turn["completedAt"]
            .as_i64()
            .and_then(|at| at.checked_mul(1000));
        let Value::Array(loaded) = turn["items"].take() else {
            return Err(Error::new("Codex 未载入 legacy 轮次条目"));
        };
        for item in loaded {
            items.push(json!({"turnId":turn_id,"item":item,"startedAtMs":started,"completedAtMs":completed}));
        }
    }
    Ok((items, turns))
}

fn project_history(
    items: &[Value],
    native_turns: &[Value],
    turns: &HashMap<String, RolloutTurn>,
    runtime_id: &str,
) -> Result<(Vec<Message>, Executions)> {
    #[derive(Default)]
    struct ProjectedTurn {
        end: usize,
        maintenance: bool,
        last_user: Option<usize>,
        items: Vec<String>,
        error: Option<String>,
    }
    // Associate items once rather than scanning the entire transcript per turn.
    let mut projected = HashMap::new();
    for (index, entry) in items.iter().enumerate() {
        if let Some(id) = entry["turnId"].as_str() {
            let turn = projected.entry(id).or_insert_with(|| ProjectedTurn {
                maintenance: true,
                ..Default::default()
            });
            turn.end = index + 1;
            turn.maintenance &= entry["item"]["type"] == "contextCompaction";
        }
    }
    let mut errors: HashMap<usize, Vec<Message>> = HashMap::new();
    let mut end = 0;
    for turn in native_turns {
        let id = turn["id"].as_str();
        if let Some(position) = id.and_then(|id| projected.get(id)) {
            end = position.end;
        }
        if let Some(message) = codex::project_turn_error(turn)? {
            // A failed turn can have no items; keep its place between adjacent turns.
            if let Some(id) = id {
                projected.entry(id).or_default().error = Some(message.id.clone());
            }
            errors.entry(end).or_default().push(message);
        }
    }
    let mut messages = Vec::new();
    for (index, entry) in items.iter().enumerate() {
        messages.extend(errors.remove(&index).unwrap_or_default());
        if let Some(mut message) = codex::project_item(&entry["item"], runtime_id) {
            message.timestamp_unix_ms = entry["startedAtMs"]
                .as_i64()
                .or_else(|| entry["completedAtMs"].as_i64());
            let id = entry["turnId"].as_str();
            if let Some(turn) = id.and_then(|id| projected.get_mut(id)) {
                turn.items.push(message.id.clone());
                if message.role == MessageRole::User {
                    turn.last_user = Some(messages.len());
                }
            }
            if message.role == MessageRole::User {
                message.branch_point = id.map(str::to_owned);
                let turn = id.and_then(|id| turns.get(id));
                message.turn_settings = Some(turn.map(|t| t.settings.clone()).unwrap_or_default());
                message.turn_usage = turn.and_then(|t| t.usage.clone());
            }
            messages.push(message);
        }
    }
    messages.extend(errors.remove(&items.len()).unwrap_or_default());
    let mut last_user = None;
    let mut last_user_execution: Option<usize> = None;
    let mut executions = Executions::default();
    for turn in native_turns {
        let Some(id) = turn["id"].as_str() else {
            continue;
        };
        let projection = projected.remove(id).unwrap_or_default();
        let mut ids = projection.items;
        ids.extend(projection.error);
        let kind = if projection.maintenance {
            ExecutionKind::ContextMaintenance
        } else {
            ExecutionKind::UserTurn
        };
        let outcome = codex::turn_outcome(turn["status"].as_str());
        if let Some(index) = projection.last_user {
            last_user = Some(index);
        } else if kind == ExecutionKind::ContextMaintenance {
            if let Some(index) = last_user
                && let Some(usage) = turns.get(id).and_then(|turn| turn.usage.as_ref())
            {
                let message: &mut Message = &mut messages[index];
                let total = message.turn_usage.get_or_insert_with(TurnUsage::default);
                total.context_tokens = usage.context_tokens.or(total.context_tokens);
                total.context_window = usage.context_window.or(total.context_window);
            }
        } else if kind == ExecutionKind::UserTurn {
            // Input-free turns belong to the preceding user message's visible
            // segment, and keep its original requested settings.
            if let Some(index) = last_user
                && let Some(usage) = turns.get(id).and_then(|turn| turn.usage.as_ref())
            {
                let message: &mut Message = &mut messages[index];
                match &mut message.turn_usage {
                    Some(total) => total.add(usage),
                    total => *total = Some(usage.clone()),
                }
            }
            if let Some(index) = last_user_execution {
                let record = &mut executions.records[index];
                record.message_ids.extend(ids);
                record.outcome = outcome;
                continue;
            }
        }
        if kind == ExecutionKind::UserTurn {
            last_user_execution = Some(executions.records.len());
        }
        executions.history(id.into(), kind, ids, outcome);
    }
    Ok((messages, executions))
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
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    rollout_turns_from_bytes(&bytes)
}

fn rollout_turns_from_bytes(bytes: &[u8]) -> HashMap<String, RolloutTurn> {
    let mut turns: HashMap<String, RolloutTurn> = HashMap::new();
    // A `token_count` event names no turn; it belongs to the record before it.
    let mut counted: Option<String> = None;
    let records = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| {
            [
                "\"turn_context\"",
                "\"token_usage_record\"",
                "\"token_count\"",
            ]
            .iter()
            .any(|kind| {
                line.windows(kind.len())
                    .any(|window| window == kind.as_bytes())
            })
        })
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok());
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
                // A tier the turn ran with belongs to the model it names:
                // the variant's selection id, so rereads keep the speed.
                let model =
                    text(&payload["model"]).map(|model| match text(&payload["service_tier"]) {
                        Some(tier) => {
                            velune_conversation::model_variant::codex_speed_model(&model, &tier)
                        }
                        None => model,
                    });
                entry.settings = TurnSettings {
                    runtime_id: None,
                    model,
                    effort: text(&payload["effort"]).or_else(|| {
                        text(&payload["collaboration_mode"]["settings"]["reasoning_effort"])
                    }),
                    permission_mode: permission_mode(payload),
                    model_selection: None,
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

/// Validate a native receipt once, at the protocol boundary. Older servers
/// and ineligible threads expose no receipt, preserving the local fallback.
pub(super) fn read_state(
    value: Option<&Value>,
    runtime: &str,
    id: &str,
) -> Result<Option<NativeReadState>> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let revision = value["revision"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new("Codex 已读状态缺少版本"))?;
    let first = value
        .get("firstUnread")
        .ok_or_else(|| Error::new("Codex 已读状态缺少位置"))?;
    if !first.is_null() && !matches!(first["type"].as_str(), Some("threadStart" | "turn")) {
        return Err(Error::new("Codex 已读状态位置无效"));
    }
    Ok(Some(NativeReadState {
        conversation_id: format!("{runtime}:{id}"),
        revision: revision.into(),
        unread: !first.is_null(),
    }))
}

#[cfg(test)]
mod tests;
