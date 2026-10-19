//! One open conversation on a device: transcript, pending interaction and composer.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    spinner::Spinner,
    text::TextView,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use velune_bindings::{
    BindingComposedTurn, BindingConversationSnapshot, BindingError, BindingInteractionAnswer,
    BindingInteractionKind, BindingMessage, BindingMessageBlock, BindingMessagePayload,
    BindingMessageRole, BindingModelSelection, BindingRunState, BindingRuntimeInstance,
    BindingRuntimeInteraction, BindingRuntimeInteractionReply, BindingRuntimeModel,
    BindingSnapshotResult, BindingSnapshotUpdate, BindingToolState,
};

use crate::core::{Core, Device, describe, run};

/// Where a conversation view points: an existing conversation, or one to start on a runtime.
#[derive(Clone)]
pub enum Target {
    Existing {
        runtime_id: String,
        conversation_id: String,
        title: String,
    },
    New {
        runtime_id: String,
        cwd: Option<String>,
    },
}

impl Target {
    fn runtime_id(&self) -> &str {
        match self {
            Target::Existing { runtime_id, .. } | Target::New { runtime_id, .. } => runtime_id,
        }
    }

    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            Target::Existing {
                conversation_id, ..
            } => Some(conversation_id),
            Target::New { .. } => None,
        }
    }
}

/// A conversation not yet started: where it will run, chosen above the composer.
struct Draft {
    runtimes: Vec<BindingRuntimeInstance>,
    projects: Vec<String>,
    cwd: Entity<InputState>,
    /// A local folder can be picked; a remote device's path is typed.
    local: bool,
}

pub struct ConversationView {
    core: Core,
    draft: Option<Draft>,
    pub device: Device,
    pub target: Target,
    snapshot: Option<BindingConversationSnapshot>,
    messages: Vec<BindingMessage>,
    /// Deferred payloads read on request, by message ID.
    payloads: HashMap<String, BindingMessage>,
    expanded: HashSet<String>,
    models: Vec<BindingRuntimeModel>,
    /// None: the runtime's own default.
    selection: Option<BindingModelSelection>,
    busy: bool,
    loading: bool,
    failure: Option<String>,
    list: ListState,
    composer: Entity<TextareaState>,
    /// Text answers of the interaction shown, by question ID.
    answers: HashMap<String, Entity<InputState>>,
    chosen: HashMap<String, String>,
    answering: Option<String>,
    _polling: Task<()>,
    _composer: Subscription,
}

