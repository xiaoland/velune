//! The main window: one interface whose target device the user switches.
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Root, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use velune_bindings::{
    BindingActiveConversation, BindingConfigurationSnapshot, BindingConversationOrganizationChange,
    BindingConversationReference, BindingListedConversation, BindingPairingPhase,
    BindingPairingStatus, BindingRunState, BindingRuntimeInstance,
};

use crate::{
    conversation::{ConversationView, Target, muted},
    core::{Device, describe, run},
    hub::Hub,
    settings::Settings,
};

#[derive(Default)]
struct Listing {
    conversations: Vec<BindingListedConversation>,
    runtimes: Vec<BindingRuntimeInstance>,
    /// Working directories of the device's conversations, for starting another in one.
    projects: Vec<String>,
    /// Running or waiting conversations by ID.
    active: HashMap<String, BindingActiveConversation>,
    loaded: bool,
    failure: Option<String>,
}

impl Listing {
    fn new(list: BindingConfigurationSnapshot, active: Vec<BindingActiveConversation>) -> Self {
        let mut projects = Vec::new();
        for project in list.conversation_projects {
            if !projects.contains(&project.path) {
                projects.push(project.path);
            }
        }
        Self {
            conversations: list
                .conversations
                .into_iter()
                .filter(|listed| {
                    !listed.archived && listed.conversation.parent_conversation_id.is_none()
                })
                .collect(),
            runtimes: list
                .runtime_instances
                .into_iter()
                .filter(|runtime| runtime.enabled)
                .collect(),
            projects,
            active: active
                .into_iter()
                .map(|active| (active.conversation_id.clone(), active))
                .collect(),
            loaded: true,
            failure: None,
        }
    }
}

/// A conversation title being edited in the list.
struct Renaming {
    runtime_id: String,
    conversation_id: String,
    title: Entity<InputState>,
    _events: Subscription,
}

pub struct Workspace {
    hub: Entity<Hub>,
    device: Device,
    listing: Listing,
    conversation: Option<Entity<ConversationView>>,
    /// Pairing attempts already put to the user.
    prompted: HashSet<String>,
    settings: Option<WindowHandle<Root>>,
    renaming: Option<Renaming>,
    /// Conversations whose read mark is on its way; the next listing may still show them unread.
    marking_read: HashSet<String>,
    foreground: bool,
    _listing: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(hub: Entity<Hub>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe_in(&hub, window, |this, _, window, cx| {
                this.hub_changed(window, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.foreground = window.is_window_active();
                this.hub
                    .read(cx)
                    .set_activity(&this.device, &this.device, this.foreground);
                this.note_seen(cx);
            }),
        ];
        let mut workspace = Self {
            hub,
            device: Device::Local,
            listing: Listing::default(),
            conversation: None,
            prompted: HashSet::new(),
            settings: None,
            renaming: None,
            marking_read: HashSet::new(),
            foreground: true,
            _listing: Task::ready(()),
            _subscriptions: subscriptions,
        };
        workspace._listing = workspace.poll_listing(cx);
        workspace
    }

