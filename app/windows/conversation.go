package main

import (
	"errors"
	"fmt"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
	"velune/bindings"
)

// target is where a conversation view points: an existing conversation, or,
// with no conversation ID, one to start on a runtime.
type target struct {
	runtimeID      string
	conversationID string
	title          string
}

// conversation is one open conversation on a device: transcript, pending
// interaction and composer. Its fields change on the main thread only.
type conversation struct {
	core   *core
	win    *mygo.Window
	device device
	target target
	// configuration is the device's, kept current by the workspace.
	configuration configuration
	openSettings  func(tab int)

	snapshot *bindings.BindingConversationSnapshot
	messages []bindings.BindingMessage
	// payloads are deferred contents read on request, by message ID.
	payloads map[string]bindings.BindingMessage
	expanded map[string]bool

	// next is the runtime the next turn runs on, which may differ from the
	// conversation's; it follows the conversation's current one until the
	// user picks another. intent holds this conversation's choices by
	// runtime, runtimeChoices what each runtime's new conversations start with.
	next           string
	nextChosen     bool
	intentRuntime  *string
	intent         map[string]bindings.BindingTurnChoice
	runtimeChoices map[string]bindings.BindingRuntimeComposerChoices
	catalogs       map[string]*catalog
	// cwd is where an unsent conversation will work; empty is the runtime's
	// default. A remote device's directory is typed, not browsed.
	cwd         string
	enteringCwd bool
	cwdInput    string

	busy     bool
	loading  bool
	failure  string
	list     ui.ListState
	md       markdown.Options
	composer string

	// answers and chosen hold the replies to the interaction shown, by question ID.
	answering string
	answers   map[string]string
	chosen    map[string]string

	closed atomic.Bool
	// polled is what the polling goroutine reads: the conversation and its cursor.
	pollMu sync.Mutex
	polled *polledConversation
}

type polledConversation struct {
	runtimeID, conversationID string
	cursor                    bindings.BindingTranscriptCursor
	running                   bool
}

// newConversation opens t; one without a conversation ID is an empty
// conversation that its first message starts.
func newConversation(core *core, win *mygo.Window, d device, t target, config configuration, openSettings func(int)) *conversation {
	if t.conversationID == "" && t.runtimeID == "" && len(config.runtimes) > 0 {
		t.runtimeID = config.runtimes[0].Id
	}
	v := &conversation{
		core: core, win: win, device: d, target: t,
		configuration: config, openSettings: openSettings,
		next:           t.runtimeID,
		intent:         map[string]bindings.BindingTurnChoice{},
		runtimeChoices: map[string]bindings.BindingRuntimeComposerChoices{},
		catalogs:       map[string]*catalog{},
		payloads:       map[string]bindings.BindingMessage{},
		expanded:       map[string]bool{},
		answers:        map[string]string{},
		chosen:         map[string]string{},
		loading:        t.conversationID != "",
	}
	v.list.Key = func(i int) any { return v.messages[i].Id }
	v.list.FollowEnd = true
	v.loadChoices()
	v.loadCatalog(v.next, false)
	v.load()
	go v.poll()
	return v
}

// unsent tells whether the conversation is still to be started.
func (v *conversation) unsent() bool { return v.target.conversationID == "" }

// close stops the view's polling; the conversation itself goes on.
func (v *conversation) close() { v.closed.Store(true) }

func (v *conversation) title() string {
	switch {
	case v.snapshot != nil:
		return v.snapshot.Conversation.Title
	case v.target.conversationID != "":
		return v.target.title
	default:
		return "新会话"
	}
}

func (v *conversation) running() bool {
	return v.snapshot != nil && (v.snapshot.RunState == bindings.BindingRunStateRunning || v.snapshot.RunState == bindings.BindingRunStateStopping)
}

func (v *conversation) load() {
	if v.unsent() {
		return
	}
	t := v.target
	run(v.win, func() result {
		application, err := v.core.application(v.device)
		if err != nil {
			return result{err: err}
		}
		read, err := application.ReadConversation(t.runtimeID, t.conversationID)
		return result{snapshot: read.Snapshot, err: err}
	}, func(r result) {
		v.loading = false
		v.apply(r)
	})
}

type result struct {
	snapshot *bindings.BindingConversationSnapshot
	err      error
}

