//! One-time, application-owned preflight for known pre-beta JSON formats.
//!
//! The caller holds `runtime.lock`. Every candidate is parsed and checked before
//! any source is changed; all originals are copied to one private recovery point
//! before the first atomic replacement.
use crate::{Error, repository::CURRENT_SCHEMA_VERSION};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Change {
    path: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
}

pub(crate) fn prepare(home: &Path) -> Result<(), Error> {
    detect_pre_beta_transitions(home)?;
    let mut changes = Vec::new();
    migrate_config(home, &mut changes)?;
    migrate_composer(home, &mut changes)?;
    migrate_choices(home, &mut changes)?;
    guard_platform_preferences(home)?;
    if !changes.is_empty() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let backup = home.join(format!("before-beta-migration-{stamp}"));
        fs::create_dir(&backup)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&backup, fs::Permissions::from_mode(0o700))?;
        }
        for change in &changes {
            let target = backup.join(
                change
                    .path
                    .file_name()
                    .ok_or_else(|| Error::invalid("migration backup path"))?,
            );
            write_new_private(&target, &change.original)?;
        }
    }
    migrate_scheduled_schema(home)?;
    for change in &changes {
        atomic_write(&change.path, &change.replacement)?;
    }
    Ok(())
}

fn detect_pre_beta_transitions(home: &Path) -> Result<(), Error> {
    let config = read_transition_json(&home.join("generic-config.json"))?;
    let links = read_transition_json(&home.join("conversation-links.json"))?;
    let mut has_old_codex = config.as_ref().is_some_and(|value| {
        value
            .get("runtimeInstances")
            .and_then(Value::as_array)
            .is_some_and(|instances| {
                instances.iter().any(|runtime| {
                    runtime.get("typeId").and_then(Value::as_str) == Some("codex-0.159")
                })
            })
    });
    has_old_codex |= links.as_ref().is_some_and(|value| {
        value
            .get("links")
            .and_then(Value::as_array)
            .is_some_and(|links| {
                links.iter().any(|link| {
                    link.get("segments")
                        .and_then(Value::as_array)
                        .is_some_and(|segments| {
                            segments.iter().any(|segment| {
                                segment.get("runtimeTypeId").and_then(Value::as_str)
                                    == Some("codex-0.159")
                            })
                        })
                })
            })
    });
    let mut has_service_tier = read_transition_json(&home.join("composer-choices.json"))?
        .as_ref()
        .is_some_and(has_legacy_composer_service_tier);
    has_service_tier |= read_transition_json(&home.join("composer-inputs.json"))?
        .as_ref()
        .is_some_and(has_legacy_input_service_tier);
    has_service_tier |= read_transition_json(&home.join("turn-requests.json"))?
        .as_ref()
        .is_some_and(has_legacy_turn_request_service_tier);
    for filename in ["mac-window-workspaces", "composer-cache"] {
        let directory = home.join(filename);
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "json")
                || entry.file_name().to_string_lossy().contains("backup")
            {
                continue;
            }
            let Some(value) = read_transition_json(&entry.path())? else {
                continue;
            };
            has_service_tier |= if filename == "mac-window-workspaces" {
                has_legacy_workspace_service_tier(&value)
            } else {
                has_legacy_windows_cache_service_tier(&value)
            };
        }
    }
    if has_service_tier {
        return Err(Error::Invalid("检测到旧 composer serviceTier 数据；原文件保留。请在与应用版本一致的源码目录退出 Velune 后运行 `python3 scripts/run-migrations.py --home <VELUNE_HOME>`，再启动 beta。该过渡可能要求已安装的 Codex 0.166 CLI。".into()));
    }
    if has_old_codex {
        return Err(Error::Invalid("检测到 codex-0.159 实例或会话关联；原文件保留。请在与应用版本一致的源码目录退出 Velune 后运行 `python3 scripts/migrations/codex-166.py --home <VELUNE_HOME> --apply`，并按脚本提示指定已安装的 0.166 CLI；随后再启动 beta。".into()));
    }
    if legacy_composer_platform_cache(home)?
        || (!home.join("composer-inputs.json").exists()
            && ["composer-drafts.json", "composer-choices.json"]
                .iter()
                .any(|name| home.join(name).exists()))
    {
        return Err(Error::Invalid("检测到早期 composer 草稿或平台缓存；原文件保留。请在与应用版本一致的源码目录退出 Velune 后运行 `python3 scripts/run-migrations.py --home <VELUNE_HOME>`；Android SharedPreferences 需先从旧包导出 XML，再运行 `python3 scripts/migrations/composer-state.py --home <VELUNE_HOME> --android-preferences <XML绝对路径> --apply`。完成后再启动 beta。".into()));
    }
    Ok(())
}

fn read_transition_json(path: &Path) -> Result<Option<Value>, Error> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Error::context("pre-beta 状态无法解析；原文件保留", e)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn is_legacy_choice(value: &Value) -> bool {
    value.as_object().is_some_and(|choice| {
        (choice.contains_key("serviceTier")
            && choice.keys().all(|key| {
                [
                    "model",
                    "effort",
                    "permissionMode",
                    "serviceTier",
                    "runtimeId",
                    "modelSelection",
                ]
                .contains(&key.as_str())
            }))
            || (choice.contains_key("ServiceTier")
                && choice.keys().all(|key| {
                    ["Model", "Effort", "PermissionMode", "ServiceTier"].contains(&key.as_str())
                }))
    })
}

