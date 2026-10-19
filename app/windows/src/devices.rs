//! Pairing and the devices on either side of control.
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex,
    input::{Input, InputState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use velune_bindings::{
    BindingContactConfirmation, BindingControllerPhase, BindingError, BindingPairingPhase,
    BindingRemoteInvitation, VeluneRemoteEndpoint, decode_remote_offer,
};

use crate::{
    conversation::muted,
    core::{describe, run},
    hub::Hub,
    qr,
    settings::{row, section},
    workspace::format_code,
};

pub struct Devices {
    hub: Entity<Hub>,
    invitation: Option<BindingRemoteInvitation>,
    invite: Entity<InputState>,
    /// The device this PC asked to control, until it answers.
    asking: Option<String>,
    busy: bool,
    failure: Option<String>,
    _hub: Subscription,
}

impl Devices {
    pub fn new(hub: Entity<Hub>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&hub, |this, hub, cx| {
            // A shown invitation is spent once someone used it, or expired.
            let used = hub.read(cx).state.as_ref().is_some_and(|state| {
                state
                    .incoming_pairings
                    .iter()
                    .any(|pairing| matches!(pairing.phase, BindingPairingPhase::AwaitingApproval))
            });
            if used
                || this
                    .invitation
                    .as_ref()
                    .is_some_and(|i| expired(i.expires_at_unix_seconds))
            {
                this.invitation = None;
            }
            cx.notify();
        });
        Self {
            hub,
            invitation: None,
            invite: cx.new(|cx| InputState::new(window, cx).placeholder("粘贴另一台设备的邀请")),
            asking: None,
            busy: false,
            failure: None,
            _hub: subscription,
        }
    }

    /// Runs one call on the remote service and shows its failure.
    fn perform<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&VeluneRemoteEndpoint) -> Result<R, BindingError> + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        if self.busy {
            return;
        }
        let core = self.hub.read(cx).core.clone();
        self.busy = true;
        self.failure = None;
        run(
            cx,
            move || work(&*core.endpoint()?),
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(value) => done(this, value, cx),
                    Err(error) => this.failure = Some(describe(&error)),
                }
            },
        );
    }

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let encoded = self.invite.read(cx).value().trim().to_owned();
        if encoded.is_empty() {
            return;
        }
        if let Err(error) = decode_remote_offer(encoded.clone()) {
            self.failure = Some(describe(&error));
            cx.notify();
            return;
        }
        self.invite
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.perform(
            cx,
            move |endpoint| endpoint.begin_pairing(encoded),
            |this, status, _| this.asking = Some(status.peer_id),
        );
    }

    fn render_this_pc(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hub = self.hub.read(cx);
        let state = hub.state.as_ref();
        let ingress = state.is_some_and(|state| state.control_ingress);
        section("这台电脑", cx)
            .child(
                row(cx)
                    .child(div().flex_1().child("设备名称"))
                    .child(muted(hub.device_name(&crate::core::Device::Local), cx)),
            )
            .child(
                row(cx)
                    .child(
                        v_flex()
                            .flex_1()
                            .child("允许其他设备控制这台电脑")
                            .child(muted("关闭后，已配对的设备暂时无法连接。", cx)),
                    )
                    .child(
                        Switch::new("ingress")
                            .checked(ingress)
                            .disabled(state.is_none() || self.busy)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                let enabled = *checked;
                                this.perform(
                                    cx,
                                    move |endpoint| endpoint.set_control_ingress(enabled),
                                    |_, _, _| {},
                                );
                            })),
                    ),
            )
    }

    fn render_invitation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.hub.read(cx).state.is_some();
        let column = section("添加控制设备", cx).child(muted(
            "在 Android 手机或另一台电脑上扫描二维码或粘贴邀请。对方连接后，这里会请你核对并允许。",
            cx,
        ));
        match &self.invitation {
            None => column.child(
                h_flex().child(
                    Button::new("invite")
                        .label("创建邀请")
                        .disabled(!ready || self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.perform(
                                cx,
                                |endpoint| endpoint.create_invitation(300),
                                |this, invitation, _| this.invitation = Some(invitation),
                            )
                        })),
                ),
            ),
            Some(invitation) => column.child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .children(qr::code(&invitation.encoded, px(4.)))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_2()
                            .child(muted("邀请五分钟内有效，只能使用一次。", cx))
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .line_clamp(4)
                                    .child(invitation.encoded.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Clipboard::new("copy-invitation")
                                            .value(invitation.encoded.clone()),
                                    )
                                    .child(muted("复制邀请", cx))
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("discard")
                                            .ghost()
                                            .small()
                                            .label("收回")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.invitation = None;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    ),
            ),
        }
    }

    fn render_grants(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let grants = self
            .hub
            .read(cx)
            .state
            .as_ref()
            .map(|state| state.incoming_grants.clone())
            .unwrap_or_default();
        section("可以控制这台电脑的设备", cx)
            .when(grants.is_empty(), |column| {
                column.child(muted("没有。", cx))
            })
            .children(grants.into_iter().map(|grant| {
                let peer = grant.peer_id.clone();
                row(cx).child(div().flex_1().child(grant.name)).child(
                    Button::new(SharedString::from(format!("revoke-{peer}")))
                        .small()
                        .label("撤销")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let peer = peer.clone();
                            this.perform(
                                cx,
                                move |endpoint| endpoint.revoke_control(peer),
                                |_, _, _| {},
                            );
                        })),
                )
            }))
    }

    fn render_contacts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.hub.read(cx).state.clone();
        let contacts = state
            .as_ref()
            .map(|state| state.contacts.clone())
            .unwrap_or_default();
        let outgoing = state
            .as_ref()
            .map(|state| state.outgoing_pairings.clone())
            .unwrap_or_default();
        let controllers = state
            .as_ref()
            .map(|state| state.controllers.clone())
            .unwrap_or_default();
        section("这台电脑可以控制的设备", cx)
            .when(contacts.is_empty() && outgoing.is_empty(), |column| {
                column.child(muted("没有。", cx))
            })
            .children(contacts.into_iter().map(|contact| {
                let pending = matches!(
                    contact.confirmation,
                    BindingContactConfirmation::Pending { .. }
                );
                let pairing = outgoing
                    .iter()
                    .find(|pairing| pairing.peer_id == contact.peer_id);
                let status = match (pending, pairing) {
                    (true, Some(pairing)) => match (&pairing.phase, &pairing.verification_code) {
                        (BindingPairingPhase::AwaitingApproval, Some(code)) => {
                            format!("核对码 {}，等待对方允许", format_code(code))
                        }
                        (BindingPairingPhase::Rejected, _) => "对方已拒绝".into(),
                        (BindingPairingPhase::Expired, _) => "邀请已过期".into(),
                        _ => "正在连接…".into(),
                    },
                    (true, None) => "等待对方允许".into(),
                    (false, _) => controllers
                        .iter()
                        .find(|controller| controller.peer_id == contact.peer_id)
                        .map(|controller| phase_name(&controller.phase))
                        .unwrap_or("已配对")
                        .into(),
                };
                let peer = contact.peer_id.clone();
                row(cx)
                    .child(
                        v_flex()
                            .flex_1()
                            .child(contact.name)
                            .child(muted(status, cx)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("forget-{peer}")))
                            .small()
                            .label(if pending { "取消" } else { "移除" })
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let peer = peer.clone();
                                this.hub.read(cx).core.forget_connection(&peer);
                                this.perform(
                                    cx,
                                    move |endpoint| {
                                        if pending {
                                            endpoint.cancel_pairing(peer.clone()).map(|_| ())?;
                                        }
                                        endpoint.forget_device(peer)
                                    },
                                    |_, _, _| {},
                                );
                            })),
                    )
            }))
    }

    fn render_connect(&self, cx: &mut Context<Self>) -> impl IntoElement {
        section("控制另一台设备", cx)
            .child(muted(
                "在另一台电脑的“设置 › 设备”中创建邀请，然后粘贴到这里。",
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.invite)))
                    .child(
                        Button::new("connect")
                            .primary()
                            .label("连接")
                            .disabled(self.busy || self.hub.read(cx).state.is_none())
                            .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
                    ),
            )
    }
}

impl Render for Devices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let failure = self.failure.clone().or(self.hub.read(cx).failure.clone());
        v_flex()
            .pb_4()
            .when_some(failure, |column, failure| {
                column.child(
                    div()
                        .px_6()
                        .pt_4()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(failure),
                )
            })
            .child(self.render_this_pc(cx))
            .child(self.render_invitation(cx))
            .child(self.render_grants(cx))
            .child(self.render_connect(cx))
            .child(self.render_contacts(cx))
    }
}

fn phase_name(phase: &BindingControllerPhase) -> &'static str {
    match phase {
        BindingControllerPhase::Connected => "已连接",
        BindingControllerPhase::Connecting | BindingControllerPhase::Discovering => "正在连接",
        BindingControllerPhase::Recovering => "正在恢复连接",
        BindingControllerPhase::Suspended | BindingControllerPhase::Disconnected => "未连接",
        BindingControllerPhase::Blocked => "无法连接",
    }
}

fn expired(expires_at_unix_seconds: u64) -> bool {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .is_ok_and(|now| now.as_secs() > expires_at_unix_seconds)
}
