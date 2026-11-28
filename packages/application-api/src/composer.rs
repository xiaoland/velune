//! Shared unfinished input. Client navigation and execution targets are separate.
use crate::TurnChoice;
use serde::{Deserialize, Serialize};

/// A source conversation, a stable `draft:<tab identity>`, or a legacy
/// unscoped new-conversation target (`None`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerTargetRequest {
    pub runtime_id: String,
    pub conversation_id: Option<String>,
}

/// The execution identity of a target without its conversation payload.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerTargetIdentity {
    pub owner_id: String,
    pub context: ComposerContextKey,
    pub default_runtime_id: String,
}

/// Execution facts used by the facade before and after consulting the input authority.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerTargetBasis {
    pub request: ComposerTargetRequest,
    pub owner_id: String,
    pub context: ComposerContextKey,
    pub default_runtime_id: String,
    pub input: crate::NextTurnInput,
}

/// The authority record and its execution projection, independent of an InputID.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PreparedComposerTarget {
    pub request: ComposerTargetRequest,
    pub owner_id: String,
    pub context: ComposerContextKey,
    pub target: ComposerTarget,
    pub default_runtime_id: String,
    pub basis: crate::NextTurnInput,
    pub next_turn: crate::NextTurn,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ComposerTargetChoiceUpdate {
    pub choices: ComposerTargetChoices,
    pub runtime_choices: Option<crate::RuntimeComposerChoices>,
}

/// Unselected fields follow current facts instead of freezing an observed default.
#[uniffi::export]
pub fn normalize_composer_target_choices(choices: ComposerTargetChoices) -> ComposerTargetChoices {
    if choices.choice_edited {
        choices
    } else {
        ComposerTargetChoices::default()
    }
}

#[uniffi::export]
pub fn resolve_composer_target(
    prepared: PreparedComposerTarget,
    choices: Option<ComposerTargetChoices>,
    catalog: Option<Vec<crate::RuntimeModel>>,
) -> Option<crate::NextTurn> {
    let choices = normalize_composer_target_choices(choices.unwrap_or(prepared.target.choices));
    let runtime = choices
        .runtime_id
        .as_ref()
        .unwrap_or(&prepared.default_runtime_id);
    if *runtime != prepared.basis.runtime_id
        || choices
            .runtime_owner_id
            .as_ref()
            .is_some_and(|owner| owner != &prepared.owner_id)
    {
        return None;
    }
    let mut input = prepared.basis;
    input.kept = choices
        .choices
        .iter()
        .find(|choice| choice.owner_id == prepared.owner_id && choice.runtime_id == *runtime)
        .map(|choice| choice.choice.clone());
    input.catalog = catalog;
    Some(crate::resolve_next_turn(input))
}