fn has_legacy_composer_service_tier(value: &Value) -> bool {
    let choices = value.get("choices");
    let runtime_choices = choices
        .and_then(|choices| choices.get("runtimes"))
        .and_then(Value::as_array)
        .is_some_and(|runtimes| {
            runtimes.iter().any(|runtime| {
                runtime.get("lastChosen").is_some_and(is_legacy_choice)
                    || runtime
                        .get("newConversation")
                        .and_then(|policy| policy.get("choice"))
                        .is_some_and(is_legacy_choice)
            })
        });
    let conversation_choices = choices
        .and_then(|choices| choices.get("conversations"))
        .and_then(Value::as_array)
        .is_some_and(|conversations| {
            conversations.iter().any(|conversation| {
                conversation
                    .get("choices")
                    .and_then(Value::as_array)
                    .is_some_and(|choices| {
                        choices
                            .iter()
                            .any(|choice| choice.get("choice").is_some_and(is_legacy_choice))
                    })
            })
        });
    runtime_choices || conversation_choices
}

fn has_legacy_runtime_choice_list(value: &Value) -> bool {
    value.as_array().is_some_and(|choices| {
        choices
            .iter()
            .any(|entry| entry.get("choice").is_some_and(is_legacy_choice))
    })
}

fn has_legacy_input_service_tier(value: &Value) -> bool {
    let input_choices = value
        .pointer("/state/inputs")
        .and_then(Value::as_array)
        .is_some_and(|inputs| {
            inputs.iter().any(|input| {
                input
                    .pointer("/content/choices")
                    .is_some_and(has_legacy_runtime_choice_list)
            })
        });
    let target_choices = value
        .pointer("/state/targets")
        .and_then(Value::as_array)
        .is_some_and(|targets| {
            targets.iter().any(|target| {
                target
                    .pointer("/choices/choices")
                    .is_some_and(has_legacy_runtime_choice_list)
            })
        });
    input_choices || target_choices
}

fn has_legacy_workspace_service_tier(value: &Value) -> bool {
    let workspace = value.get("composerWorkspace");
    let draft_choices = workspace
        .and_then(|workspace| workspace.get("drafts"))
        .and_then(Value::as_array)
        .is_some_and(|drafts| {
            drafts.iter().any(|draft| {
                let state = draft.get("state");
                state
                    .and_then(|state| state.get("choices"))
                    .and_then(Value::as_object)
                    .is_some_and(|choices| choices.values().any(is_legacy_choice))
                    || state
                        .and_then(|state| state.get("foreignChoices"))
                        .and_then(Value::as_array)
                        .is_some_and(|choices| {
                            choices
                                .iter()
                                .any(|choice| choice.get("choice").is_some_and(is_legacy_choice))
                        })
            })
        });
    let target_choices = workspace
        .and_then(|workspace| workspace.get("pendingTargets"))
        .and_then(Value::as_object)
        .is_some_and(|targets| {
            targets.values().any(|target| {
                target
                    .pointer("/choices/choices")
                    .is_some_and(has_legacy_runtime_choice_list)
            })
        });
    draft_choices || target_choices
}

fn has_legacy_windows_cache_service_tier(value: &Value) -> bool {
    value
        .pointer("/Input/Content/Choices")
        .or_else(|| value.pointer("/Input/Content/choices"))
        .and_then(Value::as_object)
        .is_some_and(|choices| choices.values().any(is_legacy_choice))
}

fn has_legacy_turn_request_service_tier(value: &Value) -> bool {
    value
        .pointer("/conversations")
        .and_then(Value::as_array)
        .is_some_and(|conversations| {
            conversations.iter().any(|conversation| {
                conversation
                    .get("turns")
                    .and_then(Value::as_array)
                    .is_some_and(|turns| {
                        turns
                            .iter()
                            .any(|turn| turn.get("settings").is_some_and(is_legacy_choice))
                    })
            })
        })
}

fn legacy_composer_platform_cache(home: &Path) -> Result<bool, Error> {
    let windows = home.join("composer-cache");
    if windows.is_dir() {
        for entry in fs::read_dir(windows)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("backup")
                || entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "json")
            {
                continue;
            }
            let value: Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            if value
                .get("Schema")
                .and_then(Value::as_u64)
                .is_some_and(|schema| schema < 2)
            {
                return Ok(true);
            }
        }
    }
    let mac = home.join("mac-window-workspaces");
    if mac.is_dir() {
        for entry in fs::read_dir(mac)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("backup")
                || entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "json")
            {
                continue;
            }
            let value: Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            if value
                .get("composer")
                .is_some_and(|composer| !composer.is_null())
                || value.get("composerWorkspace").is_some_and(|workspace| {
                    workspace
                        .get("schemaVersion")
                        .and_then(Value::as_u64)
                        .is_some_and(|schema| schema != 3)
                })
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn only_known_json_keys(source: &Value, normalized: &Value) -> bool {
    match (source, normalized) {
        (Value::Object(source), Value::Object(normalized)) => source.iter().all(|(key, value)| {
            normalized
                .get(key)
                .is_some_and(|known| only_known_json_keys(value, known))
        }),
        (Value::Array(source), Value::Array(normalized)) => {
            source.len() == normalized.len()
                && source
                    .iter()
                    .zip(normalized)
                    .all(|(source, normalized)| only_known_json_keys(source, normalized))
        }
        _ => true,
    }
}

