namespace Velune;

// An attachment uploaded for the next message: the device's reference to it,
// and its name for the composer.
sealed record Attachment(string Reference, string Name, bool Image, ComposerAttachment? Content = null);

// What follows the next turn's text: its attachments and draft, the send
// queue and steering while a turn runs, continuing after one stopped or
// failed, editing a sent message, and the ACP Agent's session options.
sealed partial class Conversation
{
    static readonly string[] imageExtensions = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".heic"];

    // The attachments the next message carries, in the order they were added.
    public List<Attachment> Attached { get; } = [];
    public int Uploading { get; private set; }
    // The ACP Agent's options for this conversation; empty when it has none.
    public AcpConfigOption[] SessionOptions { get; private set; } = [];
    (string Runtime, string Conversation)? sessionOptionsTarget;

    public FollowUp? FollowUp => Snapshot?.FollowUp;

    // Turn is a next turn of text and the attachments. Without a level or
    // mode the core keeps the conversation's current one.
    ComposedTurn Turn(string text) =>
        new(Text: text, Effort: NextTurn.Effort, Attachments: [.. Attached.Select(attachment => attachment.Reference)], PermissionMode: NextTurn.PermissionMode?.Id);

    // Draft is the text the device keeps for this conversation's composer;
    // an unsent conversation keeps none.
    public async Task<string> Draft() { await PollComposer(); return composerText; }
    public bool SaveDraft(string text)
    {
        composerText = text;
        var saved = ComposerChanged();
        _ = PollComposer();
        return saved;
    }

    // Attach uploads a file to the device that runs the conversation.
    public async Task Attach(string name, byte[] bytes)
    {
        if (!DraftEditable) return;
        Uploading++;
        Notify();
        try
        {
            var uploaded = await Call(NativeOperation.ConversationWrite, application => application.UploadComposerAttachment(name, bytes));
            Attached.Add(new Attachment("composer:" + uploaded.Id, uploaded.Name, imageExtensions.Contains(Path.GetExtension(name).ToLowerInvariant()), uploaded));
            ComposerChanged(); await PollComposer();
        }
        catch (Exception failure)
        {
            Problems.Report("上传附件「" + name + "」", failure, Device);
        }
        Uploading--;
        Notify();
    }

    public void Detach(Attachment attachment)
    {
        if (!DraftEditable) return;
        Attached.Remove(attachment); ComposerChanged(); _ = PollComposer();
        Notify();
    }

    // Queue adds the message to the send queue, which sends it as its own
    // turn after the running one ends.
    async Task<bool> Queue(string text)
    {
        if (Snapshot is not { } snapshot)
        {
            return false;
        }
        var operation = new SendQueueOperation.Add(snapshot.ContextRuntimeId, Next, NextTurn.Model, Turn(text));
        if (!await PrepareComposerSubmission()) return false;
        if (!await Change("更改发送队列", NativeOperation.ConversationWrite, application => application.SendQueue(snapshot.ContextRuntimeId, snapshot.Conversation.Id, operation)))
        {
            return false;
        }
        Attached.Clear();
        Notify();
        return true;
    }

    public string? QueuedEditIssue(string conversationID, string id, ulong revision)
    {
        var current = Snapshot?.Conversation.Id == conversationID ? FollowUp?.Queue.FirstOrDefault(item => item.Id == id) : null;
        if (current == null) return "这条消息已离开发送队列，修改仍保留在这里。";
        return current.EditContext?.Revision != revision ? "这条消息已在别处修改。可复制当前修改，再重新打开编辑。" : null;
    }

    public Task<bool> ReplaceQueued(string conversationID, string id, ulong revision, string runtimeID, TurnChoice choice,
        uint[] retainedAttachments, string text, string[] newAttachments, Action<string> failed)
    {
        if (QueuedEditIssue(conversationID, id, revision) is { } issue)
        {
            failed(issue);
            return Task.FromResult(false);
        }
        var turn = new ComposedTurn(Text: text.Trim(), Effort: choice.Effort, Attachments: newAttachments, PermissionMode: choice.PermissionMode);
        var operation = new SendQueueOperation.Replace(id, revision, runtimeID, choice.Model, retainedAttachments, turn);
        var sourceRuntimeID = Snapshot!.ContextRuntimeId;
        return Change("保存排队消息", NativeOperation.ConversationWrite,
            application => application.SendQueue(sourceRuntimeID, conversationID, operation), problem => failed(Core.Describe(problem)));
    }

    public async Task<Attachment> UploadQueuedAttachment(string name, byte[] bytes)
    {
        var uploaded = await Call(NativeOperation.ConversationWrite, application => application.UploadComposerAttachment(name, bytes));
        return new Attachment("composer:" + uploaded.Id, uploaded.Name, imageExtensions.Contains(Path.GetExtension(name).ToLowerInvariant()), uploaded);
    }

    public Catalog? QueuedEditCatalog(string runtimeID) => catalogs.GetValueOrDefault(runtimeID);

    // The queue owns the captured instruction. Its deliberately absent choices
    // must not inherit new selections made in the ordinary composer.
    public NextTurn ResolveQueuedExecution(string runtimeID, TurnChoice choice) =>
        VeluneApplicationApiMethods.ResolveNextTurn(new NextTurnInput(
            runtimeID, Configuration.RuntimeType(runtimeID), Configuration.Providers(runtimeID),
            new NewConversationChoice.Chosen(choice),
            QueuedEditCatalog(runtimeID) is { Loading: false, Failure: "" } catalog ? catalog.Models : null));

    public void RemoveQueued(string id) => Act("移除排队的消息", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new SendQueueOperation.Remove(id)));

    // MoveQueued moves a message to index in the queue as it is without it.
    public void MoveQueued(string id, int index) => Act("调整发送队列的顺序", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new SendQueueOperation.Move(id, (uint)Math.Max(index, 0))));

    public void SteerQueued(string id) => Act("插入本轮", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new SendQueueOperation.Steer(id)));

    // ResumeQueue sends the queue again after it paused.
    public void ResumeQueue() {
        if (!InputEditable) return;
        Act("继续发送队列", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
            application.SendQueue(runtimeID, conversation, new SendQueueOperation.Resume()));
    }

    public void StopSubAgents() => Act("停止子 Agent", NativeOperation.Cancel, (application, runtimeID, conversation) => application.StopSubAgents(runtimeID, conversation));

    public void SetConversationHandoff(bool release)
    {
        var action = (release, FollowUp?.ControlHandoff) switch
        {
            (true, ControlHandoff.CanRelease) => "释放会话控制权",
            (false, ControlHandoff.Releasing) => "取消交接",
            (false, ControlHandoff.Released) => "恢复 Velune 执行",
            _ => null,
        };
        if (action != null)
        {
            Act(action, NativeOperation.ConversationWrite,
                (application, runtimeID, conversation) => application.SetConversationHandoff(runtimeID, conversation, release));
        }
    }

    // Steer delivers the message into the running turn through the
    // runtime's own channel; false keeps it in the composer.
    public async Task<bool> Steer(string text)
    {
        text = text.Trim();
        if (Snapshot is not { } snapshot || !CanSteer || (text.Length == 0 && Attached.Count == 0))
        {
            return false;
        }
        var target = NextTurn;
        var choice = target.Choice;
        var attachments = Attached.ToArray();
        var turn = new ComposedTurn(Text: text, Effort: null, PermissionMode: choice.PermissionMode,
            Attachments: [.. attachments.Select(attachment => attachment.Reference)]);
        var acceptedWithoutSnapshot = false;
        var accepted = await Change("插入本轮", NativeOperation.Send,
            application => application.SteerTurn(snapshot.ContextRuntimeId, snapshot.Conversation.Id, turn, target.RuntimeId),
            failure => acceptedWithoutSnapshot = failure is BindingException.Diagnostic { code: "message_delivery_accepted" });
        if (!accepted && !acceptedWithoutSnapshot)
        {
            return false;
        }
        Attached.RemoveAll(attachment => attachments.Contains(attachment));
        Notify();
        return true;
    }

    string composerText = "";
    bool composing;
    bool stopRequested;
    bool activityRunning;
    ComposerSubmission.Phase? composerRequest;

    // The activity feed may observe a running turn before this view's snapshot.
    public void ObserveActivity(ActiveConversation? activity)
    {
        var running = activity?.RunState is RunState.Running or RunState.Stopping or RunState.Elsewhere;
        if (activityRunning == running) return;
        activityRunning = running;
        Notify();
    }

    public void UpdateComposer(string text, bool isComposing)
    {
        if (composerText == text && composing == isComposing) return;
        var changed = composerText != text;
        composerText = text;
        composing = isComposing;
        if (changed) ComposerChanged();
        Notify();
    }

    public bool CanSteerQueued => !Loading && !stopRequested && FollowUp is { CanSteer: true };
    public bool CanSteer => CanSteerQueued && !Busy && !composing && Uploading == 0
        && composerLoaded && !composerUnbound && pendingCreation == null && composerRequest == null;

    public ComposerSubmission Submission
    {
        get
        {
            var phase = stopRequested || Snapshot?.RunState == RunState.Stopping ? ComposerSubmission.Phase.Stopping
                : composerRequest ?? (Running ? ComposerSubmission.Phase.Running
                : activityRunning || Loading || Busy ? ComposerSubmission.Phase.Blocked : ComposerSubmission.Phase.Idle);
            var ready = ComposerReady && !Busy && !Loading && Next.Length > 0 && Uploading == 0;
            var canSend = ready && !activityRunning && (Target.Unsent || Snapshot is { Actions.CanSend: true });
            return ComposerSubmission.Resolve(phase, !string.IsNullOrWhiteSpace(composerText) || Attached.Count > 0 || Uploading > 0,
                composing, canSend, ready && !stopRequested && FollowUp is { CanQueue: true },
                ready && FollowUp is { CanResume: true } && Next == Snapshot?.ContextRuntimeId);
        }
    }

    public bool CanCompact => !Busy && !Loading && !activityRunning
        && FollowUp?.CanCompact == true && Next == Snapshot?.ContextRuntimeId;

    public async Task Compact()
    {
        if (!CanCompact || Snapshot is not { } snapshot) return;
        var selection = NextTurn.Model;
        await Change("压缩上下文", NativeOperation.Compact, application => application.CompactConversation(snapshot.ContextRuntimeId, snapshot.Conversation.Id, selection));
    }

    public async Task<bool> Continue()
    {
        if (Snapshot is not { } snapshot || !Submission.CanResume) return false;
        var selection = NextTurn.Model;
        var effort = NextTurn.Effort;
        var mode = NextTurn.PermissionMode?.Id;
        if (!await PrepareComposerSubmission()) return false;
        composerRequest = ComposerSubmission.Phase.Resuming;
        try
        {
            return await Change("继续", NativeOperation.Send, application => application.ResumeTurn(snapshot.ContextRuntimeId, snapshot.Conversation.Id, selection, effort, mode));
        }
        finally
        {
            composerRequest = null;
            Notify();
        }
    }

    // Edit replaces a sent user message: in place, dropping it and all after
    // it, or in a new conversation from what precedes it. It returns the
    // conversation that continues, or null when it did not.
    public async Task<string?> Edit(BindingMessageReference message, string expectedRevision, EditPlacement placement, string text)
    {
        if (Snapshot is not { } snapshot || snapshot.Conversation.Id != message.ConversationId ||
            snapshot.Conversation.RuntimeId != message.RuntimeId)
        {
            return null;
        }
        var turn = new ComposedTurn(Text: text.Trim(), Effort: null, Attachments: [], PermissionMode: null);
        DeliveredSnapshot? result = null;
        var done = await Change("编辑消息", NativeOperation.ConversationWrite, application =>
        {
            var edited = application.EditMessage(message, placement, null, turn, expectedRevision);
            result = edited.Snapshot;
            // A fork's snapshot is the new conversation's, which this one does not take.
            return placement == EditPlacement.Fork ? new SnapshotResult(null) : edited;
        });
        return !done ? null : placement == EditPlacement.Fork ? result?.Conversation.Id : snapshot.Conversation.Id;
    }

    // WorkingChanges reads the conversation's working directory as git sees
    // it, blocking; for the status line's branch.
    public Task<WorkingChanges> WorkingChanges(string runtimeID, string conversationID) =>
        core.Call(Device, NativeOperation.ConversationRead, application => application.WorkingChanges(runtimeID, conversationID));

    // Editable tells whether a user message may be edited now, and whether in place.
    public EditableMessage? Editable(string messageID) => FollowUp?.Editable.FirstOrDefault(editable => editable.MessageId == messageID);

    // Change runs one write and takes the snapshot it returns; false shows its failure.
    async Task<bool> Change(string source, NativeOperation operation, Func<VeluneApplication, SnapshotResult> work, Action<Exception>? failed = null)
    {
        if (Busy)
        {
            return false;
        }
        Busy = true;
        Notify();
        try
        {
            Apply((await Call(operation, work)).Snapshot);
            return true;
        }
        catch (Exception failure)
        {
            Problems.Report(source, failure, Device);
            failed?.Invoke(failure);
            return false;
        }
        finally
        {
            Busy = false;
            Notify();
        }
    }

    // LoadSessionOptions reads the ACP Agent's options for this conversation.
    public async void LoadSessionOptions()
    {
        if (Snapshot is not { } snapshot || Configuration.RuntimeType(snapshot.ContextRuntimeId) is not { SessionOptions: true })
        {
            SessionOptions = [];
            sessionOptionsTarget = null;
            return;
        }
        var target = (snapshot.ContextRuntimeId, snapshot.Conversation.Id);
        try
        {
            var options = await Call(NativeOperation.ConfigurationRead, application => application.AcpSessionConfig(snapshot.ContextRuntimeId, snapshot.Conversation.Id));
            if (closed || Snapshot is not { } current || (current.ContextRuntimeId, current.Conversation.Id) != target) return;
            SessionOptions = options;
            sessionOptionsTarget = target;
        }
        catch (Exception failure)
        {
            if (closed || Snapshot is not { } current || (current.ContextRuntimeId, current.Conversation.Id) != target) return;
            SessionOptions = [];
            sessionOptionsTarget = null;
            Problems.Report("读取 ACP 会话选项", failure, Device);
        }
        Notify();
    }

    public async void SetSessionOption(string id, AcpConfigValue value)
    {
        if (Snapshot is not { } snapshot)
        {
            return;
        }
        var target = (snapshot.ContextRuntimeId, snapshot.Conversation.Id);
        try
        {
            var options = await Call(NativeOperation.ConfigurationWrite, application => application.SetAcpSessionConfig(snapshot.ContextRuntimeId, snapshot.Conversation.Id, id, value));
            if (closed || Snapshot is not { } current || (current.ContextRuntimeId, current.Conversation.Id) != target) return;
            SessionOptions = options;
            sessionOptionsTarget = target;
        }
        catch (Exception failure)
        {
            Problems.Report("设置 ACP 会话选项", failure, Device);
        }
        Notify();
    }

    internal ModelSelection? AcpAutoCompactionModel =>
        Snapshot is { } snapshot
            && sessionOptionsTarget == (snapshot.ContextRuntimeId, snapshot.Conversation.Id)
            && snapshot.ContextRuntimeId == Next
            && Configuration.Runtime(Next) is { TypeId: RuntimeKind.Acp }
                ? VeluneBindingsMethods.AcpSessionModel(SessionOptions) : null;

    internal AcpConfigOption[] AcpAutoCompactionOptions => AcpAutoCompactionModel != null
        ? [.. SessionOptions.Where(VeluneBindingsMethods.CanBindAcpAutoCompactionOption)] : [];

    internal bool CanEditModelAutoCompaction(string runtimeId) =>
        !closed && InputEditable && !Busy && !Running && !Loading && Next == runtimeId;

    internal async Task<RuntimeInstance?> ReadModelCompactionRuntime(string runtimeId) =>
        (await Call(NativeOperation.ConfigurationRead, application => application.List())).RuntimeInstances
            .FirstOrDefault(runtime => runtime.Id == runtimeId && runtime.Enabled);

    internal async Task SaveModelAutoCompaction(string runtimeId, ModelSelection model, AutoCompactionSetting? setting)
    {
        if (!CanEditModelAutoCompaction(runtimeId)) throw new InvalidOperationException("当前模型已不可编辑。");
        var update = await Call(NativeOperation.ConfigurationWrite,
            application => application.SetRuntimeModelAutoCompaction(runtimeId, model, setting));
        if (closed) return;
        Configuration = Configuration with { Runtimes = [.. update.RuntimeInstances.Where(instance => instance.Enabled)] };
        Notify();
    }

    internal async void SaveAcpAutoCompaction(string runtimeId, ModelSelection model, AutoCompactionSetting.Acp? setting)
    {
        if (!CanEditModelAutoCompaction(runtimeId)) return;
        if (setting != null && (!Equals(AcpAutoCompactionModel, model) || !AcpAutoCompactionOptions.Any(option =>
            option.Id == setting.OptionId && VeluneBindingsMethods.AcpAutoCompactionAccepts(option, setting.Value)))) return;
        try
        {
            await SaveModelAutoCompaction(runtimeId, model, setting);
        }
        catch (Exception failure)
        {
            Problems.Report("保存模型自动压缩设置", failure, Device);
        }
    }
}
