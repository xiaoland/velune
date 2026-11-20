//! Known format conversion for the shared input catalog.
use super::{CURRENT_SCHEMA, Stored, decode_stored, validate_state};
use crate::{
    Error,
    persistence_migrations::{Change, only_known_json_keys, read_optional},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
use velune_application_api::{
    ComposerAttachment, ComposerCreation, ComposerInput, ComposerInputBinding,
    ComposerInputContent, ComposerInputState, ComposerRuntimeChoice, ComposerTarget,
    ComposerTargetChoices,
};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogV1 {
    schema_version: u32,
    state: StateV1,
    attachments: Vec<ComposerAttachment>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateV1 {
    owner_id: String,
    revision: u64,
    inputs: Vec<InputV1>,
    bindings: Vec<ComposerInputBinding>,
    creations: Vec<ComposerCreation>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputV1 {
    id: String,
    revision: u64,
    content: ContentV1,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContentV1 {
    text: String,
    attachments: Vec<ComposerAttachment>,
    runtime_id: Option<String>,
    runtime_owner_id: Option<String>,
    choices: Vec<ComposerRuntimeChoice>,
    choice_edited: bool,
}

pub(crate) fn prepare_migration(home: &Path, changes: &mut Vec<Change>) -> Result<(), Error> {
    let path = home.join("composer-inputs.json");
    let Some(original) = read_optional(&path)? else {
        return guard_missing_catalog(home);
    };
    let value: Value = serde_json::from_slice(&original)?;
    match value["schemaVersion"].as_u64() {
        Some(version) if version == u64::from(CURRENT_SCHEMA) => {
            decode_stored(value)?;
            return Ok(());
        }
        Some(1) => {}
        _ => {
            return Err(Error::invalid(
                "composer-inputs.json schema 不受支持；原文件保留",
            ));
        }
    }

    let legacy: CatalogV1 = serde_json::from_value(value.clone()).map_err(|error| {
        Error::context(
            "旧 composer 无法安全转换；原文件保留。已知 pre-beta 格式请参照数据迁移说明",
            error,
        )
    })?;
    if !only_known_json_keys(&value, &serde_json::to_value(&legacy)?) {
        return Err(Error::invalid(
            "旧 composer 含无法保留的未知字段；原文件保留",
        ));
    }
    let StateV1 {
        owner_id,
        revision,
        inputs,
        bindings,
        creations,
    } = legacy.state;
    let mut choices = BTreeMap::new();
    let mut content = Vec::with_capacity(inputs.len());
    for InputV1 {
        id,
        revision,
        content: old,
    } in inputs
    {
        let choice = ComposerTargetChoices {
            runtime_id: old.runtime_id,
            runtime_owner_id: old.runtime_owner_id,
            choices: old.choices,
            choice_edited: old.choice_edited,
        };
        if choices.insert(id.clone(), choice).is_some() {
            return Err(Error::invalid(
                "旧 composer 存在重复 input 身份；原文件保留",
            ));
        }
        content.push(ComposerInput {
            id,
            revision,
            content: ComposerInputContent {
                text: old.text,
                attachments: old.attachments,
            },
        });
    }
    let targets = bindings
        .iter()
        .map(|binding| {
            Ok(ComposerTarget {
                context: binding.context.clone(),
                revision: 1,
                choices: choices.get(&binding.input_id).cloned().ok_or_else(|| {
                    Error::invalid("旧 composer binding 指向缺失输入；原文件保留")
                })?,
            })
        })
        .collect::<Result<_, Error>>()?;
    let migrated = Stored {
        schema_version: CURRENT_SCHEMA,
        state: ComposerInputState {
            owner_id,
            revision: revision
                .checked_add(1)
                .ok_or_else(|| Error::invalid("旧 composer 修订无法推进；原文件保留"))?,
            inputs: content,
            bindings,
            targets,
            creations,
        },
        attachments: legacy.attachments,
    };
    validate_state(&migrated)?;
    changes.push(Change {
        path,
        original,
        replacement: serde_json::to_vec_pretty(&migrated)?,
    });
    Ok(())
}

/// Early shared drafts must be imported before a new catalog can be created.
pub(super) fn guard_missing_catalog(home: &Path) -> Result<(), Error> {
    let has_drafts = home.join("composer-drafts.json").try_exists()?;
    let import_checkpoint = home
        .join("composer-inputs.schema1.backup.json")
        .try_exists()?;
    let legacy_choices = read_optional(&home.join("composer-choices.json"))?
        .map(|bytes| serde_json::from_slice::<Value>(&bytes))
        .transpose()?
        .is_some_and(|value| {
            value["schemaVersion"].as_u64() == Some(1)
                || value
                    .pointer("/choices/conversations")
                    .and_then(Value::as_array)
                    .is_some_and(|conversations| !conversations.is_empty())
        });
    if has_drafts || import_checkpoint || legacy_choices {
        return Err(Error::invalid(
            "检测到旧共享输入；原文件保留。请退出 Velune 后参照数据迁移说明运行 scripts/run-migrations.py",
        ));
    }
    Ok(())
}
