package main

import (
	"crypto/rand"
	"errors"
	"fmt"
	"maps"
	"os"
	"slices"
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

// runtimeEditor adds a runtime instance by hand or edits one: its name,
// whether it runs, its type and the type's settings.
type runtimeEditor struct {
	open bool
	// existing is the instance edited; nil adds one.
	existing *bindings.BindingRuntimeInstance
	kind     bindings.BindingRuntimeTypeDescriptor
	name     string
	enabled  bool
	values   map[string]string
	checked  bool
}

// runtimesPane holds this PC's Agent runtime instances: a list to add,
// import, edit and delete them.
type runtimesPane struct {
	core      *core
	win       *mygo.Window
	instances []bindings.BindingRuntimeInstance
	kinds     []bindings.BindingRuntimeTypeDescriptor
	selected  int
	list      ui.ListState
	editor    runtimeEditor
	deleting  bool
	// importing shows what discovery found.
	importing  bool
	candidates []bindings.BindingRuntimeDiscoveryCandidate
	busy       bool
	status     string
	failure    string
}

func newRuntimesPane(core *core) *runtimesPane {
	p := &runtimesPane{core: core, selected: -1}
	p.list.Key = func(i int) any { return p.instances[i].Id }
	p.list.Label = func(i int) string { return p.instances[i].Name }
	p.list.Selected = &p.selected
	return p
}

func (p *runtimesPane) reload() {
	run(p.win, func() outcome[bindings.BindingConfigurationSnapshot] {
		list, err := p.core.local.List()
		return outcome[bindings.BindingConfigurationSnapshot]{list, err}
	}, func(r outcome[bindings.BindingConfigurationSnapshot]) {
		if r.err != nil {
			p.failure = describe(r.err)
			return
		}
		p.setInstances(r.value.RuntimeInstances)
		p.kinds = r.value.RuntimeTypes
	})
}

func (p *runtimesPane) setInstances(instances []bindings.BindingRuntimeInstance) {
	p.instances = instances
	if p.selected >= len(instances) {
		p.selected = len(instances) - 1
	}
}

func (p *runtimesPane) chosen() *bindings.BindingRuntimeInstance {
	if p.selected < 0 || p.selected >= len(p.instances) {
		return nil
	}
	return &p.instances[p.selected]
}

// write runs one change against this PC's core, then shows the runtimes it
// reports.
func (p *runtimesPane) write(work func(*bindings.VeluneApplication) (bindings.BindingRuntimeUpdate, error), done func()) {
	if p.busy {
		return
	}
	p.busy, p.failure = true, ""
	run(p.win, func() outcome[bindings.BindingRuntimeUpdate] {
		update, err := work(p.core.local)
		return outcome[bindings.BindingRuntimeUpdate]{update, err}
	}, func(r outcome[bindings.BindingRuntimeUpdate]) {
		p.busy = false
		if r.err != nil {
			p.failure = describe(r.err)
			return
		}
		p.setInstances(r.value.RuntimeInstances)
		if done != nil {
			done()
		}
	})
}

func (p *runtimesPane) upsert(runtime bindings.BindingRuntimeInstance, done func()) {
	p.write(func(application *bindings.VeluneApplication) (bindings.BindingRuntimeUpdate, error) {
		return application.UpsertRuntime(runtime)
	}, done)
}

func (p *runtimesPane) remove(id string) {
	p.write(func(application *bindings.VeluneApplication) (bindings.BindingRuntimeUpdate, error) {
		return application.DeleteRuntime(id)
	}, nil)
}

func (p *runtimesPane) discover() {
	if p.busy {
		return
	}
	p.busy, p.failure, p.candidates, p.status = true, "", nil, "正在发现运行时…"
	run(p.win, func() outcome[[]bindings.BindingRuntimeDiscoveryCandidate] {
		candidates, err := discoverRuntimes(p.core.local)
		return outcome[[]bindings.BindingRuntimeDiscoveryCandidate]{candidates, err}
	}, func(r outcome[[]bindings.BindingRuntimeDiscoveryCandidate]) {
		p.busy = false
		if r.err != nil {
			p.status = describe(r.err)
			return
		}
		p.candidates = r.value
		if len(r.value) == 0 {
			p.status = "未发现运行时，可以手动添加。"
		} else {
			p.status = fmt.Sprintf("发现 %d 个运行时。", len(r.value))
		}
	})
}

func discoverRuntimes(application *bindings.VeluneApplication) ([]bindings.BindingRuntimeDiscoveryCandidate, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, errors.New("找不到用户目录")
	}
	node := nodeBinary()
	if node == "" {
		return nil, errors.New("没有找到 Node（node.exe），请先安装 Node 或手动添加运行时")
	}
	overrides := map[string]string{}
	// Where the agents keep their state, and where vendors' apps install them.
	for _, key := range []string{"PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "DSH_HOME", "LOCALAPPDATA", "APPDATA"} {
		if value, ok := os.LookupEnv(key); ok {
			overrides[key] = value
		}
	}
	hints, err := application.RuntimeDiscoveryHints(home, overrides)
	if err != nil {
		return nil, err
	}
	var probes []bindings.BindingRuntimeDiscoveryProbe
	for _, hint := range hints {
		// PATH first, then where the vendor's own installer, app or editor extension put it.
		binaries := commandCandidates(hint.FamilyId, hint.Command)
		for _, bundled := range hint.BundledBinaries {
			if !slices.Contains(binaries, bundled) {
				binaries = append(binaries, bundled)
			}
		}
		for _, binary := range binaries {
			probes = append(probes, bindings.BindingRuntimeDiscoveryProbe{FamilyId: hint.FamilyId, Binary: binary, NodeBinary: node, AgentDirectory: hint.AgentDirectory})
		}
	}
	return application.DiscoverRuntimes(probes)
}