fn migrate_scheduled_schema(home: &Path) -> Result<(), Error> {
    use rusqlite::{Connection, backup::Backup};
    let path = home.join("scheduled-tasks.sqlite3");
    if !path.exists() {
        return Ok(());
    }
    let db = Connection::open(&path)
        .map_err(|e| Error::context("定时任务数据库无法打开；原文件保留", e))?;
    let version: u32 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| Error::context("定时任务数据库版本无法读取；原文件保留", e))?;
    let tables: Vec<String> = {
        let mut query = db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .map_err(|e| Error::context("定时任务数据库无法检查；原文件保留", e))?;
        query
            .query_map([], |row| row.get(0))
            .map_err(|e| Error::context("定时任务数据库无法检查；原文件保留", e))?
            .collect::<Result<_, _>>()
            .map_err(|e| Error::context("定时任务数据库无法检查；原文件保留", e))?
    };
    let table_names: Vec<&str> = tables.iter().map(String::as_str).collect();
    if version != 0 && version != 1 {
        return Err(Error::Invalid(format!(
            "定时任务数据库 schema {version} 不受支持；原数据库保留"
        )));
    }
    if tables.is_empty() {
        return if version == 0 {
            Ok(())
        } else {
            Err(Error::invalid(
                "定时任务数据库 schema 1 缺少任务表；原数据库保留",
            ))
        };
    }
    if table_names != ["runs", "tasks"]
        || columns(&db, "tasks")?
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != ["id", "definition"]
        || columns(&db, "runs")?
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != ["id", "task_id", "due", "slot", "phase", "body"]
    {
        return Err(Error::invalid(
            "定时任务数据库表结构不符合已知格式；原数据库保留",
        ));
    }
    validate_scheduled_rows(&db)?;
    if version == 1 {
        return Ok(());
    }
    let backup_path = path.with_file_name("scheduled-tasks.before-beta-migration.sqlite3");
    if backup_path.exists() {
        verify_task_backup(&db, &backup_path)?;
    } else {
        let mut backup_db = Connection::open(&backup_path)
            .map_err(|e| Error::context("定时任务备份无法创建；原数据库保留", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&backup_path, fs::Permissions::from_mode(0o600))?;
        }
        Backup::new(&db, &mut backup_db)
            .and_then(|backup| backup.run_to_completion(5, Duration::from_millis(50), None))
            .map_err(|e| Error::context("定时任务一致性备份失败；原数据库保留", e))?;
    }
    db.execute_batch("BEGIN IMMEDIATE; PRAGMA user_version=1; COMMIT;")
        .map_err(|e| Error::context("定时任务 schema 标记失败；可恢复备份已保留", e))?;
    Ok(())
}

fn validate_scheduled_rows(db: &rusqlite::Connection) -> Result<(), Error> {
    let mut tasks = db
        .prepare("SELECT id, definition FROM tasks ORDER BY id")
        .map_err(|e| Error::context("定时任务定义无法检查；原数据库保留", e))?;
    let task_rows = tasks
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| Error::context("定时任务定义无法检查；原数据库保留", e))?;
    for row in task_rows {
        let (id, definition) =
            row.map_err(|e| Error::context("定时任务定义无法读取；原数据库保留", e))?;
        let task: crate::ScheduledTask = serde_json::from_str(&definition)
            .map_err(|e| Error::context("定时任务定义含未知或无效字段；原数据库保留", e))?;
        if task.id != id {
            return Err(Error::invalid("定时任务行身份与定义不一致；原数据库保留"));
        }
    }
    let mut runs = db
        .prepare("SELECT id, body FROM runs ORDER BY id")
        .map_err(|e| Error::context("定时任务运行记录无法检查；原数据库保留", e))?;
    let run_rows = runs
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| Error::context("定时任务运行记录无法检查；原数据库保留", e))?;
    for row in run_rows {
        let (id, body) =
            row.map_err(|e| Error::context("定时任务运行记录无法读取；原数据库保留", e))?;
        let run: crate::ScheduledRun = serde_json::from_str(&body)
            .map_err(|e| Error::context("定时任务运行记录含未知或无效字段；原数据库保留", e))?;
        if run.id != id {
            return Err(Error::invalid(
                "定时任务运行行身份与记录不一致；原数据库保留",
            ));
        }
    }
    Ok(())
}

fn verify_task_backup(source: &rusqlite::Connection, path: &Path) -> Result<(), Error> {
    let backup = rusqlite::Connection::open(path)
        .map_err(|e| Error::context("已有定时任务恢复副本无法读取；原数据库保留", e))?;
    let version: u32 = backup
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| Error::context("已有定时任务恢复副本版本无效；原数据库保留", e))?;
    if version != 0
        || columns(&backup, "tasks")?
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != ["id", "definition"]
        || columns(&backup, "runs")?
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != ["id", "task_id", "due", "slot", "phase", "body"]
    {
        return Err(Error::invalid(
            "已有定时任务恢复副本与当前迁移不匹配；原数据库保留",
        ));
    }
    let source_tasks = rows(source, "SELECT id, definition FROM tasks ORDER BY id")?;
    let backup_tasks = rows(&backup, "SELECT id, definition FROM tasks ORDER BY id")?;
    let source_runs = rows(
        source,
        "SELECT id, task_id, due, slot, phase, body FROM runs ORDER BY id",
    )?;
    let backup_runs = rows(
        &backup,
        "SELECT id, task_id, due, slot, phase, body FROM runs ORDER BY id",
    )?;
    if source_tasks != backup_tasks || source_runs != backup_runs {
        return Err(Error::invalid("已有定时任务恢复副本内容不同；原数据库保留"));
    }
    Ok(())
}

