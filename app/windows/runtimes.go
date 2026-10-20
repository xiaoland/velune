package main

import (
	"crypto/rand"
	"errors"
	"fmt"
	"os"
	"slices"
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

// runtimeDraft is a runtime being added by hand: its type, name and the
// type's settings.
type runtimeDraft struct {
	kind   bindings.BindingRuntimeTypeDescriptor
	name   string
	values map[string]string
}

// runtimesPane holds this PC's Agent runtimes: discovery of installed ones,
// and manual setup.
type runtimesPane struct {
	core       *core
	win        *mygo.Window
	instances  []bindings.BindingRuntimeInstance
	kinds      []bindings.BindingRuntimeTypeDescriptor
	candidates []bindings.BindingRuntimeDiscoveryCandidate
	draft      *runtimeDraft
	busy       bool
	status     string
	failure    string
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
		p.instances, p.kinds = r.value.RuntimeInstances, r.value.RuntimeTypes
	})
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
		p.instances = r.value.RuntimeInstances
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
			p.status, p.failure = "", describe(r.err)
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

func (p *runtimesPane) chooseKind(kind bindings.BindingRuntimeTypeDescriptor) {
	values := map[string]string{}
	for _, field := range kind.Fields {
		values[field.Key] = field.Value
	}
	p.draft = &runtimeDraft{kind: kind, name: kind.Name, values: values}
}

func (p *runtimesPane) saveDraft() {
	draft := p.draft
	settings := map[string]string{}
	for _, field := range draft.kind.Fields {
		value := strings.TrimSpace(draft.values[field.Key])
		if field.Required && value == "" {
			p.failure = fmt.Sprintf("请填写“%s”", field.Label)
			return
		}
		if value != "" {
			settings[field.Key] = value
		}
	}
	name := strings.TrimSpace(draft.name)
	if name == "" {
		name = draft.kind.Name
	}
	runtime := bindings.BindingRuntimeInstance{Enabled: true, Id: newUUID(), Name: name, TypeId: draft.kind.Id, GatewayId: "default", Settings: settings}
	p.upsert(runtime, func() { p.draft = nil })
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
		p.win.Update(func() {
			if p.draft != nil {
				p.draft.values[key] = paths[0]
			}
		})
	}()
}

func (p *runtimesPane) view(c *ui.Context) {
	failureText(c, p.failure)
	p.instancesView(c)
	p.discoveryView(c)
	p.manualView(c)
}

func (p *runtimesPane) kindName(id string) string {
	for _, kind := range p.kinds {
		if kind.Id == id {
			return kind.Name
		}
	}
	return id
}

func (p *runtimesPane) instancesView(c *ui.Context) {
	section(c, "已配置的运行时", func() {
		if len(p.instances) == 0 {
			muted(c, "还没有运行时。")
		}
		for _, runtime := range p.instances {
			ui.Column(c.Key(runtime.Id)).Children(func() {
				settingsRow(c, func() {
					ui.Column(c).Grow(1).MinWidth(0).Gap(2).Children(func() {
						ui.Text(c, runtime.Name)
						muted(c, p.kindName(runtime.TypeId))
						if binary := runtime.Settings["binary"]; binary != "" {
							muted(c, binary).SingleLine()
						}
					})
					enabled := runtime.Enabled
					if ui.Switch(c, &enabled).Disabled(p.busy).Changed() {
						toggled := runtime
						toggled.Enabled = enabled
						p.upsert(toggled, nil)
					}
					if ui.Button(c, "删除").Disabled(p.busy).Clicked() {
						id := runtime.Id
						p.write(func(application *bindings.VeluneApplication) (bindings.BindingRuntimeUpdate, error) {
							return application.DeleteRuntime(id)
						}, nil)
					}
				})
			})
		}
	})
}

func (p *runtimesPane) discoveryView(c *ui.Context) {
	section(c, "发现已安装的运行时", func() {
		muted(c, "在 PATH、npm／pnpm 全局目录，以及 Codex、Claude 桌面应用与编辑器扩展的安装位置中查找 Pi、Codex 与 Claude Code，只读取公开版本，不导入登录信息。")
		ui.Row(c).Gap(12).AlignItems(ui.Center).Children(func() {
			if ui.Button(c, "发现").Disabled(p.busy).Clicked() {
				p.discover()
			}
			if p.status != "" {
				muted(c, p.status)
			}
		})
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
						muted(c, candidate.Runtime.Settings["agentDir"]).SingleLine()
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
	})
}

func (p *runtimesPane) manualView(c *ui.Context) {
	section(c, "手动添加", func() {
		label := "选择运行时类型"
		if p.draft != nil {
			label = p.draft.kind.Name
		}
		ui.Row(c).Children(func() {
			ui.MenuButton(c, label, func(m *ui.Menu) {
				// Launch arguments and environment of ACP runtimes are not edited here.
				for _, kind := range p.kinds {
					if !kind.LaunchOptions && m.Item(kind.Name).Chosen() {
						p.chooseKind(kind)
					}
				}
			})
		})
		draft := p.draft
		if draft == nil {
			return
		}
		labeled(c, "名称", 140, func() {
			ui.TextInput(c.Key("name"), &draft.name).Grow(1)
		})
		for _, field := range draft.kind.Fields {
			if field.Kind == bindings.BindingSettingKindChoice {
				continue
			}
			text := field.Label
			if field.Required {
				text += " *"
			}
			labeled(c, text, 140, func() {
				value := draft.values[field.Key]
				input := ui.TextInput(c.Key(field.Key), &value).Grow(1)
				if field.Help != nil {
					input.Placeholder(*field.Help)
				}
				draft.values[field.Key] = value
				if field.Kind != bindings.BindingSettingKindText && ui.Button(c.Key("browse-"+field.Key), "浏览…").Clicked() {
					p.browse(field.Key, field.Kind == bindings.BindingSettingKindDirectoryPath)
				}
			})
		}
		for _, field := range draft.kind.Fields {
			if field.Kind != bindings.BindingSettingKindChoice {
				continue
			}
			current := draft.values[field.Key]
			chosen := "选择"
			for _, option := range field.Options {
				if option.Id == current {
					chosen = option.Label
				}
			}
			labeled(c, field.Label, 140, func() {
				ui.MenuButton(c.Key("choice-"+field.Key), chosen, func(m *ui.Menu) {
					for _, option := range field.Options {
						if m.Item(option.Label).Checked(option.Id == current).Chosen() {
							draft.values[field.Key] = option.Id
						}
					}
				})
			})
		}
		ui.Row(c).Gap(8).Children(func() {
			ui.Spacer(c)
			if ui.Button(c, "取消").Clicked() {
				p.draft = nil
			}
			if ui.Button(c, "添加").Disabled(p.busy).Clicked() {
				p.saveDraft()
			}
		})
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
