package main

// The conversation list, as Mac's sidebar: the list's groups as sections
// that fold, and each conversation one line with its state.

import (
	"fmt"
	"path/filepath"
	"slices"
	"strings"

	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

// groupID names a group of the list, as the key of its section.
func groupID(key bindings.BindingConversationGroupKey) string {
	switch key := key.(type) {
	case bindings.BindingConversationGroupKeyPinned:
		return "pinned"
	case bindings.BindingConversationGroupKeyRuntime:
		return "runtime:" + key.RuntimeId
	case bindings.BindingConversationGroupKeyProject:
		if key.Path == nil {
			return "project:"
		}
		return "project:" + *key.Path
	}
	return "all"
}

func (w *workspace) groupTitle(group bindings.BindingConversationGroup) string {
	switch key := group.Key.(type) {
	case bindings.BindingConversationGroupKeyPinned:
		return "置顶"
	case bindings.BindingConversationGroupKeyRuntime:
		if runtime := w.listing.configuration.runtime(key.RuntimeId); runtime != nil {
			return runtime.Name
		}
		return "Agent 运行时"
	case bindings.BindingConversationGroupKeyProject:
		switch {
		case group.Name != nil:
			return *group.Name
		case key.Path != nil:
			return filepath.Base(strings.ReplaceAll(*key.Path, `\`, "/"))
		}
		return "无项目"
	}
	return "会话"
}

func (w *workspace) sidebar(c *ui.Context) {
	t := c.Theme()
	for _, failure := range []string{w.hub.failure, w.listing.failure} {
		if failure != "" {
			ui.Text(c, failure).FontSize(12).TextColor(t.Danger).Padding(0, 12, 8, 12)
		}
	}
	switch {
	case !w.listing.loaded:
		ui.Column(c).Padding(16).Children(func() { ui.Spinner(c) })
		return
	case len(w.listing.conversations) == 0:
		empty := "这台设备上还没有会话。"
		if len(w.listing.configuration.runtimes) == 0 {
			empty = "这台设备还没有可用的 Agent 运行时。"
		}
		ui.Text(c, empty).FontSize(13).TextColor(t.TextMuted).Padding(8, 16)
		return
	}
	// The open conversation is chosen; a choice made in the list opens it.
	if open := w.openID(); open != w.shownOpen {
		w.chosen, w.shownOpen = open, open
	}
	list := ui.Sidebar(c, &w.chosen, func() {
		for _, group := range w.listing.groups {
			w.section(c, group)
		}
	}).Grow(1).Padding(0, 8, 8, 8)
	if list.Changed() && w.chosen != w.openID() {
		for _, listed := range w.listing.conversations {
			if listed.Conversation.Id == w.chosen {
				w.open(target{runtimeID: listed.Conversation.RuntimeId, conversationID: listed.Conversation.Id, title: listed.Conversation.Title})
			}
		}
	}
}

// section shows a group, folded as the user left it.
func (w *workspace) section(c *ui.Context, group bindings.BindingConversationGroup) {
	id := groupID(group.Key)
	open := w.sections[id]
	if open == nil {
		open = new(bool)
		*open = !group.Collapsed
		w.sections[id] = open
	} else if *open == group.Collapsed {
		// Folded or unfolded here since the list was read.
		w.setCollapsed(group.Key, !*open)
	}
	ui.SidebarSection(c.Key(id), w.groupTitle(group), open, func() {
		for _, listed := range w.listing.conversations {
			if groupID(listed.Group) == id {
				w.item(c, listed)
			}
		}
		if group.Loaded < group.Total {
			more := ui.Button(c.Key("more:"+id), fmt.Sprintf("加载更多（%d / %d）", group.Loaded, group.Total)).Margin(4, 8)
			if more.Clicked() {
				w.loadMore(group.Key)
			}
		}
	})
}

// item is one conversation: a dot while unread, its title, and whether it
// failed, waits for the user or runs.
func (w *workspace) item(c *ui.Context, listed bindings.BindingListedConversation) {
	t := c.Theme()
	conversation := listed.Conversation
	c = c.Key(conversation.Id)
	if r := w.renaming; r != nil && r.conversationID == conversation.Id {
		ui.Row(c).Padding(2, 8).Children(func() { w.renameInput(c) })
		return
	}
	active, isActive := w.listing.active[conversation.Id]
	running := isActive && active.RunState == bindings.BindingRunStateRunning
	unread := listed.Unread && !running
	mark := readIcon
	if unread && !listed.Failed {
		mark = unreadIcon
	}
	item := ui.SidebarItem(c, conversation.Id, mark, conversation.Title).Tooltip(conversation.Title)
	item.Children(func() {
		switch {
		case isActive && active.PendingInteractions > 0:
			ui.Icon(c, waitingIcon).FontSize(14).TextColor(t.Warning).Label("等待你处理").Tooltip("等待你处理")
		case running:
			ui.Spinner(c).Label("正在运行")
		case unread && listed.Failed:
			ui.Icon(c, failedIcon).FontSize(14).TextColor(t.Danger).Label("上一轮失败").Tooltip("上一轮失败")
		}
	})
	item.ContextMenu(func(m *ui.Menu) {
		if m.Item("重命名…").Chosen() {
			w.rename(listed)
		}
	})
	if item.DoubleClicked() {
		w.rename(listed)
	}
}

// setCollapsed keeps a group folded or open in the device's list preferences.
func (w *workspace) setCollapsed(key bindings.BindingConversationGroupKey, collapsed bool) {
	id := groupID(key)
	preferences := w.listing.preferences
	preferences.Collapsed = slices.DeleteFunc(slices.Clone(preferences.Collapsed), func(k bindings.BindingConversationGroupKey) bool {
		return groupID(k) == id
	})
	if collapsed {
		preferences.Collapsed = append(preferences.Collapsed, key)
	}
	w.listing.preferences = preferences
	for i := range w.listing.groups {
		if groupID(w.listing.groups[i].Key) == id {
			w.listing.groups[i].Collapsed = collapsed
		}
	}
	d := w.device
	run(w.win, func() error {
		application, err := w.core.application(d)
		if err == nil {
			_, err = application.SetConversationBrowserPreferences(preferences)
		}
		return err
	}, w.listFailure)
}

func (w *workspace) loadMore(key bindings.BindingConversationGroupKey) {
	d := w.device
	run(w.win, func() error {
		application, err := w.core.application(d)
		if err == nil {
			_, err = application.LoadMoreConversations(key)
		}
		return err
	}, func(err error) {
		w.listFailure(err)
		if err == nil {
			w.pollListing()
		}
	})
}

func (w *workspace) listFailure(err error) {
	if err != nil {
		w.listing.failure = describe(err)
	}
}
