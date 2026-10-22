using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Velune.Bindings;
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
    // The queue as last shown, and the queued message being edited in place.
    string shownQueue = "";
    string? editingQueued;
    bool sessionOptionsRead;

    void AttachComposer()
    {
        draftTimer.Tick += (_, _) =>
        {
            draftTimer.Stop();
            Model.SaveDraft(Input.Text);
        };
        Input.TextChanged += (_, _) =>
        {
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
        ContinueButton.Click += async (_, _) =>
        {
            ScrollToEnd();
            await Model.Continue();
        };
        StatusButton.Click += (_, _) => ShowStatus();
        RestoreDraft();
    }

    // FlushDraft saves a draft edit still waiting for the timer.
    void FlushDraft()
    {
        if (draftTimer.IsEnabled)
        {
            draftTimer.Stop();
            Model.SaveDraft(Input.Text);
        }
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

    // ComposerKey handles the composer's keys: Ctrl+Shift+Enter steers the
    // running turn, Esc stops it.
    bool ComposerKey(KeyRoutedEventArgs e)
    {
        var control = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control).HasFlag(CoreVirtualKeyStates.Down);
        var shift = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift).HasFlag(CoreVirtualKeyStates.Down);
        if (e.Key == VirtualKey.Enter && control && shift)
        {
            Steer();
            return true;
        }
        if (e.Key == VirtualKey.Escape && Model.CanCancel && !Model.Busy)
        {
            Model.Stop();
            return true;
        }
        return false;
    }

    async void Steer()
    {
        var text = Input.Text;
        if (!Model.Running || Model.FollowUp is not { CanSteer: true })
        {
            return;
        }
        SetInput("");
        if (!await Model.Steer(text) && Input.Text.Length == 0)
        {
            SetInput(text);
        }
    }

    async Task AttachFile(string path)
    {
        try
        {
            await Model.Attach(Path.GetFileName(path), await File.ReadAllBytesAsync(path));
        }
        catch (Exception problem) when (problem is IOException or UnauthorizedAccessException)
        {
            FailureBar.Message = "无法读取「" + Path.GetFileName(path) + "」：" + problem.Message;
            FailureBar.IsOpen = true;
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
            FailureBar.Message = Core.Describe(problem);
            FailureBar.IsOpen = true;
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
        try
        {
            foreach (var file in (await content.GetStorageItemsAsync()).OfType<StorageFile>())
            {
                await AttachFile(file.Path);
            }
        }
        catch (Exception problem)
        {
            FailureBar.Message = Core.Describe(problem);
            FailureBar.IsOpen = true;
        }
    }

    // PasteImage attaches the clipboard's bitmap as a PNG.
    async void PasteImage(DataPackageView content)
    {
        try
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
            await Model.Attach($"粘贴的图片 {DateTime.Now:HHmmss}.png", bytes);
        }
        catch (Exception problem)
        {
            FailureBar.Message = "无法粘贴图片：" + Core.Describe(problem);
            FailureBar.IsOpen = true;
        }
    }

    void RenderComposer()
    {
        if (!sessionOptionsRead && Model.Snapshot != null && Model.SessionOpen)
        {
            sessionOptionsRead = true;
            Model.LoadSessionOptions();
        }
        RenderAttachments();
        RenderQueue();
        RenderStatus();
        var followUp = Model.FollowUp;
        var running = Model.Running;
        ContinueButton.Visibility = Visible(followUp is { CanContinue: true } && !running && Model.Sending == null);
        ContinueButton.IsEnabled = !Model.Busy;
        StopButton.Visibility = Visible(Model.CanCancel);
        StopButton.IsEnabled = !Model.Busy;
        SendButton.Content = running ? "加入队列" : "发送";
        ToolTipService.SetToolTip(SendButton, running
            ? followUp is { CanSteer: true } ? "Ctrl+Enter 加入发送队列；Ctrl+Shift+Enter 插入本轮" : "Ctrl+Enter 加入发送队列，本轮结束后发出"
            : "Ctrl+Enter");
        SendButton.IsEnabled = !Model.Busy && Model.Next.Length > 0 && Model.Uploading == 0;
        AttachButton.IsEnabled = !Model.Busy;
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

    // RenderQueue shows the queued messages, and why the queue waits when
    // no turn runs. It keeps the panel while one message is edited.
    void RenderQueue()
    {
        var followUp = Model.FollowUp;
        var queue = followUp?.Queue ?? [];
        ShowQueueBar(followUp);
        var steer = Model.Running && followUp is { CanSteer: true };
        var signature = string.Join("\n", queue.Select(message => message.Id + "\t" + message.Text + "\t" + message.Attachments.Length)) + (steer ? "\nsteer" : "") + (Model.Busy ? "\nbusy" : "");
        if (editingQueued != null && queue.Any(message => message.Id == editingQueued))
        {
            return;
        }
        editingQueued = null;
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

    Border Queued(BindingQueuedMessage message, int index, int count, bool steer)
    {
        var text = new TextBlock { Text = message.Text, TextWrapping = TextWrapping.Wrap, MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis };
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
            ToolTipService.SetToolTip(into, "经运行时的原生通道送入正在运行的这一轮，不另起一轮");
            into.Click += (_, _) => Model.SteerQueued(message.Id);
            actions.Children.Add(into);
        }
        var edit = Ui.IconButton("\uE70F", "编辑");
        var up = Ui.IconButton("\uE70E", "上移");
        var down = Ui.IconButton("\uE70D", "下移");
        var remove = Ui.IconButton("\uE711", "从队列删除");
        up.IsEnabled = index > 0 && !Model.Busy;
        down.IsEnabled = index < count - 1 && !Model.Busy;
        edit.IsEnabled = remove.IsEnabled = !Model.Busy;
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
        edit.Click += (_, _) =>
        {
            editingQueued = message.Id;
            var box = new TextBox { Text = message.Text, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 160 };
            var save = new Button { Content = "保存", Style = Ui.Style("AccentButtonStyle") };
            var cancel = new Button { Content = "取消" };
            void Done()
            {
                editingQueued = null;
                shownQueue = "";
                RenderQueue();
            }
            save.Click += (_, _) =>
            {
                if (box.Text.Trim().Length > 0)
                {
                    Model.EditQueued(message.Id, box.Text.Trim());
                }
                Done();
            };
            cancel.Click += (_, _) => Done();
            var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
            buttons.Children.Add(cancel);
            buttons.Children.Add(save);
            var editor = new StackPanel { Spacing = 8 };
            editor.Children.Add(box);
            editor.Children.Add(buttons);
            card.Child = editor;
            box.Focus(FocusState.Programmatic);
        };
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
                case BindingAcpConfigValue.Boolean { V1: var on }:
                    var toggle = new ToggleMenuFlyoutItem { Text = option.Name, IsChecked = on };
                    toggle.Click += (_, _) => Model.SetSessionOption(option.Id, new BindingAcpConfigValue.Boolean(!on));
                    if (option.Description is { Length: > 0 } about)
                    {
                        ToolTipService.SetToolTip(toggle, about);
                    }
                    PermissionMenu.Items.Add(toggle);
                    break;
                case BindingAcpConfigValue.Select { V1: var current }:
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
                        item.Click += (_, _) => Model.SetSessionOption(option.Id, new BindingAcpConfigValue.Select(value));
                        menu.Items.Add(item);
                    }
                    PermissionMenu.Items.Add(menu);
                    break;
            }
        }
    }

    void FillMessageMenu(MenuFlyout menu, BindingMessage message)
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
        menu.Items.Add(Item("编辑…", () => EditMessage(message), editable != null && !Model.Busy && !Model.Running));
    }

    static string MessageText(BindingMessage message) =>
        string.Join("\n\n", message.Blocks.OfType<BindingMessageBlock.Text>().Select(block => block.TextValue));

    // EditMessage edits a sent user message. The last one continues in
    // place; an earlier one continues in a new conversation from it, or in
    // place, which drops it and everything after it.
    async void EditMessage(BindingMessage message)
    {
        if (Model.Editable(message.Id) is not { } editable)
        {
            return;
        }
        var last = Model.Messages.LastOrDefault(shown => shown.Role == BindingMessageRole.User)?.Id == message.Id;
        var box = new TextBox { Text = MessageText(message), AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 120, MaxHeight = 320 };
        var about = last && editable.InPlace
            ? "编辑后作为新的一轮在原处继续，取代这条消息。"
            : editable.InPlace
                ? "新建会话保留原会话不变，从这条消息之前的内容继续；原地继续会舍弃这条消息及之后的全部内容。"
                : "这条消息只能在新会话中继续：新会话含有它之前的内容，原会话保持不变。";
        var panel = new StackPanel { Spacing = 12, MinWidth = 420 };
        panel.Children.Add(box);
        panel.Children.Add(Ui.Muted(about + "原消息的附件不会随之发出。"));
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "编辑消息",
            Content = panel,
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        BindingEditPlacement? primary, secondary = null;
        if (last && editable.InPlace)
        {
            dialog.PrimaryButtonText = "发送";
            primary = BindingEditPlacement.InPlace;
        }
        else
        {
            dialog.PrimaryButtonText = "在新会话中继续";
            primary = BindingEditPlacement.Fork;
            if (editable.InPlace)
            {
                dialog.SecondaryButtonText = "原地继续";
                secondary = BindingEditPlacement.InPlace;
            }
        }
        var placement = await dialog.ShowAsync() switch
        {
            ContentDialogResult.Primary => primary,
            ContentDialogResult.Secondary => secondary,
            _ => null,
        };
        if (placement is not { } chosen || box.Text.Trim().Length == 0)
        {
            return;
        }
        ScrollToEnd();
        if (await Model.Edit(message.Id, chosen, box.Text) is { } continued && chosen == BindingEditPlacement.Fork)
        {
            window.OpenLinked(continued);
        }
    }

    readonly Button resumeQueue = new() { Content = "继续发送" };
    readonly Button stopSubAgents = new() { Content = "停止子 Agent" };

    // ShowQueueBar says why the queue waits for the user, or for sub-agents.
    void ShowQueueBar(BindingFollowUp? followUp)
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
            { Paused: BindingQueuePause.TurnStopped } => ("本轮已停止，发送队列暂停。", resumeQueue),
            { Paused: BindingQueuePause.TurnFailed } => ("本轮失败，发送队列暂停。", resumeQueue),
            { Paused: BindingQueuePause.SendFailed } => ("队列中的第一条没有被接受，仍在队首。", resumeQueue),
            { Waiting: BindingQueueWait.SubAgents } => ("下一条需要新的运行时进程，会结束仍在运行的子 Agent；队列等待它们结束。", waiting.CanStopSubAgents ? stopSubAgents : null),
            _ => null,
        };
        QueueBar.IsOpen = shown != null;
        if (shown is var (message, action))
        {
            QueueBar.Message = message;
            QueueBar.ActionButton = action;
            if (action != null)
            {
                action.IsEnabled = !Model.Busy;
            }
        }
        else
        {
            QueueBar.ActionButton = null;
        }
    }
}
