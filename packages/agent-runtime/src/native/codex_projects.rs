//! Codex's project identities and ordered roots, independent of thread cwd.
use super::{
    Error, Result, codex,
    codex_history::{SOURCE_KINDS, pages, summary},
};
use crate::{conversation::ConversationSummary, history::HistoryConfig};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct NativeProject {
    pub id: String,
    pub name: String,
    pub roots: Vec<ProjectRoot>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ProjectRoot {
    pub path: String,
}
pub struct ProjectListing {
    pub projects: Vec<NativeProject>,
    pub assignments: BTreeMap<String, String>,
    pub conversations: Vec<ConversationSummary>,
}

pub fn list_projects(
    config: &HistoryConfig,
    runtime_id: &str,
    archived: bool,
) -> Result<ProjectListing> {
    let mut rpc = codex::history_client_at(
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let projects = pages(&mut rpc, "project/list", json!({"limit":100}))?
        .into_iter()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|e| Error::new(format!("Codex 项目格式无效：{e}")))
        })
        .collect::<Result<Vec<NativeProject>>>()?;
    let mut project_ids = BTreeSet::new();
    if projects
        .iter()
        .any(|project| project.id.is_empty() || !project_ids.insert(project.id.as_str()))
    {
        return Err(Error::new("Codex 返回无效或重复的项目身份"));
    }
    let threads = pages(
        &mut rpc,
        "thread/list",
        json!({"limit":100,"archived":archived,"modelProviders":[],"sourceKinds":SOURCE_KINDS,"sortKey":"updated_at","sortDirection":"desc"}),
    )?;
    let mut assignments = BTreeMap::new();
    let mut conversations = Vec::new();
    let mut seen = BTreeSet::new();
    for thread in threads {
        let conversation = summary(&thread, runtime_id)?;
        if !seen.insert(conversation.id.clone()) {
            return Err(Error::new("Codex 返回重复逻辑会话"));
        }
        if let Some(id) = thread["projectId"].as_str() {
            if !project_ids.contains(id) {
                return Err(Error::new("Codex 会话引用了未返回的项目；请刷新重试"));
            }
            assignments.insert(conversation.id.clone(), id.to_owned());
        }
        conversations.push(conversation);
    }
    Ok(ProjectListing {
        projects,
        assignments,
        conversations,
    })
}

/// Update only the native name and roots; other clients' project metadata stays owned by Codex.
pub fn update_project(
    config: &HistoryConfig,
    project_id: &str,
    name: &str,
    roots: &[String],
) -> Result<NativeProject> {
    let mut rpc = codex::history_client_at(
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let updated = rpc.request("project/update", &json!({"projectId":project_id,"name":name,"roots":roots.iter().map(|path| json!({"path":path})).collect::<Vec<_>>()}))?;
    serde_json::from_value(updated["project"].clone()).map_err(|e| Error::new(e.to_string()))
}
