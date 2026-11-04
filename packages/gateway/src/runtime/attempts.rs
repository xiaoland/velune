//! Attempts: each route candidate in turn, until one answers the caller or a
//! failure forbids trying another.
use super::{
    TelemetryContext,
    credentials::{
        CredentialMaterial, CredentialRecovery, CredentialResolutionError, CredentialTarget,
        ResolvedCredential,
    },
    failover::{AttemptEnd, NextCandidate, PendingFailure, next_candidate, structured_quota_error},
    ingress::Ingress,
    routes::RouteTarget,
    wire::{WireEvent, retry_seconds, send_failure, send_json},
};
use crate::{
    analytics::{AnalyticsOutcome, SharedTracker, Tracker, now_ms},
    config::{ExecutionBackend, Protocol},
    observation::Observation,
    scheduling::ScheduleUnavailable,
};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tracing::{Instrument, instrument::WithSubscriber};
use velune_ai::{
    DeliveryError, InvalidContract, NativeCompletion, NativeErrorKind, NativeEvent, NativeFailure,
    NativeOutput, NativeProvider, NativeRequest, NativeSink, NativeSuccess, Payload,
    http::{Header, ResponseMeta},
};
use velune_ai_provider::{NativeHttp, claude_code::ClaudeCode, claude_code::ClaudeCodeConfig};

/// One admitted caller request, as each attempt sends it.
pub(super) struct Exchange {
    pub(super) request_id: String,
    pub(super) started_at_ms: i64,
    pub(super) protocol: Protocol,
    pub(super) body: Value,
    pub(super) headers: Vec<Header>,
    pub(super) stream: bool,
}

