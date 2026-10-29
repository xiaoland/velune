namespace Velune;

// Where a conversation view points: an existing conversation, or, with no
// conversation ID, one to start on a runtime.
sealed record Target(string RuntimeID, string ConversationID, string Title)
{
    public bool Unsent => ConversationID.Length == 0;
}

// A runtime's models as read for the composer.
sealed record Catalog(RuntimeModel[] Models, bool Loading, string Failure)
{
    public static readonly Catalog Reading = new([], true, "");
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
    public Configuration Configuration
    {
        get => configuration;
        set
        {
            configuration = value;
            nextTurn = null;
        }
    }
    Configuration configuration;

    public DeliveredSnapshot? Snapshot { get; private set; }
    public List<DeliveredMessage> Messages { get; private set; } = [];
    // Deferred contents read on request, by message ID.
    public Dictionary<string, DeliveredMessage> Payloads { get; } = [];

    // Next is the runtime the next turn runs on, which may differ from the
    // conversation's; it follows the conversation's current one until the
    // user picks another. intent holds this conversation's choices by
    // runtime, runtimeChoices what each runtime's new conversations start with.
    public string Next { get; private set; }
    bool nextChosen;
    string? intentRuntime;
    readonly Dictionary<string, TurnChoice> intent = [];
    readonly Dictionary<string, RuntimeComposerChoices> runtimeChoices = [];
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
    readonly Dictionary<string, Task<AttachmentContent>> attachments = [];
    public bool Loading { get; private set; }
    // ReadKey names this conversation's read among the problems.
    string ReadKey => Problems.Key("conversation", Device, Target.ConversationID);
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
        nextTurn = null;
        if (!closed)
        {
            Changed?.Invoke();
        }
    }

    // Call runs a blocking call on the thread pool against the device.
    Task<T> Call<T>(NativeOperation operation, Func<VeluneApplication, T> work) => core.Call(Device, operation, work);

    public string Title => Snapshot?.Conversation.Title.DisplayText() ?? (Target.Unsent ? "新会话" : Target.Title);

    public bool Running => Snapshot is { RunState: RunState.Running or RunState.Stopping };

    // SessionOpen tells the device holds this conversation's runtime session:
    // it runs or waits, or this view sent to it. Reading a conversation
    // opens none, and what only a session knows, such as an ACP Agent's
    // options, waits for one.
    public bool SessionOpen => sent || Running || Interaction != null;
    bool sent;

    public bool CanCancel => !stopRequested && Snapshot is { RunState: RunState.Running, Actions.CanCancel: true };

    // Working includes a turn the runtime runs for another client: running is
    // what matters, not where it started.
    public bool Working => Running || Snapshot is { RunState: RunState.Elsewhere };

    // A native sub-agent's conversation continues from the one that started it.
    public string? ParentID => Snapshot?.Conversation.ParentConversationId;

    int UserMessages() => Messages.Count(message => message.Role == MessageRole.User);

    public RuntimeInteraction? Interaction => Snapshot?.PendingInteractions.FirstOrDefault();

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
            var read = await Call(NativeOperation.ConversationRead, application => application.ReadConversation(target.RuntimeID, target.ConversationID));
            Apply(read.Snapshot);
            Problems.Recovered(ReadKey);
        }
        catch (Exception failure)
        {
            Problems.Failing(ReadKey, "读取会话「" + Title + "」", failure, Device);
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
                var update = await Call(NativeOperation.ConversationRead, application => Update(application, snapshot));
                if (update != null && !closed)
                {
                    Merge(update);
                    Notify();
                }
                Problems.Recovered(ReadKey);
            }
            catch (Exception failure)
            {
                Problems.Failing(ReadKey, "读取会话「" + Title + "」", failure, Device);
            }
        }
    }

    static DeliveredSnapshot? Update(VeluneApplication application, DeliveredSnapshot snapshot) =>
        application.SnapshotUpdate(snapshot.ContextRuntimeId, snapshot.Conversation.Id, snapshot.Cursor) switch
        {
            SnapshotUpdate.Changed changed => changed.Snapshot,
            // The device let the conversation go (it restarted, for one); read it again.
            SnapshotUpdate.Absent => application.ReadConversation(snapshot.ContextRuntimeId, snapshot.Conversation.Id).Snapshot,
            _ => null,
        };

    void Apply(DeliveredSnapshot? snapshot)
    {
        if (snapshot != null)
        {
            Merge(snapshot);
        }
    }

    // Merge takes a snapshot in: one with a base holds only changed messages,
    // and its message IDs give the whole order.
    public void Merge(DeliveredSnapshot snapshot)
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
        nextTurn = null;
        Target = new Target(snapshot.ContextRuntimeId, snapshot.Conversation.Id, snapshot.Conversation.Title.DisplayText());
        Snapshot = snapshot;
        if (snapshot.RunState is not (RunState.Running or RunState.Stopping)) stopRequested = false;
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

    // Act runs one write at a time; a failure goes to 问题 under source.
    async void Act(string source, NativeOperation operation, Func<VeluneApplication, string, string, SnapshotResult> work)
    {
        if (Busy || Snapshot is not { } snapshot)
        {
            return;
        }
        Busy = true;
        Notify();
        var (runtimeID, id) = (snapshot.ContextRuntimeId, snapshot.Conversation.Id);
        try
        {
            Apply((await Call(operation, application => work(application, runtimeID, id))).Snapshot);
        }
        catch (Exception failure)
        {
            Problems.Report(source, failure, Device);
        }
        Busy = false;
        Notify();
    }

    // Send sends text and the attachments as the next turn, or queues them
    // while a turn runs; false keeps them in the composer.
    public async Task<bool> Send(string text)
    {
        text = text.Trim();
        if (Busy || composing || stopRequested || Snapshot?.RunState == RunState.Stopping || Loading || Uploading > 0 || (text.Length == 0 && Attached.Count == 0) || Next.Length == 0)
        {
            return false;
        }
        if (Running)
        {
            return await Queue(text);
        }
        if (Working || activityRunning || Snapshot is { Actions.CanSend: false } || Interaction != null)
        {
            return false;
        }
        var turn = Turn(text);
        var sentAttachments = Attached.ToList();
        Attached.Clear();
        var selection = NextTurn.Model;
        var existing = Snapshot is { } snapshot ? (snapshot.ContextRuntimeId, snapshot.Conversation.Id) : default((string, string)?);
        var runtimeID = Next;
        var cwd = Target.Unsent && Cwd.Trim().Length > 0 ? Cwd.Trim() : null;
        Busy = true;
        composerRequest = ComposerSubmission.Phase.Sending;
        Sending = text;
        Accepted = false;
        usersBeforeSending = UserMessages();
        Notify();
        DeliveredSnapshot? created = null;
        try
        {
            var sent = await Call(NativeOperation.Send, application =>
            {
                if (existing == null)
                {
                    created = application.CreateConversation(runtimeID, cwd, selection).Snapshot
                        ?? throw new InvalidOperationException("设备没有返回新会话");
                    existing = (created.ContextRuntimeId, created.Conversation.Id);
                }
                var (source, id) = existing.Value;
                return application.SendTurn(source, id, runtimeID, selection, turn);
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
            Problems.Report("发送消息", failure, Device);
            return false;
        }
        finally
        {
            Busy = false;
            composerRequest = null;
            Notify();
        }
    }

    public async void Stop()
    {
        if (!CanCancel || Busy || Snapshot is not { } snapshot) return;
        stopRequested = true;
        Notify();
        if (!await Change("停止", NativeOperation.Cancel, application => application.Cancel(snapshot.ContextRuntimeId, snapshot.Conversation.Id)))
        {
            stopRequested = false;
            Notify();
        }
    }

    public void Reply(string interaction, RuntimeInteractionReply reply) =>
        Act("回复运行时的请求", NativeOperation.Reply, (application, runtimeID, id) => application.ReplyRuntimeInteraction(runtimeID, id, interaction, reply));

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
            var messages = await Call(NativeOperation.ConversationRead, application => application.MessagePayloads(snapshot.ContextRuntimeId, snapshot.Conversation.Id, [messageID]));
            foreach (var message in messages)
            {
                Payloads[message.Id] = message;
            }
        }
        catch (Exception failure)
        {
            Problems.Report("读取消息内容", failure, Device);
        }
        Notify();
    }

    // ReadAttachment reads an attached image of a message, once.
    public Task<AttachmentContent> ReadAttachment(string messageID, int block)
    {
        var key = $"{messageID}/{block}";
        if (!attachments.TryGetValue(key, out var reading) || reading.IsFaulted)
        {
            var (runtimeID, id) = (Snapshot!.ContextRuntimeId, Snapshot.Conversation.Id);
            reading = Call(NativeOperation.ConversationRead, application => application.ReadAttachment(runtimeID, id, messageID, (uint)block));
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

    public Catalog? NextCatalog => catalogs.GetValueOrDefault(Next);

    // NextTurn is what the next turn goes with and what the composer offers
    // for it, as the core resolves them from this conversation's choices and
    // the runtime's catalog. It is resolved again after any change.
    public NextTurn NextTurn => nextTurn ??= VeluneApplicationApiMethods.ResolveNextTurn(new NextTurnInput(
        Next,
        Configuration.RuntimeType(Next),
        Configuration.Providers(Next),
        NextCatalog is { Loading: false, Failure: "" } read ? read.Models : null,
        Snapshot?.TranscriptTurns ?? [],
        Snapshot?.Conversation.RuntimeId,
        Snapshot?.ModelRecordKey,
        intent.TryGetValue(Next, out var kept) ? kept : null,
        runtimeChoices.GetValueOrDefault(Next)));
    NextTurn? nextTurn;

    public string ModelName() => NextTurn.Model == null ? "运行时默认" : NextTurn.ModelName ?? "选择模型";

    // Choose changes a part of this conversation's choice for the next
    // turn's runtime; the core tells when it also becomes what the runtime
    // offers first next time.
    public void Choose(TurnChoiceChange change)
    {
        if (VeluneApplicationApiMethods.ChangeNextTurn(NextTurn, runtimeChoices.GetValueOrDefault(Next), change) is not { } update)
        {
            return;
        }
        intent[Next] = update.Choice;
        SaveIntent();
        if (update.RuntimeChoices is { } choices)
        {
            runtimeChoices[Next] = choices;
            Write("保存运行时的选择", NativeOperation.ConfigurationWrite, application => application.SetRuntimeComposerChoices(choices));
        }
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
        var saved = new NextTurnIntent(Target.ConversationID, intentRuntime,
            [.. intent.OrderBy(entry => entry.Key, StringComparer.Ordinal).Select(entry => new RuntimeTurnChoice(entry.Key, entry.Value))]);
        Write("保存下一轮的选择", NativeOperation.ConfigurationWrite, application => application.SetNextTurnIntent(saved));
    }

    // LoadChoices reads what the device keeps of the composer's choices.
    async void LoadChoices()
    {
        try
        {
            var choices = await Call(NativeOperation.ConfigurationRead, application => application.ComposerChoices());
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
            Problems.Report("读取输入区的选择", failure, Device);
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
            catalogs[runtimeID] = new Catalog(await Call(NativeOperation.ConfigurationRead, application => application.RuntimeModels(runtimeID)), false, "");
        }
        catch (Exception failure)
        {
            // The model menu says why it lists none and reads again on
            // request; a catalog busy with a running turn is no problem.
            catalogs[runtimeID] = new Catalog([], false, Core.Describe(failure));
        }
        Notify();
    }

    // Write saves a choice on the device; a failure goes to 问题 under source.
    async void Write(string source, NativeOperation operation, Action<VeluneApplication> work)
    {
        try
        {
            await Call(operation, application =>
            {
                work(application);
                return true;
            });
        }
        catch (Exception failure)
        {
            Problems.Report(source, failure, Device);
        }
    }
}