    fn hub_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hub = self.hub.read(cx);
        // The device was removed or its pairing undone: return to this PC.
        if let Device::Remote(peer) = &self.device
            && hub.state.is_some()
            && !hub.devices().iter().any(|device| &device.peer_id == peer)
        {
            self.switch(Device::Local, cx);
        }
        let waiting: Vec<BindingPairingStatus> = self
            .hub
            .read(cx)
            .state
            .iter()
            .flat_map(|state| state.incoming_pairings.iter())
            .filter(|pairing| matches!(pairing.phase, BindingPairingPhase::AwaitingApproval))
            .filter(|pairing| !self.prompted.contains(&pairing.attempt_id))
            .cloned()
            .collect();
        for pairing in waiting {
            self.prompted.insert(pairing.attempt_id.clone());
            self.prompt_approval(pairing, window, cx);
        }
        cx.notify();
    }

    /// Both devices show the same code; only the user here can let the other one in.
    /// Neither answer is the default, so a stray Enter grants nothing.
    fn prompt_approval(
        &mut self,
        pairing: BindingPairingStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.activate_window();
        let core = self.hub.read(cx).core.clone();
        let code = pairing
            .verification_code
            .clone()
            .map(|code| format_code(&code))
            .unwrap_or_default();
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let answer = |allow: bool| {
                let core = core.clone();
                let pairing = pairing.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let core = core.clone();
                    let pairing = pairing.clone();
                    cx.background_executor()
                        .spawn(async move {
                            let Ok(endpoint) = core.endpoint() else {
                                return;
                            };
                            let _ = if allow {
                                endpoint.approve_pairing(pairing.attempt_id, pairing.peer_id)
                            } else {
                                endpoint.reject_pairing(pairing.attempt_id, pairing.peer_id)
                            };
                        })
                        .detach();
                    window.close_dialog(cx);
                }
            };
            dialog
                .title(format!("“{}”请求控制这台电脑", pairing.name))
                .w(px(440.))
                .overlay_closable(false)
                .keyboard(false)
                .child(
                    v_flex()
                        .gap_3()
                        .child("确认两台设备显示相同的核对码。允许后，对方可以查看和操作这台电脑上的会话。")
                        .child(
                            div()
                                .w_full()
                                .text_center()
                                .text_3xl()
                                .font_family(theme.mono_font_family.clone())
                                .child(code.clone()),
                        ),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(Button::new("reject").label("拒绝").on_click(answer(false)))
                        .child(Button::new("allow").label("允许").on_click(answer(true))),
                )
        });
    }

    fn poll_listing(&self, cx: &mut Context<Self>) -> Task<()> {
        let core = self.hub.read(cx).core.clone();
        let device = self.device.clone();
        cx.spawn(async move |this, cx| {
            let mut cursor = 0;
            loop {
                let (core, device) = (core.clone(), device.clone());
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let application = core.application(&device)?;
                        Ok::<_, velune_bindings::BindingError>((
                            application.list()?,
                            application.conversation_activity(cursor)?,
                        ))
                    })
                    .await;
                let Ok(busy) = this.update(cx, |this, cx| {
                    match result {
                        Ok((list, activity)) => {
                            cursor = activity.cursor;
                            this.listing = Listing::new(list, activity.active);
                            this.note_seen(cx);
                        }
                        Err(error) => {
                            this.listing.loaded = true;
                            this.listing.failure = Some(describe(&error));
                        }
                    }
                    cx.notify();
                    !this.listing.active.is_empty()
                }) else {
                    return;
                };
                cx.background_executor()
                    .timer(Duration::from_secs(if busy { 2 } else { 5 }))
                    .await;
            }
        })
    }

    fn switch(&mut self, device: Device, cx: &mut Context<Self>) {
        if device == self.device {
            return;
        }
        self.hub
            .read(cx)
            .set_activity(&self.device, &device, self.foreground);
        self.device = device;
        self.conversation = None;
        self.listing = Listing::default();
        self._listing = self.poll_listing(cx);
        cx.notify();
    }

    fn open(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.hub.read(cx).core.clone();
        let device = self.device.clone();
        self.conversation =
            Some(cx.new(|cx| ConversationView::new(core, device, target, window, cx)));
        self.note_seen(cx);
        cx.notify();
    }

    /// The open conversation is read once it shows in the active window.
    fn note_seen(&mut self, cx: &mut Context<Self>) {
        if !self.foreground {
            return;
        }
        let Some(id) = self
            .conversation
            .as_ref()
            .and_then(|view| view.read(cx).target.conversation_id().map(str::to_owned))
        else {
            return;
        };
        let Some(listed) = self
            .listing
            .conversations
            .iter()
            .find(|listed| listed.conversation.id == id && listed.unread)
        else {
            return;
        };
        if !self.marking_read.insert(id.clone()) {
            return;
        }
        let reference = BindingConversationReference {
            runtime_id: listed.conversation.runtime_id.clone(),
            conversation_id: id.clone(),
        };
        let core = self.hub.read(cx).core.clone();
        let device = self.device.clone();
        run(
            cx,
            move || {
                core.application(&device)?.organize_conversations(
                    vec![reference],
                    BindingConversationOrganizationChange::MarkRead,
                )
            },
            move |this: &mut Workspace, result, _| {
                this.marking_read.remove(&id);
                match result {
                    Ok(_) => {
                        if let Some(listed) = this
                            .listing
                            .conversations
                            .iter_mut()
                            .find(|listed| listed.conversation.id == id)
                        {
                            listed.unread = false;
                        }
                    }
                    Err(error) => this.listing.failure = Some(describe(&error)),
                }
            },
        );
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let hub = self.hub.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(760.), px(620.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("设置".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.settings = cx
            .open_window(options, |window, cx| {
                let settings = cx.new(|cx| Settings::new(hub, window, cx));
                cx.new(|cx| Root::new(settings, window, cx))
            })
            .ok();
    }

    fn start_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.hub.read(cx).core.clone();
        let device = self.device.clone();
        let runtimes = self.listing.runtimes.clone();
        let projects = self.listing.projects.clone();
        self.conversation =
            Some(cx.new(|cx| {
                ConversationView::new_draft(core, device, runtimes, projects, window, cx)
            }));
        cx.notify();
    }

    /// Double-click edits the conversation's real title in place.
    fn rename(
        &mut self,
        listed: BindingListedConversation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conversation = listed.conversation;
        if !conversation.can_rename {
            return;
        }
        let title =
            cx.new(|cx| InputState::new(window, cx).default_value(conversation.title.clone()));
        title.update(cx, |input, cx| input.focus(window, cx));
        let subscription =
            cx.subscribe_in(&title, window, |this, input, event, _, cx| match event {
                InputEvent::PressEnter { .. } => {
                    let text = input.read(cx).value().trim().to_owned();
                    this.finish_rename(Some(text), cx);
                }
                InputEvent::Blur => this.finish_rename(None, cx),
                _ => {}
            });
        self.renaming = Some(Renaming {
            runtime_id: conversation.runtime_id,
            conversation_id: conversation.id,
            title,
            _events: subscription,
        });
        cx.notify();
    }

    fn finish_rename(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        cx.notify();
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return;
        };
        let core = self.hub.read(cx).core.clone();
        let device = self.device.clone();
        if let Some(listed) = self
            .listing
            .conversations
            .iter_mut()
            .find(|listed| listed.conversation.id == renaming.conversation_id)
        {
            listed.conversation.title = text.clone();
        }
        run(
            cx,
            move || {
                core.application(&device)?.rename_conversation(
                    renaming.runtime_id,
                    renaming.conversation_id,
                    text,
                )
            },
            |this: &mut Workspace, result, _| {
                if let Err(error) = result {
                    this.listing.failure = Some(describe(&error));
                }
            },
        );
    }

    fn render_device_switcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hub = self.hub.read(cx);
        let name = hub.device_name(&self.device);
        let label = match self.device {
            Device::Local => format!("{name}（本机）"),
            Device::Remote(_) => name,
        };
        let mut choices = vec![(
            Device::Local,
            format!("{}（本机）", hub.device_name(&Device::Local)),
        )];
        choices.extend(
            hub.devices()
                .into_iter()
                .map(|contact| (Device::Remote(contact.peer_id), contact.name)),
        );
        let current = self.device.clone();
        let view = cx.entity().downgrade();
        Button::new("device")
            .ghost()
            .label(label)
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (device, name) in &choices {
                    let view = view.clone();
                    let device = device.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(device == current)
                            .on_click(move |_, _, cx| {
                                let device = device.clone();
                                let _ = view.update(cx, |this, cx| this.switch(device, cx));
                            }),
                    );
                }
                menu.separator()
                    .item(PopupMenuItem::new("添加设备…").on_click({
                        let view = view.clone();
                        move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| this.open_settings(cx));
                        }
                    }))
            })
    }

    fn render_row(
        &self,
        index: usize,
        listed: &BindingListedConversation,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let conversation = &listed.conversation;
        let runtime = self
            .listing
            .runtimes
            .iter()
            .find(|runtime| runtime.id == conversation.runtime_id)
            .map(|runtime| runtime.name.clone());
        let detail = [
            runtime,
            conversation.updated_at_unix_ms.map(relative_time),
            listed
                .project
                .as_ref()
                .and_then(|project| project.rsplit(['/', '\\']).next().map(str::to_owned)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let active = self.listing.active.get(&conversation.id);
        let status = if active.is_some_and(|active| active.pending_interactions > 0) {
            Some(
                div()
                    .text_xs()
                    .text_color(theme.primary)
                    .child("等待回应")
                    .into_any_element(),
            )
        } else if active.is_some_and(|active| matches!(active.run_state, BindingRunState::Running))
        {
            Some(Spinner::new().small().into_any_element())
        } else if listed.failed {
            Some(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child("失败")
                    .into_any_element(),
            )
        } else if listed.unread {
            Some(
                div()
                    .text_xs()
                    .text_color(theme.primary)
                    .child("●")
                    .into_any_element(),
            )
        } else {
            None
        };
        let selected = self
            .conversation
            .as_ref()
            .and_then(|view| view.read(cx).target.conversation_id().map(str::to_owned))
            .is_some_and(|id| id == conversation.id);
        let target = Target::Existing {
            runtime_id: conversation.runtime_id.clone(),
            conversation_id: conversation.id.clone(),
            title: conversation.title.clone(),
        };
        let renaming = listed.clone();
        h_flex()
            .id(("conversation", index))
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .rounded(px(6.))
            .when(selected, |row| row.bg(theme.sidebar_accent))
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(match &self.renaming {
                        // GPUI keeps focus when a click lands on something unfocusable, so
                        // a click anywhere else, or Esc, cancels explicitly.
                        Some(renaming) if renaming.conversation_id == conversation.id => div()
                            .on_mouse_down_out(
                                cx.listener(|this, _, _, cx| this.finish_rename(None, cx)),
                            )
                            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if event.keystroke.key == "escape" {
                                    cx.stop_propagation();
                                    this.finish_rename(None, cx);
                                }
                            }))
                            .child(Input::new(&renaming.title).small())
                            .into_any_element(),
                        _ => div()
                            .truncate()
                            .child(conversation.title.clone())
                            .into_any_element(),
                    })
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    ),
            )
            .children(status)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.rename(renaming.clone(), window, cx);
                } else if !selected {
                    this.open(target.clone(), window, cx);
                }
            }))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let hub = self.hub.read(cx);
        let device_name = hub.device_name(&self.device);
        let hub_failure = hub.failure.clone();
        let title = self
            .conversation
            .as_ref()
            .map(|view| view.read(cx).title())
            .unwrap_or_else(|| "Velune".into());
        window.set_window_title(&format!("{title} — {device_name}"));
        let rows = self
            .listing
            .conversations
            .iter()
            .enumerate()
            .map(|(index, listed)| self.render_row(index, listed, cx).into_any_element())
            .collect::<Vec<_>>();
        let sidebar = v_flex()
            .w(px(300.))
            .flex_none()
            .h_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                h_flex()
                    .px_2()
                    .py_2()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_device_switcher(cx)),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .icon(IconName::Settings)
                            .tooltip("设置")
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                    ),
            )
            .child(
                div().px_2().pb_2().child(
                    Button::new("new")
                        .w_full()
                        .icon(IconName::Plus)
                        .label("新会话")
                        .disabled(self.listing.runtimes.is_empty())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_conversation(window, cx)),
                        ),
                ),
            )
            .when_some(hub_failure, |column, failure| {
                column.child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(failure),
                )
            })
            .when_some(self.listing.failure.clone(), |column, failure| {
                column.child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(failure),
                )
            })
            .child(
                div()
                    .id("conversations")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_2()
                    .child(if !self.listing.loaded {
                        div().p_4().child(Spinner::new()).into_any_element()
                    } else if rows.is_empty() {
                        div()
                            .p_4()
                            .child(muted(
                                if self.listing.runtimes.is_empty() {
                                    "这台设备还没有可用的运行时。"
                                } else {
                                    "这台设备上还没有会话。"
                                },
                                cx,
                            ))
                            .into_any_element()
                    } else {
                        v_flex().gap_1().children(rows).into_any_element()
                    }),
            );
        // A long unbroken line (a failure with a path, say) wraps instead of widening the pane.
        let main = match &self.conversation {
            Some(view) => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
                .child(view.clone())
                .into_any_element(),
            None => div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(muted("选择或新建一个会话", cx))
                .into_any_element(),
        };
        h_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(sidebar)
            .child(main)
    }
}

/// Groups the eight digits as four and four, as the other device shows them.
pub fn format_code(code: &str) -> String {
    if code.len() == 8 && code.is_ascii() {
        format!("{} {}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    }
}

fn relative_time(unix_ms: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(unix_ms);
    let minutes = (now - unix_ms).max(0) / 60_000;
    match minutes {
        0 => "刚刚".into(),
        1..60 => format!("{minutes} 分钟前"),
        60..1440 => format!("{} 小时前", minutes / 60),
        _ => format!("{} 天前", minutes / 1440),
    }
}