fn rows(db: &rusqlite::Connection, sql: &str) -> Result<Vec<Vec<rusqlite::types::Value>>, Error> {
    let mut statement = db
        .prepare(sql)
        .map_err(|e| Error::context("无法核验定时任务恢复副本；原数据库保留", e))?;
    let count = statement.column_count();
    let mapped = statement
        .query_map([], |row| {
            (0..count)
                .map(|index| row.get(index))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| Error::context("无法核验定时任务恢复副本；原数据库保留", e))?;
    mapped
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::context("无法核验定时任务恢复副本；原数据库保留", e))
}

fn columns(db: &rusqlite::Connection, table: &str) -> Result<Vec<String>, Error> {
    let mut statement = db
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| Error::context("定时任务数据库结构无法读取；原数据库保留", e))?;
    let result = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| Error::context("定时任务数据库结构无法读取；原数据库保留", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::context("定时任务数据库结构无法读取；原数据库保留", e))?;
    Ok(result)
}

fn migrate_config(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("generic-config.json");
    let Ok(original) = fs::read(&path) else {
        return Ok(());
    };
    let mut value: Value = serde_json::from_slice(&original)?;
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::invalid("generic-config.json 缺少有效 schemaVersion；原文件保留"))?;
    if version == u64::from(CURRENT_SCHEMA_VERSION) {
        return Ok(());
    }
    if version != 10 {
        return Err(Error::Invalid(format!(
            "generic-config.json schema {version} 不受支持；原文件保留"
        )));
    }
    let root = value
        .as_object_mut()
        .ok_or_else(|| Error::invalid("schema 10 普通配置结构无效；原文件保留"))?;
    let allowed = [
        "schemaVersion",
        "gateways",
        "runtimeInstances",
        "modelTemplates",
        "conversationBrowserGroupLimit",
        "conversationBrowserTotalLimit",
        "outlinePresentation",
        "linkedConversationOpening",
        "usageDisplay",
        "contextUsageDisplay",
        "conversationBrowserPreferences",
        "statusLine",
    ];
    if root.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::invalid("schema 10 普通配置含未知字段；原文件保留"));
    }
    let gateways = root
        .get_mut("gateways")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Error::invalid("schema 10 gateways 格式无效；原文件保留"))?;
    for gateway in gateways {
        let providers = gateway
            .get_mut("providers")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| Error::invalid("schema 10 providers 格式无效；原文件保留"))?;
        for provider in providers {
            let Some(query) = provider.get_mut("quotaAuthentication") else {
                continue;
            };
            let object = query.as_object_mut().ok_or_else(|| {
                Error::invalid("schema 10 quotaAuthentication 格式无效；原文件保留")
            })?;
            if object.keys().any(|k| {
                !["accessKey", "generation", "confirmedModelGeneration"].contains(&k.as_str())
            }) {
                return Err(Error::invalid(
                    "schema 10 quotaAuthentication 含未知字段；原文件保留",
                ));
            }
            let key = object.remove("accessKey").unwrap_or(Value::Null);
            let credential = if key.is_null() {
                Value::Null
            } else {
                let fields = key
                    .as_object()
                    .ok_or_else(|| Error::invalid("schema 10 accessKey 格式无效；原文件保留"))?;
                if fields.len() != 2 || !fields.contains_key("id") || !fields.contains_key("secret")
                {
                    return Err(Error::invalid("schema 10 accessKey 字段无效；原文件保留"));
                }
                serde_json::json!({"kind":"accessKeyPair", "key":key})
            };
            object.insert("credential".into(), credential);
        }
    }
    root.insert("schemaVersion".into(), Value::from(CURRENT_SCHEMA_VERSION));
    let validated: crate::repository::PersistedConfig = serde_json::from_value(value.clone())
        .map_err(|e| Error::context("schema 10 普通配置无法安全转换；原文件保留", e))?;
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&serde_json::to_value(validated)?)?,
    });
    Ok(())
}

