package main

import (
	"fmt"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

type listing struct {
	conversations []bindings.BindingListedConversation
	runtimes      []bindings.BindingRuntimeInstance
	// projects are the working directories of the device's conversations, for
	// starting another in one.
	projects []string
	// active holds the running or waiting conversations by ID.
	active  map[string]bindings.BindingActiveConversation
	loaded  bool
	failure string
}

func newListing(list bindings.BindingConfigurationSnapshot, active []bindings.BindingActiveConversation) listing {
	l := listing{active: map[string]bindings.BindingActiveConversation{}, loaded: true}
	for _, project := range list.ConversationProjects {
		if !slices.Contains(l.projects, project.Path) {
			l.projects = append(l.projects, project.Path)
		}
	}
	for _, listed := range list.Conversations {
		if !listed.Archived && listed.Conversation.ParentConversationId == nil {
			l.conversations = append(l.conversations, listed)
		}
	}
	for _, runtime := range list.RuntimeInstances {
		if runtime.Enabled {
			l.runtimes = append(l.runtimes, runtime)
		}
	}
	for _, a := range active {
		l.active[a.ConversationId] = a
	}
	return l
}

// renaming is a conversation title being edited in the list.
type renaming struct {
	runtimeID, conversationID, title string
	focused                          bool
}

// workspace is the main window: one interface whose target device the user
// switches. Its fields change on the main thread only.
type workspace struct {
	core         *core
	hub          *hub
	win          *mygo.Window
	device       device
	listing      listing
	conversation *conversation
	renaming     *renaming
	// prompted holds the pairing attempts already put to the user; approving
	// is the one asked about now.
	prompted     map[string]bool
	approving    *bindings.BindingPairingStatus
	approvalOpen bool
	// markingRead holds conversations whose read mark is on its way; the next
	// listing may still show them unread.
	markingRead map[string]bool
	foreground  bool
	title       string
	settings    *settings
	// listing polls for one device at a time; a switch starts another round.
	round atomic.Int64
	list  ui.ListState
}

func newWorkspace(core *core, hub *hub) *workspace {
	w := &workspace{core: core, hub: hub, prompted: map[string]bool{}, markingRead: map[string]bool{}, foreground: true}
	w.list.Key = func(i int) any { return w.listing.conversations[i].Conversation.Id }
	hub.changed = w.hubChanged
	return w
}

// attach starts the window's work once it exists.
func (w *workspace) attach(win *mygo.Window) {
	w.win = win
	win.OnFocus(func() { w.setForeground(true) })
	win.OnBlur(func() { w.setForeground(false) })
	w.hub.start(win)
	w.pollListing()
}

func (w *workspace) setForeground(foreground bool) {
	win := w.win
	win.Update(func() {
		w.foreground = foreground
		w.hub.setActivity(w.device, w.device, foreground)
		w.noteSeen()
	})
}

func (w *workspace) hubChanged() {
	// The device was removed or its pairing undone: return to this PC.
	if w.device != localDevice && w.hub.state != nil {
		found := false
		for _, contact := range w.hub.devices() {
			found = found || contact.PeerId == string(w.device)
		}
		if !found {
			w.switchTo(localDevice)
		}
	}
	if w.approving == nil {
		w.nextApproval()
	}
	if w.settings != nil {
		w.settings.hubChanged()
	}
}

// openSettings shows the settings window at tab, reusing an open one;
// keepTab leaves an open window's tab as it is.
func (w *workspace) openSettings(tab int) {
	if w.settings != nil {
		w.settings.show(tab)
		return
	}
	s := openSettings(w.core, w.hub, tab)
	s.win.OnClosed(func() {
		if w.settings == s {
			w.settings = nil
		}
	})
	w.settings = s
}

// nextApproval asks about the next pairing waiting for this PC's answer.
func (w *workspace) nextApproval() {
	if w.hub.state == nil {
		return
	}
	for _, pairing := range w.hub.state.IncomingPairings {
		if pairing.Phase == bindings.BindingPairingPhaseAwaitingApproval && !w.prompted[pairing.AttemptId] {
			w.prompted[pairing.AttemptId] = true
			w.approving, w.approvalOpen = &pairing, true
			w.win.Focus()
			return
		}
	}
}

func (w *workspace) answerPairing(allow bool) {
	pairing := *w.approving
	w.approving, w.approvalOpen = nil, false
	go func() {
		endpoint, err := w.core.remote()
		if err != nil {
			return
		}
		if allow {
			_, _ = endpoint.ApprovePairing(pairing.AttemptId, pairing.PeerId)
		} else {
			_, _ = endpoint.RejectPairing(pairing.AttemptId, pairing.PeerId)
		}
	}()
	w.nextApproval()
}

func (w *workspace) pollListing() {
	round := w.round.Add(1)
	d, win := w.device, w.win
	type listed struct {
		list     bindings.BindingConfigurationSnapshot
		activity bindings.BindingConversationActivity
		err      error
	}
	go func() {
		var cursor uint64
		for w.round.Load() == round {
			var result listed
			application, err := w.core.application(d)
			if err == nil {
				result.list, err = application.List()
			}
			if err == nil {
				result.activity, err = application.ConversationActivity(cursor)
			}
			result.err = err
			if err == nil {
				cursor = result.activity.Cursor
			}
			win.Update(func() {
				if w.round.Load() != round {
					return
				}
				if result.err != nil {
					w.listing.loaded, w.listing.failure = true, describe(result.err)
					return
				}
				w.listing = newListing(result.list, result.activity.Active)
				w.noteSeen()
			})
			delay := 5 * time.Second
			if err == nil && len(result.activity.Active) > 0 {
				delay = 2 * time.Second
			}
			time.Sleep(delay)
		}
	}()
}

func (w *workspace) switchTo(d device) {
	if d == w.device {
		return
	}
	w.hub.setActivity(w.device, d, w.foreground)
	w.device = d
	w.closeConversation()
	w.listing = listing{}
	w.renaming = nil
	w.pollListing()
}

func (w *workspace) closeConversation() {
	if w.conversation != nil {
		w.conversation.close()
		w.conversation = nil
	}
}

func (w *workspace) open(t target) {
	w.closeConversation()
	w.conversation = newConversation(w.core, w.win, w.device, t)
	w.noteSeen()
}

func (w *workspace) startConversation() {
	w.closeConversation()
	w.conversation = newDraft(w.core, w.win, w.device, w.listing.runtimes, w.listing.projects)
}

func (w *workspace) openID() string {
	if w.conversation == nil {
		return ""
	}
	return w.conversation.target.conversationID
}

// noteSeen marks the open conversation read once it shows in the active window.
func (w *workspace) noteSeen() {
	id := w.openID()
	if !w.foreground || id == "" || w.markingRead[id] {
		return
	}
	var reference *bindings.BindingConversationReference
	for _, listed := range w.listing.conversations {
		if listed.Conversation.Id == id && listed.Unread {
			reference = &bindings.BindingConversationReference{RuntimeId: listed.Conversation.RuntimeId, ConversationId: id}
		}
	}
	if reference == nil {
		return
	}
	w.markingRead[id] = true
	d := w.device
	run(w.win, func() error {
		application, err := w.core.application(d)
		if err != nil {
			return err
		}
		_, err = application.OrganizeConversations([]bindings.BindingConversationReference{*reference}, bindings.BindingConversationOrganizationChangeMarkRead{})
		return err
	}, func(err error) {
		delete(w.markingRead, id)
		if err != nil {
			w.listing.failure = describe(err)
			return
		}
		for i := range w.listing.conversations {
			if w.listing.conversations[i].Conversation.Id == id {
				w.listing.conversations[i].Unread = false
			}
		}
	})
}

// rename edits the conversation's real title in place.
func (w *workspace) rename(listed bindings.BindingListedConversation) {
	if !listed.Conversation.CanRename {
		return
	}
	w.renaming = &renaming{runtimeID: listed.Conversation.RuntimeId, conversationID: listed.Conversation.Id, title: listed.Conversation.Title}
}

func (w *workspace) finishRename(keep bool) {
	r := w.renaming
	w.renaming = nil
	title := strings.TrimSpace(r.title)
	if !keep || title == "" {
		return
	}
	for i := range w.listing.conversations {
		if w.listing.conversations[i].Conversation.Id == r.conversationID {
			w.listing.conversations[i].Conversation.Title = title
		}
	}
	d := w.device
	run(w.win, func() error {
		application, err := w.core.application(d)
		if err != nil {
			return err
		}
		_, err = application.RenameConversation(r.runtimeID, r.conversationID, title)
		return err
	}, func(err error) {
		if err != nil {
			w.listing.failure = describe(err)
		}
	})
}

func (w *workspace) view(c *ui.Context) {
	t := c.Theme()
	title := "Velune"
	if w.conversation != nil {
		title = w.conversation.title()
	}
	// The first frame is built while the window is created, before attach.
	if title = title + " — " + w.hub.deviceName(w.device); title != w.title && w.win != nil {
		w.title = title
		w.win.SetTitle(title)
	}
	if c.Shortcut(0, ui.KeyF2) && w.renaming == nil {
		for _, listed := range w.listing.conversations {
			if listed.Conversation.Id == w.openID() {
				w.rename(listed)
			}
		}
	}
	ui.Row(c).Fill().AlignItems(ui.Stretch).Children(func() {
		ui.Column(c).Width(300).Shrink(0).Background(t.Surface).BorderWidth(0, 1, 0, 0).BorderColor(t.Border).Children(func() {
			w.sidebar(c)
		})
		ui.Column(c).Grow(1).MinWidth(0).Children(func() {
			if w.conversation != nil {
				w.conversation.view(c)
				return
			}
			ui.Column(c).Fill().Center().Children(func() {
				ui.Text(c, "选择或新建一个会话").TextColor(t.TextMuted)
			})
		})
	})
	w.approvalDialog(c)
}

func (w *workspace) sidebar(c *ui.Context) {
	t := c.Theme()
	ui.Row(c).Padding(8).Gap(4).Children(func() {
		w.deviceSwitcher(c)
		if ui.Button(c, "设置").Clicked() {
			w.openSettings(keepTab)
		}
	})
	ui.Row(c).Padding(0, 8, 8, 8).Children(func() {
		if ui.Button(c, "＋ 新会话").Disabled(len(w.listing.runtimes) == 0).Grow(1).Clicked() {
			w.startConversation()
		}
	})
	for _, failure := range []string{w.hub.failure, w.listing.failure} {
		if failure != "" {
			ui.Text(c, failure).FontSize(12).TextColor(t.Danger).Padding(0, 12, 8, 12)
		}
	}
	switch {
	case !w.listing.loaded:
		ui.Column(c).Padding(16).Children(func() { ui.Spinner(c) })
	case len(w.listing.conversations) == 0:
		empty := "这台设备上还没有会话。"
		if len(w.listing.runtimes) == 0 {
			empty = "这台设备还没有可用的运行时。"
		}
		ui.Text(c, empty).FontSize(13).TextColor(t.TextMuted).Padding(16)
	default:
		ui.List(c, &w.list, len(w.listing.conversations), func(i int) { w.row(c, i) }).Grow(1).Padding(0, 8).Gap(2)
	}
}

func (w *workspace) deviceSwitcher(c *ui.Context) {
	label := w.hub.deviceName(w.device)
	if w.device == localDevice {
		label += "（本机）"
	}
	ui.MenuButton(c, label, func(m *ui.Menu) {
		if m.Item(w.hub.deviceName(localDevice) + "（本机）").Checked(w.device == localDevice).Chosen() {
			w.switchTo(localDevice)
		}
		for _, contact := range w.hub.devices() {
			if m.Item(contact.Name).Checked(w.device == device(contact.PeerId)).Chosen() {
				w.switchTo(device(contact.PeerId))
			}
		}
		m.Separator()
		if m.Item("添加设备…").Chosen() {
			w.openSettings(devicesTab)
		}
	}).Grow(1)
}

func (w *workspace) row(c *ui.Context, i int) {
	t := c.Theme()
	listed := w.listing.conversations[i]
	conversation := listed.Conversation
	var detail []string
	for _, runtime := range w.listing.runtimes {
		if runtime.Id == conversation.RuntimeId {
			detail = append(detail, runtime.Name)
		}
	}
	if conversation.UpdatedAtUnixMs != nil {
		detail = append(detail, relativeTime(*conversation.UpdatedAtUnixMs))
	}
	if listed.Project != nil {
		detail = append(detail, filepath.Base(strings.ReplaceAll(*listed.Project, `\`, "/")))
	}
	selected := conversation.Id == w.openID()
	editing := w.renaming != nil && w.renaming.conversationID == conversation.Id
	row := ui.Row(c.Key("row")).Padding(8, 12).Gap(8).Radius(6).AlignItems(ui.Center)
	row.Children(func() {
		ui.Column(c).Grow(1).MinWidth(0).Gap(2).Children(func() {
			if editing {
				w.renameInput(c)
			} else {
				ui.Text(c, conversation.Title).SingleLine()
			}
			ui.Text(c, strings.Join(detail, " · ")).FontSize(12).TextColor(t.TextMuted).SingleLine()
		})
		active, isActive := w.listing.active[conversation.Id]
		switch {
		case isActive && active.PendingInteractions > 0:
			ui.Text(c, "等待回应").FontSize(12).TextColor(t.Accent)
		case isActive && active.RunState == bindings.BindingRunStateRunning:
			ui.Spinner(c)
		case listed.Failed:
			ui.Text(c, "失败").FontSize(12).TextColor(t.Danger)
		case listed.Unread:
			ui.Text(c, "●").FontSize(12).TextColor(t.Accent)
		}
	})
	if selected || row.Hovered() {
		row.Background(t.SurfaceHover)
	}
	if editing {
		return
	}
	if row.DoubleClicked() {
		w.rename(listed)
	} else if row.Clicked() && !selected {
		w.open(target{runtimeID: conversation.RuntimeId, conversationID: conversation.Id, title: conversation.Title})
	}
}

// renameInput saves on Enter; Esc or a click elsewhere cancels.
func (w *workspace) renameInput(c *ui.Context) {
	r := w.renaming
	input := ui.TextInput(c.Key("rename"), &r.title).AutoFocus()
	switch {
	case input.Submitted():
		w.finishRename(true)
	case input.Shortcut(0, ui.KeyEscape):
		w.finishRename(false)
	case input.Focused():
		r.focused = true
	case r.focused:
		w.finishRename(false)
	}
}

// approvalDialog asks whether a device may control this PC. Both devices show
// the same code; only the user here can let the other one in. Neither answer
// is the default, so a stray Enter or Esc grants nothing.
func (w *workspace) approvalDialog(c *ui.Context) {
	if w.approving == nil {
		return
	}
	t := c.Theme()
	pairing := *w.approving
	code := ""
	if pairing.VerificationCode != nil {
		code = formatCode(*pairing.VerificationCode)
	}
	if ui.Modal(c, &w.approvalOpen, func() {
		ui.Column(c).Width(420).Gap(12).Children(func() {
			ui.Text(c, fmt.Sprintf("“%s”请求控制这台电脑", pairing.Name)).Bold().FontSize(16)
			ui.Text(c, "确认两台设备显示相同的核对码。允许后，对方可以查看和操作这台电脑上的会话。").TextColor(t.TextMuted)
			ui.Row(c).Children(func() {
				ui.Spacer(c)
				// The code takes the dialog's focus, so no button is the default.
				ui.Text(c, code).Font(monoFamily()).FontSize(30).Focusable().AutoFocus()
				ui.Spacer(c)
			})
			ui.Row(c).Gap(8).Children(func() {
				ui.Spacer(c)
				if ui.Button(c, "拒绝").Clicked() {
					w.answerPairing(false)
				}
				if ui.Button(c, "允许").Clicked() {
					w.answerPairing(true)
				}
			})
		})
	}).Dismissed() {
		// The question stays until one of the buttons answers it.
		w.approvalOpen = true
	}
}

// formatCode groups the eight digits as four and four, as the other device
// shows them.
func formatCode(code string) string {
	if len(code) == 8 {
		return code[:4] + " " + code[4:]
	}
	return code
}

func relativeTime(unixMs int64) string {
	minutes := max(time.Now().UnixMilli()-unixMs, 0) / 60_000
	switch {
	case minutes == 0:
		return "刚刚"
	case minutes < 60:
		return fmt.Sprintf("%d 分钟前", minutes)
	case minutes < 1440:
		return fmt.Sprintf("%d 小时前", minutes/60)
	default:
		return fmt.Sprintf("%d 天前", minutes/1440)
	}
}
