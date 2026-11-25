//! Resolve explicit next-turn intent into source-labelled quota observations.
//! This use case never changes provider admission, alerts, history or credentials.
use super::*;
use crate::{
    ComposerQuotaContext, ComposerQuotaEntry, ModelSelection, QuotaFreshness, QuotaObservation,
    QuotaOrigin, QuotaWindowObservation, config::ProviderQuotaSource,
    provider_authentication::AuthenticationMaterial,
};

impl CoreRuntime {
    pub(crate) fn composer_quota(
        &self,
        context: ComposerQuotaContext,
        refresh: bool,
    ) -> Result<Vec<ComposerQuotaEntry>, RuntimeError> {
        let runtime = self.runtime_instance(&context.runtime_id)?;
        if !runtime.enabled {
            return Ok(Vec::new());
        }
        let entries = match &context.selection {
            Some(ModelSelection::Managed { model_record_key }) => {
                let gateway = self
                    .gateways
                    .iter()
                    .find(|g| g.id == runtime.gateway_id)
                    .ok_or_else(|| RuntimeError::invalid("运行时的网关不存在"))?;
                let mut sources: Vec<(&crate::config::ProviderDefinition, Vec<String>)> =
                    Vec::new();
                for key in gateway.members(model_record_key) {
                    let resolved = gateway
                        .resolve_dispatch(&key)
                        .map_err(|_| RuntimeError::invalid("所选模型已不存在"))?;
                    let provider = resolved.provider();
                    let authentication = provider.authentication.description();
                    let reports_credits = matches!(
                        provider.authentication.material,
                        AuthenticationMaterial::CodexSubscription {
                            configured: true,
                            ..
                        }
                    );
                    if (provider.effective_quota_source() == ProviderQuotaSource::None && !reports_credits)
                        || (authentication.method == crate::AuthenticationMethod::OAuth
                            && !authentication.configured)
                    {
                        continue;
                    }
                    let model = &resolved.model().provider_model_id;
                    if let Some((_, models)) = sources.iter_mut().find(|(p, _)| p.id == provider.id)
                    {
                        if !models.contains(model) {
                            models.push(model.clone());
                        }
                    } else {
                        sources.push((provider, vec![model.clone()]));
                    }
                }
                let mut credit_accounts = std::collections::BTreeSet::new();
                let mut entries = Vec::with_capacity(sources.len());
                for (provider, models) in sources {
                    let credits = self
                        .provider_account
                        .query_credits(&gateway.id, &provider.id, refresh)?
                        .and_then(|(credits, identity)| {
                            let reported = credits.observation.as_ref().is_some_and(|credits| {
                                credits.unlimited == Some(true)
                                    || credits
                                        .balance
                                        .as_ref()
                                        .is_some_and(|balance| !balance.trim().is_empty())
                            });
                            (!reported || identity.is_none_or(|key| credit_accounts.insert(key)))
                                .then_some(credits)
                        });
                    let quota_source = provider.effective_quota_source();
                    if quota_source == ProviderQuotaSource::None && credits.is_none() {
                        continue;
                    }
                    let mut observation = self
                        .subscription_quota
                        .query(&gateway.id, &provider.id, refresh)?
                        .observation;
                    observation.windows = scoped_windows(observation.windows, &models);
                    if quota_source == ProviderQuotaSource::AntigravityCliUsage {
                        observation.freshness = scoped_antigravity_freshness(
                            &observation.windows,
                            crate::local::unix_ms_now(),
                        );
                    }
                    entries.push(ComposerQuotaEntry {
                        id: format!("provider:{}:{}", gateway.id, provider.id),
                        name: provider.name.clone(),
                        origin: QuotaOrigin::Provider,
                        observation,
                        limits: provider.quota_limits.clone(),
                        credits,
                    });
                }
                entries
            }
            _ if crate::native_quota_source::Capture::supported(runtime.type_id) => {
                let quota = self.runtime_quota.query(
                    &runtime.id,
                    crate::native_quota_source::Capture::from_runtime(runtime),
                    refresh,
                )?;
                vec![ComposerQuotaEntry {
                    id: format!("runtime:{}", runtime.id),
                    name: runtime.name.clone(),
                    origin: QuotaOrigin::RuntimeLogin,
                    observation: quota.observation,
                    limits: Vec::new(),
                    credits: None,
                }]
            }
            _ => context
                .conversation_id
                .as_deref()
                .and_then(|id| self.delivery.quota_record(id))
                .and_then(|snapshot| recorded_quota(snapshot, runtime))
                .into_iter()
                .collect(),
        };
        Ok(entries)
    }
}

fn scoped_windows(
    mut windows: Vec<QuotaWindowObservation>,
    models: &[String],
) -> Vec<QuotaWindowObservation> {
    windows.retain(|window| {
        window
            .model_ids
            .as_ref()
            .is_none_or(|ids| ids.iter().any(|id| models.contains(id)))
    });
    windows
}

fn scoped_antigravity_freshness(windows: &[QuotaWindowObservation], now: i64) -> QuotaFreshness {
    if windows.is_empty() {
        return QuotaFreshness::Unknown;
    }
    if windows.iter().any(|window| {
        window
            .observed_at_ms
            .is_some_and(|observed| observed > 0 && observed <= now)
            && window.expires_at_ms.is_some_and(|expiry| expiry > now)
            && window.reset_at_ms.is_none_or(|reset| reset > now)
    }) {
        QuotaFreshness::Fresh
    } else {
        QuotaFreshness::Stale
    }
}