impl ConversationView {
    pub fn new(
        core: Core,
        device: Device,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 10)
                .placeholder("输入消息，Ctrl+Enter 发送")
        });
        let subscription = cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = event
            {
                this.send(window, cx);
            }
        });
        let list = ListState::new(0, ListAlignment::Top, px(800.));
        list.set_follow_mode(FollowMode::Tail);
        let mut view = Self {
            core,
            draft: None,
            device,
            loading: matches!(target, Target::Existing { .. }),
            target,
            snapshot: None,
            messages: Vec::new(),
            payloads: HashMap::new(),
            expanded: HashSet::new(),
            models: Vec::new(),
            selection: None,
            busy: false,
            failure: None,
            list,
            composer,
            answers: HashMap::new(),
            chosen: HashMap::new(),
            answering: None,
            _polling: Task::ready(()),
            _composer: subscription,
        };
        view.load(cx);
        view._polling = view.poll(cx);
        view
    }

    /// A new conversation on one of `runtimes`, started by its first message.
    pub fn new_draft(
        core: Core,
        device: Device,
        runtimes: Vec<BindingRuntimeInstance>,
        projects: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let target = Target::New {
            runtime_id: runtimes.first().map(|r| r.id.clone()).unwrap_or_default(),
            cwd: None,
        };
        let local = device == Device::Local;
        let first = projects.first().cloned().unwrap_or_default();
        let cwd = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("项目目录（留空使用运行时默认目录）")
                .default_value(first)
        });
        let mut view = Self::new(core, device, target, window, cx);
        view.draft = Some(Draft {
            runtimes,
            projects,
            cwd,
            local,
        });
        view
    }

    fn choose_runtime(&mut self, runtime_id: String, cx: &mut Context<Self>) {
        self.target = Target::New {
            runtime_id,
            cwd: None,
        };
        self.models.clear();
        self.selection = None;
        self.load(cx);
        cx.notify();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        let input = draft.cwd.clone();
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择项目目录".into()),
        });
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = cx.update(|window, cx| {
                input.update(cx, |input, cx| {
                    input.set_value(path.to_string_lossy().into_owned(), window, cx)
                })
            });
        })
        .detach();
    }

    pub fn title(&self) -> String {
        match (&self.snapshot, &self.target) {
            (Some(snapshot), _) => snapshot.conversation.title.clone(),
            (None, Target::Existing { title, .. }) => title.clone(),
            (None, Target::New { .. }) => "新会话".into(),
        }
    }

    fn running(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            matches!(
                snapshot.run_state,
                BindingRunState::Running | BindingRunState::Stopping
            )
        })
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let device = self.device.clone();
        let runtime = self.target.runtime_id().to_owned();
        run(
            cx,
            move || core.application(&device)?.runtime_models(runtime),
            |this, models, _| this.models = models.unwrap_or_default(),
        );
        if let Target::Existing {
            runtime_id,
            conversation_id,
            ..
        } = self.target.clone()
        {
            let core = self.core.clone();
            let device = self.device.clone();
            run(
                cx,
                move || {
                    core.application(&device)?
                        .read_conversation(runtime_id, conversation_id)
                },
                |this, result, cx| {
                    this.loading = false;
                    this.apply(result, cx);
                },
            );
        }
    }

    fn poll(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(delay) =
                    this.read_with(cx, |this, _| if this.running() { 700 } else { 3_000 })
                else {
                    return;
                };
                cx.background_executor()
                    .timer(Duration::from_millis(delay))
                    .await;
                let Ok(current) = this.read_with(cx, |this, _| {
                    this.snapshot.as_ref().map(|snapshot| {
                        (
                            this.core.clone(),
                            this.device.clone(),
                            snapshot.context_runtime_id.clone(),
                            snapshot.conversation.id.clone(),
                            snapshot.cursor,
                        )
                    })
                }) else {
                    return;
                };
                let Some((core, device, runtime, id, cursor)) = current else {
                    continue;
                };
                let update = cx
                    .background_executor()
                    .spawn(async move {
                        let application = core.application(&device)?;
                        match application.snapshot_update(runtime.clone(), id.clone(), cursor)? {
                            BindingSnapshotUpdate::Changed { snapshot } => Ok(Some(snapshot)),
                            BindingSnapshotUpdate::Unchanged => Ok(None),
                            // The device let the conversation go (it restarted, for one); read it again.
                            BindingSnapshotUpdate::Absent => {
                                Ok(application.read_conversation(runtime, id)?.snapshot)
                            }
                        }
                    })
                    .await;
                let applied = this.update(cx, |this, cx| {
                    match update {
                        Ok(Some(snapshot)) => this.merge(snapshot),
                        Ok(None) => {}
                        Err(error) => this.failure = Some(describe(&error)),
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
        })
    }

    fn apply(
        &mut self,
        result: Result<BindingSnapshotResult, BindingError>,
        _: &mut Context<Self>,
    ) {
        match result {
            Ok(result) => {
                self.failure = None;
                if let Some(snapshot) = result.snapshot {
                    self.merge(snapshot);
                }
            }
            Err(error) => self.failure = Some(describe(&error)),
        }
    }

    /// A snapshot with a base holds only changed messages; `message_ids` gives the whole order.
    fn merge(&mut self, snapshot: BindingConversationSnapshot) {
        let messages = if snapshot.base.is_none() {
            snapshot.messages.clone()
        } else {
            let mut known: HashMap<&str, &BindingMessage> = self
                .messages
                .iter()
                .map(|message| (message.id.as_str(), message))
                .collect();
            known.extend(
                snapshot
                    .messages
                    .iter()
                    .map(|message| (message.id.as_str(), message)),
            );
            snapshot
                .message_ids
                .iter()
                .filter_map(|id| known.get(id.as_str()).map(|message| (*message).clone()))
                .collect()
        };
        let unchanged = self
            .messages
            .iter()
            .zip(&messages)
            .take_while(|(old, new)| old.id == new.id && old.stamp == new.stamp)
            .count();
        self.list
            .splice(unchanged..self.messages.len(), messages.len() - unchanged);
        self.messages = messages;
        if self.selection.is_none() {
            self.selection = snapshot
                .model_record_key
                .clone()
                .map(|model_record_key| BindingModelSelection::Managed { model_record_key });
        }
        self.target = Target::Existing {
            runtime_id: snapshot.context_runtime_id.clone(),
            conversation_id: snapshot.conversation.id.clone(),
            title: snapshot.conversation.title.clone(),
        };
        self.snapshot = Some(snapshot);
    }

    /// Runs one write at a time and shows its failure.
    fn act(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Core, &Device) -> Result<BindingSnapshotResult, BindingError>
        + Send
        + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.failure = None;
        let core = self.core.clone();
        let device = self.device.clone();
        run(
            cx,
            move || work(&core, &device),
            |this, result, cx| {
                this.busy = false;
                this.apply(result, cx);
            },
        );
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_owned();
        let can_send = self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.actions.can_send);
        if self.busy || text.is_empty() || !can_send {
            return;
        }
        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));
        let turn = BindingComposedTurn {
            text: text.clone(),
            effort: None,
            permission_mode: None,
            attachments: Vec::new(),
        };
        let selection = self.selection.clone();
        let existing = self.snapshot.as_ref().map(|snapshot| {
            (
                snapshot.context_runtime_id.clone(),
                snapshot.conversation.id.clone(),
            )
        });
        let target = match (&self.target, &self.draft) {
            (Target::New { runtime_id, .. }, Some(draft)) => {
                let cwd = draft.cwd.read(cx).value().trim().to_owned();
                Target::New {
                    runtime_id: runtime_id.clone(),
                    cwd: (!cwd.is_empty()).then_some(cwd),
                }
            }
            (target, _) => target.clone(),
        };
        self.busy = true;
        self.failure = None;
        let core = self.core.clone();
        let device = self.device.clone();
        let composer = self.composer.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let application = core.application(&device)?;
                    let mut created = None;
                    let (runtime, id) = match existing {
                        Some(existing) => existing,
                        None => {
                            let Target::New { runtime_id, cwd } = target else {
                                return Err(BindingError::Unavailable);
                            };
                            let result = application.create_conversation_with_selection(
                                runtime_id,
                                cwd,
                                selection.clone(),
                            )?;
                            let snapshot = result.snapshot.ok_or(BindingError::Contract {
                                detail: "设备没有返回新会话".into(),
                            })?;
                            let key = (
                                snapshot.context_runtime_id.clone(),
                                snapshot.conversation.id.clone(),
                            );
                            created = Some(snapshot);
                            key
                        }
                    };
                    let sent = application.send_turn_with_selection(
                        runtime.clone(),
                        id,
                        runtime,
                        selection,
                        turn,
                    );
                    Ok((created, sent))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((created, sent)) => {
                        if let Some(created) = created {
                            this.draft = None;
                            this.merge(created);
                        }
                        if sent.is_err() {
                            composer.update(cx, |composer, cx| {
                                composer.set_value(text.clone(), window, cx)
                            });
                        }
                        this.apply(sent, cx);
                    }
                    Err(error) => {
                        composer.update(cx, |composer, cx| composer.set_value(text, window, cx));
                        this.failure = Some(describe(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let runtime = snapshot.context_runtime_id.clone();
        let id = snapshot.conversation.id.clone();
        self.act(cx, move |core, device| {
            core.application(device)?.cancel(runtime, id)
        });
    }

    fn reply(
        &mut self,
        interaction: String,
        reply: BindingRuntimeInteractionReply,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let runtime = snapshot.context_runtime_id.clone();
        let id = snapshot.conversation.id.clone();
        self.act(cx, move |core, device| {
            core.application(device)?
                .reply_runtime_interaction(runtime, id, interaction, reply)
        });
    }

    /// Large reasoning and tool output arrive by size; this reads one message's whole content.
    fn load_payload(&mut self, message: String, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let runtime = snapshot.context_runtime_id.clone();
        let id = snapshot.conversation.id.clone();
        let core = self.core.clone();
        let device = self.device.clone();
        run(
            cx,
            move || {
                core.application(&device)?
                    .message_payloads(runtime, id, vec![message])
            },
            |this, loaded, _| match loaded {
                Ok(loaded) => {
                    for message in loaded {
                        if let Some(index) = this.messages.iter().position(|m| m.id == message.id) {
                            this.list.splice(index..index + 1, 1);
                        }
                        this.payloads.insert(message.id.clone(), message);
                    }
                }
                Err(error) => this.failure = Some(describe(&error)),
            },
        );
    }

    fn toggle(&mut self, index: usize, key: String, deferred: bool, cx: &mut Context<Self>) {
        let opening = self.expanded.insert(key.clone());
        if !opening {
            self.expanded.remove(&key);
        }
        self.list.splice(index..index + 1, 1);
        if opening && deferred {
            self.load_payload(self.messages[index].id.clone(), cx);
        }
        cx.notify();
    }

    /// The user's messages on the right, the model's on the left, the
    /// system's and the Harness's in the middle.
    fn render_message(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self.messages.get(index) else {
            return div().into_any_element();
        };
        let message = self.payloads.get(&message.id).unwrap_or(message).clone();
        let blocks = message
            .blocks
            .iter()
            .enumerate()
            .map(|(position, block)| self.render_block(index, &message, position, block, cx))
            .collect::<Vec<_>>();
        let theme = cx.theme();
        let row = h_flex().w_full().px_6().py_2();
        match message.role {
            BindingMessageRole::User => row
                .justify_end()
                .child(
                    v_flex()
                        .max_w(px(560.))
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(px(12.))
                        .bg(theme.secondary)
                        .children(blocks),
                )
                .into_any_element(),
            BindingMessageRole::Assistant => row
                .child(v_flex().w_full().max_w(px(820.)).gap_2().children(blocks))
                .into_any_element(),
            BindingMessageRole::Tool | BindingMessageRole::System => row
                .child(v_flex().w_full().max_w(px(820.)).gap_1().children(blocks))
                .into_any_element(),
        }
    }

    fn render_block(
        &self,
        index: usize,
        message: &BindingMessage,
        position: usize,
        block: &BindingMessageBlock,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let id = SharedString::from(format!("{}/{position}", message.id));
        match block {
            BindingMessageBlock::Text { text } => match message.role {
                BindingMessageRole::User => div().child(text.clone()).into_any_element(),
                _ => TextView::markdown(id, text.clone())
                    .selectable(true)
                    .into_any_element(),
            },
            BindingMessageBlock::Notice { text } => div()
                .w_full()
                .text_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text.clone())
                .into_any_element(),
            BindingMessageBlock::Reasoning { summary, text } => {
                let title = if summary.trim().is_empty() {
                    "思考".to_owned()
                } else {
                    summary.clone()
                };
                self.disclosure(index, id, title, Some(text), cx)
            }
            BindingMessageBlock::Tool {
                title,
                state,
                output,
                ..
            } => {
                let suffix = match state {
                    BindingToolState::Pending => " · 等待",
                    BindingToolState::Running => " · 运行中",
                    BindingToolState::Completed => "",
                    BindingToolState::Failed => " · 失败",
                };
                self.disclosure(index, id, format!("{title}{suffix}"), output.as_ref(), cx)
            }
            BindingMessageBlock::Image { .. } => muted("［图片］", cx),
            BindingMessageBlock::File { path } => {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
                muted(format!("［文件］{name}"), cx)
            }
            BindingMessageBlock::AgentSender {
                runtime_name,
                title,
                ..
            } => muted(format!("来自 {runtime_name} · {title}"), cx),
        }
    }

    /// Reasoning and tool output stay folded; large ones are read when unfolded.
    fn disclosure(
        &self,
        index: usize,
        id: SharedString,
        title: String,
        payload: Option<&BindingMessagePayload>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let key = id.to_string();
        let open = self.expanded.contains(&key);
        let deferred = matches!(payload, Some(BindingMessagePayload::Deferred { .. }));
        let header = div()
            .id(id.clone())
            .cursor_pointer()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(format!("{} {title}", if open { "▾" } else { "▸" }))
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle(index, key.clone(), deferred, cx)),
            );
        v_flex()
            .w_full()
            .child(header)
            .when(open, |column| match payload {
                Some(BindingMessagePayload::Inline { text }) => column.child(
                    div()
                        .w_full()
                        .p_2()
                        .rounded(px(6.))
                        .bg(theme.muted)
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .child(text.clone()),
                ),
                Some(BindingMessagePayload::Deferred { .. }) => {
                    column.child(Spinner::new().small())
                }
                None => column,
            })
            .into_any_element()
    }

    /// Keeps one text field per open question of the interaction shown.
    fn prepare_answers(
        &mut self,
        interaction: Option<&BindingRuntimeInteraction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = interaction.map(|interaction| interaction.id.clone());
        if id == self.answering {
            return;
        }
        self.answering = id;
        self.answers.clear();
        self.chosen.clear();
        if let Some(BindingRuntimeInteraction {
            kind: BindingInteractionKind::UserInput { questions },
            ..
        }) = interaction
        {
            for question in questions
                .iter()
                .filter(|question| question.options.is_empty())
            {
                let secret = question.secret;
                let input = cx.new(|cx| InputState::new(window, cx).masked(secret));
                self.answers.insert(question.id.clone(), input);
            }
        }
    }

    fn render_interaction(
        &self,
        interaction: &BindingRuntimeInteraction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let id = interaction.id.clone();
        let card = v_flex()
            .mx_4()
            .my_2()
            .p_4()
            .gap_3()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover);
        match &interaction.kind {
            BindingInteractionKind::Approval {
                title,
                detail,
                options,
            } => card
                .child(div().font_semibold().child(title.clone()))
                .when(!detail.trim().is_empty(), |card| {
                    card.child(
                        div()
                            .id("interaction-detail")
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .p_2()
                            .rounded(px(6.))
                            .bg(theme.muted)
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .child(detail.clone()),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .children(options.iter().map(|option| {
                            let interaction = id.clone();
                            let choice = option.id.clone();
                            Button::new(SharedString::from(format!("option-{}", option.id)))
                                .label(option.label.clone())
                                .disabled(self.busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.reply(
                                        interaction.clone(),
                                        BindingRuntimeInteractionReply::Decision {
                                            option_id: choice.clone(),
                                        },
                                        cx,
                                    )
                                }))
                        })),
                )
                .into_any_element(),
            BindingInteractionKind::UserInput { questions } => {
                let complete = questions.iter().all(|question| {
                    self.chosen.contains_key(&question.id)
                        || self
                            .answers
                            .get(&question.id)
                            .is_some_and(|input| !input.read(cx).value().trim().is_empty())
                });
                let mut card = card;
                for question in questions {
                    card = card.child(div().font_semibold().child(question.text.clone()));
                    if question.options.is_empty() {
                        if let Some(input) = self.answers.get(&question.id) {
                            card = card.child(Input::new(input));
                        }
                    } else {
                        let chosen = self.chosen.get(&question.id).cloned();
                        card = card.child(h_flex().gap_2().flex_wrap().children(
                            question.options.iter().map(|option| {
                                let question = question.id.clone();
                                let label = option.label.clone();
                                let button = Button::new(SharedString::from(format!(
                                    "answer-{question}-{}",
                                    option.id
                                )))
                                .label(option.label.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.chosen.insert(question.clone(), label.clone());
                                    cx.notify();
                                }));
                                if chosen.as_deref() == Some(option.label.as_str()) {
                                    button.primary()
                                } else {
                                    button.outline()
                                }
                            }),
                        ));
                    }
                }
                let submit_questions = questions.clone();
                let submit_id = id.clone();
                card.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("answers-submit")
                                .primary()
                                .label("提交")
                                .disabled(self.busy || !complete)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let answers = submit_questions
                                        .iter()
                                        .map(|question| BindingInteractionAnswer {
                                            question_id: question.id.clone(),
                                            values: vec![
                                                this.chosen
                                                    .get(&question.id)
                                                    .cloned()
                                                    .unwrap_or_else(|| {
                                                        this.answers
                                                            .get(&question.id)
                                                            .map(|input| {
                                                                input.read(cx).value().to_string()
                                                            })
                                                            .unwrap_or_default()
                                                    }),
                                            ],
                                        })
                                        .collect();
                                    this.reply(
                                        submit_id.clone(),
                                        BindingRuntimeInteractionReply::Answers { answers },
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("answers-cancel")
                                .ghost()
                                .label("取消")
                                .disabled(self.busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.reply(
                                        id.clone(),
                                        BindingRuntimeInteractionReply::Cancel,
                                        cx,
                                    )
                                })),
                        ),
                )
                .into_any_element()
            }
        }
    }

    /// Where a new conversation runs: its runtime and working directory.
    fn render_draft(&self, draft: &Draft, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let current = self.target.runtime_id().to_owned();
        let runtime_name = draft
            .runtimes
            .iter()
            .find(|runtime| runtime.id == current)
            .map(|runtime| runtime.name.clone())
            .unwrap_or_else(|| "选择运行时".into());
        let runtimes = draft.runtimes.clone();
        let projects = draft.projects.clone();
        let cwd = draft.cwd.clone();
        h_flex()
            .px_4()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("runtime")
                    .outline()
                    .label(runtime_name)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for runtime in &runtimes {
                            let view = view.clone();
                            let id = runtime.id.clone();
                            menu = menu.item(
                                PopupMenuItem::new(runtime.name.clone())
                                    .checked(runtime.id == current)
                                    .on_click(move |_, _, cx| {
                                        let id = id.clone();
                                        let _ =
                                            view.update(cx, |this, cx| this.choose_runtime(id, cx));
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .child(div().flex_1().child(Input::new(&draft.cwd)))
            .when(!projects.is_empty(), |row| {
                row.child(
                    Button::new("projects")
                        .ghost()
                        .label("最近的项目")
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for project in &projects {
                                let cwd = cwd.clone();
                                let path = project.clone();
                                menu = menu.item(PopupMenuItem::new(project.clone()).on_click(
                                    move |_, window, cx| {
                                        cwd.update(cx, |input, cx| {
                                            input.set_value(path.clone(), window, cx)
                                        });
                                    },
                                ));
                            }
                            menu
                        }),
                )
            })
            .when(draft.local, |row| {
                row.child(
                    Button::new("browse")
                        .label("浏览…")
                        .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))),
                )
            })
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let model_name = self
            .models
            .iter()
            .find(|model| Some(&model.selection) == self.selection.as_ref())
            .map(|model| model.name.clone())
            .unwrap_or_else(|| "运行时默认模型".into());
        let models = self.models.clone();
        let selection = self.selection.clone();
        let can_cancel = self.running()
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.actions.can_cancel);
        v_flex()
            .px_4()
            .py_3()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(Textarea::new(&self.composer))
            .child(
                h_flex()
                    .gap_2()
                    .when(!models.is_empty(), |row| {
                        row.child(
                            Button::new("model")
                                .ghost()
                                .small()
                                .label(model_name)
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    let choices =
                                        std::iter::once((None, "运行时默认模型".to_owned())).chain(
                                            models.iter().map(|model| {
                                                (Some(model.selection.clone()), model.name.clone())
                                            }),
                                        );
                                    for (choice, name) in choices {
                                        let view = view.clone();
                                        let checked = choice == selection;
                                        menu = menu.item(
                                            PopupMenuItem::new(name).checked(checked).on_click(
                                                move |_, _, cx| {
                                                    let choice = choice.clone();
                                                    let _ = view.update(cx, |this, cx| {
                                                        this.selection = choice;
                                                        cx.notify();
                                                    });
                                                },
                                            ),
                                        );
                                    }
                                    menu
                                }),
                        )
                    })
                    .when(self.running(), |row| {
                        row.child(Spinner::new().small())
                            .child(muted("正在运行", cx))
                    })
                    .child(div().flex_1())
                    .child(if can_cancel {
                        Button::new("stop")
                            .label("停止")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                    } else {
                        Button::new("send")
                            .primary()
                            .label("发送")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                    }),
            )
    }
}

