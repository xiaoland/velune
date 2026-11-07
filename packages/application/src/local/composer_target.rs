//! Execution facts for the shared target-preparation use case.
use super::*;
use velune_application_api::{
    ComposerContextKey, ComposerTargetBasis, ComposerTargetChoices, ComposerTargetIdentity,
    ComposerTargetRequest, NextTurnInput, normalize_composer_target_choices,
};

impl CoreRuntime {
    fn composer_target_snapshot(
        &mut self,
        request: &ComposerTargetRequest,
    ) -> Result<Option<crate::conversation::ConversationSnapshot>, RuntimeError> {
        match &request.conversation_id {
            Some(id) => {
                if id.starts_with("side-chat:") {
                    return Err(RuntimeError::invalid("侧聊使用自己的临时执行选择"));
                }
                Ok(Some(
                    self.queried_snapshot(&request.runtime_id, id)?
                        .ok_or_else(|| RuntimeError::invalid("目标会话尚不可用"))?,
                ))
            }
            None => Ok(None),
        }
    }

    pub(crate) fn composer_target_identity(
        &mut self,
        request: ComposerTargetRequest,
        owner_id: String,
    ) -> Result<ComposerTargetIdentity, RuntimeError> {
        let snapshot = self.composer_target_snapshot(&request)?;
        Ok(target_identity(&request, owner_id, snapshot.as_ref()))
    }

    pub(crate) fn composer_target_basis(
        &mut self,
        request: ComposerTargetRequest,
        owner_id: String,
        choices: Option<ComposerTargetChoices>,
    ) -> Result<ComposerTargetBasis, RuntimeError> {
        let snapshot = self.composer_target_snapshot(&request)?;
        let ComposerTargetIdentity {
            context,
            default_runtime_id,
            ..
        } = target_identity(&request, owner_id.clone(), snapshot.as_ref());
        let choices = normalize_composer_target_choices(choices.unwrap_or_default());
        if choices
            .runtime_owner_id
            .as_ref()
            .is_some_and(|owner| owner != &owner_id)
        {
            return Err(RuntimeError::invalid("所选运行时位于另一设备，输入已保留"));
        }
        let runtime_id = choices.runtime_id.as_ref().unwrap_or(&default_runtime_id);
        let runtime = self.runtime_instance(runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("所选运行时已停用，输入已保留"));
        }
        let providers = self
            .public_gateways()
            .into_iter()
            .find(|gateway| gateway.id == runtime.gateway_id)
            .map(|gateway| gateway.providers)
            .ok_or_else(|| RuntimeError::invalid("所选运行时的网关不可用"))?;
        let input = NextTurnInput {
            runtime_id: runtime.id.clone(),
            runtime_type: runtime_types()
                .into_iter()
                .find(|kind| kind.id == runtime.type_id),
            providers,
            catalog: None,
            turn_settings: snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .transcript_turns
                        .iter()
                        .filter_map(|turn| turn.settings.clone())
                        .collect()
                })
                .unwrap_or_default(),
            conversation_runtime_id: snapshot
                .as_ref()
                .map(|snapshot| snapshot.context_runtime_id.clone()),
            model_record_key: snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.model_record_key.clone()),
            kept: choices
                .choices
                .iter()
                .find(|choice| choice.owner_id == owner_id && choice.runtime_id == runtime.id)
                .map(|choice| choice.choice.clone()),
            runtime_choices: self
                .composer_choices
                .runtimes
                .iter()
                .find(|choice| choice.runtime_id == runtime.id)
                .cloned(),
            new_conversation: runtime.new_conversation_choice()?,
        };
        Ok(ComposerTargetBasis {
            request,
            owner_id,
            context,
            default_runtime_id,
            input,
        })
    }
}

fn target_identity(
    request: &ComposerTargetRequest,
    owner_id: String,
    snapshot: Option<&crate::conversation::ConversationSnapshot>,
) -> ComposerTargetIdentity {
    let context =
        request
            .conversation_id
            .as_ref()
            .map_or(ComposerContextKey::NewConversation, |id| {
                ComposerContextKey::Conversation {
                    owner_id: owner_id.clone(),
                    conversation_id: id.clone(),
                }
            });
    ComposerTargetIdentity {
        owner_id,
        context,
        default_runtime_id: snapshot
            .map(|snapshot| snapshot.context_runtime_id.clone())
            .unwrap_or_else(|| request.runtime_id.clone()),
    }
}
