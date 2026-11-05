//! An instance's new-conversation policy is configuration, committed with its execution settings.
use crate::{Failure, NewConversationChoice, RuntimeComposerChoices, RuntimeInstance, TurnChoice};

pub const NEW_CONVERSATION_SETTING: &str = "newConversation";

impl RuntimeInstance {
    pub fn new_conversation_choice(&self) -> Result<NewConversationChoice, Failure> {
        self.settings
            .get(NEW_CONVERSATION_SETTING)
            .map(|value| serde_json::from_str(value))
            .transpose()
            .map(Option::unwrap_or_default)
            .map_err(|_| Failure::invalid("运行时的新会话策略格式无效"))
    }

    pub fn set_new_conversation_choice(
        &mut self,
        choice: NewConversationChoice,
    ) -> Result<(), Failure> {
        self.settings.insert(
            NEW_CONVERSATION_SETTING.into(),
            serde_json::to_string(&choice)?,
        );
        Ok(())
    }

    /// Prepared execution also depends on executor-only settings (for example Pi's sdkHelper).
    /// Native history caches use their concrete reader configuration instead.
    pub fn execution_configuration(&self) -> Self {
        let mut runtime = self.clone();
        runtime.settings.remove(NEW_CONVERSATION_SETTING);
        runtime
    }
}

pub fn runtime_new_conversation_choice(
    runtime: RuntimeInstance,
) -> Result<NewConversationChoice, Failure> {
    runtime.new_conversation_choice()
}

pub fn runtime_with_new_conversation_choice(
    mut runtime: RuntimeInstance,
    choice: NewConversationChoice,
) -> Result<RuntimeInstance, Failure> {
    runtime.set_new_conversation_choice(choice)?;
    Ok(runtime)
}

/// The instance policy and a last choice of its current adapter type determine a fresh target.
pub fn runtime_start_choice(
    runtime: RuntimeInstance,
    last: Option<RuntimeComposerChoices>,
) -> Result<TurnChoice, Failure> {
    Ok(match runtime.new_conversation_choice()? {
        NewConversationChoice::Chosen { choice } => choice,
        NewConversationChoice::LastChosen => last
            .filter(|last| last.runtime_id == runtime.id && last.runtime_type == runtime.type_id)
            .map(|last| last.last_chosen)
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AcpLaunch, RuntimeExecution, RuntimeKind};
    use std::collections::BTreeMap;

    fn instance() -> RuntimeInstance {
        RuntimeInstance {
            enabled: false,
            id: "synthetic".into(),
            name: "Synthetic".into(),
            type_id: RuntimeKind::Acp,
            gateway_id: "default".into(),
            settings: BTreeMap::new(),
        }
    }

    #[test]
    fn policy_shares_instance_storage_without_changing_execution_configuration() {
        let mut runtime = instance();
        runtime.set_execution(RuntimeExecution::default()).unwrap();
        runtime
            .set_acp_launch(AcpLaunch {
                args: vec!["synthetic".into()],
                environment: BTreeMap::new(),
            })
            .unwrap();
        let execution = runtime.execution().unwrap();
        let launch = runtime.acp_launch().unwrap();
        let choice = NewConversationChoice::Chosen {
            choice: TurnChoice::default(),
        };
        runtime.set_new_conversation_choice(choice.clone()).unwrap();
        let encoded = serde_json::to_vec(&runtime).unwrap();
        let decoded: RuntimeInstance = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.new_conversation_choice().unwrap(), choice);
        assert_eq!(decoded.execution().unwrap(), execution);
        assert_eq!(decoded.acp_launch().unwrap(), launch);
        assert_eq!(decoded.acp_launch().unwrap().args, ["synthetic"]);
        runtime
            .settings
            .insert(NEW_CONVERSATION_SETTING.into(), "broken".into());
        assert!(runtime.new_conversation_choice().is_err());
    }

    #[test]
    fn remembered_defaults_require_the_current_instance_and_adapter_type() {
        let runtime = instance();
        let remembered = RuntimeComposerChoices {
            runtime_id: runtime.id.clone(),
            runtime_type: RuntimeKind::Codex166,
            last_chosen: TurnChoice {
                effort: Some("high".into()),
                ..TurnChoice::default()
            },
        };
        assert_eq!(
            runtime_start_choice(runtime.clone(), Some(remembered.clone())).unwrap(),
            TurnChoice::default()
        );
        let mut same_type = remembered;
        same_type.runtime_type = RuntimeKind::Acp;
        assert_eq!(
            runtime_start_choice(runtime.clone(), Some(same_type.clone())).unwrap(),
            same_type.last_chosen
        );
        same_type.runtime_id = "other".into();
        assert_eq!(
            runtime_start_choice(runtime, Some(same_type)).unwrap(),
            TurnChoice::default()
        );
    }
}
