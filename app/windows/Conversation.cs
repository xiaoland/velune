using Velune.Bindings;

namespace Velune;

// Where a conversation view points: an existing conversation, or, with no
// conversation ID, one to start on a runtime.
sealed record Target(string RuntimeID, string ConversationID, string Title)
{
    public bool Unsent => ConversationID.Length == 0;
}

// A runtime's models as read for the composer.
sealed record Catalog(BindingRuntimeModel[] Models, bool Loading, string Failure)
{
    public static readonly Catalog Reading = new([], true, "");
}

// A model the composer offers.
sealed record ModelChoice(BindingModelSelection Selection, string Name, string[] Efforts)
{
    public bool Managed => Selection is BindingModelSelection.Managed;
}

// One open conversation on a device: its transcript, pending interaction and
// next turn's choices. It changes on the UI thread only; Changed tells the
// view to show it again.
sealed partial class Conversation
{
    readonly Core core;
    public Device Device { get; }
    public Target Target { get; private set; }
    // The device's, kept current by the main window.
    public Configuration Configuration { get; set; }

    public BindingConversationSnapshot? Snapshot { get; private set; }
    public List<BindingMessage> Messages { get; private set; } = [];
    // Deferred contents read on request, by message ID.
    public Dictionary<string, BindingMessage> Payloads { get; } = [];

    // Next is the runtime the next turn runs on, which may differ from the
    // conversation's; it follows the conversation's current one until the
    // user picks another. intent holds this conversation's choices by
    // runtime, runtimeChoices what each runtime's new conversations start with.
    public string Next { get; private set; }
    bool nextChosen;
    string? intentRuntime;
    readonly Dictionary<string, BindingTurnChoice> intent = [];
    readonly Dictionary<string, BindingRuntimeComposerChoices> runtimeChoices = [];
    readonly Dictionary<string, Catalog> catalogs = [];
    // Where an unsent conversation will work; empty is the runtime's default.
    public string Cwd { get; set; } = "";

    public bool Busy { get; private set; }
    // Sending is the text of a message sent but not yet in the transcript;
    // Accepted tells the runtime took it.
    public string? Sending { get; private set; }
    public bool Accepted { get; private set; }
    int usersBeforeSending;
    // Attached images read for showing, by message and block.
    readonly Dictionary<string, Task<BindingAttachmentContent>> attachments = [];
    public bool Loading { get; private set; }
    public string Failure { get; private set; } = "";
    bool closed;

    public event Action? Changed;

    // An unsent conversation has no conversation ID; its first message starts it.
    public Conversation(Core core, Device device, Target target, Configuration configuration)
    {
        if (target.Unsent && target.RuntimeID.Length == 0 && configuration.Runtimes.Count > 0)
        {
            target = target with { RuntimeID = configuration.Runtimes[0].Id };
        }
        this.core = core;
        Device = device;
        Target = target;
        Configuration = configuration;
        Next = target.RuntimeID;
        Loading = !target.Unsent;
        LoadChoices();
        LoadCatalog(Next, false);
        Load();
        Poll();
    }

    // Close stops the view's polling; the conversation itself goes on.
    public void Close() => closed = true;

    void Notify()
    {
        if (!closed)
        {
            Changed?.Invoke();
        }
    }

    // Call runs a blocking call on the thread pool against the device.
    Task<T> Call<T>(Func<VeluneApplication, T> work) => Task.Run(() => work(core.Application(Device)));

    public string Title => Snapshot?.Conversation.Title ?? (Target.Unsent ? "新会话" : Target.Title);

    public bool Running => Snapshot is { RunState: BindingRunState.Running or BindingRunState.Stopping };

    // SessionOpen tells the device holds this conversation's runtime session:
    // it runs or waits, or this view sent to it. Reading a conversation
    // opens none, and what only a session knows, such as an ACP Agent's
    // options, waits for one.
    public bool SessionOpen => sent || Running || Interaction != null;
    bool sent;

    public bool CanCancel => Running && Snapshot!.Actions.CanCancel;

    // Working includes a turn the runtime runs for another client: running is
    // what matters, not where it started.
    public bool Working => Running || Snapshot is { RunState: BindingRunState.Elsewhere };

    // A native sub-agent's conversation continues from the one that started it.
    public string? ParentID => Snapshot?.Conversation.ParentConversationId;

