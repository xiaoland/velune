//! Editing a user message: the edited text goes out as a new turn that
//! replaces the message, either in its conversation, which goes back to just
//! before it, or in a new conversation holding what precedes it.
//!
//! Each runtime branches through its own native means; Velune keeps no copy
//! of the history. See `NativeSession::branch` and Pi's `velune-rewind`.
use super::*;
use crate::conversation::{
    ConversationSnapshot, EditPlacement, EditableMessage, Message, MessageBlock, MessageRole,
};

/// The runtime types whose native history can branch at a user message.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Branching {
    Codex,
    Claude,
    Pi,
}

struct EditingChoice {
    public: EditableMessage,
    native: bool,
}

impl Branching {
    fn of(kind: RuntimeKind) -> Option<Self> {
        match kind {
            RuntimeKind::Codex166 => Some(Self::Codex),
            RuntimeKind::ClaudeCode => Some(Self::Claude),
            RuntimeKind::Pi => Some(Self::Pi),
            RuntimeKind::DeepSeek | RuntimeKind::Acp => None,
        }
    }
}

/// The text a user message carries, for recognizing it in a fresh read.
fn message_text(message: &Message) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            MessageBlock::Text { text } => Some(text.trim()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The entry a Pi message identity names: `pi:<session>:<entry>:<ordinal>`.
fn pi_entry(message_id: &str) -> Option<&str> {
    let mut parts = message_id.strip_prefix("pi:")?.rsplitn(3, ':');
    let _ordinal = parts.next()?;
    let entry = parts.next()?;
    parts.next()?;
    Some(entry)
}

impl CoreRuntime {
    /// The user messages of the addressed session that may be edited now.
    /// Editing continues as a fork or an in-place replacement; an in-place
    /// replacement stops a running turn first, so idleness is not a
    /// condition.
    pub(super) fn editable_messages(
        &self,
        snapshot: &ConversationSnapshot,
    ) -> Vec<EditableMessage> {
        self.editing_choices(snapshot)
            .into_iter()
            .map(|choice| choice.public)
            .collect()
    }

    /// A single native source without a cutoff or handoff branches directly.
    /// The conversation's own link decides — never the addressed session's
    /// projection, which may be a different conversation in read paths.
    fn simple_source(&self, conversation_id: &str) -> bool {
        self.link_of(conversation_id).is_none_or(|link| {
            matches!(link.segments.as_slice(), [segment] if segment.cutoff.is_none() && segment.handoff.is_none())
        })
    }

    /// The conversation's persisted association link, if it is a linked
    /// (cross-runtime) conversation.
    fn link_of(&self, conversation_id: &str) -> Option<&crate::config::ConversationLink> {
        self.conversation_links
            .iter()
            .find(|link| link.id == conversation_id)
    }

    fn editing_choices(&self, snapshot: &ConversationSnapshot) -> Vec<EditingChoice> {
        if snapshot.conversation.parent_conversation_id.is_some() {
            // A native sub-agent's conversation is continued by its parent's agent.
            return Vec::new();
        }
        let simple_source = self.simple_source(&snapshot.conversation.id);
        let branching = self
            .runtime_instance(&snapshot.conversation.runtime_id)
            .ok()
            .and_then(|runtime| Branching::of(runtime.type_id));
        // A linked (cross-runtime) conversation has no single native source:
        // editing continues in a translated copy instead.
        let translated = !simple_source && self.translatable(snapshot);
        if branching.is_none() && !translated {
            return Vec::new();
        }
        let mut turns = std::collections::BTreeSet::new();
        snapshot
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .enumerate()
            .map(|(ordinal, message)| {
                let native = simple_source
                    && match branching {
                        // A Codex turn goes back as a whole: a message steered
                        // into it cannot be replaced on its own.
                        Some(Branching::Codex) => message
                            .branch_point
                            .as_ref()
                            .is_some_and(|point| turns.insert(point.clone())),
                        // Claude Code resumes after a record; the first message
                        // has none before it.
                        Some(Branching::Claude | Branching::Pi) => true,
                        None => false,
                    };
                EditingChoice {
                    native,
                    public: EditableMessage {
                        message_id: message.id.clone(),
                        in_place: native && (branching != Some(Branching::Claude) || ordinal > 0),
                    },
                }
            })
            .filter(|choice| choice.native || translated)
            .collect()
    }

    /// Whether a linked conversation's prefix can be translated for editing
    /// or forking: every segment exports its effective context, and the
    /// conversation's current context runtime writes translated history.
    fn translatable(&self, snapshot: &ConversationSnapshot) -> bool {
        let Some(link) = self.link_of(&snapshot.conversation.id) else {
            return false;
        };
        let Ok(context_runtime) = self.runtime_instance(&snapshot.context_runtime_id) else {
            return false;
        };
        if !matches!(
            context_runtime.type_id,
            RuntimeKind::ClaudeCode | RuntimeKind::Codex166 | RuntimeKind::Pi
        ) {
            return false;
        }
        !link.segments.is_empty()
            && link.segments.iter().all(|segment| {
                self.runtime_instance(&segment.runtime_instance_id)
                    .map(|runtime| {
                        matches!(
                            runtime.type_id,
                            RuntimeKind::Pi | RuntimeKind::Codex166 | RuntimeKind::ClaudeCode
                        )
                    })
                    .unwrap_or(false)
            })
    }

    /// The user message IDs whose turns a fork may start from now. Each
    /// completed turn on the conversation's own branching runtime qualifies.
    /// The UI may arrange the copy while a later turn runs; execution still
    /// waits for idleness so releasing a native writer cannot stop that turn.
    pub(super) fn forkable_turn_users(
        &self,
        snapshot: &ConversationSnapshot,
    ) -> std::collections::BTreeSet<String> {
        if snapshot.conversation.parent_conversation_id.is_some() {
            return Default::default();
        }
        if !self.simple_source(&snapshot.conversation.id) {
            // Linked conversation: a fork is a translated copy, so any
            // completed turn qualifies when translation is available.
            return if self.translatable(snapshot) {
                snapshot
                    .transcript_turns
                    .iter()
                    .filter(|turn| !turn.is_running)
                    .map(|turn| turn.user_message_id.clone())
                    .collect()
            } else {
                Default::default()
            };
        }
        let Some(branching) = self
            .runtime_instance(&snapshot.conversation.runtime_id)
            .ok()
            .and_then(|runtime| Branching::of(runtime.type_id))
        else {
            return Default::default();
        };
        snapshot
            .transcript_turns
            .iter()
            .filter(|turn| !turn.is_running)
            .filter(|turn| {
                branching != Branching::Codex || {
                    // A Codex fork resolves the boundary at the turn's own
                    // start or the nearest one before it; a steered turn
                    // shares its native turn's point.
                    let index = snapshot
                        .messages
                        .iter()
                        .position(|message| message.id == turn.user_message_id);
                    index.is_some_and(|index| {
                        snapshot.messages[..=index].iter().any(|message| {
                            message.role == MessageRole::User && message.branch_point.is_some()
                        })
                    })
                }
            })
            .map(|turn| turn.user_message_id.clone())
            .collect()
    }

    /// Stamp each transcript turn with whether a fork may start from its end.
    pub(super) fn stamp_forkable_turns(&self, snapshot: &mut ConversationSnapshot) {
        let forkable = self.forkable_turn_users(snapshot);
        for turn in &mut snapshot.transcript_turns {
            turn.can_fork = forkable.contains(&turn.user_message_id);
        }
    }

    /// Whether a turn's closing reply may fork the conversation now.
    pub(super) fn forkable(&self, snapshot: &ConversationSnapshot) -> bool {
        snapshot.transcript_turns.iter().any(|turn| turn.can_fork)
    }

    /// Fork the conversation through the turn the user message `message_id`
    /// starts: the result is the new conversation's snapshot, holding
    /// everything up to the next user message. This conversation stays as it
    /// is; the copy waits for its next turn like a new conversation.
    pub(crate) fn fork_message(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let current = self
            .current_snapshot()
            .filter(|snapshot| {
                snapshot.conversation.id == conversation_id
                    && snapshot.conversation.runtime_id == runtime_id
            })
            .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
        if !self.forkable_turn_users(&current).contains(message_id) {
            return Err(RuntimeError::invalid("会话当前不能分叉"));
        }
        if !self.simple_source(&current.conversation.id) {
            return self.translated_branch(&current, message_id, None, None);
        }
        if self.turn_running() {
            // Forking reopens the source natively; a running turn must not
            // be disturbed. Hold for the turn to end and retry.
            return Err(crate::Error::Busy);
        }
        let branching =
            Branching::of(self.runtime_instance(runtime_id)?.type_id).expect("forkable runtime");
        // The message as the native history has it now: what the view shows
        // may predate a change made outside Velune.
        let native_id = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| projection.link.segments[0].native_conversation_id.as_str())
            .unwrap_or(conversation_id)
            .to_owned();
        let fresh = self.read_history(runtime_id, &native_id)?;
        let users: Vec<&Message> = fresh
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .collect();
        // Live user messages can still carry provisional IDs (Claude Code
        // persists its own UUID). Match the addressed turn to fresh history
        // by its user ordinal and content, just as message editing does.
        let (ordinal, expected) = current
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .enumerate()
            .find(|(_, message)| message.id == message_id)
            .ok_or_else(|| RuntimeError::invalid("会话中没有这条用户消息"))?;
        let message = users
            .get(ordinal)
            .filter(|message| message_text(message) == message_text(expected))
            .ok_or_else(|| RuntimeError::invalid("会话已在别处改变，请重新打开后再分叉"))?;
        let message_id = message.id.clone();
        // Pi forks before an entry: the next user message's, or the whole
        // branch when the turn is the last.
        let pi_boundary = match branching {
            Branching::Pi => Some(
                users
                    .get(ordinal + 1)
                    .map(|message| {
                        pi_entry(&message.id)
                            .map(str::to_owned)
                            .ok_or_else(|| RuntimeError::invalid("这条消息没有 Pi 条目身份"))
                    })
                    .transpose()?,
            ),
            _ => None,
        };
        let selection = self.session.model_selection.clone().unwrap_or_else(|| {
            fresh
                .model_record_key
                .clone()
                .map(|model_record_key| crate::ModelSelection::Managed { model_record_key })
        });
        let origin = self.session.key;
        // A parked Codex session keeps its writer. Release it before a
        // separate app-server opens the source to create the fork.
        if branching == Branching::Codex {
            self.invalidate_execution()?;
        }
        self.focus_new();
        let branched = (|| {
            self.prepare_with_selection(fresh.clone(), selection.as_ref(), false)?;
            match &mut self.session.active_state {
                ActiveState::Native(session) => session
                    .fork_through(&message_id)
                    .map_err(|error| RuntimeError::context("会话分叉", error)),
                ActiveState::Pi(session) => {
                    session
                        .fork_at(pi_boundary.flatten().as_deref())
                        .map_err(|error| RuntimeError::context("会话分叉", error))?;
                    self.sync_projection()
                }
                _ => Err(RuntimeError::invalid("此运行时不提供会话分叉")),
            }
        })();
        if let Err(error) = branched {
            // The new session holds nothing worth keeping; the original
            // conversation is addressed again as it was.
            let _ = self.shutdown_active();
            self.session.unsubmitted_draft = None;
            self.focus_key(origin);
            return Err(error);
        }
        if let ActiveState::Native(session) = &self.session.active_state {
            self.session.pending_native_fork = session.pending_fork();
        }
        self.snapshot_result()
    }

    /// Send `turn` in place of the user message `message_id`, continuing as
    /// `placement` says. A fork's result is the new conversation's snapshot.
    pub(crate) fn edit_message(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
        message_id: &str,
        placement: EditPlacement,
        selection: Option<crate::ModelSelection>,
        turn: crate::ComposedTurn,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let current = self
            .current_snapshot()
            .filter(|snapshot| {
                snapshot.conversation.id == conversation_id
                    && snapshot.conversation.runtime_id == runtime_id
            })
            .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
        let choice = self
            .editing_choices(&current)
            .into_iter()
            .find(|choice| choice.public.message_id == message_id)
            .ok_or_else(|| RuntimeError::invalid("这条消息当前不能编辑"))?;
        if placement == EditPlacement::InPlace && !choice.public.in_place {
            return Err(RuntimeError::invalid(
                "此运行时不能在原会话中替换这条消息，请在新会话中继续",
            ));
        }
        let target_runtime_id = if choice.native {
            runtime_id
        } else {
            &current.context_runtime_id
        };
        let runtime = self.runtime_instance(target_runtime_id)?.clone();
        // Nothing changes before the replacement is known to be deliverable.
        let attachments = self.turn_attachments(&runtime, &turn.attachments)?;
        let text = super::attachments::compose_text(&turn.text, &attachments.files);
        if text.trim().is_empty() && attachments.images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        self.validate_model_selection(&runtime, selection.as_ref())?;
        if let Some(effort) = &turn.effort {
            self.validate_effort(&runtime, selection.as_ref(), effort)?;
        }
        if let Some(mode) = &turn.permission_mode {
            self.validate_permission_mode(&runtime, mode)?;
        }
        if placement == EditPlacement::Fork && self.turn_running() {
            // Releasing a native source writer must not interrupt its turn.
            // Mac retains the arranged fork and retries after it ends.
            return Err(RuntimeError::Busy);
        }
        // Validate the replacement before stopping a running turn: rejected
        // attachments or settings must leave the current work untouched.
        if placement == EditPlacement::InPlace && self.turn_running() {
            self.cancel(runtime_id)?;
        }
        if !choice.native {
            // No native boundary at this message: continue in a translated
            // copy of everything before it, in the conversation's context
            // runtime.
            return self.translated_branch(&current, message_id, selection, Some(turn));
        }
        let branching = Branching::of(runtime.type_id).expect("native editing runtime");
        // The message as the native history has it now: what the view shows
        // may predate a change made outside Velune.
        let users = |snapshot: &ConversationSnapshot| {
            snapshot
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::User)
                .cloned()
                .collect::<Vec<_>>()
        };
        let ordinal = users(&current)
            .iter()
            .position(|message| message.id == message_id)
            .expect("editable user message");
        let expected = users(&current)
            .get(ordinal)
            .map(message_text)
            .unwrap_or_default();
        let native_id = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| projection.link.segments[0].native_conversation_id.as_str())
            .unwrap_or(conversation_id)
            .to_owned();
        let fresh = self.read_history(runtime_id, &native_id)?;
        let original = users(&fresh)
            .into_iter()
            .nth(ordinal)
            .filter(|message| message_text(message) == expected)
            .ok_or_else(|| RuntimeError::invalid("会话已在别处改变，请重新打开后再编辑"))?;
        let first = ordinal == 0;

        let origin = self.session.key;
        if placement == EditPlacement::Fork {
            if branching == Branching::Codex {
                self.invalidate_execution()?;
            }
            self.focus_new();
        }
        // A single-target alias is a public identity, not a separate history.
        // Branch its native source, then keep the original public association
        // for an in-place edit. A fork has its own new session identity.
        let alias = self.session.logical_projection.take();
        let branched = (|| {
            if placement == EditPlacement::Fork && first && branching != Branching::Pi {
                // Nothing precedes the first message: a new conversation in
                // the same directory.
                self.create_conversation(
                    runtime_id,
                    fresh.conversation.cwd.clone(),
                    selection.clone(),
                )?;
                return Ok(());
            }
            if placement == EditPlacement::InPlace {
                self.invalidate_execution()?;
            }
            self.prepare_with_selection(fresh.clone(), selection.as_ref(), false)?;
            match &mut self.session.active_state {
                ActiveState::Native(session) => {
                    let id = session
                        .snapshot()
                        .map(users)
                        .unwrap_or_default()
                        .into_iter()
                        .nth(ordinal)
                        .map(|message| message.id)
                        .ok_or_else(|| RuntimeError::invalid("运行时会话中没有这条消息"))?;
                    session
                        .branch(&id, placement)
                        .map_err(|error| RuntimeError::context("消息编辑", error))
                }
                ActiveState::Pi(session) => {
                    let entry = pi_entry(&original.id)
                        .ok_or_else(|| RuntimeError::invalid("这条消息没有 Pi 条目身份"))?;
                    session
                        .branch(entry, placement)
                        .map_err(|error| RuntimeError::context("消息编辑", error))?;
                    self.sync_projection()
                }
                _ => Err(RuntimeError::invalid("此运行时不提供消息编辑")),
            }
        })();
        if alias.is_some() {
            self.session.logical_projection = alias;
        }
        if let Err(error) = branched {
            if placement == EditPlacement::Fork {
                // The new session holds nothing worth keeping; the original
                // conversation is addressed again as it was.
                let _ = self.shutdown_active();
                self.session.unsubmitted_draft = None;
                self.focus_key(origin);
            } else {
                let _ = self.invalidate_execution();
            }
            return Err(error);
        }
        if let ActiveState::Native(session) = &self.session.active_state {
            self.session.pending_native_fork = session.pending_fork();
        }
        let conversation = self
            .session
            .conversation_id()
            .ok_or_else(|| RuntimeError::invalid("会话未打开"))?
            .to_owned();
        self.send_turn(super::conversations::TurnRequest {
            source_runtime_id: runtime_id.into(),
            conversation_id: conversation,
            runtime_id: runtime_id.into(),
            selection,
            turn,
        })
    }

    /// Continue through the user message `message_id` in a conversation whose
    /// history spans native segments (or whose runtime cannot branch there):
    /// translate every segment's effective model context into a new native
    /// conversation of the conversation's current context runtime, cut at the
    /// boundary, and send `turn` there — or, for a fork with no turn, leave
    /// the copy waiting for its next message. The original conversation and
    /// its association graph are never rewritten.
    fn translated_branch(
        &mut self,
        current: &ConversationSnapshot,
        message_id: &str,
        selection: Option<crate::ModelSelection>,
        turn: Option<crate::ComposedTurn>,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let link = self
            .session
            .logical_projection
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("会话缺少关联信息"))?
            .link
            .clone();
        // Authoritative re-read of every sealed segment, as handoff does.
        let (tail, checked) = self.read_link(&link)?;
        let tail_index = link.segments.len() - 1;
        let tail_segment = link.segments.last().expect("linked tail");
        let target_runtime = self.runtime_instance(&current.context_runtime_id)?.clone();
        // Assemble the displayed messages to locate the boundary, the same
        // way a read of the linked conversation presents them.
        let mut displayed = checked.prefix;
        if tail_index > 0 {
            displayed.push(self.boundary_message(tail_segment, tail_index));
        }
        displayed.extend(super::continuation::display_segment(
            tail_segment,
            &tail.messages,
            tail_index,
        )?);
        let boundary = displayed
            .iter()
            .position(|message| message.id == message_id)
            .ok_or_else(|| RuntimeError::invalid("会话中没有这条消息"))?;
        let boundary_text = message_text(&displayed[boundary]);
        if boundary_text.is_empty() {
            return Err(RuntimeError::invalid("这条消息没有可翻译的内容"));
        }
        if displayed[..boundary]
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .any(|message| message_text(message) == boundary_text)
        {
            return Err(RuntimeError::invalid(
                "此前存在内容相同的消息，无法确定分叉边界",
            ));
        }
        // Which segment holds the boundary: display ids are namespaced.
        let boundary_segment = message_id
            .strip_prefix("segment-")
            .and_then(|rest| rest.split(':').next())
            .and_then(|index| index.parse::<usize>().ok())
            .unwrap_or(tail_index);
        if boundary_segment > tail_index {
            return Err(RuntimeError::invalid("分叉边界已不在当前会话中"));
        }
        // Export each segment up to the boundary and translate the whole
        // prefix into one native conversation of the context runtime.
        let mut translated = Vec::new();
        for (index, segment) in link.segments.iter().enumerate() {
            if index > boundary_segment {
                break;
            }
            let runtime = self.runtime_instance(&segment.runtime_instance_id)?;
            let native_id = segment
                .native_conversation_id
                .strip_prefix(&format!("{}:", runtime.id))
                .ok_or_else(|| RuntimeError::invalid("关联段的原生身份无效"))?;
            let mut export = match runtime.type_id {
                RuntimeKind::Pi => velune_agent_runtime::history::export_pi_context(
                    &self.pi_history_config(runtime)?,
                    std::path::Path::new(native_id),
                ),
                RuntimeKind::Codex166 | RuntimeKind::ClaudeCode => {
                    velune_agent_runtime::history::export_context(
                        &self.history_config(runtime)?,
                        native_id,
                    )
                }
                _ => return Err(RuntimeError::invalid("关联段的运行时不支持导出上下文")),
            }
            .map_err(|error| RuntimeError::context("导出原生上下文", error))?;
            if index == boundary_segment {
                truncate_context(&mut export, &boundary_text)?;
            }
            translated.extend(export.messages);
        }
        if translated.is_empty() {
            return Err(RuntimeError::invalid("分叉边界之前没有可翻译的内容"));
        }
        let context = velune_agent_runtime::import::Context {
            messages: translated,
        };
        let cwd = std::path::PathBuf::from(
            current
                .conversation
                .cwd
                .clone()
                .ok_or_else(|| RuntimeError::invalid("会话缺少工作目录"))?,
        );
        let managed = matches!(selection, Some(crate::ModelSelection::Managed { .. }));
        let imported = self.import_context_to(&target_runtime, &cwd, &context, managed)?;
        let native_id = match imported {
            velune_agent_runtime::import::ImportedConversation::ClaudeCode { session_id } => {
                session_id
            }
            velune_agent_runtime::import::ImportedConversation::Codex { thread_id } => thread_id,
            velune_agent_runtime::import::ImportedConversation::Pi { path } => {
                path.to_string_lossy().into_owned()
            }
        };
        let origin = self.session.key;
        self.focus_new();
        let result = (|| {
            let mut target = self.read_history(
                &target_runtime.id,
                &format!("{}:{native_id}", target_runtime.id),
            )?;
            target.context_runtime_id = target_runtime.id.clone();
            let selection = selection.or_else(|| {
                target
                    .model_record_key
                    .clone()
                    .map(|model_record_key| crate::ModelSelection::Managed { model_record_key })
            });
            self.prepare_with_selection(target, selection.as_ref(), false)?;
            if let Some(turn) = turn {
                let conversation = self
                    .session
                    .conversation_id()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?
                    .to_owned();
                self.send_turn(super::conversations::TurnRequest {
                    source_runtime_id: target_runtime.id.clone(),
                    conversation_id: conversation,
                    runtime_id: target_runtime.id.clone(),
                    selection,
                    turn,
                })
            } else {
                self.snapshot_result()
            }
        })();
        if let Err(error) = result {
            let _ = self.shutdown_active();
            self.session.unsubmitted_draft = None;
            self.focus_key(origin);
            return Err(error);
        }
        result
    }
}

