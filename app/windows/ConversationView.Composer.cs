using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.System;
using Windows.UI.Core;

namespace Velune;

// The composer around the text: its draft, which the device keeps, the
// attachments, the send queue and steering while a turn runs, continuing
// after one stopped or failed, and the status line.
public sealed partial class ConversationView
{
    readonly DispatcherTimer draftTimer = new() { Interval = TimeSpan.FromMilliseconds(600) };
    // Set while the view puts text in the input, which is no edit to save.
    bool restoringDraft;
    ulong composerVersion;
    bool composing;
    // The queue as last shown.
    string shownQueue = "";
    bool sessionOptionsRead;
    bool showingModelCompaction;

    void AttachComposer()
    {
        draftTimer.Tick += (_, _) =>
        {
            draftTimer.Stop();
            Model.SaveDraft(Input.Text);
        };
        Input.TextCompositionStarted += (_, _) => { composing = true; Model.UpdateComposer(Input.Text, true); };
        Input.TextCompositionEnded += (_, _) => { composing = false; Model.UpdateComposer(Input.Text, false); };
        Input.TextChanged += (_, _) =>
        {
            Model.UpdateComposer(Input.Text, composing);
            if (!restoringDraft)
            {
                draftTimer.Stop();
                draftTimer.Start();
            }
        };
        Input.Paste += Paste;
        AttachButton.Click += async (_, _) =>
        {
            foreach (var path in await Native.PickFiles(window))
            {
                await AttachFile(path);
            }
        };
        ComposerPanel.DragOver += (_, e) =>
        {
            if (e.DataView.Contains(StandardDataFormats.StorageItems))
            {
                e.AcceptedOperation = DataPackageOperation.Copy;
            }
        };
        ComposerPanel.Drop += Drop;
        CompactButton.Click += async (_, _) => await Model.Compact();
        HandoffActionButton.Click += (_, _) => Model.SetConversationHandoff(false);
        StatusButton.Click += (_, _) => ShowStatus();
        RestoreDraft();
    }

    // FlushDraft also retries an earlier failed cache write after the timer
    // stopped. A successful return means the current input is saved locally.
    internal bool FlushDraft()
    {
        if (draftTimer.IsEnabled)
        {
            draftTimer.Stop();
            return Model.SaveDraft(Input.Text);
        }
        return Model.FlushComposerCache();
    }

    async void RestoreDraft()
    {
        var draft = await Model.Draft();
        // Text typed meanwhile is newer than the kept draft.
        if (draft.Length > 0 && Input.Text.Length == 0)
        {
            restoringDraft = true;
            Input.Text = draft;
            Input.SelectionStart = draft.Length;
            restoringDraft = false;
        }
    }

    // SetInput replaces the input's text as an edit, which the draft keeps.
    void SetInput(string text)
    {
        Input.Text = text;
        draftTimer.Stop();
        Model.SaveDraft(text);
    }

    // Steer sends the text into the running turn; without a turn to steer,
    // or a runtime that steers, it sends or queues as the send key does.
    async void Steer()
    {
        var text = Input.Text;
        if (Model.FollowUp?.CanSteer != true)
        {
            Send();
            return;
        }
        if (await Model.Steer(text) && Input.Text == text) SetInput("");
    }

    async Task AttachFile(string path)
    {
        try
        {
            await Model.Attach(Path.GetFileName(path), await File.ReadAllBytesAsync(path));
        }
        catch (Exception problem) when (problem is IOException or UnauthorizedAccessException)
        {
            Problems.Report("读取文件「" + Path.GetFileName(path) + "」", problem);
        }
    }

    async void Drop(object sender, DragEventArgs e)
    {
        if (!e.DataView.Contains(StandardDataFormats.StorageItems))
        {
            return;
        }
        var deferral = e.GetDeferral();
        try
        {
            foreach (var file in (await e.DataView.GetStorageItemsAsync()).OfType<StorageFile>())
            {
                await AttachFile(file.Path);
            }
        }
        catch (Exception problem)
        {
            Problems.Report("添加附件", problem);
        }
        deferral.Complete();
    }

