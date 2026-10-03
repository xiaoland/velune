//! Service-owned inward contract. Provider implementations depend on this module, not vice versa.
use crate::{
    InvalidContract, OperationFuture,
    ids::*,
    observation::{AttemptContext, Usage},
    sampling::*,
};
use std::{collections::HashSet, sync::Arc};

pub struct ProviderSamplingRequest {
    pub context: AttemptContext,
    pub input: SamplingInput,
}
pub struct ProviderSamplingOutcome {
    pub result: Result<SamplingOutput, SamplingFailure>,
    pub usage: Usage,
}
/// One actual attempt. Implementations translate protocol/SDK types internally; no hidden retries.
/// An instance must retain immutable runtime settings and must not reread global configuration.
pub trait SamplingProvider: Send + Sync {
    fn sampling(
        &self,
        request: ProviderSamplingRequest,
    ) -> OperationFuture<ProviderSamplingOutcome>;
}

/// The composition root creates a new binding for each config revision. No setters or global store.
/// The adapter captures its own protocol/model mapping/credential reference, not SDK types here.
pub struct ProviderBinding {
    id: ProviderId,
    revision: ConfigRevision,
    models: Vec<ModelId>,
    adapter: Arc<dyn SamplingProvider>,
}
impl ProviderBinding {
    pub fn new(
        id: ProviderId,
        revision: ConfigRevision,
        models: Vec<ModelId>,
        adapter: Arc<dyn SamplingProvider>,
    ) -> Result<Self, InvalidContract> {
        if models.is_empty() {
            return Err(InvalidContract("provider needs at least one model"));
        }
        let mut seen = HashSet::new();
        if models.iter().any(|m| !seen.insert(m)) {
            return Err(InvalidContract("duplicate provider model"));
        }
        Ok(Self {
            id,
            revision,
            models,
            adapter,
        })
    }
    pub fn id(&self) -> &ProviderId {
        &self.id
    }
    pub fn revision(&self) -> ConfigRevision {
        self.revision
    }
    /// Capture the current binding before starting asynchronous work. Replacing the app's
    /// selected Arc cannot retarget an already prepared invocation.
    pub fn prepare(
        self: &Arc<Self>,
        request: SamplingRequest,
        attempt: AttemptId,
    ) -> Result<PreparedSampling, InvalidContract> {
        if request.provider != self.id || !self.models.contains(&request.model) {
            return Err(InvalidContract(
                "sampling target is outside this provider binding",
            ));
        }
        let context = AttemptContext {
            call: request.call_id,
            attempt,
            provider: self.id.clone(),
            model: request.model,
            revision: self.revision,
        };
        Ok(PreparedSampling {
            binding: Arc::clone(self),
            request: ProviderSamplingRequest {
                context,
                input: request.input,
            },
        })
    }
}
/// Only an immutable snapshot and request, not an executor, router, or retry loop.
pub struct PreparedSampling {
    binding: Arc<ProviderBinding>,
    request: ProviderSamplingRequest,
}
impl PreparedSampling {
    pub fn context(&self) -> &AttemptContext {
        &self.request.context
    }
    /// Consume once; the returned Arc keeps the chosen runtime instance alive for the attempt.
    /// The future service implementation owns dispatch and observations. This slice does neither.
    pub fn into_parts(self) -> (Arc<dyn SamplingProvider>, ProviderSamplingRequest) {
        (Arc::clone(&self.binding.adapter), self.request)
    }
}
