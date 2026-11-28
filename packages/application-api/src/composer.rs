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

/// A conversation identity that lives only in a client or in core memory and
/// never names a runtime transcript: `draft:<tab identity>` for an unsent tab,
/// `side-chat:<sequence>` for an ephemeral side chat. This module is the only
/// place that knows the encoding; clients mint and classify through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LocalConversation {
    Draft,
    SideChat,
}

impl LocalConversation {
    const DRAFT_PREFIX: &str = "draft:";
    const SIDE_CHAT_PREFIX: &str = "side-chat:";
    /// The draft that older clients shared across every unsent tab. It edits
    /// [`ComposerContextKey::NewConversation`].
    pub const LEGACY_NEW_CONVERSATION_ID: &str = "draft:new-conversation";

    pub fn draft_id(tab_identity: &str) -> String {
        format!("{}{tab_identity}", Self::DRAFT_PREFIX)
    }

    /// `None` means the ID names a runtime conversation.
    pub fn of(conversation_id: &str) -> Option<Self> {
        if conversation_id.starts_with(Self::DRAFT_PREFIX) {
            Some(Self::Draft)
        } else if conversation_id.starts_with(Self::SIDE_CHAT_PREFIX) {
            Some(Self::SideChat)
        } else {
            None
        }
    }

    pub fn side_chat_id(sequence: u64) -> String {
        format!("{}{sequence}", Self::SIDE_CHAT_PREFIX)
    }
}

/// `None` means the ID names a runtime conversation.
#[uniffi::export]
pub fn local_conversation(conversation_id: String) -> Option<LocalConversation> {
    LocalConversation::of(&conversation_id)
}

/// The conversation identity of an unsent tab.
#[uniffi::export]
pub fn draft_conversation_id(tab_identity: String) -> String {
    LocalConversation::draft_id(&tab_identity)
}

#[uniffi::export]
pub fn legacy_new_conversation_id() -> String {
    LocalConversation::LEGACY_NEW_CONVERSATION_ID.into()
}

impl ComposerContextKey {
    /// The shared input context a client conversation edits. A side chat keeps
    /// temporary input and has none; a conversation of an unknown executor
    /// (`owner_id` absent) has none yet.
    pub fn of_conversation(owner_id: Option<String>, conversation_id: String) -> Option<Self> {
        if conversation_id == LocalConversation::LEGACY_NEW_CONVERSATION_ID {
            return Some(Self::NewConversation);
        }
        if LocalConversation::of(&conversation_id) == Some(LocalConversation::SideChat) {
            return None;
        }
        owner_id.map(|owner_id| Self::Conversation {
            owner_id,
            conversation_id,
        })
    }

    /// The client conversation identity that edits this context.
    pub fn conversation_id(&self) -> &str {
        match self {
            Self::Conversation {
                conversation_id, ..
            } => conversation_id,
            Self::NewConversation => LocalConversation::LEGACY_NEW_CONVERSATION_ID,
        }
    }
}

#[uniffi::export]
pub fn composer_context_key(
    owner_id: Option<String>,
    conversation_id: String,
) -> Option<ComposerContextKey> {
    ComposerContextKey::of_conversation(owner_id, conversation_id)
}

#[uniffi::export]
pub fn composer_context_conversation_id(context: ComposerContextKey) -> String {
    context.conversation_id().into()
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
mod local_conversation_tests {
    use super::LocalConversation;

    #[test]
    fn local_identities_are_classified_and_runtime_ids_are_not() {
        assert_eq!(
            LocalConversation::of("draft:tab-1"),
            Some(LocalConversation::Draft)
        );
        assert_eq!(
            LocalConversation::of("draft:new-conversation"),
            Some(LocalConversation::Draft)
        );
        let side_chat = LocalConversation::side_chat_id(7);
        assert_eq!(side_chat, "side-chat:7");
        assert_eq!(
            LocalConversation::of(&side_chat),
            Some(LocalConversation::SideChat)
        );
        assert_eq!(LocalConversation::of("0198-native-session"), None);
        assert_eq!(LocalConversation::of("drafted"), None);
        assert_eq!(LocalConversation::draft_id("tab-1"), "draft:tab-1");
    }

    #[test]
    fn client_conversations_map_to_their_composer_contexts() {
        use super::ComposerContextKey;
        let owner = || Some("owner".to_owned());
        let legacy = LocalConversation::LEGACY_NEW_CONVERSATION_ID;
        assert_eq!(
            ComposerContextKey::of_conversation(None, legacy.into()),
            Some(ComposerContextKey::NewConversation)
        );
        assert_eq!(
            ComposerContextKey::NewConversation.conversation_id(),
            legacy
        );
        assert_eq!(
            ComposerContextKey::of_conversation(owner(), LocalConversation::side_chat_id(1)),
            None
        );
        assert_eq!(
            ComposerContextKey::of_conversation(None, "native".into()),
            None
        );
        for id in [LocalConversation::draft_id("tab-1"), "native".into()] {
            let context = ComposerContextKey::of_conversation(owner(), id.clone()).unwrap();
            assert_eq!(context.conversation_id(), id);
        }
    }
}