pub(super) async fn dispatch(
    state: Arc<Ingress>,
    exchange: Exchange,
    candidates: Vec<RouteTarget>,
    sender: mpsc::Sender<WireEvent>,
    telemetry: TelemetryContext,
) {
    let mut last_failure = None;
    let mut unavailable = ScheduleUnavailable { retry_after: None };
    for (index, target) in candidates.iter().enumerate() {
        let permit = match state.health.acquire(
            &state.gateway_id,
            &state.candidate(target),
            state.quotas.as_ref(),
        ) {
            Ok(permit) => permit,
            Err(blocked) => {
                if target.continuation_required {
                    let headers = blocked
                        .retry_after
                        .into_iter()
                        .map(|delay| Header {
                            name: "retry-after".into(),
                            value: retry_seconds(delay).to_string(),
                        })
                        .collect();
                    send_failure(&sender,Some(ResponseMeta { status:409,headers }),None,409,
                        "Claude tool continuation is paused by quota, health or an invalidated provider binding").await;
                    return;
                }
                if let Some(delay) = blocked.retry_after {
                    unavailable.retry_after =
                        Some(unavailable.retry_after.map_or(delay, |old| old.min(delay)));
                }
                continue;
            }
        };
        let attempt = index + 1;
        let request_id = &exchange.request_id;
        let operation = telemetry.operation("gateway.attempt", velune_telemetry::SpanKind::Client);
        let dispatcher = operation
            .as_ref()
            .map(|op| op.dispatch())
            .unwrap_or_else(|| tracing::dispatcher::get_default(Clone::clone));
        let attempt_span = tracing::dispatcher::with_default(
            &dispatcher,
            || tracing::info_span!(parent: None, "gateway_attempt", request_id, attempt, operation_id = tracing::field::Empty),
        );
        let mut observation = Observation::new(
            "gateway_attempt_finished",
            attempt_span.clone(),
            state.stopping.clone(),
        )
        .with_telemetry(operation, telemetry.owner.clone());
        attempt_span.in_scope(|| {
            tracing::info!(
                event = "gateway_route_selected",
                provider_slot = target.provider_slot,
                model_slot = target.model_slot
            )
        });
        let analytics = if attempt == 1 {
            state.analytics(request_id.clone(), exchange.started_at_ms, target)
        } else {
            state.analytics(format!("{request_id}/{attempt}"), now_ms(), target)
        };
        // Health and sticky success have identical semantics with or without a sink.
        let tracker = analytics
            .as_ref()
            .map(|guard| guard.tracker.clone())
            .unwrap_or_else(|| Tracker::new(target.protocol));
        let end = Attempt {
            state: &state,
            target,
            tracker: &tracker,
            sender: &sender,
            observation: &mut observation,
        }
        .run(&exchange)
        .instrument(attempt_span.clone())
        .with_subscriber(dispatcher)
        .await;
        if matches!(&end, AttemptEnd::Completed)
            || tracker.lock().expect("owned analytics tracker").completed()
        {
            permit.completed(
                &target.route_key,
                &target.model_record_key,
                if target.sticky {
                    state.session.as_ref()
                } else {
                    None
                },
            );
        }
        drop(analytics);
        let failure = match end {
            AttemptEnd::Failed(failure) => failure,
            AttemptEnd::Interrupted { cooldown } => {
                if let Some(duration) = cooldown {
                    permit.cool(duration);
                }
                return;
            }
            AttemptEnd::Completed | AttemptEnd::Settled => return,
        };
        match &failure.next {
            NextCandidate::Allowed { cooldown } => {
                if let Some(duration) = cooldown {
                    permit.cool(*duration);
                    unavailable.retry_after = Some(
                        unavailable
                            .retry_after
                            .map_or(*duration, |old| old.min(*duration)),
                    );
                }
                attempt_span.in_scope(|| {
                    tracing::info!(event = "gateway_failover", http_status = failure.status)
                });
                last_failure = Some(failure);
            }
            NextCandidate::Stop { cooldown } => {
                if let Some(duration) = cooldown {
                    permit.cool(*duration);
                }
                drop(observation);
                send_failure(
                    &sender,
                    failure.meta,
                    failure.body,
                    failure.status,
                    failure.message,
                )
                .await;
                return;
            }
        }
    }
    if let Some(mut failure) = last_failure {
        let candidates: Vec<_> = candidates
            .iter()
            .map(|target| state.candidate(target))
            .collect();
        if unavailable.retry_after.is_some()
            && let Some(delay) =
                state
                    .health
                    .next_retry(&state.gateway_id, &candidates, state.quotas.as_ref())
        {
            let meta = failure.meta.get_or_insert(ResponseMeta {
                status: failure.status,
                headers: Vec::new(),
            });
            meta.headers
                .retain(|header| !header.name.eq_ignore_ascii_case("retry-after"));
            meta.headers.push(Header {
                name: "retry-after".into(),
                value: retry_seconds(delay).to_string(),
            });
        }
        send_failure(
            &sender,
            failure.meta,
            failure.body,
            failure.status,
            failure.message,
        )
        .await;
    } else {
        let status = if unavailable.retry_after.is_some() {
            429
        } else {
            503
        };
        let headers = unavailable
            .retry_after
            .into_iter()
            .map(|delay| Header {
                name: "retry-after".into(),
                value: retry_seconds(delay).to_string(),
            })
            .collect();
        send_failure(
            &sender,
            Some(ResponseMeta { status, headers }),
            None,
            status,
            "no eligible provider: quota or health observations do not permit a request",
        )
        .await;
    }
}

/// One attempt against one route target.
struct Attempt<'a> {
    state: &'a Arc<Ingress>,
    target: &'a RouteTarget,
    tracker: &'a SharedTracker,
    sender: &'a mpsc::Sender<WireEvent>,
    observation: &'a mut Observation,
}

