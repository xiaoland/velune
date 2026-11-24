//! Claude Code 2.1.286 stream-json control; provider overrides exist only in the turn child.
use super::*;

/// Levels `--effort` accepts; the CLI applies one per launch and keeps none
/// in the session.
pub const CLAUDE_EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// `--agents` takes the definitions a file's frontmatter would hold, keyed
/// by name, with the instructions as `prompt`.
fn claude_agents(agents: &[SessionAgent]) -> Value {
    Value::Object(
        agents
            .iter()
            .map(|agent| {
                let mut definition = json!({
                    "description": agent.description,
                    "prompt": agent.instructions,
                });
                if let Some(model) = &agent.model {
                    definition["model"] = json!(model);
                }
                if let Some(effort) = agent
                    .effort
                    .as_deref()
                    .filter(|level| crate::management::CLAUDE_EFFORTS.contains(level))
                {
                    definition["effort"] = json!(effort);
                }
                if !agent.tools.is_empty() {
                    definition["tools"] = json!(agent.tools);
                }
                if !agent.skills.is_empty() {
                    definition["skills"] = json!(agent.skills);
                }
                (agent.name.clone(), definition)
            })
            .collect(),
    )
}

/// What only a Claude Code session tracks.
#[derive(Default)]
pub(super) struct State {
    pub(super) commands: super::claude_port::Commands,
    /// Configuration of the child owning the current turn.
    pub(super) launched_effort: Option<String>,
    pub(super) permission_mode: Option<String>,
    /// The turn's result, whether it failed, and the last activity since.
    pub(super) result: Option<(TurnOutcome, std::time::Instant)>,
    pub(super) foreground: Option<ForegroundCommand>,
    pub(super) release_when_idle: bool,
    pub(super) background_followup_capable: bool,
    pub(super) background_followups: BTreeMap<String, BackgroundFollowup>,
    /// A refused response can trigger a native follow-up before the result.
    /// Keep the visible failure until a real completed response recovers it.
    safety_stopped: bool,
    /// An explicit API failure remains pending until native terminal/recovery evidence.
    api_failed: bool,
    /// Background tasks the turn started.
    pub(super) tasks: std::collections::BTreeSet<String>,
    pub(super) task_infos: BTreeMap<String, BackgroundTaskInfo>,
    pub(super) task_order: std::collections::VecDeque<String>,
    pub(super) task_runs: BTreeMap<String, String>,
    pub(super) task_stops: BTreeMap<String, (String, std::time::Instant)>,
    pub(super) task_outputs: BTreeMap<String, (String, bool)>,
    pub(super) task_reads: crate::background::Reads,
    pub(super) task_read_retention: std::collections::VecDeque<(String, DataRetention)>,
    pub(super) task_issue: Option<String>,
    pub(super) task_sequence: u64,
    pub(super) task_stop_capable: bool,
    pub(super) requires_action: bool,
    pub(super) stream_id: Option<String>,
    /// The session selected when this RPC child was launched, which can
    /// differ from a newly represented, not-yet-started snapshot.
    pub(super) bound_session_id: Option<String>,
    /// The model, input tokens (including cache), and final output tokens
    /// of the latest reply; streaming output usage can arrive after the reply.
    pub(super) context: Option<(String, u64, u64)>,
    pub(super) start: ClaudeStart,
    /// Messages steered into turns, for their local identities.
    pub(super) steer_sequence: u64,
    /// A side chat's questions and answers so far, which each question
    /// carries, and the question waiting for its answer.
    pub(super) side_history: Vec<Value>,
    pub(super) side_question: Option<SideQuestion>,
}

pub(super) struct ForegroundCommand {
    id: String,
    result_confirmed: bool,
    command_completed: bool,
}

#[derive(PartialEq, Eq)]
pub(super) enum BackgroundFollowup {
    Running,
    AwaitingResult,
    AwaitingIdle,
}

pub(super) struct SideQuestion {
    request_id: String,
    question: String,
}

/// How the next Claude Code turn child takes up the conversation's session.
#[derive(Clone, Default)]
pub(super) enum ClaudeStart {
    /// Resume the session.
    #[default]
    Resume,
    /// Begin a new session under the conversation's identity.
    Fresh { name: Option<String> },
    /// Resume with the history cut after record `at`: the new prompt follows
    /// it, and what came after stays in the file on another branch.
    ResumeAt { at: String },
    /// Begin the conversation's session as a copy of session `from` up to
    /// record `at`, or of its whole current branch without one; `from` is
    /// left as it is.
    Fork {
        from: String,
        at: Option<String>,
        name: Option<String>,
    },
    /// Read the session for a side question and save nothing.
    Side,
    /// A portable side context, with no native session to resume or save.
    SideContext { path: PathBuf },
}

pub(super) fn claude_permission_mode(mode: &str) -> Option<&str> {
    let mode = if mode == "default" { "manual" } else { mode };
    crate::version::CLAUDE_PERMISSION_MODES
        .iter()
        .any(|known| known.id == mode)
        .then_some(mode)
}

fn steer_settings_changed(detail: impl std::fmt::Display) -> Error {
    Error::with_code(
        format!("Claude Code 已确认切换权限，插入消息未发送：{detail}"),
        "steer_settings_changed",
    )
}

