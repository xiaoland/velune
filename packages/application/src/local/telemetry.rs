use super::*;

impl CoreRuntime {
    pub(crate) fn set_telemetry(&mut self, owner: Arc<velune_telemetry::ClientTelemetry>) {
        self.telemetry = Some(owner);
        self.publish_execution();
    }

    pub(crate) fn publish_execution(&self) {
        if let Some(owner) = &self.telemetry {
            let count = std::iter::once(&self.session)
                .chain(&self.parked)
                .filter(|session| session.turn_running())
                .count();
            owner.observe_execution(count as u64);
        }
    }
    pub(super) fn started_turn_observation(&mut self) {
        if let Some(observation) = &mut self.session.turn_observation {
            observation.execution_started();
        }
    }
    pub(super) fn begin_turn_observation(&mut self) {
        self.session.turn_observation = self.telemetry.as_ref().map(|owner| {
            owner.operation(
                "execution.turn",
                velune_telemetry::TraceCarrier::current().as_ref(),
                false,
            )
        });
        if let Some(runner) = &self.session.gateway_runner {
            // A shared runner cannot distinguish a previous turn's background requests from a new turn.
            let unique = self.session.work_settled();
            let parent = unique
                .then(|| {
                    self.session
                        .turn_observation
                        .as_ref()
                        .and_then(|op| op.carrier())
                })
                .flatten();
            let revision = unique
                .then(|| {
                    self.session
                        .turn_observation
                        .as_ref()
                        .and_then(|op| op.revision())
                })
                .flatten();
            runner.set_telemetry(self.telemetry.clone(), parent, revision);
        }
    }

    pub(super) fn complete_turn_observation(&mut self, outcome: velune_telemetry::Outcome) {
        if let Some(mut observation) = self.session.turn_observation.take() {
            if let Some(owner) = &self.telemetry
                && owner.active_revision() != observation.revision()
            {
                let mut result =
                    owner.operation("execution.result", observation.carrier().as_ref(), false);
                result.retain_identity_from(&observation);
                result.correlation_gap(velune_telemetry::CorrelationGap::RetiredGeneration);
                result.execution_finished(outcome);
                result.complete(outcome);
            }
            if self
                .telemetry
                .as_ref()
                .is_none_or(|owner| owner.active_revision() == observation.revision())
            {
                observation.execution_finished(outcome);
            }
            observation.complete(outcome);
        }
    }
}

pub(super) fn observe<T, E>(
    owner: Option<&Arc<velune_telemetry::ClientTelemetry>>,
    name: &'static str,
    body: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let mut observation = owner.map(|owner| {
        owner.operation(
            name,
            velune_telemetry::TraceCarrier::current().as_ref(),
            false,
        )
    });
    let result = if let Some(observation) = &observation {
        observation.run(body)
    } else {
        body()
    };
    if let Some(observation) = &mut observation {
        observation.complete(if result.is_ok() {
            velune_telemetry::Outcome::Success
        } else {
            velune_telemetry::Outcome::Failure
        });
    }
    result
}