// addable are the types set up here; an ACP Agent's launch arguments and
// environment are not edited here.
func (p *runtimesPane) addable() []bindings.BindingRuntimeTypeDescriptor {
	var kinds []bindings.BindingRuntimeTypeDescriptor
	for _, kind := range p.kinds {
		if !kind.LaunchOptions {
			kinds = append(kinds, kind)
		}
	}
	return kinds
}

func (p *runtimesPane) kind(id string) *bindings.BindingRuntimeTypeDescriptor {
	for i := range p.kinds {
		if p.kinds[i].Id == id {
			return &p.kinds[i]
		}
	}
	return nil
}

func (p *runtimesPane) add() {
	kinds := p.addable()
	if len(kinds) == 0 {
		return
	}
	p.editor, p.failure = runtimeEditor{open: true, enabled: true}, ""
	p.chooseKind(kinds[0])
}

func (p *runtimesPane) edit(runtime bindings.BindingRuntimeInstance) {
	kind := p.kind(runtime.TypeId)
	if kind == nil {
		p.failure = "这个运行时的类型已不受支持"
		return
	}
	values := maps.Clone(runtime.Settings)
	p.editor, p.failure = runtimeEditor{open: true, existing: &runtime, kind: *kind, name: runtime.Name, enabled: runtime.Enabled, values: values}, ""
}

func (p *runtimesPane) chooseKind(kind bindings.BindingRuntimeTypeDescriptor) {
	values := map[string]string{}
	for _, field := range kind.Fields {
		values[field.Key] = field.Value
	}
	p.editor.kind, p.editor.name, p.editor.values, p.editor.checked = kind, kind.Name, values, false
}

// missing is the first required field left empty; a stopped runtime needs none.
func (e *runtimeEditor) missing() *bindings.BindingSettingField {
	if !e.enabled {
		return nil
	}
	for i, field := range e.kind.Fields {
		if field.Required && strings.TrimSpace(e.values[field.Key]) == "" {
			return &e.kind.Fields[i]
		}
	}
	return nil
}

