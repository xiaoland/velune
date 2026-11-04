//! What a conversation's next turn goes with, and what the composer offers
//! for it. Platforms hold the user's choices and the catalogs they read; the
//! rules that turn those into options and an effective choice live here, so
//! every composer resolves them alike.
use crate::config::NewConversationChoice;
use crate::conversation::{TranscriptTurn, TurnSettings};
use crate::{
    ModelSelection, PermissionMode, ProviderSummary, RuntimeComposerChoices, RuntimeModel,
    RuntimeTypeDescriptor, TurnChoice,
};

/// What resolving the next turn depends on.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NextTurnInput {
    /// The runtime instance the next turn runs on.
    pub runtime_id: String,
    pub runtime_type: Option<RuntimeTypeDescriptor>,
    /// The providers of the runtime's gateway.
    pub providers: Vec<ProviderSummary>,
    /// The runtime's catalog once read; `None` while it is unread or could
    /// not be read.
    #[uniffi(default = None)]
    pub catalog: Option<Vec<RuntimeModel>>,
    /// The conversation's turns; none for an unsent conversation.
    #[uniffi(default = [])]
    pub turns: Vec<TranscriptTurn>,
    /// The conversation's runtime and its recorded managed model.
    #[uniffi(default = None)]
    pub conversation_runtime_id: Option<String>,
    #[uniffi(default = None)]
    pub model_record_key: Option<String>,
    /// This conversation's kept choice on the runtime.
    #[uniffi(default = None)]
    pub kept: Option<TurnChoice>,
    /// The runtime's choices across conversations.
    #[uniffi(default = None)]
    pub runtime_choices: Option<RuntimeComposerChoices>,
}

/// The next turn's options and effective choice.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NextTurn {
    pub runtime_id: String,
    /// The choice in effect: the conversation's kept one, else, on a runtime
    /// it has not run on, what the runtime's new conversations start with.
    /// An absent part follows the conversation's current one.
    pub choice: TurnChoice,
    /// The conversation ran on the runtime, so an unchosen part follows its
    /// current one; otherwise it is the runtime's default.
    pub has_current: bool,
    /// The models offered: the catalog once read, with the conversation's
    /// current model first when the catalog lists it under another ID
    /// (Claude Code lists aliases). Until then, the gateway's models without
    /// reasoning levels.
    pub models: Vec<RuntimeModel>,
    /// The model the turn goes with: the chosen one, else the conversation's
    /// current one on the runtime, else its recorded managed model. A read
    /// catalog drops those it does not offer; none is the runtime's default.
    pub model: Option<ModelSelection>,
    /// How `model` is named; absent for a managed model no longer configured.
    pub model_name: Option<String>,
    /// The chosen model is no longer offered, so the turn goes with another.
    pub chosen_model_unavailable: bool,
    /// Reasoning levels `model` accepts, in the runtime's own vocabulary.
    pub efforts: Vec<String>,
    /// The chosen level while `model` accepts it; absent keeps the
    /// conversation's current one.
    pub effort: Option<String>,
    /// The level of the conversation's last turn, while it ran on the runtime
    /// and `model` accepts it.
    pub current_effort: Option<String>,
    /// Modes the runtime type declares.
    pub permission_modes: Vec<PermissionMode>,
    /// The chosen mode while the runtime offers it; absent keeps the
    /// conversation's current one, which is the session's state in the runtime.
    pub permission_mode: Option<PermissionMode>,
    /// The mode of the conversation's last turn, while it ran on the runtime.
    pub current_permission_mode: Option<PermissionMode>,
}

/// A change to one part of the next turn's choice; `None` follows the
/// conversation's current one.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum TurnChoiceChange {
    Model { selection: Option<ModelSelection> },
    Effort { level: Option<String> },
    PermissionMode { mode: Option<String> },
}

/// A changed choice to keep for the conversation, and the runtime's choices
/// when they change too.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TurnChoiceUpdate {
    pub choice: TurnChoice,
    pub runtime_choices: Option<RuntimeComposerChoices>,
}

