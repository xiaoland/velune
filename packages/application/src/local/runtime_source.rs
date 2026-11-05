//! Native cache identity comes from the same concrete configuration that readers consume.
use super::*;

#[derive(Clone, PartialEq, Eq)]
enum Reader {
    Pi(Box<PiConfig>),
    Native(HistoryConfig),
    Acp(velune_agent_runtime::acp::Config),
}

/// No gateway/provider or presentation defaults enter a native history request.
#[derive(Clone, PartialEq, Eq)]
struct RuntimeSource {
    runtime_id: String,
    kind: RuntimeKind,
    reader: Reader,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RuntimeSourceKey(Arc<RuntimeSource>);

impl RuntimeSourceKey {
    pub(super) fn runtime_id(&self) -> &str {
        &self.0.runtime_id
    }

    pub(super) fn is_current(&self, core: &CoreRuntime) -> bool {
        core.runtime_instance(self.runtime_id())
            .is_ok_and(|runtime| {
                runtime.enabled
                    && core
                        .runtime_source_key(runtime)
                        .is_ok_and(|current| current == *self)
            })
    }

    pub(super) fn matches(&self, core: &CoreRuntime, runtime: &RuntimeInstance) -> bool {
        self.is_current(core)
            && core
                .runtime_source_key(runtime)
                .is_ok_and(|source| source == *self)
    }
}

impl CoreRuntime {
    /// Pure resolution: no process startup, SSH probe, HOME creation or helper installation.
    pub(super) fn runtime_source_key(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<RuntimeSourceKey, RuntimeError> {
        let reader = match runtime.type_id {
            RuntimeKind::Pi => Reader::Pi(Box::new(self.pi_history_configuration(runtime)?)),
            RuntimeKind::Acp => Reader::Acp(self.acp_configuration(runtime, Vec::new())?),
            RuntimeKind::Codex166 | RuntimeKind::ClaudeCode | RuntimeKind::DeepSeek => {
                Reader::Native(self.history_configuration(runtime)?)
            }
        };
        Ok(RuntimeSourceKey(Arc::new(RuntimeSource {
            runtime_id: runtime.id.clone(),
            kind: runtime.type_id,
            reader,
        })))
    }

    pub(super) fn same_runtime_source(
        &self,
        before: &RuntimeInstance,
        after: &RuntimeInstance,
    ) -> bool {
        match (
            self.runtime_source_key(before),
            self.runtime_source_key(after),
        ) {
            (Ok(before), Ok(after)) => before == after,
            _ => false,
        }
    }
}