func (p *runtimesPane) saveEditor() {
	e := &p.editor
	e.checked = true
	if e.missing() != nil {
		return
	}
	runtime := bindings.BindingRuntimeInstance{Id: newUUID(), TypeId: e.kind.Id, GatewayId: defaultGateway, Settings: map[string]string{}}
	if e.existing != nil {
		// The instance's other settings stay as they are.
		runtime = *e.existing
		runtime.Settings = maps.Clone(runtime.Settings)
	}
	for _, field := range e.kind.Fields {
		if value := strings.TrimSpace(e.values[field.Key]); value != "" {
			runtime.Settings[field.Key] = value
		} else {
			delete(runtime.Settings, field.Key)
		}
	}
	runtime.Name, runtime.Enabled = strings.TrimSpace(e.name), e.enabled
	if runtime.Name == "" {
		runtime.Name = e.kind.Name
	}
	p.upsert(runtime, func() { p.editor.open = false })
}

func (p *runtimesPane) browse(key string, directory bool) {
	title := "选择文件"
	if directory {
		title = "选择目录"
	}
	go func() {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{Parent: p.win, Title: title, Directory: directory})
		if err != nil || len(paths) == 0 {
			return
		}
		p.win.Update(func() { p.editor.values[key] = paths[0] })
	}()
}

// pageFailure is what failed, shown on the page unless a sheet shows it.
func (p *runtimesPane) pageFailure() string {
	if p.editor.open || p.importing {
		return ""
	}
	return p.failure
}

func (p *runtimesPane) summary(runtime bindings.BindingRuntimeInstance) string {
	parts := []string{runtime.TypeId}
	if kind := p.kind(runtime.TypeId); kind != nil {
		parts[0] = kind.Name
	}
	if !runtime.Enabled {
		parts = append(parts, "已停用")
	}
	return strings.Join(parts, " · ")
}