    int UserMessages() => Messages.Count(message => message.Role == BindingMessageRole.User);

    public BindingRuntimeInteraction? Interaction => Snapshot?.PendingInteractions.FirstOrDefault();

    // Load reads the conversation; reading the list again reads it again too.
    public async void Load()
    {
        if (Target.Unsent)
        {
            return;
        }
        var target = Target;
        try
        {
            var read = await Call(application => application.ReadConversation(target.RuntimeID, target.ConversationID));
            Apply(read.Snapshot);
        }
        catch (Exception failure)
        {
            Failure = Core.Describe(failure);
        }
        Loading = false;
        Notify();
    }

    // Poll follows the conversation: quickly while it runs, slowly otherwise.
    async void Poll()
    {
        while (!closed)
        {
            await Task.Delay(Running ? 700 : 3_000);
            if (closed || Snapshot is not { } snapshot)
            {
                continue;
            }
            try
            {
                var update = await Call(application => Update(application, snapshot));
                if (update != null && !closed)
                {
                    Merge(update);
                    Notify();
                }
            }
            catch (Exception failure)
            {
                Failure = Core.Describe(failure);
                Notify();
            }
        }
    }

    static BindingConversationSnapshot? Update(VeluneApplication application, BindingConversationSnapshot snapshot) =>
        application.SnapshotUpdate(snapshot.ContextRuntimeId, snapshot.Conversation.Id, snapshot.Cursor) switch
        {
            BindingSnapshotUpdate.Changed changed => changed.Snapshot,
            // The device let the conversation go (it restarted, for one); read it again.
            BindingSnapshotUpdate.Absent => application.ReadConversation(snapshot.ContextRuntimeId, snapshot.Conversation.Id).Snapshot,
            _ => null,
        };

    void Apply(BindingConversationSnapshot? snapshot)
    {
        Failure = "";
        if (snapshot != null)
        {
            Merge(snapshot);
        }
    }

    // Merge takes a snapshot in: one with a base holds only changed messages,
    // and its message IDs give the whole order.
    public void Merge(BindingConversationSnapshot snapshot)
    {
        var messages = snapshot.Messages.ToList();
        if (snapshot.Base != null)
        {
            var known = Messages.ToDictionary(message => message.Id);
            foreach (var message in snapshot.Messages)
            {
                known[message.Id] = message;
            }
            messages = [.. snapshot.MessageIds.Where(known.ContainsKey).Select(id => known[id])];
        }
        foreach (var message in messages)
        {
            // A message that changed replaces the payload read for its earlier content.
            if (Payloads.TryGetValue(message.Id, out var loaded) && loaded.Stamp != message.Stamp)
            {
                Payloads.Remove(message.Id);
            }
        }
        Messages = messages;
        Target = new Target(snapshot.ContextRuntimeId, snapshot.Conversation.Id, snapshot.Conversation.Title);
        Snapshot = snapshot;
        // The sent message shows until the transcript has it, or its turn ends without it.
        if (Sending != null && Accepted && (UserMessages() > usersBeforeSending || !Working))
        {
            Sending = null;
        }
        if (!nextChosen)
        {
            FollowCurrentRuntime();
        }
    }

    // Act runs one write at a time and shows its failure.
    async void Act(Func<VeluneApplication, string, string, BindingSnapshotResult> work)
    {
        if (Busy || Snapshot is not { } snapshot)
        {
            return;
        }
        Busy = true;
        Failure = "";
        Notify();
        var (runtimeID, id) = (snapshot.ContextRuntimeId, snapshot.Conversation.Id);
        try
        {
            Apply((await Call(application => work(application, runtimeID, id))).Snapshot);
        }
        catch (Exception failure)
        {
            Failure = Core.Describe(failure);
        }
        Busy = false;
        Notify();
    }