    // Paste attaches a pasted image or files; text pastes as text.
    void Paste(object sender, TextControlPasteEventArgs e)
    {
        var content = Clipboard.GetContent();
        if (content.Contains(StandardDataFormats.Text))
        {
            return;
        }
        if (content.Contains(StandardDataFormats.StorageItems))
        {
            e.Handled = true;
            PasteFiles(content);
        }
        else if (content.Contains(StandardDataFormats.Bitmap))
        {
            e.Handled = true;
            PasteImage(content);
        }
    }

    async void PasteFiles(DataPackageView content)
    {
        if (!Model.DraftEditable) return;
        try
        {
            foreach (var file in (await content.GetStorageItemsAsync()).OfType<StorageFile>())
            {
                await AttachFile(file.Path);
            }
        }
        catch (Exception problem)
        {
            Problems.Report("添加附件", problem);
        }
    }

    // PasteImage attaches the clipboard's bitmap as a PNG.
    async void PasteImage(DataPackageView content)
    {
        if (!Model.DraftEditable) return;
        try
        {
            await Model.Attach($"粘贴的图片 {DateTime.Now:HHmmss}.png", await ReadPastedImage(content));
        }
        catch (Exception problem)
        {
            Problems.Report("粘贴图片", problem);
        }
    }

    static async Task<byte[]> ReadPastedImage(DataPackageView content)
    {
        using var source = await (await content.GetBitmapAsync()).OpenReadAsync();
        var decoder = await Windows.Graphics.Imaging.BitmapDecoder.CreateAsync(source);
        using var bitmap = await decoder.GetSoftwareBitmapAsync();
        using var png = new Windows.Storage.Streams.InMemoryRandomAccessStream();
        var encoder = await Windows.Graphics.Imaging.BitmapEncoder.CreateAsync(Windows.Graphics.Imaging.BitmapEncoder.PngEncoderId, png);
        encoder.SetSoftwareBitmap(bitmap);
        await encoder.FlushAsync();
        var bytes = new byte[png.Size];
        png.Seek(0);
        using (var reader = new Windows.Storage.Streams.DataReader(png.GetInputStreamAt(0)))
        {
            await reader.LoadAsync((uint)png.Size);
            reader.ReadBytes(bytes);
        }
        return bytes;
    }

    void RenderComposer()
    {
        if (composerVersion != Model.ComposerVersion) {
            composerVersion = Model.ComposerVersion;
            restoringDraft = true; Input.Text = Model.ComposerText; restoringDraft = false;
        }
        Input.IsReadOnly = !Model.DraftEditable;
        var handoff = Model.FollowUp?.ControlHandoff;
        var handoffMessage = handoff switch
        {
            ControlHandoff.Releasing => "正在交还控制权…",
            ControlHandoff.Released => "控制权已交还",
            _ => null,
        };
        InputLock.Text = handoffMessage ?? Model.InputLockMessage ?? Model.ComposerIssue ?? "";
        InputLock.Visibility = Visible(handoffMessage != null || !Model.InputEditable || Model.ComposerIssue != null);
        HandoffActionButton.Visibility = Visible(handoffMessage != null);
        HandoffActionButton.Content = handoff == ControlHandoff.Releasing ? "取消交接" : "恢复 Velune 执行";
        HandoffActionButton.IsEnabled = !Model.Busy;
        if (!Model.DraftEditable) CompletionPopup.IsOpen = false;
        if (!sessionOptionsRead && Model.Snapshot != null && Model.SessionOpen)
        {
            sessionOptionsRead = true;
            Model.LoadSessionOptions();
        }
        RenderAttachments();
        RenderQueue();
        RenderStatus();
        var submission = Model.Submission;
        StopButton.Visibility = Visible(Model.CanCancel);
        StopButton.IsEnabled = !Model.Busy;
        SendButton.Content = submission.Title;
        SendButton.Visibility = Visible(submission.Action != ComposerSubmission.Kind.None);
        SendButton.IsEnabled = submission.Enabled;
        var keys = window.Shell.Commands;
        Input.PlaceholderText = $"输入消息，{keys.SendKey} 发送，{keys.NewLineKey} 换行，/ 命令";
        ToolTipService.SetToolTip(SendButton, submission.Action == ComposerSubmission.Kind.Resume ? "继续当前会话；不支持原生恢复的运行时会发送“continue”"
            : submission.Action == ComposerSubmission.Kind.Queue
                ? Model.CanSteer ? $"{keys.SendKey} 加入发送队列；{keys.SteerKey} 插入本轮并沿用本轮模型和推理强度" : $"{keys.SendKey} 加入发送队列，本轮结束后发出"
                : $"{keys.SendKey} 发送消息");
        AttachButton.IsEnabled = !Model.Busy && Model.DraftEditable;
        CompactButton.IsEnabled = Model.CanCompact;
    }