// poll follows the conversation: quickly while it runs, slowly otherwise.
func (v *conversation) poll() {
	for {
		v.pollMu.Lock()
		polled := v.polled
		v.pollMu.Unlock()
		delay := 3 * time.Second
		if polled != nil && polled.running {
			delay = 700 * time.Millisecond
		}
		time.Sleep(delay)
		if v.closed.Load() {
			return
		}
		if polled == nil {
			continue
		}
		next, err := v.update(polled)
		if next == nil && err == nil {
			continue
		}
		v.win.Update(func() {
			if v.closed.Load() {
				return
			}
			if err != nil {
				v.failure = describe(err)
			} else {
				v.merge(*next)
			}
		})
	}
}

func (v *conversation) update(polled *polledConversation) (*bindings.BindingConversationSnapshot, error) {
	application, err := v.core.application(v.device)
	if err != nil {
		return nil, err
	}
	update, err := application.SnapshotUpdate(polled.runtimeID, polled.conversationID, polled.cursor)
	if err != nil {
		return nil, err
	}
	switch update := update.(type) {
	case bindings.BindingSnapshotUpdateChanged:
		return &update.Snapshot, nil
	case bindings.BindingSnapshotUpdateAbsent:
		// The device let the conversation go (it restarted, for one); read it again.
		read, err := application.ReadConversation(polled.runtimeID, polled.conversationID)
		return read.Snapshot, err
	default:
		return nil, nil
	}
}

func (v *conversation) apply(r result) {
	if r.err != nil {
		v.failure = describe(r.err)
		return
	}
	v.failure = ""
	if r.snapshot != nil {
		v.merge(*r.snapshot)
	}
}

// merge takes a snapshot in: one with a base holds only changed messages,
// and its message IDs give the whole order.
func (v *conversation) merge(snapshot bindings.BindingConversationSnapshot) {
	messages := snapshot.Messages
	if snapshot.Base != nil {
		known := make(map[string]bindings.BindingMessage, len(v.messages))
		for _, message := range v.messages {
			known[message.Id] = message
		}
		for _, message := range snapshot.Messages {
			known[message.Id] = message
		}
		messages = make([]bindings.BindingMessage, 0, len(snapshot.MessageIds))
		for _, id := range snapshot.MessageIds {
			if message, ok := known[id]; ok {
				messages = append(messages, message)
			}
		}
	}
	for _, message := range messages {
		// A message that changed replaces the payload read for its earlier content.
		if loaded, ok := v.payloads[message.Id]; ok && loaded.Stamp != message.Stamp {
			delete(v.payloads, message.Id)
		}
	}
	v.messages = messages
	v.target = target{runtimeID: snapshot.ContextRuntimeId, conversationID: snapshot.Conversation.Id, title: snapshot.Conversation.Title}
	v.snapshot = &snapshot
	if !v.nextChosen {
		v.followCurrentRuntime()
	}
	v.pollMu.Lock()
	v.polled = &polledConversation{runtimeID: snapshot.ContextRuntimeId, conversationID: snapshot.Conversation.Id, cursor: snapshot.Cursor, running: v.running()}
	v.pollMu.Unlock()
}

// act runs one write at a time and shows its failure.
func (v *conversation) act(work func(*bindings.VeluneApplication, string, string) (bindings.BindingSnapshotResult, error)) {
	if v.busy || v.snapshot == nil {
		return
	}
	v.busy, v.failure = true, ""
	runtimeID, id := v.snapshot.ContextRuntimeId, v.snapshot.Conversation.Id
	run(v.win, func() result {
		application, err := v.core.application(v.device)
		if err != nil {
			return result{err: err}
		}
		done, err := work(application, runtimeID, id)
		return result{snapshot: done.Snapshot, err: err}
	}, func(r result) {
		v.busy = false
		v.apply(r)
	})
}

