using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Velune;

sealed partial class Conversation
{
    ComposerInput? composerInput;
    string? composerOwner;
    string? intentOwner;
    sealed record PendingComposerCreation(string InputId, string OwnerId, string RuntimeId, string ConversationId);
    PendingComposerCreation? pendingCreation;
    ulong? composerDirectoryRevision;
    ulong? composerBaseline;
    bool composerLoaded, composerDirty, composerSaving, composerPolling, receivingComposer, composerUnbound, composerRestored;
    ulong? composerBindingRevision, composerBindingBaseline;
    string? composerFailure;
    public ulong ComposerVersion { get; private set; }
    public string ComposerText => composerText;
    public string? ComposerIssue => composerFailure ?? (intentOwner != null && intentOwner != composerOwner ? "所选运行时位于另一设备，输入已保留" : null) ?? (!composerLoaded ? "正在恢复输入" : null);
    public bool ComposerReady => composerLoaded && composerFailure == null && pendingCreation == null && !composerUnbound && !composerSaving && (intentOwner == null || intentOwner == composerOwner) && Configuration.Runtimes.Any(runtime => runtime.Id == Next && runtime.Enabled);
    string ComposerCachePath => ComposerCachePathFor(Target.Unsent ? "newConversation" : "conversation:" + Target.ConversationID);
    string ComposerCachePathFor(string context) => Path.Combine(core.Home, "composer-cache", Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(Device.PeerId + "/" + context))) + ".json");
    sealed record CachedComposer(int Schema, ComposerInput Input, bool Pending, ulong? Baseline, string? Owner, ulong? DirectoryRevision, PendingComposerCreation? Creation, bool Unbound, ulong? BindingRevision, ulong? BindingBaseline);
    static readonly JsonSerializerOptions ComposerJson = new() { Converters = { new ComposerModelJson() } };

    sealed class ComposerModelJson : JsonConverter<ModelSelection>
    {
        public override ModelSelection Read(ref Utf8JsonReader reader, Type type, JsonSerializerOptions options)
        {
            using var document = JsonDocument.ParseValue(ref reader);
            var value = document.RootElement;
            return value.GetProperty("kind").GetString() switch
            {
                "managed" => new ModelSelection.Managed(value.GetProperty("key").GetString()!),
                "native" => new ModelSelection.Native(value.GetProperty("provider").ValueKind == JsonValueKind.Null ? null : value.GetProperty("provider").GetString(), value.GetProperty("model").GetString()!),
                _ => throw new JsonException("无法识别本机输入的模型选择，原数据已保留"),
            };
        }
        public override void Write(Utf8JsonWriter writer, ModelSelection value, JsonSerializerOptions options)
        {
            writer.WriteStartObject();
            if (value is ModelSelection.Managed managed) { writer.WriteString("kind", "managed"); writer.WriteString("key", managed.ModelRecordKey); }
            else if (value is ModelSelection.Native native) { writer.WriteString("kind", "native"); writer.WriteString("provider", native.ProviderId); writer.WriteString("model", native.ModelId); }
            writer.WriteEndObject();
        }
    }
    void RestoreComposerCache()
    {
        try
        {
            if (!File.Exists(ComposerCachePath)) return;
            var saved = JsonSerializer.Deserialize<CachedComposer>(File.ReadAllText(ComposerCachePath), ComposerJson);
            if (saved is not { Schema: 1 }) throw new JsonException("不支持的本机输入格式，原数据已保留");
            composerDirty = saved.Pending; composerBaseline = saved.Baseline; composerOwner = saved.Owner; composerDirectoryRevision = saved.DirectoryRevision; pendingCreation = saved.Creation; composerUnbound = saved.Unbound; composerRestored = true; composerBindingRevision = saved.BindingRevision; composerBindingBaseline = saved.BindingBaseline;
            ReceiveComposer(saved.Input);
        }
        catch (Exception failure) { Problems.Report("恢复本机输入", failure, Device); }
    }
    void CacheComposer()
    {
        if (composerInput == null) return;
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(ComposerCachePath)!);
            var temporary = ComposerCachePath + ".tmp";
            File.WriteAllText(temporary, JsonSerializer.Serialize(new CachedComposer(1, composerInput with { Content = ComposerContent() }, composerDirty, composerBaseline, composerOwner, composerDirectoryRevision, pendingCreation, composerUnbound, composerBindingRevision, composerBindingBaseline), ComposerJson));
            File.Move(temporary, ComposerCachePath, true);
        }
        catch (Exception failure) { Problems.Report("保存本机输入", failure, Device); }
    }
    ComposerInputContent ComposerContent() {
        var runtime = intentRuntime ?? (Target.Unsent && Next.Length > 0 ? Next : null);
        return new(composerText,
        [.. Attached.Select(attachment => attachment.Content ?? throw new InvalidOperationException("附件尚未同步，请重新添加该附件"))], runtime, runtime == null ? null : intentOwner ?? composerOwner,
        [.. (composerInput?.Content.Choices.Where(choice => choice.OwnerId != composerOwner && choice.OwnerId.Length > 0) ?? []).Concat(intent.OrderBy(entry => entry.Key, StringComparer.Ordinal).Select(entry => new ComposerRuntimeChoice(composerOwner ?? "", entry.Key, entry.Value)))], composerInput?.Content.ChoiceEdited ?? false);
    }
    void ReceiveComposer(ComposerInput input)
    {
        receivingComposer = true;
        composerInput = input; composerText = input.Content.Text; intentRuntime = input.Content.RuntimeId; intentOwner = input.Content.RuntimeOwnerId;
        if (intentRuntime != null) { Next = intentRuntime; nextChosen = true; }
        intent.Clear(); foreach (var choice in input.Content.Choices.Where(choice => choice.OwnerId == composerOwner || composerOwner == null && choice.OwnerId.Length == 0)) intent[choice.RuntimeId] = choice.Choice;
        Attached.Clear(); Attached.AddRange(input.Content.Attachments.Select(file => new Attachment("composer:" + file.Id, file.Name, imageExtensions.Contains(Path.GetExtension(file.Name).ToLowerInvariant()), file)));
        receivingComposer = false; ComposerVersion++; Notify();
        LoadCatalog(Next, false);
    }
    void ComposerChanged()
    {
        if (receivingComposer) return;
        if (!composerDirty) composerBaseline = composerInput?.Revision;
        composerDirty = true;
        try {
            if (composerInput == null) { composerInput = new(Guid.NewGuid().ToString(), 0, ComposerContent()); composerUnbound = true; composerBindingBaseline = composerBindingRevision; }
            CacheComposer();
        } catch (Exception failure) { composerFailure = Core.Describe(failure); Problems.Report("保存输入", failure, Device); }
    }
    async Task FlushComposer()
    {
        if (!composerLoaded || composerSaving) return;
        composerSaving = true;
        try
        {
            while (composerDirty && composerInput is { } input)
            {
                var content = ComposerContent();
                var update = await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.Save(input.Id, composerBaseline == 0 ? null : composerBaseline, content)));
                var saved = update.State?.Inputs.FirstOrDefault(candidate => candidate.Id == input.Id) ?? throw new InvalidOperationException("设备没有返回输入");
                composerInput = saved; composerBaseline = saved.Revision;
                // Generated records contain arrays: compare the serialized value, not array references.
                if (JsonSerializer.Serialize(ComposerContent(), ComposerJson) == JsonSerializer.Serialize(content, ComposerJson)) { composerDirty = false; composerBaseline = null; }
                CacheComposer();
            }
            if (composerUnbound && composerInput is { } fresh && composerOwner != null) {
                ComposerContextKey context = Target.Unsent ? new ComposerContextKey.NewConversation() : new ComposerContextKey.Conversation(composerOwner, Target.ConversationID);
                await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.Bind(context, fresh.Id, composerBindingBaseline)));
                composerUnbound = false; composerBindingBaseline = null; CacheComposer();
            }
        }
        finally { composerSaving = false; }
    }
    async Task CompleteCreation() {
        if (pendingCreation is not { } creation) return;
        if (creation.OwnerId != composerOwner) throw new InvalidOperationException("新会话的执行来源不可用，输入已保留");
        var oldPath = ComposerCachePath;
        if (Snapshot == null) {
            var read = await Call(NativeOperation.ConversationRead, application => application.ReadConversation(creation.RuntimeId, creation.ConversationId));
            if (closed) return;
            Merge(read.Snapshot ?? throw new InvalidOperationException("设备没有返回新会话"));
        }
        await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.Promote(creation.InputId, creation.OwnerId, creation.ConversationId, creation.RuntimeId)));
        pendingCreation = null; CacheComposer();
        var newPath = ComposerCachePathFor("newConversation");
        if (File.Exists(newPath) && JsonSerializer.Deserialize<CachedComposer>(File.ReadAllText(newPath), ComposerJson)?.Creation?.InputId == creation.InputId) File.Delete(newPath);
        if (oldPath != ComposerCachePath && File.Exists(oldPath)) File.Delete(oldPath);
    }
    async Task PollComposer()
    {
        if (closed || Busy || composerPolling || composerSaving) return;
        composerPolling = true;
        try
        {
            composerOwner ??= await Call(NativeOperation.ConversationRead, application => application.ComposerOwnerId());
            await CompleteCreation();
            ComposerContextKey context = Target.Unsent ? new ComposerContextKey.NewConversation() : new ComposerContextKey.Conversation(composerOwner, Target.ConversationID);
            var baseline = composerDirectoryRevision ?? 0;
            var update = await Call(NativeOperation.ConversationRead, application => application.ManageComposerInputs(new ComposerInputOperation.Read(composerDirectoryRevision)));
            if (closed) return;
            if (update.State is { } state)
            {
                var creation = state.Creations.FirstOrDefault(creation => creation.InputId == composerInput?.Id && creation.OwnerId == composerOwner && creation.Revision > baseline && state.Bindings.Any(binding => binding.InputId == creation.InputId && binding.Context == new ComposerContextKey.Conversation(composerOwner, creation.ConversationId)));
                if (Target.Unsent && creation != null) {
                    composerLoaded = false; Notify();
                    var oldPath = ComposerCachePath;
                    var created = await Call(NativeOperation.ConversationRead, application => application.ReadConversation(creation.RuntimeId, creation.ConversationId));
                    if (closed) return;
                    Merge(created.Snapshot ?? throw new InvalidOperationException("设备没有返回新会话"));
                    context = new ComposerContextKey.Conversation(composerOwner, creation.ConversationId);
                    CacheComposer(); if (oldPath != ComposerCachePath && File.Exists(oldPath)) File.Delete(oldPath);
                }
                if (!state.Bindings.Any(binding => binding.Context == context))
                {
                    var ensured = await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.Ensure(context, Guid.NewGuid().ToString(), new("", [], null, null, [], false))));
                    state = ensured.State ?? throw new InvalidOperationException("设备没有返回输入目录");
                }
                composerDirectoryRevision = state.Revision;
                var binding = state.Bindings.First(binding => binding.Context == context);
                var id = binding.InputId; composerBindingRevision = binding.Revision;
                if (composerUnbound && id == composerInput?.Id) { composerUnbound = false; composerBindingBaseline = null; }
                else if (composerUnbound && !composerRestored && composerInput?.Revision == 0) composerBindingBaseline = binding.Revision;
                if (!composerDirty && !composerUnbound) ReceiveComposer(state.Inputs.First(input => input.Id == id));
            }
            composerLoaded = true;
            await FlushComposer();
            composerFailure = null;
        }
        catch (Exception failure) { composerFailure = Core.Describe(failure); Problems.Failing("composer:" + ComposerCachePath, "接续输入", failure, Device); }
        finally { composerPolling = false; if (!closed) Notify(); }
    }
}
