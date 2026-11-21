using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.System;

namespace Velune;

// A history browser owns its client view and selection. Its archive flag,
// filters and pagination never replace the workspace's conversation list.
sealed class ArchivedConversationsWindow : Window
{
    readonly Shell shell;
    readonly Device device;
    readonly string viewId = "archived-" + Guid.NewGuid().ToString("N");
    readonly AutoSuggestBox search = new() { PlaceholderText = "搜索归档会话", QueryIcon = new SymbolIcon(Symbol.Find), MinWidth = 220 };
    readonly DropDownButton display = new() { Content = "显示" };
    readonly MenuFlyout displayMenu = new();
    readonly Button refresh = Ui.IconButton("\uE72C", "刷新 / 重试");
    readonly Button open = new() { Content = "打开会话" };
    readonly Button restore = new() { Content = "取消归档" };
    readonly Button delete = new() { Content = "删除…" };
    readonly Button cancel = new() { Content = "取消排队", Visibility = Visibility.Collapsed };
    readonly TextBlock status = new() { Style = Ui.Style("SecondaryTextStyle"), TextWrapping = TextWrapping.Wrap };
    readonly InfoBar error = new() { IsClosable = false, Severity = InfoBarSeverity.Error };
    readonly ProgressRing spinner = new() { Width = 20, Height = 20 };
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Extended };
    readonly DispatcherTimer searchTimer = new() { Interval = TimeSpan.FromMilliseconds(300) };
    readonly DispatcherTimer pollTimer = new() { Interval = TimeSpan.FromSeconds(5) };
    readonly Queue<(ListedConversation Conversation, bool Delete)> pending = new();
    VeluneApplication? application;
    Listing listing = Listing.Loading;
    ConversationSearch? found;
    bool closed;
    bool loading;
    bool working;
    bool dialogOpen;
    bool managementFailed;
    string? processingId;
    int revision;

    public ArchivedConversationsWindow(Shell shell, Device device)
    {
        this.shell = shell;
        this.device = device;
        var root = Ui.Chrome(this, "已归档会话（" + shell.Hub.DeviceName(device) + "）");
        var header = new Grid { Padding = new Thickness(16, 0, 16, 12), ColumnSpacing = 8 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.Children.Add(search);
        foreach (var control in new FrameworkElement[] { display, refresh })
        {
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            Grid.SetColumn(control, header.ColumnDefinitions.Count - 1);
            header.Children.Add(control);
        }
        Ui.Row(root, header, GridLength.Auto);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Margin = new Thickness(16, 0, 16, 12) };
        foreach (var control in new FrameworkElement[] { open, restore, delete, cancel, spinner }) actions.Children.Add(control);
        Ui.Row(root, actions, GridLength.Auto);
        status.Margin = new Thickness(16, 0, 16, 12);
        error.Margin = new Thickness(16, 0, 16, 12);
        Ui.Row(root, error, GridLength.Auto);
        Ui.Row(root, status, GridLength.Auto);
        Ui.Row(root, Ui.Divided(list), new GridLength(1, GridUnitType.Star));
        WorkspaceWindow.Size(this, 800, 620, 560, 420);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(search, "搜索归档会话");
        display.Flyout = displayMenu;
        displayMenu.Opening += (_, _) => FillDisplayMenu();
        list.SelectionChanged += (_, _) => ShowActions();
        list.KeyDown += (_, e) => { if (e.Key == VirtualKey.Delete) { e.Handled = true; Delete(); } };
        open.Click += (_, _) => OpenConversation();
        restore.Click += (_, _) => Enqueue(false);
        delete.Click += (_, _) => Delete();
        cancel.Click += (_, _) => { pending.Clear(); ShowActions(); };
        refresh.Click += (_, _) => { managementFailed = false; Load(); };
        search.TextChanged += (_, e) =>
        {
            if (e.Reason != AutoSuggestionBoxTextChangeReason.UserInput) return;
            revision++;
            searchTimer.Stop();
            searchTimer.Start();
        };
        search.QuerySubmitted += (_, _) => { searchTimer.Stop(); Load(); };
        searchTimer.Tick += (_, _) => { searchTimer.Stop(); Load(); };
        pollTimer.Tick += (_, _) => Load();
        root.Loaded += (_, _) => { pollTimer.Start(); Load(); };
        Closed += (_, _) =>
        {
            closed = true;
            revision++;
            searchTimer.Stop();
            pollTimer.Stop();
            // Accepted management work outlives this browser. Only the
            // explicit cancel action removes work that has not started.
            // The in-flight operation owns the handle until its continuation.
            if (!loading && !working) Release();
        };
        ShowActions();
    }

    Task<T> Call<T>(NativeOperation operation, Func<VeluneApplication, T> work) =>
        shell.Core.Telemetry.Operation(operation, carrier => Task.Run(() =>
        {
            using var traced = application!.WithTraceContext(carrier);
            return work(traced);
        }));

    void Release()
    {
        var released = application;
        application = null;
        if (released != null) _ = Task.Run(() => { try { released.ReleaseView(); } catch { } finally { released.Dispose(); } });
    }

    async void Load(Func<VeluneApplication, ConfigurationSnapshot>? change = null)
    {
        if (closed || loading || working) return;
        loading = true;
        var asked = ++revision;
        var text = search.Text.Trim();
        ShowActions();
        try
        {
            application ??= await shell.Core.Call(device, NativeOperation.ConfigurationRead, source => source.WithClientView(viewId));
            if (closed) return;
            var result = await Call(NativeOperation.ConfigurationRead, source =>
            {
                var snapshot = change != null ? change(source) : source.ShowArchivedConversations(true);
                var activity = source.ConversationActivity(0);
                var matches = text.Length > 0 ? source.SearchConversations(text) : null;
                return (Listing: Listing.From(snapshot, activity.Active), Search: matches);
            });
            if (closed || asked != revision) return;
            listing = result.Listing;
            found = result.Search;
            if (!managementFailed) error.IsOpen = false;
            Render();
        }
        catch (Exception failure)
        {
            if (!closed)
            {
                error.Message = "读取失败：" + failure.Message + "。可点刷新重试。";
                error.IsOpen = true;
                status.Text = "无法更新已归档会话；保留上次读取的列表。";
                Problems.Report("读取已归档会话", failure, device);
            }
        }
        finally
        {
            loading = false;
            if (closed) { if (!working) Release(); }
            else { ShowActions(); if (asked != revision) Load(); }
        }
    }

    IReadOnlyList<ListedConversation> Conversations => found?.Conversations ?? (IReadOnlyList<ListedConversation>)listing.Conversations;
    IReadOnlyList<ConversationGroup> Groups => found?.Groups ?? listing.Groups;
    List<ListedConversation> Chosen() => [.. list.SelectedItems.OfType<ListViewItem>().Select(row => row.Tag).OfType<ListedConversation>()];
    bool Running(ListedConversation item) => listing.Active.Values.Any(active =>
        (active.ConversationId == item.Conversation.Id || active.ParentConversationId == item.Conversation.Id)
        && active.RunState == RunState.Running);
    bool CanDelete(ListedConversation item) => item.Conversation.CanDelete && !Running(item);

    void Render()
    {
        var selected = Chosen().Select(item => item.Conversation.Id).ToHashSet();
        list.Items.Clear();
        var items = Conversations.Where(item => item.Archived && item.Conversation.ParentConversationId == null).ToList();
        foreach (var group in Groups)
        {
            if (listing.Preferences?.Grouping != ConversationBrowserGrouping.None || group.Key is ConversationGroupKey.Pinned)
                list.Items.Add(new ListViewItem { Content = Ui.Text(listing.GroupTitle(group), "BodyStrongTextBlockStyle"), IsEnabled = false });
            foreach (var item in items.Where(item => Listing.GroupID(item.Group) == Listing.GroupID(group.Key)))
            {
                var runtime = listing.Configuration.Runtime(item.Conversation.RuntimeId)?.Name ?? item.Conversation.RuntimeId;
                var project = item.Project is { } path ? ProjectName(path) : "无项目";
                var detail = runtime + "，" + project + (Running(item) ? "，正在运行" : "");
                var row = new ListViewItem { Tag = item, Content = Ui.TwoLines(item.Conversation.Title.DisplayText(), detail) };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, item.Conversation.Title.DisplayText() + "，" + detail);
                list.Items.Add(row);
                if (selected.Contains(item.Conversation.Id)) list.SelectedItems.Add(row);
            }
            if (found == null && group.Loaded < group.Total)
            {
                var key = group.Key;
                var more = new Button { Content = $"加载更多（{group.Loaded} / {group.Total}）", Margin = new Thickness(12, 4, 12, 8) };
                more.Click += (_, _) => Load(source => source.LoadMoreConversations(key));
                list.Items.Add(new ListViewItem { Content = more, IsTabStop = false });
            }
        }
        status.Text = items.Count == 0 ? (search.Text.Trim().Length > 0 ? "没有匹配的已归档会话。" : "没有符合筛选的已归档会话。") : $"已显示 {items.Count} 个已归档会话";
        ShowActions();
    }

    void ShowActions()
    {
        if (closed) return;
        var chosen = Chosen();
        open.IsEnabled = !loading && chosen.Count == 1;
        restore.IsEnabled = !loading && chosen.Count > 0;
        delete.IsEnabled = !loading && !dialogOpen && chosen.Count > 0 && chosen.All(CanDelete);
        display.IsEnabled = !loading && !working && listing.Loaded;
        refresh.IsEnabled = !loading && !working;
        spinner.IsActive = loading || working;
        spinner.Visibility = loading || working ? Visibility.Visible : Visibility.Collapsed;
        cancel.Visibility = pending.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (working) status.Text = $"正在处理，另有 {pending.Count} 个会话排队；取消排队只取消尚未开始的操作。";
        else if (!listing.Loaded && !error.IsOpen) status.Text = "正在读取已归档会话…";
    }

    void OpenConversation()
    {
        if (closed || loading || Chosen() is not [var item]) return;
        shell.OpenConversationWindow(device, null,
            new Target(item.Conversation.RuntimeId, item.Conversation.Id, item.Conversation.Title.DisplayText()), null);
    }

    async void Delete()
    {
        var chosen = Chosen();
        if (closed || loading || dialogOpen || chosen.Count == 0 || !chosen.All(CanDelete)) return;
        dialogOpen = true;
        ShowActions();
        try
        {
            var dialog = new ContentDialog
            {
                XamlRoot = Content.XamlRoot,
                RequestedTheme = ((FrameworkElement)Content).ActualTheme,
                Title = chosen.Count == 1 ? "删除已归档会话？" : $"删除 {chosen.Count} 个已归档会话？",
                Content = Ui.Text("所选会话将从它的 Agent 运行时中删除，无法撤销。"),
                PrimaryButtonText = "删除", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
            };
            if (await dialog.ShowAsync() == ContentDialogResult.Primary && !closed)
                Queue(chosen, true);
        }
        catch (Exception failure) { if (!closed) ShowError("确认删除", failure); }
        finally { dialogOpen = false; ShowActions(); }
    }

    void Enqueue(bool deleting) => Queue(Chosen(), deleting);
    void Queue(List<ListedConversation> chosen, bool deleting)
    {
        if (closed || loading) return;
        foreach (var item in chosen)
        {
            if ((!deleting || CanDelete(item)) && item.Conversation.Id != processingId && !pending.Any(work => work.Conversation.Conversation.Id == item.Conversation.Id))
                pending.Enqueue((item, deleting));
        }
        Drain();
    }

    async void Drain()
    {
        if (working || closed || application == null) return;
        working = true;
        try
        {
            while (pending.TryDequeue(out var work))
            {
                ShowActions();
                var item = work.Conversation;
                processingId = item.Conversation.Id;
                try
                {
                    var snapshot = await Call(NativeOperation.ConversationWrite, source =>
                    {
                        // Refresh run state when queued work reaches the device.
                        if (work.Delete)
                        {
                            var active = source.ConversationActivity(0).Active;
                            if (!item.Conversation.CanDelete || active.Any(state => (state.ConversationId == item.Conversation.Id || state.ParentConversationId == item.Conversation.Id) && state.RunState == RunState.Running))
                                throw new InvalidOperationException("正在运行的会话不能删除");
                            return source.DeleteConversation(item.Conversation.RuntimeId, item.Conversation.Id);
                        }
                        return source.OrganizeConversations([new ConversationReference(item.Conversation.RuntimeId, item.Conversation.Id, item.Conversation.NativeReadState)], new ConversationOrganizationChange.Unarchive());
                    });
                    if (work.Delete) shell.Deleted(device, item.Conversation.Id);
                    shell.Listings.Reread(device);
                    if (closed) continue;
                    listing = Listing.From(snapshot, [.. listing.Active.Values]);
                    // Search rows may include sessions outside the loaded page.
                    if (found != null) found = found with { Conversations = [.. found.Conversations.Where(row => row.Conversation.Id != item.Conversation.Id)] };
                    Render();
                }
                catch (Exception failure)
                {
                    var source = work.Delete ? "删除会话" : "取消归档";
                    if (closed) Problems.Report(source, failure, device);
                    else ShowError(source, failure);
                }
                finally { processingId = null; }
            }
        }
        finally
        {
            working = false;
            if (closed) { if (!loading) Release(); }
            else { ShowActions(); Load(); }
        }
    }

    void ShowError(string source, Exception failure)
    {
        managementFailed = true;
        error.Message = source + "失败：" + failure.Message;
        error.IsOpen = true;
        Problems.Report(source, failure, device);
    }

    string ProjectName(string path) => listing.Projects.FirstOrDefault(project => project.Path == path)?.Name is { Length: > 0 } name ? name : Listing.LastPart(path);
    void Save(ConversationBrowserPreferences preferences) => Load(source => source.SetConversationBrowserPreferences(preferences));
    RadioMenuFlyoutItem Choice(string group, string title, bool selected, Action action)
    {
        var item = new RadioMenuFlyoutItem { Text = title, GroupName = viewId + group, IsChecked = selected };
        item.Click += (_, _) => action();
        return item;
    }

    void FillDisplayMenu()
    {
        displayMenu.Items.Clear();
        if (listing.Preferences is not { } preferences) return;
        var grouping = new MenuFlyoutSubItem { Text = "分组" };
        foreach (var (value, name) in new[] { (ConversationBrowserGrouping.None, "不分组"), (ConversationBrowserGrouping.Runtime, "按 Agent 运行时"), (ConversationBrowserGrouping.Project, "按项目") })
            grouping.Items.Add(Choice("group", name, preferences.Grouping == value, () => Save(preferences with { Grouping = value, Collapsed = [] })));
        displayMenu.Items.Add(grouping);
        var runtimes = new MenuFlyoutSubItem { Text = "Agent 运行时" };
        runtimes.Items.Add(Choice("runtime", "全部", preferences.RuntimeId == null, () => Save(preferences with { RuntimeId = null })));
        foreach (var runtime in listing.Configuration.Runtimes)
            runtimes.Items.Add(Choice("runtime", runtime.Name, preferences.RuntimeId == runtime.Id, () => Save(preferences with { RuntimeId = runtime.Id })));
        displayMenu.Items.Add(runtimes);
        var projects = new MenuFlyoutSubItem { Text = "项目" };
        projects.Items.Add(Choice("project", "全部", preferences.Project is ConversationBrowserProject.All, () => Save(preferences with { Project = new ConversationBrowserProject.All() })));
        projects.Items.Add(Choice("project", "无项目", preferences.Project is ConversationBrowserProject.Unspecified, () => Save(preferences with { Project = new ConversationBrowserProject.Unspecified() })));
        foreach (var project in listing.Projects)
        {
            var path = project.Path;
            projects.Items.Add(Choice("project", ProjectName(path), preferences.Project is ConversationBrowserProject.Path chosen && chosen.PathValue == path, () => Save(preferences with { Project = new ConversationBrowserProject.Path(path) })));
        }
        displayMenu.Items.Add(projects);
        var sort = new MenuFlyoutSubItem { Text = "排序" };
        sort.Items.Add(Choice("sort", "更新时间", preferences.Sort == ConversationBrowserSort.Updated, () => Save(preferences with { Sort = ConversationBrowserSort.Updated })));
        sort.Items.Add(Choice("sort", "创建时间", preferences.Sort == ConversationBrowserSort.Created, () => Save(preferences with { Sort = ConversationBrowserSort.Created })));
        sort.Items.Add(new MenuFlyoutSeparator());
        sort.Items.Add(Choice("direction", "最新在前", !preferences.OldestFirst, () => Save(preferences with { OldestFirst = false })));
        sort.Items.Add(Choice("direction", "最早在前", preferences.OldestFirst, () => Save(preferences with { OldestFirst = true })));
        displayMenu.Items.Add(sort);
    }
}