impl Attempt<'_> {
    fn track(&self, record: impl FnOnce(&mut Tracker)) {
        record(&mut self.tracker.lock().expect("owned analytics tracker"));
    }

    fn cli(&self) -> bool {
        matches!(
            self.target.execution_backend,
            ExecutionBackend::ClaudeCli { .. }
        )
    }

    fn destination(&self) -> CredentialTarget {
        CredentialTarget {
            protocol: self.target.protocol,
            endpoint: self.target.endpoint.clone(),
            execution_backend: self.target.execution_backend.clone(),
            request_profile: self.target.request_profile,
        }
    }

    /// Ends the attempt before anything was sent; another member may serve it.
    fn abandon(&mut self, reason: &'static str, status: u16, message: &'static str) -> AttemptEnd {
        self.track(|tracker| tracker.finish(AnalyticsOutcome::Failed));
        self.observation.finish(reason);
        AttemptEnd::Failed(PendingFailure::new(
            status,
            message,
            NextCandidate::Allowed { cooldown: None },
        ))
    }

    /// Resolve the target's credential and send; a rejected refreshable
    /// credential is recovered once and the request sent again.
    async fn run(mut self, exchange: &Exchange) -> AttemptEnd {
        self.observation
            .transport(if self.cli() { "cli" } else { "http" });
        tracing::info!(event = "gateway_attempt_started");
        let mut body = exchange.body.clone();
        // The gateway alone resolves ingress aliases to the exact provider identifier.
        body["model"] = Value::String(self.target.model.as_str().to_owned());
        let plan = if exchange.protocol == self.target.protocol {
            None
        } else {
            match velune_ai_provider::translation::prepare(
                exchange.protocol,
                self.target.protocol,
                &body,
                self.target.max_output_tokens,
            ) {
                Ok(plan) => {
                    tracing::info!(
                        event = "gateway_translation_prepared",
                        source_protocol = ?exchange.protocol,
                        target_protocol = ?self.target.protocol,
                    );
                    body = plan.request().clone();
                    Some(plan)
                }
                Err(error) => {
                    tracing::warn!(event = "gateway_translation_failed", field = error.field);
                    return self.abandon(
                        "translation_failed",
                        400,
                        if error.field == "max_tokens_required" {
                            "protocol conversion requires max_tokens or a configured model output limit"
                        } else {
                            "protocol conversion failed for the selected provider"
                        },
                    );
                }
            }
        };
        // The model record's extra parameters override what translation set;
        // a field the upstream rejects fails the request with its own error.
        if !self.target.request_parameters.is_empty()
            && let Some(object) = body.as_object_mut()
        {
            for (key, value) in &self.target.request_parameters {
                object.insert(key.clone(), Value::String(value.clone()));
            }
        }
        let request = NativeRequest {
            model: self.target.model.clone(),
            body: Payload::new(body),
            headers: if plan.is_some() {
                crate::translation::upstream_headers(exchange.headers.clone(), self.target.protocol)
            } else {
                exchange.headers.clone()
            },
            stream: exchange.stream,
        };
        let Some(plan) = plan else {
            return self.execute(request).await;
        };
        let sender = self.sender.clone();
        let model = self.target.model.as_str().to_owned();
        let (upstream, receiver) = mpsc::channel(1);
        // A failed attempt that emitted no headers has nothing to convert. A
        // successful one retains its own tool state, model rules and limits.
        // Both futures stay in the ingress task for cancellation and backpressure.
        let native = async {
            let end = Attempt {
                sender: &upstream,
                ..self
            }
            .execute(request)
            .await;
            drop(upstream);
            end
        };
        let conversion =
            crate::translation::deliver(plan, model, exchange.stream, receiver, sender);
        tokio::join!(native, conversion).0
    }

    async fn execute(mut self, request: NativeRequest) -> AttemptEnd {
        let reference = self.target.reference.clone();
        let destination = self.destination();
        let resolver = self.state.resolver.clone();
        let credential = match resolver
            .resolve(reference.clone(), destination.clone())
            .await
        {
            Ok(credential) => credential,
            Err(error) => {
                tracing::warn!(event="gateway_credential_failed",kind=?error);
                // Resolved on this machine; another member's source may still work.
                return self.abandon(
                    "credential_failed",
                    503,
                    "认证资源不可用；请在认证设置中检查认证或重新导入来源。",
                );
            }
        };
        let recovery = credential.recovery.clone();
        let account = match &credential.material {
            CredentialMaterial::Http { account_id, .. } => account_id.clone(),
            CredentialMaterial::LocalSession { .. } => None,
        };
        let Some(rejected_token) = credential
            .token()
            .map(str::to_owned)
            .filter(|_| !self.cli() && matches!(recovery, CredentialRecovery::Refreshable { .. }))
        else {
            return self.send(request, credential).await;
        };
        let result = self.send(request.clone(), credential).await;
        if !rejected_authentication(&result) {
            return result;
        }
        tracing::info!(event = "gateway_credential_recovery");
        let recovered = resolver
            .refresh(
                reference.clone(),
                destination.clone(),
                rejected_token.clone(),
            )
            .await;
        let credential = match recovered {
            Ok(credential)
                if credential.recovery == recovery
                    && matches!(&credential.material, CredentialMaterial::Http { account_id, .. } if account_id == &account) =>
            {
                credential
            }
            Err(CredentialResolutionError::AuthenticationExpired) => {
                resolver
                    .invalidate(reference, destination, rejected_token)
                    .await;
                return allow_auth_failover(result);
            }
            _ => return allow_auth_failover(result),
        };
        let rejected_token = credential
            .token()
            .expect("recovered HTTP credential")
            .to_owned();
        let result = self.send(request, credential).await;
        if rejected_authentication(&result) {
            resolver
                .invalidate(reference, destination, rejected_token)
                .await;
            return allow_auth_failover(result);
        }
        result
    }

    async fn send(&mut self, request: NativeRequest, credential: ResolvedCredential) -> AttemptEnd {
        // Resolution can await owner verification; never dispatch newly resolved
        // material under a Runner whose captured binding was invalidated meanwhile.
        let provider = &self.target.provider_id;
        if self.state.health.epoch(&self.state.gateway_id, provider)
            != self.state.provider_epochs[provider]
        {
            return self.abandon(
                "credential_stale",
                503,
                "provider credentials changed while resolving; prepare a new Runner",
            );
        }
        self.track(Tracker::begin);
        let Ok(adapter) = self.adapter(credential) else {
            return self.abandon(
                "configuration_failed",
                502,
                "provider configuration is invalid",
            );
        };
        let sent = Arc::new(AtomicBool::new(false));
        let completion = adapter
            .send(
                request,
                sink(self.sender.clone(), self.tracker.clone(), sent.clone()),
            )
            .await;
        self.settle(completion, sent.load(Ordering::Acquire)).await
    }

    fn adapter(
        &self,
        credential: ResolvedCredential,
    ) -> Result<Box<dyn NativeProvider>, InvalidContract> {
        Ok(match &self.target.execution_backend {
            ExecutionBackend::Http => Box::new(NativeHttp::new(
                self.target.config.clone().expect("validated HTTP target"),
                self.state.client.clone(),
                credential,
            )?),
            ExecutionBackend::ClaudeCli { binary } => {
                let CredentialMaterial::LocalSession { config_dir } = credential.material else {
                    return Err(InvalidContract(
                        "Claude CLI requires a local session source",
                    ));
                };
                Box::new(ClaudeCode::new(
                    self.state.backends.claude_code.clone(),
                    ClaudeCodeConfig {
                        executable: binary.into(),
                        config_dir: config_dir.map(Into::into),
                        session: self.state.backend_session(),
                        binding: self.state.backend_bindings[&self.target.provider_id].clone(),
                    },
                )?)
            }
        })
    }

    async fn settle(&mut self, completion: NativeCompletion, headers_sent: bool) -> AttemptEnd {
        let cancelled =
            matches!(&completion, Err(failure) if failure.kind == NativeErrorKind::Cancelled);
        self.track(|tracker| {
            tracker.finish(match &completion {
                Ok(_) => AnalyticsOutcome::Completed,
                Err(_) if cancelled => AnalyticsOutcome::Cancelled,
                Err(_) => AnalyticsOutcome::Failed,
            })
        });
        let status = match &completion {
            Ok(success) => Some(match &success.output {
                NativeOutput::Json(body) => body.meta.status,
                NativeOutput::Stream(meta) => meta.status,
            }),
            Err(failure) => failure.response.as_ref().map(|meta| meta.status),
        };
        if let Some(status) = status {
            self.observation.http_response(status);
        }
        self.observation.finish(match &completion {
            Ok(_) => "transport_completed",
            Err(_) if cancelled => "downstream_closed",
            Err(_) => "upstream_failed",
        });
        match completion {
            Ok(NativeSuccess {
                output: NativeOutput::Json(body),
                completed,
            }) => {
                tracing::info!(
                    event = "gateway_upstream_headers",
                    http_status = body.meta.status
                );
                self.track(|tracker| tracker.json(body.meta.status, body.body.get()));
                let delivered = send_json(self.sender, body).await;
                if !delivered {
                    self.track(|tracker| tracker.finish(AnalyticsOutcome::Cancelled));
                    self.observation.finish("downstream_closed");
                }
                if delivered && completed {
                    AttemptEnd::Completed
                } else {
                    AttemptEnd::Settled
                }
            }
            Ok(NativeSuccess {
                output: NativeOutput::Stream(_),
                completed,
            }) => {
                if completed {
                    AttemptEnd::Completed
                } else {
                    AttemptEnd::Settled
                }
            }
            Err(failure) => self.failed(failure, headers_sent).await,
        }
    }

    async fn failed(&mut self, failure: NativeFailure, headers_sent: bool) -> AttemptEnd {
        use NativeErrorKind as Kind;
        let kind = failure.kind;
        if kind == Kind::Cancelled {
            tracing::info!(event = "gateway_dispatch_canceled");
        } else {
            let protocol = match self.target.protocol {
                Protocol::ChatCompletions => "chat_completions",
                Protocol::Responses => "responses",
                Protocol::Messages => "messages",
            };
            tracing::warn!(event="gateway_dispatch_failed",protocol,kind=?kind);
        }
        let status = failure.response.as_ref().map(|meta| meta.status);
        self.track(|tracker| match (&failure.body, status) {
            (Some(body), _) => tracker.json(status.unwrap_or(502), body.get()),
            (None, Some(status)) => tracker.headers(status),
            (None, None) => {}
        });
        if let Some(status) = status {
            tracing::info!(event = "gateway_upstream_headers", http_status = status);
        }
        if kind == Kind::QuotaExhausted {
            self.state.quotas.quota_exhausted(&self.target.provider_id);
        }
        let unreachable = matches!(kind, Kind::Transport | Kind::Timeout);
        if headers_sent {
            let _ = self.sender.send(WireEvent::Failed).await;
            return AttemptEnd::Interrupted {
                cooldown: unreachable.then_some(Duration::from_secs(5)),
            };
        }
        let cli = self.cli();
        let next = if kind == Kind::Permission {
            NextCandidate::Stop { cooldown: None }
        } else {
            next_candidate(
                matches!(
                    kind,
                    Kind::RateLimited | Kind::QuotaExhausted | Kind::Overloaded
                ) || (status == Some(402)
                    && structured_quota_error(failure.body.as_ref().map(Payload::get))),
                kind == Kind::Cancelled,
                // An HTTP request that got no answer may be generated again.
                unreachable
                    || (!cli
                        && kind == Kind::ProviderFailure
                        && failure.submitted
                        && failure.response.is_none()),
                status,
                failure.response.as_ref().map_or(&[], |meta| &meta.headers),
            )
        };
        // A local execution backend may already have acted on what it received.
        let next = if cli && failure.submitted {
            NextCandidate::Stop {
                cooldown: next.cooldown(),
            }
        } else {
            next
        };
        AttemptEnd::Failed(PendingFailure {
            status: match kind {
                Kind::InvalidInput => 400,
                Kind::Unsupported => 422,
                Kind::Authentication => 401,
                Kind::Permission => 403,
                Kind::RateLimited => 429,
                Kind::Timeout => 504,
                _ => 502,
            },
            meta: failure.response,
            body: failure.body,
            message: "upstream request failed",
            next,
            authentication: kind == Kind::Authentication,
        })
    }
}

