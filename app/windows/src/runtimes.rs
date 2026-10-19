//! This PC's Agent runtimes: discovery of installed ones, and manual setup.
use std::collections::HashMap;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use velune_bindings::{
    BindingError, BindingRuntimeDiscoveryCandidate, BindingRuntimeDiscoveryProbe,
    BindingRuntimeInstance, BindingRuntimeTypeDescriptor, BindingSettingField, BindingSettingKind,
};

use crate::{
    conversation::muted,
    core::{Core, Device, describe, run},
    discovery,
    settings::{row, section},
};

/// A runtime being added by hand: its type, name and the type's settings.
struct Draft {
    kind: BindingRuntimeTypeDescriptor,
    name: Entity<InputState>,
    fields: Vec<(BindingSettingField, Entity<InputState>)>,
    choices: HashMap<String, String>,
}

pub struct Runtimes {
    core: Core,
    instances: Vec<BindingRuntimeInstance>,
    kinds: Vec<BindingRuntimeTypeDescriptor>,
    candidates: Option<Vec<BindingRuntimeDiscoveryCandidate>>,
    draft: Option<Draft>,
    busy: bool,
    status: Option<String>,
    failure: Option<String>,
}

impl Runtimes {
    pub fn new(core: Core, cx: &mut Context<Self>) -> Self {
        let mut runtimes = Self {
            core,
            instances: Vec::new(),
            kinds: Vec::new(),
            candidates: None,
            draft: None,
            busy: false,
            status: None,
            failure: None,
        };
        runtimes.reload(cx);
        runtimes
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        run(
            cx,
            move || core.application(&Device::Local)?.list(),
            |this, list, _| match list {
                Ok(list) => {
                    this.instances = list.runtime_instances;
                    this.kinds = list.runtime_types;
                }
                Err(error) => this.failure = Some(describe(&error)),
            },
        );
    }

