//! Native runtime composition maps model ownership into a credential-blind gateway injection.
use super::*;
use velune_agent_runtime::native::SessionAgent;
/// Preserve whether the native operation was rejected, may have completed,
/// or already created a target, so callers never create a duplicate fallback.
pub(super) fn fork_failure(error: velune_agent_runtime::native::Error) -> RuntimeError {
    let mut failure =
        crate::Failure::new(crate::FailureKind::Internal, format!("会话分叉：{error}"));
    failure.code = error.code().into();
    RuntimeError::Contract(failure)
}

impl CoreRuntime {
    pub(super) fn interaction_is_visible(
        &self,
        conversation_id: &str,
        interaction: &crate::conversation::RuntimeInteraction,
    ) -> bool {
        !self
            .organization
            .conversations
            .iter()
            .find(|marks| marks.conversation_id == conversation_id)
            .is_some_and(|marks| marks.resolved_async_questions.contains(&interaction.id))
    }
    pub(super) fn retry_async_question_marks(&mut self) {
        if self
            .async_question_marks_retry_at
            .is_some_and(|at| std::time::Instant::now() >= at)
        {
            self.async_question_marks_retry_at = match self
                .repository
                .store_conversation_organization(&self.organization)
            {
                Ok(()) => None,
                Err(error) => {
                    tracing::warn!(event="async_question_marks_store_failed", %error);
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(5))
                }
            };
        }
    }
    pub(super) fn native_injection(
        &self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<GatewayInjection, RuntimeError> {
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let resolved = gateway
            .resolve_dispatch(key)
            .map_err(RuntimeError::invalid)?;
        let provider = resolved.provider();
        let model = resolved.model();
        let ingress = crate::config::runtime_ingress(runtime.type_id, &provider.protocol)
            .ok_or_else(|| RuntimeError::invalid("不支持的运行时版本类型"))?;
        let protocol = match &ingress {
            Protocol::ChatCompletions => "openai-completions",
            Protocol::Responses => "openai-responses",
            Protocol::Messages => "anthropic-messages",
            Protocol::Gemini => return Err(RuntimeError::invalid("专用适配器不使用 Gemini 入口")),
        };
        let runner = self
            .session
            .gateway_runner
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let endpoint = self
            .session
            .gateway_tunnel
            .as_ref()
            .map_or(runner.endpoint(), |tunnel| tunnel.endpoint());
        Ok(GatewayInjection {
            endpoint: if ingress == Protocol::Messages {
                endpoint
                    .strip_suffix("/v1")
                    .expect("gateway base path")
                    .into()
            } else {
                endpoint.into()
            },
            token: runner.token().into(),
            model_alias: model.provider_model_id.clone(),
            protocol: protocol.into(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            reasoning_levels: model.reasoning_levels.clone(),
        })
    }
    /// The turn's model under its provider model ID, and each sub-agent
    /// definition enabled on the runtime that names a native model ID, under
    /// that ID, when exactly one Velune model has it. The turn's own model
    /// keeps its name.
    pub(super) fn native_aliases(
        &self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let model = gateway
            .resolve_dispatch(key)
            .map_err(RuntimeError::invalid)?
            .model();
        let mut aliases = BTreeMap::from([(model.provider_model_id.clone(), key.into())]);
        for template in self.enabled_subagents(runtime) {
            let Some(crate::SubagentModel::Native(id)) = template.content.subagent_model() else {
                continue;
            };
            let mut matching = gateway
                .providers
                .iter()
                .flat_map(|provider| &provider.models)
                .filter(|model| model.provider_model_id == id);
            if let (Some(model), None) = (matching.next(), matching.next()) {
                aliases
                    .entry(id.to_owned())
                    .or_insert_with(|| model.record_key.clone());
            }
        }
        Ok(aliases)
    }

    /// The library's sub-agent definitions enabled on a runtime.
    fn enabled_subagents(&self, runtime: &RuntimeInstance) -> Vec<&crate::ResourceTemplate> {
        self.resource_bindings
            .iter()
            .filter(|binding| binding.runtime_id == runtime.id)
            .filter_map(|binding| {
                self.resource_templates
                    .iter()
                    .find(|template| template.id == binding.template_id)
            })
            .filter(|template| template.content.subagent_model().is_some())
            .collect()
    }

    /// The enabled sub-agents on a Velune model, supplied to a turn through
    /// the runtime's gateway, which routes every Velune model by record key.
    pub(super) fn turn_subagents(&self, runtime: &RuntimeInstance) -> Vec<SessionAgent> {
        let Some(gateway) = self.gateways.iter().find(|g| g.id == runtime.gateway_id) else {
            return Vec::new();
        };
        self.enabled_subagents(runtime)
            .into_iter()
            .filter_map(|template| {
                let crate::ResourceTemplateContent::Subagent {
                    description,
                    instructions,
                    velune_model: Some(key),
                    tools,
                    effort,
                    skills,
                    ..
                } = &template.content
                else {
                    return None;
                };
                // A model removed since, or kept by another gateway, leaves the agent out.
                gateway
                    .providers
                    .iter()
                    .flat_map(|provider| &provider.models)
                    .any(|model| &model.record_key == key)
                    .then(|| SessionAgent {
                        name: template.name.clone(),
                        description: description.clone(),
                        instructions: instructions.clone(),
                        model: Some(format!("velune/model/{key}")),
                        effort: effort.clone(),
                        tools: tools.clone(),
                        skills: skills.clone(),
                    })
            })
            .collect()
    }
    pub(super) fn active_instance(&self) -> Result<&RuntimeInstance, RuntimeError> {
        if let Some(configuration) = &self.session.execution_configuration {
            return Ok(&configuration.runtime);
        }
        let id = match &self.session.active_state {
            ActiveState::History(snapshot) => &snapshot.conversation.runtime_id,
            ActiveState::Native(_)
            | ActiveState::Acp(_)
            | ActiveState::Pi(_)
            | ActiveState::Pending(_) => self
                .session
                .execution_runtime_id
                .as_ref()
                .ok_or_else(|| RuntimeError::invalid("active runtime"))?,
            ActiveState::Empty => return Err(RuntimeError::invalid("active runtime")),
        };
        self.runtime_instances
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| RuntimeError::invalid("active runtime"))
    }
    pub(super) fn history_config(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<HistoryConfig, RuntimeError> {
        let config = self.history_configuration(runtime)?;
        self.prepare_execution_context(runtime)?;
        Ok(config)
    }

    /// The exact reader configuration, resolved without preparing processes or resources.
    pub(super) fn history_configuration(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<HistoryConfig, RuntimeError> {
        let sessions = match runtime.type_id {
            RuntimeKind::ClaudeCode => "projects",
            RuntimeKind::Codex166 | RuntimeKind::DeepSeek => "sessions",
            RuntimeKind::Pi | RuntimeKind::Acp => {
                return Err(RuntimeError::invalid("native history type"));
            }
        };
        let context = self.execution_context(runtime)?;
        let home = context.agent_dir;
        Ok(HistoryConfig {
            kind: runtime.type_id,
            binary: setting_path(runtime, "binary")?,
            node_binary: setting_path_optional(runtime, "nodeBinary").unwrap_or_default(),
            resources_directory: context.resources,
            execution: context.target,
            os_home: context.os_home,
            root: home.join(sessions),
            home,
        })
    }
    pub(super) fn native_create(
        &mut self,
        cwd: Option<&Path>,
        key: &str,
        project: Option<
            &velune_agent_runtime::creation::Retained<
                velune_agent_runtime::native::codex_projects::NativeProject,
            >,
        >,
    ) -> Result<(), RuntimeError> {
        let runtime = self.active_instance()?.clone();
        let side_context = self.session.side.as_ref().map(|side| side.context.clone());
        let reference_context = self.session.reference_context.take();
        if let ActiveState::Native(session) = &mut self.session.active_state {
            if let Some(context) = side_context {
                session
                    .create_side_context(cwd, &runtime.id, &context)
                    .map_err(|error| RuntimeError::context("侧聊上下文创建", error))?;
            } else {
                if let Some(context) = reference_context {
                    session
                        .reference_context(context)
                        .map_err(|error| RuntimeError::context("编辑引用上下文", error))?;
                }
                match project {
                    Some(project) => session.create_in_project(cwd, &runtime.id, project),
                    None => session.create(cwd, &runtime.id),
                }
                .map_err(|error| RuntimeError::context("会话创建失败", error))?;
            }
        }
        if runtime.type_id == RuntimeKind::DeepSeek {
            // ACP new flushes its native header before replying, but does not
            // return its date. Read the existing source adapter rather than
            // inventing a timestamp or reimplementing the persistence format.
            let id = self
                .current_snapshot()
                .expect("created native session")
                .conversation
                .id;
            let conversation = self
                .conversation_summary(&runtime, &id)?
                .ok_or_else(|| RuntimeError::invalid("新建会话的原生元数据不可用"))?;
            if let ActiveState::Native(session) = &mut self.session.active_state {
                session.set_conversation(conversation.into_retained()?);
            }
        }
        self.session.model_record_key = Some(key.into());
        Ok(())
    }
    pub(super) fn native_fork_source(
        &mut self,
        source: &crate::conversation::ConversationSnapshot,
        key: &str,
        through_turn: &str,
    ) -> Result<(), RuntimeError> {
        let runtime = self.active_instance()?.clone();
        let source_id = source
            .conversation
            .id
            .strip_prefix(&format!("{}:", runtime.id))
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let ActiveState::Native(session) = &mut self.session.active_state else {
            return Err(RuntimeError::invalid("分叉执行尚未准备"));
        };
        session
            .fork_codex_from(
                source_id,
                source.conversation.cwd.as_deref().map(Path::new),
                &runtime.id,
                through_turn,
            )
            .map_err(fork_failure)?;
        self.session.model_record_key = Some(key.into());
        Ok(())
    }

    pub(super) fn native_resume(
        &mut self,
        snapshot: &velune_agent_runtime::creation::Retained<
            crate::conversation::ConversationSnapshot,
        >,
        key: &str,
    ) -> Result<(), RuntimeError> {
        let runtime = self.active_instance()?.clone();
        let native_id = snapshot
            .conversation
            .id
            .strip_prefix(&format!("{}:", runtime.id))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
        let side = self.session.side.is_some();
        if let ActiveState::Native(session) = &mut self.session.active_state {
            if side {
                session
                    .open_side(native_id, cwd.as_deref(), &runtime.id)
                    .map_err(|error| RuntimeError::context("侧聊打开失败", error))?;
            } else {
                let history = snapshot
                    .try_clone()
                    .map_err(|error| RuntimeError::context("会话历史保留额度", error))?
                    .map(|snapshot| snapshot.messages);
                telemetry::observe(self.telemetry.as_ref(), "harness.resume", || {
                    session.open(native_id, cwd.as_deref(), &runtime.id, history)
                })
                .map_err(|error| {
                    if error.code() == "control_held_elsewhere" {
                        let mut failure = crate::Failure::new(
                            crate::FailureKind::Conflict,
                            "另一个客户端正在控制此会话；草稿已保留，未自动重发",
                        );
                        failure.code = error.code().into();
                        RuntimeError::Contract(failure)
                    } else {
                        RuntimeError::context("会话恢复失败", error)
                    }
                })?;
                session.restore_context(snapshot).map_err(|error| {
                    RuntimeError::context("native context retained credit", error)
                })?;
            }
        }
        self.session.model_record_key = Some(key.into());
        Ok(())
    }
    pub(crate) fn reply_runtime_interaction(
        &mut self,
        runtime_id: &str,
        id: &str,
        reply: velune_conversation::RuntimeInteractionReply,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        self.ensure_active(runtime_id)?;
        if let Some(snapshot) = self.current_snapshot()
            && let Some(interaction) = snapshot
                .pending_interactions
                .iter()
                .find(|interaction| interaction.id == id)
            && let velune_conversation::InteractionKind::AsyncUserInput {
                questions,
                delivery: velune_conversation::AsyncQuestionDelivery::ConversationInput,
            } = &interaction.kind
        {
            let text = async_answer_text(questions, &reply)?;
            let mut accepted_error = None;
            if let Some(text) = text {
                let running = self.turn_running();
                let dispatched =
                    if running && snapshot.follow_up.can_steer && !self.session.compacting {
                        self.steer_turn(&async_turn_request(&snapshot, None, text).turn, None)
                    } else {
                        let queued = running || self.session.compacting;
                        if queued && !snapshot.follow_up.can_queue {
                            return Err(RuntimeError::invalid("当前会话暂时不能接收回答"));
                        }
                        let selection = self.async_question_selection(&snapshot)?;
                        let request = async_turn_request(&snapshot, selection, text);
                        if queued {
                            self.send_queue(
                                &request.conversation_id,
                                crate::SendQueueOperation::Add {
                                    source_runtime_id: request.source_runtime_id,
                                    target_runtime_id: request.runtime_id,
                                    selection: request.selection,
                                    turn: request.turn,
                                },
                            )
                        } else {
                            self.send_turn(request)
                        }
                    };
                if let Err(error) = dispatched {
                    if !error.delivery_accepted() {
                        return Err(error);
                    }
                    accepted_error = Some(error);
                }
            }
            let mut next = self.organization.clone();
            let index = next
                .conversations
                .iter()
                .position(|marks| marks.conversation_id == snapshot.conversation.id)
                .unwrap_or_else(|| {
                    next.conversations.push(crate::config::ConversationMarks {
                        conversation_id: snapshot.conversation.id.clone(),
                        ..Default::default()
                    });
                    next.conversations.len() - 1
                });
            if !next.conversations[index]
                .resolved_async_questions
                .iter()
                .any(|known| known == id)
            {
                next.conversations[index]
                    .resolved_async_questions
                    .push(id.to_owned());
            }
            // Preserve the accepted/dismissed state in memory if its persisted
            // presentation mark fails, and retry only that mark, not the input.
            let stored = self.repository.store_conversation_organization(&next);
            self.organization = next;
            self.async_question_marks_retry_at = stored
                .as_ref()
                .err()
                .map(|_| std::time::Instant::now() + std::time::Duration::from_secs(5));
            stored.map_err(|error| {
                RuntimeError::context("回答已处理，但无法保存问题卡片状态；请勿重复发送", error)
            })?;
            if let Some(error) = accepted_error {
                return Err(error);
            }
            return self.snapshot_result();
        }
        if self.try_start_pi_control(Some((id, reply.clone())))? {
            return self.snapshot_result();
        }
        if self.start_native_control(velune_agent_runtime::native::CodexCommand::Reply {
            interaction_id: id.to_owned(),
            reply: reply.clone(),
        })? {
            return self.snapshot_result();
        }
        let id = id.to_owned();
        self.start_live_operation("reply", move |state| {
            state
                .live_mut()
                .ok_or_else(|| RuntimeError::invalid("此运行时没有待回复的请求"))?
                .reply(&id, reply)
                .map_err(|error| {
                    RuntimeError::runtime_operation(
                        "运行时请求回复失败；请求可能已取消或所属会话已改变",
                        error,
                    )
                })
        })?;
        if !matches!(self.session.active_state, ActiveState::Pending(_)) {
            self.drain_runtime()?;
        }
        self.snapshot_result()
    }

    fn async_question_selection(
        &mut self,
        snapshot: &crate::conversation::ConversationSnapshot,
    ) -> Result<Option<crate::ModelSelection>, RuntimeError> {
        if let Some(selection) = self.session.model_selection.clone() {
            return Ok(selection);
        }
        if let Some(selection) = self.history_send_selection(&snapshot.context_runtime_id)? {
            return Ok(selection);
        }
        // A question restored from native history need not have a Velune
        // send record. Resolve its recorded model just as an ordinary next
        // turn does, rather than waiting for a preparation that never occurs.
        let runtime = self.runtime_instance(&snapshot.context_runtime_id)?.clone();
        let mut input = self.conversation_next_turn_input(&runtime, Some(snapshot), None)?;
        let (catalog, _catalog_retention) = self.runtime_models(runtime.id.clone())?.into_parts();
        input.catalog = Some(catalog);
        // Answer the question in this conversation's execution context. Its
        // missing facts must not borrow another conversation's last choice or
        // the instance's new-conversation policy.
        input.runtime_choices = None;
        input.new_conversation = crate::config::NewConversationChoice::Chosen {
            choice: Default::default(),
        };
        let next = velune_application_api::resolve_next_turn(input);
        let selection = next
            .has_current
            .then_some(next.model)
            .flatten()
            .ok_or_else(|| {
                RuntimeError::invalid(
                    "无法确认此问题所属会话的模型；请先在输入框选择模型并发送，或忽略此问题",
                )
            })?;
        if runtime.type_id == RuntimeKind::Codex166 {
            // A raw model ID may exist in both native and managed catalogs.
            // Answering a restored question must not treat that match as an
            // instruction to switch providers and create another context.
            let native = self
                .native_snapshot()
                .ok_or_else(|| RuntimeError::invalid("问题所属的原生会话不可用"))?;
            let native_id = native
                .conversation
                .id
                .strip_prefix(&format!("{}:", runtime.id))
                .ok_or_else(|| RuntimeError::invalid("问题所属的 Codex 会话身份无效"))?;
            if !velune_agent_runtime::native::resume_provider_matches(
                &self.history_config(&runtime)?,
                native_id,
                matches!(selection, crate::ModelSelection::Managed { .. }),
                native.conversation.cwd.as_deref(),
            )
            .map_err(|error| RuntimeError::context("核验问题所属会话的提供商", error))?
            {
                return Err(RuntimeError::invalid(
                    "无法确认问题所属会话的提供商与模型；未切换提供商或创建新上下文",
                ));
            }
        }
        Ok(Some(selection))
    }
}

pub(super) fn async_turn_request(
    snapshot: &crate::conversation::ConversationSnapshot,
    selection: Option<crate::ModelSelection>,
    text: String,
) -> TurnRequest {
    TurnRequest {
        submission_id: None,
        source_runtime_id: snapshot.conversation.runtime_id.clone(),
        conversation_id: snapshot.conversation.id.clone(),
        runtime_id: snapshot.context_runtime_id.clone(),
        selection,
        turn: crate::ComposedTurn {
            text,
            effort: None,
            permission_mode: None,
            attachments: Vec::new(),
        },
    }
}

/// Async questions have no native RPC response. Send the user's explicit
/// answers with their questions as ordinary input, preserving free-form text.
fn async_answer_text(
    questions: &[velune_conversation::InteractionQuestion],
    reply: &velune_conversation::RuntimeInteractionReply,
) -> Result<Option<String>, RuntimeError> {
    use velune_conversation::RuntimeInteractionReply;
    match reply {
        RuntimeInteractionReply::Cancel => Ok(None),
        RuntimeInteractionReply::Answers { answers } => {
            if answers.len() != questions.len() {
                return Err(RuntimeError::invalid("请回答所有问题"));
            }
            let mut lines = Vec::new();
            for question in questions {
                let matching = answers
                    .iter()
                    .filter(|answer| answer.question_id == question.id)
                    .collect::<Vec<_>>();
                if matching.len() != 1
                    || matching[0].values.is_empty()
                    || matching[0]
                        .values
                        .iter()
                        .any(|value| value.trim().is_empty())
                {
                    return Err(RuntimeError::invalid("答案与当前问题不匹配或为空"));
                }
                lines.push(format!(
                    "{}\n回答：{}",
                    question.text,
                    matching[0].values.join("、")
                ));
            }
            Ok(Some(lines.join("\n\n")))
        }
        _ => Err(RuntimeError::invalid("此请求需要用户输入答案")),
    }
}

#[cfg(test)]
mod async_question_tests {
    use super::*;
    use velune_conversation::{InteractionAnswer, InteractionQuestion, RuntimeInteractionReply};
    #[test]
    fn answers_preserve_freeform_text_and_reject_partial_or_duplicate_questions() {
        let questions = vec![
            InteractionQuestion {
                id: "0".into(),
                text: "Choose".into(),
                options: vec![],
                secret: false,
                default_text: None,
                multiline: false,
                allow_other: true,
                multiple: false,
            },
            InteractionQuestion {
                id: "1".into(),
                text: "Details".into(),
                options: vec![],
                secret: false,
                default_text: None,
                multiline: false,
                allow_other: true,
                multiple: false,
            },
        ];
        let answer = |id: &str, value: &str| InteractionAnswer {
            question_id: id.into(),
            values: vec![value.into()],
        };
        let reply = RuntimeInteractionReply::Answers {
            answers: vec![answer("0", "A"), answer("1", "my own answer")],
        };
        assert_eq!(
            async_answer_text(&questions, &reply).unwrap(),
            Some("Choose\n回答：A\n\nDetails\n回答：my own answer".into())
        );
        assert!(
            async_answer_text(
                &questions,
                &RuntimeInteractionReply::Answers {
                    answers: vec![answer("0", "A")]
                }
            )
            .is_err()
        );
        assert!(
            async_answer_text(
                &questions,
                &RuntimeInteractionReply::Answers {
                    answers: vec![answer("0", "A"), answer("0", "B")]
                }
            )
            .is_err()
        );
        assert_eq!(
            async_answer_text(&questions, &RuntimeInteractionReply::Cancel).unwrap(),
            None
        );
    }
}

#[cfg(all(test, unix))]
#[path = "native_composition/question_tests.rs"]
mod question_delivery_tests;
