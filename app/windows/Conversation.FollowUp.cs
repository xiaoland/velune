using Velune.Bindings;

namespace Velune;

// An attachment uploaded for the next message: the device's reference to it,
// and its name for the composer.
sealed record Attachment(string Reference, string Name, bool Image);

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
    public BindingAcpConfigOption[] SessionOptions { get; private set; } = [];

    public BindingFollowUp? FollowUp => Snapshot?.FollowUp;

    // Turn is a next turn of text and the attachments. Without a level or
    // mode the core keeps the conversation's current one.
    BindingComposedTurn Turn(string text) =>
        new(Text: text, Effort: Effort(), Attachments: [.. Attached.Select(attachment => attachment.Reference)], PermissionMode: PermissionMode(Choice().PermissionMode)?.Id);

    // Draft is the text the device keeps for this conversation's composer;
    // an unsent conversation keeps none.
    public async Task<string> Draft()
    {
        if (Target.Unsent)
        {
            return "";
        }
        var id = Target.ConversationID;
        try
        {
            var drafts = await Call(NativeOperation.ConversationRead, application => application.ComposerDrafts());
            return drafts.FirstOrDefault(draft => draft.ConversationId == id)?.Text ?? "";
        }
        catch (Exception failure)
        {
            Problems.Report("读取草稿", failure, Device);
            return "";
        }
    }

    public void SaveDraft(string text)
    {
        if (!Target.Unsent)
        {
            var id = Target.ConversationID;
            Write("保存草稿", NativeOperation.ConversationWrite, application => application.SetComposerDraft(id, text));
        }
    }

    // Attach uploads a file to the device that runs the conversation.
    public async Task Attach(string name, byte[] bytes)
    {
        Uploading++;
        Notify();
        try
        {
            var uploaded = await Call(NativeOperation.ConversationWrite, application => application.UploadAttachment(name, bytes));
            Attached.Add(new Attachment(uploaded.Reference, uploaded.Name, imageExtensions.Contains(Path.GetExtension(name).ToLowerInvariant())));
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
        Attached.Remove(attachment);
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
        var operation = new BindingSendQueueOperation.Add(snapshot.ContextRuntimeId, Next, NextModel(), Turn(text));
        if (!await Change("更改发送队列", NativeOperation.ConversationWrite, application => application.SendQueue(snapshot.ContextRuntimeId, snapshot.Conversation.Id, operation)))
        {
            return false;
        }
        Attached.Clear();
        Notify();
        return true;
    }

    public void EditQueued(string id, string text) => Act("编辑排队的消息", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Edit(id, text)));

    public void RemoveQueued(string id) => Act("移除排队的消息", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Remove(id)));

    // MoveQueued moves a message to index in the queue as it is without it.
    public void MoveQueued(string id, int index) => Act("调整发送队列的顺序", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Move(id, (uint)Math.Max(index, 0))));

    public void SteerQueued(string id) => Act("插入本轮", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Steer(id)));

    // ResumeQueue sends the queue again after it paused.
    public void ResumeQueue() => Act("继续发送队列", NativeOperation.ConversationWrite, (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Resume()));

    public void StopSubAgents() => Act("停止子 Agent", NativeOperation.Cancel, (application, runtimeID, conversation) => application.StopSubAgents(runtimeID, conversation));

    // TakeQueued takes a queued message out of the queue to send it again:
    // its attachments go back to the composer, the text to the caller.
    public async Task<bool> TakeQueued(BindingQueuedMessage message)
    {
        if (Snapshot is not { } snapshot)
        {
            return false;
        }
        var operation = new BindingSendQueueOperation.Remove(message.Id);
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
    public void ObserveActivity(BindingActiveConversation? activity)
    {
        var running = activity?.RunState is BindingRunState.Running or BindingRunState.Stopping or BindingRunState.Elsewhere;
        if (activityRunning == running) return;
        activityRunning = running;
        Notify();
    }

    public void UpdateComposer(string text, bool isComposing)
    {
        if (composerText == text && composing == isComposing) return;
        composerText = text;
        composing = isComposing;
        Notify();
    }

    public bool CanSteer => !Busy && !stopRequested && !composing && Snapshot?.RunState == BindingRunState.Running && FollowUp is { CanSteer: true };

    public ComposerSubmission Submission
    {
        get
        {
            var phase = stopRequested || Snapshot?.RunState == BindingRunState.Stopping ? ComposerSubmission.Phase.Stopping
                : composerRequest ?? (Running ? ComposerSubmission.Phase.Running
                : Working || activityRunning || Loading || Busy || ParentID != null ? ComposerSubmission.Phase.Blocked : ComposerSubmission.Phase.Idle);
            var ready = !Busy && !Loading && Next.Length > 0 && Uploading == 0;
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
        var selection = NextModel();
        await Change("压缩上下文", NativeOperation.Compact, application => application.CompactConversation(snapshot.ContextRuntimeId, snapshot.Conversation.Id, selection));
    }

    public async Task<bool> Continue()
    {
        if (Snapshot is not { } snapshot || !Submission.CanResume) return false;
        var selection = NextModel();
        var effort = Effort();
        var mode = PermissionMode(Choice().PermissionMode)?.Id;
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
    public async Task<string?> Edit(string messageID, BindingEditPlacement placement, string text)
    {
        if (Snapshot is not { } snapshot)
        {
            return null;
        }
        var turn = new BindingComposedTurn(Text: text.Trim(), Effort: Effort(), Attachments: [], PermissionMode: PermissionMode(Choice().PermissionMode)?.Id);
        var selection = NextModel();
        BindingConversationSnapshot? result = null;
        var done = await Change("编辑消息", NativeOperation.ConversationWrite, application =>
        {
            var edited = application.EditMessage(snapshot.ContextRuntimeId, snapshot.Conversation.Id, messageID, placement, selection, turn);
            result = edited.Snapshot;
            // A fork's snapshot is the new conversation's, which this one does not take.
            return placement == BindingEditPlacement.Fork ? new BindingSnapshotResult(null) : edited;
        });
        return !done ? null : placement == BindingEditPlacement.Fork ? result?.Conversation.Id : snapshot.Conversation.Id;
    }

    // WorkingChanges reads the conversation's working directory as git sees
    // it, blocking; for the status line's branch.
    public Task<BindingWorkingChanges> WorkingChanges(string runtimeID, string conversationID) =>
        core.Call(Device, NativeOperation.ConversationRead, application => application.WorkingChanges(runtimeID, conversationID));

    // Editable tells whether a user message may be edited now, and whether in place.
    public BindingEditableMessage? Editable(string messageID) => FollowUp?.Editable.FirstOrDefault(editable => editable.MessageId == messageID);

    // Change runs one write and takes the snapshot it returns; false shows its failure.
    async Task<bool> Change(string source, NativeOperation operation, Func<VeluneApplication, BindingSnapshotResult> work)
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

    public async void SetSessionOption(string id, BindingAcpConfigValue value)
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
