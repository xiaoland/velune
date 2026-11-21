//! Known format conversions for repository-owned configuration and remembered choices.
use super::{
    COMPOSER_CHOICES_SCHEMA, CURRENT_SCHEMA_VERSION, ORGANIZATION_SCHEMA_VERSION,
    RuntimeError as Error, decode_composer_choices, decode_config,
    decode_conversation_organization, reject_unknown_envelope_fields, retired_codex,
};
use crate::{
    config::{
        ComposerChoices, NewConversationChoice, NextTurnIntent, RuntimeComposerChoices, TurnChoice,
    },
    persistence_migrations::{Change, only_known_json_keys, read_optional},
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};
use velune_application_api::NEW_CONVERSATION_SETTING;

pub(crate) fn prepare_migrations(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    migrate_config(home, changes)?;
    migrate_choices(home, changes)?;
    migrate_organization(home, changes)?;
    migrate_links(home, changes)
}

fn migrate_links(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("conversation-links.json");
    let Some(original) = read_optional(&path)? else {
        return Ok(());
    };
    let mut value: Value = serde_json::from_slice(&original)?;
    // The current decoder validates all IDs, cutoffs and known fields before
    // any migration may commit. Future/corrupt data is never reinterpreted.
    super::decode_conversation_links(value.clone())?;
    if value["schemaVersion"].as_u64() == Some(3) {
        return Ok(());
    }
    value["schemaVersion"] = 3.into();
    super::decode_conversation_links(value.clone())?;
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&value)?,
    });
    Ok(())
}

fn migrate_organization(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("conversation-organization.json");
    let Some(original) = read_optional(&path)? else {
        return Ok(());
    };
    let mut value: Value = serde_json::from_slice(&original)?;
    match value["schemaVersion"].as_u64() {
        Some(ORGANIZATION_SCHEMA_VERSION) => {
            decode_conversation_organization(value)?;
            return Ok(());
        }
        Some(2) => {}
        _ => {
            return Err(Error::invalid(
                "conversation-organization.json schema 不受支持；原文件保留",
            ));
        }
    }
    // V2 had no question acknowledgements. Validate its known mark fields
    // before using the current decoder; a future field must not be reinterpreted.
    if value["organization"]["conversations"]
        .as_array()
        .is_some_and(|marks| {
            marks.iter().any(|mark| {
                mark.as_object().is_some_and(|fields| {
                    fields.keys().any(|key| {
                        ![
                            "conversationID",
                            "pinnedAtUnixMs",
                            "archivedAtUnixMs",
                            "seenActivityUnixMs",
                            "markedUnread",
                            "project",
                        ]
                        .contains(&key.as_str())
                    })
                })
            })
        })
    {
        return Err(Error::invalid(
            "conversation-organization.json schema 2 会话标记含未知字段；原文件保留",
        ));
    }
    value["schemaVersion"] = Value::from(ORGANIZATION_SCHEMA_VERSION);
    decode_conversation_organization(value.clone())?;
    // Preserve the exact legacy JSON values; absent acknowledgements decode empty.
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&value)?,
    });
    Ok(())
}