func (p *runtimesPane) view(c *ui.Context) {
	selected := p.chosen()
	listPage(c, p.pageFailure(), func() {
		list := ui.List(c, &p.list, len(p.instances), func(i int) {
			runtime := p.instances[i]
			listItem(c, runtime.Name, p.summary(runtime), i == p.selected).ContextMenu(func(m *ui.Menu) {
				if m.Item("编辑…").Chosen() {
					p.edit(runtime)
				}
				label := "停用"
				if !runtime.Enabled {
					label = "启用"
				}
				if m.Item(label).Disabled(p.busy).Chosen() {
					runtime.Enabled = !runtime.Enabled
					p.upsert(runtime, nil)
				}
				if m.Item("删除…").Chosen() {
					p.selected, p.deleting = i, true
				}
			})
		}).Grow(1)
		list.Children(func() {
			if len(p.instances) == 0 {
				muted(c, "还没有 Agent 运行时。用＋手动添加，或快速导入已安装的。").Padding(12)
			}
		})
		if list.Submitted() && selected != nil {
			p.edit(*selected)
		}
	}, func() {
		if iconButton(c, addIcon, "添加 Agent 运行时实例").Disabled(len(p.addable()) == 0).Clicked() {
			p.add()
		}
		if ui.Button(c, "快速导入…").Disabled(p.busy).Clicked() {
			p.importing = true
			p.discover()
		}
		if iconButton(c, removeIcon, "删除运行时实例").Disabled(selected == nil || p.busy).Clicked() {
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
	if p.importing {
		p.importView(c)
	}
	if p.deleting {
		if ui.AlertDialog(c, &p.deleting, "删除运行时实例？", "只删除此配置实例；Agent 运行时自己的会话、资源和配置文件不会被删除。", "删除", "取消") == 0 && selected != nil {
			p.remove(selected.Id)
		}
	}
}

func (p *runtimesPane) editorView(c *ui.Context) {
	e := &p.editor
	title := "添加 Agent 运行时"
	if e.existing != nil {
		title = "编辑 Agent 运行时"
	}
	missing := e.missing()
	sheet(c, &e.open, title, p.failure, func() {
		ui.Form(c, func() {
			ui.Field(c, "实例名称", func() { ui.TextInput(c.Key("name"), &e.name).Grow(1) })
			ui.Field(c, "启用此运行时", func() { ui.Switch(c, &e.enabled) }).
				Description("停用后不读取其历史，也不创建或执行新任务；原生会话数据保留。")
			ui.Field(c, "类型", func() {
				if e.existing != nil {
					ui.Text(c, e.kind.Name)
					return
				}
				ui.MenuButton(c, e.kind.Name, func(m *ui.Menu) {
					for _, kind := range p.addable() {
						if m.Item(kind.Name).Checked(kind.Id == e.kind.Id).Chosen() {
							p.chooseKind(kind)
						}
					}
				})
			})
			if len(e.kind.Fields) > 0 {
				ui.Fieldset(c, "实例配置", func() {
					for _, field := range e.kind.Fields {
						p.fieldView(c, field, e.checked && missing != nil && missing.Key == field.Key)
					}
				})
			}
		})
		if e.kind.LaunchOptions {
			muted(c, "启动参数与环境请在 Mac 上编辑。")
		}
	}, func() {
		if ui.Button(c, "取消").Clicked() {
			e.open = false
		}
		if ui.PrimaryButton(c, "保存").Disabled(p.busy).Clicked() {
			p.saveEditor()
		}
	})
}

func (p *runtimesPane) fieldView(c *ui.Context, field bindings.BindingSettingField, missing bool) {
	e := &p.editor
	key := c.Key(field.Key)
	invalid := ""
	if missing {
		invalid = "请填写“" + field.Label + "”"
	}
	help := ""
	if field.Help != nil {
		help = *field.Help
	}
	ui.Field(key, field.Label, func() {
		value := e.values[field.Key]
		if field.Kind == bindings.BindingSettingKindChoice {
			chosen := "选择"
			for _, option := range field.Options {
				if option.Id == value {
					chosen = option.Label
				}
			}
			ui.MenuButton(c, chosen, func(m *ui.Menu) {
				for _, option := range field.Options {
					if m.Item(option.Label).Checked(option.Id == value).Chosen() {
						e.values[field.Key] = option.Id
					}
				}
			})
			return
		}
		ui.Row(c).Gap(8).Grow(1).AlignItems(ui.Center).Children(func() {
			ui.TextInput(c, &value).Grow(1)
			e.values[field.Key] = value
			if field.Kind != bindings.BindingSettingKindText && ui.Button(c, "浏览…").Clicked() {
				p.browse(field.Key, field.Kind == bindings.BindingSettingKindDirectoryPath)
			}
		})
	}).Description(help).Error(invalid)
}

// importView lists the runtimes discovery found, to add them one by one.
func (p *runtimesPane) importView(c *ui.Context) {
	sheet(c, &p.importing, "快速导入 Agent 运行时", p.failure, func() {
		muted(c, "在 PATH、npm／pnpm 全局目录，以及 Codex、Claude 桌面应用与编辑器扩展的安装位置中查找 Pi、Codex 与 Claude Code，只读取公开版本，不导入登录信息。")
		if p.status != "" {
			ui.Text(c, p.status).FontSize(13)
		}
		for i, candidate := range p.candidates {
			ui.Column(c.Key(i)).Children(func() {
				settingsRow(c, func() {
					ui.Column(c).Grow(1).MinWidth(0).Gap(2).Children(func() {
						title := candidate.Runtime.Name
						if candidate.Version != nil {
							title += " " + *candidate.Version
						}
						ui.Text(c, title)
						muted(c, candidate.Runtime.Settings["binary"]).SingleLine()
						if !candidate.Supported {
							muted(c, candidate.Detail)
						}
					})
					if candidate.AlreadyConfigured {
						muted(c, "已配置")
					} else if ui.Button(c, "添加").Disabled(p.busy || !candidate.Supported).Clicked() {
						p.upsert(candidate.Runtime, func() {
							if i < len(p.candidates) {
								p.candidates[i].AlreadyConfigured = true
							}
						})
					}
				})
			})
		}
	}, func() {
		if ui.Button(c, "重新发现").Disabled(p.busy).Clicked() {
			p.discover()
		}
		if ui.PrimaryButton(c, "完成").Clicked() {
			p.importing = false
		}
	})
}

// newUUID is a random (version 4) UUID.
func newUUID() string {
	b := make([]byte, 16)
	_, _ = rand.Read(b)
	b[6] = b[6]&0x0f | 0x40
	b[8] = b[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:])
}
