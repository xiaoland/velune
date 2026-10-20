package main

import (
	"path/filepath"
	"strings"

	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

// composerView is the input and, below it, the next turn's action row: what
// it runs on, what it may do, and for an unsent conversation where it works.
func (v *conversation) composerView(c *ui.Context) {
	t := c.Theme()
	canCancel := v.running() && v.snapshot.Actions.CanCancel
	ui.Column(c).Gap(8).Padding(12, 16).BorderWidth(1, 0, 0, 0).BorderColor(t.Border).Children(func() {
		area := ui.TextArea(c.Key("composer"), &v.composer).Lines(2, 10).Placeholder("输入消息，Ctrl+Enter 发送").AutoFocus()
		if area.Shortcut(ui.Ctrl, ui.KeyEnter) {
			v.send()
		}
		ui.Row(c).Gap(4).AlignItems(ui.Center).Children(func() {
			if v.next != "" {
				v.agentMenu(c)
				if len(v.permissionModes()) > 0 {
					v.permissionMenu(c)
				}
			}
			if v.unsent() {
				v.cwdMenu(c)
			}
			if v.running() {
				ui.Spinner(c)
				ui.Text(c, "正在运行").FontSize(13).TextColor(t.TextMuted)
			}
			ui.Spacer(c)
			if canCancel {
				if ui.Button(c, "停止").Disabled(v.busy).Clicked() {
					v.stop()
				}
			} else if ui.PrimaryButton(c, "发送").Disabled(v.busy || v.next == "").Clicked() {
				v.send()
			}
		})
	})
}

// agentMenu chooses the next turn's runtime, model and reasoning level, one
// submenu each, titled with the choice.
func (v *conversation) agentMenu(c *ui.Context) {
	runtime := "选择运行时"
	if r := v.configuration.runtime(v.next); r != nil {
		runtime = r.Name
	}
	model := v.modelName()
	levels := v.effortOptions()
	// An unchosen part follows the conversation's current one, or is the runtime's own.
	unchosen := "运行时默认"
	if v.hasTurns(v.next) {
		unchosen = "沿用当前"
	}
	parts := []string{runtime, model}
	effort := v.effort()
	if effort == nil {
		effort = v.currentEffort()
	}
	if len(levels) > 0 && effort != nil {
		parts = append(parts, *effort)
	}
	ui.MenuButton(c, strings.Join(parts, " · "), func(m *ui.Menu) {
		m.Submenu("运行时："+runtime, func(m *ui.Menu) {
			for _, r := range v.configuration.runtimes {
				if m.Item(r.Name).Checked(r.Id == v.next).Chosen() {
					v.chooseRuntime(r.Id)
				}
			}
			m.Separator()
			if m.Item("Agent 运行时设置…").Chosen() {
				v.openSettings(runtimesTab)
			}
		})
		m.Submenu("模型："+model, func(m *ui.Menu) { v.modelItems(m, model) })
		if len(levels) == 0 {
			return
		}
		title := unchosen
		if effort != nil {
			title = *effort
		}
		m.Submenu("推理强度："+title, func(m *ui.Menu) {
			first := unchosen
			if current := v.currentEffort(); current != nil {
				first = "沿用当前（" + *current + "）"
			}
			chosen := v.effort()
			if m.Item(first).Checked(chosen == nil).Chosen() {
				v.choose(false, func(choice *bindings.BindingTurnChoice) { choice.Effort = nil })
			}
			for _, level := range levels {
				if m.Item(level).Checked(chosen != nil && *chosen == level).Chosen() {
					v.choose(true, func(choice *bindings.BindingTurnChoice) { choice.Effort = &level })
				}
			}
		})
	}).Tooltip("下一轮的运行时、模型与推理强度")
}

// modelItems lists no choice first (the conversation's current model, or
// the runtime's default), then the runtime's own models, before the models
// Velune's providers serve.
func (v *conversation) modelItems(m *ui.Menu, model string) {
	if catalog := v.catalogs[v.next]; catalog != nil {
		switch {
		case catalog.loading:
			m.Item("正在读取运行时的模型…").Disabled(true)
		case catalog.failure != "":
			m.Item("无法读取运行时的模型：" + catalog.failure).Disabled(true)
		}
	}
	chosen := v.choice().Model
	first := "运行时默认"
	if v.hasTurns(v.next) && v.nextModel() != nil {
		first = "沿用当前（" + model + "）"
	}
	if m.Item(first).Checked(chosen == nil).Chosen() {
		v.choose(false, func(choice *bindings.BindingTurnChoice) { choice.Model = nil })
	}
	pick := func(model modelChoice) {
		if m.Item(model.name).Checked(sameSelection(model.selection, chosen)).Chosen() {
			v.choose(true, func(choice *bindings.BindingTurnChoice) { choice.Model = model.selection })
		}
	}
	choices := v.modelChoices()
	managed := false
	for _, choice := range choices {
		if choice.managed() {
			managed = true
		} else {
			pick(choice)
		}
	}
	if managed {
		m.Separator()
		m.Item("Velune 提供商").Disabled(true)
		for _, choice := range choices {
			if choice.managed() {
				pick(choice)
			}
		}
	}
	m.Separator()
	loading := v.catalogs[v.next] != nil && v.catalogs[v.next].loading
	if m.Item("重新读取模型").Disabled(loading).Chosen() {
		v.loadCatalog(v.next, true)
	}
	if m.Item("AI 提供商设置…").Chosen() {
		v.openSettings(providersTab)
	}
}

// permissionMenu chooses what the next turn may do, among the runtime
// type's modes in its own names. A mode is the session's state in the
// runtime: choosing none keeps the current one.
func (v *conversation) permissionMenu(c *ui.Context) {
	mode := v.permissionMode(v.choice().PermissionMode)
	var current *bindings.BindingPermissionMode
	if settings := v.currentSettings(); settings != nil {
		current = v.permissionMode(settings.PermissionMode)
	}
	title := "运行时默认权限"
	if shown := mode; shown != nil || current != nil {
		if shown == nil {
			shown = current
		}
		title = shown.Label
	}
	tooltip := "权限模式：由运行时使用自己的默认模式"
	switch {
	case mode != nil:
		tooltip = "权限模式：" + mode.Label + "。" + mode.Detail
	case v.hasTurns(v.next):
		tooltip = "权限模式：沿用会话当前的模式"
	}
	ui.MenuButton(c, title, func(m *ui.Menu) {
		first := "运行时默认"
		switch {
		case current != nil:
			first = "沿用当前（" + current.Label + "）"
		case v.hasTurns(v.next):
			first = "沿用当前"
		}
		if m.Item(first).Checked(mode == nil).Chosen() {
			v.choose(false, func(choice *bindings.BindingTurnChoice) { choice.PermissionMode = nil })
		}
		for _, option := range v.permissionModes() {
			if m.Item(option.Label).Checked(mode != nil && mode.Id == option.Id).Chosen() {
				v.choose(true, func(choice *bindings.BindingTurnChoice) { choice.PermissionMode = &option.Id })
			}
		}
	}).Tooltip(tooltip)
}

// cwdMenu chooses where an unsent conversation works. This PC's folders are
// browsed; another device's are typed, as they exist only on it.
func (v *conversation) cwdMenu(c *ui.Context) {
	title, tooltip := "工作目录", "工作目录：留空时使用运行时的默认目录"
	if v.cwd != "" {
		title, tooltip = filepath.Base(strings.ReplaceAll(v.cwd, `\`, "/")), v.cwd
	}
	ui.MenuButton(c, title, func(m *ui.Menu) {
		if v.device == localDevice {
			if m.Item("选择目录…").Chosen() {
				v.browse()
			}
		} else if m.Item("输入目录…").Chosen() {
			v.cwdInput, v.enteringCwd = v.cwd, true
		}
		if v.cwd != "" && m.Item("使用运行时默认目录").Chosen() {
			v.cwd = ""
		}
		if projects := v.configuration.projects; len(projects) > 0 {
			m.Separator()
			m.Item("最近使用").Disabled(true)
			for _, project := range projects[:min(len(projects), 5)] {
				if m.Item(project).Checked(project == v.cwd).Chosen() {
					v.cwd = project
				}
			}
		}
	}).Tooltip(tooltip)
}

// cwdDialog takes an absolute path on the device the conversation runs on;
// the device checks it when the conversation starts.
func (v *conversation) cwdDialog(c *ui.Context) {
	if !v.enteringCwd {
		return
	}
	t := c.Theme()
	use := func() {
		v.cwd, v.enteringCwd = strings.TrimSpace(v.cwdInput), false
	}
	ui.Modal(c, &v.enteringCwd, func() {
		ui.Column(c).Width(420).Gap(12).Children(func() {
			ui.Text(c, "新会话的工作目录").Bold().FontSize(16)
			ui.Text(c, "输入那台设备上的绝对路径；留空使用运行时的默认目录。").TextColor(t.TextMuted)
			if ui.TextInput(c.Key("cwd"), &v.cwdInput).AutoFocus().Submitted() {
				use()
			}
			ui.Row(c).Gap(8).Children(func() {
				ui.Spacer(c)
				if ui.Button(c, "取消").Clicked() {
					v.enteringCwd = false
				}
				if ui.PrimaryButton(c, "使用").Clicked() {
					use()
				}
			})
		})
	})
}