func (v *conversation) send() {
	text := strings.TrimSpace(v.composer)
	if v.busy || text == "" || (v.snapshot != nil && !v.snapshot.Actions.CanSend) {
		return
	}
	v.composer = ""
	// Without a level or mode the core keeps the conversation's current one.
	turn := bindings.BindingComposedTurn{Text: text, Effort: v.effort()}
	if mode := v.permissionMode(v.choice().PermissionMode); mode != nil {
		turn.PermissionMode = &mode.Id
	}
	selection := v.nextModel()
	var existing *target
	if v.snapshot != nil {
		existing = &target{runtimeID: v.snapshot.ContextRuntimeId, conversationID: v.snapshot.Conversation.Id}
	}
	runtimeID := v.next
	var cwd *string
	if path := strings.TrimSpace(v.cwd); v.unsent() && path != "" {
		cwd = &path
	}
	v.busy, v.failure = true, ""
	type sent struct {
		created *bindings.BindingConversationSnapshot
		sent    result
		err     error
	}
	run(v.win, func() sent {
		application, err := v.core.application(v.device)
		if err != nil {
			return sent{err: err}
		}
		var created *bindings.BindingConversationSnapshot
		if existing == nil {
			result, err := application.CreateConversationWithSelection(runtimeID, cwd, selection)
			if err != nil {
				return sent{err: err}
			}
			if result.Snapshot == nil {
				return sent{err: errors.New("设备没有返回新会话")}
			}
			created = result.Snapshot
			existing = &target{runtimeID: created.ContextRuntimeId, conversationID: created.Conversation.Id}
		}
		done, err := application.SendTurnWithSelection(existing.runtimeID, existing.conversationID, runtimeID, selection, turn)
		return sent{created: created, sent: result{snapshot: done.Snapshot, err: err}}
	}, func(s sent) {
		v.busy = false
		if s.err != nil {
			v.composer, v.failure = text, describe(s.err)
			return
		}
		if s.created != nil {
			v.merge(*s.created)
			// The choices made before the first message are the created conversation's.
			v.saveIntent()
		}
		if s.sent.err != nil {
			v.composer = text
		}
		v.apply(s.sent)
	})
}

func (v *conversation) stop() {
	v.act(func(application *bindings.VeluneApplication, runtimeID, id string) (bindings.BindingSnapshotResult, error) {
		return application.Cancel(runtimeID, id)
	})
}

func (v *conversation) reply(interaction string, reply bindings.BindingRuntimeInteractionReply) {
	v.act(func(application *bindings.VeluneApplication, runtimeID, id string) (bindings.BindingSnapshotResult, error) {
		return application.ReplyRuntimeInteraction(runtimeID, id, interaction, reply)
	})
}

// loadPayload reads a message's whole content: large reasoning and tool
// output arrive by size only.
func (v *conversation) loadPayload(messageID string) {
	if v.snapshot == nil {
		return
	}
	runtimeID, id := v.snapshot.ContextRuntimeId, v.snapshot.Conversation.Id
	type loaded struct {
		messages []bindings.BindingMessage
		err      error
	}
	run(v.win, func() loaded {
		application, err := v.core.application(v.device)
		if err != nil {
			return loaded{err: err}
		}
		messages, err := application.MessagePayloads(runtimeID, id, []string{messageID})
		return loaded{messages, err}
	}, func(l loaded) {
		if l.err != nil {
			v.failure = describe(l.err)
			return
		}
		for _, message := range l.messages {
			v.payloads[message.Id] = message
		}
	})
}

// followCurrentRuntime points the next turn at the runtime the conversation
// last ran on.
func (v *conversation) followCurrentRuntime() {
	next := v.snapshot.ContextRuntimeId
	if turns := v.snapshot.TranscriptTurns; len(turns) > 0 {
		if settings := turns[len(turns)-1].Settings; settings != nil && settings.RuntimeId != nil && v.configuration.runtime(*settings.RuntimeId) != nil {
			next = *settings.RuntimeId
		}
	}
	if next != v.next {
		v.next = next
		v.loadCatalog(next, false)
	}
}

func (v *conversation) browse() {
	go func() {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{Parent: v.win, Title: "选择新会话的工作目录", Directory: true})
		if err != nil || len(paths) == 0 {
			return
		}
		v.win.Update(func() { v.cwd = paths[0] })
	}()
}

func (v *conversation) view(c *ui.Context) {
	t := c.Theme()
	var interaction *bindings.BindingRuntimeInteraction
	if v.snapshot != nil && len(v.snapshot.PendingInteractions) > 0 {
		interaction = &v.snapshot.PendingInteractions[0]
	}
	v.prepareAnswers(interaction)
	ui.Column(c).Fill().Children(func() {
		switch {
		case v.loading:
			ui.Column(c).Grow(1).Center().Children(func() { ui.Spinner(c) })
		case len(v.messages) == 0:
			empty := "还没有消息。"
			if v.snapshot == nil {
				empty = "输入第一条消息开始会话。"
			}
			ui.Column(c).Grow(1).Center().Children(func() { ui.Text(c, empty).TextColor(t.TextMuted) })
		default:
			ui.List(c, &v.list, len(v.messages), func(i int) { v.message(c, i) }).Grow(1).PaddingY(8)
		}
		if v.failure != "" {
			ui.Text(c, v.failure).TextColor(t.Danger).FontSize(13).Padding(4, 24)
		}
		if interaction != nil {
			v.interaction(c, *interaction)
		}
		v.composerView(c)
	})
	v.cwdDialog(c)
}

