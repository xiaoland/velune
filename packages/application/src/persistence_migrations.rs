//! Coordinate application-owned JSON upgrades under the application home lock.
//! Owners prepare and validate conversions before the coordinator backs up and
//! replaces the files in dependency order.
use crate::Error;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) struct Change {
    pub(crate) path: PathBuf,
    pub(crate) original: Vec<u8>,
    pub(crate) replacement: Vec<u8>,
}

pub(crate) fn prepare(home: &Path) -> Result<(), Error> {
    let mut changes = Vec::new();
    crate::composer::prepare_migration(home, &mut changes)?;
    crate::repository::prepare_migrations(home, &mut changes)?;
    if changes.is_empty() {
        return Ok(());
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup = home.join(format!("before-beta-migration-{stamp}"));
    let staging = home.join(format!(".before-beta-migration-{stamp}.tmp"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&staging)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(&staging)?;
    let backup_result = (|| -> Result<(), Error> {
        for change in &changes {
            let target = staging.join(
                change
                    .path
                    .file_name()
                    .ok_or_else(|| Error::invalid("migration backup path"))?,
            );
            write_new_private(&target, &change.original)?;
        }
        #[cfg(unix)]
        fs::File::open(&staging)?.sync_all()?;
        fs::rename(&staging, backup)?;
        #[cfg(unix)]
        fs::File::open(home)?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = backup_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    for change in &changes {
        atomic_write(&change.path, &change.replacement)?;
        #[cfg(unix)]
        fs::File::open(home)?.sync_all()?;
    }
    Ok(())
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn only_known_json_keys(source: &Value, normalized: &Value) -> bool {
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
    fn application_opens_with_platform_state_left_to_its_owner() {
        let home = home();
        let config_path = home.join("generic-config.json");
        let config = crate::repository::PersistedConfig {
            conversation_browser_group_limit: 42,
            ..Default::default()
        };
        let config_bytes = serde_json::to_vec(&config).unwrap();
        fs::write(&config_path, &config_bytes).unwrap();
        let snapshots = home.join("mac-window-workspaces");
        fs::create_dir(&snapshots).unwrap();
        let snapshot = snapshots.join("current.json");
        let snapshot_bytes = br#"{"schemaVersion":1,"composer":{"text":"draft","attachments":[]},"composerWorkspace":{"schemaVersion":3}}"#;
        fs::write(&snapshot, snapshot_bytes).unwrap();
        let unsupported = snapshots.join("unknown.json");
        fs::write(&unsupported, b"corrupt").unwrap();
        let appearance = home.join("mac-appearance.json");
        fs::write(&appearance, br#"{"schemaVersion":99}"#).unwrap();

        let application = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: home.clone(),
        })
        .unwrap();
        drop(application);
        assert!(home.join("composer-inputs.json").is_file());
        assert_eq!(fs::read(&config_path).unwrap(), config_bytes);
        assert_eq!(fs::read(&snapshot).unwrap(), snapshot_bytes);
        assert_eq!(fs::read(&unsupported).unwrap(), b"corrupt");
        assert_eq!(fs::read(&appearance).unwrap(), br#"{"schemaVersion":99}"#);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn interrupted_legacy_import_does_not_create_a_new_catalog() {
        for (name, original) in [
            ("composer-inputs.schema1.backup.json", br#"{"schemaVersion":1}"#.as_slice()),
            ("composer-choices.json", br#"{"schemaVersion":2,"choices":{"conversations":[{"conversationID":"synthetic","runtimeID":null,"choices":[]}],"runtimes":[]}}"#.as_slice()),
        ] {
            let home = home();
            let source = home.join(name);
            fs::write(&source, original).unwrap();
            let result = crate::Application::open(crate::Options {
                home_directory: home.clone(),
                resources_directory: home.clone(),
            });
            assert!(
                matches!(result, Err(error) if error.to_string().contains("scripts/run-migrations.py"))
            );
            assert!(!home.join("composer-inputs.json").exists());
            assert_eq!(fs::read(&source).unwrap(), original);
            fs::remove_dir_all(home).unwrap();
        }
    }

    #[test]
    fn resumes_policy_transfer_after_configuration_commit() {
        let home = home();
        let config_path = home.join("generic-config.json");
        let choices_path = home.join("composer-choices.json");
        let mut config =
            serde_json::to_value(crate::repository::PersistedConfig::default()).unwrap();
        config["gateways"] = serde_json::json!([{"id":"default","name":"Default","providers":[]}]);
        config["runtimeInstances"] = serde_json::json!([{"id":"codex","name":"Synthetic Codex","enabled":false,"typeId":"codex-0.166","gatewayId":"default","settings":{}}]);
        let config_bytes = serde_json::to_vec(&config).unwrap();
        let choices_bytes = br#"{"schemaVersion":1,"choices":{"conversations":[],"runtimes":[{"runtimeID":"codex","lastChosen":{"model":null,"effort":null,"permissionMode":null},"newConversation":{"kind":"lastChosen"}}]}}"#;
        fs::write(&config_path, &config_bytes).unwrap();
        crate::composer::ComposerInputs::open(&home).unwrap();
        fs::write(&choices_path, choices_bytes).unwrap();

        let blocked_temp = home.join(".composer-choices.json.beta-migrate.tmp");
        fs::create_dir(&blocked_temp).unwrap();
        assert!(prepare(&home).is_err());
        let partial_config: Value =
            serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        assert_eq!(
            partial_config["runtimeInstances"][0]["settings"]["newConversation"],
            r#"{"kind":"lastChosen"}"#
        );
        assert_eq!(fs::read(&choices_path).unwrap(), choices_bytes);
        let backup = fs::read_dir(&home)
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
            fs::read(backup.join("generic-config.json")).unwrap(),
            config_bytes
        );
        assert_eq!(
            fs::read(backup.join("composer-choices.json")).unwrap(),
            choices_bytes
        );

        fs::remove_dir(blocked_temp).unwrap();
        let application = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: home.clone(),
        })
        .unwrap();
        drop(application);
        let converted: Value = serde_json::from_slice(&fs::read(&choices_path).unwrap()).unwrap();
        assert_eq!(converted["schemaVersion"], 2);
        assert_eq!(
            converted["choices"]["runtimes"][0]["runtimeType"],
            "codex-0.166"
        );
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
    fn duplicate_legacy_runtime_choices_are_rejected_without_overwrite() {
        let home = home();
        let path = home.join("composer-choices.json");
        crate::composer::ComposerInputs::open(&home).unwrap();
        let mut config =
            serde_json::to_value(crate::repository::PersistedConfig::default()).unwrap();
        config["gateways"] = serde_json::json!([{"id":"default","name":"Default","providers":[]}]);
        config["runtimeInstances"] = serde_json::json!([{"id":"synthetic","name":"Synthetic","enabled":false,"typeId":"codex-0.166","gatewayId":"default","settings":{}}]);
        fs::write(
            home.join("generic-config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let source = br#"{"schemaVersion":1,"choices":{"runtimes":[{"runtimeID":"synthetic","lastChosen":{}},{"runtimeID":"synthetic","lastChosen":{}}],"conversations":[]}}"#;
        fs::write(&path, source).unwrap();
        assert!(
            prepare(&home)
                .unwrap_err()
                .to_string()
                .contains("重复实例身份")
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::read_dir(&home).unwrap().count(), 3);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn known_composer_catalog_upgrades_with_original_content_and_choices() {
        let home = home();
        let path = home.join("composer-inputs.json");
        let input = "22222222-2222-4222-8222-222222222222";
        let owner = "11111111-1111-4111-8111-111111111111";
        let original = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "state": {
                "ownerId": owner, "revision": 4, "creations": [],
                "inputs": [{"id": input, "revision": 3, "content": {
                    "text": "draft", "attachments": [], "runtimeId": "synthetic", "runtimeOwnerId": owner,
                    "choices": [{"ownerId": owner, "runtimeId": "synthetic", "choice": {"effort": "high"}}],
                    "choiceEdited": true
                }}],
                "bindings": [{"context": {"kind": "newConversation"}, "inputId": input, "revision": 4}]
            },
            "attachments": []
        })).unwrap();
        fs::write(&path, &original).unwrap();
        let app = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: home.clone(),
        })
        .unwrap();
        assert_eq!(app.composer.legacy_drafts()[0].text, "draft");
        let target = app
            .composer
            .target(&velune_application_api::ComposerContextKey::NewConversation)
            .unwrap();
        assert_eq!(
            target.choices.choices[0].choice.effort.as_deref(),
            Some("high")
        );
        drop(app);
        let migrated: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(migrated["schemaVersion"], 2);
        assert_eq!(migrated["state"]["inputs"][0]["id"], input);
        let backup = fs::read_dir(&home)
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
            fs::read(backup.join("composer-inputs.json")).unwrap(),
            original
        );
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
        assert!(crate::scheduled_tasks::Store::open(&home).is_err());
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
        assert!(crate::scheduled_tasks::Store::open(&home).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn legacy_service_tier_composer_round_trips_through_runner_and_application() {
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
        let result = velune_agent_runtime::process::command("python3")
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
        drop(crate::scheduled_tasks::Store::open(&home).unwrap());
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
