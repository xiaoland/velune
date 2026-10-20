package main

import (
	"slices"

	"velune/bindings"
)

// configuration is what a device's composer chooses among, from its latest
// listing.
type configuration struct {
	// runtimes are the enabled runtime instances.
	runtimes []bindings.BindingRuntimeInstance
	types    []bindings.BindingRuntimeTypeDescriptor
	// projects are the working directories of the latest conversations,
	// newest first.
	projects []string
}

func (c configuration) runtime(id string) *bindings.BindingRuntimeInstance {
	for i := range c.runtimes {
		if c.runtimes[i].Id == id {
			return &c.runtimes[i]
		}
	}
	return nil
}

func (c configuration) runtimeType(runtimeID string) *bindings.BindingRuntimeTypeDescriptor {
	runtime := c.runtime(runtimeID)
	if runtime == nil {
		return nil
	}
	for i := range c.types {
		if c.types[i].Id == runtime.TypeId {
			return &c.types[i]
		}
	}
	return nil
}

// catalog is a runtime's models as read for the composer.
type catalog struct {
	models  []bindings.BindingRuntimeModel
	loading bool
	failure string
}

// modelChoice is a model the composer offers.
type modelChoice struct {
	selection bindings.BindingModelSelection
	name      string
	efforts   []string
}

func (c modelChoice) managed() bool {
	_, ok := c.selection.(bindings.BindingModelSelectionManaged)
	return ok
}

// hasTurns tells whether the conversation ran on the runtime, whose choices
// an unchosen part then follows; otherwise it is the runtime's default.
func (v *conversation) hasTurns(runtimeID string) bool {
	if v.snapshot == nil {
		return false
	}
	for _, turn := range v.snapshot.TranscriptTurns {
		if turn.Settings != nil && turn.Settings.RuntimeId != nil && *turn.Settings.RuntimeId == runtimeID {
			return true
		}
	}
	return false
}

// currentSettings are the settings of the conversation's last turn when it
// ran on the next turn's runtime.
func (v *conversation) currentSettings() *bindings.BindingTurnSettings {
	if v.snapshot == nil || len(v.snapshot.TranscriptTurns) == 0 {
		return nil
	}
	settings := v.snapshot.TranscriptTurns[len(v.snapshot.TranscriptTurns)-1].Settings
	if settings == nil || settings.RuntimeId == nil || *settings.RuntimeId != v.next {
		return nil
	}
	return settings
}

// choice is what this conversation chose for the next turn's runtime. A
// runtime it has not run on and chose nothing for starts from what the
// runtime's new conversations start with.
func (v *conversation) choice() bindings.BindingTurnChoice {
	if choice, ok := v.intent[v.next]; ok {
		return choice
	}
	if v.hasTurns(v.next) {
		return bindings.BindingTurnChoice{}
	}
	choices, ok := v.runtimeChoices[v.next]
	if !ok {
		return bindings.BindingTurnChoice{}
	}
	if chosen, ok := choices.NewConversation.(bindings.BindingNewConversationChoiceChosen); ok {
		return chosen.Choice
	}
	return choices.LastChosen
}

func (v *conversation) catalogChoices() []modelChoice {
	catalog := v.catalogs[v.next]
	if catalog == nil {
		return nil
	}
	choices := make([]modelChoice, 0, len(catalog.models))
	for _, model := range catalog.models {
		choices = append(choices, modelChoice{selection: model.Selection, name: model.Name, efforts: model.ReasoningEfforts})
	}
	return choices
}

func (v *conversation) catalogReady() bool {
	catalog := v.catalogs[v.next]
	return catalog != nil && !catalog.loading && catalog.failure == ""
}

// modelChoices are the catalog's models and, first, the conversation's
// current model when the runtime recorded it by an ID its catalog does not
// list (Claude Code lists aliases).
func (v *conversation) modelChoices() []modelChoice {
	choices := v.catalogChoices()
	current := v.latestTurnModel(v.next)
	native, ok := current.(bindings.BindingModelSelectionNative)
	if !ok || !v.catalogReady() || slices.ContainsFunc(choices, func(c modelChoice) bool { return sameSelection(c.selection, current) }) {
		return choices
	}
	return append([]modelChoice{{selection: current, name: native.ModelId + "（会话当前）", efforts: v.defaultEfforts()}}, choices...)
}