// message lays out the user's messages on the right, the model's on the
// left, and the system's and the Harness's in the middle.
func (v *conversation) message(c *ui.Context, i int) {
	t := c.Theme()
	message := v.messages[i]
	if loaded, ok := v.payloads[message.Id]; ok {
		message = loaded
	}
	blocks := func() {
		for position, block := range message.Blocks {
			v.block(c, message, position, block)
		}
	}
	ui.Row(c).Padding(8, 24).Children(func() {
		switch message.Role {
		case bindings.BindingMessageRoleUser:
			ui.Spacer(c)
			ui.Column(c).MaxWidth(560).Gap(8).Padding(8, 12).Radius(12).Background(t.Surface).Children(blocks)
		case bindings.BindingMessageRoleAssistant:
			ui.Column(c).Grow(1).MinWidth(0).MaxWidth(820).Gap(8).Children(blocks)
		default:
			ui.Column(c).Grow(1).MinWidth(0).MaxWidth(820).Gap(4).Children(blocks)
		}
	})
}

func (v *conversation) block(c *ui.Context, message bindings.BindingMessage, position int, block bindings.BindingMessageBlock) {
	t := c.Theme()
	key := fmt.Sprintf("%s/%d", message.Id, position)
	muted := func(text string) { ui.Text(c, text).FontSize(13).TextColor(t.TextMuted).Selectable() }
	switch block := block.(type) {
	case bindings.BindingMessageBlockText:
		switch {
		case message.Role == bindings.BindingMessageRoleUser:
			ui.Text(c, block.Text).Selectable()
		case message.Completed:
			markdown.View(c, block.Text, &v.md)
		default:
			markdown.Stream(c, block.Text, &v.md)
		}
	case bindings.BindingMessageBlockNotice:
		ui.Row(c).Children(func() {
			ui.Spacer(c)
			muted(block.Text)
			ui.Spacer(c)
		})
	case bindings.BindingMessageBlockReasoning:
		title := strings.TrimSpace(block.Summary)
		if title == "" {
			title = "思考"
		}
		v.disclosure(c, message.Id, key, title, block.Text)
	case bindings.BindingMessageBlockTool:
		suffix := map[bindings.BindingToolState]string{
			bindings.BindingToolStatePending: " · 等待",
			bindings.BindingToolStateRunning: " · 运行中",
			bindings.BindingToolStateFailed:  " · 失败",
		}[block.State]
		v.disclosure(c, message.Id, key, block.Title+suffix, block.Output)
	case bindings.BindingMessageBlockImage:
		muted("［图片］")
	case bindings.BindingMessageBlockFile:
		muted("［文件］" + filepath.Base(strings.ReplaceAll(block.Path, `\`, "/")))
	case bindings.BindingMessageBlockAgentSender:
		muted(fmt.Sprintf("来自 %s · %s", block.RuntimeName, block.Title))
	}
}

// disclosure keeps reasoning and tool output folded; large ones are read
// when unfolded.
func (v *conversation) disclosure(c *ui.Context, messageID, key, title string, payload bindings.BindingMessagePayload) {
	t := c.Theme()
	open := v.expanded[key]
	arrow := "▸"
	if open {
		arrow = "▾"
	}
	header := ui.Text(c.Key(key), arrow+" "+title).FontSize(13).TextColor(t.TextMuted).Cursor(ui.CursorPointer)
	if header.Clicked() {
		open = !open
		v.expanded[key] = open
		if _, deferred := payload.(bindings.BindingMessagePayloadDeferred); open && deferred {
			v.loadPayload(messageID)
		}
	}
	if !open {
		return
	}
	switch payload := payload.(type) {
	case bindings.BindingMessagePayloadInline:
		ui.Text(c, payload.Text).Selectable().Font(monoFamily()).FontSize(12).Padding(8).Radius(6).Background(t.Surface)
	case bindings.BindingMessagePayloadDeferred:
		ui.Spinner(c)
	}
}

func monoFamily() string {
	if runtime.GOOS == "windows" {
		return "Consolas"
	}
	return "Menlo"
}

// prepareAnswers resets the replies when another interaction shows.
func (v *conversation) prepareAnswers(interaction *bindings.BindingRuntimeInteraction) {
	id := ""
	if interaction != nil {
		id = interaction.Id
	}
	if id == v.answering {
		return
	}
	v.answering = id
	v.answers, v.chosen = map[string]string{}, map[string]string{}
}

func (v *conversation) interaction(c *ui.Context, interaction bindings.BindingRuntimeInteraction) {
	t := c.Theme()
	ui.Column(c).Margin(8, 16).Padding(16).Gap(12).Radius(8).BorderWidth(1).BorderColor(t.Border).Background(t.Background).Children(func() {
		switch kind := interaction.Kind.(type) {
		case bindings.BindingInteractionKindApproval:
			ui.Text(c, kind.Title).Bold()
			if strings.TrimSpace(kind.Detail) != "" {
				ui.Scroll(c).MaxHeight(180).Radius(6).Background(t.Surface).Children(func() {
					ui.Text(c, kind.Detail).Selectable().Font(monoFamily()).FontSize(12).Padding(8)
				})
			}
			ui.Row(c).Gap(8).Wrap().Children(func() {
				for _, option := range kind.Options {
					if ui.Button(c.Key("option-"+option.Id), option.Label).Disabled(v.busy).Clicked() {
						v.reply(interaction.Id, bindings.BindingRuntimeInteractionReplyDecision{OptionId: option.Id})
					}
				}
			})
		case bindings.BindingInteractionKindUserInput:
			v.questions(c, interaction.Id, kind.Questions)
		}
	})
}

func (v *conversation) questions(c *ui.Context, interactionID string, questions []bindings.BindingInteractionQuestion) {
	complete := true
	for _, question := range questions {
		ui.Text(c, question.Text).Bold()
		if len(question.Options) == 0 {
			answer := v.answers[question.Id]
			input := ui.TextInput(c.Key("answer-"+question.Id), &answer)
			if question.Secret {
				input.Password()
			}
			v.answers[question.Id] = answer
			complete = complete && strings.TrimSpace(answer) != ""
			continue
		}
		chosen, ok := v.chosen[question.Id]
		complete = complete && ok
		ui.Row(c).Gap(8).Wrap().Children(func() {
			for _, option := range question.Options {
				key := c.Key("answer-" + question.Id + "-" + option.Id)
				var button ui.Element
				if ok && chosen == option.Label {
					button = ui.PrimaryButton(key, option.Label)
				} else {
					button = ui.Button(key, option.Label)
				}
				if button.Clicked() {
					v.chosen[question.Id] = option.Label
				}
			}
		})
	}
	ui.Row(c).Gap(8).Children(func() {
		if ui.PrimaryButton(c, "提交").Disabled(v.busy || !complete).Clicked() {
			answers := make([]bindings.BindingInteractionAnswer, 0, len(questions))
			for _, question := range questions {
				value, ok := v.chosen[question.Id]
				if !ok {
					value = v.answers[question.Id]
				}
				answers = append(answers, bindings.BindingInteractionAnswer{QuestionId: question.Id, Values: []string{value}})
			}
			v.reply(interactionID, bindings.BindingRuntimeInteractionReplyAnswers{Answers: answers})
		}
		if ui.Button(c, "取消").Disabled(v.busy).Clicked() {
			v.reply(interactionID, bindings.BindingRuntimeInteractionReplyCancel{})
		}
	})
}

func sameSelection(a, b bindings.BindingModelSelection) bool {
	switch a := a.(type) {
	case bindings.BindingModelSelectionManaged:
		b, ok := b.(bindings.BindingModelSelectionManaged)
		return ok && a.ModelRecordKey == b.ModelRecordKey
	case bindings.BindingModelSelectionNative:
		b, ok := b.(bindings.BindingModelSelectionNative)
		return ok && a.ModelId == b.ModelId && (a.ProviderId == nil) == (b.ProviderId == nil) && (a.ProviderId == nil || *a.ProviderId == *b.ProviderId)
	default:
		return a == nil && b == nil
	}
}
