use super::*;
use serde::{Deserialize, Serialize};

fn logical_handoff_source(
    links: &[crate::config::ConversationLink],
    logical_runtime_id: &str,
    conversation_id: &str,
) -> Result<(String, String), RuntimeError> {
    if let Some(link) = links.iter().find(|link| link.id == conversation_id) {
        if link
            .segments
            .first()
            .is_none_or(|segment| segment.runtime_instance_id != logical_runtime_id)
        {
            return Err(RuntimeError::invalid("会话来源运行时与共享身份不匹配"));
        }
        let segment = link
            .segments
            .last()
            .ok_or_else(|| RuntimeError::invalid("会话链接没有物理来源"))?;
        // Canonical links can retain a runtime-qualified snapshot ID. Codex
        // RPCs require the underlying UUID; normalize only this physical
        // runtime's qualifier, without rewriting links or native file paths.
        let native_id = if segment.runtime_type_id == RuntimeKind::Codex166 {
            segment
                .native_conversation_id
                .strip_prefix(&format!("{}:", segment.runtime_instance_id))
                .unwrap_or(&segment.native_conversation_id)
        } else {
            &segment.native_conversation_id
        };
        Ok((segment.runtime_instance_id.clone(), native_id.to_owned()))
    } else {
        let prefix = format!("{logical_runtime_id}:");
        let native_id = conversation_id
            .strip_prefix(&prefix)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("来源会话身份无效"))?
            .to_owned();
        Ok((logical_runtime_id.to_owned(), native_id))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ForkRecord {
    work_id: String,
    source_runtime_id: String,
    source_native_id: String,
    source_home: PathBuf,
    temporary_thread_id: Option<String>,
    temporary_fingerprint: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ForkRegistryDisk {
    schema_version: u32,
    records: Vec<ForkRecord>,
}

/// A damaged or future registry is retained and disables only new summary
/// work. It never blocks normal application startup or ordinary conversations.
pub(crate) struct ForkRegistry {
    path: PathBuf,
    records: Result<Vec<ForkRecord>, String>,
}

#[derive(Clone)]
pub(crate) struct ForkRecoveryPlan {
    pub work_id: String,
    pub source_runtime_id: String,
    pub source_native_id: String,
    pub source_home: PathBuf,
    pub temporary_thread_id: Option<String>,
    pub temporary_fingerprint: Option<String>,
}

impl ForkRegistry {
    pub(crate) fn open(home: &Path) -> Self {
        let path = home.join("portable-handoff-forks.json");
        let records = Self::load(&path).map_err(|error| {
            tracing::warn!(target: "velune_application", event = "handoff_fork_registry_unavailable", detail = %error);
            error.to_string()
        });
        Self { path, records }
    }

    fn load(path: &Path) -> Result<Vec<ForkRecord>, RuntimeError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let disk: ForkRegistryDisk = serde_json::from_slice(&bytes)?;
        if disk.schema_version != 1 {
            return Err(RuntimeError::invalid(
                "摘要副本登记格式不受支持，原文件已保留",
            ));
        }
        for (index, record) in disk.records.iter().enumerate() {
            if record.work_id.trim().is_empty()
                || record.source_runtime_id.trim().is_empty()
                || record.source_native_id.trim().is_empty()
                || !record.source_home.is_absolute()
                || disk.records[..index]
                    .iter()
                    .any(|prior| prior.work_id == record.work_id)
            {
                return Err(RuntimeError::invalid("摘要副本登记身份无效，原文件已保留"));
            }
        }
        Ok(disk.records)
    }

    fn save(&mut self, records: Vec<ForkRecord>) -> Result<(), RuntimeError> {
        self.records
            .as_ref()
            .map_err(|detail| RuntimeError::Invalid(format!("无法更新摘要副本登记：{detail}")))?;
        let disk = ForkRegistryDisk {
            schema_version: 1,
            records: records.clone(),
        };
        crate::repository::commit(&self.path, &serde_json::to_vec_pretty(&disk)?)?;
        self.records = Ok(records);
        Ok(())
    }

    pub(crate) fn begin(
        &mut self,
        work_id: &str,
        source_runtime_id: &str,
        source_native_id: &str,
        source_home: &Path,
    ) -> Result<(), RuntimeError> {
        let current = self
            .records
            .as_ref()
            .map_err(|detail| RuntimeError::Invalid(format!("无法读取摘要副本登记：{detail}")))?;
        if current.len() >= 64 {
            return Err(RuntimeError::invalid(
                "摘要副本登记已满，请先恢复未完成的摘要工作",
            ));
        }
        if current.iter().any(|record| record.work_id == work_id) {
            return Err(RuntimeError::invalid("摘要作业身份重复"));
        }
        let mut records = current.clone();
        records.push(ForkRecord {
            work_id: work_id.to_owned(),
            source_runtime_id: source_runtime_id.to_owned(),
            source_native_id: source_native_id.to_owned(),
            source_home: source_home.to_owned(),
            temporary_thread_id: None,
            temporary_fingerprint: None,
        });
        self.save(records)
    }

    pub(crate) fn observe_temporary(
        &mut self,
        work_id: &str,
        temporary_thread_id: &str,
        fingerprint: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.observe_temporary_artifact(work_id, Some(temporary_thread_id), fingerprint)
    }

    pub(crate) fn observe_temporary_artifact(
        &mut self,
        work_id: &str,
        temporary_thread_id: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let current = self
            .records
            .as_ref()
            .map_err(|detail| RuntimeError::Invalid(format!("无法读取摘要副本登记：{detail}")))?;
        let mut records = current.clone();
        let record = records
            .iter_mut()
            .find(|record| record.work_id == work_id)
            .ok_or_else(|| RuntimeError::invalid("摘要副本登记已过期"))?;
        if let Some(temporary_thread_id) = temporary_thread_id {
            record.temporary_thread_id = Some(temporary_thread_id.to_owned());
        }
        if let Some(fingerprint) = fingerprint {
            record.temporary_fingerprint = Some(fingerprint.to_owned());
        }
        self.save(records)
    }

    pub(crate) fn finish(&mut self, work_id: &str) -> Result<(), RuntimeError> {
        let current = self
            .records
            .as_ref()
            .map_err(|detail| RuntimeError::Invalid(format!("无法读取摘要副本登记：{detail}")))?;
        if !current.iter().any(|record| record.work_id == work_id) {
            return Ok(());
        }
        let records = current
            .iter()
            .filter(|record| record.work_id != work_id)
            .cloned()
            .collect();
        self.save(records)
    }

    pub(crate) fn recovery_plans(&self) -> Result<Vec<ForkRecoveryPlan>, RuntimeError> {
        let records = self
            .records
            .as_ref()
            .map_err(|detail| RuntimeError::Invalid(format!("无法读取摘要副本登记：{detail}")))?;
        Ok(records
            .iter()
            .map(|record| ForkRecoveryPlan {
                work_id: record.work_id.clone(),
                source_runtime_id: record.source_runtime_id.clone(),
                source_native_id: record.source_native_id.clone(),
                source_home: record.source_home.clone(),
                temporary_thread_id: record.temporary_thread_id.clone(),
                temporary_fingerprint: record.temporary_fingerprint.clone(),
            })
            .collect())
    }
}

/// The owner prepares only immutable paths/configuration. All native history
/// and model catalog reads happen on the shared HistoryWorkers.
#[derive(Clone)]
pub(crate) struct HandoffRequirementSource {
    /// Runtime that owns the last physical segment in the logical chain.
    pub physical_runtime_id: String,
    pub history: HistoryConfig,
    pub native: NativeConfig,
    pub native_id: String,
}

impl CoreRuntime {
    pub(crate) fn handoff_recovery_history(
        &self,
        runtime_id: &str,
        expected_home: &Path,
    ) -> Result<Option<HistoryConfig>, RuntimeError> {
        let runtime = self.runtime_instance(runtime_id)?;
        if runtime.type_id != RuntimeKind::Codex166 {
            return Ok(None);
        }
        let config = self.history_configuration(runtime)?;
        if config.home != expected_home {
            return Ok(None);
        }
        Ok(Some(config))
    }

    pub(crate) fn handoff_summary_basis(
        &self,
        source_runtime_id: &str,
        source_model_id: &str,
        source_provider_id: &str,
        models: Vec<crate::RuntimeModel>,
    ) -> velune_application_api::NextTurnInput {
        velune_application_api::NextTurnInput {
            runtime_id: source_runtime_id.to_owned(),
            runtime_type: runtime_types()
                .into_iter()
                .find(|descriptor| descriptor.id == RuntimeKind::Codex166),
            providers: Vec::new(),
            // Some(empty) is intentional: the shared resolver must not fall
            // back to every managed model on the source gateway.
            catalog: Some(models),
            turn_settings: vec![crate::conversation::TurnSettings {
                runtime_id: Some(source_runtime_id.to_owned()),
                model: Some(source_model_id.to_owned()),
                effort: None,
                permission_mode: None,
                model_selection: Some(crate::ModelSelection::Native {
                    provider_id: Some(source_provider_id.to_owned()),
                    model_id: source_model_id.to_owned(),
                }),
            }],
            conversation_runtime_id: Some(source_runtime_id.to_owned()),
            model_record_key: None,
            kept: None,
            runtime_choices: None,
            new_conversation: Default::default(),
        }
    }

    pub(crate) fn handoff_requirement_source(
        &self,
        source_runtime_id: &str,
        conversation_id: &str,
        target_runtime_id: &str,
    ) -> Result<Option<HandoffRequirementSource>, RuntimeError> {
        let target = self.runtime_instance(target_runtime_id)?;
        if !target.enabled {
            return Err(RuntimeError::invalid("目标运行时已停用"));
        }
        let (physical_runtime_id, native_id) =
            logical_handoff_source(&self.conversation_links, source_runtime_id, conversation_id)?;
        let source = self.runtime_instance(&physical_runtime_id)?;
        if !source.enabled {
            return Err(RuntimeError::invalid("当前物理来源运行时已停用"));
        }
        // A new Codex provider, like Pi, cannot read the source's private
        // context. The client requests this after a typed handoff rejection
        // when the runtime itself did not change.
        if source.type_id != RuntimeKind::Codex166
            || !matches!(target.type_id, RuntimeKind::Codex166 | RuntimeKind::Pi)
        {
            return Ok(None);
        }
        Ok(Some(HandoffRequirementSource {
            physical_runtime_id,
            history: self.history_configuration(source)?,
            native: self.native_config(source, None)?,
            native_id,
        }))
    }
}

#[cfg(test)]
#[path = "handoff_summary_tests.rs"]
mod tests;