    void RenderAttachments()
    {
        AttachmentPanel.Children.Clear();
        foreach (var attachment in Model.Attached)
        {
            var remove = new Button { Content = new FontIcon { Glyph = "\uE711", FontSize = 10 }, Padding = new Thickness(4), Background = null, BorderThickness = new Thickness(0) };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(remove, "移除「" + attachment.Name + "」");
            remove.Click += (_, _) => Model.Detach(attachment);
            AttachmentPanel.Children.Add(Chip(attachment.Image ? "\uEB9F" : "\uE8A5", attachment.Name, remove));
        }
        if (Model.Uploading > 0)
        {
            AttachmentPanel.Children.Add(Chip(null, "正在上传…", null));
        }
        AttachmentScroller.Visibility = Visible(AttachmentPanel.Children.Count > 0);
    }

    static Border Chip(string? glyph, string text, UIElement? trailing)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        row.Children.Add(glyph != null ? new FontIcon { Glyph = glyph, FontSize = 14 } : new ProgressRing { IsActive = true, Width = 14, Height = 14 });
        row.Children.Add(new TextBlock { Text = text, MaxWidth = 220, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center });
        if (trailing != null)
        {
            row.Children.Add(trailing);
        }
        return new Border { Padding = new Thickness(8, 4, 4, 4), CornerRadius = new CornerRadius(4), Style = Ui.Style("CardStyle"), Child = row };
    }

    // RenderQueue shows the queued messages, and why the queue is waiting.
    void RenderQueue()
    {
        var followUp = Model.FollowUp;
        var queue = followUp?.Queue ?? [];
        ShowQueueBar(followUp);
        var steer = Model.CanSteerQueued;
        var signature = string.Join("\n", queue.Select(message => $"{message.Id}\t{message.EditContext?.Revision}\t{message.Text}\t{message.Attachments.Length}")) + (steer ? "\nsteer" : "") + (Model.Busy ? "\nbusy" : "");
        if (signature == shownQueue)
        {
            return;
        }
        shownQueue = signature;
        QueuePanel.Children.Clear();
        for (var i = 0; i < queue.Length; i++)
        {
            QueuePanel.Children.Add(Queued(queue[i], i, queue.Length, steer));
        }
        QueuePanel.Visibility = Visible(queue.Length > 0);
    }

    Border Queued(QueuedMessage message, int index, int count, bool steer)
    {
        var text = new TextBlock { Text = message.Text.Length == 0 ? "（仅附件）" : message.Text, TextWrapping = TextWrapping.Wrap, MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis };
        var lines = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        lines.Children.Add(text);
        if (message.Attachments.Length > 0)
        {
            lines.Children.Add(Ui.Muted($"{message.Attachments.Length} 个附件"));
        }
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        if (steer)
        {
            var into = new Button { Content = "插入本轮", IsEnabled = !Model.Busy };
            ToolTipService.SetToolTip(into, "插入正在运行的本轮，沿用本轮的模型和推理强度");
            into.Click += (_, _) => Model.SteerQueued(message.Id);
            actions.Children.Add(into);
        }
        var edit = Ui.IconButton("\uE70F", message.EditContext == null ? "更新执行端后可编辑排队消息" : "编辑排队消息");
        var up = Ui.IconButton("\uE70E", "上移");
        var down = Ui.IconButton("\uE70D", "下移");
        var remove = Ui.IconButton("\uE711", "从队列删除");
        up.IsEnabled = index > 0 && !Model.Busy;
        down.IsEnabled = index < count - 1 && !Model.Busy;
        edit.IsEnabled = message.EditContext != null && !Model.Busy;
        remove.IsEnabled = !Model.Busy;
        up.Click += (_, _) => Model.MoveQueued(message.Id, index - 1);
        down.Click += (_, _) => Model.MoveQueued(message.Id, index + 1);
        remove.Click += (_, _) => Model.RemoveQueued(message.Id);
        actions.Children.Add(edit);
        actions.Children.Add(up);
        actions.Children.Add(down);
        actions.Children.Add(remove);
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(lines);
        Grid.SetColumn(actions, 1);
        row.Children.Add(actions);
        var card = new Border { Padding = new Thickness(12, 6, 8, 6), CornerRadius = new CornerRadius(4), Style = Ui.Style("CardStyle"), Child = row };
        edit.Click += (_, _) => OpenQueuedEdit(message.Id);
        card.DoubleTapped += (_, _) => OpenQueuedEdit(message.Id);
        return card;
    }

    // FillSessionOptions offers the ACP Agent's own options for this
    // conversation: a choice among values, or one that is on or off.
    void FillSessionOptions()
    {
        if (Model.SessionOptions.Length == 0)
        {
            return;
        }
        if (PermissionMenu.Items.Count > 0)
        {
            PermissionMenu.Items.Add(new MenuFlyoutSeparator());
        }
        foreach (var option in Model.SessionOptions)
        {
            switch (option.Value)
            {
                case AcpConfigValue.Boolean { V1: var on }:
                    var toggle = new ToggleMenuFlyoutItem { Text = option.Name, IsChecked = on };
                    toggle.Click += (_, _) => Model.SetSessionOption(option.Id, new AcpConfigValue.Boolean(!on));
                    if (option.Description is { Length: > 0 } about)
                    {
                        ToolTipService.SetToolTip(toggle, about);
                    }
                    PermissionMenu.Items.Add(toggle);
                    break;
                case AcpConfigValue.Select { V1: var current }:
                    var shown = option.Choices.FirstOrDefault(choice => choice.Value == current)?.Name ?? current;
                    var menu = new MenuFlyoutSubItem { Text = option.Name + "：" + shown };
                    string? group = null;
                    foreach (var choice in option.Choices)
                    {
                        // Choices in groups show under their group's name.
                        if (choice.Group != group && choice.GroupName is { Length: > 0 } groupName)
                        {
                            menu.Items.Add(new MenuFlyoutItem { Text = groupName, IsEnabled = false });
                        }
                        group = choice.Group;
                        var item = new RadioMenuFlyoutItem { Text = choice.Name, GroupName = "acp:" + option.Id, IsChecked = choice.Value == current };
                        if (choice.Description is { Length: > 0 } detail)
                        {
                            ToolTipService.SetToolTip(item, detail);
                        }
                        var value = choice.Value;
                        item.Click += (_, _) => Model.SetSessionOption(option.Id, new AcpConfigValue.Select(value));
                        menu.Items.Add(item);
                    }
                    PermissionMenu.Items.Add(menu);
                    break;
            }
        }
    }

    bool CanConfigureModel(string runtimeId) => !showingQueuedEditor && Model.CanEditModelAutoCompaction(runtimeId);

    void FillModelAutoCompaction(IList<MenuFlyoutItemBase> items)
    {
        if (Model.Configuration.Runtime(Model.Next) is not { } runtime) return;
        var acp = runtime.TypeId == RuntimeKind.Acp;
        if (!acp && !ModelCompactionDialog.Supports(runtime.TypeId)) return;
        var enabled = CanConfigureModel(runtime.Id);
        ModelAutoCompaction[] saved;
        try { saved = VeluneBindingsMethods.RuntimeModelAutoCompactions(runtime); }
        catch (BindingException failure)
        {
            items.Add(Heading("无法读取自动压缩设置：" + Core.Describe(failure)));
            return;
        }

        var current = acp ? Model.AcpAutoCompactionModel : Model.NextTurn.Choice.Model ?? Model.NextTurn.Model;
        string Name(ModelSelection.Native model) => Model.NextTurn.Models.FirstOrDefault(option => Equals(option.Selection, model))?.Name
            ?? (model.ProviderId is { Length: > 0 } provider ? model.ModelId + "（" + provider + "）" : model.ModelId);
        if (current is ModelSelection.Native native)
        {
            var existing = saved.FirstOrDefault(entry => Equals(entry.Model, native))?.Setting;
            if (acp) FillAcpAutoCompaction(items, runtime.Id, native, Name(native), existing, enabled);
            else items.Add(Item("自动压缩…", () => ConfigureNativeModel(runtime.Id, native, Name(native)), enabled));
        }

        var other = new MenuFlyoutSubItem { Text = "已保存模型的自动压缩", IsEnabled = enabled };
        foreach (var entry in saved)
        {
            if (entry.Model is not ModelSelection.Native model || Equals(model, current)) continue;
            var title = Name(model);
            if (!Model.NextTurn.Models.Any(option => Equals(option.Selection, model))) title += "（目录未列出）";
            if (acp)
            {
                var unavailable = new MenuFlyoutSubItem { Text = title };
                unavailable.Items.Add(Heading("需打开使用此模型的 ACP 会话才能修改 Agent 选项"));
                unavailable.Items.Add(Item("跟随 Agent 设置", () => Model.SaveAcpAutoCompaction(runtime.Id, model, null)));
                other.Items.Add(unavailable);
            }
            else other.Items.Add(Item(title, () => ConfigureNativeModel(runtime.Id, model, Name(model))));
        }
        if (other.Items.Count > 0) items.Add(other);
    }

    async void ConfigureNativeModel(string runtimeId, ModelSelection.Native model, string name)
    {
        if (showingModelCompaction || !CanConfigureModel(runtimeId)) return;
        showingModelCompaction = true;
        try
        {
            if (await Model.ReadModelCompactionRuntime(runtimeId) is not { } runtime || !ModelCompactionDialog.Supports(runtime.TypeId)) return;
            await ModelCompactionDialog.Show(window.ShowDialog, name, model, [runtime], () => CanConfigureModel(runtimeId),
                (_, setting) => Model.SaveModelAutoCompaction(runtimeId, model, setting));
        }
        catch (Exception failure) { Problems.Report("配置原生模型自动压缩", failure, Model.Device); }
        finally { showingModelCompaction = false; }
    }

    void FillAcpAutoCompaction(IList<MenuFlyoutItemBase> items, string runtimeId, ModelSelection model, string name, AutoCompactionSetting? current, bool enabled)
    {
        var options = Model.AcpAutoCompactionOptions;
        if (options.Length == 0 && current == null) return;
        var menu = new MenuFlyoutSubItem { Text = "自动压缩 · " + name, IsEnabled = enabled };
        ToolTipService.SetToolTip(menu, "对当前模型的后续轮次生效");
        menu.Items.Add(Item("跟随 Agent 设置", () => Model.SaveAcpAutoCompaction(runtimeId, model, null), current != null));
        var other = new MenuFlyoutSubItem { Text = "从 Agent 选项中选取" };
        ToolTipService.SetToolTip(other, "仅选择此 Agent 用于自动压缩的选项");
        foreach (var option in options)
        {
            var choiceMenu = new MenuFlyoutSubItem { Text = option.Name };
            if (option.Description is { Length: > 0 } detail) ToolTipService.SetToolTip(choiceMenu, detail);
            void AddChoice(string title, AcpConfigValue value)
            {
                var setting = new AutoCompactionSetting.Acp(option.Id, value);
                choiceMenu.Items.Add(Choice("acp-compaction", title, Equals(current, setting), () => Model.SaveAcpAutoCompaction(runtimeId, model, setting)));
            }
            switch (option.Value)
            {
                case AcpConfigValue.Boolean:
                    AddChoice("开启", new AcpConfigValue.Boolean(true));
                    AddChoice("关闭", new AcpConfigValue.Boolean(false));
                    break;
                case AcpConfigValue.Select:
                    foreach (var choice in option.Choices) AddChoice(choice.Name, new AcpConfigValue.Select(choice.Value));
                    break;
            }
            (VeluneBindingsMethods.IsAcpAutoCompactionOption(option) ? menu : other).Items.Add(choiceMenu);
        }
        if (other.Items.Count > 0) menu.Items.Add(other);
        if (current is AutoCompactionSetting.Acp saved && !options.Any(option =>
            option.Id == saved.OptionId && VeluneBindingsMethods.AcpAutoCompactionAccepts(option, saved.Value)))
            menu.Items.Add(new MenuFlyoutItem { Text = "已保存的压缩设置当前不可用", IsEnabled = false });
        items.Add(menu);
    }

    void FillMessageMenu(MenuFlyout menu, DeliveredMessage message)
    {
        menu.Items.Clear();
        var text = MessageText(message);
        menu.Items.Add(Item("拷贝", () =>
        {
            var data = new DataPackage();
            data.SetText(text);
            Clipboard.SetContent(data);
        }, text.Length > 0));
        var editable = Model.Editable(message.Id);
        menu.Items.Add(Item("编辑…", () => EditMessage(message), editable?.Revision != null && !Model.Busy && !Model.Running));
    }

    static string MessageText(DeliveredMessage message) =>
        string.Join("\n\n", message.Blocks.OfType<DeliveredBlock.Text>().Select(block => block.TextValue));

    bool showingHistoricalEditor;

    // EditMessage edits a sent user message. The last one continues in
    // place; an earlier one continues in a new conversation from it, or in
    // place, which drops it and everything after it.
    async void EditMessage(DeliveredMessage message)
    {
        if (showingHistoricalEditor || window.IsClosed || Model.Snapshot?.Conversation is not { } source ||
            Model.Editable(message.Id) is not { Revision: { } revision } editable)
        {
            return;
        }
        var reference = new BindingMessageReference(source.RuntimeId, source.Id, message.Id);
        var last = Model.Messages.LastOrDefault(shown => shown.Role == MessageRole.User)?.Id == message.Id;
        var box = new TextBox { Text = MessageText(message), AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 120, MaxHeight = 320 };
        var about = last && editable.InPlace
            ? "编辑后作为新的一轮在原处继续，取代这条消息。"
            : editable.InPlace
                ? "新建会话保留原会话不变，从这条消息之前的内容继续；原地继续会舍弃这条消息及之后的全部内容。"
                : "这条消息只能在新会话中继续：新会话含有它之前的内容，原会话保持不变。";
        var panel = new StackPanel { Spacing = 12, MinWidth = 420 };
        panel.Children.Add(box);
        panel.Children.Add(Ui.Muted(about + "原消息的附件不会随之发出。"));
        var notice = Ui.Muted("");
        notice.TextWrapping = TextWrapping.Wrap;
        notice.Visibility = Visibility.Collapsed;
        panel.Children.Add(notice);
        var dialog = new ContentDialog
        {
            Title = "编辑消息",
            Content = panel,
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        EditPlacement primary;
        EditPlacement? secondary = null;
        if (last && editable.InPlace)
        {
            dialog.PrimaryButtonText = "发送";
            primary = EditPlacement.InPlace;
        }
        else
        {
            dialog.PrimaryButtonText = "在新会话中继续";
            primary = EditPlacement.Fork;
            if (editable.InPlace)
            {
                dialog.SecondaryButtonText = "原地继续";
                secondary = EditPlacement.InPlace;
            }
        }
        bool saving = false;
        bool needsFreshEditor = false;
        string? continued = null;
        EditPlacement? acceptedPlacement = null;
        void RefreshButtons()
        {
            var canSubmit = !saving && !needsFreshEditor && !string.IsNullOrWhiteSpace(box.Text);
            dialog.IsPrimaryButtonEnabled = canSubmit;
            dialog.IsSecondaryButtonEnabled = canSubmit && secondary != null;
            box.IsReadOnly = saving;
        }
        void ShowFailure(string detail)
        {
            if (window.IsClosed) return;
            notice.Text = detail;
            notice.Visibility = Visibility.Visible;
        }
        async Task Submit(EditPlacement placement)
        {
            if (saving || needsFreshEditor || window.IsClosed || string.IsNullOrWhiteSpace(box.Text)) return;
            saving = true;
            RefreshButtons();
            notice.Visibility = Visibility.Collapsed;
            try
            {
                continued = await Model.Edit(reference, revision, placement, box.Text, ReportRejectedEdit);
                if (window.IsClosed) return;
                if (continued != null)
                {
                    acceptedPlacement = placement;
                    dialog.Hide();
                }
                else if (notice.Visibility != Visibility.Visible)
                {
                    ShowFailure("未能提交，当前编辑仍保留。请确认会话状态后再试。");
                }
            }
            catch (Exception failure) { ReportRejectedEdit(failure); }
            finally
            {
                saving = false;
                if (!window.IsClosed && continued == null) RefreshButtons();
            }
        }
        void ReportRejectedEdit(Exception failure)
        {
            var code = failure switch
            {
                BindingException.Diagnostic diagnostic => diagnostic.code,
                BindingException.RemoteRequest remote => remote.code,
                _ => null,
            };
            needsFreshEditor = code is "edit_history_changed" or "fork_created" or "fork_outcome_unknown" or "timeout" or "transport";
            var detail = Core.Describe(failure);
            ShowFailure(needsFreshEditor ? detail + "\n请先检查会话结果，确认后重新打开编辑器。" : detail);
        }
        dialog.PrimaryButtonClick += (_, args) => { args.Cancel = true; _ = Submit(primary); };
        dialog.SecondaryButtonClick += (_, args) =>
        {
            args.Cancel = true;
            if (secondary is { } placement) _ = Submit(placement);
        };
        dialog.Closing += (_, args) => args.Cancel = saving && acceptedPlacement == null;
        box.TextChanged += (_, _) => RefreshButtons();
        RefreshButtons();
        showingHistoricalEditor = true;
        try
        {
            await window.ShowDialog(dialog);
            if (window.IsClosed || continued == null) return;
            if (acceptedPlacement == EditPlacement.Fork) window.OpenLinked(continued);
            else ScrollToEnd();
        }
        catch (Exception failure) { Problems.Report("编辑消息", failure, Model.Device); }
        finally { showingHistoricalEditor = false; }
    }

    readonly Button resumeQueue = new() { Content = "继续发送队列" };
    readonly Button stopSubAgents = new() { Content = "停止子 Agent" };

    // ShowQueueBar says why the queue waits for the user, or for sub-agents.
    void ShowQueueBar(FollowUp? followUp)
    {
        if (resumeQueue.Tag == null)
        {
            resumeQueue.Tag = true;
            resumeQueue.Click += (_, _) => Model.ResumeQueue();
            stopSubAgents.Click += (_, _) => Model.StopSubAgents();
        }
        var waiting = followUp is { Queue.Length: > 0 } ? followUp : null;
        (string Message, Button? Action)? shown = waiting switch
        {
            { Paused: QueuePause.TurnStopped } => ("本轮已停止，发送队列暂停。", resumeQueue),
            { Paused: QueuePause.TurnFailed } => ("本轮失败，发送队列暂停。", resumeQueue),
            { Paused: QueuePause.SendFailed } => ("队列中的第一条没有被接受，仍在队首。", resumeQueue),
            { Waiting: QueueWait.SubAgents } => ("下一条需要新的运行时进程，会结束仍在运行的子 Agent；队列等待它们结束。", waiting.CanStopSubAgents ? stopSubAgents : null),
            _ => null,
        };
        QueueBar.IsOpen = shown != null;
        if (shown is var (message, action))
        {
            QueueBar.Message = message;
            QueueBar.ActionButton = action;
            if (action != null)
            {
                action.IsEnabled = !Model.Busy && (action != resumeQueue || Model.InputEditable);
            }
        }
        else
        {
            QueueBar.ActionButton = null;
        }
    }
}