#[uniffi::export]
pub fn resolve_next_turn(input: NextTurnInput) -> NextTurn {
    let NextTurnInput {
        runtime_id,
        runtime_type,
        providers,
        catalog,
        turns,
        conversation_runtime_id,
        model_record_key,
        kept,
        runtime_choices,
    } = input;
    let ready = catalog.is_some();
    let catalog = catalog.unwrap_or_else(|| managed_models(&providers));
    let has_current = turns
        .iter()
        .any(|turn| turn_runtime(turn) == Some(runtime_id.as_str()));
    let choice = kept.unwrap_or_else(|| match runtime_choices {
        Some(choices) if !has_current => match choices.new_conversation {
            NewConversationChoice::LastChosen => choices.last_chosen,
            NewConversationChoice::Chosen { choice } => choice,
        },
        _ => TurnChoice::default(),
    });
    let default_efforts = if runtime_type
        .as_ref()
        .is_some_and(|kind| kind.session_options)
    {
        Vec::new()
    } else {
        let mut levels = Vec::new();
        for model in &catalog {
            if matches!(model.selection, ModelSelection::Native { .. }) {
                for level in &model.reasoning_efforts {
                    if !levels.contains(level) {
                        levels.push(level.clone());
                    }
                }
            }
        }
        levels
    };
    let latest = latest_turn_model(&turns, &runtime_id, &catalog, &providers, ready);
    let mut models = catalog;
    if ready
        && let Some(current @ ModelSelection::Native { model_id, .. }) = &latest
        && !models.iter().any(|model| &model.selection == current)
    {
        models.insert(
            0,
            RuntimeModel {
                selection: current.clone(),
                name: format!("{model_id}（会话当前）"),
                reasoning_efforts: default_efforts.clone(),
            },
        );
    }
    let offered =
        |selection: &ModelSelection| models.iter().find(|model| &model.selection == selection);
    let recorded = (conversation_runtime_id.as_deref() == Some(runtime_id.as_str()))
        .then_some(model_record_key)
        .flatten()
        .map(|key| ModelSelection::Managed {
            model_record_key: key,
        });
    let model = [choice.model.clone(), latest, recorded]
        .into_iter()
        .flatten()
        .find(|candidate| !ready || offered(candidate).is_some());
    let chosen_model_unavailable = ready
        && choice
            .model
            .as_ref()
            .is_some_and(|chosen| offered(chosen).is_none());
    let model_name = model.as_ref().and_then(|model| match offered(model) {
        Some(offered) => Some(offered.name.clone()),
        None => match model {
            ModelSelection::Native { model_id, .. } => Some(model_id.clone()),
            ModelSelection::Managed { .. } => None,
        },
    });
    let efforts = match &model {
        None => default_efforts,
        Some(model) => offered(model)
            .map(|model| model.reasoning_efforts.clone())
            .unwrap_or_default(),
    };
    let current = turns
        .last()
        .and_then(|turn| turn.settings.as_ref())
        .filter(|settings| settings.runtime_id.as_deref() == Some(runtime_id.as_str()));
    let accepted = |level: Option<&String>| level.filter(|level| efforts.contains(level)).cloned();
    let effort = accepted(choice.effort.as_ref());
    let current_effort = accepted(current.and_then(|settings| settings.effort.as_ref()));
    let permission_modes = runtime_type
        .map(|kind| kind.permission_modes)
        .unwrap_or_default();
    let mode = |id: Option<&String>| {
        id.and_then(|id| permission_modes.iter().find(|mode| &mode.id == id))
            .cloned()
    };
    let permission_mode = mode(choice.permission_mode.as_ref());
    let current_permission_mode =
        mode(current.and_then(|settings| settings.permission_mode.as_ref()));
    NextTurn {
        runtime_id,
        choice,
        has_current,
        models,
        model,
        model_name,
        chosen_model_unavailable,
        efforts,
        effort,
        current_effort,
        permission_modes,
        permission_mode,
        current_permission_mode,
    }
}