fn migrate_config(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("generic-config.json");
    let Some(original) = read_optional(&path)? else {
        return Ok(());
    };
    let mut value: Value = serde_json::from_slice(&original)?;
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::invalid("generic-config.json 缺少有效 schemaVersion；原文件保留"))?;
    if version == u64::from(CURRENT_SCHEMA_VERSION) {
        decode_config(value)?;
        return Ok(());
    }
    if version != 10 {
        return Err(Error::Invalid(format!(
            "generic-config.json schema {version} 不受支持；原文件保留"
        )));
    }
    if value["runtimeInstances"]
        .as_array()
        .is_some_and(|runtimes| {
            runtimes
                .iter()
                .any(|runtime| runtime["typeId"].as_str() == Some("codex-0.159"))
        })
    {
        return Err(retired_codex());
    }
    let root = value
        .as_object_mut()
        .ok_or_else(|| Error::invalid("schema 10 普通配置结构无效；原文件保留"))?;
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
    let typed: super::PersistedConfig = serde_json::from_value(value.clone())
        .map_err(|e| Error::context("schema 10 普通配置无法安全转换；原文件保留", e))?;
    let normalized = serde_json::to_value(typed)?;
    if !only_known_json_keys(&value, &normalized) {
        return Err(Error::invalid("schema 10 普通配置含未知字段；原文件保留"));
    }
    decode_config(normalized.clone())?;
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&normalized)?,
    });
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChoicesV1 {
    conversations: Vec<NextTurnIntent>,
    runtimes: Vec<RuntimeChoiceV1>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeChoiceV1 {
    #[serde(rename = "runtimeID")]
    runtime_id: String,
    last_chosen: TurnChoice,
    new_conversation: Option<NewConversationChoice>,
}

fn migrate_choices(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("composer-choices.json");
    let Some(original) = read_optional(&path)? else {
        return Ok(());
    };
    let choices: Value = serde_json::from_slice(&original)?;
    match choices["schemaVersion"].as_u64() {
        Some(COMPOSER_CHOICES_SCHEMA) => {
            decode_composer_choices(choices)?;
            return Ok(());
        }
        Some(1) => {}
        _ => {
            return Err(Error::invalid(
                "composer-choices.json schema 不受支持；原文件保留",
            ));
        }
    }
    reject_unknown_envelope_fields("composer-choices.json", &choices, &["choices"])?;
    let legacy: ChoicesV1 = serde_json::from_value(choices["choices"].clone()).map_err(|error| {
        Error::context(
            "旧选择无法安全转换；原文件保留。已知 pre-beta 格式可参照数据迁移说明运行 scripts/run-migrations.py",
            error,
        )
    })?;
    let config_path = home.join("generic-config.json");
    let config_change = changes.iter_mut().find(|change| change.path == config_path);
    let config_bytes = match config_change.as_ref() {
        Some(change) => change.replacement.clone(),
        None => read_optional(&config_path)?.ok_or_else(|| {
            Error::invalid("schema 1 composer choices 缺少关联普通配置；原文件保留")
        })?,
    };
    let mut config = decode_config(serde_json::from_slice(&config_bytes)?)?;
    let mut config_changed = false;
    let mut seen = BTreeSet::new();
    let mut runtimes = Vec::with_capacity(legacy.runtimes.len());
    for choice in legacy.runtimes {
        if !seen.insert(choice.runtime_id.clone()) {
            return Err(Error::invalid(
                "旧 composer runtime choices 含重复实例身份；原文件保留",
            ));
        }
        let mut matches = config
            .runtime_instances
            .iter_mut()
            .filter(|instance| instance.id == choice.runtime_id);
        let runtime = matches
            .next()
            .ok_or_else(|| Error::invalid("旧 runtime choice 指向不存在的实例；原文件保留"))?;
        if matches.next().is_some() {
            return Err(Error::invalid("旧选择的运行时实例身份不唯一；原文件保留"));
        }
        if let Some(policy) = choice.new_conversation {
            if runtime.settings.contains_key(NEW_CONVERSATION_SETTING) {
                if runtime.new_conversation_choice()? != policy {
                    return Err(Error::invalid("旧新会话策略与实例设置冲突；原文件保留"));
                }
            } else {
                runtime.set_new_conversation_choice(policy)?;
                config_changed = true;
            }
        }
        runtimes.push(RuntimeComposerChoices {
            runtime_id: choice.runtime_id,
            runtime_type: runtime.type_id,
            last_chosen: choice.last_chosen,
        });
    }

    if config_changed {
        let replacement = serde_json::to_vec_pretty(&config)?;
        if let Some(change) = config_change {
            change.replacement = replacement;
        } else {
            changes.push(Change {
                path: config_path,
                original: config_bytes,
                replacement,
            });
        }
    }
    // Commit the destination first. Until then, v1 retains the original
    // policy so a failed or interrupted upgrade can be retried.
    let choices = ComposerChoices {
        conversations: legacy.conversations,
        runtimes,
    };
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": COMPOSER_CHOICES_SCHEMA,
            "choices": choices,
        }))?,
    });
    Ok(())
}

