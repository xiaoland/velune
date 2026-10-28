use crate::{Application, Error, config::*};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub use velune_application_api::records::*;
impl From<&GatewayConfig> for GatewaySummary {
    fn from(g: &GatewayConfig) -> Self {
        Self {
            id: g.id.clone(),
            name: g.name.clone(),
            providers: g
                .providers
                .iter()
                .map(|p| ProviderSummary {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    protocol: p.protocol.clone(),
                    endpoint: p.endpoint.clone(),
                    execution_backend: p.execution_backend.clone(),
                    request_profile: p.request_profile,
                    authentication_source: p.authentication.source(),
                    quota_source: p.quota_source,
                    quota_authentication: p
                        .quota_authentication
                        .description(p.authentication.generation),
                    authentication: {
                        let mut description = p.authentication.description();
                        if p.request_profile == crate::config::RequestProfile::KimiCoding {
                            description.disclosure =
                                Some(crate::SubscriptionDisclosure::kimi_coding_plan());
                        }
                        if p.request_profile == crate::config::RequestProfile::ArkCoding {
                            description.disclosure =
                                Some(crate::SubscriptionDisclosure::ark_coding_plan());
                        }
                        description
                    },
                    quota_limits: p.quota_limits.clone(),
                    models: p.models.iter().map(Into::into).collect(),
                })
                .collect(),
            routes: g.routes.clone(),
        }
    }
}
impl Application {
    pub fn analytics_query(&self, query: AnalyticsQuery) -> Result<AnalyticsReport, Error> {
        if query.from_ms >= query.to_ms
            || query.bucket_boundaries_ms.len() < 2
            || query.bucket_boundaries_ms.len() > 370
            || query.bucket_boundaries_ms.first() != Some(&query.from_ms)
            || query.bucket_boundaries_ms.last() != Some(&query.to_ms)
            || query
                .bucket_boundaries_ms
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || query.request_limit == 0
        {
            return Err(Error::invalid("分析查询范围无效"));
        }
        let mut overview = AnalyticsAccumulator::default();
        let mut trend = query
            .bucket_boundaries_ms
            .windows(2)
            .map(|bounds| (bounds[0], bounds[1], AnalyticsAccumulator::default()))
            .collect::<Vec<_>>();
        let mut providers = BTreeMap::<String, AnalyticsGroup>::new();
        let mut models = BTreeMap::<(String, String), AnalyticsGroup>::new();
        let mut requests = Vec::new();
        self.analytics.visit(
            query.from_ms,
            query.to_ms,
            query.provider_id.as_deref(),
            query.model_record_key.as_deref(),
            |row| {
                overview.add(&row)?;
                let index = query
                    .bucket_boundaries_ms
                    .partition_point(|boundary| *boundary <= row.terminal_at_ms)
                    .saturating_sub(1);
                trend[index].2.add(&row)?;
                providers
                    .entry(row.provider_id.clone())
                    .or_insert_with(|| AnalyticsGroup::provider(&row))
                    .totals
                    .add(&row)?;
                models
                    .entry((row.provider_id.clone(), row.model_record_key.clone()))
                    .or_insert_with(|| AnalyticsGroup::model(&row))
                    .totals
                    .add(&row)?;
                if requests.len() < query.request_limit.min(500) as usize {
                    requests.push(analytics_request(row)?);
                }
                Ok(())
            },
        )?;
        Ok(AnalyticsReport {
            overview: overview.finish(),
            trend: trend
                .into_iter()
                .map(|(from_ms, to_ms, total)| AnalyticsBucket {
                    from_ms,
                    to_ms,
                    totals: total.finish(),
                })
                .collect(),
            providers: providers
                .into_values()
                .map(AnalyticsGroup::finish)
                .collect(),
            models: models.into_values().map(AnalyticsGroup::finish).collect(),
            requests,
            coverage: "gateway_upstream_observation".into(),
            storage_warning: self.analytics.warning(),
            dropped_count: self.analytics.dropped(),
        })
    }
    fn execute<T: DeserializeOwned>(
        &mut self,
        action: &str,
        mut payload: Value,
        runtime_id: Option<&str>,
    ) -> Result<T, Error> {
        if let Some(runtime_id) = runtime_id {
            payload["runtimeInstanceID"] = json!(runtime_id);
        }
        let request =
            json!({"version":3,"action":action,"runtimeInstanceID":runtime_id,"payload":payload});
        let mut value = self.runtime.request_inner(&request)?;
        // ACP launch arguments and environment are read only through the explicit editor API.
        if let Some(runtimes) = value
            .get_mut("runtimeInstances")
            .and_then(Value::as_array_mut)
        {
            for runtime in runtimes {
                if let Some(settings) = runtime.get_mut("settings").and_then(Value::as_object_mut) {
                    settings.remove("acpLaunch");
                }
            }
        }
        if action == "discoverRuntimes"
            && let Some(candidates) = value.as_array_mut()
        {
            for candidate in candidates {
                if let Some(settings) = candidate
                    .get_mut("runtime")
                    .and_then(|runtime| runtime.get_mut("settings"))
                    .and_then(Value::as_object_mut)
                {
                    settings.remove("acpLaunch");
                }
            }
        }

        serde_json::from_value(value).map_err(Error::from)
    }
    /// Load a known native ACP reference even when the Agent cannot list sessions.
    pub fn open_acp_conversation(
        &mut self,
        runtime_id: String,
        native_session_id: String,
        cwd: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "openAcpConversation",
            json!({"nativeSessionID":native_session_id,"cwd":cwd}),
            Some(&runtime_id),
        )
    }
    pub fn acp_session_config(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Vec<crate::AcpConfigOption>, Error> {
        self.execute(
            "acpSessionConfig",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    pub fn set_acp_session_config(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        config_id: String,
        value: crate::AcpConfigValue,
    ) -> Result<Vec<crate::AcpConfigOption>, Error> {
        self.execute(
            "setAcpSessionConfig",
            json!({"conversationID":conversation_id,"configID":config_id,"value":value}),
            Some(&runtime_id),
        )
    }
    pub fn create_conversation(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        model_record_key: String,
    ) -> Result<SnapshotResult, Error> {
        self.create_conversation_with_selection(
            runtime_id,
            cwd,
            Some(ModelSelection::Managed { model_record_key }),
        )
    }
    pub fn create_conversation_with_selection(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        selection: Option<ModelSelection>,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "create",
            json!({"cwd":cwd,"modelSelection":selection}),
            Some(&runtime_id),
        )
    }
    pub fn open_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "open",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Read one native conversation without changing the active execution or
    /// transcript projection.
    pub fn read_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "readConversation",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    pub fn rename_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        title: String,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "renameConversation",
            json!({"conversationID":conversation_id,"title":title}),
            Some(&runtime_id),
        )
    }
    pub fn delete_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "deleteConversation",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Query the current projection in its runtime context. An enabled known
    /// instance returns no snapshot when no conversation is loaded; a different
    /// instance cannot query another instance's loaded conversation. Use the
    /// snapshot's context_runtime_id, not its logical conversation's origin.
    pub fn snapshot(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "getSnapshot",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Like `snapshot`, relative to a cursor from an earlier delivery of the
    /// same view: unchanged content is not transferred again.
    pub fn snapshot_update(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        since: crate::TranscriptCursor,
    ) -> Result<crate::SnapshotUpdate, Error> {
        self.execute(
            "getSnapshotUpdate",
            json!({"conversationID":conversation_id,"since":since}),
            Some(&runtime_id),
        )
    }
    /// Complete payloads of messages in a delivered view, by message identifier.
    pub fn message_payloads(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        message_ids: Vec<String>,
    ) -> Result<Vec<crate::DeliveredMessage>, Error> {
        self.execute(
            "messagePayloads",
            json!({"conversationID":conversation_id,"messageIDs":message_ids}),
            Some(&runtime_id),
        )
    }
    /// Accept this turn's source conversation and target runtime/model. A different instance creates a new
    /// native segment and carries quoted text context; it does not replay tools
    /// or resume a foreign native session. Association commits precede dispatch,
    /// and an ambiguous dispatch is never automatically retried.
    pub fn send_turn(
        &mut self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        model_record_key: String,
        text: String,
    ) -> Result<SnapshotResult, Error> {
        self.send_turn_with_selection(
            source_runtime_id,
            conversation_id,
            target_runtime_id,
            Some(ModelSelection::Managed { model_record_key }),
            ComposedTurn {
                text,
                effort: None,
                permission_mode: None,
                attachments: Vec::new(),
            },
        )
    }
    pub fn send_turn_with_selection(
        &mut self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, Error> {
        self.execute("sendTurn", json!({"sourceRuntimeID":source_runtime_id,"conversationID":conversation_id,"modelSelection":selection,"text":turn.text,"effort":turn.effort,"permissionMode":turn.permission_mode,"attachments":turn.attachments}), Some(&target_runtime_id))
    }
    /// Start a native turn after a stop or failure without adding a user
    /// message (`FollowUp::can_resume`). The runtime must own the current
    /// context. Queued messages remain paused; errors never send fallback text.
    pub fn resume_turn(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        selection: Option<ModelSelection>,
        effort: Option<String>,
        permission_mode: Option<String>,
    ) -> Result<SnapshotResult, Error> {
        self.execute("resumeTurn", json!({"conversationID":conversation_id,"modelSelection":selection,"effort":effort,"permissionMode":permission_mode}), Some(&runtime_id))
    }
    /// Compact only this conversation’s native context, using the selected model.
    /// This is an asynchronous effect; progress and completion arrive in snapshots.
    pub fn compact_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        selection: Option<ModelSelection>,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "compactConversation",
            json!({"conversationID":conversation_id,"modelSelection":selection}),
            Some(&runtime_id),
        )
    }

    /// Change the conversation's send queue. A queue that is added to or
    /// resumed while no turn runs sends its first message at once.
    pub fn send_queue(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        operation: crate::SendQueueOperation,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "sendQueue",
            json!({"conversationID":conversation_id,"operation":operation}),
            Some(&runtime_id),
        )
    }
    /// Deliver a message into the conversation's running turn through the
    /// runtime's own channel (`FollowUp::can_steer`); it does not start a
    /// turn, and the next-turn choices do not apply to it.
    pub fn steer_turn(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "steerTurn",
            json!({"conversationID":conversation_id,"text":turn.text,"attachments":turn.attachments}),
            Some(&runtime_id),
        )
    }
    /// Send `turn` in place of the user message `message_id`
    /// (`FollowUp::editable`), continuing as `placement` says. A fork's
    /// result is the new conversation's snapshot.
    pub fn edit_message(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        placement: crate::conversation::EditPlacement,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, Error> {
        self.execute("editMessage", json!({"conversationID":conversation_id,"messageID":message_id,"placement":placement,"modelSelection":selection,"text":turn.text,"effort":turn.effort,"permissionMode":turn.permission_mode,"attachments":turn.attachments}), Some(&runtime_id))
    }
    pub fn cancel(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "cancel",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Stop the native sub-agents the conversation started (`FollowUp::can_stop_sub_agents`);
    /// they may run after its turn has ended. A queue waiting for them goes on once they have.
    pub fn stop_sub_agents(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "stopSubAgents",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Conversations executing or waiting for the user across open sessions,
    /// and turns that ended after `since` (0 for all retained); cheap enough
    /// to poll.
    pub fn conversation_activity(&mut self, since: u64) -> Result<ConversationActivity, Error> {
        self.execute("conversationActivity", json!({"since":since}), None)
    }
    /// Uncommitted changes in the conversation's working directory.
    pub fn working_changes(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<WorkingChanges, Error> {
        self.execute(
            "workingChanges",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// The branch checked out in the conversation's working directory.
    pub fn working_branch(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Option<String>, Error> {
        self.execute(
            "workingBranch",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// The diff of one file listed by `working_changes`.
    pub fn file_diff(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        path: String,
    ) -> Result<FileDiff, Error> {
        self.execute(
            "fileDiff",
            json!({"conversationID":conversation_id,"path":path}),
            Some(&runtime_id),
        )
    }
    pub fn reply_runtime_interaction(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        interaction_id: String,
        reply: velune_conversation::RuntimeInteractionReply,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "replyRuntimeInteraction",
            json!({"conversationID":conversation_id,"interactionID":interaction_id,"reply":reply}),
            Some(&runtime_id),
        )
    }
}

#[derive(Default)]
struct AnalyticsAccumulator {
    totals: AnalyticsTotals,
    eligible_output: u64,
    eligible_elapsed_ms: u64,
}
impl AnalyticsAccumulator {
    fn add(&mut self, row: &crate::analytics::AnalyticsRow) -> Result<(), Error> {
        let total = &mut self.totals;
        total.request_count += 1;
        match row.outcome.as_str() {
            "completed" => total.completed_count += 1,
            "cancelled" => total.cancelled_count += 1,
            "failed" => total.failed_count += 1,
            "incomplete" => total.incomplete_count += 1,
            "rejected" => total.rejected_count += 1,
            _ => return Err(Error::invalid("分析记录包含未知结果")),
        }
        total.usage_reported_count += u64::from(row.usage_reported);
        total.usage_complete_count += u64::from(row.usage_complete);
        sum_optional(&mut total.input_tokens, row.input_tokens)?;
        sum_optional(&mut total.output_tokens, row.output_tokens)?;
        sum_optional(
            &mut total.reasoning_output_tokens,
            row.reasoning_output_tokens,
        )?;
        if let (Some(input), Some(output)) = (row.input_tokens, row.output_tokens) {
            sum_optional(&mut total.total_tokens, Some(sum(input, output)?))?;
            total.total_reported_count += 1;
        }
        total.input_reported_count += u64::from(row.input_tokens.is_some());
        total.output_reported_count += u64::from(row.output_tokens.is_some());
        total.reasoning_reported_count += u64::from(row.reasoning_output_tokens.is_some());
        total.cache_reported_count += u64::from(
            row.cached_input_tokens.is_some()
                || row.cache_read_input_tokens.is_some()
                || row.cache_creation_input_tokens.is_some(),
        );
        if matches!(row.outcome.as_str(), "completed" | "incomplete")
            && let (Some(output), Some(elapsed)) = (
                row.output_tokens,
                row.terminal_elapsed_ms.filter(|value| *value > 0),
            )
        {
            total.eligible_speed_count += 1;
            self.eligible_output = sum(self.eligible_output, output)?;
            self.eligible_elapsed_ms = sum(self.eligible_elapsed_ms, elapsed)?;
        }
        Ok(())
    }
    fn finish(mut self) -> AnalyticsTotals {
        if self.eligible_elapsed_ms > 0 {
            self.totals.output_tokens_per_second =
                Some(self.eligible_output as f64 * 1000.0 / self.eligible_elapsed_ms as f64);
        }
        self.totals
    }
}
fn sum(a: u64, b: u64) -> Result<u64, Error> {
    a.checked_add(b)
        .ok_or_else(|| Error::invalid("分析统计超出数量范围"))
}
fn sum_optional(target: &mut Option<u64>, value: Option<u64>) -> Result<(), Error> {
    if let Some(value) = value {
        *target = Some(sum(target.unwrap_or(0), value)?);
    }
    Ok(())
}
struct AnalyticsGroup {
    key: String,
    name: String,
    provider_id: String,
    model_record_key: Option<String>,
    totals: AnalyticsAccumulator,
}
impl AnalyticsGroup {
    fn provider(row: &crate::analytics::AnalyticsRow) -> Self {
        Self {
            key: row.provider_id.clone(),
            name: row.provider_name.clone(),
            provider_id: row.provider_id.clone(),
            model_record_key: None,
            totals: Default::default(),
        }
    }
    fn model(row: &crate::analytics::AnalyticsRow) -> Self {
        Self {
            key: serde_json::json!([row.provider_id, row.model_record_key]).to_string(),
            name: row.provider_model_id.clone(),
            provider_id: row.provider_id.clone(),
            model_record_key: Some(row.model_record_key.clone()),
            totals: Default::default(),
        }
    }
    fn finish(self) -> AnalyticsBreakdown {
        AnalyticsBreakdown {
            key: self.key,
            name: self.name,
            provider_id: Some(self.provider_id),
            model_record_key: self.model_record_key,
            totals: self.totals.finish(),
        }
    }
}
fn analytics_request(row: crate::analytics::AnalyticsRow) -> Result<AnalyticsRequest, Error> {
    let protocol = match row.protocol.as_str() {
        "chatCompletions" => AnalyticsProtocol::ChatCompletions,
        "responses" => AnalyticsProtocol::Responses,
        "messages" => AnalyticsProtocol::Messages,
        _ => return Err(Error::invalid("分析记录包含未知协议")),
    };
    let outcome = match row.outcome.as_str() {
        "completed" => AnalyticsOutcome::Completed,
        "incomplete" => AnalyticsOutcome::Incomplete,
        "cancelled" => AnalyticsOutcome::Cancelled,
        "rejected" => AnalyticsOutcome::Rejected,
        "failed" => AnalyticsOutcome::Failed,
        _ => return Err(Error::invalid("分析记录包含未知结果")),
    };
    let output_tokens_per_second = if matches!(
        outcome,
        AnalyticsOutcome::Completed | AnalyticsOutcome::Incomplete
    ) {
        row.terminal_elapsed_ms
            .filter(|v| *v > 0)
            .zip(row.output_tokens)
            .map(|(elapsed, tokens)| tokens as f64 * 1000.0 / elapsed as f64)
    } else {
        None
    };
    Ok(AnalyticsRequest {
        request_id: row.request_id,
        provider_id: row.provider_id,
        provider_name: row.provider_name,
        model_record_key: row.model_record_key,
        provider_model_id: row.provider_model_id,
        protocol,
        started_at_ms: row.started_at_ms,
        terminal_at_ms: row.terminal_at_ms,
        elapsed_ms: row.elapsed_ms,
        first_output_ms: row.first_output_ms,
        terminal_elapsed_ms: row.terminal_elapsed_ms,
        output_tokens_per_second,
        status: row.status,
        outcome,
        input_tokens: row.input_tokens,
        uncached_input_tokens: row.uncached_input_tokens,
        output_tokens: row.output_tokens,
        reasoning_output_tokens: row.reasoning_output_tokens,
        cached_input_tokens: row.cached_input_tokens,
        cache_read_input_tokens: row.cache_read_input_tokens,
        cache_creation_input_tokens: row.cache_creation_input_tokens,
        usage_reported: row.usage_reported,
        usage_complete: row.usage_complete,
    })
}