fn migrate_choices(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("composer-choices.json");
    let Ok(original) = fs::read(&path) else {
        return Ok(());
    };
    let mut choices: Value = serde_json::from_slice(&original)?;
    let version = choices
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            Error::invalid("composer-choices.json 缺少有效 schemaVersion；原文件保留")
        })?;
    if version == 2 {
        return Ok(());
    }
    if version != 1 {
        return Err(Error::Invalid(format!(
            "composer-choices.json schema {version} 不受支持；原文件保留"
        )));
    }
    let config_path = home.join("generic-config.json");
    let config_value =
        if let Some(change) = changes.iter().find(|change| change.path == config_path) {
            serde_json::from_slice::<Value>(&change.replacement)?
        } else if let Ok(bytes) = fs::read(&config_path) {
            serde_json::from_slice::<Value>(&bytes)?
        } else {
            return Err(Error::invalid(
                "schema 1 composer choices 缺少关联普通配置；原文件保留",
            ));
        };
    let instances = config_value
        .get("runtimeInstances")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::invalid("普通配置 runtimeInstances 无效；原文件保留"))?;
    let mut config_updates = Vec::new();
    let runtimes = choices
        .pointer_mut("/choices/runtimes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Error::invalid("旧 composer runtime choices 格式无效；原文件保留"))?;
    let mut seen_runtime_ids = std::collections::BTreeSet::new();
    for runtime in runtimes {
        let object = runtime
            .as_object_mut()
            .ok_or_else(|| Error::invalid("旧 runtime choice 格式无效；原文件保留"))?;
        if object
            .keys()
            .any(|key| !["runtimeID", "lastChosen", "newConversation"].contains(&key.as_str()))
        {
            return Err(Error::invalid("旧 runtime choice 含未知字段；原文件保留"));
        }
        let id = object
            .get("runtimeID")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid("旧 runtime choice 缺少 runtimeID；原文件保留"))?
            .to_owned();
        if !seen_runtime_ids.insert(id.clone()) {
            return Err(Error::invalid(
                "旧 composer runtime choices 含重复实例身份；原文件保留",
            ));
        }
        let config_runtime = instances
            .iter()
            .find(|instance| instance.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .ok_or_else(|| Error::invalid("旧 runtime choice 指向不存在的实例；原文件保留"))?;
        let runtime_type = config_runtime
            .get("typeId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid("旧 runtime 缺少 typeId；原文件保留"))?;
        if let Some(policy) = object.remove("newConversation") {
            let mut runtime = config_runtime.clone();
            let settings = runtime
                .get_mut("settings")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| Error::invalid("旧 runtime settings 无效；原文件保留"))?;
            let encoded = serde_json::to_string(&policy)?;
            if settings
                .get("newConversation")
                .and_then(Value::as_str)
                .is_some_and(|current| current != encoded)
            {
                return Err(Error::invalid("旧新会话策略与实例设置冲突；原文件保留"));
            }
            settings.insert("newConversation".into(), Value::String(encoded));
            config_updates.push((id, runtime));
        }
        object.insert("runtimeType".into(), Value::String(runtime_type.into()));
    }
    if let Some(config_change) = changes.iter_mut().find(|change| change.path == config_path) {
        let mut config: Value = serde_json::from_slice(&config_change.replacement)?;
        apply_runtime_updates(&mut config, config_updates)?;
        config_change.replacement = serde_json::to_vec_pretty(&config)?;
    } else if !config_updates.is_empty() {
        let original_config = fs::read(&config_path)?;
        let mut config: Value = serde_json::from_slice(&original_config)?;
        apply_runtime_updates(&mut config, config_updates)?;
        changes.push(Change {
            path: config_path,
            original: original_config,
            replacement: serde_json::to_vec_pretty(&config)?,
        });
    }
    choices["schemaVersion"] = Value::from(2);
    // The former start policy belongs to a runtime and cannot be inferred from
    // a detached choice record; keep the selected per-turn choices intact.
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&choices)?,
    });
    Ok(())
}

fn apply_runtime_updates(config: &mut Value, updates: Vec<(String, Value)>) -> Result<(), Error> {
    let instances = config
        .get_mut("runtimeInstances")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Error::invalid("普通配置 runtimeInstances 无效；原文件保留"))?;
    for (id, updated) in updates {
        let target = instances
            .iter_mut()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .ok_or_else(|| Error::invalid("迁移中的 runtime 实例消失；原文件保留"))?;
        *target = updated;
    }
    Ok(())
}

