//! Model overrides live with the runtime instance that interprets them.
use crate::{AutoCompactionSetting, Failure, ModelAutoCompaction, RuntimeInstance, RuntimeKind};
use std::collections::BTreeSet;
use velune_conversation::ModelSelection;

pub const MODEL_AUTO_COMPACTION_SETTING: &str = "modelAutoCompaction";

impl RuntimeInstance {
    pub fn model_auto_compactions(&self) -> Result<Vec<ModelAutoCompaction>, Failure> {
        let Some(raw) = self.settings.get(MODEL_AUTO_COMPACTION_SETTING) else {
            return Ok(Vec::new());
        };
        let entries: Vec<ModelAutoCompaction> =
            serde_json::from_str(raw).map_err(|_| Failure::invalid("模型自动压缩设置格式无效"))?;
        validate(self.type_id, &entries)?;
        Ok(entries)
    }

    pub fn model_auto_compaction(
        &self,
        model: &ModelSelection,
    ) -> Result<Option<AutoCompactionSetting>, Failure> {
        Ok(self
            .model_auto_compactions()?
            .into_iter()
            .find_map(|entry| (entry.model == *model).then_some(entry.setting)))
    }

    pub fn set_model_auto_compactions(
        &mut self,
        mut entries: Vec<ModelAutoCompaction>,
    ) -> Result<(), Failure> {
        validate(self.type_id, &entries)?;
        entries.sort_by(|left, right| left.model.cmp(&right.model));
        self.settings.insert(
            MODEL_AUTO_COMPACTION_SETTING.into(),
            serde_json::to_string(&entries)?,
        );
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn validate(runtime: RuntimeKind, entries: &[ModelAutoCompaction]) -> Result<(), Failure> {
    let mut models = BTreeSet::new();
    for entry in entries {
        let valid_model = match &entry.model {
            ModelSelection::Managed { model_record_key } => {
                runtime != RuntimeKind::Acp && valid_id(model_record_key)
            }
            ModelSelection::Native {
                provider_id,
                model_id,
            } => {
                let requires_provider = matches!(runtime, RuntimeKind::Pi | RuntimeKind::DeepSeek);
                let allows_provider =
                    !matches!(runtime, RuntimeKind::Acp | RuntimeKind::ClaudeCode);
                valid_id(model_id)
                    && provider_id
                        .as_deref()
                        .map_or(!requires_provider, |id| allows_provider && valid_id(id))
            }
        };
        if !valid_model || !models.insert(&entry.model) {
            return Err(Failure::invalid("自动压缩设置的模型无效或重复"));
        }
        match (runtime, &entry.setting) {
            (RuntimeKind::Codex166, AutoCompactionSetting::Codex { token_limit })
                if *token_limit > 0 => {}
            (RuntimeKind::ClaudeCode, AutoCompactionSetting::Claude { window_tokens })
                if (100_000..=1_000_000).contains(window_tokens) => {}
            (RuntimeKind::Pi, AutoCompactionSetting::Pi { reserve_tokens })
                if *reserve_tokens > 0 => {}
            (RuntimeKind::DeepSeek, AutoCompactionSetting::DeepSeek { threshold_percent })
                if (1..=100).contains(threshold_percent) => {}
            (RuntimeKind::Acp, AutoCompactionSetting::Acp { option_id, .. })
                if valid_id(option_id) => {}
            _ => {
                return Err(Failure::invalid(match runtime {
                    RuntimeKind::Codex166 => "Codex 自动压缩阈值必须是正整数",
                    RuntimeKind::ClaudeCode => "Claude Code 自动压缩窗口须为 100K 到 1M token",
                    RuntimeKind::Pi => "Pi 自动压缩预留量必须是正整数",
                    RuntimeKind::DeepSeek => "DeepSeek 自动压缩比例须为 1 到 100 的整数百分比",
                    RuntimeKind::Acp => "ACP 自动压缩设置必须引用 Agent 提供的配置选项",
                }));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(type_id: RuntimeKind) -> RuntimeInstance {
        RuntimeInstance {
            enabled: true,
            id: "synthetic".into(),
            name: "Synthetic".into(),
            type_id,
            gateway_id: "default".into(),
            settings: Default::default(),
        }
    }

    fn managed(key: &str) -> ModelSelection {
        ModelSelection::Managed {
            model_record_key: key.into(),
        }
    }

    #[test]
    fn model_route_and_native_policies_round_trip_without_inheriting_each_other() {
        let mut runtime = runtime(RuntimeKind::Codex166);
        let a = managed("first-channel");
        let b = managed("second-channel");
        let route = managed("route-record");
        let native = ModelSelection::Native {
            provider_id: Some("codex-provider".into()),
            model_id: "model".into(),
        };
        let entry = |model, token_limit| ModelAutoCompaction {
            model,
            setting: AutoCompactionSetting::Codex { token_limit },
        };
        runtime
            .set_model_auto_compactions(vec![
                entry(a.clone(), 123_000),
                entry(b.clone(), 200_000),
                entry(route.clone(), 167_000),
                entry(native.clone(), 250_000),
            ])
            .unwrap();
        let mut runtime: RuntimeInstance =
            serde_json::from_value(serde_json::to_value(&runtime).unwrap()).unwrap();
        for (model, token_limit) in [
            (a, 123_000),
            (b, 200_000),
            (route, 167_000),
            (native, 250_000),
        ] {
            assert_eq!(
                runtime.model_auto_compaction(&model).unwrap(),
                Some(AutoCompactionSetting::Codex { token_limit })
            );
        }
        assert_eq!(
            runtime
                .model_auto_compaction(&managed("other-route"))
                .unwrap(),
            None
        );
        runtime.set_model_auto_compactions(vec![]).unwrap();
        assert!(runtime.model_auto_compactions().unwrap().is_empty());
        assert_eq!(
            runtime.settings.get(MODEL_AUTO_COMPACTION_SETTING).unwrap(),
            "[]"
        );
    }

    #[test]
    fn runtime_rejects_invalid_windows_and_duplicate_model_identities() {
        let mut runtime = runtime(RuntimeKind::ClaudeCode);
        let entry = |window_tokens| ModelAutoCompaction {
            model: managed("opus"),
            setting: AutoCompactionSetting::Claude { window_tokens },
        };
        assert!(runtime.model_auto_compactions().unwrap().is_empty());
        assert!(
            runtime
                .set_model_auto_compactions(vec![entry(9_000)])
                .is_err()
        );
        assert!(
            runtime
                .set_model_auto_compactions(vec![entry(2_000_000)])
                .is_err()
        );
        assert!(
            runtime
                .set_model_auto_compactions(vec![entry(200_000), entry(300_000)])
                .is_err()
        );
        runtime
            .set_model_auto_compactions(vec![entry(200_000)])
            .unwrap();
        assert!(
            runtime
                .set_model_auto_compactions(vec![ModelAutoCompaction {
                    model: managed("opus"),
                    setting: AutoCompactionSetting::Pi {
                        reserve_tokens: 32_000
                    },
                }])
                .is_err()
        );
        assert_eq!(
            runtime.model_auto_compactions().unwrap(),
            vec![entry(200_000)]
        );
    }

    #[test]
    fn acp_keeps_opaque_values_for_the_agent_to_validate() {
        let mut runtime = runtime(RuntimeKind::Acp);
        let model = ModelSelection::Native {
            provider_id: None,
            model_id: "native".into(),
        };
        let setting = AutoCompactionSetting::Acp {
            option_id: "auto_compact".into(),
            value: velune_conversation::acp_config::AcpConfigValue::Select(String::new()),
        };
        runtime
            .set_model_auto_compactions(vec![ModelAutoCompaction {
                model: model.clone(),
                setting: setting.clone(),
            }])
            .unwrap();
        assert_eq!(
            runtime.model_auto_compaction(&model).unwrap(),
            Some(setting)
        );
    }
}
