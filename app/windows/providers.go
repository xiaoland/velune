package main

// This PC's gateway providers. HTTP templates retain their composition while
// API keys stay in the application's private configuration.

import (
	"fmt"
	"slices"
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

const defaultGateway = "default"

// providerEditor adds a provider or edits one; template is "" for a custom
// API, and only chosen when adding.
type providerEditor struct {
	open bool
	// existing is the provider edited; nil adds one.
	existing                    *bindings.BindingProviderDefinition
	template                    string
	name, endpoint, key, models string
	protocol                    *bindings.BindingGatewayProtocol
	// shown is whether the key field shows its text, as once the saved key is read.
	shown   bool
	checked bool
	// The ARK Coding Plan's usage query credentials, apart from the model
	// key: a full pair replaces them, none keeps them.
	quotaID, quotaSecret          string
	quotaConfirmed, clearingQuota bool
}

type providersPane struct {
	core      *core
	win       *mygo.Window
	gateway   *bindings.BindingGatewayConfig
	protocols []bindings.BindingProtocolDescriptor
	templates []bindings.BindingProviderTemplate
	selected  int
	list      ui.ListState
	editor    providerEditor
	deleting  bool
	busy      bool
	failure   string
}

func newProvidersPane(core *core) *providersPane {
	p := &providersPane{core: core, selected: -1}
	p.list.Key = func(i int) any { return p.providers()[i].Id }
	p.list.Label = func(i int) string { return p.providers()[i].Name }
	p.list.Selected = &p.selected
	return p
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
	})
}

func (p *providersPane) setGateway(gateways []bindings.BindingGatewayConfig) {
	p.gateway = nil
	for _, gateway := range gateways {
		if gateway.Id == defaultGateway {
			p.gateway = &gateway
		}
	}
	if n := len(p.providers()); p.selected >= n {
		p.selected = n - 1
	}
}

func (p *providersPane) providers() []bindings.BindingProviderDefinition {
	if p.gateway == nil {
		return nil
	}
	return p.gateway.Providers
}

func (p *providersPane) chosen() *bindings.BindingProviderDefinition {
	providers := p.providers()
	if p.selected < 0 || p.selected >= len(providers) {
		return nil
	}
	return &providers[p.selected]
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
		if p.templates[i].Id == p.editor.template {
			return &p.templates[i]
		}
	}
	return nil
}

// pageFailure is what failed, shown on the page unless the editor shows it.
func (p *providersPane) pageFailure() string {
	if p.editor.open {
		return ""
	}
	return p.failure
}

func (p *providersPane) add() {
	p.editor, p.failure = providerEditor{open: true}, ""
	p.chooseTemplate("")
}

func (p *providersPane) edit(provider bindings.BindingProviderDefinition) {
	var models []string
	for _, model := range provider.Models {
		models = append(models, model.ProviderModelId)
	}
	protocol := provider.Protocol
	p.failure = ""
	p.editor = providerEditor{open: true, existing: &provider, name: provider.Name, endpoint: provider.Endpoint, models: strings.Join(models, ", "), protocol: &protocol}
}

