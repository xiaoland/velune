//! The settings window: devices, and this PC's runtimes and providers.
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, tab::TabBar, v_flex};
use gpui_kit::*;

use crate::{devices::Devices, hub::Hub, providers::Providers, runtimes::Runtimes};

pub struct Settings {
    tab: usize,
    devices: Entity<Devices>,
    runtimes: Entity<Runtimes>,
    providers: Entity<Providers>,
}

impl Settings {
    pub fn new(hub: Entity<Hub>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let core = hub.read(cx).core.clone();
        Self {
            tab: 0,
            devices: cx.new(|cx| Devices::new(hub, window, cx)),
            runtimes: cx.new(|cx| Runtimes::new(core.clone(), cx)),
            providers: cx.new(|cx| Providers::new(core, window, cx)),
        }
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let pane = match self.tab {
            0 => self.devices.clone().into_any_element(),
            1 => self.runtimes.clone().into_any_element(),
            _ => self.providers.clone().into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TabBar::new("settings-tabs")
                    .underline()
                    .px_4()
                    .selected_index(self.tab)
                    .children(["设备", "运行时", "提供商"])
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.tab = *index;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("settings-pane")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(pane),
            )
    }
}

/// A titled group of settings rows.
pub fn section(title: &'static str, cx: &App) -> Div {
    v_flex().gap_2().px_6().py_4().child(
        div()
            .text_sm()
            .font_semibold()
            .text_color(cx.theme().muted_foreground)
            .child(title),
    )
}

/// One row: what it is on the left, its controls on the right.
pub fn row(cx: &App) -> Div {
    gpui_kit::component::h_flex()
        .w_full()
        .gap_3()
        .px_3()
        .py_2()
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border)
}
