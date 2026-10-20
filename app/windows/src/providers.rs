//! This PC's gateway providers. HTTP templates retain their composition while
//! API keys stay in the application's private configuration.
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
    BindingAuthenticationEdit, BindingExecutionBackend, BindingGatewayConfig,
    BindingGatewayProtocol, BindingProtocolDescriptor, BindingProviderAuthenticationSource,
    BindingProviderDraft, BindingProviderModel, BindingProviderQuotaSource,
    BindingProviderTemplate, BindingProviderTemplateKind, BindingQuotaAuthenticationEdit,
    BindingRequestProfile,
};

use crate::{
    conversation::muted,
    core::{Core, Device, describe, run},
    settings::{row, section},
};

const GATEWAY: &str = "default";

struct Draft {
    template: Option<String>,
    name: Entity<InputState>,
    protocol: Option<BindingGatewayProtocol>,
    endpoint: Entity<InputState>,
    key: Entity<InputState>,
    models: Entity<InputState>,
    quota_ak: Entity<InputState>,
    quota_sk: Entity<InputState>,
    quota_confirmed: bool,
}

pub struct Providers {
    core: Core,
    gateway: Option<BindingGatewayConfig>,
    protocols: Vec<BindingProtocolDescriptor>,
    templates: Vec<BindingProviderTemplate>,
    draft: Draft,
    busy: bool,
    failure: Option<String>,
}

