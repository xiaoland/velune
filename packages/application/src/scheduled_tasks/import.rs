//! Read only selected task definitions. No authentication or transcript import.
use super::*;
use crate::{ScheduledTarget, ScheduledTaskSource};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Deserialize, serde::Serialize)]
struct Source {
    version: Option<u64>,
    id: String,
    name: String,
    prompt: String,
    status: String,
    kind: Option<String>,
    rrule: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    cwds: Option<Vec<String>>,
    target_thread_id: Option<String>,
    execution_environment: Option<String>,
    notification_policy: Option<String>,
    local_environment_config_path: Option<String>,
    target: Option<serde_json::Value>,
    auto_archive: Option<bool>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HostState {
    database: String,
    status: String,
    next_run_at: Option<i64>,
    in_flight: bool,
}
/// Known desktop stores, opened read-only and queried only for the selected ID.
/// A portable task export has no database; it still requires explicit handover.
fn host_state(path: &Path, id: &str) -> Result<Option<HostState>, Error> {
    use rusqlite::{OpenFlags, OptionalExtension};
    let Some(home) = path.parent().and_then(Path::parent).and_then(Path::parent) else {
        return Ok(None);
    };
    let mut found = None;
    for name in ["codex.db", "codex-dev.db"] {
        let database = home.join("sqlite").join(name);
        if !database.is_file() {
            continue;
        }
        let connection = Connection::open_with_flags(
            &database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| Error::context("来源任务状态无法读取", e))?;
        connection
            .busy_timeout(std::time::Duration::ZERO)
            .map_err(db)?;
        let row = connection
            .query_row(
                "SELECT status,next_run_at FROM automations WHERE id=?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()
            .map_err(|e| Error::context("来源任务数据库版本或状态不可读取", e))?;
        let Some((status, next_run_at)) = row else {
            continue;
        };
        if found.is_some() {
            return Err(Error::invalid(
                "两个来源数据库包含同一任务，请明确导出来源后再导入",
            ));
        }
        let in_flight:bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM automation_runs WHERE automation_id=?1 AND status='IN_PROGRESS')",[id],|row|row.get(0)).map_err(|e| Error::context("来源在途运行无法确认",e))?;
        found = Some(HostState {
            database: database.to_string_lossy().into(),
            status,
            next_run_at,
            in_flight,
        });
    }
    Ok(found)
}
fn read(path: &Path) -> Result<(Source, String), Error> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 {
        return Err(Error::invalid(
            "来源必须是小于 512 KiB 的普通 automation.toml 文件",
        ));
    }
    let text = std::fs::read_to_string(path)?;
    let source: Source =
        toml_edit::de::from_str(&text).map_err(|e| Error::context("任务定义无法解析", e))?;
    if source.version.is_some_and(|v| v != 1) {
        return Err(Error::invalid("暂不支持此来源任务版本"));
    }
    let mut value = serde_json::to_value(&source)?;
    value
        .as_object_mut()
        .expect("source record")
        .remove("status");
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?));
    Ok((source, fingerprint))
}
pub(crate) fn verify_source(task: &ScheduledTask) -> Result<(), Error> {
    let Some(provenance) = &task.source else {
        return Ok(());
    };
    let (source, fingerprint) = read(Path::new(&provenance.path))?;
    if fingerprint != provenance.fingerprint {
        return Err(Error::invalid("来源任务已变更，请重新预览导入并确认差异"));
    }
    if !matches!(source.status.as_str(), "PAUSED" | "DISABLED" | "DELETED") {
        return Err(Error::invalid(
            "原宿主任务仍为启用状态；请先在 ChatGPT 中暂停，并确认在途运行已处理",
        ));
    }
    if let Some(host) = host_state(Path::new(&provenance.path), &source.id)? {
        if !matches!(host.status.as_str(), "PAUSED" | "DISABLED" | "DELETED") {
            return Err(Error::invalid(
                "原宿主数据库仍在启用此任务，请在原应用暂停后再确认",
            ));
        }
        if host.in_flight {
            return Err(Error::invalid(
                "原宿主还有在途运行，请先在原应用处理，再确认交接",
            ));
        }
    }
    Ok(())
}
pub(crate) fn import_one(
    path: &Path,
    runtime_id: &str,
    zone: &str,
) -> Result<ScheduledTask, Error> {
    if !path.is_absolute()
        || path
            .file_name()
            .is_none_or(|name| name != "automation.toml")
    {
        return Err(Error::invalid("请选择绝对路径的 automation.toml"));
    }
    let (source, fingerprint) = read(path)?;
    let kind = source.kind.as_deref().unwrap_or("cron");
    let target = match kind {
        "cron" => ScheduledTarget::NewConversation {
            runtime_id: runtime_id.into(),
            working_directories: source.cwds.clone().unwrap_or_default(),
            worktree: source
                .execution_environment
                .as_deref()
                .unwrap_or("worktree")
                == "worktree",
        },
        "heartbeat" => {
            let id = source
                .target_thread_id
                .as_ref()
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| Error::invalid("heartbeat 缺少目标会话"))?;
            ScheduledTarget::ExistingConversation {
                runtime_id: runtime_id.into(),
                conversation_id: format!("{runtime_id}:{id}"),
            }
        }
        _ => return Err(Error::invalid("暂不支持此任务类型")),
    };
    if source
        .execution_environment
        .as_deref()
        .is_some_and(|e| e != "local" && e != "worktree")
    {
        return Err(Error::invalid("来源执行环境未知，不能自动转成工作目录执行"));
    }
    let host = host_state(path, &source.id)?;
    let metadata = serde_json::to_string(&serde_json::json!({"definition":source,"host":host}))?;
    let task = ScheduledTask { id:String::new(), revision:0, name:source.name, prompt:source.prompt,
        rrule: source.rrule.unwrap_or_else(|| "FREQ=HOURLY;INTERVAL=24;BYMINUTE=0".into()), time_zone:zone.into(),
        status:ScheduledTaskStatus::Paused, target, model:None, effort:source.reasoning_effort, permission_mode:None,
        notify_only_failures:source.notification_policy.as_deref() == Some("failed_runs_only"), next_run_at_unix_ms:None,
        source:Some(ScheduledTaskSource { path:path.canonicalize()?.to_string_lossy().into(), fingerprint, source_id:source.id, original_model:source.model,
            original_status:source.status, original_metadata:metadata, handover_confirmed:false }),
        issue:Some(if source.local_environment_config_path.is_some() { "已暂停导入；来源的本地环境配置需要在 Velune 工作目录中另行配置，再确认运行时、模型和宿主交接" } else { "已暂停导入；请核对时区、运行时、模型和目录，并在原宿主暂停后确认交接" }.into()),
    };
    next_occurrence(&task, now())?;
    Ok(task)
}
pub(crate) fn preview_import(
    directory: &Path,
    runtime_id: &str,
    zone: &str,
) -> Result<(Vec<ScheduledTask>, Vec<String>), Error> {
    if !directory.is_absolute() {
        return Err(Error::invalid(
            "请选择绝对路径的 Codex HOME 或 automations 目录",
        ));
    }
    let root: PathBuf = if directory.file_name().is_some_and(|n| n == "automations") {
        directory.into()
    } else {
        directory.join("automations")
    };
    let mut tasks = Vec::new();
    let mut issues = Vec::new();
    for entry in std::fs::read_dir(root)?.take(257) {
        if tasks.len() + issues.len() >= 256 {
            issues.push("最多预览 256 项，请缩小来源范围".into());
            break;
        }
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path().join("automation.toml");
        match import_one(&path, runtime_id, zone) {
            Ok(task) => tasks.push(task),
            Err(e) => issues.push(format!("{}：{e}", entry.file_name().to_string_lossy())),
        }
    }
    Ok((tasks, issues))
}
