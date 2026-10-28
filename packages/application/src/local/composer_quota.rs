//! Resolve explicit next-turn intent into source-labelled quota observations.
//! This use case never changes provider admission, alerts, history or credentials.
use super::*;
use crate::{
    ComposerQuotaContext, ComposerQuotaEntry, ModelSelection, QuotaFreshness, QuotaObservation,
    QuotaOrigin, QuotaWindowObservation,
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
                    if provider.quota_source == crate::config::ProviderQuotaSource::None
                        || (matches!(
                            provider.authentication.description().method,
                            crate::AuthenticationMethod::OAuth
                        ) && !provider.authentication.description().configured)
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
                sources
                    .into_iter()
                    .map(|(provider, models)| {
                        let mut quota =
                            self.subscription_quota
                                .query(&gateway.id, &provider.id, refresh)?;
                        quota.windows = scoped_windows(quota.windows, &models);
                        Ok(ComposerQuotaEntry {
                            id: format!("provider:{}:{}", gateway.id, provider.id),
                            name: provider.name.clone(),
                            origin: QuotaOrigin::Provider,
                            observation: QuotaObservation {
                                freshness: quota.freshness,
                                windows: quota.windows,
                                balances: quota.balances,
                                available: quota.available,
                                refreshing: quota.refreshing,
                                problem: quota.problem,
                            },
                            limits: provider.quota_limits.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, RuntimeError>>()?
            }
            _ if crate::native_quota_source::Capture::supported(&runtime.type_id) => {
                let quota = self.runtime_quota.query(
                    &runtime.id,
                    crate::native_quota_source::Capture::from_runtime(runtime),
                    refresh,
                )?;
                vec![ComposerQuotaEntry {
                    id: format!("runtime:{}", runtime.id),
                    name: runtime.name.clone(),
                    origin: QuotaOrigin::RuntimeLogin,
                    observation: QuotaObservation {
                        freshness: quota.freshness,
                        windows: quota.windows,
                        balances: Vec::new(),
                        available: None,
                        refreshing: quota.refreshing,
                        problem: quota.problem,
                    },
                    limits: Vec::new(),
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

    fn record(runtime: &str, selection: Option<ModelSelection>) -> TranscriptTurn {
        TranscriptTurn {
            id: "turn".into(),
            user_message_id: "user".into(),
            work_message_ids: vec![],
            last_message_id: None,
            reply_start_block: 0,
            duration_ms: None,
            is_running: false,
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
