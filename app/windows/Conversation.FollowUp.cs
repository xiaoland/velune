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
            var drafts = await Call(application => application.ComposerDrafts());
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
            Write("保存草稿", application => application.SetComposerDraft(id, text));
        }
    }

    // Attach uploads a file to the device that runs the conversation.
    public async Task Attach(string name, byte[] bytes)
    {
        Uploading++;
        Notify();
        try
        {
            var uploaded = await Call(application => application.UploadAttachment(name, bytes));
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
        if (!await Change("更改发送队列", application => application.SendQueue(snapshot.ContextRuntimeId, snapshot.Conversation.Id, operation)))
        {
            return false;
        }
        Attached.Clear();
        Notify();
        return true;
    }

    public void EditQueued(string id, string text) => Act("编辑排队的消息", (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Edit(id, text)));

    public void RemoveQueued(string id) => Act("移除排队的消息", (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Remove(id)));

    // MoveQueued moves a message to index in the queue as it is without it.
    public void MoveQueued(string id, int index) => Act("调整发送队列的顺序", (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Move(id, (uint)Math.Max(index, 0))));

    public void SteerQueued(string id) => Act("插入本轮", (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Steer(id)));

    // ResumeQueue sends the queue again after it paused.
    public void ResumeQueue() => Act("继续发送队列", (application, runtimeID, conversation) =>
        application.SendQueue(runtimeID, conversation, new BindingSendQueueOperation.Resume()));

    public void StopSubAgents() => Act("停止子 Agent", (application, runtimeID, conversation) => application.StopSubAgents(runtimeID, conversation));

    // Steer delivers the message into the running turn through the
    // runtime's own channel; false keeps it in the composer.
    public async Task<bool> Steer(string text)
    {
        text = text.Trim();
        if (Snapshot is not { } snapshot || FollowUp is not { CanSteer: true } || (text.Length == 0 && Attached.Count == 0))
        {
            return false;
        }
        var turn = Turn(text);
        if (!await Change("插入本轮", application => application.SteerTurn(snapshot.ContextRuntimeId, snapshot.Conversation.Id, turn)))
        {
            return false;
        }
        Attached.Clear();
        Notify();
        return true;
    }

    // Continue asks the runtime to go on after a stopped or failed turn, as
    // a real user message; it replays nothing.
    public Task<bool> Continue() => Send("继续");

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
        var done = await Change("编辑消息", application =>
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
    public BindingWorkingChanges WorkingChanges(string runtimeID, string conversationID) =>
        core.Application(Device).WorkingChanges(runtimeID, conversationID);

    // Editable tells whether a user message may be edited now, and whether in place.
    public BindingEditableMessage? Editable(string messageID) => FollowUp?.Editable.FirstOrDefault(editable => editable.MessageId == messageID);

    // Change runs one write and takes the snapshot it returns; false shows its failure.
    async Task<bool> Change(string source, Func<VeluneApplication, BindingSnapshotResult> work)
    {
        if (Busy)
        {
            return false;
        }
        Busy = true;
        Notify();
        try
        {
            Apply((await Call(work)).Snapshot);
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
            SessionOptions = await Call(application => application.AcpSessionConfig(snapshot.ContextRuntimeId, snapshot.Conversation.Id));
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
            SessionOptions = await Call(application => application.SetAcpSessionConfig(snapshot.ContextRuntimeId, snapshot.Conversation.Id, id, value));
        }
        catch (Exception failure)
        {
            Problems.Report("设置 ACP 会话选项", failure, Device);
        }
        Notify();
    }
}
