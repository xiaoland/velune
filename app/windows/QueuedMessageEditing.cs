using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.System;

namespace Velune;

public sealed partial class ConversationView
{
    bool showingQueuedEditor;

    bool OpenQueuedEdit(string id)
    {
        if (showingQueuedEditor || Model.Busy || Model.Snapshot is not { } snapshot ||
            snapshot.FollowUp.Queue.FirstOrDefault(item => item.Id == id) is not { EditContext: { } instruction } message)
            return false;
        showingQueuedEditor = true;
        _ = ShowQueuedEdit(snapshot.Conversation.Id, message, instruction);
        return true;
    }

    async Task ShowQueuedEdit(string conversationID, QueuedMessage message, QueuedMessageEditContext instruction)
    {
        try
        {
            using var editor = new QueuedMessageEdit(Model, window, conversationID, message, instruction);
            await window.ShowDialog(editor.Dialog);
        }
        catch (Exception failure) { Problems.Report("编辑排队消息", failure, Model.Device); }
        finally { showingQueuedEditor = false; }
    }

    // The queue entry owns the submitted instruction. Each original attachment
    // remains an index into that revision, including a file on the execution host.
    sealed class QueuedMessageEdit : IDisposable
    {
        readonly Conversation conversation;
        readonly WorkspaceWindow window;
        readonly string conversationID;
        readonly string messageID;
        readonly QueuedMessageEditContext original;
        readonly TurnChoice originalChoice;
        readonly TextBox input = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 96, MaxHeight = 280 };
        readonly Button attach = Ui.IconButton("\uE723", "添加附件");
        readonly Button agent = new() { Flyout = new MenuFlyout() };
        readonly Button permission = new() { Flyout = new MenuFlyout() };
        readonly StackPanel chips = new() { Orientation = Orientation.Horizontal, Spacing = 6 };
        readonly ScrollViewer scroller = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled };
        readonly TextBlock notice = Ui.Muted("");
        readonly List<(Attachment Item, uint? OriginalIndex)> attachments = [];
        readonly Dictionary<string, TurnChoice> choices = [];
        string runtimeID;
        string failure = "";
        int uploading;
        bool saving, composing, closed;

        public ContentDialog Dialog { get; } = new() { Title = "编辑排队消息", PrimaryButtonText = "保存修改", CloseButtonText = "取消" };

        TurnChoice Current => choices[runtimeID];
        NextTurn Execution => conversation.ResolveQueuedExecution(runtimeID, Current);
        string? EntryIssue => conversation.QueuedEditIssue(conversationID, messageID, original.Revision);
        bool CanEdit => !closed && !saving && EntryIssue == null;
        string? ConfigurationIssue(NextTurn next) => conversation.Configuration.Runtime(runtimeID) is not { Enabled: true }
            ? "所选运行时当前不可用。" : next.ChosenModelUnavailable ? "所选模型当前不可用。" : null;
        bool CanSave(NextTurn next) => CanEdit && !conversation.Busy && !composing && uploading == 0
            && (!string.IsNullOrWhiteSpace(input.Text) || attachments.Count != 0)
            && ((runtimeID == original.RuntimeId && Equals(Current, originalChoice)) || ConfigurationIssue(next) == null);

        public QueuedMessageEdit(Conversation conversation, WorkspaceWindow window, string conversationID, QueuedMessage message, QueuedMessageEditContext instruction)
        {
            this.conversation = conversation;
            this.window = window;
            this.conversationID = conversationID;
            messageID = message.Id;
            original = instruction;
            runtimeID = instruction.RuntimeId;
            originalChoice = new TurnChoice(instruction.Selection, instruction.Effort, instruction.PermissionMode);
            choices.Add(runtimeID, originalChoice);
            for (var index = 0; index < message.Attachments.Length; index++)
            {
                var path = message.Attachments[index];
                attachments.Add((new Attachment(path, Path.GetFileName(path.Replace('\\', '/')), false), (uint)index));
            }
            input.Text = message.Text;
            input.TextCompositionStarted += (_, _) => { composing = true; Render(); };
            input.TextCompositionEnded += (_, _) => { composing = false; Render(); };
            input.TextChanged += (_, _) => Render();
            input.PreviewKeyDown += InputKey;
            input.Paste += Paste;
            attach.Click += async (_, _) => await AddFromPicker();
            var agentMenu = (MenuFlyout)agent.Flyout;
            agentMenu.Opening += (_, _) => FillAgent(agentMenu);
            var permissionMenu = (MenuFlyout)permission.Flyout;
            permissionMenu.Opening += (_, _) => FillPermission(permissionMenu);

            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            actions.Children.Add(attach);
            actions.Children.Add(agent);
            actions.Children.Add(permission);
            scroller.Content = chips;
            var layout = new StackPanel { MinWidth = 420, Spacing = 10, AllowDrop = true };
            layout.Children.Add(scroller);
            layout.Children.Add(input);
            layout.Children.Add(actions);
            layout.Children.Add(notice);
            layout.DragOver += (_, e) =>
            {
                if (CanEdit && e.DataView.Contains(StandardDataFormats.StorageItems)) e.AcceptedOperation = DataPackageOperation.Copy;
            };
            layout.Drop += async (_, e) =>
            {
                if (!CanEdit || !e.DataView.Contains(StandardDataFormats.StorageItems)) return;
                var deferral = e.GetDeferral();
                try { await AddFiles(e.DataView); }
                finally { deferral.Complete(); }
            };
            Dialog.Resources["ContentDialogMaxWidth"] = 620d;
            Dialog.Content = layout;
            Dialog.PrimaryButtonClick += (sender, args) => { args.Cancel = true; _ = Save(); };
            Dialog.Closing += (_, args) => args.Cancel = saving;
            Dialog.Opened += (_, _) =>
            {
                conversation.LoadCatalog(runtimeID, false);
                input.Focus(FocusState.Programmatic);
            };
            conversation.Changed += Render;
            Render();
        }

        void Render()
        {
            if (closed) return;
            var next = Execution;
            input.IsReadOnly = saving;
            attach.IsEnabled = agent.IsEnabled = permission.IsEnabled = CanEdit;
            var modelLabel = next.ModelName ?? (Current.Model switch
            {
                ModelSelection.Native native => native.ModelId,
                ModelSelection.Managed managed => managed.ModelRecordKey,
                _ => "运行时默认",
            });
            var runtime = conversation.Configuration.Runtime(runtimeID)?.Name ?? runtimeID;
            var title = string.Join(" · ", new[] { runtime, modelLabel, Current.Effort }.OfType<string>());
            agent.Content = new TextBlock { Text = title, MaxWidth = 250, TextTrimming = TextTrimming.CharacterEllipsis };
            ToolTipService.SetToolTip(agent, title);
            permission.Content = next.PermissionMode?.Label ?? Current.PermissionMode ?? "权限";
            permission.Visibility = Visible(next.PermissionModes.Length != 0 || Current.PermissionMode != null);
            Dialog.PrimaryButtonText = saving ? "正在保存…" : "保存修改";
            Dialog.IsPrimaryButtonEnabled = CanSave(next);
            notice.Text = string.Join("\n", new[] { EntryIssue, failure, ConfigurationIssue(next) }.Where(text => !string.IsNullOrWhiteSpace(text)).Distinct());
            notice.TextWrapping = TextWrapping.Wrap;
            notice.Visibility = Visible(notice.Text.Length != 0);

            chips.Children.Clear();
            foreach (var entry in attachments)
            {
                var remove = Ui.IconButton("\uE711", "移除「" + entry.Item.Name + "」");
                remove.IsEnabled = CanEdit;
                remove.Click += (_, _) => { if (CanEdit && attachments.Remove(entry)) Render(); };
                chips.Children.Add(Chip(entry.Item.Image ? "\uEB9F" : "\uE8A5", entry.Item.Name, remove));
            }
            if (uploading != 0) chips.Children.Add(Chip(null, "正在上传…", null));
            scroller.Visibility = Visible(chips.Children.Count != 0);
        }

        void FillAgent(MenuFlyout menu)
        {
            menu.Items.Clear();
            if (!CanEdit) return;
            var next = Execution;
            var runtimeMenu = new MenuFlyoutSubItem { Text = "运行时" };
            foreach (var runtime in conversation.Configuration.Runtimes.Where(candidate => candidate.Enabled))
                runtimeMenu.Items.Add(Choice("queued-runtime", runtime.Name, runtime.Id == runtimeID, () =>
                {
                    if (!CanEdit) return;
                    runtimeID = runtime.Id;
                    choices.TryAdd(runtimeID, new TurnChoice(null, null, null));
                    conversation.LoadCatalog(runtimeID, false);
                    Render();
                }));
            menu.Items.Add(runtimeMenu);

            var modelMenu = new MenuFlyoutSubItem { Text = "模型" };
            var catalog = conversation.QueuedEditCatalog(runtimeID);
            if (catalog is { Loading: true }) modelMenu.Items.Add(Heading("正在读取运行时的模型…"));
            else if (catalog is { Failure.Length: > 0 }) modelMenu.Items.Add(Heading("无法读取运行时的模型"));
            modelMenu.Items.Add(Choice("queued-model", "运行时默认", Current.Model == null, () => Choose(Current with { Model = null, Effort = null })));
            foreach (var model in next.Models)
                modelMenu.Items.Add(Choice("queued-model", model.Name, Equals(model.Selection, Current.Model), () =>
                    Choose(Current with { Model = model.Selection, Effort = model.ReasoningEfforts.Contains(Current.Effort) ? Current.Effort : null })));
            modelMenu.Items.Add(Item("重新读取模型", () => conversation.LoadCatalog(runtimeID, true), catalog is not { Loading: true }));
            menu.Items.Add(modelMenu);

            if (next.Efforts.Length == 0) return;
            var effortMenu = new MenuFlyoutSubItem { Text = "推理强度" };
            effortMenu.Items.Add(Choice("queued-effort", "运行时默认", Current.Effort == null, () => Choose(Current with { Effort = null })));
            foreach (var effort in next.Efforts)
                effortMenu.Items.Add(Choice("queued-effort", effort, effort == Current.Effort, () => Choose(Current with { Effort = effort })));
            menu.Items.Add(effortMenu);
        }

        void FillPermission(MenuFlyout menu)
        {
            menu.Items.Clear();
            if (!CanEdit) return;
            menu.Items.Add(Choice("queued-permission", "沿用运行时设置", Current.PermissionMode == null, () => Choose(Current with { PermissionMode = null })));
            foreach (var mode in Execution.PermissionModes)
            {
                var item = Choice("queued-permission", mode.Label, mode.Id == Current.PermissionMode, () => Choose(Current with { PermissionMode = mode.Id }));
                ToolTipService.SetToolTip(item, mode.Detail);
                menu.Items.Add(item);
            }
        }

        void Choose(TurnChoice choice)
        {
            if (!CanEdit) return;
            choices[runtimeID] = choice;
            Render();
        }

        async Task AddFromPicker()
        {
            if (!CanEdit) return;
            try
            {
                foreach (var path in await Native.PickFiles(window)) await AddPath(path);
            }
            catch (Exception problem) { Fail(Core.Describe(problem)); }
        }

        async Task AddFiles(DataPackageView files)
        {
            try
            {
                foreach (var file in (await files.GetStorageItemsAsync()).OfType<StorageFile>()) await AddPath(file.Path);
            }
            catch (Exception problem) { Fail(Core.Describe(problem)); }
        }

        Task AddPath(string path) => AddAttachment(Path.GetFileName(path), () => File.ReadAllBytesAsync(path));

        async Task AddAttachment(string name, Func<Task<byte[]>> read)
        {
            if (!CanEdit) return;
            uploading++;
            failure = "";
            Render();
            try
            {
                var content = await read();
                if (!CanEdit) return;
                var attachment = await conversation.UploadQueuedAttachment(name, content);
                if (!closed) attachments.Add((attachment, null));
            }
            catch (Exception problem) { Fail("无法添加「" + name + "」：" + Core.Describe(problem)); }
            finally { uploading--; Render(); }
        }

        async void Paste(object sender, TextControlPasteEventArgs args)
        {
            if (!CanEdit) return;
            try
            {
                var content = Clipboard.GetContent();
                if (content.Contains(StandardDataFormats.Text)) return;
                if (content.Contains(StandardDataFormats.StorageItems))
                {
                    args.Handled = true;
                    await AddFiles(content);
                }
                else if (content.Contains(StandardDataFormats.Bitmap))
                {
                    args.Handled = true;
                    await AddAttachment($"粘贴的图片 {DateTime.Now:HHmmss}.png", () => ReadPastedImage(content));
                }
            }
            catch (Exception problem) { Fail(Core.Describe(problem)); }
        }

        void InputKey(object sender, KeyRoutedEventArgs args)
        {
            if (args.Key != VirtualKey.Enter || composing) return;
            var save = window.Shell.Commands.SendMode == SendMode.Enter
                ? Plain : Down(VirtualKey.Control) && !Down(VirtualKey.Menu) && !Down(VirtualKey.Shift);
            if (!save) return;
            args.Handled = true;
            _ = Save();
        }

        async Task Save()
        {
            if (!CanSave(Execution)) return;
            saving = true;
            failure = "";
            Render();
            bool saved = false;
            try
            {
                saved = await conversation.ReplaceQueued(conversationID, messageID, original.Revision, runtimeID, Current,
                    [.. attachments.Select(entry => entry.OriginalIndex).OfType<uint>()], input.Text,
                    [.. attachments.Where(entry => entry.OriginalIndex == null).Select(entry => entry.Item.Reference)], Fail);
                if (!saved && failure.Length == 0) Fail("未能保存，当前修改仍保留。");
            }
            catch (Exception problem) { Fail(Core.Describe(problem)); }
            finally { saving = false; }
            if (closed) return;
            if (saved) Dialog.Hide();
            else Render();
        }

        void Fail(string detail)
        {
            if (closed) return;
            failure = detail;
            Render();
        }

        public void Dispose()
        {
            closed = true;
            conversation.Changed -= Render;
        }
    }
}