    // Send sends text and the attachments as the next turn, or queues them
    // while a turn runs; false keeps them in the composer.
    public async Task<bool> Send(string text)
    {
        text = text.Trim();
        if (Busy || (text.Length == 0 && Attached.Count == 0) || Next.Length == 0)
        {
            return false;
        }
        if (Running)
        {
            return await Queue(text);
        }
        if (Snapshot is { Actions.CanSend: false })
        {
            return false;
        }
        var turn = Turn(text);
        var sentAttachments = Attached.ToList();
        Attached.Clear();
        var selection = NextModel();
        var existing = Snapshot is { } snapshot ? (snapshot.ContextRuntimeId, snapshot.Conversation.Id) : default((string, string)?);
        var runtimeID = Next;
        var cwd = Target.Unsent && Cwd.Trim().Length > 0 ? Cwd.Trim() : null;
        Busy = true;
        Failure = "";
        Sending = text;
        Accepted = false;
        usersBeforeSending = UserMessages();
        Notify();
        BindingConversationSnapshot? created = null;
        try
        {
            var sent = await Call(application =>
            {
                if (existing == null)
                {
                    created = application.CreateConversationWithSelection(runtimeID, cwd, selection).Snapshot
                        ?? throw new InvalidOperationException("设备没有返回新会话");
                    existing = (created.ContextRuntimeId, created.Conversation.Id);
                }
                var (source, id) = existing.Value;
                return application.SendTurnWithSelection(source, id, runtimeID, selection, turn);
            });
            if (created != null)
            {
                Merge(created);
                // The choices made before the first message are the created conversation's.
                SaveIntent();
            }
            Accepted = true;
            this.sent = true;
            Apply(sent.Snapshot);
            return true;
        }
        catch (Exception failure)
        {
            Sending = null;
            // What was sent goes back to the composer, ahead of anything attached since.
            Attached.InsertRange(0, sentAttachments);
            if (created != null)
            {
                Merge(created);
                SaveIntent();
            }
            Failure = Core.Describe(failure);
            return false;
        }
        finally
        {
            Busy = false;
            Notify();
        }
    }

    public void Stop() => Act((application, runtimeID, id) => application.Cancel(runtimeID, id));

    public void Reply(string interaction, BindingRuntimeInteractionReply reply) =>
        Act((application, runtimeID, id) => application.ReplyRuntimeInteraction(runtimeID, id, interaction, reply));

    // LoadPayload reads a message's whole content: large reasoning and tool
    // output arrive by size only.
    public async void LoadPayload(string messageID)
    {
        if (Snapshot is not { } snapshot)
        {
            return;
        }
        try
        {
            var messages = await Call(application => application.MessagePayloads(snapshot.ContextRuntimeId, snapshot.Conversation.Id, [messageID]));
            foreach (var message in messages)
            {
                Payloads[message.Id] = message;
            }
        }
        catch (Exception failure)
        {
            Failure = Core.Describe(failure);
        }
        Notify();
    }

    // ReadAttachment reads an attached image of a message, once.
    public Task<BindingAttachmentContent> ReadAttachment(string messageID, int block)
    {
        var key = $"{messageID}/{block}";
        if (!attachments.TryGetValue(key, out var reading) || reading.IsFaulted)
        {
            var (runtimeID, id) = (Snapshot!.ContextRuntimeId, Snapshot.Conversation.Id);
            reading = Call(application => application.ReadAttachment(runtimeID, id, messageID, (uint)block));
            attachments[key] = reading;
        }
        return reading;
    }

    // FollowCurrentRuntime points the next turn at the runtime the
    // conversation last ran on.
    void FollowCurrentRuntime()
    {
        var next = Snapshot!.ContextRuntimeId;
        if (Snapshot.TranscriptTurns.LastOrDefault()?.Settings?.RuntimeId is { } last && Configuration.Runtime(last) != null)
        {
            next = last;
        }
        if (next != Next)
        {
            Next = next;
            LoadCatalog(next, false);
        }
    }

    // HasTurns tells whether the conversation ran on the runtime, whose
    // choices an unchosen part then follows; otherwise it is the runtime's default.
    public bool HasTurns(string runtimeID) =>
        Snapshot?.TranscriptTurns.Any(turn => turn.Settings?.RuntimeId == runtimeID) ?? false;

    // CurrentSettings are the settings of the conversation's last turn when
    // it ran on the next turn's runtime.
    public BindingTurnSettings? CurrentSettings() =>
        Snapshot?.TranscriptTurns.LastOrDefault()?.Settings is { } settings && settings.RuntimeId == Next ? settings : null;