#[cfg(test)]
mod organization_tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Home(PathBuf);
    impl Home {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "velune-organization-migration-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn source(&self) -> PathBuf {
            self.0.join("conversation-organization.json")
        }
        fn backups(&self) -> Vec<PathBuf> {
            fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("before-beta-migration-")
                })
                .collect()
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn legacy() -> Value {
        serde_json::json!({
            "schemaVersion": 2,
            "organization": {
                "generatedTitles": {"synthetic:conversation": "保留标题"},
                "readBaselineUnixMs": 123,
                "conversations": [{
                    "conversationID": "synthetic:conversation",
                    "pinnedAtUnixMs": 124, "archivedAtUnixMs": 125,
                    "seenActivityUnixMs": 126, "markedUnread": true,
                    "project": "/synthetic/project"
                }],
                "projects": [{"primaryFolder": "/synthetic/project", "name": "保留项目", "folders": ["/synthetic/extra"]}]
            }
        })
    }

    #[test]
    fn upgrades_with_exact_private_backup_and_preserves_current_acknowledgements() {
        let home = Home::new();
        let original = serde_json::to_vec(&legacy()).unwrap();
        fs::write(home.source(), &original).unwrap();
        let (repository, _) = super::super::Repository::open(&home.0).unwrap();
        let migrated: Value = serde_json::from_slice(&fs::read(home.source()).unwrap()).unwrap();
        assert_eq!(migrated["schemaVersion"], 3);
        assert_eq!(migrated["organization"], legacy()["organization"]);
        let backups = home.backups();
        assert_eq!(backups.len(), 1);
        let backup = backups[0].join("conversation-organization.json");
        assert_eq!(fs::read(&backup).unwrap(), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&backups[0]).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let mut organization = repository.load_conversation_organization().unwrap();
        assert!(
            organization.conversations[0]
                .resolved_async_questions
                .is_empty()
        );
        organization.conversations[0]
            .resolved_async_questions
            .push("codex-async:answered".into());
        repository
            .store_conversation_organization(&organization)
            .unwrap();
        let current = fs::read(home.source()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&current).unwrap()["schemaVersion"],
            3
        );
        drop(repository);

        let (reopened, _) = super::super::Repository::open(&home.0).unwrap();
        assert_eq!(
            reopened.load_conversation_organization().unwrap(),
            organization
        );
        assert_eq!(fs::read(home.source()).unwrap(), current);
        assert_eq!(home.backups().len(), 1);
    }

    #[test]
    fn rejects_unknown_and_corrupt_organization_before_any_file_changes() {
        let mut invalid = vec![b"{broken".to_vec()];
        for version in [1, 4] {
            let mut value = legacy();
            value["schemaVersion"] = version.into();
            invalid.push(serde_json::to_vec(&value).unwrap());
        }
        for pointer in [
            "/future",
            "/organization/future",
            "/organization/conversations/0/future",
            "/organization/projects/0/future",
            "/organization/conversations/0/resolvedAsyncQuestions",
        ] {
            let mut value = legacy();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            value.pointer_mut(parent).unwrap()[key] = serde_json::json!(["retain"]);
            invalid.push(serde_json::to_vec(&value).unwrap());
        }
        let mut empty_mark = legacy();
        empty_mark["organization"]["conversations"] =
            serde_json::json!([{"conversationID":"empty"}]);
        invalid.push(serde_json::to_vec(&empty_mark).unwrap());
        for original in invalid {
            let home = Home::new();
            fs::write(home.source(), &original).unwrap();
            // Even a valid earlier migration must not commit before validation ends.
            let config =
                br#"{"schemaVersion":10,"gateways":[],"runtimeInstances":[],"modelTemplates":[]}"#;
            fs::write(home.0.join("generic-config.json"), config).unwrap();
            assert!(crate::persistence_migrations::prepare(&home.0).is_err());
            assert_eq!(fs::read(home.source()).unwrap(), original);
            assert_eq!(
                fs::read(home.0.join("generic-config.json")).unwrap(),
                config
            );
            assert!(home.backups().is_empty());
        }
    }

    #[test]
    fn failed_replacement_keeps_source_and_recoverable_backup_then_retries() {
        let home = Home::new();
        let original = serde_json::to_vec(&legacy()).unwrap();
        fs::write(home.source(), &original).unwrap();
        let blocked = home
            .0
            .join(".conversation-organization.json.beta-migrate.tmp");
        fs::create_dir(&blocked).unwrap();
        assert!(crate::persistence_migrations::prepare(&home.0).is_err());
        assert_eq!(fs::read(home.source()).unwrap(), original);
        let backups = home.backups();
        assert_eq!(backups.len(), 1);
        let backup = backups[0].join("conversation-organization.json");
        assert_eq!(fs::read(&backup).unwrap(), original);
        fs::remove_dir(blocked).unwrap();
        crate::persistence_migrations::prepare(&home.0).unwrap();
        let migrated = fs::read(home.source()).unwrap();
        fs::copy(backup, home.source()).unwrap();
        crate::persistence_migrations::prepare(&home.0).unwrap();
        assert_eq!(fs::read(home.source()).unwrap(), migrated);
    }
}