#[uniffi::export]
pub fn change_composer_target(
    mut next: crate::NextTurn,
    target_choices: ComposerTargetChoices,
    owner_id: String,
    runtime_choices: Option<crate::RuntimeComposerChoices>,
    change: crate::TurnChoiceChange,
) -> Option<ComposerTargetChoiceUpdate> {
    let mut choices = normalize_composer_target_choices(target_choices);
    next.choice = choices
        .choices
        .iter()
        .find(|choice| choice.owner_id == owner_id && choice.runtime_id == next.runtime_id)
        .map(|choice| choice.choice.clone())
        .unwrap_or_default();
    let runtime_id = next.runtime_id.clone();
    let update = crate::change_next_turn(next, runtime_choices, change)?;
    choices
        .choices
        .retain(|choice| choice.owner_id != owner_id || choice.runtime_id != runtime_id);
    if !update.choice.is_empty() {
        choices.choices.push(ComposerRuntimeChoice {
            owner_id,
            runtime_id,
            choice: update.choice,
        });
    }
    choices.choice_edited = true;
    Some(ComposerTargetChoiceUpdate {
        choices,
        runtime_choices: update.runtime_choices,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ComposerContextKey {
    Conversation {
        owner_id: String,
        conversation_id: String,
    },
    NewConversation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerAttachment {
    pub id: String,
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerAttachmentContent {
    pub attachment: ComposerAttachment,
    #[serde(with = "crate::bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerRuntimeChoice {
    pub owner_id: String,
    pub runtime_id: String,
    pub choice: TurnChoice,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerInputContent {
    pub text: String,
    pub attachments: Vec<ComposerAttachment>,
}

/// Execution choices belong to a target, independently of borrowed input content.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerTargetChoices {
    pub runtime_id: Option<String>,
    pub runtime_owner_id: Option<String>,
    pub choices: Vec<ComposerRuntimeChoice>,
    pub choice_edited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerTarget {
    pub context: ComposerContextKey,
    pub revision: u64,
    pub choices: ComposerTargetChoices,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerInput {
    pub id: String,
    pub revision: u64,
    pub content: ComposerInputContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerInputBinding {
    pub context: ComposerContextKey,
    pub input_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerCreation {
    pub input_id: String,
    pub owner_id: String,
    pub runtime_id: String,
    pub conversation_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerInputState {
    /// Stable execution-application identity, independent of the viewing client.
    pub owner_id: String,
    pub revision: u64,
    pub inputs: Vec<ComposerInput>,
    pub bindings: Vec<ComposerInputBinding>,
    pub targets: Vec<ComposerTarget>,
    /// Native identity handoffs, including completed operations, for idempotent recovery.
    pub creations: Vec<ComposerCreation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ComposerInputUpdate {
    pub revision: u64,
    /// None means the caller already has this exact directory revision.
    pub state: Option<ComposerInputState>,
}

#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(
    tag = "operation",
    content = "parameters",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ComposerInputOperation {
    Read {
        since_revision: Option<u64>,
    },
    /// Resolve once. Reopening a context must not reinitialize its content.
    Ensure {
        context: ComposerContextKey,
        input_id: String,
        seed: ComposerInputContent,
    },
    /// None creates a new identity; Some updates exactly the observed revision.
    Save {
        input_id: String,
        expected_revision: Option<u64>,
        content: ComposerInputContent,
    },
    EnsureTarget {
        context: ComposerContextKey,
        seed: ComposerTargetChoices,
    },
    SaveTarget {
        context: ComposerContextKey,
        expected_revision: Option<u64>,
        choices: ComposerTargetChoices,
    },
    Bind {
        context: ComposerContextKey,
        input_id: String,
        expected_revision: Option<u64>,
    },
    /// The old new-conversation input becomes this native session's input.
    Promote {
        input_id: String,
        owner_id: String,
        conversation_id: String,
        runtime_id: String,
    },
    Forget {
        context: ComposerContextKey,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelSelection, NextTurn, NextTurnInput, RuntimeModel, TurnChoiceChange};

    fn model(id: &str) -> ModelSelection {
        ModelSelection::Native {
            provider_id: None,
            model_id: id.into(),
        }
    }

    fn catalog() -> Vec<RuntimeModel> {
        ["current", "selected"]
            .into_iter()
            .map(|id| RuntimeModel {
                selection: model(id),
                name: id.into(),
                reasoning_efforts: vec!["low".into(), "high".into()],
            })
            .collect()
    }

    fn next() -> NextTurn {
        NextTurn {
            runtime_id: "runtime".into(),
            runtime_type: Some(crate::RuntimeKind::Codex166),
            choice: TurnChoice {
                model: Some(model("current")),
                effort: Some("low".into()),
                permission_mode: None,
            },
            has_current: true,
            models: catalog(),
            model: Some(model("current")),
            model_name: Some("current".into()),
            chosen_model_unavailable: false,
            efforts: vec!["low".into(), "high".into()],
            effort: Some("low".into()),
            current_effort: Some("low".into()),
            permission_modes: vec![],
            permission_mode: None,
            current_permission_mode: None,
        }
    }

    fn prepared() -> PreparedComposerTarget {
        PreparedComposerTarget {
            request: ComposerTargetRequest {
                runtime_id: "runtime".into(),
                conversation_id: Some("conversation".into()),
            },
            owner_id: "owner".into(),
            context: ComposerContextKey::NewConversation,
            target: ComposerTarget {
                context: ComposerContextKey::NewConversation,
                revision: 7,
                choices: Default::default(),
            },
            default_runtime_id: "runtime".into(),
            next_turn: next(),
            basis: NextTurnInput {
                runtime_id: "runtime".into(),
                runtime_type: None,
                providers: vec![],
                catalog: None,
                turn_settings: vec![],
                conversation_runtime_id: None,
                model_record_key: None,
                kept: None,
                runtime_choices: None,
                new_conversation: crate::config::NewConversationChoice::Chosen {
                    choice: TurnChoice {
                        model: Some(model("current")),
                        effort: Some("low".into()),
                        permission_mode: None,
                    },
                },
            },
        }
    }

    fn scoped(owner: &str, runtime: &str) -> ComposerRuntimeChoice {
        ComposerRuntimeChoice {
            owner_id: owner.into(),
            runtime_id: runtime.into(),
            choice: TurnChoice {
                model: Some(model("selected")),
                effort: Some("high".into()),
                permission_mode: None,
            },
        }
    }

    #[test]
    fn unedited_target_ignores_stale_observed_owner_runtime_and_choices() {
        let stale = ComposerTargetChoices {
            runtime_id: Some("old-runtime".into()),
            runtime_owner_id: Some("old-owner".into()),
            choices: vec![scoped("owner", "runtime")],
            choice_edited: false,
        };
        assert_eq!(
            normalize_composer_target_choices(stale.clone()),
            ComposerTargetChoices::default()
        );
        let resolved = resolve_composer_target(prepared(), Some(stale), Some(catalog())).unwrap();
        assert_eq!(resolved.model, Some(model("current")));
        assert_eq!(resolved.effort.as_deref(), Some("low"));
    }

    #[test]
    fn target_resolution_requires_the_prepared_owner_runtime_and_scoped_choice() {
        let valid = ComposerTargetChoices {
            runtime_id: Some("runtime".into()),
            runtime_owner_id: Some("owner".into()),
            choices: vec![
                scoped("another-owner", "runtime"),
                scoped("owner", "runtime"),
            ],
            choice_edited: true,
        };
        assert_eq!(normalize_composer_target_choices(valid.clone()), valid);
        let resolved =
            resolve_composer_target(prepared(), Some(valid.clone()), Some(catalog())).unwrap();
        assert_eq!(resolved.model, Some(model("selected")));
        assert_eq!(resolved.effort.as_deref(), Some("high"));
        for wrong_owner in [false, true] {
            let mut stale = valid.clone();
            if wrong_owner {
                stale.runtime_owner_id = Some("another-owner".into());
            } else {
                stale.runtime_id = Some("another-runtime".into());
            }
            assert!(resolve_composer_target(prepared(), Some(stale), Some(catalog())).is_none());
        }
        for row in [
            scoped("another-owner", "runtime"),
            scoped("owner", "another-runtime"),
        ] {
            let mut choices = valid.clone();
            choices.choices = vec![row];
            let resolved =
                resolve_composer_target(prepared(), Some(choices), Some(catalog())).unwrap();
            assert_eq!(resolved.model, Some(model("current")));
            assert_eq!(resolved.effort.as_deref(), Some("low"));
        }
    }

    #[test]
    fn target_changes_keep_other_scopes_without_freezing_observed_defaults() {
        let other = vec![
            scoped("another-owner", "runtime"),
            scoped("owner", "another-runtime"),
        ];
        let choices = ComposerTargetChoices {
            runtime_id: Some("runtime".into()),
            runtime_owner_id: Some("owner".into()),
            choices: other.clone(),
            choice_edited: true,
        };
        let update = change_composer_target(
            next(),
            choices,
            "owner".into(),
            None,
            TurnChoiceChange::Effort {
                level: Some("high".into()),
            },
        )
        .unwrap();
        assert_eq!(&update.choices.choices[..2], &other);
        assert_eq!(
            update.choices.choices[2],
            ComposerRuntimeChoice {
                owner_id: "owner".into(),
                runtime_id: "runtime".into(),
                choice: TurnChoice {
                    model: None,
                    effort: Some("high".into()),
                    permission_mode: None,
                },
            }
        );
        assert_eq!(update.runtime_choices.unwrap().last_chosen.model, None);
        assert!(update.choices.choice_edited);
        let cleared = change_composer_target(
            next(),
            update.choices,
            "owner".into(),
            None,
            TurnChoiceChange::Effort { level: None },
        )
        .unwrap();
        assert_eq!(cleared.choices.choices, other);
        assert_eq!(cleared.choices.runtime_id.as_deref(), Some("runtime"));
        assert_eq!(cleared.choices.runtime_owner_id.as_deref(), Some("owner"));
    }
}
