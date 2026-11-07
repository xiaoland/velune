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
    sealed record PendingComposerCreation(string InputId, string OwnerId, string RuntimeId, string ConversationId, ComposerTargetChoices Choices);
    PendingComposerCreation? pendingCreation;
    ulong? composerDirectoryRevision;
    ulong? composerBaseline;
    ComposerTargetChoices? composerTargetChoices;
    PreparedComposerTarget? preparedComposerTarget;
    ulong? composerTargetRevision, composerTargetBaseline;
    ulong composerTargetVersion;
    bool composerTargetDirty, composerTargetLoaded, composerCacheBlocked;
    bool composerCacheSaved = true;
    bool composerReadRequired = true;
    bool composerLoaded, composerDirty, composerSaving, composerPolling, receivingComposer, composerUnbound, composerRestored;
    ulong? composerBindingRevision, composerBindingBaseline;
    string? composerFailure;
    public ulong ComposerVersion { get; private set; }
    public string ComposerText => composerText;
    public string? ComposerIssue => (composerTargetDirty && composerTargetBaseline != composerTargetRevision ? "配置已在另一端更新，请重新选择以采用当前配置" : composerFailure) ?? (intentOwner != null && intentOwner != composerOwner ? "所选运行时位于另一设备，输入已保留" : null) ?? (!composerLoaded ? "正在恢复输入" : null);
    public bool ComposerReady => composerLoaded && composerTargetLoaded && PreparedNextTurn()?.ChosenModelUnavailable == false && composerFailure == null && pendingCreation == null && !composerUnbound && !composerSaving && (intentOwner == null || intentOwner == composerOwner) && Configuration.Runtimes.Any(runtime => runtime.Id == Next && runtime.Enabled);
    string ComposerCachePath => ComposerCachePathFor(Target.Unsent ? "newConversation" : "conversation:" + Target.ConversationID);
    string ComposerCachePathFor(string context) => Path.Combine(core.Home, "composer-cache", Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(Device.PeerId + "/" + context))) + ".json");
    sealed record CachedComposer(int Schema, ComposerInput? Input, bool Pending, ulong? Baseline, string? Owner, ulong? DirectoryRevision, PendingComposerCreation? Creation, bool Unbound, ulong? BindingRevision, ulong? BindingBaseline, ComposerTargetChoices? TargetChoices, ulong? TargetRevision, bool TargetPending, ulong? TargetBaseline);
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
            using var document = JsonDocument.Parse(File.ReadAllText(ComposerCachePath));
            if (document.RootElement.GetProperty("Schema").GetInt32() != 2)
                throw new JsonException("本机输入需要显式迁移，原文件已保留");
            var saved = JsonSerializer.Deserialize<CachedComposer>(document.RootElement.GetRawText(), ComposerJson)
                ?? throw new JsonException("无法读取本机输入，原文件已保留");
            composerDirty = saved.Pending; composerBaseline = saved.Baseline; composerOwner = saved.Owner; composerDirectoryRevision = saved.DirectoryRevision; pendingCreation = saved.Creation; composerUnbound = saved.Unbound; composerRestored = true; composerBindingRevision = saved.BindingRevision; composerBindingBaseline = saved.BindingBaseline;
            composerTargetDirty = saved.TargetPending; composerTargetBaseline = saved.TargetBaseline; composerTargetRevision = saved.TargetRevision;
            if (saved.Input is { } input) ReceiveComposer(input);
            if (saved.TargetChoices is { } choices) ReceiveTargetChoices(choices);
        }
        catch (Exception failure)
        {
            composerCacheBlocked = true; composerFailure = Core.Describe(failure);
            Problems.Report("恢复本机输入", failure, Device);
        }
    }
    void CacheComposer()
    {
        composerCacheSaved = false;
        if (composerDirty && composerInput == null)
        {
            composerInput = new(Guid.NewGuid().ToString(), 0, ComposerContent());
            composerUnbound = true;
            composerBindingBaseline = composerBindingRevision;
        }
        if (composerCacheBlocked) throw new InvalidOperationException("本机输入需要显式迁移，原文件已保留");
        Directory.CreateDirectory(Path.GetDirectoryName(ComposerCachePath)!);
        var temporary = ComposerCachePath + ".tmp";
        var input = composerInput is { } current ? current with { Content = ComposerContent() } : null;
        var saved = new CachedComposer(2, input, composerDirty, composerBaseline, composerOwner, composerDirectoryRevision, pendingCreation, composerUnbound, composerBindingRevision, composerBindingBaseline, ComposerTargetContent(), composerTargetRevision, composerTargetDirty, composerTargetBaseline);
        File.WriteAllText(temporary, JsonSerializer.Serialize(saved, ComposerJson));
        File.Move(temporary, ComposerCachePath, true);
        composerCacheSaved = true;
    }

    // A previous cache write may have failed after its debounce timer stopped.
    // Retry that same write before a caller relies on the local copy at exit.
    public bool FlushComposerCache() => composerCacheSaved || TryCacheComposer("保存输入");

    bool TryCacheComposer(string operation)
    {
        try
        {
            CacheComposer();
            return true;
        }
        catch (Exception failure)
        {
            composerFailure = Core.Describe(failure);
            Problems.Report(operation, failure, Device);
            return false;
        }
    }
    ComposerInputContent ComposerContent() => new(composerText,
        [.. Attached.Select(attachment => attachment.Content ?? throw new InvalidOperationException("附件尚未同步，请重新添加该附件"))]);

    ComposerTargetChoices ComposerTargetContent()
    {
        var runtime = intentRuntime;
        return VeluneApplicationApiMethods.NormalizeComposerTargetChoices(new(runtime, runtime == null ? null : intentOwner ?? composerOwner,
            [.. (composerTargetChoices?.Choices.Where(choice => choice.OwnerId != composerOwner && choice.OwnerId.Length > 0) ?? []).Concat(intent.OrderBy(entry => entry.Key, StringComparer.Ordinal).Select(entry => new ComposerRuntimeChoice(composerOwner ?? "", entry.Key, entry.Value)))], composerTargetChoices?.ChoiceEdited ?? false));
    }
    ComposerContextKey ComposerContext() => Target.Unsent
        ? new ComposerContextKey.NewConversation()
        : new ComposerContextKey.Conversation(composerOwner ?? throw new InvalidOperationException("输入来源尚未恢复"), Target.ConversationID);
    static bool SameComposerValue<T>(T left, T right) => JsonSerializer.Serialize(left, ComposerJson) == JsonSerializer.Serialize(right, ComposerJson);

    void ReceiveComposer(ComposerInput input)
    {
        receivingComposer = true;
        composerInput = input; composerText = input.Content.Text;
        Attached.Clear(); Attached.AddRange(input.Content.Attachments.Select(file => new Attachment("composer:" + file.Id, file.Name, imageExtensions.Contains(Path.GetExtension(file.Name).ToLowerInvariant()), file)));
        receivingComposer = false; ComposerVersion++; Notify();
    }
    void ReceiveTargetChoices(ComposerTargetChoices choices)
    {
        choices = VeluneApplicationApiMethods.NormalizeComposerTargetChoices(choices);
        composerTargetChoices = choices;
        intentRuntime = choices.RuntimeId; intentOwner = choices.RuntimeOwnerId;
        nextChosen = intentRuntime != null;
        if (intentRuntime != null)
        {
            Next = intentRuntime;
            if (Target.Unsent) Target = Target with { RuntimeID = intentRuntime };
        }
        else if (preparedComposerTarget is { } prepared) Next = prepared.DefaultRuntimeId;
        else if (Snapshot != null) FollowCurrentRuntime();
        else Next = Target.RuntimeID;
        intent.Clear(); foreach (var choice in choices.Choices.Where(choice => choice.OwnerId == composerOwner || composerOwner == null && choice.OwnerId.Length == 0)) intent[choice.RuntimeId] = choice.Choice;
        Notify(); LoadCatalog(Next, false);
    }
    void ReceiveTarget(ComposerTarget target)
    {
        composerTargetRevision = target.Revision; composerTargetLoaded = true;
        ReceiveTargetChoices(target.Choices);
    }
    ComposerTargetRequest ComposerTargetRequest() => new(
        Snapshot?.Conversation.RuntimeId ?? Target.RuntimeID,
        Target.Unsent ? null : Target.ConversationID);
    NextTurn? PreparedNextTurn()
    {
        if (preparedComposerTarget is not { } prepared || prepared.Request != ComposerTargetRequest()
            || Snapshot is { } snapshot && prepared.DefaultRuntimeId != snapshot.ContextRuntimeId) return null;
        return VeluneApplicationApiMethods.ResolveComposerTarget(prepared, ComposerTargetContent(),
            NextCatalog is { Loading: false, Failure: "" } catalog ? catalog.Models : null);
    }
    bool ComposerChanged()
    {
        if (receivingComposer) return true;
        if (!composerDirty) composerBaseline = composerInput?.Revision;
        composerDirty = true;
        return TryCacheComposer("保存输入");
    }
    void ComposerTargetChanged()
    {
        composerTargetVersion++;
        // Only an explicit choice may adopt a newer target revision after a conflict.
        if (!composerTargetDirty || composerTargetBaseline != composerTargetRevision) composerTargetBaseline = composerTargetRevision;
        composerTargetDirty = true;
        composerTargetChoices = (composerTargetChoices ?? new(null, null, [], false)) with { ChoiceEdited = true };
        composerTargetChoices = ComposerTargetContent();
        TryCacheComposer("保存发送目标");
    }
    async Task<bool> PrepareComposerSubmission()
    {
        try { await FlushComposer(); return true; }
        catch (Exception failure)
        {
            composerFailure = Core.Describe(failure); composerReadRequired = true;
            Problems.Report("同步输入", failure, Device); Notify(); return false;
        }
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
                if (SameComposerValue(ComposerContent(), content)) { composerDirty = false; composerBaseline = null; }
                CacheComposer();
            }
            if (composerUnbound && composerInput is { } fresh && composerOwner != null)
            {
                await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.Bind(ComposerContext(), fresh.Id, composerBindingBaseline)));
                composerUnbound = false; composerBindingBaseline = null; CacheComposer();
            }
            await FlushComposerTarget(ComposerContext());
        }
        finally { composerSaving = false; }
    }
    async Task FlushComposerTarget(ComposerContextKey context)
    {
        while (composerTargetDirty)
        {
            var choices = ComposerTargetContent();
            var update = await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.SaveTarget(context, composerTargetBaseline, choices)));
            var saved = update.State?.Targets.FirstOrDefault(candidate => candidate.Context == context) ?? throw new InvalidOperationException("设备没有返回发送目标");
            composerTargetRevision = saved.Revision; composerTargetBaseline = saved.Revision; composerTargetLoaded = true;
            if (SameComposerValue(ComposerTargetContent(), choices))
            {
                composerTargetChoices = saved.Choices; composerTargetDirty = false; composerTargetBaseline = null;
            }
            CacheComposer();
        }
    }
    async Task CompleteCreation(DeliveredSnapshot? createdSnapshot = null)
    {
        if (pendingCreation is not { } creation) return;
        if (creation.OwnerId != composerOwner) throw new InvalidOperationException("新会话的执行来源不可用，输入已保留");
        var oldPath = ComposerCachePath;
        if (Snapshot == null)
        {
            var snapshot = createdSnapshot;
            if (snapshot == null)
            {
                var read = await Call(NativeOperation.ConversationRead, application => application.ReadConversation(creation.RuntimeId, creation.ConversationId));
                if (closed) return;
                snapshot = read.Snapshot ?? throw new InvalidOperationException("设备没有返回新会话");
            }
            // Until the view adopts the native identity, edits belong to the global new target.
            if (Target.Unsent) await FlushComposerTarget(new ComposerContextKey.NewConversation());
            Merge(snapshot); composerTargetLoaded = false; composerTargetRevision = null;
        }
        var targetVersion = composerTargetVersion;
        var context = new ComposerContextKey.Conversation(creation.OwnerId, creation.ConversationId);
        var choices = creation.Choices with { RuntimeId = creation.RuntimeId, RuntimeOwnerId = creation.OwnerId };
        var ensured = await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.EnsureTarget(context, choices)));
        var target = ensured.State?.Targets.FirstOrDefault(candidate => candidate.Context == context) ?? throw new InvalidOperationException("设备没有返回新会话的发送目标");
        await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.Promote(creation.InputId, creation.OwnerId, creation.ConversationId, creation.RuntimeId)));
        pendingCreation = null; composerDirectoryRevision = null;
        if (composerTargetVersion == targetVersion && !composerTargetDirty) ReceiveTarget(target);
        else
        {
            // Keep choices edited while the native target was being persisted.
            composerTargetRevision = target.Revision; composerTargetLoaded = true;
            composerTargetBaseline = SameComposerValue(target.Choices, VeluneApplicationApiMethods.NormalizeComposerTargetChoices(choices)) ? target.Revision : null;
        }
        CacheComposer();
        var newPath = ComposerCachePathFor("newConversation");
        if (File.Exists(newPath) && JsonSerializer.Deserialize<CachedComposer>(File.ReadAllText(newPath), ComposerJson)?.Creation?.InputId == creation.InputId) File.Delete(newPath);
        if (oldPath != ComposerCachePath && File.Exists(oldPath)) File.Delete(oldPath);
    }
    void ObserveComposerTarget(ComposerTarget target)
    {
        composerTargetRevision = target.Revision;
        if (composerTargetDirty && SameComposerValue(ComposerTargetContent(), VeluneApplicationApiMethods.NormalizeComposerTargetChoices(target.Choices)))
        {
            composerTargetDirty = false; composerTargetBaseline = null;
        }
        if (!composerTargetDirty) ReceiveTarget(target);
    }
    async Task PollComposer()
    {
        if (closed || Busy || composerPolling || composerSaving || composerCacheBlocked) return;
        if (!Target.Unsent && Snapshot == null) return;
        composerPolling = true;
        try
        {
            composerOwner ??= await Call(NativeOperation.ConversationRead, application => application.ComposerOwnerId());
            await CompleteCreation();
            var request = ComposerTargetRequest();
            var context = ComposerContext();
            var baseline = composerDirectoryRevision ?? 0;
            var update = await Call(NativeOperation.ConversationRead, application => application.ManageComposerInputs(new ComposerInputOperation.Read(composerReadRequired ? null : composerDirectoryRevision)));
            if (closed || request != ComposerTargetRequest()) return;
            composerReadRequired = false;
            if (update.State is { } state)
            {
                var creation = state.Creations.FirstOrDefault(creation => creation.InputId == composerInput?.Id && creation.OwnerId == composerOwner && creation.Revision > baseline && state.Bindings.Any(binding => binding.InputId == creation.InputId && binding.Context == new ComposerContextKey.Conversation(composerOwner, creation.ConversationId)));
                if (Target.Unsent && creation != null)
                {
                    composerLoaded = false; Notify();
                    var oldPath = ComposerCachePath;
                    var created = await Call(NativeOperation.ConversationRead, application => application.ReadConversation(creation.RuntimeId, creation.ConversationId));
                    if (closed) return;
                    await FlushComposerTarget(context);
                    Merge(created.Snapshot ?? throw new InvalidOperationException("设备没有返回新会话"));
                    composerTargetLoaded = false; composerTargetRevision = null; preparedComposerTarget = null;
                    CacheComposer(); if (oldPath != ComposerCachePath && File.Exists(oldPath)) File.Delete(oldPath);
                    return;
                }
                if (!state.Bindings.Any(binding => binding.Context == context))
                {
                    var ensured = await Call(NativeOperation.ConversationWrite, application => application.ManageComposerInputs(new ComposerInputOperation.Ensure(context, Guid.NewGuid().ToString(), new("", []))));
                    if (closed || request != ComposerTargetRequest()) return;
                    state = ensured.State ?? throw new InvalidOperationException("设备没有返回输入目录");
                }
                composerDirectoryRevision = state.Revision;
                var binding = state.Bindings.First(binding => binding.Context == context);
                var id = binding.InputId; composerBindingRevision = binding.Revision;
                if (composerUnbound && id == composerInput?.Id) { composerUnbound = false; composerBindingBaseline = null; }
                else if (composerUnbound && !composerRestored && composerInput?.Revision == 0) composerBindingBaseline = binding.Revision;
                if (composerInput is { } local && state.Inputs.FirstOrDefault(input => input.Id == local.Id) is { } shared && composerDirty && SameComposerValue(ComposerContent(), shared.Content))
                {
                    composerInput = shared; composerDirty = false; composerBaseline = null;
                }
                if (!composerDirty && !composerUnbound) ReceiveComposer(state.Inputs.First(input => input.Id == id));
                if (state.Targets.FirstOrDefault(target => target.Context == context) is { } target) ObserveComposerTarget(target);
            }
            composerLoaded = true;
            var overlay = composerTargetDirty ? ComposerTargetContent() : null;
            var prepared = await Call(NativeOperation.ConversationWrite, application => application.PrepareComposerTarget(request, overlay));
            if (closed || request != ComposerTargetRequest()) return;
            if (prepared.OwnerId != composerOwner) throw new InvalidOperationException("执行端身份已改变，请重新打开会话，输入已保留");
            preparedComposerTarget = prepared;
            ObserveComposerTarget(prepared.Target);
            await FlushComposer();
            CacheComposer(); composerFailure = null;
        }
        catch (Exception failure) { composerReadRequired = true; composerFailure = Core.Describe(failure); Problems.Failing("composer:" + ComposerCachePath, "接续输入", failure, Device); }
        finally { composerPolling = false; if (!closed) Notify(); }
    }
}