#[cfg(test)]
mod links_tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Home(PathBuf);
    impl Home {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "velune-links-migration-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn source(&self) -> PathBuf {
            self.0.join("conversation-links.json")
        }
        fn backups(&self) -> Vec<PathBuf> {
            fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("before-beta-migration-")
                })
                .collect()
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn legacy(version: u64) -> Value {
        serde_json::json!({"schemaVersion":version,"links":[{"id":"source:thread","segments":[
            {"runtimeInstanceId":"source","runtimeTypeId":crate::conversation::RuntimeKind::Codex166,"nativeConversationId":"source:thread","cwd":"/synthetic","cutoff":{"messageCount":1,"prefixDigest":"a".repeat(64)},"handoff":null,"deleted":false},
            {"runtimeInstanceId":"pi","runtimeTypeId":crate::conversation::RuntimeKind::Pi,"nativeConversationId":"pi:/synthetic/session.jsonl","cwd":"/synthetic","cutoff":null,"handoff":null,"imported":{"prefix":{"messageCount":1,"prefixDigest":"b".repeat(64)}},"deleted":false}
        ]}]})
    }
    #[test]
    fn known_links_upgrade_with_exact_backup_and_reopen_summary_provenance() {
        for version in [1, 2] {
            let home = Home::new();
            let original = serde_json::to_vec(&legacy(version)).unwrap();
            fs::write(home.source(), &original).unwrap();
            let (repository, _) = super::super::Repository::open(&home.0).unwrap();
            let disk: Value = serde_json::from_slice(&fs::read(home.source()).unwrap()).unwrap();
            assert_eq!(disk["schemaVersion"], 3);
            assert_eq!(disk["links"], legacy(version)["links"]);
            let backups = home.backups();
            assert_eq!(backups.len(), 1);
            assert_eq!(
                fs::read(backups[0].join("conversation-links.json")).unwrap(),
                original
            );
            let mut links = repository.load_conversation_links().unwrap();
            links[0].segments[1]
                .imported
                .as_mut()
                .unwrap()
                .summary_work_id = Some("work-1".into());
            repository.store_conversation_links(&links).unwrap();
            assert!(
                super::super::Repository::open(&home.0).is_err(),
                "the fixture must still own the directory lock"
            );
            repository.unlock().unwrap();
            drop(repository);
            let (reopened, _) = super::super::Repository::open(&home.0).unwrap();
            assert_eq!(reopened.load_conversation_links().unwrap(), links);
            assert_eq!(home.backups().len(), 1);
        }
    }
    #[test]
    fn unknown_or_corrupt_links_never_overwrite_or_create_partial_migration() {
        let mut invalid = vec![b"{broken".to_vec()];
        let mut future = legacy(4);
        invalid.push(serde_json::to_vec(&future).unwrap());
        future = legacy(2);
        future["links"][0]["segments"][1]["imported"]["summaryWorkId"] = Value::Null;
        invalid.push(serde_json::to_vec(&future).unwrap());
        future = legacy(2);
        future["links"][0]["future"] = true.into();
        invalid.push(serde_json::to_vec(&future).unwrap());
        for original in invalid {
            let home = Home::new();
            fs::write(home.source(), &original).unwrap();
            assert!(crate::persistence_migrations::prepare(&home.0).is_err());
            assert_eq!(fs::read(home.source()).unwrap(), original);
            assert!(home.backups().is_empty());
        }
    }
    #[test]
    fn interrupted_links_upgrade_keeps_recoverable_source_and_retries() {
        let home = Home::new();
        let original = serde_json::to_vec(&legacy(2)).unwrap();
        fs::write(home.source(), &original).unwrap();
        let blocked = home.0.join(".conversation-links.json.beta-migrate.tmp");
        fs::create_dir(&blocked).unwrap();
        assert!(crate::persistence_migrations::prepare(&home.0).is_err());
        assert_eq!(fs::read(home.source()).unwrap(), original);
        assert_eq!(
            fs::read(home.backups()[0].join("conversation-links.json")).unwrap(),
            original
        );
        fs::remove_dir(blocked).unwrap();
        crate::persistence_migrations::prepare(&home.0).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(home.source()).unwrap()).unwrap()["schemaVersion"],
            3
        );
    }
}