fn context_user_text(message: &velune_agent_runtime::import::ContextMessage) -> Vec<&str> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            velune_agent_runtime::import::Block::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Cut an exported context at the message containing `boundary_text`. Prefer
/// an exact text block (a steered turn merges entries into one message, each
/// keeping its own block); fall back to a unique containment match; refuse
/// an ambiguous one rather than cut at the wrong place.
fn truncate_context(
    context: &mut velune_agent_runtime::import::Context,
    boundary_text: &str,
) -> Result<(), RuntimeError> {
    use velune_agent_runtime::import::Role;
    let exact = context.messages.iter().position(|message| {
        message.role == Role::User
            && context_user_text(message)
                .iter()
                .any(|text| text.trim() == boundary_text)
    });
    let cut = exact.or_else(|| {
        let matches: Vec<usize> = context
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                message.role == Role::User
                    && context_user_text(message)
                        .iter()
                        .any(|text| text.contains(boundary_text))
            })
            .map(|(index, _)| index)
            .collect();
        match matches.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    });
    let cut = cut
        .ok_or_else(|| RuntimeError::invalid("原生上下文中找不到分叉边界，它可能已被压缩或改写"))?;
    context.messages.truncate(cut + 1);
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn rejected_running_edits_preserve_the_source_execution() {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-edit-admission-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, r#"
import json, pathlib, sys
for line in sys.stdin:
    request = json.loads(line)
    with pathlib.Path(__file__).with_name('control.log').open('a') as log:
        log.write(request['type'] + '\n')
    print(json.dumps({'type':'response','id':request['id'],'command':request['type'],'success':True,'data':{}}), flush=True)
"#).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "pi".into(),
            name: "Synthetic Pi".into(),
            type_id: RuntimeKind::Pi,
            gateway_id: String::new(),
            settings: Default::default(),
        }];
        let config: PiConfig = serde_json::from_value(json!({
            "execution":{"kind":"local"}, "os_home":root,
            "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3", "rpc_entry":helper,
        }))
        .unwrap();
        let client = pi::Client::spawn(config.clone()).unwrap();
        let mut projection = PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: "pi:synthetic".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::ConversationTitle::Untitled,
            cwd: Some(root.to_string_lossy().into_owned()),
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        });
        let snapshot = projection.snapshot.as_mut().unwrap();
        snapshot.run_state = RunState::Running;
        snapshot.messages = serde_json::from_value(json!([
            {"id":"u1","role":"user","completed":true,"blocks":[{"kind":"text","text":"first"}]},
            {"id":"a1","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"answer"}]},
            {"id":"u2","role":"user","completed":true,"blocks":[{"kind":"text","text":"running"}]}
        ])).unwrap();
        let mut session = pi::PiSession::new(config);
        session.start(client, projection);
        core.session.active_state = ActiveState::Pi(Box::new(session));
        core.session.execution_runtime_id = Some("pi".into());
        let source_key = core.session.key;
        let missing = root.join("missing.txt").to_string_lossy().into_owned();
        let error = core
            .edit_message(
                "pi",
                "pi:synthetic",
                "u1",
                EditPlacement::InPlace,
                None,
                crate::ComposedTurn {
                    text: "replacement".into(),
                    attachments: vec![missing],
                    effort: None,
                    permission_mode: None,
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("无法读取附件"));
        let error = core
            .edit_message(
                "pi",
                "pi:synthetic",
                "u1",
                EditPlacement::Fork,
                None,
                crate::ComposedTurn {
                    text: "replacement".into(),
                    attachments: vec![],
                    effort: None,
                    permission_mode: None,
                },
            )
            .unwrap_err();
        assert!(matches!(error, RuntimeError::Busy));
        assert!(matches!(
            core.fork_message("pi", "pi:synthetic", "u1"),
            Err(RuntimeError::Busy)
        ));
        assert!(core.turn_running());
        assert!(core.session.key == source_key);
        assert!(
            !root.join("control.log").exists(),
            "rejected edits sent runtime controls"
        );
        core.shutdown_active().unwrap();
        application.close_stopping_work().unwrap();
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }
}
