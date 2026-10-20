package main

// This PC's gateway providers. HTTP templates retain their composition while
// API keys stay in the application's private configuration.

import (
	"fmt"
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

const defaultGateway = "default"

// providerDraft is the provider being added; template is "" for a custom API.
type providerDraft struct {
	template                    string
	name, endpoint, key, models string
	protocol                    *bindings.BindingGatewayProtocol
}

type providersPane struct {
	core      *core
	win       *mygo.Window
	gateway   *bindings.BindingGatewayConfig
	protocols []bindings.BindingProtocolDescriptor
	templates []bindings.BindingProviderTemplate
	draft     providerDraft
	busy      bool
	failure   string
}

type providerListing struct {
	list      bindings.BindingConfigurationSnapshot
	templates []bindings.BindingProviderTemplate
}

func (p *providersPane) reload() {
	run(p.win, func() outcome[providerListing] {
		var r outcome[providerListing]
		r.value.list, r.err = p.core.local.List()
		if r.err == nil {
			r.value.templates, r.err = bindings.ProviderTemplates()
		}
		return r
	}, func(r outcome[providerListing]) {
		if r.err != nil {
			p.failure = describe(r.err)
			return
		}
		p.templates = nil
		for _, template := range r.value.templates {
			_, http := template.ExecutionBackend.(bindings.BindingExecutionBackendHttp)
			if http && template.AuthenticationSource == bindings.BindingProviderAuthenticationSourceApiKey {
				p.templates = append(p.templates, template)
			}
		}
		p.setGateway(r.value.list.Gateways)
		p.protocols = nil
		for _, protocol := range r.value.list.Protocols {
			if protocol.Supported {
				p.protocols = append(p.protocols, protocol)
			}
		}
		if p.draft.protocol == nil {
			p.draft.protocol = p.firstProtocol()
		}
	})
}

func (p *providersPane) setGateway(gateways []bindings.BindingGatewayConfig) {
	p.gateway = nil
	for _, gateway := range gateways {
		if gateway.Id == defaultGateway {
			p.gateway = &gateway
		}
	}
}

func (p *providersPane) firstProtocol() *bindings.BindingGatewayProtocol {
	if len(p.protocols) == 0 {
		return nil
	}
	protocol := p.protocols[0].Id
	return &protocol
}

func (p *providersPane) selectedTemplate() *bindings.BindingProviderTemplate {
	for i := range p.templates {
		if p.templates[i].Id == p.draft.template {
			return &p.templates[i]
		}
	}
	return nil
}

func (p *providersPane) chooseTemplate(id string) {
	p.draft = providerDraft{template: id, models: p.draft.models}
	if template := p.selectedTemplate(); template != nil {
		protocol := template.Protocol
		p.draft.protocol = &protocol
		p.draft.name, p.draft.endpoint = template.Name, template.Endpoint
	} else {
		p.draft.protocol = p.firstProtocol()
	}
}

func (p *providersPane) protocolName(protocol bindings.BindingGatewayProtocol) string {
	for _, descriptor := range p.protocols {
		if descriptor.Id == protocol {
			return descriptor.Name
		}
	}
	return fmt.Sprintf("协议 %d", protocol)
}

func (p *providersPane) save() {
	if p.busy || p.draft.protocol == nil {
		return
	}
	name, endpoint, key := strings.TrimSpace(p.draft.name), strings.TrimSpace(p.draft.endpoint), strings.TrimSpace(p.draft.key)
	models := strings.FieldsFunc(p.draft.models, func(r rune) bool { return r == ',' || r == '，' || r == ' ' })
	if name == "" || endpoint == "" || key == "" || len(models) == 0 {
		p.failure = "请填写名称、接口地址、API key 与至少一个模型 ID"
		return
	}
	draft := bindings.BindingProviderDraft{
		Name:                 name,
		Protocol:             *p.draft.protocol,
		Endpoint:             endpoint,
		ExecutionBackend:     bindings.BindingExecutionBackendHttp{},
		RequestProfile:       bindings.BindingRequestProfileNative,
		AuthenticationSource: bindings.BindingProviderAuthenticationSourceApiKey,
		QuotaSource:          bindings.BindingProviderQuotaSourceNone,
	}
	if template := p.selectedTemplate(); template != nil {
		draft.ExecutionBackend, draft.RequestProfile = template.ExecutionBackend, template.RequestProfile
		draft.AuthenticationSource, draft.QuotaSource = template.AuthenticationSource, template.QuotaSource
	}
	for _, model := range models {
		draft.Models = append(draft.Models, bindings.BindingProviderModel{ProviderModelId: model, Nickname: model})
	}
	p.busy, p.failure = true, ""
	run(p.win, func() outcome[bindings.BindingGatewayUpdate] {
		update, err := p.core.local.SaveProvider(defaultGateway, draft, bindings.BindingAuthenticationEditSetApiKey{Value: key})
		return outcome[bindings.BindingGatewayUpdate]{update, err}
	}, func(r outcome[bindings.BindingGatewayUpdate]) {
		p.busy = false
		if r.err != nil {
			p.failure = describe(r.err)
			return
		}
		p.setGateway(r.value.Gateways)
		p.draft.models = ""
		p.chooseTemplate(p.draft.template)
	})
}

func (p *providersPane) delete(id string) {
	if p.busy {
		return
	}
	p.busy, p.failure = true, ""
	run(p.win, func() outcome[bindings.BindingGatewayUpdate] {
		update, err := p.core.local.DeleteProvider(defaultGateway, id)
		return outcome[bindings.BindingGatewayUpdate]{update, err}
	}, func(r outcome[bindings.BindingGatewayUpdate]) {
		p.busy = false
		if r.err != nil {
			p.failure = describe(r.err)
			return
		}
		p.setGateway(r.value.Gateways)
	})
}

func (p *providersPane) view(c *ui.Context) {
	failureText(c, p.failure)
	p.providersView(c)
	p.draftView(c)
}

func (p *providersPane) providersView(c *ui.Context) {
	var providers []bindings.BindingProviderDefinition
	if p.gateway != nil {
		providers = p.gateway.Providers
	}
	section(c, "提供商", func() {
		if len(providers) == 0 {
			muted(c, "还没有提供商。")
		}
		for _, provider := range providers {
			var models []string
			for _, model := range provider.Models {
				models = append(models, model.ProviderModelId)
			}
			ui.Column(c.Key(provider.Id)).Children(func() {
				settingsRow(c, func() {
					ui.Column(c).Grow(1).MinWidth(0).Gap(2).Children(func() {
						ui.Text(c, provider.Name)
						muted(c, p.protocolName(provider.Protocol)+" · "+provider.Endpoint).SingleLine()
						muted(c, strings.Join(models, "、")).SingleLine()
					})
					if ui.Button(c, "删除").Disabled(p.busy).Clicked() {
						p.delete(provider.Id)
					}
				})
			})
		}
	})
}

func (p *providersPane) draftView(c *ui.Context) {
	selected := p.selectedTemplate()
	fixed := selected != nil && selected.Kind == bindings.BindingProviderTemplateKindSubscription
	section(c, "添加提供商", func() {
		labeled(c, "模板", 96, func() {
			label := "自定义 API"
			if selected != nil {
				label = selected.Name
			}
			ui.MenuButton(c, label, func(m *ui.Menu) {
				if m.Item("自定义 API").Checked(selected == nil).Chosen() {
					p.chooseTemplate("")
				}
				for _, template := range p.templates {
					if m.Item(template.Name).Checked(template.Id == p.draft.template).Chosen() {
						p.chooseTemplate(template.Id)
					}
				}
			})
		})
		if selected != nil && selected.Disclosure != nil {
			muted(c, selected.Disclosure.Title+"："+selected.Disclosure.Detail)
		}
		labeled(c, "名称", 96, func() {
			ui.TextInput(c.Key("name"), &p.draft.name).Placeholder("名称，例如 DeepSeek").Grow(1)
		})
		labeled(c, "协议", 96, func() {
			label := "选择协议"
			if p.draft.protocol != nil {
				label = p.protocolName(*p.draft.protocol)
			}
			ui.MenuButton(c, label, func(m *ui.Menu) {
				for _, protocol := range p.protocols {
					if m.Item(protocol.Name).Checked(p.draft.protocol != nil && *p.draft.protocol == protocol.Id).Chosen() {
						id := protocol.Id
						p.draft.protocol = &id
					}
				}
			}).Disabled(fixed)
		})
		labeled(c, "接口地址", 96, func() {
			if fixed {
				muted(c, selected.Endpoint).Grow(1)
				return
			}
			ui.TextInput(c.Key("endpoint"), &p.draft.endpoint).Placeholder("接口地址，例如 https://api.example.com/v1").Grow(1)
		})
		labeled(c, "API key", 96, func() {
			ui.TextInput(c.Key("key"), &p.draft.key).Placeholder("API key").Password().Grow(1)
		})
		labeled(c, "模型", 96, func() {
			ui.TextInput(c.Key("models"), &p.draft.models).Placeholder("模型 ID，多个以逗号分隔").Grow(1)
		})
		ui.Row(c).Children(func() {
			ui.Spacer(c)
			if ui.Button(c, "添加").Disabled(p.busy).Clicked() {
				p.save()
			}
		})
	})
}
