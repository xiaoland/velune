//! Velune for Windows: a full device that runs agents on this PC and controls
//! paired devices through the same interface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod conversation;
mod core;
mod devices;
mod discovery;
mod hub;
mod providers;
mod qr;
mod runtimes;
mod settings;
mod workspace;

use gpui_kit::component::Root;
use gpui_kit::*;

use crate::{core::Core, hub::Hub, workspace::Workspace};

struct StartupFailure(SharedString);

impl Render for StartupFailure {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .child(self.0.clone())
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(780.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Velune".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let opened = match Core::open() {
                Ok(core) => {
                    let shutdown = core.clone();
                    cx.on_app_quit(move |_| {
                        let core = shutdown.clone();
                        async move { core.shutdown() }
                    })
                    .detach();
                    let hub = cx.new(|cx| Hub::new(core, cx));
                    cx.open_window(options, |window, cx| {
                        let workspace = cx.new(|cx| Workspace::new(hub, window, cx));
                        cx.new(|cx| Root::new(workspace, window, cx))
                    })
                    .map(|window| window.into())
                }
                Err(failure) => cx
                    .open_window(options, |window, cx| {
                        let view = cx
                            .new(|_| StartupFailure(format!("Velune 无法启动：{failure}").into()));
                        cx.new(|cx| Root::new(view, window, cx))
                    })
                    .map(|window| window.into()),
            };
            let main: AnyWindowHandle = opened.expect("failed to open the main window");
            // Closing the main window quits; the settings window does not outlive it.
            cx.on_window_closed(move |cx, closed| {
                if closed == main.window_id() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}
