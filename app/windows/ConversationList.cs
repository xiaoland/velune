using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Velune.Bindings;
using Windows.System;

namespace Velune;

// The conversation list as a tool window: search and the 显示 menu over the
// device's list in its groups. Group titles are rows of the list, so it
// keeps one selection across groups; choosing one conversation opens it,
// and the menus act on every chosen one.
sealed partial class ConversationList : UserControl
{
    readonly WorkspaceWindow window;
    readonly AutoSuggestBox searchBox = new() { PlaceholderText = "搜索会话", QueryIcon = new SymbolIcon(Symbol.Find) };
    readonly DropDownButton displayButton = new();
    readonly MenuFlyout displayMenu = new() { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.BottomEdgeAlignedRight };
    readonly TextBlock failure = new() { Style = Ui.Style("ErrorTextStyle"), TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed, Margin = new Thickness(12, 0, 12, 4) };
    readonly ProgressRing spinner = new() { Width = 20, Height = 20, HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(12, 4, 12, 4) };
    readonly TextBlock empty = new() { Style = Ui.Style("SecondaryTextStyle"), TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed, Margin = new Thickness(12, 4, 12, 4) };
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Extended, Padding = new Thickness(0, 0, 0, 8), CanDragItems = true, AllowDrop = true };
    // The rows by tag: "group:…" for a group's title, "more:…" to load more
    // of a group, else a conversation's ID.
    readonly Dictionary<string, ListViewItem> rows = [];
    // RenamingID is the conversation whose title is edited in the list.
    string? renamingID;
    // Set while the list selects what the window shows, so it opens nothing.
    bool syncing;

    Device Device => window.Device;
    Listing Listing => window.Listing;
    Listings Listings => window.Shell.Listings;

    public ConversationList(WorkspaceWindow window)
    {
        this.window = window;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(searchBox, "搜索会话");
        displayButton.Content = new FontIcon { Glyph = "\uE71C", FontSize = 14 };
        displayButton.Flyout = displayMenu;
        ToolTipService.SetToolTip(displayButton, "分组、筛选、排序与位置");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(displayButton, "显示");
        var header = new Grid { Margin = new Thickness(8, 8, 8, 6), ColumnSpacing = 4 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(searchBox);
        Grid.SetColumn(displayButton, 1);
        header.Children.Add(displayButton);
        var status = new StackPanel();
        status.Children.Add(failure);
        status.Children.Add(spinner);
        status.Children.Add(empty);
        var root = new Grid();
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.Children.Add(header);
        Grid.SetRow(status, 1);
        root.Children.Add(status);
        Grid.SetRow(list, 2);
        root.Children.Add(list);
        Content = root;

        list.SelectionChanged += SelectionChanged;
        list.KeyDown += ListKeyDown;
        list.DragItemsStarting += DragStarting;
        list.DragItemsCompleted += DragCompleted;
        list.DragOver += GroupDragOver;
        list.Drop += GroupDrop;
        AttachBrowser();
        Loaded += (_, _) => Render();
    }

    public void FocusSearch() => searchBox.Focus(FocusState.Keyboard);

    // Render shows the device's latest list, keeping rows, so the list keeps
    // its place and focus.
    public void Render()
    {
        var listing = Listing;
        var failures = new[] { window.Shell.Hub.Failure, listing.Failure }.Where(text => text.Length > 0).ToList();
        failure.Text = string.Join("\n", failures);
        failure.Visibility = Visible(failures.Count > 0);
        spinner.IsActive = !listing.Loaded;
        spinner.Visibility = Visible(!listing.Loaded);
        var (groups, conversations) = Shown();
        empty.Visibility = Visible(listing.Loaded && conversations.Count == 0);
        empty.Text = Searching ? "没有匹配的会话。"
            : listing.Configuration.Runtimes.Count == 0 ? "这台设备还没有可用的 Agent 运行时。"
            : listing.ShowsArchived ? "没有已归档的会话。"
            : listing.Preferences is { RuntimeId: not null } or { Project: not BindingConversationBrowserProject.All } ? "没有符合筛选的会话。"
            : "这台设备上还没有会话。";

        var wanted = new List<object>();
        var shown = new HashSet<string>();
        // Without grouping the list is one group, whose title it does not show.
        var titled = groups.Count > 1 || groups.Any(group => group.Key is not BindingConversationGroupKey.All);
        foreach (var group in groups)
        {
            var id = "group:" + Listing.GroupID(group.Key);
            if (titled)
            {
                shown.Add(id);
                wanted.Add(GroupRow(group, id));
            }
            if (group.Collapsed && titled && !Searching)
            {
                continue;
            }
            foreach (var listed in conversations.Where(listed => "group:" + Listing.GroupID(listed.Group) == id))
            {
                shown.Add(listed.Conversation.Id);
                wanted.Add(ConversationRow(listed, titled));
            }
            if (!Searching && group.Loaded < group.Total)
            {
                var moreID = "more:" + Listing.GroupID(group.Key);
                if (!rows.TryGetValue(moreID, out var more))
                {
                    more = new ListViewItem { Tag = moreID };
                    more.AddHandler(TappedEvent, new TappedEventHandler((_, _) =>
                    {
                        if (Shown().Groups.FirstOrDefault(found => "more:" + Listing.GroupID(found.Key) == moreID) is { } loading)
                        {
                            LoadMore(loading.Key);
                        }
                    }), true);
                    rows[moreID] = more;
                }
                more.Content = new TextBlock { Text = $"加载更多（{group.Loaded} / {group.Total}）", Style = Ui.Style("SecondaryTextStyle") };
                more.Padding = new Thickness(titled ? 28 : 12, 0, 12, 0);
                shown.Add(moreID);
                wanted.Add(more);
            }
        }
        foreach (var gone in rows.Keys.Where(id => !shown.Contains(id)).ToList())
        {
            rows.Remove(gone);
        }
        syncing = true;
        Ui.Reconcile(list.Items, wanted);
        syncing = false;
        ShowSelection();
    }

    static Visibility Visible(bool visible) => visible ? Visibility.Visible : Visibility.Collapsed;

    ListViewItem GroupRow(BindingConversationGroup group, string id)
    {
        if (!rows.TryGetValue(id, out var row))
        {
            row = new ListViewItem { Tag = id, Padding = new Thickness(8, 0, 12, 0), MinHeight = 32 };
            row.AddHandler(TappedEvent, new TappedEventHandler((_, e) =>
            {
                e.Handled = true;
                Fold(id);
            }), true);
            var menu = new MenuFlyout();
            menu.Opening += (_, _) => FillGroupMenu(menu, id);
            row.ContextFlyout = menu;
            rows[id] = row;
        }
        var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        line.Children.Add(new FontIcon { Glyph = group.Collapsed && !Searching ? "\uE76C" : "\uE70D", FontSize = 10 });
        line.Children.Add(new TextBlock { Text = Listing.GroupTitle(group), Style = Ui.Style("BodyStrongTextBlockStyle"), TextTrimming = TextTrimming.CharacterEllipsis });
        row.Content = line;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, Listing.GroupTitle(group));
        return row;
    }

    // ConversationRow is one conversation: a dot while unread, its title,
    // and whether it failed, waits for the user or runs.
    ListViewItem ConversationRow(BindingListedConversation listed, bool indented)
    {
        var id = listed.Conversation.Id;
        if (!rows.TryGetValue(id, out var row))
        {
            row = new ListViewItem { Tag = id };
            row.DoubleTapped += (_, e) =>
            {
                if (Listed(id) is { } current)
                {
                    e.Handled = true;
                    Rename(current);
                }
            };
            // The middle button opens it in a new tab, as in a browser.
            row.AddHandler(PointerPressedEvent, new PointerEventHandler((_, e) =>
            {
                if (e.GetCurrentPoint(row).Properties.IsMiddleButtonPressed && Listed(id) is { } current)
                {
                    e.Handled = true;
                    window.Open(Target(current), Opening.NewTab);
                }
            }), true);
            var menu = new MenuFlyout();
            menu.Opening += (_, _) => FillConversationMenu(menu, id);
            row.ContextFlyout = menu;
            rows[id] = row;
        }
        row.Padding = new Thickness(indented ? 28 : 12, 0, 12, 0);
        ToolTipService.SetToolTip(row, listed.Conversation.Title);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, listed.Conversation.Title);
        if (renamingID == id)
        {
            return row;
        }
        Listing.Active.TryGetValue(id, out var active);
        var running = active?.RunState == BindingRunState.Running;
        var unread = listed.Unread && !running;
        var line = new Grid { ColumnSpacing = 8 };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        line.Children.Add(new TextBlock { Text = listed.Conversation.Title, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center });
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
        else if (unread)
        {
            state = new InfoBadge { VerticalAlignment = VerticalAlignment.Center };
            ToolTipService.SetToolTip(state, "未读");
        }
        if (state != null)
        {
            Grid.SetColumn(state, 1);
            line.Children.Add(state);
        }
        row.Content = line;
        return row;
    }

    static FontIcon StateIcon(string glyph, string style, string label)
    {
        var icon = new FontIcon { Glyph = glyph, FontSize = 14, Style = Ui.Style(style) };
        ToolTipService.SetToolTip(icon, label);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(icon, label);
        return icon;
    }

    static Target Target(BindingListedConversation listed) =>
        new(listed.Conversation.RuntimeId, listed.Conversation.Id, listed.Conversation.Title);

    BindingListedConversation? Listed(string id) =>
        Listing.Conversations.FirstOrDefault(listed => listed.Conversation.Id == id)
        ?? (Searching ? search!.Conversations.FirstOrDefault(listed => listed.Conversation.Id == id) : null);

    // Chosen are the selected conversations, in the list's order.
    List<BindingListedConversation> Chosen() =>
        [.. list.SelectedItems.OfType<ListViewItem>().Select(row => row.Tag as string).OfType<string>()
            .Where(id => !id.Contains(':')).Select(Listed).OfType<BindingListedConversation>()];

    void SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        // Titles and 加载更多 are rows to act on, not to choose.
        foreach (var row in e.AddedItems.OfType<ListViewItem>().Where(row => row.Tag is string id && id.Contains(':')).ToList())
        {
            list.SelectedItems.Remove(row);
        }
        if (syncing)
        {
            return;
        }
        var shift = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift).HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
        var control = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control).HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
        if (shift || control || Chosen() is not [var one])
        {
            return;
        }
        window.Open(Target(one), Opening.CurrentTab);
    }

    void ListKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.OriginalSource is ListViewItem { Tag: string id } && id.StartsWith("group:", StringComparison.Ordinal) && e.Key is VirtualKey.Enter or VirtualKey.Space)
        {
            e.Handled = true;
            Fold(id);
        }
    }

    // ShowSelection selects what the focused tab shows, unless more is chosen.
    public void ShowSelection()
    {
        if (list.SelectedItems.Count > 1)
        {
            return;
        }
        syncing = true;
        if (window.ShownID is { } id && rows.TryGetValue(id, out var row))
        {
            if (!ReferenceEquals(list.SelectedItem, row))
            {
                list.SelectedItem = row;
            }
        }
        else
        {
            list.SelectedItems.Clear();
        }
        syncing = false;
    }

    // Fold folds or opens a group and keeps it in the device's list preferences.
    void Fold(string id)
    {
        var listing = Listing;
        if (Searching || listing.Preferences is not { } preferences || listing.Groups.FirstOrDefault(group => "group:" + Listing.GroupID(group.Key) == id) is not { } group)
        {
            return;
        }
        var collapsed = !group.Collapsed;
        var kept = (preferences.Collapsed ?? []).Where(key => Listing.GroupID(key) != Listing.GroupID(group.Key));
        preferences = preferences with { Collapsed = collapsed ? [.. kept, group.Key] : [.. kept] };
        listing.Preferences = preferences;
        listing.Groups = [.. listing.Groups.Select(shown => shown == group ? shown with { Collapsed = collapsed } : shown)];
        Listings.Notify(Device);
        Listings.Write(Device, application => application.SetConversationBrowserPreferences(preferences));
    }

    async void LoadMore(BindingConversationGroupKey key)
    {
        var target = Device;
        try
        {
            await Task.Run(() => window.Shell.Core.Application(target).LoadMoreConversations(key));
            Listings.Reread(target);
        }
        catch (Exception failure)
        {
            Listings.Fail(target, failure);
        }
    }

    // Rename edits the conversation's real title in place.
    public void Rename(BindingListedConversation listed)
    {
        if (!listed.Conversation.CanRename || !rows.TryGetValue(listed.Conversation.Id, out var row))
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
                window.SaveTitle(listed, title);
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
        row.Content = input;
        input.Loaded += (_, _) =>
        {
            input.Focus(FocusState.Programmatic);
            input.SelectAll();
        };
    }

    // DragStarting carries a conversation to a tab bar, a pane or out of
    // every window, or a group to another place among the groups.
    void DragStarting(object sender, DragItemsStartingEventArgs e)
    {
        DragSource.Clear();
        var ids = e.Items.OfType<ListViewItem>().Select(row => row.Tag as string).OfType<string>().ToList();
        if (ids is [var only] && only.StartsWith("group:", StringComparison.Ordinal))
        {
            if (Searching || only == "group:pinned" || Listing.Preferences is not { Grouping: not BindingConversationBrowserGrouping.None })
            {
                e.Cancel = true;
                return;
            }
            DragSource.Group = (this, only);
        }
        else if (ids.Where(id => !id.Contains(':')).Select(Listed).OfType<BindingListedConversation>().FirstOrDefault() is { } listed)
        {
            DragSource.Conversation = (Device, Target(listed));
        }
        else
        {
            e.Cancel = true;
            return;
        }
        DragSource.Carry(e.Data);
    }

    // DragCompleted opens a conversation dropped outside every window in a
    // window of its own there.
    void DragCompleted(ListViewBase sender, DragItemsCompletedEventArgs e)
    {
        if (DragSource.Conversation is var (device, target) && e.DropResult == Windows.ApplicationModel.DataTransfer.DataPackageOperation.None
            && window.Shell.WindowAt(Native.Cursor()) == null)
        {
            var at = Native.Cursor();
            var scale = Native.Scale(window);
            window.Shell.OpenConversationWindow(device, null, target, new Bounds(at.X, at.Y, (int)(720 * scale), (int)(560 * scale)));
        }
        DragSource.Clear();
    }

    // GroupAt is the group of the row under a drag: a title's, or the group
    // a conversation is in.
    BindingConversationGroupKey? GroupAt(DragEventArgs e)
    {
        var point = e.GetPosition(list);
        foreach (var row in list.Items.OfType<ListViewItem>())
        {
            if (row.ActualHeight <= 0 || row.Tag is not string id)
            {
                continue;
            }
            var top = row.TransformToVisual(list).TransformPoint(new Windows.Foundation.Point(0, 0)).Y;
            if (point.Y < top || point.Y >= top + row.ActualHeight)
            {
                continue;
            }
            if (id.StartsWith("group:", StringComparison.Ordinal))
            {
                return Listing.Groups.FirstOrDefault(group => "group:" + Listing.GroupID(group.Key) == id)?.Key;
            }
            return Listed(id)?.Group;
        }
        return null;
    }

    void GroupDragOver(object sender, DragEventArgs e)
    {
        if (DragSource.Group is not var (source, _) || source != this || GroupAt(e) is null or BindingConversationGroupKey.Pinned)
        {
            return;
        }
        e.AcceptedOperation = Windows.ApplicationModel.DataTransfer.DataPackageOperation.Move;
        e.DragUIOverride.Caption = "移到此处";
        e.DragUIOverride.IsGlyphVisible = false;
        e.Handled = true;
    }

    // GroupDrop puts the dragged group where the target group is, which
    // orders the groups by hand from now on.
    void GroupDrop(object sender, DragEventArgs e)
    {
        if (DragSource.Group is not var (source, groupID) || source != this || GroupAt(e) is not { } target || target is BindingConversationGroupKey.Pinned)
        {
            return;
        }
        e.Handled = true;
        DragSource.Clear();
        if (Listing.Groups.FirstOrDefault(group => "group:" + Listing.GroupID(group.Key) == groupID) is not { } dragged)
        {
            return;
        }
        var to = GroupOrder().FindIndex(key => Listing.GroupID(key) == Listing.GroupID(target));
        if (to >= 0)
        {
            MoveGroup(dragged.Key, to);
        }
    }

    // RenameShown renames the one chosen conversation, else the focused tab's.
    public void RenameShown()
    {
        if (Chosen() is [var one])
        {
            Rename(one);
        }
        else if (window.ShownID is { } id && Listed(id) is { } shown)
        {
            Rename(shown);
        }
    }

    // DeviceChanged empties the list for another device's.
    public void DeviceChanged()
    {
        renamingID = null;
        ClearSearch();
        rows.Clear();
        list.Items.Clear();
        Render();
    }
}
