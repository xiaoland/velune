//! Direct dispatch to one immutable binding, without routing, fallback or retries.
use crate::{
    InvalidContract, OperationFuture, SamplingService, ids::AttemptId, observation::*,
    provider::ProviderBinding, sampling::*,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub struct DirectSamplingService {
    binding: Arc<ProviderBinding>,
}
impl DirectSamplingService {
    pub fn new(binding: Arc<ProviderBinding>) -> Self {
        Self { binding }
    }
}
impl SamplingService for DirectSamplingService {
    fn sampling(
        &self,
        request: SamplingRequest,
        attempt: AttemptId,
        events: SamplingSink,
    ) -> Result<OperationFuture<SamplingCompletion>, InvalidContract> {
        // Synchronous preparation: app replacement cannot retarget this future.
        let prepared = self.binding.prepare(request, attempt)?;
        let context = prepared.context().clone();
        let (adapter, request) = prepared.into_parts();
        Ok(Box::pin(async move {
            let start = Instant::now();
            let sink = Arc::new(Mutex::new((0_u64, events)));
            let emit_sink = Arc::clone(&sink);
            let emit_context = context.clone();
            let result = adapter
                .sampling(
                    request,
                    Box::new(move |delta| {
                        let mut state = emit_sink.lock().expect("sampling sink poisoned");
                        state.0 += 1;
                        let sequence = state.0;
                        (state.1)(SamplingEvent {
                            context: emit_context.clone(),
                            sequence,
                            kind: SamplingEventKind::Delta(delta),
                        });
                    }),
                )
                .await;
            let elapsed = start.elapsed();
            let (execution, error) = match &result.result {
                Ok(_) => (ExecutionKnowledge::Accepted, None),
                Err(failure) => (failure.execution, Some(failure.kind)),
            };
            let mut state = sink.lock().expect("sampling sink poisoned");
            state.0 += 1;
            let sequence = state.0;
            (state.1)(SamplingEvent {
                context: context.clone(),
                sequence,
                kind: SamplingEventKind::Terminal { error },
            });
            SamplingCompletion {
                outcome: SamplingOutcome {
                    call_id: context.call.clone(),
                    result: result.result,
                    usage: result.usage,
                },
                call: CallObservation {
                    call: context.call.clone(),
                    operation: Operation::Sampling,
                    revision: context.revision,
                    elapsed,
                    attempts: u32::from(result.submitted),
                },
                attempt: result.submitted.then_some(AttemptObservation {
                    context,
                    execution,
                    error,
                    usage: result.usage,
                    elapsed,
                }),
            }
        }))
    }
}