    /// Runs one write against this PC's core, then shows the runtimes it reports.
    fn write(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Core) -> Result<Vec<BindingRuntimeInstance>, BindingError> + Send + 'static,
        done: impl FnOnce(&mut Self) + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.failure = None;
        let core = self.core.clone();
        run(
            cx,
            move || work(&core),
            move |this, result, _| {
                this.busy = false;
                match result {
                    Ok(instances) => {
                        this.instances = instances;
                        done(this);
                    }
                    Err(error) => this.failure = Some(describe(&error)),
                }
            },
        );
    }

    fn upsert(
        &mut self,
        runtime: BindingRuntimeInstance,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self) + 'static,
    ) {
        self.write(
            cx,
            move |core| Ok(core.local().upsert_runtime(runtime)?.runtime_instances),
            done,
        );
    }

    fn discover(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.failure = None;
        self.candidates = None;
        self.status = Some("正在发现运行时…".into());
        let core = self.core.clone();
        run(
            cx,
            move || {
                let home = discovery::user_home().ok_or(BindingError::Invalid {
                    detail: "找不到用户目录".into(),
                })?;
                let node = discovery::node().ok_or(BindingError::Invalid {
                    detail: "没有找到 Node（node.exe），请先安装 Node 或手动添加运行时".into(),
                })?;
                let overrides = [
                    "PI_CODING_AGENT_DIR",
                    "CODEX_HOME",
                    "CLAUDE_CONFIG_DIR",
                    "DSH_HOME",
                ]
                .into_iter()
                .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
                .collect();
                let application = core.local();
                let probes = application
                    .runtime_discovery_hints(home, overrides)?
                    .into_iter()
                    .flat_map(|hint| {
                        let node = node.clone();
                        discovery::candidates(&hint.family_id, &hint.command)
                            .into_iter()
                            .map(move |binary| BindingRuntimeDiscoveryProbe {
                                family_id: hint.family_id.clone(),
                                binary,
                                node_binary: node.clone(),
                                agent_directory: hint.agent_directory.clone(),
                            })
                    })
                    .collect();
                application.discover_runtimes(probes)
            },
            |this, result, _| {
                this.busy = false;
                match result {
                    Ok(candidates) => {
                        this.status = Some(if candidates.is_empty() {
                            "未发现运行时，可以手动添加。".into()
                        } else {
                            format!("发现 {} 个运行时。", candidates.len())
                        });
                        this.candidates = Some(candidates);
                    }
                    Err(error) => {
                        this.status = None;
                        this.failure = Some(describe(&error));
                    }
                }
            },
        );
    }

    fn import(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(candidate) = self.candidates.as_ref().and_then(|c| c.get(index)) else {
            return;
        };
        let runtime = candidate.runtime.clone();
        self.upsert(runtime, cx, move |this| {
            if let Some(candidate) = this.candidates.as_mut().and_then(|c| c.get_mut(index)) {
                candidate.already_configured = true;
            }
        });
    }

    fn choose_kind(
        &mut self,
        kind: BindingRuntimeTypeDescriptor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = kind.name.clone();
        let fields = kind
            .fields
            .iter()
            .filter(|field| !matches!(field.kind, BindingSettingKind::Choice))
            .map(|field| {
                let value = field.value.clone();
                let placeholder = field.help.clone().unwrap_or_default();
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(value)
                });
                (field.clone(), input)
            })
            .collect();
        let choices = kind
            .fields
            .iter()
            .filter(|field| matches!(field.kind, BindingSettingKind::Choice))
            .map(|field| (field.key.clone(), field.value.clone()))
            .collect();
        self.draft = Some(Draft {
            name: cx.new(|cx| InputState::new(window, cx).default_value(name)),
            kind,
            fields,
            choices,
        });
        cx.notify();
    }

    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        let name = draft.name.read(cx).value().trim().to_owned();
        let mut settings: HashMap<String, String> = draft.choices.clone();
        for (field, input) in &draft.fields {
            let value = input.read(cx).value().trim().to_owned();
            if field.required && value.is_empty() {
                self.failure = Some(format!("请填写“{}”", field.label));
                cx.notify();
                return;
            }
            if !value.is_empty() {
                settings.insert(field.key.clone(), value);
            }
        }
        let runtime = BindingRuntimeInstance {
            enabled: true,
            id: uuid::Uuid::new_v4().to_string(),
            name: if name.is_empty() {
                draft.kind.name.clone()
            } else {
                name
            },
            type_id: draft.kind.id.clone(),
            gateway_id: "default".into(),
            settings,
        };
        self.upsert(runtime, cx, |this| this.draft = None);
    }

    fn browse(
        &mut self,
        input: Entity<InputState>,
        directories: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: None,
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

    fn render_instances(&self, cx: &mut Context<Self>) -> impl IntoElement {
        section("已配置的运行时", cx)
            .when(self.instances.is_empty(), |column| {
                column.child(muted("还没有运行时。", cx))
            })
            .children(self.instances.iter().map(|runtime| {
                let kind = self
                    .kinds
                    .iter()
                    .find(|kind| kind.id == runtime.type_id)
                    .map(|kind| kind.name.clone())
                    .unwrap_or_else(|| runtime.type_id.clone());
                let binary = runtime.settings.get("binary").cloned().unwrap_or_default();
                let toggled = runtime.clone();
                let id = runtime.id.clone();
                row(cx)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(runtime.name.clone())
                            .child(muted(kind, cx))
                            .when(!binary.is_empty(), |column| {
                                column.child(div().truncate().child(muted(binary, cx)))
                            }),
                    )
                    .child(
                        Switch::new(SharedString::from(format!("enabled-{}", runtime.id)))
                            .checked(runtime.enabled)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let mut runtime = toggled.clone();
                                runtime.enabled = *checked;
                                this.upsert(runtime, cx, |_| {});
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{}", runtime.id)))
                            .small()
                            .label("删除")
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let id = id.clone();
                                this.write(
                                    cx,
                                    move |core| {
                                        Ok(core.local().delete_runtime(id)?.runtime_instances)
                                    },
                                    |_| {},
                                );
                            })),
                    )
            }))
    }

    fn render_discovery(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let candidates = self.candidates.clone().unwrap_or_default();
        section("发现已安装的运行时", cx)
            .child(muted(
                "在 PATH 与 npm／pnpm 全局目录中查找 Pi、Codex 与 Claude Code，只读取公开版本，不导入登录信息。",
                cx,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("discover")
                            .label("发现")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.discover(cx))),
                    )
                    .when_some(self.status.clone(), |row, status| row.child(muted(status, cx))),
            )
            .children(candidates.into_iter().enumerate().map(|(index, candidate)| {
                let directory = candidate.runtime.settings.get("agentDir").cloned().unwrap_or_default();
                let binary = candidate.runtime.settings.get("binary").cloned().unwrap_or_default();
                let title = match &candidate.version {
                    Some(version) => format!("{} {version}", candidate.runtime.name),
                    None => candidate.runtime.name.clone(),
                };
                row(cx)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(title)
                            .child(div().truncate().child(muted(binary, cx)))
                            .child(div().truncate().child(muted(directory, cx)))
                            .when(!candidate.supported, |column| {
                                column.child(muted(candidate.detail.clone(), cx))
                            }),
                    )
                    .child(if candidate.already_configured {
                        muted("已配置", cx)
                    } else {
                        Button::new(("import", index))
                            .small()
                            .label("添加")
                            .disabled(self.busy || !candidate.supported)
                            .on_click(cx.listener(move |this, _, _, cx| this.import(index, cx)))
                            .into_any_element()
                    })
            }))
    }

    fn render_manual(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        // Launch arguments and environment of ACP runtimes are not edited here.
        let kinds: Vec<_> = self
            .kinds
            .iter()
            .filter(|kind| !kind.launch_options)
            .cloned()
            .collect();
        let column = section("手动添加", cx).child(
            h_flex().child(
                Button::new("kind")
                    .outline()
                    .label(
                        self.draft
                            .as_ref()
                            .map(|draft| draft.kind.name.clone())
                            .unwrap_or_else(|| "选择运行时类型".into()),
                    )
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for kind in &kinds {
                            let view = view.clone();
                            let chosen = kind.clone();
                            menu = menu.item(PopupMenuItem::new(kind.name.clone()).on_click(
                                move |_, window, cx| {
                                    let chosen = chosen.clone();
                                    let _ = view.update(cx, |this, cx| {
                                        this.choose_kind(chosen, window, cx)
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            ),
        );
        let Some(draft) = &self.draft else {
            return column;
        };
        let label = |text: String| div().w(px(140.)).flex_none().child(text);
        let mut column = column.child(
            h_flex()
                .gap_2()
                .child(label("名称".into()))
                .child(div().flex_1().child(Input::new(&draft.name))),
        );
        for (index, (field, input)) in draft.fields.iter().enumerate() {
            let directories = matches!(field.kind, BindingSettingKind::DirectoryPath);
            let browsing = input.clone();
            column = column.child(
                h_flex()
                    .gap_2()
                    .child(label(if field.required {
                        format!("{} *", field.label)
                    } else {
                        field.label.clone()
                    }))
                    .child(div().flex_1().child(Input::new(input)))
                    .when(!matches!(field.kind, BindingSettingKind::Text), |row| {
                        row.child(Button::new(("browse", index)).label("浏览…").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.browse(browsing.clone(), directories, window, cx)
                            }),
                        ))
                    }),
            );
        }
        for field in draft
            .kind
            .fields
            .iter()
            .filter(|field| matches!(field.kind, BindingSettingKind::Choice))
        {
            let current = draft.choices.get(&field.key).cloned().unwrap_or_default();
            let current_label = field
                .options
                .iter()
                .find(|option| option.id == current)
                .map(|option| option.label.clone())
                .unwrap_or_else(|| "选择".into());
            let view = cx.entity().downgrade();
            let key = field.key.clone();
            let options = field.options.clone();
            column = column.child(
                h_flex().gap_2().child(label(field.label.clone())).child(
                    Button::new(SharedString::from(format!("choice-{key}")))
                        .outline()
                        .label(current_label)
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for option in &options {
                                let view = view.clone();
                                let key = key.clone();
                                let id = option.id.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(option.label.clone())
                                        .checked(option.id == current)
                                        .on_click(move |_, _, cx| {
                                            let (key, id) = (key.clone(), id.clone());
                                            let _ = view.update(cx, |this, cx| {
                                                if let Some(draft) = &mut this.draft {
                                                    draft.choices.insert(key, id);
                                                }
                                                cx.notify();
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                ),
            );
        }
        column.child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("draft-cancel")
                        .label("取消")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft = None;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("draft-save")
                        .primary()
                        .label("添加")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.save_draft(cx))),
                ),
        )
    }
}

impl Render for Runtimes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .pb_4()
            .when_some(self.failure.clone(), |column, failure| {
                column.child(
                    div()
                        .px_6()
                        .pt_4()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(failure),
                )
            })
            .child(self.render_instances(cx))
            .child(self.render_discovery(cx))
            .child(self.render_manual(cx))
    }
}