/// Applies a change the next turn offers. A part chosen, or one left to a
/// new conversation's runtime default, also becomes what the runtime offers
/// first next time; following a conversation's current one is no choice.
/// `None` when the change chooses something not offered.
#[uniffi::export]
pub fn change_next_turn(
    next: NextTurn,
    runtime_choices: Option<RuntimeComposerChoices>,
    change: TurnChoiceChange,
) -> Option<TurnChoiceUpdate> {
    let chosen = match &change {
        TurnChoiceChange::Model { selection } => {
            if selection.as_ref().is_some_and(|model| {
                !next
                    .models
                    .iter()
                    .any(|offered| &offered.selection == model)
            }) {
                return None;
            }
            selection.is_some()
        }
        TurnChoiceChange::Effort { level } => {
            if level
                .as_ref()
                .is_some_and(|level| !next.efforts.contains(level))
            {
                return None;
            }
            level.is_some()
        }
        TurnChoiceChange::PermissionMode { mode } => {
            if mode.as_ref().is_some_and(|mode| {
                !next
                    .permission_modes
                    .iter()
                    .any(|offered| &offered.id == mode)
            }) {
                return None;
            }
            mode.is_some()
        }
    };
    let choice = changed(next.choice, &change);
    let runtime_choices = (chosen || !next.has_current).then(|| {
        let mut choices = runtime_choices.unwrap_or_else(|| RuntimeComposerChoices {
            runtime_id: next.runtime_id.clone(),
            last_chosen: TurnChoice::default(),
            new_conversation: NewConversationChoice::LastChosen,
        });
        choices.last_chosen = changed(choices.last_chosen, &change);
        choices
    });
    Some(TurnChoiceUpdate {
        choice,
        runtime_choices,
    })
}

fn changed(mut choice: TurnChoice, change: &TurnChoiceChange) -> TurnChoice {
    match change {
        TurnChoiceChange::Model { selection } => choice.model = selection.clone(),
        TurnChoiceChange::Effort { level } => choice.effort = level.clone(),
        TurnChoiceChange::PermissionMode { mode } => choice.permission_mode = mode.clone(),
    }
    choice
}

fn turn_runtime(turn: &TranscriptTurn) -> Option<&str> {
    turn.settings.as_ref()?.runtime_id.as_deref()
}

/// The gateway's models, named as the runtime's catalog names them.
fn managed_models(providers: &[ProviderSummary]) -> Vec<RuntimeModel> {
    providers
        .iter()
        .flat_map(|provider| {
            provider.models.iter().map(|model| RuntimeModel {
                selection: ModelSelection::Managed {
                    model_record_key: model.record_key.clone(),
                },
                name: format!("{} · {}", model.display_name(), provider.name),
                reasoning_efforts: Vec::new(),
            })
        })
        .collect()
}