func (p *providersPane) chooseTemplate(id string) {
	e := &p.editor
	e.template = id
	e.quotaID, e.quotaSecret, e.quotaConfirmed, e.clearingQuota = "", "", false, false
	if template := p.selectedTemplate(); template != nil {
		protocol := template.Protocol
		e.protocol = &protocol
		e.name, e.endpoint = template.Name, template.Endpoint
	} else {
		e.protocol, e.name, e.endpoint = p.firstProtocol(), "", ""
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

// apiKey tells whether the provider authenticates with an API key kept here.
func (e *providerEditor) apiKey() bool {
	return e.existing == nil || e.existing.AuthenticationSource == bindings.BindingProviderAuthenticationSourceApiKey
}

// keySaved tells whether an edited provider has a key that an empty field keeps.
func (e *providerEditor) keySaved() bool {
	return e.existing != nil && e.existing.Authentication.Configured
}

// quotaSource is where the provider's usage is read: the edited one's, or
// the chosen template's.
func (p *providersPane) quotaSource() bindings.BindingProviderQuotaSource {
	if e := p.editor; e.existing != nil {
		return e.existing.QuotaSource
	}
	if template := p.selectedTemplate(); template != nil {
		return template.QuotaSource
	}
	return bindings.BindingProviderQuotaSourceNone
}

// quotaEdit is what saving does to the usage query credentials; nil keeps them.
func (e *providerEditor) quotaEdit() bindings.BindingQuotaAuthenticationEdit {
	id, secret := strings.TrimSpace(e.quotaID), strings.TrimSpace(e.quotaSecret)
	switch {
	case e.clearingQuota:
		return bindings.BindingQuotaAuthenticationEditClear{}
	case id != "" && secret != "":
		return bindings.BindingQuotaAuthenticationEditSetAccessKey{AccessKeyId: id, SecretAccessKey: secret, Confirmed: e.quotaConfirmed}
	case e.quotaConfirmed:
		return bindings.BindingQuotaAuthenticationEditConfirmBinding{}
	}
	return nil
}

// quotaPairIncomplete tells whether only one of the access key's two parts is filled.
func (e *providerEditor) quotaPairIncomplete() bool {
	return (strings.TrimSpace(e.quotaID) == "") != (strings.TrimSpace(e.quotaSecret) == "")
}

func (e *providerEditor) modelIDs() []string {
	return strings.FieldsFunc(e.models, func(r rune) bool { return r == ',' || r == '，' || r == ' ' })
}

// revealKey reads the saved key into the field to show and edit it.
func (p *providersPane) revealKey() {
	id := p.editor.existing.Id
	run(p.win, func() outcome[string] {
		key, err := p.core.local.ReadProviderApiKey(defaultGateway, id)
		return outcome[string]{key, err}
	}, func(r outcome[string]) {
		if r.err != nil {
			p.failure = describe(r.err)
			return
		}
		if p.editor.existing != nil && p.editor.existing.Id == id {
			p.editor.key, p.editor.shown = r.value, true
		}
	})
}

func (p *providersPane) save() {
	e := &p.editor
	e.checked = true
	name, endpoint, key := strings.TrimSpace(e.name), strings.TrimSpace(e.endpoint), strings.TrimSpace(e.key)
	ids := e.modelIDs()
	if p.busy || e.protocol == nil || name == "" || endpoint == "" || len(ids) == 0 || (e.apiKey() && key == "" && !e.keySaved()) || e.quotaPairIncomplete() {
		return
	}
	draft := bindings.BindingProviderDraft{
		Name:                 name,
		Protocol:             *e.protocol,
		Endpoint:             endpoint,
		ExecutionBackend:     bindings.BindingExecutionBackendHttp{},
		RequestProfile:       bindings.BindingRequestProfileNative,
		AuthenticationSource: bindings.BindingProviderAuthenticationSourceApiKey,
		QuotaSource:          bindings.BindingProviderQuotaSourceNone,
	}
	var kept []bindings.BindingProviderModel
	if existing := e.existing; existing != nil {
		draft.Id, draft.QuotaLimits = existing.Id, existing.QuotaLimits
		draft.ExecutionBackend, draft.RequestProfile = existing.ExecutionBackend, existing.RequestProfile
		draft.AuthenticationSource, draft.QuotaSource = existing.AuthenticationSource, existing.QuotaSource
		kept = existing.Models
	} else if template := p.selectedTemplate(); template != nil {
		draft.ExecutionBackend, draft.RequestProfile = template.ExecutionBackend, template.RequestProfile
		draft.AuthenticationSource, draft.QuotaSource = template.AuthenticationSource, template.QuotaSource
	}
	// A model kept by its ID keeps its record, which routes and choices name.
	for _, id := range ids {
		index := slices.IndexFunc(kept, func(m bindings.BindingProviderModel) bool { return m.ProviderModelId == id })
		if index >= 0 {
			draft.Models = append(draft.Models, kept[index])
		} else {
			draft.Models = append(draft.Models, bindings.BindingProviderModel{ProviderModelId: id, Nickname: id})
		}
	}
	var authentication bindings.BindingAuthenticationEdit = bindings.BindingAuthenticationEditKeep{}
	if e.apiKey() && key != "" {
		authentication = bindings.BindingAuthenticationEditSetApiKey{Value: key}
	}
	// The model key and the usage credentials are saved together.
	if p.quotaSource() == bindings.BindingProviderQuotaSourceArkCodingUsage {
		draft.QuotaAuthenticationEdit = e.quotaEdit()
	}
	p.busy, p.failure = true, ""
	run(p.win, func() outcome[bindings.BindingGatewayUpdate] {
		update, err := p.core.local.SaveProvider(defaultGateway, draft, authentication)
		return outcome[bindings.BindingGatewayUpdate]{update, err}
	}, func(r outcome[bindings.BindingGatewayUpdate]) {
		p.busy = false
		if r.err != nil {
			p.failure = describe(r.err)
			return
		}
		p.setGateway(r.value.Gateways)
		p.editor = providerEditor{}
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
	providers := p.providers()
	selected := p.chosen()
	listPage(c, p.pageFailure(), func() {
		list := ui.List(c, &p.list, len(providers), func(i int) {
			provider := providers[i]
			summary := fmt.Sprintf("%d 个模型 · %s", len(provider.Models), provider.Endpoint)
			listItem(c, provider.Name, summary, i == p.selected).ContextMenu(func(m *ui.Menu) {
				if m.Item("编辑…").Chosen() {
					p.edit(provider)
				}
				if m.Item("删除…").Chosen() {
					p.selected, p.deleting = i, true
				}
			})
		}).Grow(1)
		list.Children(func() {
			if len(providers) == 0 {
				muted(c, "还没有 AI 提供商。用＋添加。").Padding(12)
			}
		})
		if list.Submitted() && selected != nil {
			p.edit(*selected)
		}
	}, func() {
		if iconButton(c, addIcon, "添加 AI 提供商").Clicked() {
			p.add()
		}
		if iconButton(c, removeIcon, "删除 AI 提供商").Disabled(selected == nil || p.busy).Clicked() {
			p.deleting = true
		}
		ui.Spacer(c)
		if ui.Button(c, "编辑…").Disabled(selected == nil).Clicked() {
			p.edit(*selected)
		}
	})
	if p.editor.open {
		p.editorView(c)
	}
	if p.deleting && selected != nil {
		message := "删除「" + selected.Name + "」及其模型配置。使用这些模型的运行时需要重新选择模型；原运行时文件与会话保留。"
		if ui.AlertDialog(c, &p.deleting, "删除 AI 提供商？", message, "删除", "取消") == 0 {
			p.delete(selected.Id)
		}
	}
}

func (p *providersPane) editorView(c *ui.Context) {
	e := &p.editor
	selected := p.selectedTemplate()
	fixed := selected != nil && selected.Kind == bindings.BindingProviderTemplateKindSubscription
	required := func(value, label string) string {
		if e.checked && strings.TrimSpace(value) == "" {
			return "请填写" + label
		}
		return ""
	}
	title := "添加 AI 提供商"
	if e.existing != nil {
		title = "编辑 AI 提供商"
	}
	sheet(c, &e.open, title, p.failure, func() {
		ui.Form(c, func() {
			if e.existing == nil {
				ui.Field(c, "模板", func() {
					label := "自定义 API"
					if selected != nil {
						label = selected.Name
					}
					ui.MenuButton(c, label, func(m *ui.Menu) {
						if m.Item("自定义 API").Checked(selected == nil).Chosen() {
							p.chooseTemplate("")
						}
						for _, template := range p.templates {
							if m.Item(template.Name).Checked(template.Id == e.template).Chosen() {
								p.chooseTemplate(template.Id)
							}
						}
					})
				})
			}
			ui.Field(c, "名称", func() {
				ui.TextInput(c.Key("name"), &e.name).Placeholder("例如 DeepSeek").Grow(1)
			}).Error(required(e.name, "名称"))
			ui.Field(c, "协议", func() {
				label := "选择协议"
				if e.protocol != nil {
					label = p.protocolName(*e.protocol)
				}
				ui.MenuButton(c, label, func(m *ui.Menu) {
					for _, protocol := range p.protocols {
						if m.Item(protocol.Name).Checked(e.protocol != nil && *e.protocol == protocol.Id).Chosen() {
							id := protocol.Id
							e.protocol = &id
						}
					}
				}).Disabled(fixed)
			})
			ui.Field(c, "接口地址", func() {
				if fixed {
					ui.Text(c, selected.Endpoint)
					return
				}
				ui.TextInput(c.Key("endpoint"), &e.endpoint).Placeholder("例如 https://api.example.com/v1").Grow(1)
			}).Error(required(e.endpoint, "接口地址"))
			if e.apiKey() {
				p.keyField(c)
			} else {
				ui.Field(c, "认证", func() { muted(c, "此提供商的认证请在 Mac 上管理。") })
			}
			ui.Field(c, "模型 ID", func() {
				ui.TextInput(c.Key("models"), &e.models).Placeholder("提供商 API 规定的标识，多个以逗号分隔").Grow(1)
			}).Error(required(e.models, "至少一个模型 ID"))
			if p.quotaSource() == bindings.BindingProviderQuotaSourceArkCodingUsage {
				p.quotaFields(c)
			}
		})
		if selected != nil && selected.Disclosure != nil {
			muted(c, selected.Disclosure.Title+"："+selected.Disclosure.Detail)
		}
	}, func() {
		if ui.Button(c, "取消").Clicked() {
			e.open = false
		}
		if ui.PrimaryButton(c, "保存").Disabled(p.busy).Clicked() {
			p.save()
		}
	})
}

// keyField takes a new API key; an edited provider's saved key stays unless
// replaced, and can be read to view it.
func (p *providersPane) keyField(c *ui.Context) {
	e := &p.editor
	invalid := ""
	if e.checked && strings.TrimSpace(e.key) == "" && !e.keySaved() {
		invalid = "请填写 API key"
	}
	ui.Field(c, "API key", func() {
		ui.Row(c).Gap(8).Grow(1).AlignItems(ui.Center).Children(func() {
			input := ui.TextInput(c.Key("key"), &e.key).Grow(1)
			if !e.shown {
				input.Password()
			}
			if e.keySaved() {
				input.Placeholder("已保存；留空保持不变")
				if !e.shown && ui.Button(c, "显示").Clicked() {
					p.revealKey()
				}
			}
		})
	}).Description("保存在这台电脑的应用私有配置中。").Error(invalid)
}

// quotaFields are the ARK Coding Plan's usage query credentials, which the
// user links to the model key; the account is not checked.
func (p *providersPane) quotaFields(c *ui.Context) {
	e := &p.editor
	var described bindings.BindingQuotaAuthenticationDescription
	if e.existing != nil {
		described = e.existing.QuotaAuthentication
	}
	status := "尚未配置"
	switch {
	case e.clearingQuota:
		status = "保存后清除"
	case described.Configured && described.BindingConfirmed:
		status = "已由用户确认关联"
	case described.Configured:
		status = "需要重新确认关联"
	}
	incomplete := ""
	if e.checked && e.quotaPairIncomplete() {
		incomplete = "额度查询需要完整 AK/SK"
	}
	ui.Fieldset(c, "额度查询认证", func() {
		ui.Field(c, "状态", func() { ui.Text(c, status) }).
			Description("模型 API key 与此处 AK/SK 分别用于模型和额度查询。账户归属未自动核验。")
		ui.Field(c, "AccessKey ID", func() { ui.TextInput(c.Key("quota-id"), &e.quotaID).Grow(1) })
		ui.Field(c, "SecretAccessKey", func() { ui.TextInput(c.Key("quota-secret"), &e.quotaSecret).Password().Grow(1) }).
			Description("输入完整 AK/SK 替换；留空保留。改模型 Key 后需再次确认，有上限且未确认时暂停派发。上限按此来源观测执行，不是计费硬预算。").
			Error(incomplete)
		ui.Field(c, "确认关联", func() { ui.Switch(c, &e.quotaConfirmed) }).
			Description("此次保存时确认此来源对应当前模型 Key 的个人 Coding Plan")
		if described.Configured && !e.clearingQuota {
			ui.Field(c, "", func() {
				if ui.Button(c, "清除额度查询认证").Clicked() {
					e.quotaID, e.quotaSecret, e.quotaConfirmed, e.clearingQuota = "", "", false, true
				}
			})
		}
	})
}