// latestTurnModel is the model of the conversation's latest turn on a
// runtime: the one Velune sent it with, else the catalog entry its record
// names, else the runtime's own model by the recorded ID.
func (v *conversation) latestTurnModel(runtimeID string) bindings.BindingModelSelection {
	if v.snapshot == nil {
		return nil
	}
	var settings *bindings.BindingTurnSettings
	for _, turn := range v.snapshot.TranscriptTurns {
		s := turn.Settings
		if s != nil && s.RuntimeId != nil && *s.RuntimeId == runtimeID && (s.Model != nil || s.ModelSelection != nil) {
			settings = s
		}
	}
	switch {
	case settings == nil:
		return nil
	case settings.ModelSelection != nil:
		return settings.ModelSelection
	}
	model := *settings.Model
	choices := v.catalogChoices()
	for _, choice := range choices {
		if choice.name == model {
			return choice.selection
		}
	}
	for _, choice := range choices {
		if native, ok := choice.selection.(bindings.BindingModelSelectionNative); ok && native.ModelId == model {
			return choice.selection
		}
	}
	natives := 0
	for _, choice := range choices {
		native, ok := choice.selection.(bindings.BindingModelSelectionNative)
		if ok && native.ProviderId != nil {
			return nil
		}
		if ok {
			natives++
		}
	}
	if !v.catalogReady() || natives == 0 {
		return nil
	}
	return bindings.BindingModelSelectionNative{ModelId: model}
}

// nextModel is the model the next turn goes with: the one chosen, else the
// conversation's current one on the runtime, else the one it recorded; none
// is the runtime's default. A read catalog drops those it no longer offers.
func (v *conversation) nextModel() bindings.BindingModelSelection {
	var recorded bindings.BindingModelSelection
	if v.snapshot != nil && v.snapshot.Conversation.RuntimeId == v.next && v.snapshot.ModelRecordKey != nil {
		recorded = bindings.BindingModelSelectionManaged{ModelRecordKey: *v.snapshot.ModelRecordKey}
	}
	choices := v.modelChoices()
	for _, candidate := range []bindings.BindingModelSelection{v.choice().Model, v.latestTurnModel(v.next), recorded} {
		if candidate == nil {
			continue
		}
		if !v.catalogReady() || slices.ContainsFunc(choices, func(c modelChoice) bool { return sameSelection(c.selection, candidate) }) {
			return candidate
		}
	}
	return nil
}

func (v *conversation) modelName() string {
	model := v.nextModel()
	if model == nil {
		return "运行时默认"
	}
	for _, choice := range v.modelChoices() {
		if sameSelection(choice.selection, model) {
			return choice.name
		}
	}
	if native, ok := model.(bindings.BindingModelSelectionNative); ok {
		return native.ModelId
	}
	return "选择模型"
}

// defaultEfforts are the levels of the runtime's own default model, which no
// catalog entry describes: those its own models take, unless its sessions
// offer them.
func (v *conversation) defaultEfforts() []string {
	if t := v.configuration.runtimeType(v.next); t != nil && t.SessionOptions {
		return nil
	}
	var levels []string
	for _, choice := range v.catalogChoices() {
		if choice.managed() {
			continue
		}
		for _, level := range choice.efforts {
			if !slices.Contains(levels, level) {
				levels = append(levels, level)
			}
		}
	}
	return levels
}

// effortOptions are the levels the next turn's model accepts, in the
// runtime's own vocabulary.
func (v *conversation) effortOptions() []string {
	model := v.nextModel()
	if model == nil {
		return v.defaultEfforts()
	}
	for _, choice := range v.modelChoices() {
		if sameSelection(choice.selection, model) {
			return choice.efforts
		}
	}
	return nil
}

// effort is the chosen level while the model offers it; none keeps the
// conversation's current one.
func (v *conversation) effort() *string {
	if effort := v.choice().Effort; effort != nil && slices.Contains(v.effortOptions(), *effort) {
		return effort
	}
	return nil
}

func (v *conversation) currentEffort() *string {
	if settings := v.currentSettings(); settings != nil && settings.Effort != nil && slices.Contains(v.effortOptions(), *settings.Effort) {
		return settings.Effort
	}
	return nil
}

func (v *conversation) permissionModes() []bindings.BindingPermissionMode {
	if t := v.configuration.runtimeType(v.next); t != nil {
		return t.PermissionModes
	}
	return nil
}

func (v *conversation) permissionMode(id *string) *bindings.BindingPermissionMode {
	if id == nil {
		return nil
	}
	for _, mode := range v.permissionModes() {
		if mode.Id == *id {
			return &mode
		}
	}
	return nil
}