/// Recorded limits are a historical source, never a fallback account identity.
fn recorded_quota(
    snapshot: &crate::conversation::ConversationSnapshot,
    runtime: &RuntimeInstance,
) -> Option<ComposerQuotaEntry> {
    let turn = recorded_turn(
        &snapshot.context_runtime_id,
        &runtime.id,
        &snapshot.transcript_turns,
    )?;
    let windows = turn
        .usage
        .as_ref()?
        .rate_limits
        .iter()
        .enumerate()
        .map(|(index, limit)| QuotaWindowObservation {
            id: limit.window_minutes.map_or_else(
                || format!("record:{index}"),
                |minutes| format!("window:{}", minutes.saturating_mul(60)),
            ),
            model_ids: None,
            used_percent: f64::from(limit.used_percent),
            reset_at_ms: limit.resets_at_unix_ms,
            window_ms: limit
                .window_minutes
                .and_then(|m| i64::try_from(m).ok())
                .and_then(|m| m.checked_mul(60_000)),
            observed_at_ms: None,
            expires_at_ms: None,
            pace: None,
        })
        .collect();
    Some(ComposerQuotaEntry {
        id: format!("record:{}:{}", snapshot.conversation.id, turn.id),
        name: runtime.name.clone(),
        origin: QuotaOrigin::ConversationRecord,
        observation: QuotaObservation {
            freshness: QuotaFreshness::Stale,
            windows,
            balances: Vec::new(),
            available: None,
            refreshing: false,
            problem: None,
        },
        limits: Vec::new(),
        credits: None,
    })
}

/// A recorded limit remains attached to the context and turn that reported it.
fn recorded_turn<'a>(
    context_runtime_id: &str,
    runtime_id: &str,
    turns: &'a [crate::conversation::TranscriptTurn],
) -> Option<&'a crate::conversation::TranscriptTurn> {
    if context_runtime_id != runtime_id {
        return None;
    }
    turns.iter().rev().find(|turn| {
        turn.settings.as_ref().is_some_and(|settings| {
            settings.runtime_id.as_deref() == Some(runtime_id)
                && !matches!(
                    settings.model_selection,
                    Some(ModelSelection::Managed { .. })
                )
        }) && turn
            .usage
            .as_ref()
            .is_some_and(|usage| !usage.rate_limits.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{RateLimit, TranscriptTurn, TurnSettings, TurnUsage};
    #[test]
    fn provider_scope_keeps_shared_windows_and_the_union_of_route_models() {
        let windows: Vec<QuotaWindowObservation> = serde_json::from_value(json!([
            {"id":"5h","usedPercent":54}, {"id":"7d","usedPercent":25},
            {"id":"sonnet-window","usedPercent":30,"modelIds":["sonnet"]},
            {"id":"opus-window","usedPercent":40,"modelIds":["opus"]},
            {"id":"other-account-model","usedPercent":90,"modelIds":["other"]}
        ]))
        .unwrap();
        let direct = scoped_windows(windows.clone(), &["sonnet".into()]);
        assert_eq!(
            direct
                .iter()
                .map(|window| window.id.as_str())
                .collect::<Vec<_>>(),
            ["5h", "7d", "sonnet-window"]
        );
        let route = scoped_windows(windows, &["opus".into(), "sonnet".into()]);
        assert_eq!(
            route
                .iter()
                .map(|window| window.id.as_str())
                .collect::<Vec<_>>(),
            ["5h", "7d", "sonnet-window", "opus-window"]
        );
    }

    #[test]
    fn antigravity_composer_uses_the_selected_models_window_freshness() {
        let windows: Vec<QuotaWindowObservation> = serde_json::from_value(json!([
            {"id":"expired","modelIds":["hydrogen"],"usedPercent":100,"observedAtMs":1,"expiresAtMs":2},
            {"id":"current","modelIds":["argon"],"usedPercent":20,"observedAtMs":1,"expiresAtMs":4},
            {"id":"unassigned","modelIds":[],"usedPercent":0,"observedAtMs":1,"expiresAtMs":4}
        ])).unwrap();
        for (model, expected) in [
            ("hydrogen", QuotaFreshness::Stale),
            ("argon", QuotaFreshness::Fresh),
            ("other", QuotaFreshness::Unknown),
        ] {
            let scoped = scoped_windows(windows.clone(), &[model.into()]);
            assert_eq!(
                std::mem::discriminant(&scoped_antigravity_freshness(&scoped, 3)),
                std::mem::discriminant(&expected),
            );
        }
    }

    fn record(runtime: &str, selection: Option<ModelSelection>) -> TranscriptTurn {
        TranscriptTurn {
            id: "turn".into(),
            user_message_id: "user".into(),
            work_message_ids: vec![],
            last_message_id: None,
            reply_start_block: 0,
            duration_ms: None,
            is_running: false,
            can_fork: false,
            outcome: None,
            settings: Some(TurnSettings {
                runtime_id: Some(runtime.into()),
                model_selection: selection,
                ..Default::default()
            }),
            usage: Some(TurnUsage {
                rate_limits: vec![RateLimit {
                    window_minutes: Some(300),
                    used_percent: 54,
                    resets_at_unix_ms: None,
                }],
                ..Default::default()
            }),
        }
    }
    #[test]
    fn recorded_limits_do_not_follow_a_switch_to_another_native_runtime() {
        let turns = [record("codex", None)];
        assert!(recorded_turn("codex", "codex", &turns).is_some());
        assert!(recorded_turn("codex", "pi", &turns).is_none());
        assert!(recorded_turn("pi", "pi", &turns).is_none());
    }
    #[test]
    fn managed_records_cannot_become_native_account_observations() {
        let turns = [record(
            "codex",
            Some(ModelSelection::Managed {
                model_record_key: "provider-model".into(),
            }),
        )];
        assert!(recorded_turn("codex", "codex", &turns).is_none());
    }
}
