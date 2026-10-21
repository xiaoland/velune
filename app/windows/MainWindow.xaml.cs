using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Velune.Bindings;
using Windows.Graphics;
using Windows.System;

namespace Velune;

// The main window: one interface whose target device the user switches.
// Its state changes on the UI thread only.
public sealed partial class MainWindow : Window
{
    readonly Core core;
    internal Hub Hub { get; }
    Device device = Device.Local;
    Listing listing = Listing.Loading;
    ConversationView? conversation;
    SettingsWindow? settings;
    // The pairing attempts already put to the user.
    readonly HashSet<string> prompted = [];
    bool approving;
    // Conversations whose read mark is on its way; the next listing may
    // still show them unread.
    readonly HashSet<string> markingRead = [];
    bool foreground = true;
    // The listing polls one device at a time; a switch starts another round.
    int round;
    // A closed window throws on any further use; the core shuts down after it.
    bool closed;
    // The sidebar's items: each group's, and each conversation's, by ID.
    readonly Dictionary<string, NavigationViewItem> groupItems = [];
    readonly Dictionary<string, NavigationViewItem> conversationItems = [];
    // RenamingID is the conversation whose title is edited in the list.
    string? renamingID;
    readonly SemaphoreSlim dialogs = new(1, 1);

    internal MainWindow(Core core)
    {
        InitializeComponent();
        this.core = core;
        Hub = new Hub(core, DispatcherQueue);
        Hub.Changed += HubChanged;

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Velune.ico"));
        if (MicaController.IsSupported())
        {
            SystemBackdrop = new MicaBackdrop();
        }
        else
        {
            Root.Style = Ui.Style("WindowBackgroundStyle");
        }
        Size(this, 1180, 780, 720, 480);

        AppTitleBar.PaneToggleRequested += (_, _) => Navigation.IsPaneOpen = !Navigation.IsPaneOpen;
        // The built-in item follows the system language; the sidebar is ours.
        Navigation.Loaded += (_, _) =>
        {
            if (Navigation.SettingsItem is NavigationViewItem item)
            {
                item.Content = "设置";
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, "设置");
            }
        };
        // Closing the main window quits: stop everything that would update it,
        // and close settings with it.
        Closed += (_, _) =>
        {
            closed = true;
            round++;
            Hub.Changed -= HubChanged;
            conversation?.Close();
            settings?.Close();
        };
        NewAccelerator.Invoked += (_, e) => { e.Handled = true; StartConversation(); };
        var settingsAccelerator = new KeyboardAccelerator { Key = (VirtualKey)188, Modifiers = VirtualKeyModifiers.Control };
        settingsAccelerator.Invoked += (_, e) => { e.Handled = true; OpenSettings(SettingsPage.Keep); };
        Root.KeyboardAccelerators.Add(settingsAccelerator);
        RenameAccelerator.Invoked += (_, e) =>
        {
            if (renamingID == null && OpenID is { } id && Listed(id) is { } listed)
            {
                e.Handled = true;
                Rename(listed);
            }
        };
        NewButton.Click += (_, _) => StartConversation();
        DeviceMenu.Opening += (_, _) => FillDeviceMenu();
        Navigation.ItemInvoked += ItemInvoked;
        Navigation.Expanding += (_, e) => Folded(e.ExpandingItemContainer, false);
        Navigation.Collapsed += (_, e) => Folded(e.CollapsedItemContainer, true);
        WelcomeButton.Click += (_, _) =>
        {
            if (listing.Configuration.Runtimes.Count == 0)
            {
                OpenSettings(SettingsPage.Runtimes);
            }
            else
            {
                StartConversation();
            }
        };
        Root.ActualThemeChanged += (_, _) => ShowMarks();
        Activated += (_, e) => SetForeground(e.WindowActivationState != WindowActivationState.Deactivated);