// choose changes a part of this conversation's choice for the next turn's
// runtime. A part chosen, or left to a new conversation's runtime default,
// is also what the runtime offers first next time.
func (v *conversation) choose(chosen bool, update func(*bindings.BindingTurnChoice)) {
	choice := v.choice()
	update(&choice)
	v.intent[v.next] = choice
	v.saveIntent()
	if !chosen && v.hasTurns(v.next) {
		return
	}
	choices, ok := v.runtimeChoices[v.next]
	if !ok {
		choices = bindings.BindingRuntimeComposerChoices{RuntimeId: v.next, NewConversation: bindings.BindingNewConversationChoiceLastChosen{}}
	}
	update(&choices.LastChosen)
	v.runtimeChoices[v.next] = choices
	v.write(func(application *bindings.VeluneApplication) error {
		return application.SetRuntimeComposerChoices(choices)
	})
}

func (v *conversation) chooseRuntime(id string) {
	v.next, v.nextChosen, v.intentRuntime = id, true, &id
	if v.target.conversationID == "" {
		v.target.runtimeID = id
	}
	v.saveIntent()
	v.loadCatalog(id, false)
}

// saveIntent keeps the conversation's choices; an unsent one's are kept once
// it is created.
func (v *conversation) saveIntent() {
	if v.target.conversationID == "" {
		return
	}
	intent := v.nextTurnIntent(v.target.conversationID)
	v.write(func(application *bindings.VeluneApplication) error {
		return application.SetNextTurnIntent(intent)
	})
}

func (v *conversation) nextTurnIntent(conversationID string) bindings.BindingNextTurnIntent {
	intent := bindings.BindingNextTurnIntent{ConversationId: conversationID, RuntimeId: v.intentRuntime}
	for runtimeID, choice := range v.intent {
		intent.Choices = append(intent.Choices, bindings.BindingRuntimeTurnChoice{RuntimeId: runtimeID, Choice: choice})
	}
	slices.SortFunc(intent.Choices, func(a, b bindings.BindingRuntimeTurnChoice) int {
		switch {
		case a.RuntimeId < b.RuntimeId:
			return -1
		case a.RuntimeId > b.RuntimeId:
			return 1
		}
		return 0
	})
	return intent
}

// loadChoices reads what the device keeps of the composer's choices.
func (v *conversation) loadChoices() {
	run(v.win, func() outcome[bindings.BindingComposerChoices] {
		application, err := v.core.application(v.device)
		if err != nil {
			return outcome[bindings.BindingComposerChoices]{err: err}
		}
		choices, err := application.ComposerChoices()
		return outcome[bindings.BindingComposerChoices]{choices, err}
	}, func(r outcome[bindings.BindingComposerChoices]) {
		if r.err != nil {
			v.failure = describe(r.err)
			return
		}
		for _, choices := range r.value.Runtimes {
			v.runtimeChoices[choices.RuntimeId] = choices
		}
		for _, intent := range r.value.Conversations {
			if intent.ConversationId != v.target.conversationID || v.target.conversationID == "" {
				continue
			}
			v.intentRuntime = intent.RuntimeId
			for _, choice := range intent.Choices {
				if _, chosen := v.intent[choice.RuntimeId]; !chosen {
					v.intent[choice.RuntimeId] = choice.Choice
				}
			}
		}
	})
}

// loadCatalog reads a runtime's models once; again only when asked.
func (v *conversation) loadCatalog(runtimeID string, again bool) {
	if runtimeID == "" || (v.catalogs[runtimeID] != nil && !again) {
		return
	}
	v.catalogs[runtimeID] = &catalog{loading: true}
	run(v.win, func() outcome[[]bindings.BindingRuntimeModel] {
		application, err := v.core.application(v.device)
		if err != nil {
			return outcome[[]bindings.BindingRuntimeModel]{err: err}
		}
		models, err := application.RuntimeModels(runtimeID)
		return outcome[[]bindings.BindingRuntimeModel]{models, err}
	}, func(r outcome[[]bindings.BindingRuntimeModel]) {
		if r.err != nil {
			v.catalogs[runtimeID] = &catalog{failure: describe(r.err)}
			return
		}
		v.catalogs[runtimeID] = &catalog{models: r.value}
	})
}

// write saves a choice on the device and shows its failure.
func (v *conversation) write(work func(*bindings.VeluneApplication) error) {
	run(v.win, func() error {
		application, err := v.core.application(v.device)
		if err != nil {
			return err
		}
		return work(application)
	}, func(err error) {
		if err != nil {
			v.failure = describe(err)
		}
	})
}