fn rejected_authentication(result: &AttemptEnd) -> bool {
    matches!(result, AttemptEnd::Failed(failure) if failure.authentication)
}

fn allow_auth_failover(mut result: AttemptEnd) -> AttemptEnd {
    if let AttemptEnd::Failed(failure) = &mut result {
        failure.next = NextCandidate::Allowed { cooldown: None };
    }
    result
}

/// Forward what the provider delivers to the caller, recording it and
/// whether the response head was sent.
fn sink(
    sender: mpsc::Sender<WireEvent>,
    tracker: SharedTracker,
    sent: Arc<AtomicBool>,
) -> NativeSink {
    Box::new(move |event| {
        let sender = sender.clone();
        let tracker = tracker.clone();
        let sent = sent.clone();
        Box::pin(async move {
            let event = match event {
                NativeEvent::Headers(meta) => {
                    tracker
                        .lock()
                        .expect("owned analytics tracker")
                        .headers(meta.status);
                    tracing::info!(
                        event = "gateway_upstream_headers",
                        http_status = meta.status
                    );
                    sent.store(true, Ordering::Release);
                    WireEvent::Headers(meta)
                }
                NativeEvent::Body(bytes) => {
                    tracker
                        .lock()
                        .expect("owned analytics tracker")
                        .chunk(bytes.get());
                    WireEvent::Body(bytes.into_inner())
                }
            };
            sender.send(event).await.map_err(|_| DeliveryError::Closed)
        })
    })
}