    // Choice is what this conversation chose for the next turn's runtime. A
    // runtime it has not run on and chose nothing for starts from what the
    // runtime's new conversations start with.
    public BindingTurnChoice Choice()
    {
        if (intent.TryGetValue(Next, out var choice))
        {
            return choice;
        }
        if (HasTurns(Next) || !runtimeChoices.TryGetValue(Next, out var choices))
        {
            return new BindingTurnChoice(null, null, null);
        }
        return choices.NewConversation is BindingNewConversationChoice.Chosen chosen ? chosen.Choice : choices.LastChosen;
    }

    public Catalog? NextCatalog => catalogs.GetValueOrDefault(Next);

    bool CatalogReady => NextCatalog is { Loading: false, Failure: "" };

    List<ModelChoice> CatalogChoices() =>
        NextCatalog?.Models.Select(model => new ModelChoice(model.Selection, model.Name, model.ReasoningEfforts)).ToList() ?? [];

    // ModelChoices are the catalog's models and, first, the conversation's
    // current model when the runtime recorded it by an ID its catalog does
    // not list (Claude Code lists aliases).
    public List<ModelChoice> ModelChoices()
    {
        var choices = CatalogChoices();
        var current = LatestTurnModel(Next);
        if (current is not BindingModelSelection.Native native || !CatalogReady || choices.Any(choice => Equals(choice.Selection, current)))
        {
            return choices;
        }
        return [new ModelChoice(current, native.ModelId + "（会话当前）", DefaultEfforts()), .. choices];
    }

    // LatestTurnModel is the model of the conversation's latest turn on a
    // runtime: the one Velune sent it with, else the catalog entry its
    // record names, else the runtime's own model by the recorded ID.
    BindingModelSelection? LatestTurnModel(string runtimeID)
    {
        var settings = Snapshot?.TranscriptTurns
            .Select(turn => turn.Settings)
            .LastOrDefault(settings => settings?.RuntimeId == runtimeID && (settings.Model != null || settings.ModelSelection != null));
        if (settings == null)
        {
            return null;
        }
        if (settings.ModelSelection != null)
        {
            return settings.ModelSelection;
        }
        var model = settings.Model!;
        var choices = CatalogChoices();
        if (choices.FirstOrDefault(choice => choice.Name == model) is { } named)
        {
            return named.Selection;
        }
        if (choices.FirstOrDefault(choice => choice.Selection is BindingModelSelection.Native { } n && n.ModelId == model) is { } identified)
        {
            return identified.Selection;
        }
        var natives = choices.Select(choice => choice.Selection).OfType<BindingModelSelection.Native>().ToList();
        if (natives.Any(native => native.ProviderId != null) || !CatalogReady || natives.Count == 0)
        {
            return null;
        }
        return new BindingModelSelection.Native(null, model);
    }

    // NextModel is the model the next turn goes with: the one chosen, else
    // the conversation's current one on the runtime, else the one it
    // recorded; none is the runtime's default. A read catalog drops those it
    // no longer offers.
    public BindingModelSelection? NextModel()
    {
        BindingModelSelection? recorded = Snapshot is { } snapshot && snapshot.Conversation.RuntimeId == Next && snapshot.ModelRecordKey is { } key
            ? new BindingModelSelection.Managed(key)
            : null;
        var choices = ModelChoices();
        foreach (var candidate in new[] { Choice().Model, LatestTurnModel(Next), recorded })
        {
            if (candidate != null && (!CatalogReady || choices.Any(choice => Equals(choice.Selection, candidate))))
            {
                return candidate;
            }
        }
        return null;
    }

    public string ModelName()
    {
        var model = NextModel();
        if (model == null)
        {
            return "运行时默认";
        }
        if (ModelChoices().FirstOrDefault(choice => Equals(choice.Selection, model)) is { } choice)
        {
            return choice.Name;
        }
        return model is BindingModelSelection.Native native ? native.ModelId : "选择模型";
    }

    // DefaultEfforts are the levels of the runtime's own default model, which
    // no catalog entry describes: those its own models take, unless its
    // sessions offer them.
    string[] DefaultEfforts()
    {
        if (Configuration.RuntimeType(Next) is { SessionOptions: true })
        {
            return [];
        }
        return [.. CatalogChoices().Where(choice => !choice.Managed).SelectMany(choice => choice.Efforts).Distinct()];
    }