fn migrate_composer(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("composer-inputs.json");
    let Ok(original) = fs::read(&path) else {
        return Ok(());
    };
    let mut value: Value = serde_json::from_slice(&original)?;
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::invalid("composer-inputs.json 缺少有效 schemaVersion；原文件保留"))?;
    if version == 2 {
        return Ok(());
    }
    if version != 1 {
        return Err(Error::Invalid(format!(
            "composer-inputs.json schema {version} 不受支持；原文件保留"
        )));
    }
    let envelope = value
        .as_object()
        .ok_or_else(|| Error::invalid("旧 composer envelope 结构无效；原文件保留"))?;
    if envelope
        .keys()
        .any(|key| !["schemaVersion", "state", "attachments"].contains(&key.as_str()))
        || !envelope.contains_key("attachments")
    {
        return Err(Error::invalid(
            "旧 composer envelope 含未知或缺失字段；原文件保留",
        ));
    }
    let state = value
        .get_mut("state")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::invalid("旧 composer state 格式无效；原文件保留"))?;
    if state.keys().any(|key| {
        !["ownerId", "revision", "creations", "inputs", "bindings"].contains(&key.as_str())
    }) || ["ownerId", "revision", "creations", "inputs", "bindings"]
        .iter()
        .any(|key| !state.contains_key(*key))
    {
        return Err(Error::invalid(
            "旧 composer state 含未知或缺失字段；原文件保留",
        ));
    }
    let inputs = state
        .get("inputs")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::invalid("旧 composer inputs 格式无效；原文件保留"))?
        .clone();
    let mut by_id = std::collections::BTreeMap::new();
    for input in &inputs {
        let item = input
            .as_object()
            .ok_or_else(|| Error::invalid("旧 composer input 结构无效；原文件保留"))?;
        if item
            .keys()
            .any(|key| !["id", "revision", "content"].contains(&key.as_str()))
            || ["id", "revision", "content"]
                .iter()
                .any(|key| !item.contains_key(*key))
        {
            return Err(Error::invalid(
                "旧 composer input 含未知或缺失字段；原文件保留",
            ));
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid("旧 composer input 缺少身份；原文件保留"))?
            .to_owned();
        let content = item
            .get("content")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::invalid("旧 composer content 结构无效；原文件保留"))?;
        if content.keys().any(|key| {
            ![
                "text",
                "attachments",
                "runtimeId",
                "runtimeOwnerId",
                "choices",
                "choiceEdited",
            ]
            .contains(&key.as_str())
        }) || !content.contains_key("text")
            || !content.contains_key("attachments")
        {
            return Err(Error::invalid(
                "旧 composer content 含未知或缺失字段；原文件保留",
            ));
        }
        if by_id.insert(id, Value::Object(content.clone())).is_some() {
            return Err(Error::invalid(
                "旧 composer 存在重复 input 身份；原文件保留",
            ));
        }
    }
    let bindings = state
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::invalid("旧 composer bindings 格式无效；原文件保留"))?;
    let mut targets = Vec::new();
    let mut bound_contexts = std::collections::BTreeSet::new();
    for binding in bindings {
        let binding_object = binding
            .as_object()
            .ok_or_else(|| Error::invalid("旧 composer binding 结构无效；原文件保留"))?;
        if binding_object
            .keys()
            .any(|key| !["context", "inputId", "revision"].contains(&key.as_str()))
            || ["context", "inputId", "revision"]
                .iter()
                .any(|key| !binding_object.contains_key(*key))
        {
            return Err(Error::invalid(
                "旧 composer binding 含未知或缺失字段；原文件保留",
            ));
        }
        let context = binding
            .get("context")
            .cloned()
            .ok_or_else(|| Error::invalid("旧 composer binding 缺少 context；原文件保留"))?;
        let context_key = serde_json::to_string(&context)?;
        if !bound_contexts.insert(context_key) {
            return Err(Error::invalid("旧 composer context 重复绑定；原文件保留"));
        }
        let input_id = binding
            .get("inputId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid("旧 composer binding 缺少 inputId；原文件保留"))?;
        let content = by_id
            .get(input_id)
            .ok_or_else(|| Error::invalid("旧 composer binding 指向缺失输入；原文件保留"))?;
        let mut choices = serde_json::Map::new();
        for name in ["runtimeId", "runtimeOwnerId", "choices", "choiceEdited"] {
            if let Some(v) = content.get(name) {
                choices.insert(name.into(), v.clone());
            }
        }
        targets.push(serde_json::json!({"context":context,"revision":1,"choices":choices}));
    }
    for input in state
        .get_mut("inputs")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Error::invalid("旧 composer inputs 格式无效；原文件保留"))?
    {
        let content = input
            .get_mut("content")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| Error::invalid("旧 composer content 格式无效；原文件保留"))?;
        content.retain(|key, _| key == "text" || key == "attachments");
    }
    state.insert(
        "revision".into(),
        Value::from(
            state
                .get("revision")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_add(1),
        ),
    );
    state.insert("targets".into(), Value::Array(targets));
    value["schemaVersion"] = Value::from(2);
    let normalized = crate::composer::validate_migrated(&value)
        .map_err(|e| Error::context("旧 composer 无法安全转换；原文件保留", e))?;
    if !only_known_json_keys(&value, &normalized) {
        return Err(Error::invalid(
            "旧 composer 含无法保留的未知字段；原文件保留",
        ));
    }
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&normalized)?,
    });
    Ok(())
}

fn guard_platform_preferences(home: &Path) -> Result<(), Error> {
    let appearance_path = home.join("mac-appearance.json");
    if let Ok(bytes) = fs::read(&appearance_path) {
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::context("Mac 外观偏好无法读取；原文件保留", e))?;
        let object = value
            .as_object()
            .ok_or_else(|| Error::invalid("Mac 外观偏好结构未知；原文件保留"))?;
        let current_keys = ["ui", "messages", "code", "density", "composerWidthRatio"];
        let legacy_keys = ["ui", "messages", "code", "density", "composerWidth"];
        let is_current = object.contains_key("composerWidthRatio")
            && object
                .keys()
                .all(|key| current_keys.contains(&key.as_str()))
            && object
                .get("composerWidthRatio")
                .and_then(Value::as_f64)
                .is_some_and(|ratio| (0.5..=1.0).contains(&ratio));
        let is_known_legacy = object.contains_key("composerWidth")
            && object.keys().all(|key| legacy_keys.contains(&key.as_str()))
            && object
                .get("composerWidth")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite);
        if !is_current && !is_known_legacy {
            return Err(Error::invalid("Mac 外观偏好不属于当前格式；原文件保留"));
        }
    }
    for (name, fields) in [
        ("mac-tool-layout.json", &["schema", "tools"][..]),
        (
            "mac-shortcuts.json",
            &["schema", "shortcuts", "sendMode"][..],
        ),
        ("windows-layout.json", &["schema", "tools", "windows"][..]),
        (
            "windows-shortcuts.json",
            &["schema", "shortcuts", "sendMode"][..],
        ),
    ] {
        let path = home.join(name);
        let Ok(bytes) = fs::read(&path) else { continue };
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::context("本机偏好文件无法识别；原文件保留", e))?;
        let object = value
            .as_object()
            .ok_or_else(|| Error::invalid("本机偏好文件结构未知；原文件保留"))?;
        if object.keys().any(|key| !fields.contains(&key.as_str()))
            || !object.contains_key("schema")
        {
            return Err(Error::Invalid(format!(
                "{name} 含未知字段或缺少 schema；原文件保留"
            )));
        }
        let schema = value.get("schema").and_then(Value::as_u64);
        if schema != Some(1) {
            return Err(Error::Invalid(format!(
                "{name} schema 不受支持；原文件保留"
            )));
        }
    }
    Ok(())
}