impl NativeSession {
    /// Whether a local instance's configuration directory is the CLI default
    /// for the app's own HOME, where the user's own `claude` login lives. A
    /// managed or other HOME keeps its explicit, separately keyed directory.
    pub(super) fn claude_default_config(config: &NativeConfig) -> bool {
        claude_default_directory(
            &config.agent_dir,
            &config.execution,
            config.os_home.as_deref(),
        )
    }
    pub(super) fn claude_command(
        config: &NativeConfig,
        selection: Option<&RuntimeModel>,
        session: Option<(&str, Option<&Path>, &ClaudeStart)>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<Command> {
        let mut command = crate::process_tree::command(&config.binary);
        command
            .env("CLAUDE_CODE_ENTRYPOINT", "velune")
            .env("CLAUDE_CODE_SDK_READS_SESSION_STATE", "1");
        command.args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
        ]);
        if let Some(effort) = effort {
            if !CLAUDE_EFFORT_LEVELS.contains(&effort) {
                return Err(Error::new("Claude Code 不接受此推理强度"));
            }
            command.arg("--effort").arg(effort);
        }
        if let Some(mode) = permission_mode {
            let mode = claude_permission_mode(mode)
                .ok_or_else(|| Error::new("Claude Code 不接受此权限模式"))?;
            command.arg("--permission-mode").arg(mode);
        }
        if let Some((id, cwd, start)) = session {
            if !matches!(start, ClaudeStart::Side | ClaudeStart::SideContext { .. }) {
                // Make the native option available for an explicit later
                // control request. The initial mode stays as selected above.
                command.arg("--allow-dangerously-skip-permissions");
            }
            // `--resume-session-at` and `--fork-session` apply in print mode.
            match start {
                ClaudeStart::Resume => command.arg("--resume").arg(id),
                ClaudeStart::Fresh { name } => {
                    if let Some(name) = name {
                        command.arg("--name").arg(name);
                    }
                    command.arg("--session-id").arg(id)
                }
                ClaudeStart::ResumeAt { at } => command
                    .arg("--resume")
                    .arg(id)
                    .arg("--resume-session-at")
                    .arg(at),
                ClaudeStart::Fork { from, at, name } => {
                    if let Some(name) = name {
                        command.arg("--name").arg(name);
                    }
                    command.arg("--resume").arg(from);
                    if let Some(at) = at {
                        command.arg("--resume-session-at").arg(at);
                    }
                    command.arg("--fork-session").arg("--session-id").arg(id)
                }
                ClaudeStart::Side => command
                    .arg("--resume")
                    .arg(id)
                    .arg("--no-session-persistence"),
                ClaudeStart::SideContext { path } => command
                    .arg("--session-id")
                    .arg(id)
                    .arg("--no-session-persistence")
                    .arg("--append-system-prompt-file")
                    .arg(path),
            };
            if let Some(cwd) = cwd {
                command.current_dir(cwd);
            }
        }
        if let Some(gateway) = &config.gateway {
            if gateway.protocol != "anthropic-messages" {
                return Err(Error::new("Claude Code 需要 Anthropic Messages 网关协议"));
            }
            // The host-managed flag prevents settings.env from replacing the turn's endpoint and auth.
            command
                .env("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1")
                .env(
                    "ANTHROPIC_BASE_URL",
                    gateway
                        .endpoint
                        .strip_suffix("/v1")
                        .unwrap_or(&gateway.endpoint),
                )
                .env("ANTHROPIC_AUTH_TOKEN", &gateway.token)
                .env("ANTHROPIC_API_KEY", "")
                .env("CLAUDE_CODE_OAUTH_TOKEN", "")
                .env("CLAUDE_CODE_USE_BEDROCK", "0")
                .env("CLAUDE_CODE_USE_VERTEX", "0")
                .env("CLAUDE_CODE_USE_FOUNDRY", "0")
                .env("ANTHROPIC_CUSTOM_HEADERS", "")
                .env("ANTHROPIC_MODEL", &gateway.model_alias)
                .env("ANTHROPIC_DEFAULT_SONNET_MODEL", &gateway.model_alias)
                .env("ANTHROPIC_DEFAULT_OPUS_MODEL", &gateway.model_alias)
                .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", &gateway.model_alias)
                .env("ANTHROPIC_SMALL_FAST_MODEL", &gateway.model_alias)
                .env("CLAUDE_CODE_SUBAGENT_MODEL", &gateway.model_alias)
                .arg("--model")
                .arg(&gateway.model_alias);
            if !config.session_agents.is_empty() {
                command
                    .arg("--agents")
                    .arg(claude_agents(&config.session_agents).to_string());
            }
        } else {
            command.env_remove("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST");
            if let Some(selection) = selection {
                command.arg("--model").arg(&selection.id);
            }
        }
        Ok(command)
    }
    pub(super) fn claude_initialize(&mut self) -> Result<()> {
        let response = self
            .rpc
            .claude_control(json!({"subtype":"initialize","hooks":{}}))?;
        let (models, retention) = response
            .map(|mut value| {
                value["models"]
                    .take()
                    .as_array_mut()
                    .map(std::mem::take)
                    .unwrap_or_default()
            })
            .into_parts();
        self.available_models = models;
        self.data.insert(DataOwner::Catalog, retention);
        // Velune's services are optional for the conversation.
        if !self.config.mcp_servers.is_empty()
            && let Err(error) = self.claude_add_mcp_servers()
        {
            tracing::warn!(%error, "Claude did not accept Velune's MCP services; running without them");
        }
        Ok(())
    }
    fn claude_add_mcp_servers(&mut self) -> Result<()> {
        let existing = self.rpc.claude_control(json!({"subtype":"mcp_status"}))?;
        let existing = existing["mcpServers"]
            .as_array()
            .ok_or_else(|| Error::new("Claude MCP 服务目录无效"))?;
        for server in &self.config.mcp_servers {
            if existing.iter().any(|value| value["name"] == server.name) {
                return Err(Error::new("临时 MCP 服务与 Claude 既有服务名称冲突"));
            }
        }
        // The fixed CLI's dynamic host control merges these servers without writing .mcp.json.
        // Sending headers on stdin also keeps the lease out of process arguments and files.
        let servers: serde_json::Map<String, Value> = self
            .config
            .mcp_servers
            .iter()
            .map(|server| (server.name.clone(), server.http()))
            .collect();
        let result = self
            .rpc
            .claude_control(json!({"subtype":"mcp_set_servers","servers":servers}))?;
        if result["errors"]
            .as_object()
            .is_none_or(|errors| !errors.is_empty())
        {
            return Err(Error::new("Claude 未能添加临时 MCP 服务"));
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let status = self.rpc.claude_control(json!({"subtype":"mcp_status"}))?;
            let statuses = status["mcpServers"]
                .as_array()
                .ok_or_else(|| Error::new("Claude MCP 服务状态无效"))?;
            let mut ready = true;
            for server in &self.config.mcp_servers {
                match statuses
                    .iter()
                    .find(|value| value["name"] == server.name)
                    .and_then(|value| value["status"].as_str())
                {
                    Some("connected") => {}
                    Some("pending") => ready = false,
                    _ => return Err(Error::new("Claude 临时 MCP 服务未能连接")),
                }
            }
            if ready {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::new("Claude 临时 MCP 服务连接超时"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(())
    }
    /// Claude Code 2.1.286 lists `supportedEffortLevels` for models that
    /// `supportsEffort`.
    pub(super) fn claude_models(&self) -> Result<Retained<Vec<ModelOption>>> {
        let retention = self
            .data
            .try_clone(|owner| matches!(owner, DataOwner::Catalog))?;
        Ok(Retained::from_parts(
            self.available_models
                .iter()
                .filter_map(|model| {
                    let id = model["value"].as_str()?;
                    let efforts = if model["supportsEffort"] == true {
                        model["supportedEffortLevels"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|level| level.as_str().map(str::to_owned))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    Some(ModelOption {
                        model: RuntimeModel {
                            provider_id: None,
                            id: id.into(),
                            name: model["displayName"].as_str().unwrap_or(id).into(),
                        },
                        reasoning_efforts: efforts,
                    })
                })
                .collect(),
            retention,
        ))
    }
    pub(super) fn claude_create(&mut self, cwd: Option<&Path>, runtime_id: &str) -> Result<()> {
        let id = claude_uuid()?;
        let cwd = cwd
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.default_cwd.clone());
        self.replace_snapshot(&id, Some(&cwd), runtime_id, Vec::new());
        self.claude.start = ClaudeStart::Fresh { name: None };
        Ok(())
    }
    /// The next turn child resumes at the record before the message; a fork
    /// copies the session up to there under a new identity, which the
    /// first turn creates.
    pub(super) fn claude_branch(&mut self, index: usize, placement: EditPlacement) -> Result<()> {
        let from = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("没有原生会话身份"))?;
        let snapshot = self.snapshot.as_ref().expect("branched snapshot");
        let at = snapshot.messages[index]
            .branch_point
            .clone()
            .ok_or_else(|| Error::new("Claude Code 不能替换会话的第一条消息"))?;
        if snapshot.messages[index + 1..]
            .iter()
            .flat_map(|message| &message.blocks)
            .any(|block| matches!(block, MessageBlock::ContextCompaction { .. }))
        {
            // Even a retained UUID can replay a newer compact summary that
            // describes the message being replaced or the discarded suffix.
            return Err(Error::with_code(
                "所选消息之后有 Claude 上下文压缩，需要从原始历史重建",
                "claude_replay_boundary_unavailable",
            ));
        }
        self.claude_require_replay_boundary(from, &at)?;
        match placement {
            EditPlacement::InPlace => {
                self.claude.start = ClaudeStart::ResumeAt { at };
                self.truncate_snapshot(index);
            }
            EditPlacement::Fork => {
                let from = from.to_owned();
                let id = claude_uuid()?;
                let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
                let runtime_id = snapshot.conversation.runtime_id.clone();
                let history = snapshot.messages[..index].to_vec();
                self.replace_snapshot(&id, cwd.as_deref(), &runtime_id, history);
                self.claude.start = ClaudeStart::Fork {
                    from,
                    at: Some(at),
                    name: None,
                };
            }
        }
        Ok(())
    }
    /// The fork's first turn child copies the session up to the record
    /// before the next user message after `index`, or through the current
    /// branch's last record. Pin even the last turn: the source can receive
    /// another turn before this fork's first child is launched.
    pub(super) fn claude_fork(&mut self, index: usize) -> Result<()> {
        let snapshot = self.snapshot.as_ref().expect("forked snapshot");
        let from = self
            .native_id
            .clone()
            .ok_or_else(|| Error::new("没有原生会话身份"))?;
        let next = snapshot.messages[index + 1..]
            .iter()
            .position(|message| message.role == MessageRole::User)
            .map(|offset| index + 1 + offset);
        let at = next
            .map(|next| {
                snapshot.messages[next]
                    .branch_point
                    .clone()
                    .ok_or_else(|| Error::new("Claude Code 不能在该处分叉会话"))
            })
            .transpose()?
            .or_else(|| snapshot.messages.last().map(|message| message.id.clone()));
        if let Some(at) = &at {
            self.claude_require_replay_boundary(&from, at)?;
        }
        let id = claude_uuid()?;
        let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
        let runtime_id = snapshot.conversation.runtime_id.clone();
        let history = snapshot.messages[..next.unwrap_or(snapshot.messages.len())].to_vec();
        self.replace_snapshot(&id, cwd.as_deref(), &runtime_id, history);
        self.claude.start = ClaudeStart::Fork {
            from,
            at,
            name: None,
        };
        Ok(())
    }

    /// `--resume-session-at` looks up the boundary after Claude loads its
    /// current replay chain. A UUID in the transcript can have been compacted
    /// away. Check before changing the target or truncating the live view.
    fn claude_require_replay_boundary(&self, from: &str, at: &str) -> Result<()> {
        if !crate::history::claude_replay_contains(&self.claude_history_config(), from, at)? {
            return Err(Error::with_code(
                "所选位置已移出 Claude 当前原生上下文，需要从历史重建",
                "claude_replay_boundary_unavailable",
            ));
        }
        Ok(())
    }

    pub(super) fn claude_open(
        &mut self,
        id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
        history: Vec<Message>,
    ) -> Result<()> {
        let empty_import = if history.is_empty() && !id.contains('/') {
            if let Some(cwd) = cwd {
                let path = crate::import::claude_project_directory(&self.config.agent_dir, cwd)
                    .join(format!("{id}.jsonl"));
                let bytes = self.config.execution.read_file(&path)?;
                claude_empty_import(&bytes, id, cwd)
            } else {
                false
            }
        } else {
            false
        };
        self.replace_snapshot(id, cwd, runtime_id, history);
        self.claude.start = if empty_import {
            ClaudeStart::Fresh { name: None }
        } else {
            ClaudeStart::Resume
        };
        Ok(())
    }
    /// A side chat answers each question in a child that reads the session
    /// and saves nothing; Velune keeps what was asked and answered.
    pub(super) fn claude_open_side(
        &mut self,
        id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
    ) -> Result<()> {
        self.replace_snapshot(id, cwd, runtime_id, Vec::new());
        self.claude.start = ClaudeStart::Side;
        self.claude.side_history.clear();
        self.claude.side_question = None;
        Ok(())
    }
    /// Claude Code's own side question (`/btw`): answered on the session's
    /// context and the side chat's earlier exchanges, without tools.
    fn claude_side_send(&mut self, input: &crate::TurnInput) -> Result<()> {
        if !input.images.is_empty() {
            return Err(Error::new("Claude Code 侧聊不接受图片"));
        }
        self.claude.steer_sequence = self
            .claude
            .steer_sequence
            .checked_add(1)
            .ok_or_else(|| Error::new("Claude message sequence exhausted"))?;
        let request_id = format!(
            "velune-side-{}-{}-{}",
            self.epoch, self.claude.commands.generation, self.claude.steer_sequence
        );
        self.claude_write(
            &json!({"type":"control_request","request_id":request_id,"request":{
            "subtype":"side_question","question":input.text,"history":self.claude.side_history}}),
        )?;
        self.claude.side_question = Some(SideQuestion {
            request_id,
            question: input.text.clone(),
        });
        self.turn_id = Some(format!(
            "{}:{}",
            self.epoch,
            self.snapshot.as_ref().map_or(0, |s| s.revision)
        ));
        self.message(Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            completed: true,
            id: format!("{}:user", self.turn_id.as_deref().unwrap_or_default()),
            role: MessageRole::User,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Text {
                text: input.text.clone(),
            }],
        });
        self.running();
        Ok(())
    }
    /// The answer to the side question, which ends its turn. The child's
    /// other records belong to no turn.
    fn claude_side_record(&mut self, record: &Value) -> Result<()> {
        if self.claude.commands.enabled && self.failed && !self.claude_has_failed_turn() {
            return Ok(());
        }
        let response = &record["response"];
        let Some(pending) = self.claude.side_question.take_if(|pending| {
            record["type"] == "control_response" && response["request_id"] == pending.request_id
        }) else {
            return Ok(());
        };
        let answer = response["response"]["response"]
            .as_str()
            .filter(|_| response["subtype"] == "success");
        match answer {
            Some(answer) => {
                self.message(Message {
                    turn_settings: None,
                    turn_usage: None,
                    branch_point: None,
                    completed: true,
                    id: format!(
                        "{}:answer",
                        self.claude_received_turn_id().unwrap_or_default()
                    ),
                    role: MessageRole::Assistant,
                    timestamp_unix_ms: None,
                    blocks: vec![MessageBlock::Text {
                        text: answer.into(),
                    }],
                });
                self.data.retain(DataOwner::SideHistory);
                self.claude
                    .side_history
                    .push(json!({"question": pending.question, "response": answer}));
                self.claude_finish_received_outcome(TurnOutcome::Completed);
                self.claude_finish_received_turn(RunState::Idle)
            }
            None => {
                let detail = response["error"]
                    .as_str()
                    .filter(|detail| !detail.is_empty())
                    .unwrap_or("未返回回答");
                self.claude_error(vec![MessageBlock::Notice {
                    text: format!("Claude Code 未回答侧聊问题：{detail}"),
                }]);
                self.claude_finish_received_outcome(TurnOutcome::Failed);
                self.claude_finish_received_turn(RunState::Failed)
            }
        }
    }
    pub(super) fn claude_send(&mut self, input: &crate::TurnInput) -> Result<()> {
        if self.side {
            return self.claude_side_send(input);
        }
        let text = input.text.as_str();
        // Images go before the text, as Anthropic recommends.
        let content = input
            .images
            .iter()
            .map(|image| json!({"type":"image","source":{"type":"base64","media_type":image.kind.media_type(),"data":image.base64()}}))
            .chain((!text.is_empty()).then(|| json!({"type":"text","text":text})))
            .collect::<Vec<_>>();
        // Claude persists the UUID supplied on an ordinary input. Use it on
        // the live row so history reloads keep the same edit and fork target.
        let id = claude_uuid()?;
        self.claude_write(&json!({"type":"user","uuid":id,"session_id":self.native_id,"parent_tool_use_id":null,"message":{"role":"user","content":content}}))?;
        self.compaction_message_id = None;
        self.claude.result = None;
        self.claude.commands.result_is_terminal = false;
        self.claude.safety_stopped = false;
        self.claude.api_failed = false;
        self.claude.foreground = Some(ForegroundCommand {
            id: id.clone(),
            result_confirmed: false,
            command_completed: false,
        });
        self.claude.requires_action = false;
        self.claude.stream_id = None;
        self.turn_id = Some(format!(
            "{}:{}",
            self.epoch,
            self.snapshot.as_ref().map_or(0, |s| s.revision)
        ));
        if !content.is_empty() {
            self.message(Message {
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
                completed: true,
                id,
                role: MessageRole::User,
                timestamp_unix_ms: None,
                blocks: input
                    .images
                    .iter()
                    .map(|image| MessageBlock::Image {
                        media_type: Some(image.kind.media_type().into()),
                        source: crate::conversation::ImageSource::Path {
                            path: image.path.to_string_lossy().into_owned(),
                        },
                    })
                    .chain((!text.is_empty()).then(|| MessageBlock::Text { text: text.into() }))
                    .collect(),
            });
        }
        self.claude.start = ClaudeStart::Resume;
        self.running();
        Ok(())
    }
    /// A user message written while a turn runs joins that turn: Claude Code
    /// takes it in at its next step instead of starting another.
    pub(super) fn claude_steer(&mut self, input: &crate::TurnInput) -> Result<()> {
        if !self.claude.commands.enabled {
            self.claude_poll_steer_boundary()?;
        }
        let turn_id = self
            .turn_id
            .clone()
            .filter(|_| self.claude_can_insert())
            .ok_or_else(|| Error::new("Claude Code 当前轮次不能插入消息"))?;
        if let Some(effort) = input.effort.as_deref()
            && self.claude.launched_effort.as_deref() != Some(effort)
        {
            return Err(Error::with_code(
                "Claude Code 当前轮次无法更改推理强度，请排队发送",
                "steer_settings_unsupported",
            ));
        }
        let permission_mode = input
            .permission_mode
            .as_deref()
            .map(|mode| {
                claude_permission_mode(mode)
                    .ok_or_else(|| Error::new("Claude Code 不接受此权限模式"))
            })
            .transpose()?
            .filter(|mode| self.claude.permission_mode.as_deref() != Some(*mode));
        if let Some(mode) = permission_mode {
            if self.claude.commands.enabled {
                return Err(Error::with_code(
                    "Claude Code 异步命令接口目前无法随插入消息更改权限，请排队发送",
                    "steer_settings_unsupported",
                ));
            }
            self.claude_set_steer_permission(mode)?;
            self.claude_poll_steer_boundary()
                .map_err(steer_settings_changed)?;
            if self.turn_id.as_deref() != Some(turn_id.as_str()) || !self.claude_can_insert() {
                return Err(steer_settings_changed("原轮次已结束或不可插入"));
            }
            if self.claude.permission_mode.as_deref() != Some(mode) {
                return Err(steer_settings_changed("运行时报告权限模式已再次变化"));
            }
        }
        let result = self.claude_write_steer(input);
        match (permission_mode, result) {
            (Some(_), Err(error)) if matches!(error.code(), "transport" | "timeout") => {
                Err(Error::with_code(
                    format!(
                        "Claude Code 已确认切换权限。插入消息交付未确认，请查看会话记录：{error}"
                    ),
                    error.code(),
                ))
            }
            (Some(_), Err(error)) => Err(steer_settings_changed(error)),
            (_, result) => result,
        }
    }

    fn claude_poll_steer_boundary(&mut self) -> Result<()> {
        self.poll()?;
        self.rpc
            .check_failure()
            .inspect_err(|_| self.fail_connection())
    }

    fn claude_can_insert(&self) -> bool {
        self.can_steer()
            && !self.failed
            && !self.process_ended
            && !self.compacting
            && !self.claude.commands.result_is_terminal
            && self.claude.bound_session_id.is_some()
            && self.claude.bound_session_id == self.native_id
    }

    fn claude_set_steer_permission(&mut self, mode: &str) -> Result<()> {
        let method = "set_permission_mode";
        let deadline = std::time::Instant::now() + rpc::CONTROL_TIMEOUT;
        let result = rpc::observe_request(method, || {
            let id = self.rpc.begin_claude_control(
                &json!({"subtype":method,"mode":mode}),
                crate::transport::WriteClass::Ordinary,
                deadline,
            )?;
            loop {
                let record = self.rpc.receive(method, deadline)?;
                if record["type"] == "control_response" && record["response"]["request_id"] == id {
                    let response = &record["response"];
                    return match response["subtype"].as_str() {
                        Some("success") => Ok(()),
                        Some("error") => {
                            let reason = response["error"]
                                .as_str()
                                .filter(|text| !text.is_empty())
                                .map(|text| text.chars().take(512).collect::<String>())
                                .unwrap_or_else(|| "运行时未提供原因".into());
                            Err(Error::with_code(
                                format!("Claude Code 拒绝切换权限，插入消息未发送：{reason}"),
                                "steer_permission_rejected",
                            ))
                        }
                        _ => Err(Error::with_code("Claude 权限控制回执无效", "transport")),
                    };
                }
                self.claude_receive_record(record)?;
            }
        });
        match result {
            Ok(()) => {
                self.claude.permission_mode = Some(mode.into());
                Ok(())
            }
            Err(error) if error.code() == "steer_permission_rejected" => Err(error),
            Err(error) => {
                if matches!(error.code(), "transport" | "timeout") {
                    self.fail_connection();
                }
                Err(Error::with_code(
                    format!("Claude Code 权限切换未确认，插入消息未发送：{error}"),
                    "steer_permission_unconfirmed",
                ))
            }
        }
    }

    fn claude_write_steer(&mut self, input: &crate::TurnInput) -> Result<()> {
        let text = input.text.as_str();
        let content = input
            .images
            .iter()
            .map(|image| json!({"type":"image","source":{"type":"base64","media_type":image.kind.media_type(),"data":image.base64()}}))
            .chain((!text.is_empty()).then(|| json!({"type":"text","text":text})))
            .collect::<Vec<_>>();
        self.claude.steer_sequence = self
            .claude
            .steer_sequence
            .checked_add(1)
            .ok_or_else(|| Error::new("Claude message sequence exhausted"))?;
        self.claude_write(&json!({"type":"user","session_id":self.native_id,"parent_tool_use_id":null,"message":{"role":"user","content":content}}))?;
        let id = format!(
            "{}:steer:{}",
            self.turn_id.as_deref().unwrap_or_default(),
            self.claude.steer_sequence
        );
        self.message(Message {
            turn_settings: self.claude.permission_mode.clone().map(|permission_mode| {
                crate::conversation::TurnSettings {
                    permission_mode: Some(permission_mode),
                    ..Default::default()
                }
            }),
            turn_usage: None,
            branch_point: None,
            completed: true,
            id,
            role: MessageRole::User,
            timestamp_unix_ms: None,
            blocks: input
                .images
                .iter()
                .map(|image| MessageBlock::Image {
                    media_type: Some(image.kind.media_type().into()),
                    source: crate::conversation::ImageSource::Path {
                        path: image.path.to_string_lossy().into_owned(),
                    },
                })
                .chain((!text.is_empty()).then(|| MessageBlock::Text { text: text.into() }))
                .collect(),
        });
        Ok(())
    }
    pub(super) fn claude_compact(&mut self) -> Result<()> {
        self.claude_write(
            &json!({"type":"user","session_id":self.native_id,"parent_tool_use_id":null,
            "message":{"role":"user","content":[{"type":"text","text":"/compact"}]}}),
        )?;
        self.claude.result = None;
        self.claude.commands.result_is_terminal = false;
        self.claude.safety_stopped = false;
        self.claude.api_failed = false;
        self.compaction_message_id = None;
        self.claude.context = None;
        self.turn_id = Some(format!(
            "{}:compact:{}",
            self.epoch,
            self.snapshot.as_ref().map_or(0, |s| s.revision)
        ));
        self.claude.start = ClaudeStart::Resume;
        self.running();
        Ok(())
    }

    fn claude_compaction_status(&mut self, text: String, completed: bool, native_id: Option<&str>) {
        // Without a preceding `status: compacting`, a new native boundary
        // still starts a new compaction. A provisional row can be renamed to
        // this boundary; a completed native row is history.
        let previous = self.compaction_message_id.clone().filter(|previous| {
            native_id.is_none_or(|id| id == previous)
                || previous.starts_with("claude-compaction:")
                || self.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .messages
                        .iter()
                        .any(|message| &message.id == previous && !message.completed)
                })
        });
        let id = native_id
            .map(str::to_owned)
            .or_else(|| previous.clone())
            .unwrap_or_else(|| {
                format!(
                    "claude-compaction:{}:{}",
                    self.epoch,
                    self.snapshot.as_ref().map_or(0, |s| s.revision)
                )
            });
        if let Some(previous) = previous.filter(|previous| previous != &id)
            && let Some(snapshot) = &mut self.snapshot
        {
            if let Some(message) = snapshot
                .messages
                .iter_mut()
                .find(|message| message.id == previous)
            {
                message.id = id.clone();
            }
            snapshot.message_identity_confirmations.push(
                crate::conversation::MessageIdentityConfirmation {
                    previous_id: previous,
                    current_id: id.clone(),
                },
            );
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .executions
                .confirm(&snapshot.message_identity_confirmations);
        }
        // Status-only frames retain the summary already owned by this boundary.
        let text = if !text.contains('\n') {
            self.snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.messages.iter().find(|message| message.id == id))
                .and_then(|message| {
                    message.blocks.iter().find_map(|block| match block {
                        MessageBlock::ContextCompaction { text } => {
                            text.split_once("\n\n").map(|(_, detail)| detail)
                        }
                        _ => None,
                    })
                })
                .map(|detail| format!("{text}\n\n{detail}"))
                .unwrap_or(text)
        } else {
            text
        };
        self.compaction_message_id = Some(id.clone());
        self.message(Message {
            id,
            completed,
            role: MessageRole::System,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::ContextCompaction { text }],
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
        });
    }

    pub(super) fn claude_cancel(&mut self) -> Result<()> {
        // A side question cannot be interrupted; its child is ended instead.
        if self.side {
            self.rpc.retire();
            self.tunnel = None;
            self.process_ended = true;
            self.claude.side_question = None;
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.executions.finish(Some(TurnOutcome::Cancelled));
            }
            return self.finish_turn(RunState::Idle);
        }
        self.rpc.claude_control_with_class(
            json!({"subtype":"interrupt"}),
            crate::transport::WriteClass::ProtocolReplyCancel,
        )?;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Stopping;
            snapshot.revision += 1;
        }
        Ok(())
    }
    pub(super) fn claude_receive_record(&mut self, record: Retained<Value>) -> Result<()> {
        self.receive_native_record(record, Self::claude_handle_record)
    }
    pub(super) fn claude_handle_record(&mut self, record: Value) -> Result<()> {
        if self.side {
            return self.claude_side_record(&record);
        }
        if record["type"] == "system"
            && matches!(record["subtype"].as_str(), Some("init" | "status"))
            && let Some(mode) = record["permissionMode"]
                .as_str()
                .and_then(claude_permission_mode)
        {
            self.claude.permission_mode = Some(mode.into());
        }
        self.claude_background_record(&record);
        match record["type"].as_str() {
            Some("command_lifecycle") if record["state"] == "completed" => {
                if let Some(command) = self.claude.foreground.as_mut()
                    && record["command_uuid"].as_str() == Some(command.id.as_str())
                {
                    command.command_completed = true;
                    self.claude_finish_background_foreground()?;
                }
            }
            Some("control_cancel_request") => {
                let expired = self
                    .interactions
                    .iter()
                    .find(|(_, pending)| pending.id == record["request_id"])
                    .map(|(id, _)| id.clone());
                if let Some(id) = expired {
                    self.interactions.remove(&id);
                    if let Some(snapshot) = &mut self.snapshot {
                        snapshot
                            .pending_interactions
                            .retain(|interaction| interaction.id != id);
                        snapshot.revision += 1;
                    }
                }
            }
            Some("control_request") => {
                let request = &record["request"];
                let id = record["request_id"].clone();
                if request["subtype"] == "can_use_tool"
                    && self.turn_running()
                    && request["tool_name"] == "AskUserQuestion"
                {
                    let questions = request["input"]["questions"]
                        .as_array()
                        .ok_or_else(|| Error::new("Claude 用户问题格式无效"))?
                        .iter()
                        .map(|question| {
                            let text = question["question"]
                                .as_str()
                                .filter(|text| !text.is_empty())
                                .ok_or_else(|| Error::new("Claude 用户问题为空"))?;
                            let options = question["options"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|option| {
                                    option["label"].as_str().map(|label| InteractionOption {
                                        id: label.into(),
                                        label: label.into(),
                                        approval_kind: None,
                                    })
                                })
                                .collect();
                            Ok(InteractionQuestion {
                                id: text.into(),
                                text: text.into(),
                                options,
                                secret: false,
                                default_text: None,
                                multiline: false,
                                allow_other: true,
                                multiple: question["multiSelect"].as_bool().unwrap_or(false),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if questions.is_empty() {
                        return Err(Error::new("Claude 用户问题为空"));
                    }
                    self.add_interaction(
                        PendingRequest {
                            background: false,
                            id,
                            method: "AskUserQuestion".into(),
                            options: Vec::new(),
                            question_ids: questions.iter().map(|q| q.id.clone()).collect(),
                            payload: request.clone(),
                        },
                        InteractionKind::UserInput { questions },
                    );
                } else if request["subtype"] == "can_use_tool" && self.turn_running() {
                    self.add_interaction(
                        PendingRequest {
                            background: false,
                            id,
                            method: "can_use_tool".into(),
                            options: vec!["allow".into(), "deny".into()],
                            question_ids: Vec::new(),
                            payload: request.clone(),
                        },
                        InteractionKind::Approval {
                            title: format!(
                                "允许 {}？",
                                request["tool_name"].as_str().unwrap_or("工具操作")
                            ),
                            detail: serde_json::to_string(&request["input"]).unwrap_or_default(),
                            options: vec![
                                InteractionOption {
                                    id: "allow".into(),
                                    label: "允许这一次".into(),
                                    approval_kind: Some(ApprovalOptionKind::AllowOnce),
                                },
                                InteractionOption {
                                    id: "deny".into(),
                                    label: "拒绝".into(),
                                    approval_kind: Some(ApprovalOptionKind::RejectOnce),
                                },
                            ],
                        },
                    );
                } else {
                    self.claude_automatic_reply(&json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"Unsupported control request"}}))?;
                }
            }
            Some("system") => match record["subtype"].as_str() {
                Some("status") if record["status"] == "compacting" => {
                    let active = self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.messages.iter().any(|message| {
                            Some(&message.id) == self.compaction_message_id.as_ref()
                                && !message.completed
                        })
                    });
                    if !active {
                        self.compaction_message_id = None;
                    }
                    self.claude_compaction_status("正在压缩上下文".into(), false, None);
                }
                Some("status") if record["compact_result"].is_string() => {
                    let text = if self
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.run_state == RunState::Stopping)
                    {
                        "上下文压缩已取消".into()
                    } else if record["compact_result"] == "success" {
                        "上下文已压缩".into()
                    } else {
                        format!(
                            "上下文压缩失败：{}",
                            record["compact_error"]
                                .as_str()
                                .unwrap_or("运行时未提供原因")
                        )
                    };
                    self.claude_compaction_status(text, true, None);
                }
                Some("compact_boundary") => {
                    // Compaction reports the current context independently of
                    // user-turn billing. Keep it fresh even for manual /compact.
                    let tokens = record["compact_metadata"]["post_tokens"]
                        .as_u64()
                        .or_else(|| record["compactMetadata"]["postTokens"].as_u64());
                    if let Some(tokens) = tokens {
                        self.context_usage(Some(tokens), None);
                    }
                    self.claude_compaction_status(
                        "上下文已压缩".into(),
                        true,
                        record["uuid"].as_str(),
                    );
                }
                Some("session_state_changed") => {
                    self.claude.requires_action = record["state"] == "requires_action";
                    if record["state"] == "idle" {
                        self.claude
                            .background_followups
                            .retain(|_, kind| *kind != BackgroundFollowup::AwaitingIdle);
                        self.claude_finish_received_result()?;
                    }
                }
                Some("task_started")
                    if matches!(
                        record["task_type"].as_str(),
                        Some("local_agent" | "local_workflow")
                    ) =>
                {
                    if let Some(id) = record["task_id"].as_str() {
                        self.claude.tasks.insert(id.into());
                        if record["task_type"] == "local_agent"
                            && let Some(conversation) = self.claude_agent_conversation(id)
                        {
                            self.sub_agents.insert(conversation.clone());
                            // Background observation precedes relationship discovery.
                            // Link the first task receipt immediately; a quiet agent
                            // need not emit a later progress event to become openable.
                            if let Some(task) = self
                                .claude
                                .task_runs
                                .get(id)
                                .and_then(|run| self.claude.task_infos.get_mut(run))
                            {
                                task.conversation_id = Some(conversation);
                            }
                        }
                    }
                }
                Some("task_notification") => {
                    if let Some(id) = record["task_id"].as_str() {
                        if !self.claude_task_run_matches(&record, id) {
                            return Ok(());
                        }
                        self.claude.tasks.remove(id);
                        let conversation = self.claude_agent_conversation(id);
                        if let Some(conversation) =
                            conversation.filter(|c| self.sub_agents.contains(c))
                        {
                            self.message(Message {
                                turn_settings: None,
                                turn_usage: None,
                                branch_point: None,
                                completed: true,
                                id: record["uuid"].as_str().unwrap_or(id).into(),
                                role: MessageRole::Tool,
                                timestamp_unix_ms: None,
                                blocks: vec![MessageBlock::Tool {
                                    tool_id: record["tool_use_id"].as_str().map(str::to_owned),
                                    title: "任务通知".into(),
                                    state: if record["status"] == "completed" {
                                        ToolState::Completed
                                    } else {
                                        ToolState::Failed
                                    },
                                    output: record["summary"].as_str().map(str::to_owned),
                                    conversation_id: Some(conversation),
                                }],
                            });
                        }
                    }
                }
                Some("task_updated")
                    if matches!(
                        record["patch"]["status"].as_str(),
                        Some("completed" | "failed" | "stopped" | "killed")
                    ) =>
                {
                    if let Some(id) = record["task_id"].as_str() {
                        if !self.claude_task_run_matches(&record, id) {
                            return Ok(());
                        }
                        self.claude.tasks.remove(id);
                    }
                }
                _ => {}
            },
            Some("assistant" | "user") => {
                if let Some((_, last_activity)) = &mut self.claude.result {
                    *last_activity = std::time::Instant::now();
                }
                if !record["parent_tool_use_id"].is_string() && record["type"] == "user" {
                    if claude_refused_message(&record) {
                        self.claude.safety_stopped = true;
                        if !self.claude_preserves_terminal_result() {
                            self.claude.commands.result_is_terminal = false;
                            self.claude.result =
                                Some((TurnOutcome::Failed, std::time::Instant::now()));
                        }
                        self.claude_error(claude_refusal_blocks(&record));
                        return Ok(());
                    }
                    if record["isCompactSummary"] == true || record["is_compact_summary"] == true {
                        let summary = claude_plain_text(&record["message"]["content"]);
                        if !summary.is_empty() {
                            self.claude_compaction_status(
                                format!("上下文已压缩\n\n{summary}"),
                                true,
                                None,
                            );
                        }
                        return Ok(());
                    }
                    // Claude 2.1.292's SDK serializer replaces compact/meta flags
                    // with isSynthetic. Resolve summary ownership from native
                    // structured history, never from a user-text prefix.
                    if record["isSynthetic"] == true
                        && claude_blocks(&record["message"]["content"], None)
                            .iter()
                            .all(|block| matches!(block, MessageBlock::Text { .. }))
                    {
                        let has_summary = self.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot.messages.iter().any(|message| {
                                Some(&message.id) == self.compaction_message_id.as_ref()
                                    && message.blocks.iter().any(|block| matches!(block,
                                        MessageBlock::ContextCompaction { text }
                                            if text.split_once("\n\n").is_some_and(|(_, detail)| !detail.trim().is_empty())
                                    ))
                            })
                        });
                        if self.compaction_message_id.is_some() && !has_summary {
                            self.claude_refresh_context();
                        }
                        return Ok(());
                    }
                }
                // A sub-agent's own messages belong to its conversation, as
                // in the native history, not to this transcript.
                if record["parent_tool_use_id"].is_string()
                    || record["isCompactSummary"] == true
                    || record["is_compact_summary"] == true
                    || record["isMeta"] == true
                    || record["is_meta"] == true
                {
                    return Ok(());
                }
                if record["type"] == "assistant" && record["message"]["stop_reason"] == "refusal" {
                    self.claude.safety_stopped = true;
                    if !self.claude_preserves_terminal_result() {
                        self.claude.commands.result_is_terminal = false;
                        self.claude.result = Some((TurnOutcome::Failed, std::time::Instant::now()));
                    }
                    self.claude_error(claude_refusal_blocks(&record));
                    return Ok(());
                }
                if record["type"] == "assistant"
                    && (record["error"].is_string()
                        || record["is_api_error_message"] == true
                        || record["isApiErrorMessage"] == true)
                {
                    self.claude.api_failed = true;
                    if !self.claude_preserves_terminal_result() {
                        self.claude.commands.result_is_terminal = false;
                        self.claude.result = Some((TurnOutcome::Failed, std::time::Instant::now()));
                    }
                    self.claude_error(claude_error_blocks(&record)?);
                    return Ok(());
                }
                if record["type"] == "assistant"
                    && matches!(
                        record["message"]["stop_reason"].as_str(),
                        Some("end_turn" | "stop_sequence")
                    )
                {
                    self.claude.commands.result_is_terminal = false;
                    self.claude.safety_stopped = false;
                    if self.claude.api_failed {
                        self.claude.result =
                            Some((TurnOutcome::Completed, std::time::Instant::now()));
                    }
                    self.claude.api_failed = false;
                    // A native successful response proves recovery; a success
                    // envelope alone must not erase the preceding API failure.
                    let id = format!("{}:error", self.turn_id.as_deref().unwrap_or_default());
                    if let Some(message) = self.snapshot.as_mut().and_then(|snapshot| {
                        snapshot
                            .messages
                            .iter_mut()
                            .find(|message| message.id == id)
                    }) {
                        for block in &mut message.blocks {
                            if let MessageBlock::RuntimeFailure { retry, .. } = block {
                                *retry = Some(RuntimeRetry::Recovered);
                            }
                        }
                    }
                }
                // Local /compact output belongs to maintenance; status/result carry its outcome.
                if self.compacting && record["type"] == "assistant" {
                    return Ok(());
                }
                let agent = record["tool_use_result"]["agentId"]
                    .as_str()
                    .and_then(|agent| self.claude_agent_conversation(agent));
                // What the latest reply's request held: where the context stands.
                let reply = &record["message"];
                if record["type"] == "assistant"
                    && let (Some(model), Some((input, output))) =
                        (reply["model"].as_str(), claude_context(&reply["usage"]))
                    && !model.starts_with('<')
                {
                    self.data.replace(DataOwner::Stream);
                    self.claude.context = Some((model.to_owned(), input, output));
                }
                let role = if record["type"] == "assistant" {
                    MessageRole::Assistant
                } else {
                    MessageRole::Tool
                };
                let id = record["message"]["id"]
                    .as_str()
                    .or_else(|| record["uuid"].as_str())
                    .unwrap_or("message");
                let mut blocks = claude_blocks(&record["message"]["content"], agent);
                if role == MessageRole::Tool {
                    blocks.retain(|result| !self.claude_complete_call(result));
                }
                if !blocks.is_empty() {
                    self.message(Message {
                        turn_settings: None,
                        turn_usage: None,
                        branch_point: None,
                        completed: true,
                        id: id.into(),
                        role,
                        timestamp_unix_ms: None,
                        blocks,
                    });
                }
            }
            Some("stream_event") => {
                if self.compacting {
                    return Ok(());
                }
                if let Some((_, last_activity)) = &mut self.claude.result {
                    *last_activity = std::time::Instant::now();
                }
                let event = &record["event"];
                if event["type"] == "message_start" {
                    self.data.retain(DataOwner::Stream);
                    self.claude.stream_id = event["message"]["id"].as_str().map(str::to_owned);
                }
                if event["type"] == "message_delta"
                    && !record["parent_tool_use_id"].is_string()
                    && let Some(output) = event["usage"]["output_tokens"].as_u64()
                    && let Some((_, _, tokens)) = &mut self.claude.context
                {
                    *tokens = output;
                }
                if event["type"] == "content_block_delta" {
                    let thought = event["delta"]["type"] == "thinking_delta";
                    let delta = if thought {
                        event["delta"]["thinking"].as_str()
                    } else {
                        event["delta"]["text"].as_str()
                    };
                    if let Some(delta) = delta {
                        let id = self.claude.stream_id.clone().unwrap_or_else(|| {
                            format!(
                                "{}:stream:{}",
                                self.turn_id.as_deref().unwrap_or_default(),
                                event["index"]
                            )
                        });
                        let (position, previous) = self.take_message(&id);
                        let mut message = previous.unwrap_or(Message {
                            turn_settings: None,
                            turn_usage: None,
                            branch_point: None,
                            completed: false,
                            id,
                            role: MessageRole::Assistant,
                            timestamp_unix_ms: None,
                            blocks: Vec::new(),
                        });
                        match message.blocks.last_mut() {
                            Some(MessageBlock::Text { text }) if !thought => text.push_str(delta),
                            Some(MessageBlock::Reasoning { text }) if thought => {
                                text.push_str(delta)
                            }
                            _ => message.blocks.push(if thought {
                                MessageBlock::Reasoning { text: delta.into() }
                            } else {
                                MessageBlock::Text { text: delta.into() }
                            }),
                        }
                        self.message_at(message, position);
                    }
                }
            }
            Some("result") => {
                if record["origin"]["kind"] == "task-notification" {
                    if let Some(run) = record["origin"]["runId"].as_str()
                        && let Some(kind) = self.claude.background_followups.get_mut(run)
                    {
                        *kind = BackgroundFollowup::AwaitingIdle;
                    }
                    return Ok(());
                }
                let result_uuid = record["user_message_uuid"].as_str();
                let result_uuids = record["user_message_uuids"].as_array();
                if let Some(command) = self.claude.foreground.as_mut()
                    && (result_uuid.is_some() || result_uuids.is_some_and(|ids| !ids.is_empty()))
                {
                    if result_uuid != Some(command.id.as_str())
                        && !result_uuids.is_some_and(|ids| {
                            ids.iter()
                                .any(|id| id.as_str() == Some(command.id.as_str()))
                        })
                    {
                        return Ok(());
                    }
                    command.result_confirmed = true;
                }
                // Partial records are replaced by the authoritative assistant message before terminal delivery.
                let prefix = format!(
                    "{}:stream:",
                    self.claude_received_turn_id().unwrap_or_default()
                );
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot
                        .messages
                        .retain(|message| !message.id.starts_with(&prefix));
                }
                let interrupted = matches!(
                    record["terminal_reason"].as_str(),
                    Some("aborted_streaming" | "aborted_tools")
                );
                let refusal = record["stop_reason"] == "refusal" || self.claude.safety_stopped;
                let failed = self.claude.api_failed
                    || refusal
                    || (!interrupted
                        && (record["is_error"] == true || record["subtype"] != "success"));
                if refusal {
                    self.claude_error(claude_refusal_blocks(&record));
                }
                if self.compacting && self.compaction_message_id.is_none() {
                    let detail = record["result"]
                        .as_str()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or("运行时未确认执行压缩");
                    self.claude_compaction_status(format!("上下文未压缩：{detail}"), true, None);
                }
                if self.compacting && !failed {
                    // SDK boundary notifications can omit postTokens. The
                    // persisted boundary owns that count; don't estimate it
                    // from the summarizer's (pre-compaction) request usage.
                    self.claude_refresh_context();
                }
                if failed {
                    self.claude_error(claude_error_blocks(&record)?);
                }
                // Protect an actual result as soon as the owner latches its
                // barrier, before fail_connection runs at the end of the batch.
                // API/refusal notices and new successful assistant responses
                // are recovery evidence, not duplicate terminal envelopes.
                if !self.claude_preserves_terminal_result() {
                    self.claude.commands.result_is_terminal = true;
                    self.claude.result = Some((
                        if failed {
                            TurnOutcome::Failed
                        } else if interrupted {
                            TurnOutcome::Cancelled
                        } else {
                            TurnOutcome::Completed
                        },
                        std::time::Instant::now(),
                    ));
                }
                // The turn's usage over its requests, the context its last
                // reply left with that model's window, and the session's cost
                // while Claude Code knows every model's price.
                if record["usage"].is_object() {
                    let mut usage = claude_usage(&record["usage"]);
                    let models = record["modelUsage"].as_object();
                    if let Some((model, input, output)) = self.claude.context.take() {
                        usage.context_tokens = Some(input + output);
                        usage.context_window = models
                            .and_then(|models| models.get(&model))
                            .and_then(|entry| entry["contextWindow"].as_u64());
                    }
                    if models.is_some_and(|models| {
                        models.values().all(|entry| entry["costBasis"] != "unknown")
                    }) {
                        usage.session_cost_micro_usd =
                            record["total_cost_usd"].as_f64().and_then(micro_usd);
                    }
                    self.turn_usage(|total| match total {
                        Some(total) => total.add(&usage),
                        None => *total = Some(usage),
                    });
                }
                self.claude_finish_background_foreground()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn claude_finish_background_foreground(&mut self) -> Result<()> {
        // A queued completion notification may start another native turn before
        // idle. Only a correlated foreground result and command completion can
        // settle the user's turn early; older CLI records still use idle.
        if self
            .claude
            .foreground
            .as_ref()
            .is_some_and(|command| command.result_confirmed && command.command_completed)
            && self.claude_has_ongoing_background()
        {
            self.claude_finish_received_result()?;
        }
        Ok(())
    }

    fn claude_finish_received_result(&mut self) -> Result<()> {
        if (self.turn_running() || self.claude_has_failed_turn())
            && let Some((outcome, _)) = self.claude.result.take()
        {
            self.claude.foreground = None;
            self.claude_finish_received_outcome(outcome);
            self.claude_finish_received_turn(if outcome == TurnOutcome::Failed {
                RunState::Failed
            } else {
                RunState::Idle
            })?;
        }
        Ok(())
    }
    fn claude_refresh_context(&mut self) {
        if self.claude.commands.enabled {
            self.claude.commands.context_refresh_required = true;
            return;
        }
        if let Err(error) = self.claude_read_context() {
            tracing::warn!(event = "claude_compaction_context_refresh_failed", %error);
        }
    }
    pub(super) fn claude_read_context(&mut self) -> Result<()> {
        let id = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("No Claude session for history refresh"))?;
        let runtime_id = self
            .snapshot
            .as_ref()
            .map(|s| s.context_runtime_id.as_str())
            .unwrap_or_default();
        let history = crate::history::read(&self.claude_history_config(), runtime_id, id)
            .map_err(|error| Error::new(error.to_string()))?;
        if let Some(message) = history
            .messages
            .iter()
            .find(|message| Some(&message.id) == self.compaction_message_id.as_ref())
        {
            self.message(message.clone());
        }
        if let Some(usage) = history
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.turn_usage.as_ref())
        {
            self.context_usage(usage.context_tokens, usage.context_window);
        }
        Ok(())
    }

    fn claude_history_config(&self) -> crate::history::HistoryConfig {
        crate::history::HistoryConfig {
            kind: crate::version::RuntimeKind::ClaudeCode,
            execution: self.config.execution.clone(),
            os_home: self.config.os_home.clone(),
            provider: "claude-code".into(),
            binary: self.config.binary.clone(),
            node_binary: self.config.node_binary.clone().unwrap_or_default(),
            resources_directory: self.config.resources_directory.clone(),
            root: self.config.agent_dir.join("projects"),
            home: self.config.agent_dir.clone(),
        }
    }

    fn claude_error(&mut self, blocks: Vec<MessageBlock>) {
        if self.compacting {
            let text = blocks
                .into_iter()
                .filter_map(|block| match block {
                    MessageBlock::Notice { text } | MessageBlock::Text { text } => Some(text),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.claude_compaction_status(format!("上下文压缩失败：{text}"), true, None);
            return;
        }
        let Some(turn) = &self.turn_id else { return };
        let id = format!("{turn}:error");
        let (position, previous) = self.take_message(&id);
        let mut message = previous.unwrap_or(Message {
            id,
            role: MessageRole::System,
            timestamp_unix_ms: None,
            blocks: Vec::new(),
            completed: true,
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
        });
        let mut details = std::mem::take(&mut message.blocks)
            .into_iter()
            .filter_map(|block| match block {
                MessageBlock::RuntimeFailure { detail, .. } => Some(detail),
                _ => None,
            })
            .collect::<Vec<_>>();
        for block in blocks {
            if let MessageBlock::Notice { text } | MessageBlock::Text { text } = block
                && !details
                    .iter()
                    .any(|detail| format!("\n{detail}\n").contains(&format!("\n{text}\n")))
            {
                details.push(text);
            }
        }
        if details.is_empty() {
            details.push("Claude Code 执行失败（上游未提供错误详情）。".into());
        }
        message.blocks = vec![MessageBlock::RuntimeFailure {
            failure: RuntimeFailureKind::Other,
            detail: details.join("\n"),
            retry: None,
        }];
        self.message_at(message, position);
    }
    /// Complete the call a tool result answers, as one row like the native
    /// transcript shows it; false when the call is not in this projection.
    fn claude_complete_call(&mut self, result: &MessageBlock) -> bool {
        let MessageBlock::Tool {
            tool_id: Some(id),
            state,
            output,
            conversation_id,
            ..
        } = result
        else {
            return false;
        };
        let Some(snapshot) = &mut self.snapshot else {
            return false;
        };
        if let Some(message) = snapshot.messages.iter().rev().find(|message| message.blocks.iter().any(|block| matches!(block, MessageBlock::Tool { tool_id: Some(call), .. } if call == id))) {
            self.data.retain(DataOwner::Message(message.id.clone()));
        }
        let call = snapshot
            .messages
            .iter_mut()
            .rev()
            .flat_map(|message| message.blocks.iter_mut())
            .find_map(|block| match block {
                MessageBlock::Tool {
                    tool_id: Some(call),
                    state,
                    output,
                    conversation_id,
                    ..
                } if call == id => Some((state, output, conversation_id)),
                _ => None,
            });
        let Some((call_state, call_output, call_conversation)) = call else {
            return false;
        };
        *call_state = state.clone();
        *call_output = output.clone();
        *call_conversation = conversation_id.clone();
        snapshot.revision += 1;
        self.sub_agents.extend(conversation_id.clone());
        true
    }
    /// A sub-agent's session, as the history reader names it.
    pub(super) fn claude_agent_conversation(&self, agent: &str) -> Option<String> {
        let runtime_id = &self.snapshot.as_ref()?.conversation.runtime_id;
        Some(format!(
            "{runtime_id}:{}/agent-{agent}",
            self.native_id.as_deref()?
        ))
    }
    pub(super) fn claude_check_terminal_timeout(&mut self) {
        // Claude SDK 2.1.286 can keep observers alive after result; its between-turn ceiling is ten minutes.
        if self
            .claude
            .result
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(600))
            && self.claude.tasks.is_empty()
            && !self.claude.requires_action
            && self.interactions.is_empty()
        {
            self.claude.result = None;
            self.claude.commands.result_is_terminal = false;
            self.claude.safety_stopped = false;
            self.settle(RunState::Failed);
            self.notice(if self.claude.commands.enabled {
                "Claude Code 返回结果后未确认空闲；需要显式清理本轮进程，请检查原生运行时状态。"
            } else {
                "Claude Code 返回结果后未确认空闲；已结束本轮进程，请检查原生运行时状态。"
            });
        }
    }
    pub(super) fn claude_reply(
        &mut self,
        pending: &PendingRequest,
        reply: RuntimeInteractionReply,
    ) -> Result<()> {
        let response = match reply {
            RuntimeInteractionReply::Answers { answers } if pending.method == "AskUserQuestion" => {
                let mut selected = serde_json::Map::new();
                for answer in answers {
                    if !pending.question_ids.contains(&answer.question_id)
                        || selected.contains_key(&answer.question_id)
                    {
                        return Err(Error::new("Claude 答案与当前问题不匹配"));
                    }
                    selected.insert(answer.question_id, json!(answer.values.join(", ")));
                }
                if selected.len() != pending.question_ids.len() {
                    return Err(Error::new("请回答所有问题"));
                }
                let mut input = pending.payload["input"].clone();
                input["answers"] = json!(selected);
                json!({"behavior":"allow","updatedInput":input})
            }
            RuntimeInteractionReply::Decision { option_id } if option_id == "allow" => {
                json!({"behavior":"allow","updatedInput":pending.payload["input"]})
            }
            RuntimeInteractionReply::Decision { option_id } if option_id == "deny" => {
                json!({"behavior":"deny","message":"User denied this operation"})
            }
            RuntimeInteractionReply::Cancel => {
                json!({"behavior":"deny","message":"User cancelled this operation","interrupt":true})
            }
            _ => return Err(Error::new("Claude 工具权限回复无效")),
        };
        self.claude_write_with_class(&json!({"type":"control_response","response":{"subtype":"success","request_id":pending.id,"response":response}}), crate::transport::WriteClass::ProtocolReplyCancel)
    }
}
/// `agent` is the conversation of the sub-agent a tool result started.
/// Blank text and thinking show nothing: Claude Code records omitted thinking
/// as an empty block beside its signature.
fn claude_plain_text(content: &Value) -> String {
    claude_blocks(content, None)
        .into_iter()
        .filter_map(|block| match block {
            MessageBlock::Text { text } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn claude_refused_message(record: &Value) -> bool {
    record["refused_message_id"].is_string() || record["refusedMessageId"].is_string()
}

fn claude_refusal_blocks(record: &Value) -> Vec<MessageBlock> {
    let mut blocks = vec![MessageBlock::Notice {
        text: "Claude Code 响应被安全系统停止（refusal）。".into(),
    }];
    let text = claude_plain_text(&record["message"]["content"]);
    if !text.is_empty() {
        blocks.push(MessageBlock::Notice { text });
    }
    blocks
}

fn claude_error_blocks(record: &Value) -> Result<Vec<MessageBlock>> {
    let mut details = Vec::new();
    if record["type"] == "assistant" {
        for block in claude_blocks(&record["message"]["content"], None) {
            if let MessageBlock::Text { text } = block {
                details.push(text);
            }
        }
        if let Some(code) = record["error"].as_str() {
            details.push(format!("Claude Code 错误类型：{code}"));
        }
    } else {
        if !record["errors"].is_null() {
            let errors: Vec<String> = serde_json::from_value(record["errors"].clone())
                .map_err(|error| Error::new(format!("Claude Code 错误列表无效：{error}")))?;
            details.extend(errors);
        }
        if let Some(result) = record["result"].as_str() {
            details.push(result.into());
        }
        if let Some(code) = record["subtype"].as_str().filter(|code| *code != "success") {
            details.push(format!("Claude Code 错误类型：{code}"));
        }
        if let Some(status) = record["api_error_status"].as_u64() {
            details.push(format!("HTTP {status}"));
        }
    }
    Ok(details
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .map(|text| MessageBlock::Notice { text })
        .collect())
}

fn claude_blocks(content: &Value, agent: Option<String>) -> Vec<MessageBlock> {
    fn visible(text: &Value) -> Option<&str> {
        text.as_str().filter(|text| !text.trim().is_empty())
    }
    if content.is_string() {
        return visible(content)
            .map(|text| MessageBlock::Text { text: text.into() })
            .into_iter()
            .collect();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => {
                visible(&block["text"]).map(|text| MessageBlock::Text { text: text.into() })
            }
            Some("thinking") => visible(&block["thinking"])
                .map(|text| MessageBlock::Reasoning { text: text.into() }),
            Some("tool_use") => Some(MessageBlock::Tool {
                tool_id: block["id"].as_str().map(str::to_owned),
                title: block["name"].as_str().unwrap_or("工具操作").into(),
                state: ToolState::Running,
                output: None,
                conversation_id: None,
            }),
            Some("tool_result") => Some(MessageBlock::Tool {
                tool_id: block["tool_use_id"].as_str().map(str::to_owned),
                title: "工具结果".into(),
                state: if block["is_error"] == true {
                    ToolState::Failed
                } else {
                    ToolState::Completed
                },
                output: Some(if let Some(text) = block["content"].as_str() {
                    text.into()
                } else {
                    block["content"].to_string()
                }),
                conversation_id: agent.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Anthropic counts input without the cache reads and writes; Velune's input
/// includes them. Claude Code copies thinking tokens from the response and
/// writes 0 when there were none to copy, so only more than 0 counts.
fn claude_usage(usage: &Value) -> TurnUsage {
    let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    TurnUsage {
        input_tokens: usage["input_tokens"]
            .as_u64()
            .map(|input| input + read + write),
        cache_read_tokens: Some(read),
        cache_write_tokens: Some(write),
        output_tokens: usage["output_tokens"].as_u64(),
        reasoning_tokens: usage["output_tokens_details"]["thinking_tokens"]
            .as_u64()
            .filter(|tokens| *tokens > 0),
        ..TurnUsage::default()
    }
}

/// What one reply's request held: its input, cache included, and its output.
/// Recognize only our single empty import metadata record. Other empty or
/// damaged histories must never silently become a new model conversation.
fn claude_empty_import(bytes: &[u8], id: &str, cwd: &Path) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(line) = lines.next() else {
        return false;
    };
    if lines.next().is_some() {
        return false;
    }
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    record["type"] == "queue-operation"
        && record["operation"] == "dequeue"
        && record["veluneEmptyImport"] == true
        && record["sessionId"] == id
        && record["cwd"].as_str() == cwd.to_str()
}

fn claude_context(usage: &Value) -> Option<(u64, u64)> {
    let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    Some((
        usage["input_tokens"].as_u64()? + read + write,
        usage["output_tokens"].as_u64()?,
    ))
}

/// The user's own default Claude Code configuration directory on this machine.
pub fn claude_user_config_directory() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude"))
}

/// The `CLAUDE_CONFIG_DIR` a local Claude Code login kept at `agent_dir` runs
/// with: `None` for the user's own default directory, whose login the
/// variable would hide even when set to that directory.
pub fn claude_login_directory(agent_dir: &Path) -> Option<PathBuf> {
    (!claude_default_directory(agent_dir, &crate::execution::ExecutionTarget::Local, None))
        .then(|| agent_dir.to_path_buf())
}

/// `NativeSession::claude_default_config` for a directory, HOME and machine.
fn claude_default_directory(
    agent_dir: &Path,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> bool {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    let same = |left: &Path, right: &Path| {
        left == right
            || matches!(
                (std::fs::canonicalize(left), std::fs::canonicalize(right)),
                (Ok(left), Ok(right)) if left == right
            )
    };
    execution.is_local()
        && os_home.is_none_or(|os_home| same(os_home, &home))
        && same(agent_dir, &home.join(".claude"))
}

/// Name a Claude Code session as the user would with `/rename`: a print-mode
/// child resumes it in `cwd`, the folder Claude Code keeps it under, and
/// takes the SDK's `rename_session` from the host. No turn is sent, so no
/// model is asked; a process that has the session open reads the name from
/// the transcript.
pub fn rename_claude_session_at(
    binary: &Path,
    home: &Path,
    id: &str,
    cwd: &Path,
    title: &str,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<()> {
    if !binary.is_absolute() || !home.is_absolute() || !cwd.is_absolute() {
        return Err(Error::new("Claude Code 会话配置必须使用绝对路径"));
    }
    crate::version::check_version_at(
        crate::version::RuntimeKind::ClaudeCode,
        binary,
        None,
        execution,
        os_home,
    )?;
    let mut command = crate::process_tree::command(binary);
    command
        .args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--resume",
            id,
        ])
        .current_dir(cwd)
        .env("CLAUDE_CODE_ENTRYPOINT", "velune")
        .env_remove("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST")
        .env_remove("VELUNE_GATEWAY_TOKEN");
    // As for a turn: set even to the default, the variable hides the user's login.
    if claude_default_directory(home, execution, os_home) {
        command.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        command.env("CLAUDE_CONFIG_DIR", home);
    }
    let mut rpc = rpc::RpcClient::spawn_at(command, execution, os_home)?;
    let result = rpc
        .claude_control(json!({"subtype":"initialize","hooks":{}}))
        .and_then(|_| {
            rpc.claude_control(
                json!({"subtype":"rename_session","title":title,"source":"host","session_id":id}),
            )
        });
    rpc.shutdown();
    result.map(|_| ())
}

/// A new random (version 4) UUID for a Claude session or input record.
fn claude_uuid() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| Error::new(format!("Claude 身份生成失败：{error}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

#[cfg(test)]
mod error_tests;

#[cfg(all(test, unix))]
mod question_reconciliation_tests;
