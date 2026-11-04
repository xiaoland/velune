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

    public FollowUp? FollowUp => Snapshot?.FollowUp;

    // Turn is a next turn of text and the attachments. Without a level or
    // mode the core keeps the conversation's current one.
    ComposedTurn Turn(string text) =>
        new(Text: text, Effort: NextTurn.Effort, Attachments: [.. Attached.Select(attachment => attachment.Reference)], PermissionMode: NextTurn.PermissionMode?.Id);

    // Draft is the text the device keeps for this conversation's composer;
    // an unsent conversation keeps none.
    public async Task<string> Draft() { await PollComposer(); return composerText; }
    public void SaveDraft(string text) { composerText = text; ComposerChanged(); _ = PollComposer(); }

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
        if (!await Change("更改发送队列", NativeOperation.ConversationWrite, application => application.SendQueue(snapshot.ContextRuntimeId, snapshot.Conversation.Id, operation)))
        {
            return false;
        }
        Attached.Clear();
        Notify();
        return true;
    }

    public void EditQueued(string id, string text) => Act("编辑排队的消息", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new SendQueueOperation.Edit(id, text)));

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

    // TakeQueued takes a queued message out of the queue to send it again:
    // its attachments go back to the composer, the text to the caller.
    public async Task<bool> TakeQueued(QueuedMessage message)
    {
        if (Snapshot is not { } snapshot)
        {
            return false;
        }
        var operation = new SendQueueOperation.Remove(message.Id);
        if (!await Change("取回排队的消息", NativeOperation.ConversationWrite, application => application.SendQueue(snapshot.ContextRuntimeId, snapshot.Conversation.Id, operation)))
        {
            return false;
        }
        foreach (var reference in message.Attachments)
        {
            Attached.Add(new Attachment(reference, Path.GetFileName(reference), imageExtensions.Contains(Path.GetExtension(reference).ToLowerInvariant())));
        }
        Notify();
        return true;
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
        var turn = Turn(text);
        if (!await Change("插入本轮", NativeOperation.Send, application => application.SteerTurn(snapshot.ContextRuntimeId, snapshot.Conversation.Id, turn)))
        {
            return false;
        }
        Attached.Clear();
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

    public bool CanSteer => InputEditable && !Busy && !stopRequested && !composing && Snapshot?.RunState == RunState.Running && FollowUp is { CanSteer: true };

    public ComposerSubmission Submission
    {
        get
        {
            var phase = stopRequested || Snapshot?.RunState == RunState.Stopping ? ComposerSubmission.Phase.Stopping
                : composerRequest ?? (Running ? ComposerSubmission.Phase.Running
                : Working || activityRunning || Loading || Busy || ParentID != null ? ComposerSubmission.Phase.Blocked : ComposerSubmission.Phase.Idle);
            var ready = ComposerReady && InputEditable && !Busy && !Loading && Next.Length > 0 && Uploading == 0;
            var canSend = ready && !Working && !activityRunning && (Target.Unsent || Snapshot is { Actions.CanSend: true } snapshot && !snapshot.PendingInteractions.Any());
            return ComposerSubmission.Resolve(phase, !string.IsNullOrWhiteSpace(composerText) || Attached.Count > 0 || Uploading > 0,
                composing, canSend, ready && Running && !stopRequested && FollowUp?.IsCompacting != true,
                FollowUp is { CanResume: true } && Next == Snapshot?.ContextRuntimeId);
        }
    }

    public bool CanCompact => !Busy && !Loading && !Working && !activityRunning
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
    public async Task<string?> Edit(string messageID, EditPlacement placement, string text)
    {
        if (Snapshot is not { } snapshot)
        {
            return null;
        }
        var turn = new ComposedTurn(Text: text.Trim(), Effort: NextTurn.Effort, Attachments: [], PermissionMode: NextTurn.PermissionMode?.Id);
        var selection = NextTurn.Model;
        DeliveredSnapshot? result = null;
        var done = await Change("编辑消息", NativeOperation.ConversationWrite, application =>
        {
            var edited = application.EditMessage(snapshot.ContextRuntimeId, snapshot.Conversation.Id, messageID, placement, selection, turn);
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
    async Task<bool> Change(string source, NativeOperation operation, Func<VeluneApplication, SnapshotResult> work)
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
            return;
        }
        try
        {
            SessionOptions = await Call(NativeOperation.ConfigurationRead, application => application.AcpSessionConfig(snapshot.ContextRuntimeId, snapshot.Conversation.Id));
        }
        catch (Exception failure)
        {
            SessionOptions = [];
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
        try
        {
            SessionOptions = await Call(NativeOperation.ConfigurationWrite, application => application.SetAcpSessionConfig(snapshot.ContextRuntimeId, snapshot.Conversation.Id, id, value));
        }
        catch (Exception failure)
        {
            Problems.Report("设置 ACP 会话选项", failure, Device);
        }
        Notify();
    }
}
