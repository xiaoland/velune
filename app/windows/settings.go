package main

import (
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The settings tabs, named and ordered as Mac's that Windows has.
const (
	providersTab = iota
	runtimesTab
	devicesTab
	keepTab = -1
)

// settings is the settings window: this PC's providers and runtimes, and
// devices. Its fields change on the main thread only.
type settings struct {
	win       *mygo.Window
	tab       int
	devices   *devicesPane
	runtimes  *runtimesPane
	providers *providersPane
}

// openSettings opens the window at tab; open shows a paired device in the
// main window.
func openSettings(core *core, hub *hub, tab int, open func(device)) *settings {
	s := &settings{
		tab:       max(tab, providersTab),
		devices:   &devicesPane{core: core, hub: hub, open: open},
		runtimes:  newRuntimesPane(core),
		providers: newProvidersPane(core),
	}
	s.win = mygo.NewWindow(mygo.WindowOptions{
		Title: "设置", Width: 720, Height: 600, MinWidth: 560, MinHeight: 420,
		Content: ui.View(s.view),
	})
	s.devices.win, s.runtimes.win, s.providers.win = s.win, s.win, s.win
	s.runtimes.reload()
	s.providers.reload()
	return s
}

func (s *settings) show(tab int) {
	if tab != keepTab {
		s.tab = tab
	}
	s.win.Invalidate()
	s.win.Focus()
}

// hubChanged shows the remote service's new state.
func (s *settings) hubChanged() {
	s.devices.hubChanged()
	s.win.Invalidate()
}

func (s *settings) view(c *ui.Context) {
	t := c.Theme()
	ui.Column(c).Fill().Children(func() {
		ui.Row(c).Padding(8, 16, 0, 16).BorderWidth(0, 0, 1, 0).BorderColor(t.Border).Children(func() {
			ui.Tabs(c, &s.tab, "AI提供商", "Agent 运行时", "设备")
		})
		switch s.tab {
		case providersTab:
			s.providers.view(c)
		case runtimesTab:
			s.runtimes.view(c)
		default:
			ui.Scroll(c).Grow(1).Children(func() {
				ui.Column(c).Padding(0, 0, 16, 0).Children(func() { s.devices.view(c) })
			})
		}
	})
}

// listPage is a settings page of one bordered list and, below it, its
// actions: ＋ and − first, the others after.
func listPage(c *ui.Context, failure string, list func(), actions func()) {
	t := c.Theme()
	ui.Column(c).Grow(1).MinHeight(0).Gap(12).Padding(16, 20).Children(func() {
		if failure != "" {
			ui.Text(c, failure).FontSize(13).TextColor(t.Danger).Selectable()
		}
		ui.Column(c).Grow(1).MinHeight(0).Radius(6).BorderWidth(1).BorderColor(t.Border).Background(t.Background).Clip().Children(list)
		ui.Row(c).Gap(4).AlignItems(ui.Center).Children(actions)
	})
}

// listItem is a row of a settings list: a name and, below it, a summary,
// which a selected row shows on the accent color.
func listItem(c *ui.Context, name, summary string, selected bool) ui.Element {
	return ui.Row(c).Padding(6, 12).AlignItems(ui.Center).Children(func() {
		ui.Column(c).Grow(1).MinWidth(0).Gap(2).Children(func() {
			ui.Text(c, name).SingleLine()
			text := muted(c, summary).SingleLine()
			if selected {
				text.TextColor(c.Theme().AccentText)
			}
		})
	})
}

// sheet is a dialog over the settings window for adding or editing one
// item, with what failed and its buttons at the bottom.
func sheet(c *ui.Context, open *bool, title, failure string, body func(), buttons func()) {
	ui.Modal(c, open, func() {
		ui.Column(c).Width(520).Gap(16).Children(func() {
			ui.Text(c, title).Bold().FontSize(16)
			ui.Scroll(c).MaxHeight(440).Children(func() {
				ui.Column(c).Gap(12).Padding(0, 4, 0, 0).Children(body)
			})
			if failure != "" {
				ui.Text(c, failure).FontSize(13).TextColor(c.Theme().Danger).Selectable()
			}
			ui.Row(c).Gap(8).Children(func() {
				ui.Spacer(c)
				buttons()
			})
		})
	})
}

// section is a titled group of settings rows.
func section(c *ui.Context, title string, body func()) {
	ui.Column(c).Gap(8).Padding(16, 24, 0, 24).Children(func() {
		ui.Text(c, title).Bold().FontSize(13).TextColor(c.Theme().TextMuted)
		body()
	})
}

// settingsRow is one row: what it is on the left, its controls on the right.
func settingsRow(c *ui.Context, body func()) ui.Element {
	t := c.Theme()
	return ui.Row(c).Gap(12).Padding(8, 12).Radius(6).BorderWidth(1).BorderColor(t.Border).AlignItems(ui.Center).Children(body)
}

func muted(c *ui.Context, text string) ui.Element {
	return ui.Text(c, text).FontSize(12).TextColor(c.Theme().TextMuted)
}

func failureText(c *ui.Context, failure string) {
	if failure != "" {
		ui.Text(c, failure).FontSize(13).TextColor(c.Theme().Danger).Padding(16, 24, 0, 24).Selectable()
	}
}