/// The model of the conversation's latest turn on the runtime: the one
/// Velune sent it with, else as the record names it among the models
/// offered. A record names the model Velune sent, the native model ID, or a
/// provider's model ID; the last counts only when one provider model has it.
/// A native ID the read catalog does not list is the runtime's own model
/// where its models take no provider.
fn latest_turn_model(
    turns: &[TranscriptTurn],
    runtime_id: &str,
    catalog: &[RuntimeModel],
    providers: &[ProviderSummary],
    ready: bool,
) -> Option<ModelSelection> {
    let settings: &TurnSettings = turns.iter().rev().find_map(|turn| {
        turn.settings.as_ref().filter(|settings| {
            settings.runtime_id.as_deref() == Some(runtime_id)
                && (settings.model.is_some() || settings.model_selection.is_some())
        })
    })?;
    if let Some(selection) = &settings.model_selection {
        return Some(selection.clone());
    }
    let recorded = settings.model.as_deref()?;
    if let Some(model) = catalog.iter().find(|model| model.name == recorded) {
        return Some(model.selection.clone());
    }
    if let Some(model) = catalog.iter().find(
        |model| matches!(&model.selection, ModelSelection::Native { model_id, .. } if model_id == recorded),
    ) {
        return Some(model.selection.clone());
    }
    let keys: Vec<&str> = providers
        .iter()
        .flat_map(|provider| &provider.models)
        .filter(|model| model.provider_model_id == recorded)
        .map(|model| model.record_key.as_str())
        .collect();
    let mut managed = catalog.iter().filter(|model| {
        matches!(&model.selection, ModelSelection::Managed { model_record_key } if keys.contains(&model_record_key.as_str()))
    });
    if let (Some(only), None) = (managed.next(), managed.next()) {
        return Some(only.selection.clone());
    }
    let mut native = catalog
        .iter()
        .filter_map(|model| match &model.selection {
            ModelSelection::Native { provider_id, .. } => Some(provider_id),
            ModelSelection::Managed { .. } => None,
        })
        .peekable();
    (ready && native.peek().is_some() && native.all(Option::is_none)).then(|| {
        ModelSelection::Native {
            provider_id: None,
            model_id: recorded.to_owned(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProviderModel, RuntimeKind};

    fn native(id: &str) -> ModelSelection {
        ModelSelection::Native {
            provider_id: None,
            model_id: id.into(),
        }
    }

    fn managed(key: &str) -> ModelSelection {
        ModelSelection::Managed {
            model_record_key: key.into(),
        }
    }

    fn model(selection: ModelSelection, name: &str, efforts: &[&str]) -> RuntimeModel {
        RuntimeModel {
            selection,
            name: name.into(),
            reasoning_efforts: efforts.iter().map(|level| level.to_string()).collect(),
        }
    }

    fn turn(settings: TurnSettings) -> TranscriptTurn {
        TranscriptTurn {
            id: "turn".into(),
            user_message_id: "user".into(),
            work_message_ids: Vec::new(),
            last_message_id: None,
            reply_start_block: 0,
            duration_ms: None,
            is_running: false,
            outcome: None,
            settings: Some(settings),
            usage: None,
        }
    }

    fn on(runtime: &str, model: &str) -> TurnSettings {
        TurnSettings {
            runtime_id: Some(runtime.into()),
            model: Some(model.into()),
            ..TurnSettings::default()
        }
    }

    fn provider() -> ProviderSummary {
        ProviderSummary {
            id: "provider".into(),
            name: "Provider".into(),
            protocol: crate::Protocol::ChatCompletions,
            endpoint: "https://example.invalid".into(),
            execution_backend: crate::ExecutionBackend::Http,
            request_profile: crate::RequestProfile::Native,
            authentication_source: crate::ProviderAuthenticationSource::ApiKey,
            quota_source: crate::ProviderQuotaSource::None,
            authentication: crate::ProviderAuthenticationDescription {
                method: crate::AuthenticationMethod::ApiKey,
                configured: true,
                account_label: None,
                provenance: None,
                actions: Vec::new(),
                disclosure: None,
                linked_login: None,
            },
            quota_authentication: crate::QuotaAuthenticationDescription {
                configured: false,
                binding_confirmed: false,
            },
            quota_limits: Vec::new(),
            models: vec![ProviderModel {
                record_key: "key".into(),
                provider_model_id: "gpt".into(),
                nickname: String::new(),
                icon: None,
                context_window: None,
                max_output_tokens: None,
                reasoning_levels: None,
                request_parameters: None,
            }],
        }
    }

    fn input(catalog: Option<Vec<RuntimeModel>>, turns: Vec<TranscriptTurn>) -> NextTurnInput {
        NextTurnInput {
            runtime_id: "runtime".into(),
            runtime_type: None,
            providers: vec![provider()],
            catalog,
            turns,
            conversation_runtime_id: Some("runtime".into()),
            model_record_key: None,
            kept: None,
            runtime_choices: None,
        }
    }

    #[test]
    fn an_unread_catalog_offers_the_gateway_models_and_keeps_the_choice() {
        let mut unread = input(None, Vec::new());
        unread.kept = Some(TurnChoice {
            model: Some(native("elsewhere")),
            effort: Some("high".into()),
            permission_mode: None,
        });
        let next = resolve_next_turn(unread);
        assert_eq!(
            next.models,
            vec![model(managed("key"), "gpt · Provider", &[])]
        );
        assert_eq!(next.model, Some(native("elsewhere")));
        assert_eq!(next.model_name.as_deref(), Some("elsewhere"));
        assert!(!next.chosen_model_unavailable);
        // Levels are known only once read; the chosen one waits.
        assert_eq!(next.effort, None);
        assert_eq!(next.choice.effort.as_deref(), Some("high"));
    }

    #[test]
    fn a_current_model_the_catalog_lists_under_an_alias_comes_first() {
        let catalog = vec![model(native("opus"), "Opus", &["low", "high"])];
        let next = resolve_next_turn(input(
            Some(catalog),
            vec![turn(on("runtime", "claude-opus-4"))],
        ));
        assert_eq!(next.models[0].name, "claude-opus-4（会话当前）");
        assert_eq!(next.models[0].reasoning_efforts, ["low", "high"]);
        assert_eq!(next.model, Some(native("claude-opus-4")));
        assert!(next.has_current);
    }

    #[test]
    fn a_recorded_provider_model_id_names_its_only_managed_model() {
        let catalog = vec![model(managed("key"), "gpt · Provider", &["medium"])];
        let next = resolve_next_turn(input(Some(catalog), vec![turn(on("runtime", "gpt"))]));
        assert_eq!(next.model, Some(managed("key")));
        assert_eq!(next.efforts, ["medium"]);
    }

    #[test]
    fn a_chosen_model_no_longer_offered_gives_way_to_the_current_one() {
        let mut read = input(
            Some(vec![
                model(native("a"), "A", &[]),
                model(native("b"), "B", &[]),
            ]),
            vec![turn(on("runtime", "b"))],
        );
        read.kept = Some(TurnChoice {
            model: Some(native("gone")),
            ..TurnChoice::default()
        });
        let next = resolve_next_turn(read);
        assert!(next.chosen_model_unavailable);
        assert_eq!(next.model, Some(native("b")));
    }

    #[test]
    fn a_runtime_without_turns_starts_from_its_new_conversation_choice() {
        let mut fresh = input(Some(Vec::new()), vec![turn(on("other", "x"))]);
        fresh.runtime_choices = Some(RuntimeComposerChoices {
            runtime_id: "runtime".into(),
            last_chosen: TurnChoice {
                effort: Some("low".into()),
                ..TurnChoice::default()
            },
            new_conversation: NewConversationChoice::LastChosen,
        });
        let next = resolve_next_turn(fresh.clone());
        assert!(!next.has_current);
        assert_eq!(next.choice.effort.as_deref(), Some("low"));
        fresh.turns.push(turn(on("runtime", "x")));
        assert_eq!(resolve_next_turn(fresh).choice, TurnChoice::default());
    }

    #[test]
    fn session_options_leave_the_default_model_without_levels() {
        let mut acp = input(Some(vec![model(native("a"), "A", &["high"])]), Vec::new());
        assert_eq!(resolve_next_turn(acp.clone()).efforts, ["high"]);
        acp.runtime_type = Some(RuntimeTypeDescriptor {
            id: RuntimeKind::Acp,
            family_id: String::new(),
            compatible_versions: String::new(),
            can_rename_conversations: false,
            can_delete_conversations: false,
            launch_options: true,
            agent_info: true,
            session_options: true,
            native_account_quota: false,
            remote_resources: false,
            supported_protocols: Vec::new(),
            supported_provider_protocols: Vec::new(),
            name: String::new(),
            fields: Vec::new(),
            permission_modes: Vec::new(),
        });
        assert!(resolve_next_turn(acp).efforts.is_empty());
    }

    #[test]
    fn following_the_current_part_is_no_choice_for_the_runtime() {
        let next = resolve_next_turn(input(
            Some(vec![model(native("a"), "A", &["high"])]),
            vec![turn(on("runtime", "a"))],
        ));
        let follow = change_next_turn(next.clone(), None, TurnChoiceChange::Effort { level: None })
            .expect("offered");
        assert_eq!(follow.runtime_choices, None);
        let chosen = change_next_turn(
            next.clone(),
            None,
            TurnChoiceChange::Effort {
                level: Some("high".into()),
            },
        )
        .expect("offered");
        assert_eq!(chosen.choice.effort.as_deref(), Some("high"));
        assert_eq!(
            chosen
                .runtime_choices
                .map(|choices| choices.last_chosen.effort),
            Some(Some("high".into()))
        );
        assert_eq!(
            change_next_turn(
                next,
                None,
                TurnChoiceChange::Effort {
                    level: Some("max".into())
                }
            ),
            None
        );
    }
}