    // EffortOptions are the levels the next turn's model accepts, in the
    // runtime's own vocabulary.
    public string[] EffortOptions()
    {
        var model = NextModel();
        if (model == null)
        {
            return DefaultEfforts();
        }
        return ModelChoices().FirstOrDefault(choice => Equals(choice.Selection, model))?.Efforts ?? [];
    }

    // Effort is the chosen level while the model offers it; none keeps the
    // conversation's current one.
    public string? Effort() => Choice().Effort is { } effort && EffortOptions().Contains(effort) ? effort : null;

    public string? CurrentEffort() =>
        CurrentSettings()?.Effort is { } effort && EffortOptions().Contains(effort) ? effort : null;

    public BindingPermissionMode[] PermissionModes() => Configuration.RuntimeType(Next)?.PermissionModes ?? [];

    public BindingPermissionMode? PermissionMode(string? id) =>
        id == null ? null : PermissionModes().FirstOrDefault(mode => mode.Id == id);

    // Choose changes a part of this conversation's choice for the next turn's
    // runtime. A part chosen, or left to a new conversation's runtime
    // default, is also what the runtime offers first next time.
    public void Choose(bool chosen, Func<BindingTurnChoice, BindingTurnChoice> update)
    {
        intent[Next] = update(Choice());
        SaveIntent();
        if (!chosen && HasTurns(Next))
        {
            Notify();
            return;
        }
        var choices = runtimeChoices.TryGetValue(Next, out var known)
            ? known
            : new BindingRuntimeComposerChoices(Next, new BindingTurnChoice(null, null, null), new BindingNewConversationChoice.LastChosen());
        choices = choices with { LastChosen = update(choices.LastChosen) };
        runtimeChoices[Next] = choices;
        Write(application => application.SetRuntimeComposerChoices(choices));
        Notify();
    }

    public void ChooseRuntime(string id)
    {
        Next = id;
        nextChosen = true;
        intentRuntime = id;
        if (Target.Unsent)
        {
            Target = Target with { RuntimeID = id };
        }
        SaveIntent();
        LoadCatalog(id, false);
        Notify();
    }

    // SaveIntent keeps the conversation's choices; an unsent one's are kept
    // once it is created.
    void SaveIntent()
    {
        if (Target.Unsent)
        {
            return;
        }
        var saved = new BindingNextTurnIntent(Target.ConversationID, intentRuntime,
            [.. intent.OrderBy(entry => entry.Key, StringComparer.Ordinal).Select(entry => new BindingRuntimeTurnChoice(entry.Key, entry.Value))]);
        Write(application => application.SetNextTurnIntent(saved));
    }

    // LoadChoices reads what the device keeps of the composer's choices.
    async void LoadChoices()
    {
        try
        {
            var choices = await Call(application => application.ComposerChoices());
            foreach (var runtime in choices.Runtimes)
            {
                runtimeChoices[runtime.RuntimeId] = runtime;
            }
            foreach (var saved in choices.Conversations.Where(saved => !Target.Unsent && saved.ConversationId == Target.ConversationID))
            {
                intentRuntime = saved.RuntimeId;
                foreach (var choice in saved.Choices)
                {
                    intent.TryAdd(choice.RuntimeId, choice.Choice);
                }
            }
        }
        catch (Exception failure)
        {
            Failure = Core.Describe(failure);
        }
        Notify();
    }

    // LoadCatalog reads a runtime's models once; again only when asked.
    public async void LoadCatalog(string runtimeID, bool again)
    {
        if (runtimeID.Length == 0 || (catalogs.ContainsKey(runtimeID) && !again))
        {
            return;
        }
        catalogs[runtimeID] = Catalog.Reading;
        Notify();
        try
        {
            catalogs[runtimeID] = new Catalog(await Call(application => application.RuntimeModels(runtimeID)), false, "");
        }
        catch (Exception failure)
        {
            catalogs[runtimeID] = new Catalog([], false, Core.Describe(failure));
        }
        Notify();
    }

    // Write saves a choice on the device and shows its failure.
    async void Write(Action<VeluneApplication> work)
    {
        try
        {
            await Call(application =>
            {
                work(application);
                return true;
            });
        }
        catch (Exception failure)
        {
            Failure = Core.Describe(failure);
            Notify();
        }
    }
}