impl Providers {
    pub fn new(core: Core, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str,
                     masked: bool,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(masked)
            })
        };
        let draft = Draft {
            template: None,
            name: input("名称，例如 DeepSeek", false, window, cx),
            protocol: None,
            endpoint: input(
                "接口地址，例如 https://api.example.com/v1",
                false,
                window,
                cx,
            ),
            key: input("API key", true, window, cx),
            models: input("模型 ID，多个以逗号分隔", false, window, cx),
            quota_ak: input("额度查询 AccessKey ID", false, window, cx),
            quota_sk: input("额度查询 SecretAccessKey", true, window, cx),
            quota_confirmed: false,
        };
        let mut providers = Self {
            core,
            gateway: None,
            protocols: Vec::new(),
            templates: Vec::new(),
            draft,
            busy: false,
            failure: None,
        };
        providers.reload(cx);
        providers
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        run(
            cx,
            move || {
                Ok((
                    core.application(&Device::Local)?.list()?,
                    velune_bindings::provider_templates()?,
                ))
            },
            |this, list, _| match list {
                Ok((list, templates)) => {
                    this.templates = templates
                        .into_iter()
                        .filter(|template| {
                            matches!(template.execution_backend, BindingExecutionBackend::Http)
                                && matches!(
                                    template.authentication_source,
                                    BindingProviderAuthenticationSource::ApiKey
                                )
                        })
                        .collect();
                    this.gateway = list
                        .gateways
                        .into_iter()
                        .find(|gateway| gateway.id == GATEWAY);
                    this.protocols = list
                        .protocols
                        .into_iter()
                        .filter(|protocol| protocol.supported)
                        .collect();
                    if this.draft.protocol.is_none() {
                        this.draft.protocol =
                            this.protocols.first().map(|protocol| protocol.id.clone());
                    }
                }
                Err(error) => this.failure = Some(describe(&error)),
            },
        );
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let value = |input: &Entity<InputState>, cx: &App| input.read(cx).value().trim().to_owned();
        let name = value(&self.draft.name, cx);
        let endpoint = value(&self.draft.endpoint, cx);
        let key = value(&self.draft.key, cx);
        let models: Vec<String> = value(&self.draft.models, cx)
            .split([',', '，', ' '])
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_owned)
            .collect();
        let Some(protocol) = self.draft.protocol.clone() else {
            return;
        };
        if name.is_empty() || endpoint.is_empty() || key.is_empty() || models.is_empty() {
            self.failure = Some("请填写名称、接口地址、API key 与至少一个模型 ID".into());
            cx.notify();
            return;
        }
        let template = self.selected_template();
        let draft = BindingProviderDraft {
            quota_authentication_edit: if template.is_some_and(|t| {
                matches!(t.quota_source, BindingProviderQuotaSource::ArkCodingUsage)
            }) {
                let ak = self.draft.quota_ak.read(cx).value().to_string();
                let sk = self.draft.quota_sk.read(cx).value().to_string();
                if ak.is_empty() && sk.is_empty() {
                    None
                } else if ak.is_empty() || sk.is_empty() {
                    self.failure = Some("额度查询需要完整 AK/SK".into());
                    cx.notify();
                    return;
                } else {
                    Some(BindingQuotaAuthenticationEdit::SetAccessKey {
                        access_key_id: ak,
                        secret_access_key: sk,
                        confirmed: self.draft.quota_confirmed,
                    })
                }
            } else {
                None
            },
            id: String::new(),
            name,
            protocol,
            endpoint,
            execution_backend: template
                .map(|value| value.execution_backend.clone())
                .unwrap_or(BindingExecutionBackend::Http),
            request_profile: template
                .map(|value| value.request_profile)
                .unwrap_or(BindingRequestProfile::Native),
            authentication_source: template
                .map(|value| value.authentication_source)
                .unwrap_or(BindingProviderAuthenticationSource::ApiKey),
            quota_source: template
                .map(|value| value.quota_source)
                .unwrap_or(BindingProviderQuotaSource::None),
            models: models
                .into_iter()
                .map(|model| BindingProviderModel {
                    record_key: String::new(),
                    provider_model_id: model.clone(),
                    nickname: model,
                    icon: None,
                    context_window: None,
                    max_output_tokens: None,
                    reasoning_levels: None,
                })
                .collect(),
            quota_limits: Vec::new(),
        };
        self.busy = true;
        self.failure = None;
        let core = self.core.clone();
        let inputs = [
            self.draft.name.clone(),
            self.draft.endpoint.clone(),
            self.draft.key.clone(),
            self.draft.models.clone(),
            self.draft.quota_ak.clone(),
            self.draft.quota_sk.clone(),
        ];
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    core.local().save_provider(
                        GATEWAY.into(),
                        draft,
                        BindingAuthenticationEdit::SetApiKey { value: key },
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(update) => {
                        this.gateway = update
                            .gateways
                            .into_iter()
                            .find(|gateway| gateway.id == GATEWAY);
                        for input in &inputs {
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        if this.draft.template.is_some() {
                            this.choose_template(this.draft.template.clone(), window, cx);
                        }
                    }
                    Err(error) => this.failure = Some(describe(&error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let core = self.core.clone();
        run(
            cx,
            move || core.local().delete_provider(GATEWAY.into(), id),
            |this, result, _| {
                this.busy = false;
                match result {
                    Ok(update) => {
                        this.gateway = update
                            .gateways
                            .into_iter()
                            .find(|gateway| gateway.id == GATEWAY)
                    }
                    Err(error) => this.failure = Some(describe(&error)),
                }
            },
        );
    }

    fn selected_template(&self) -> Option<&BindingProviderTemplate> {
        self.templates
            .iter()
            .find(|template| Some(&template.id) == self.draft.template.as_ref())
    }
    fn choose_template(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.template = id;
        self.draft.quota_confirmed = false;
        for field in [&self.draft.quota_ak, &self.draft.quota_sk] {
            field.update(cx, |input, cx| input.set_value("", window, cx));
        }
        let template = self.selected_template().cloned();
        self.draft.protocol = template
            .as_ref()
            .map(|value| value.protocol.clone())
            .or_else(|| self.protocols.first().map(|value| value.id.clone()));
        self.draft.name.update(cx, |input, cx| {
            input.set_value(
                template
                    .as_ref()
                    .map(|value| value.name.clone())
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        self.draft.endpoint.update(cx, |input, cx| {
            input.set_value(
                template.map(|value| value.endpoint).unwrap_or_default(),
                window,
                cx,
            )
        });
        self.draft
            .key
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }
    fn protocol_name(&self, protocol: &BindingGatewayProtocol) -> String {
        self.protocols
            .iter()
            .find(|descriptor| same(&descriptor.id, protocol))
            .map(|descriptor| descriptor.name.clone())
            .unwrap_or_else(|| format!("{protocol:?}"))
    }
}

impl Render for Providers {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let providers = self
            .gateway
            .as_ref()
            .map(|gateway| gateway.providers.clone())
            .unwrap_or_default();
        let view = cx.entity().downgrade();
        let protocols = self.protocols.clone();
        let current = self.draft.protocol.clone();
        let templates = self.templates.clone();
        let selected = self.selected_template().cloned();
        let fixed = selected
            .as_ref()
            .is_some_and(|value| matches!(value.kind, BindingProviderTemplateKind::Subscription));
        let template_view = cx.entity().downgrade();
        let selected_id = self.draft.template.clone();
        let label = |text: &'static str| div().w(px(96.)).flex_none().child(text);
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
            .child(
                section("提供商", cx)
                    .when(providers.is_empty(), |column| {
                        column.child(muted("还没有提供商。", cx))
                    })
                    .children(providers.into_iter().map(|provider| {
                        let models = provider
                            .models
                            .iter()
                            .map(|model| model.provider_model_id.clone())
                            .collect::<Vec<_>>()
                            .join("、");
                        let id = provider.id.clone();
                        row(cx)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(provider.name.clone())
                                    .child(muted(
                                        format!(
                                            "{} · {}",
                                            self.protocol_name(&provider.protocol),
                                            provider.endpoint
                                        ),
                                        cx,
                                    ))
                                    .child(div().truncate().child(muted(models, cx))),
                            )
                            .child(
                                Button::new(SharedString::from(format!("delete-{id}")))
                                    .small()
                                    .label("删除")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.delete(id.clone(), cx)
                                    })),
                            )
                    })),
            )
            .child(
                section("添加提供商", cx)
                    .child(
                        h_flex().gap_2().child(label("模板")).child(
                            Button::new("provider-template")
                                .outline()
                                .label(
                                    selected
                                        .as_ref()
                                        .map(|value| value.name.clone())
                                        .unwrap_or_else(|| "自定义 API".into()),
                                )
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    let view = template_view.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new("自定义 API")
                                            .checked(selected_id.is_none())
                                            .on_click(move |_, window, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.choose_template(None, window, cx)
                                                });
                                            }),
                                    );
                                    for template in &templates {
                                        let view = template_view.clone();
                                        let id = template.id.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(template.name.clone())
                                                .checked(selected_id.as_ref() == Some(&id))
                                                .on_click(move |_, window, cx| {
                                                    let _ = view.update(cx, |this, cx| {
                                                        this.choose_template(
                                                            Some(id.clone()),
                                                            window,
                                                            cx,
                                                        )
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                }),
                        ),
                    )
                    .when_some(
                        selected
                            .as_ref()
                            .and_then(|value| value.disclosure.as_ref())
                            .map(|value| format!("{}：{}", value.title, value.detail)),
                        |column, note| column.child(muted(note, cx)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(label("名称"))
                            .child(div().flex_1().child(Input::new(&self.draft.name))),
                    )
                    .child(
                        h_flex().gap_2().child(label("协议")).child(
                            Button::new("protocol")
                                .outline()
                                .disabled(fixed)
                                .label(
                                    current
                                        .as_ref()
                                        .map(|protocol| self.protocol_name(protocol))
                                        .unwrap_or_else(|| "选择协议".into()),
                                )
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for protocol in &protocols {
                                        let view = view.clone();
                                        let id = protocol.id.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(protocol.name.clone())
                                                .checked(current.as_ref().is_some_and(|current| {
                                                    same(current, &protocol.id)
                                                }))
                                                .on_click(move |_, _, cx| {
                                                    let id = id.clone();
                                                    let _ = view.update(cx, |this, cx| {
                                                        this.draft.protocol = Some(id);
                                                        cx.notify();
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                }),
                        ),
                    )
                    .child(
                        h_flex().gap_2().child(label("接口地址")).child(
                            div()
                                .flex_1()
                                .when(fixed, |view| {
                                    view.child(muted(
                                        selected
                                            .as_ref()
                                            .map(|value| value.endpoint.clone())
                                            .unwrap_or_default(),
                                        cx,
                                    ))
                                })
                                .when(!fixed, |view| view.child(Input::new(&self.draft.endpoint))),
                        ),
                    )
                    .child(
                        h_flex().gap_2().child(label("API key")).child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.draft.key).mask_toggle()),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(label("模型"))
                            .child(div().flex_1().child(Input::new(&self.draft.models))),
                    )
                    .when(
                        selected.as_ref().is_some_and(|t| {
                            matches!(t.quota_source, BindingProviderQuotaSource::ArkCodingUsage)
                        }),
                        |column| {
                            column
                                .child(muted("额度来源由用户关联，账户归属未自动核验。", cx))
                                .child(Input::new(&self.draft.quota_ak))
                                .child(Input::new(&self.draft.quota_sk).mask_toggle())
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child("确认此来源对应当前模型 Key 的个人 Coding Plan")
                                        .child(
                                            Switch::new("quota-confirmed")
                                                .checked(self.draft.quota_confirmed)
                                                .on_click(cx.listener(
                                                    |this, checked: &bool, _, cx| {
                                                        this.draft.quota_confirmed = *checked;
                                                        cx.notify();
                                                    },
                                                )),
                                        ),
                                )
                        },
                    )
                    .child(
                        h_flex().justify_end().child(
                            Button::new("provider-save")
                                .primary()
                                .label("添加")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                        ),
                    ),
            )
    }
}

fn same(a: &BindingGatewayProtocol, b: &BindingGatewayProtocol) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}
