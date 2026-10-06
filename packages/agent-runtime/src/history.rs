//! Read-only historical projection through the installed huihua package.
use crate::{
    conversation::*,
    native::{Error, Result, rpc::bounded_process},
};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, process::Command};
pub struct HistoryConfig {
    pub provider: String,
    pub node_binary: PathBuf,
    pub resources_directory: PathBuf,
    pub root: PathBuf,
    pub home: PathBuf,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub native_id: String,
    pub title: ConversationTitle,
    pub cwd: Option<String>,
    pub messages: Vec<Message>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    native_id: String,
    title: ConversationTitle,
    cwd: Option<String>,
    updated_at_unix_ms: Option<i64>,
    created_at_unix_ms: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct List {
    contract_version: u32,
    sessions: Vec<Summary>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Read {
    contract_version: u32,
    history: History,
}
fn execute(config: &HistoryConfig, operation: &str, native_id: Option<&str>) -> Result<Vec<u8>> {
    if !matches!(config.provider.as_str(), "codex" | "deepseek") {
        return Err(Error::new("会话历史来源不受支持"));
    }
    for path in [
        &config.node_binary,
        &config.resources_directory,
        &config.root,
        &config.home,
    ] {
        if !path.is_absolute() {
            return Err(Error::new("会话历史读取路径必须为绝对路径"));
        }
    }
    let request = json!({"operation":operation,"provider":config.provider,"home":config.home,"roots":[config.root],"nativeId":native_id});
    let input = serde_json::to_vec(&request).map_err(|_| Error::new("历史读取请求编码失败"))?;
    let mut command = Command::new(&config.node_binary);
    command.arg(config.resources_directory.join("huihua_sessions.mjs"));
    bounded_process(command, Some(input), 16 * 1024 * 1024)
}
pub fn list(config: &HistoryConfig, runtime_id: &str) -> Result<Vec<ConversationSummary>> {
    let result: List = serde_json::from_slice(&execute(config, "list", None)?)
        .map_err(|_| Error::new("会话历史列表契约不匹配"))?;
    if result.contract_version != 1 {
        return Err(Error::new("会话历史桥版本不匹配"));
    }
    Ok(result
        .sessions
        .into_iter()
        .map(|s| ConversationSummary {
            id: format!("{runtime_id}:{}", s.native_id),
            title: s.title,
            updated_at_unix_ms: s.updated_at_unix_ms,
            created_at_unix_ms: s.created_at_unix_ms,
            runtime_id: runtime_id.into(),
            cwd: s.cwd,
        })
        .collect())
}
pub fn read(config: &HistoryConfig, native_id: &str) -> Result<History> {
    let result: Read = serde_json::from_slice(&execute(config, "read", Some(native_id))?)
        .map_err(|_| Error::new("会话历史详情契约不匹配"))?;
    if result.contract_version != 1 || result.history.native_id != native_id {
        return Err(Error::new("会话历史桥身份或版本不匹配"));
    }
    Ok(result.history)
}

/// Execute the fixed Pi SDK history helper without starting the Agent. Output
/// and wall time share the bounded native helper transport; stderr is discarded.
pub fn read_pi(
    config: &crate::Config,
    session: Option<&std::path::Path>,
) -> Result<serde_json::Value> {
    execute_pi(config, session, None)
}
/// Modify Pi's native source with the fixed SDK; no Velune metadata overlay.
pub fn manage_pi(
    config: &crate::Config,
    session: &std::path::Path,
    title: Option<&str>,
) -> Result<()> {
    let result = execute_pi(config, Some(session), Some(title))?;
    if result["ok"] != true {
        return Err(Error::new("Pi 会话修改未确认"));
    }
    Ok(())
}
fn execute_pi(
    config: &crate::Config,
    session: Option<&std::path::Path>,
    mutation: Option<Option<&str>>,
) -> Result<serde_json::Value> {
    let node = config
        .node_binary
        .as_ref()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::new("Pi 历史 Node 路径无效"))?;
    let helper = config
        .sdk_helper
        .as_ref()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::new("Pi 历史 SDK helper 路径无效"))?;
    let mut command = Command::new(node);
    let agent_dir = config
        .agent_dir
        .as_ref()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::new("Pi 历史来源目录无效"))?;
    command.arg(helper).arg("--agent-dir").arg(agent_dir);
    if let Some(session) = session {
        command
            .arg(match mutation {
                Some(Some(_)) => "--rename-session",
                Some(None) => "--delete-session",
                None => "--inspect-session",
            })
            .arg(session);
    } else {
        command.arg("--all");
    }
    if let Some(root) = &config.session_dir {
        command.arg("--session-dir").arg(root);
    }
    let input = mutation
        .flatten()
        .map(|title| serde_json::to_vec(&json!({"title":title})))
        .transpose()
        .map_err(|_| Error::new("Pi 会话修改请求无效"))?;
    let output = bounded_process(command, input, 16 * 1024 * 1024)?;
    serde_json::from_slice(&output).map_err(|_| Error::new("Pi 历史 SDK 格式不匹配"))
}