impl Render for ConversationView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let interaction = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.pending_interactions.first().cloned());
        self.prepare_answers(interaction.as_ref(), window, cx);
        let empty = if self.loading {
            Some(Spinner::new().into_any_element())
        } else if self.messages.is_empty() {
            Some(muted(
                if self.snapshot.is_none() {
                    "输入第一条消息开始会话。"
                } else {
                    "还没有消息。"
                },
                cx,
            ))
        } else {
            None
        };
        v_flex()
            .size_full()
            .when_some(
                self.draft
                    .as_ref()
                    .filter(|_| self.snapshot.is_none())
                    .map(|draft| self.render_draft(draft, cx).into_any_element()),
                |column, header| column.child(header),
            )
            .child(match empty {
                Some(empty) => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(empty)
                    .into_any_element(),
                None => list(
                    self.list.clone(),
                    cx.processor(|this, index, _, cx| this.render_message(index, cx)),
                )
                .flex_1()
                .py_2()
                .into_any_element(),
            })
            .when_some(self.failure.clone(), |column, failure| {
                column.child(
                    div()
                        .px_6()
                        .py_1()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(failure),
                )
            })
            .when_some(interaction, |column, interaction| {
                column.child(self.render_interaction(&interaction, cx))
            })
            .child(self.render_composer(cx))
    }
}

pub fn muted(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}
