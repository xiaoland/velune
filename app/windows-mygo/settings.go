package main

import (
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

const (
	devicesTab = iota
	runtimesTab
	providersTab
	keepTab = -1
)

// settings is the settings window: devices, and this PC's runtimes and
// providers. Its fields change on the main thread only.
type settings struct {
	win       *mygo.Window
	tab       int
	devices   *devicesPane
	runtimes  *runtimesPane
	providers *providersPane
}

func openSettings(core *core, hub *hub, tab int) *settings {
	s := &settings{
		tab:       max(tab, devicesTab),
		devices:   &devicesPane{core: core, hub: hub},
		runtimes:  &runtimesPane{core: core},
		providers: &providersPane{core: core},
	}
	s.win = mygo.NewWindow(mygo.WindowOptions{
		Title: "设置", Width: 760, Height: 620, MinWidth: 560, MinHeight: 420,
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
			ui.Tabs(c, &s.tab, "设备", "运行时", "提供商")
		})
		ui.Scroll(c).Grow(1).Children(func() {
			ui.Column(c).Padding(0, 0, 16, 0).Children(func() {
				switch s.tab {
				case devicesTab:
					s.devices.view(c)
				case runtimesTab:
					s.runtimes.view(c)
				default:
					s.providers.view(c)
				}
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
func settingsRow(c *ui.Context, body func()) {
	t := c.Theme()
	ui.Row(c).Gap(12).Padding(8, 12).Radius(6).BorderWidth(1).BorderColor(t.Border).AlignItems(ui.Center).Children(body)
}

// labeled is a form line: a fixed-width label, then its control.
func labeled(c *ui.Context, label string, width float32, control func()) {
	ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
		ui.Text(c, label).Width(width).Shrink(0)
		control()
	})
}

func muted(c *ui.Context, text string) ui.Element {
	return ui.Text(c, text).FontSize(12).TextColor(c.Theme().TextMuted)
}

func failureText(c *ui.Context, failure string) {
	if failure != "" {
		ui.Text(c, failure).FontSize(13).TextColor(c.Theme().Danger).Padding(16, 24, 0, 24).Selectable()
	}
}