        ShowMarks();
        Render();
        Hub.Start();
        PollListing();
    }

    // Size sizes a window in effective pixels and keeps it from shrinking past
    // its minimum.
    internal static void Size(Window window, int width, int height, int minWidth, int minHeight)
    {
        var scale = Native.Scale(window);
        window.AppWindow.Resize(new SizeInt32((int)(width * scale), (int)(height * scale)));
        if (window.AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = (int)(minWidth * scale);
            presenter.PreferredMinimumHeight = (int)(minHeight * scale);
        }
    }

    void ShowMarks()
    {
        var dark = Root.ActualTheme == ElementTheme.Dark;
        TitleIcon.ImageSource = Core.Brand("small", dark);
        WelcomeMark.Source = Core.Brand("medium", dark);
    }

    void SetForeground(bool active)
    {
        foreground = active;
        Hub.SetActivity(device, device, active);
        NoteSeen();
    }

    void HubChanged()
    {
        // The device was removed or its pairing undone: return to this PC.
        if (!device.IsLocal && Hub.State != null && !Hub.Devices().Any(contact => contact.PeerId == device.PeerId))
        {
            SwitchTo(Device.Local);
        }
        if (!approving)
        {
            NextApproval();
        }
        settings?.HubChanged();
        Render();
    }

    // OpenSettings shows the settings window at a page, reusing an open one.
    internal void OpenSettings(SettingsPage page)
    {
        if (settings == null)
        {
            settings = new SettingsWindow(core, Hub, opened =>
            {
                // Opening a device shows it in the main window.
                SwitchTo(opened);
                Activate();
            });
            settings.Closed += (_, _) => settings = null;
        }
        settings.Show(page);
    }

    // ShowDialog shows one dialog at a time over the window.
    internal async Task<ContentDialogResult> ShowDialog(ContentDialog dialog)
    {
        await dialogs.WaitAsync();
        try
        {
            dialog.XamlRoot = Root.XamlRoot;
            return await dialog.ShowAsync();
        }
        finally
        {
            dialogs.Release();
        }
    }

    internal Task<string?> PickFolder() => Native.PickFolder(this);

    // NextApproval asks about the next pairing waiting for this PC's answer.
    async void NextApproval()
    {
        var pairing = Hub.State?.IncomingPairings.FirstOrDefault(pairing =>
            pairing.Phase == BindingPairingPhase.AwaitingApproval && !prompted.Contains(pairing.AttemptId));
        if (pairing == null)
        {
            return;
        }
        prompted.Add(pairing.AttemptId);
        approving = true;
        Activate();
        var allow = await AskApproval(pairing);
        _ = Task.Run(() =>
        {
            try
            {
                var endpoint = core.Remote();
                if (allow)
                {
                    endpoint.ApprovePairing(pairing.AttemptId, pairing.PeerId);
                }
                else
                {
                    endpoint.RejectPairing(pairing.AttemptId, pairing.PeerId);
                }
            }
            catch (BindingException)
            {
                // The next state shows where the pairing stands.
            }
        });
        approving = false;
        NextApproval();
    }

    // AskApproval asks whether a device may control this PC. Both devices show
    // the same code; only the user here can let the other one in. Neither
    // answer is the default, so a stray Enter or Esc grants nothing.
    async Task<bool> AskApproval(BindingPairingStatus pairing)
    {
        var code = new TextBlock
        {
            Text = Hub.FormatCode(pairing.VerificationCode),
            FontFamily = Markdown.Mono(),
            FontSize = 30,
            HorizontalAlignment = HorizontalAlignment.Center,
            IsTextSelectionEnabled = true,
        };
        var dialog = new ContentDialog
        {
            Title = $"“{pairing.Name}”请求控制这台电脑",
            Content = new StackPanel
            {
                Spacing = 16,
                Children =
                {
                    new TextBlock { Text = "确认两台设备显示相同的核对码。允许后，对方可以查看和操作这台电脑上的会话。", TextWrapping = TextWrapping.Wrap },
                    code,
                },
            },
            PrimaryButtonText = "允许",
            SecondaryButtonText = "拒绝",
            DefaultButton = ContentDialogButton.None,
        };
        // The question stays until one of the buttons answers it.
        dialog.Closing += (_, e) => e.Cancel = e.Result == ContentDialogResult.None;
        // The code takes the dialog's focus, so no button is the default.
        dialog.Opened += (_, _) => code.Focus(FocusState.Programmatic);
        return await ShowDialog(dialog) == ContentDialogResult.Primary;
    }

    async void PollListing()
    {
        var current = ++round;
        var polled = device;
        ulong cursor = 0;
        while (round == current)
        {
            var delay = 5_000;
            try
            {
                var (list, activity) = await Task.Run(() =>
                {
                    var application = core.Application(polled);
                    return (application.List(), application.ConversationActivity(cursor));
                });
                if (round != current)
                {
                    return;
                }
                cursor = activity.Cursor;
                listing = Listing.From(list, activity.Active);
                if (activity.Active.Length > 0)
                {
                    delay = 2_000;
                }
                if (conversation != null)
                {
                    conversation.Model.Configuration = listing.Configuration;
                }
                NoteSeen();
            }
            catch (Exception failure)
            {
                if (round != current)
                {
                    return;
                }
                listing = listing.Failed(Core.Describe(failure));
            }
            Render();
            await Task.Delay(delay);
        }
    }
    void SwitchTo(Device next)
    {
        if (next == device)
        {
            return;
        }
        Hub.SetActivity(device, next, foreground);
        device = next;
        CloseConversation();
        listing = Listing.Loading;
        renamingID = null;
        groupItems.Clear();
        conversationItems.Clear();
        Navigation.MenuItems.Clear();
        Render();
        PollListing();
    }

    void CloseConversation()
    {
        conversation?.Close();
        conversation = null;
        Page.Content = null;
    }

    void Open(Target target)
    {
        CloseConversation();
        conversation = new ConversationView(new Conversation(core, device, target, listing.Configuration), this);
        conversation.Model.Changed += Render;
        Page.Content = conversation;
        NoteSeen();
        Render();
    }

    // StartConversation opens an empty conversation; its first message starts it.
    void StartConversation()
    {
        if (listing.Configuration.Runtimes.Count == 0)
        {
            return;
        }
        Open(new Target("", "", ""));
    }

    string? OpenID => conversation?.Model.Target is { Unsent: false } target ? target.ConversationID : null;

    BindingListedConversation? Listed(string id) =>
        listing.Conversations.FirstOrDefault(listed => listed.Conversation.Id == id);

    // NoteSeen marks the open conversation read once it shows in the active window.
    async void NoteSeen()
    {
        if (!foreground || OpenID is not { } id || markingRead.Contains(id) || Listed(id) is not { Unread: true } listed)
        {
            return;
        }
        markingRead.Add(id);
        var reference = new BindingConversationReference(listed.Conversation.RuntimeId, id);
        var target = device;
        try
        {
            await Task.Run(() => core.Application(target).OrganizeConversations([reference], new BindingConversationOrganizationChange.MarkRead()));
            if (target == device && Listed(id) is { } shown)
            {
                listing.Conversations[listing.Conversations.IndexOf(shown)] = shown with { Unread = false };
                Render();
            }
        }
        catch (Exception failure)
        {
            listing.Failure = Core.Describe(failure);
            Render();
        }
        markingRead.Remove(id);
    }

    void ItemInvoked(NavigationView sender, NavigationViewItemInvokedEventArgs e)
    {
        if (e.IsSettingsInvoked)
        {
            OpenSettings(SettingsPage.Keep);
            ShowSelection();
            return;
        }
        if (e.InvokedItemContainer?.Tag is not string tag)
        {
            return;
        }
        if (tag.StartsWith("more:", StringComparison.Ordinal))
        {
            var group = listing.Groups.FirstOrDefault(group => "more:" + Listing.GroupID(group.Key) == tag);
            if (group != null)
            {
                LoadMore(group.Key);
            }
            ShowSelection();
            return;
        }
        if (tag != OpenID && Listed(tag) is { } listed)
        {
            Open(new Target(listed.Conversation.RuntimeId, listed.Conversation.Id, listed.Conversation.Title));
        }
    }

    // Folded keeps a group folded or open in the device's list preferences.
    async void Folded(NavigationViewItemBase container, bool collapsed)
    {
        if (container.Tag is not string id || !id.StartsWith("group:", StringComparison.Ordinal) || listing.Preferences is not { } preferences)
        {
            return;
        }
        var group = listing.Groups.FirstOrDefault(group => "group:" + Listing.GroupID(group.Key) == id);
        if (group == null || group.Collapsed == collapsed)
        {
            return;
        }
        var kept = (preferences.Collapsed ?? []).Where(key => Listing.GroupID(key) != Listing.GroupID(group.Key));
        preferences = preferences with { Collapsed = collapsed ? [.. kept, group.Key] : [.. kept] };
        listing.Preferences = preferences;
        listing.Groups = [.. listing.Groups.Select(g => g == group ? g with { Collapsed = collapsed } : g)];
        var target = device;
        try
        {
            await Task.Run(() => core.Application(target).SetConversationBrowserPreferences(preferences));
        }
        catch (Exception failure)
        {
            listing.Failure = Core.Describe(failure);
            Render();
        }
    }

    async void LoadMore(BindingConversationGroupKey key)
    {
        var target = device;
        try
        {
            await Task.Run(() => core.Application(target).LoadMoreConversations(key));
            PollListing();
        }
        catch (Exception failure)
        {
            listing.Failure = Core.Describe(failure);
            Render();
        }
    }

    // Rename edits the conversation's real title in place.
    void Rename(BindingListedConversation listed)
    {
        if (!listed.Conversation.CanRename || !conversationItems.TryGetValue(listed.Conversation.Id, out var item))
        {
            return;
        }
        renamingID = listed.Conversation.Id;
        var input = new TextBox { Text = listed.Conversation.Title, MinWidth = 0 };
        var done = false;
        void Finish(bool keep)
        {
            if (done)
            {
                return;
            }
            done = true;
            renamingID = null;
            var title = input.Text.Trim();
            if (keep && title.Length > 0 && title != listed.Conversation.Title)
            {
                SaveTitle(listed, title);
            }
            Render();
        }
        // Enter saves; Esc or a click elsewhere cancels.
        input.KeyDown += (_, e) =>
        {
            if (e.Key is VirtualKey.Enter or VirtualKey.Escape)
            {
                e.Handled = true;
                Finish(e.Key == VirtualKey.Enter);
            }
        };
        input.LostFocus += (_, _) => Finish(false);
        item.Content = input;
        input.Loaded += (_, _) =>
        {
            input.Focus(FocusState.Programmatic);
            input.SelectAll();
        };
    }

    async void SaveTitle(BindingListedConversation listed, string title)
    {
        var index = listing.Conversations.IndexOf(listed);
        if (index >= 0)
        {
            listing.Conversations[index] = listed with { Conversation = listed.Conversation with { Title = title } };
        }
        var target = device;
        try
        {
            await Task.Run(() => core.Application(target).RenameConversation(listed.Conversation.RuntimeId, listed.Conversation.Id, title));
        }
        catch (Exception failure)
        {
            listing.Failure = Core.Describe(failure);
            Render();
        }
    }

    void FillDeviceMenu()
    {
        DeviceMenu.Items.Clear();
        void Add(string name, Device target)
        {
            var item = new RadioMenuFlyoutItem { Text = name, GroupName = "device", IsChecked = target == device };
            item.Click += (_, _) => SwitchTo(target);
            DeviceMenu.Items.Add(item);
        }
        Add(Hub.DeviceName(Device.Local) + "（本机）", Device.Local);
        foreach (var contact in Hub.Devices())
        {
            Add(contact.Name, new Device(contact.PeerId));
        }
        DeviceMenu.Items.Add(new MenuFlyoutSeparator());
        var add = new MenuFlyoutItem { Text = "添加设备…" };
        add.Click += (_, _) => OpenSettings(SettingsPage.Devices);
        DeviceMenu.Items.Add(add);
    }

    void Render()
    {
        if (closed)
        {
            return;
        }
        var deviceName = Hub.DeviceName(device) + (device.IsLocal ? "（本机）" : "");
        DeviceButton.Content = deviceName;
        AppTitleBar.Subtitle = deviceName;
        var title = conversation?.Model.Title;
        Title = title == null ? $"Velune — {deviceName}" : $"{title} — {deviceName} — Velune";
        PageTitle.Text = title ?? "";
        PageTitle.Visibility = title == null ? Visibility.Collapsed : Visibility.Visible;
        NewButton.IsEnabled = listing.Configuration.Runtimes.Count > 0;

        var failures = new[] { Hub.Failure, listing.Failure }.Where(failure => failure.Length > 0).ToList();
        ListFailure.Text = string.Join("\n", failures);
        ListFailure.Visibility = failures.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        ListSpinner.IsActive = !listing.Loaded;
        ListSpinner.Visibility = listing.Loaded ? Visibility.Collapsed : Visibility.Visible;
        var empty = listing.Loaded && listing.Conversations.Count == 0;
        ListEmpty.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        ListEmpty.Text = listing.Configuration.Runtimes.Count == 0 ? "这台设备还没有可用的 Agent 运行时。" : "这台设备上还没有会话。";
        RenderSidebar();

        Welcome.Visibility = conversation == null ? Visibility.Visible : Visibility.Collapsed;
        var noRuntime = listing.Loaded && listing.Configuration.Runtimes.Count == 0;
        WelcomeTitle.Text = noRuntime ? "先添加一个 Agent 运行时" : "选择或新建一个会话";
        WelcomeDetail.Text = noRuntime ? "Velune 在这台设备上通过 Agent 运行时执行会话。" : "左侧列出这台设备上的会话；新会话在发送第一条消息时开始。";
        WelcomeButton.Content = noRuntime ? "添加 Agent 运行时…" : "新会话";
        ToolTipService.SetToolTip(WelcomeButton, noRuntime ? null : "Ctrl+N");
    }

    // RenderSidebar shows the list's groups as sections that fold, each
    // conversation one item with its state. Items are kept and updated, so
    // the list keeps its place and focus.
    void RenderSidebar()
    {
        var groups = new List<object>();
        var shownGroups = new HashSet<string>();
        var shownConversations = new HashSet<string>();
        foreach (var group in listing.Groups)
        {
            var id = "group:" + Listing.GroupID(group.Key);
            shownGroups.Add(id);
            if (!groupItems.TryGetValue(id, out var groupItem))
            {
                groupItem = new NavigationViewItem { Tag = id, SelectsOnInvoked = false, IsExpanded = !group.Collapsed };
                groupItems[id] = groupItem;
            }
            groupItem.Content = listing.GroupTitle(group);
            var children = new List<object>();
            foreach (var listed in listing.Conversations.Where(listed => "group:" + Listing.GroupID(listed.Group) == id))
            {
                shownConversations.Add(listed.Conversation.Id);
                children.Add(ConversationItem(listed));
            }
            if (group.Loaded < group.Total)
            {
                var moreID = "more:" + Listing.GroupID(group.Key);
                if (!conversationItems.TryGetValue(moreID, out var more))
                {
                    more = new NavigationViewItem { Tag = moreID, SelectsOnInvoked = false };
                    conversationItems[moreID] = more;
                }
                shownConversations.Add(moreID);
                more.Content = $"加载更多（{group.Loaded} / {group.Total}）";
                children.Add(more);
            }
            Reconcile(groupItem.MenuItems, children);
            groups.Add(groupItem);
        }
        foreach (var gone in groupItems.Keys.Where(id => !shownGroups.Contains(id)).ToList())
        {
            groupItems.Remove(gone);
        }
        foreach (var gone in conversationItems.Keys.Where(id => !shownConversations.Contains(id)).ToList())
        {
            conversationItems.Remove(gone);
        }
        Reconcile(Navigation.MenuItems, groups);
        ShowSelection();
    }

    static void Reconcile(IList<object> items, List<object> wanted)
    {
        for (var i = 0; i < wanted.Count; i++)
        {
            if (i < items.Count && ReferenceEquals(items[i], wanted[i]))
            {
                continue;
            }
            var at = items.IndexOf(wanted[i]);
            if (at > i)
            {
                items.RemoveAt(at);
            }
            if (i < items.Count)
            {
                items.Insert(i, wanted[i]);
            }
            else
            {
                items.Add(wanted[i]);
            }
        }
        while (items.Count > wanted.Count)
        {
            items.RemoveAt(items.Count - 1);
        }
    }

    // ConversationItem is one conversation: a dot while unread, its title,
    // and whether it failed, waits for the user or runs.
    NavigationViewItem ConversationItem(BindingListedConversation listed)
    {
        var id = listed.Conversation.Id;
        if (!conversationItems.TryGetValue(id, out var item))
        {
            item = new NavigationViewItem { Tag = id };
            item.DoubleTapped += (_, e) =>
            {
                if (Listed(id) is { } current)
                {
                    e.Handled = true;
                    Rename(current);
                }
            };
            var menu = new MenuFlyout();
            var rename = new MenuFlyoutItem { Text = "重命名…", KeyboardAcceleratorTextOverride = "F2" };
            rename.Click += (_, _) =>
            {
                if (Listed(id) is { } current)
                {
                    Rename(current);
                }
            };
            menu.Items.Add(rename);
            item.ContextFlyout = menu;
            conversationItems[id] = item;
        }
        ToolTipService.SetToolTip(item, listed.Conversation.Title);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, listed.Conversation.Title);
        if (renamingID == id)
        {
            return item;
        }
        listing.Active.TryGetValue(id, out var active);
        var running = active?.RunState == BindingRunState.Running;
        var unread = listed.Unread && !running;
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(new TextBlock { Text = listed.Conversation.Title, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center });
        FrameworkElement? state = null;
        if (active is { PendingInteractions: > 0 })
        {
            state = StateIcon("\uE7BA", "CautionIconStyle", "等待你处理");
        }
        else if (running)
        {
            state = new ProgressRing { IsActive = true, Width = 14, Height = 14 };
            ToolTipService.SetToolTip(state, "正在运行");
        }
        else if (unread && listed.Failed)
        {
            state = StateIcon("\uEA39", "CriticalIconStyle", "上一轮失败");
        }
        if (state != null)
        {
            Grid.SetColumn(state, 1);
            row.Children.Add(state);
        }
        item.Content = row;
        item.InfoBadge = unread && !listed.Failed ? new InfoBadge() : null;
        return item;
    }

    static FontIcon StateIcon(string glyph, string style, string label)
    {
        var icon = new FontIcon { Glyph = glyph, FontSize = 14, Style = Ui.Style(style) };
        ToolTipService.SetToolTip(icon, label);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(icon, label);
        return icon;
    }

    // ShowSelection selects the open conversation in the list.
    void ShowSelection()
    {
        Navigation.SelectedItem = OpenID is { } id && conversationItems.TryGetValue(id, out var item) ? item : null;
    }
}