fn write_new_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    let temp = path.with_file_name(format!(".{name}.beta-migrate.tmp"));
    let _ = fs::remove_file(&temp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    fn home() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "velune-beta-migration-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn migrates_schema_ten_with_private_exact_backup() {
        let home = home();
        let source =
            br#"{"schemaVersion":10,"gateways":[],"runtimeInstances":[],"modelTemplates":[]}"#;
        fs::write(home.join("generic-config.json"), source).unwrap();
        prepare(&home).unwrap();
        let migrated: Value =
            serde_json::from_slice(&fs::read(home.join("generic-config.json")).unwrap()).unwrap();
        assert_eq!(migrated["schemaVersion"], 11);
        let backup_dir = fs::read_dir(&home)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("before-beta-migration-")
            })
            .unwrap();
        assert_eq!(
            fs::read(backup_dir.join("generic-config.json")).unwrap(),
            source
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn future_config_is_rejected_without_writes_or_backup() {
        let home = home();
        let source = br#"{"schemaVersion":99,"synthetic":"retain"}"#;
        fs::write(home.join("generic-config.json"), source).unwrap();
        assert!(prepare(&home).is_err());
        assert_eq!(fs::read(home.join("generic-config.json")).unwrap(), source);
        assert_eq!(fs::read_dir(&home).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn pre_beta_service_tier_points_to_preserving_transition_script() {
        let home = home();
        let path = home.join("composer-choices.json");
        let source = br#"{"schemaVersion":1,"choices":{"runtimes":[{"runtimeID":"synthetic","lastChosen":{"serviceTier":null}}],"conversations":[]}}"#;
        fs::write(&path, source).unwrap();
        let error = prepare(&home).unwrap_err().to_string();
        assert!(error.contains("scripts/run-migrations.py"));
        assert_eq!(fs::read(path).unwrap(), source);
        assert_eq!(fs::read_dir(&home).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn transition_detection_ignores_user_text_and_provider_parameters() {
        let home = home();
        fs::write(
            home.join("generic-config.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion":11,
                "runtimeInstances":[{"id":"synthetic","typeId":"codex-0.166","settings":{"note":"codex-0.159"}}],
                "gateways":[{"providers":[{"requestParameters":{"serviceTier":"literal"}}]}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            home.join("composer-inputs.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion":2,
                "state":{"inputs":[{"content":{"text":"serviceTier codex-0.159"}}],"targets":[]}
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(detect_pre_beta_transitions(&home).is_ok());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn known_legacy_mac_appearance_is_left_for_native_owner_under_home_lock() {
        let home = home();
        let path = home.join("mac-appearance.json");
        let source = br#"{"ui":"standard","messages":"standard","code":"standard","density":"standard","composerWidth":900}"#;
        fs::write(&path, source).unwrap();
        guard_platform_preferences(&home).unwrap();
        assert_eq!(fs::read(path).unwrap(), source);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn duplicate_legacy_runtime_choices_are_rejected_without_overwrite() {
        let home = home();
        let path = home.join("composer-choices.json");
        let source = br#"{"schemaVersion":1,"choices":{"runtimes":[{"runtimeID":"synthetic","lastChosen":null},{"runtimeID":"synthetic","lastChosen":null}],"conversations":[]}}"#;
        fs::write(&path, source).unwrap();
        assert!(prepare(&home).is_err());
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::read_dir(&home).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn future_scheduled_record_blocks_baseline_registration_without_writes() {
        let home = home();
        let path = home.join("scheduled-tasks.sqlite3");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY, definition TEXT NOT NULL); CREATE TABLE runs(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, due INTEGER NOT NULL, slot TEXT NOT NULL, phase TEXT NOT NULL, body TEXT NOT NULL, UNIQUE(task_id,due,slot));")
            .unwrap();
        let task = serde_json::json!({
            "id":"synthetic","revision":1,"name":"Synthetic","prompt":"Keep","rrule":"FREQ=DAILY","timeZone":"UTC",
            "status":"active","target":{"kind":"newConversation","runtimeId":"synthetic","workingDirectories":[],"worktree":false},
            "model":null,"effort":null,"permissionMode":null,"notifyOnlyFailures":false,"nextRunAtUnixMs":null,
            "source":null,"issue":null,"future":"retain"
        });
        db.execute(
            "INSERT INTO tasks VALUES(?1, ?2)",
            rusqlite::params!["synthetic", task.to_string()],
        )
        .unwrap();
        drop(db);
        let before = fs::read(&path).unwrap();
        assert!(prepare(&home).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert!(
            !home
                .join("scheduled-tasks.before-beta-migration.sqlite3")
                .exists()
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn future_scheduled_run_blocks_startup_rewrite_without_writes() {
        let home = home();
        let path = home.join("scheduled-tasks.sqlite3");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY, definition TEXT NOT NULL); CREATE TABLE runs(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, due INTEGER NOT NULL, slot TEXT NOT NULL, phase TEXT NOT NULL, body TEXT NOT NULL, UNIQUE(task_id,due,slot)); PRAGMA user_version=1;")
            .unwrap();
        let task = serde_json::json!({
            "id":"synthetic","revision":1,"name":"Synthetic","prompt":"Keep","rrule":"FREQ=DAILY","timeZone":"UTC",
            "status":"paused","target":{"kind":"newConversation","runtimeId":"synthetic","workingDirectories":[],"worktree":false},
            "model":null,"effort":null,"permissionMode":null,"notifyOnlyFailures":false,"nextRunAtUnixMs":null,
            "source":null,"issue":null
        });
        db.execute(
            "INSERT INTO tasks VALUES(?1, ?2)",
            rusqlite::params!["synthetic", task.to_string()],
        )
        .unwrap();
        let run = serde_json::json!({
            "id":"run-1","taskId":"synthetic","definitionRevision":1,"dueAtUnixMs":1,"phase":"waiting",
            "turnOutcome":null,"cwd":null,"runtimeId":"synthetic","conversationId":null,"detail":null,"future":"retain"
        });
        db.execute(
            "INSERT INTO runs VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "run-1",
                "synthetic",
                1i64,
                "slot",
                "Waiting",
                run.to_string()
            ],
        )
        .unwrap();
        drop(db);
        let before = fs::read(&path).unwrap();
        assert!(prepare(&home).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn legacy_service_tier_composer_round_trips_through_runner_and_application() {
        use std::process::Command;
        let home = home();
        let owner = "11111111-1111-4111-8111-111111111111";
        let input = "22222222-2222-4222-8222-222222222222";
        let attachment = "33333333-3333-4333-8333-333333333333";
        let choice = serde_json::json!({
            "model":{"native":{"provider_id":null,"model_id":"gpt-test"}},
            "effort":"high","permissionMode":"read-only","serviceTier":null
        });
        let mut config =
            serde_json::to_value(crate::repository::PersistedConfig::default()).unwrap();
        config["gateways"] = serde_json::json!([{"id":"default","name":"Default","providers":[]}]);
        config["runtimeInstances"] = serde_json::json!([{"id":"codex","name":"Synthetic Codex","enabled":false,"typeId":"codex-0.166","gatewayId":"default","settings":{}}]);
        fs::write(
            home.join("generic-config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        fs::write(home.join("composer-inputs.json"), serde_json::to_vec(&serde_json::json!({
            "schemaVersion":1,"state":{"ownerId":owner,"revision":9,"creations":[],"inputs":[{"id":input,"revision":4,"content":{
                "text":"合成草稿","attachments":[{"id":attachment,"name":"draft.txt","bytes":6}],
                "runtimeId":"codex","runtimeOwnerId":owner,"choices":[{"ownerId":owner,"runtimeId":"codex","choice":choice}],"choiceEdited":true
            }}],"bindings":[{"context":{"kind":"newConversation"},"inputId":input,"revision":9}]},"attachments":[{"id":attachment,"name":"draft.txt","bytes":6}]
        })).unwrap()).unwrap();
        fs::write(home.join("composer-choices.json"), serde_json::to_vec(&serde_json::json!({
            "schemaVersion":1,"choices":{"conversations":[],"runtimes":[{"runtimeID":"codex","lastChosen":choice,"newConversation":{"kind":"chosen","choice":choice}}]}
        })).unwrap()).unwrap();
        let attachment_path = home.join("composer-attachments").join(attachment);
        fs::create_dir_all(&attachment_path).unwrap();
        fs::write(attachment_path.join("draft.txt"), b"sample").unwrap();

        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runner = repository.join("scripts/run-migrations.py");
        let result = Command::new("python3")
            .arg(runner)
            .arg("--home")
            .arg(&home)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );

        let application = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: home.clone(),
        })
        .unwrap();
        assert_eq!(application.composer.legacy_drafts()[0].text, "合成草稿");
        let content = application.composer.read_attachment(attachment).unwrap();
        assert_eq!(content.attachment.name, "draft.txt");
        assert_eq!(content.bytes, b"sample");
        let target = application
            .composer
            .target(&velune_application_api::ComposerContextKey::NewConversation)
            .unwrap();
        let chosen = serde_json::to_value(&target.choices.choices[0].choice).unwrap();
        assert_eq!(chosen["model"]["native"]["model_id"], "gpt-test");
        assert!(chosen.get("serviceTier").is_none());
        drop(application);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn new_home_has_no_migration_side_effects() {
        let home = home();
        prepare(&home).unwrap();
        assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn registers_known_task_database_only_after_consistent_backup() {
        let home = home();
        let path = home.join("scheduled-tasks.sqlite3");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY, definition TEXT NOT NULL); CREATE TABLE runs(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, due INTEGER NOT NULL, slot TEXT NOT NULL, phase TEXT NOT NULL, body TEXT NOT NULL, UNIQUE(task_id,due,slot));")
            .unwrap();
        let definition = serde_json::json!({
            "id":"synthetic","revision":1,"name":"Synthetic","prompt":"Keep","rrule":"FREQ=DAILY","timeZone":"UTC",
            "status":"paused","target":{"kind":"newConversation","runtimeId":"synthetic","workingDirectories":[],"worktree":false},
            "model":null,"effort":null,"permissionMode":null,"notifyOnlyFailures":false,"nextRunAtUnixMs":null,
            "source":null,"issue":null
        })
        .to_string();
        db.execute(
            "INSERT INTO tasks VALUES(?1, ?2)",
            rusqlite::params!["synthetic", definition],
        )
        .unwrap();
        drop(db);
        prepare(&home).unwrap();
        let migrated = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            migrated
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            migrated
                .query_row(
                    "SELECT definition FROM tasks WHERE id='synthetic'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            definition
        );
        drop(migrated);
        let backup =
            rusqlite::Connection::open(home.join("scheduled-tasks.before-beta-migration.sqlite3"))
                .unwrap();
        assert_eq!(
            backup
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            backup
                .query_row(
                    "SELECT definition FROM tasks WHERE id='synthetic'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            definition
        );
        fs::remove_dir_all(home).unwrap();
    }
}
