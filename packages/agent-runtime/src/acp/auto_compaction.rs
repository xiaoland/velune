use super::{Client, ConfigOption, ConfigValue, Error, Result};
use velune_conversation::acp_config::current_model;

pub(super) struct Applied {
    model: String,
    option_id: String,
    value: ConfigValue,
    before: ConfigValue,
}

impl Client {
    pub fn current_model_id(&self) -> Option<&str> {
        current_model(&self.options)
    }

    /// An ACP policy applies through the options advertised for the active
    /// model. Old policies can stay saved when an Agent drops a capability.
    pub fn reconcile_auto_compaction(
        &mut self,
        desired: Option<(&str, &ConfigValue)>,
    ) -> Result<()> {
        self.ensure_idle()?;
        if let Some(applied) = &self.auto_compaction
            && self.current_model_id() == Some(applied.model.as_str())
            && desired == Some((applied.option_id.as_str(), &applied.value))
            && self
                .compaction_option(&applied.option_id)
                .is_some_and(|option| {
                    option.value == applied.value && option.accepts(&applied.value)
                })
        {
            return Ok(());
        }
        self.restore_auto_compaction()?;
        let Some((option_id, value)) = desired else {
            return Ok(());
        };
        let Some(model) = self.current_model_id().map(str::to_owned) else {
            return Ok(());
        };
        let Some(option) = self
            .compaction_option(option_id)
            .filter(|option| option.accepts(value))
        else {
            return Ok(());
        };
        if option.value == *value {
            return Ok(());
        }
        let before = option.value.clone();
        self.write_config_option(option_id, value.clone())?;
        if self.current_model_id() != Some(model.as_str())
            || self
                .compaction_option(option_id)
                .is_none_or(|option| option.value != *value)
        {
            return Err(Error::new("ACP Agent 未确认此模型的自动压缩设置"));
        }
        self.auto_compaction = Some(Applied {
            model,
            option_id: option_id.into(),
            value: value.clone(),
            before,
        });
        Ok(())
    }

    pub(super) fn restore_auto_compaction(&mut self) -> Result<()> {
        let Some(applied) = &self.auto_compaction else {
            return Ok(());
        };
        // Preserve a later Agent or session edit. An older value only belongs
        // to the model and option that originally announced it.
        if self.current_model_id() == Some(applied.model.as_str())
            && self
                .compaction_option(&applied.option_id)
                .is_some_and(|option| {
                    option.value == applied.value && option.accepts(&applied.before)
                })
        {
            let option_id = applied.option_id.clone();
            let before = applied.before.clone();
            self.write_config_option(&option_id, before.clone())?;
            if self
                .compaction_option(&option_id)
                .is_some_and(|option| option.value != before)
            {
                return Err(Error::new("ACP Agent 未恢复原来的自动压缩设置"));
            }
        }
        self.auto_compaction = None;
        Ok(())
    }

    fn compaction_option(&self, option_id: &str) -> Option<&ConfigOption> {
        self.options
            .iter()
            .find(|option| option.id == option_id && option.can_bind_auto_compaction())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn send_and_finish(client: &mut Client, expected: &str) {
        client.send(expected, &[]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.turn_running() && Instant::now() < deadline {
            client.poll().unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            client.snapshot().unwrap().run_state,
            velune_conversation::RunState::Idle
        );
    }

    #[test]
    fn model_auto_compaction_uses_announced_options_and_restores_agent_values() {
        let (mut client, root) = super::super::tests::synthetic_client("model-auto-compaction");
        client.create(&root, "synthetic-acp").unwrap();
        assert_eq!(client.current_model_id(), Some("model/a"));
        assert!(client.config_options()[1].is_auto_compaction());
        assert!(!client.config_options()[2].can_bind_auto_compaction());

        let boolean = ConfigValue::Boolean(true);
        let selected = Some(("auto_compact_enabled", &boolean));
        client.reconcile_auto_compaction(selected).unwrap();
        client.reconcile_auto_compaction(selected).unwrap();
        send_and_finish(&mut client, "true");

        client.select_model("model/b").unwrap();
        assert_eq!(client.current_model_id(), Some("model/b"));
        assert!(!client.config_options()[1].is_auto_compaction());
        assert!(client.config_options()[1].can_bind_auto_compaction());
        client.reconcile_auto_compaction(selected).unwrap();
        client
            .reconcile_auto_compaction(Some(("opaque/safety", &boolean)))
            .unwrap();
        send_and_finish(&mut client, r#""default""#);

        let mode = ConfigValue::Select("less".into());
        let custom = Some(("vendor.retention", &mode));
        client.reconcile_auto_compaction(custom).unwrap();
        send_and_finish(&mut client, r#""less""#);
        client.load("synthetic", &root, "synthetic-acp").unwrap();
        send_and_finish(&mut client, r#""default""#);
        client.reconcile_auto_compaction(custom).unwrap();
        client.reconcile_auto_compaction(None).unwrap();
        send_and_finish(&mut client, r#""default""#);
        client.shutdown().unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("setting-writes"))
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [
                "auto_compact_enabled",
                "auto_compact_enabled",
                "opaque/model-id",
                "vendor.retention",
                "vendor.retention",
                "vendor.retention",
                "vendor.retention"
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
