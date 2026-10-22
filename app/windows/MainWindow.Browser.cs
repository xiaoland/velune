using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;
using Windows.ApplicationModel.DataTransfer;

namespace Velune;

// Managing the conversation list: searching it, how it groups, filters and
// sorts, organizing conversations (pin, read, project, archive), group order
// and projects, and deleting. Organization is Velune's own record of the
// list; renaming, deleting and Codex's archive act on the native session.
public sealed partial class MainWindow
{
    readonly DispatcherTimer searchTimer = new() { Interval = TimeSpan.FromMilliseconds(300) };
    // The latest search's result, while the search box has text.
    BindingConversationSearch? search;
    // Changes of the list under way, and those finished: a list read while
    // one was under way may show the state before it, so it is not shown.
    int writing;
    int written;

    void AttachBrowser()
    {
        searchTimer.Tick += (_, _) =>
        {
            searchTimer.Stop();
            Search();
        };
        SearchBox.TextChanged += (_, e) =>
        {
            if (e.Reason == AutoSuggestionBoxTextChangeReason.UserInput)
            {
                searchTimer.Stop();
                searchTimer.Start();
            }
        };
        SearchBox.QuerySubmitted += (_, _) =>
        {
            searchTimer.Stop();
            Search();
        };
        DisplayMenu.Opening += (_, _) => FillDisplayMenu();
    }

    bool Searching => search != null && SearchBox.Text.Trim().Length > 0;

    // Shown is what the list shows: the search's result while searching.
    (IReadOnlyList<BindingConversationGroup> Groups, IReadOnlyList<BindingListedConversation> Conversations) Shown() =>
        Searching
            ? (search!.Groups, search.Conversations.Where(listed => listed.Conversation.ParentConversationId == null).ToList())
            : (listing.Groups, listing.Conversations);

    // Search matches the device's latest list; an empty box shows the list.
    async void Search()
    {
        var text = SearchBox.Text.Trim();
        if (text.Length == 0)
        {
            search = null;
            Render();
            return;
        }
        var target = device;
        try
        {
            var found = await Task.Run(() => core.Application(target).SearchConversations(text));
            if (target == device && SearchBox.Text.Trim() == text)
            {
                search = found;
                Render();
            }
        }
        catch (Exception failure)
        {
            listing.Failure = Core.Describe(failure);
            Render();
        }
    }

    void ClearSearch()
    {
        SearchBox.Text = "";
        search = null;
    }

    // ApplyList shows a list the device returned after a change.
    void ApplyList(BindingConfigurationSnapshot list)
    {
        listing = Listing.From(list, [.. listing.Active.Values]);
        if (conversation != null)
        {
            conversation.Model.Configuration = listing.Configuration;
        }
        Render();
        if (Searching)
        {
            Search();
        }
    }

    // Change runs one change of the list on the device and shows the list it returns.
    async void Change(Func<VeluneApplication, BindingConfigurationSnapshot> work)
    {
        var target = device;
        writing++;
        try
        {
            var list = await Task.Run(() => work(core.Application(target)));
            if (target == device)
            {
                ApplyList(list);
            }
        }
        catch (Exception failure)
        {
            listing.Failure = Core.Describe(failure);
            Render();
        }
        writing--;
        written++;
    }

    void SavePreferences(BindingConversationBrowserPreferences preferences)
    {
        listing.Preferences = preferences;
        Change(application => application.SetConversationBrowserPreferences(preferences));
    }

    void Organize(BindingListedConversation listed, BindingConversationOrganizationChange change) =>
        Change(application => application.OrganizeConversations([new BindingConversationReference(listed.Conversation.RuntimeId, listed.Conversation.Id)], change));

    static MenuFlyoutItem MenuItem(string text, Action chosen, bool enabled = true)
    {
        var item = new MenuFlyoutItem { Text = text, IsEnabled = enabled };
        item.Click += (_, _) => chosen();
        return item;
    }

    static RadioMenuFlyoutItem Radio(string group, string text, bool chosen, Action choose)
    {
        var item = new RadioMenuFlyoutItem { Text = text, GroupName = group, IsChecked = chosen };
        item.Click += (_, _) => choose();
        return item;
    }

    static MenuFlyoutItem MenuHeading(string text) => new() { Text = text, IsEnabled = false };

    // FillDisplayMenu offers how the list groups, orders its groups and its
    // conversations, what it filters to, whether it shows the archived ones,
    // and reading it again.
    void FillDisplayMenu()
    {
        DisplayMenu.Items.Clear();
        if (listing.Preferences is not { } preferences)
        {
            DisplayMenu.Items.Add(MenuItem("重新读取", Reread));
            return;
        }
        DisplayMenu.Items.Add(MenuHeading("分组"));
        foreach (var (grouping, name) in new[] { (BindingConversationBrowserGrouping.None, "不分组"), (BindingConversationBrowserGrouping.Runtime, "按 Agent 运行时"), (BindingConversationBrowserGrouping.Project, "按项目") })
        {
            DisplayMenu.Items.Add(Radio("grouping", name, preferences.Grouping == grouping, () => SavePreferences(preferences with { Grouping = grouping })));
        }
        if (preferences.Grouping != BindingConversationBrowserGrouping.None)
        {
            DisplayMenu.Items.Add(new MenuFlyoutSeparator());
            DisplayMenu.Items.Add(MenuHeading("组排序"));
            DisplayMenu.Items.Add(Radio("groupSort", "最近活动", preferences.GroupSort == BindingConversationGroupSort.Activity,
                () => SavePreferences(preferences with { GroupSort = BindingConversationGroupSort.Activity })));
            // Turning manual fixes the order the list shows now.
            DisplayMenu.Items.Add(Radio("groupSort", "手动", preferences.GroupSort == BindingConversationGroupSort.Manual,
                () => SetManualOrder(GroupOrder())));
        }
        DisplayMenu.Items.Add(new MenuFlyoutSeparator());
        DisplayMenu.Items.Add(MenuHeading("会话排序"));
        DisplayMenu.Items.Add(Radio("sort", "更新时间", preferences.Sort == BindingConversationBrowserSort.Updated, () => SavePreferences(preferences with { Sort = BindingConversationBrowserSort.Updated })));
        DisplayMenu.Items.Add(Radio("sort", "创建时间", preferences.Sort == BindingConversationBrowserSort.Created, () => SavePreferences(preferences with { Sort = BindingConversationBrowserSort.Created })));
        DisplayMenu.Items.Add(Radio("direction", "最新在前", !preferences.OldestFirst, () => SavePreferences(preferences with { OldestFirst = false })));
        DisplayMenu.Items.Add(Radio("direction", "最早在前", preferences.OldestFirst, () => SavePreferences(preferences with { OldestFirst = true })));

        DisplayMenu.Items.Add(new MenuFlyoutSeparator());
        var runtimes = new MenuFlyoutSubItem { Text = "Agent 运行时：" + (preferences.RuntimeId is { } runtimeID ? listing.Configuration.Runtime(runtimeID)?.Name ?? "已移除" : "全部") };
        runtimes.Items.Add(Radio("runtimeFilter", "全部", preferences.RuntimeId == null, () => SavePreferences(preferences with { RuntimeId = null })));
        foreach (var runtime in listing.Configuration.Runtimes)
        {
            runtimes.Items.Add(Radio("runtimeFilter", runtime.Name, preferences.RuntimeId == runtime.Id, () => SavePreferences(preferences with { RuntimeId = runtime.Id })));
        }
        DisplayMenu.Items.Add(runtimes);
        var shownProject = preferences.Project switch
        {
            BindingConversationBrowserProject.Unspecified => "无项目",
            BindingConversationBrowserProject.Path { PathValue: var path } => ProjectName(path),
            _ => "全部",
        };
        var projects = new MenuFlyoutSubItem { Text = "项目：" + shownProject };
        projects.Items.Add(Radio("projectFilter", "全部", preferences.Project is BindingConversationBrowserProject.All,
            () => SavePreferences(preferences with { Project = new BindingConversationBrowserProject.All() })));
        projects.Items.Add(Radio("projectFilter", "无项目", preferences.Project is BindingConversationBrowserProject.Unspecified,
            () => SavePreferences(preferences with { Project = new BindingConversationBrowserProject.Unspecified() })));
        foreach (var project in listing.Projects)
        {
            var path = project.Path;
            projects.Items.Add(Radio("projectFilter", ProjectName(path), preferences.Project is BindingConversationBrowserProject.Path { } chosen && chosen.PathValue == path,
                () => SavePreferences(preferences with { Project = new BindingConversationBrowserProject.Path(path) })));
        }
        DisplayMenu.Items.Add(projects);

        DisplayMenu.Items.Add(new MenuFlyoutSeparator());
        var archived = new ToggleMenuFlyoutItem { Text = "显示已归档的会话", IsChecked = listing.ShowsArchived };
        var showing = listing.ShowsArchived;
        archived.Click += (_, _) => Change(application => application.ShowArchivedConversations(!showing));
        DisplayMenu.Items.Add(archived);
        DisplayMenu.Items.Add(MenuItem("重新读取", Reread));
    }

    // ProjectName is a project's name, else its primary folder's name.
    string ProjectName(string path) =>
        listing.Projects.FirstOrDefault(project => project.Path == path)?.Name is { Length: > 0 } name ? name : Listing.LastPart(path);

    // Reread reads the list and the open conversation again from the runtimes.
    void Reread()
    {
        PollListing();
        conversation?.Model.Load();
    }

    // GroupOrder is the order the list shows its groups in now, pinned aside.
    List<BindingConversationGroupKey> GroupOrder() =>
        [.. listing.Groups.Select(group => group.Key).Where(key => key is not BindingConversationGroupKey.Pinned)];

    // SetManualOrder keeps groups in the given order, which only the user
    // changes; the other grouping keeps its own order.
    void SetManualOrder(List<BindingConversationGroupKey> order)
    {
        if (listing.Preferences is not { } preferences)
        {
            return;
        }
        static bool Runtime(BindingConversationGroupKey key) => key is BindingConversationGroupKey.Runtime;
        var grouping = preferences.Grouping;
        var others = preferences.GroupOrder.Where(key => grouping == BindingConversationBrowserGrouping.Runtime ? !Runtime(key) : key is not BindingConversationGroupKey.Project);
        SavePreferences(preferences with { GroupSort = BindingConversationGroupSort.Manual, GroupOrder = [.. others, .. order] });
    }

    void MoveGroup(BindingConversationGroupKey key, int to)
    {
        var order = GroupOrder();
        var at = order.FindIndex(shown => Listing.GroupID(shown) == Listing.GroupID(key));
        if (at < 0)
        {
            return;
        }
        order.RemoveAt(at);
        order.Insert(Math.Clamp(to, 0, order.Count), key);
        SetManualOrder(order);
    }

    // FillGroupMenu offers a group's project and its place among the groups.
    void FillGroupMenu(MenuFlyout menu, string id)
    {
        menu.Items.Clear();
        var group = Shown().Groups.FirstOrDefault(group => "group:" + Listing.GroupID(group.Key) == id);
        if (group == null)
        {
            return;
        }
        if (group.Key is BindingConversationGroupKey.Project { Path: { } path })
        {
            menu.Items.Add(MenuItem("编辑项目…", () => EditProject(path)));
        }
        if (group.Key is not BindingConversationGroupKey.Pinned && !Searching && listing.Preferences is { Grouping: not BindingConversationBrowserGrouping.None })
        {
            var order = GroupOrder();
            var at = order.FindIndex(shown => Listing.GroupID(shown) == Listing.GroupID(group.Key));
            if (menu.Items.Count > 0)
            {
                menu.Items.Add(new MenuFlyoutSeparator());
            }
            menu.Items.Add(MenuItem("上移", () => MoveGroup(group.Key, at - 1), at > 0));
            menu.Items.Add(MenuItem("下移", () => MoveGroup(group.Key, at + 1), at >= 0 && at < order.Count - 1));
            menu.Items.Add(MenuItem("移到最前", () => MoveGroup(group.Key, 0), at > 0));
            menu.Items.Add(MenuItem("移到最后", () => MoveGroup(group.Key, order.Count), at >= 0 && at < order.Count - 1));
        }
    }

    // FillConversationMenu offers what may be done with a conversation now.
    void FillConversationMenu(MenuFlyout menu, string id)
    {
        menu.Items.Clear();
        if (Listed(id) is not { } listed)
        {
            return;
        }
        var rename = MenuItem("重命名…", () => Rename(listed), listed.Conversation.CanRename);
        rename.KeyboardAcceleratorTextOverride = "F2";
        menu.Items.Add(rename);
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(MenuItem(listed.Pinned ? "取消置顶" : "置顶", () => Organize(listed, listed.Pinned
            ? new BindingConversationOrganizationChange.Unpin()
            : new BindingConversationOrganizationChange.Pin())));
        menu.Items.Add(MenuItem(listed.Unread ? "标为已读" : "标为未读", () => Organize(listed, listed.Unread
            ? new BindingConversationOrganizationChange.MarkRead()
            : new BindingConversationOrganizationChange.MarkUnread())));
        // Moving changes only where the list shows it, not where it runs.
        var move = new MenuFlyoutSubItem { Text = "移到项目" };
        foreach (var project in listing.Projects)
        {
            var path = project.Path;
            move.Items.Add(Radio("move:" + id, ProjectName(path), listed.Project == path,
                () => Organize(listed, new BindingConversationOrganizationChange.MoveToProject(path))));
        }
        if (listed.Moved)
        {
            move.Items.Add(new MenuFlyoutSeparator());
            move.Items.Add(MenuItem("回到工作目录所在的项目", () => Organize(listed, new BindingConversationOrganizationChange.MoveToProject(null))));
        }
        if (move.Items.Count == 0)
        {
            move.Items.Add(MenuHeading("还没有项目"));
        }
        menu.Items.Add(move);
        menu.Items.Add(MenuItem(listed.Archived ? "取消归档" : "归档", () => Organize(listed, listed.Archived
            ? new BindingConversationOrganizationChange.Unarchive()
            : new BindingConversationOrganizationChange.Archive())));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(MenuItem("拷贝会话 ID", () => Copy(listed.Conversation.Id)));
        menu.Items.Add(MenuItem("拷贝链接", () => Copy($"velune://conversation?runtime={Uri.EscapeDataString(listed.Conversation.RuntimeId)}&id={Uri.EscapeDataString(listed.Conversation.Id)}")));
        if (listed.Conversation.CanDelete)
        {
            menu.Items.Add(new MenuFlyoutSeparator());
            menu.Items.Add(MenuItem("删除…", () => Delete(listed)));
        }
    }

    static void Copy(string text)
    {
        var data = new DataPackage();
        data.SetText(text);
        Clipboard.SetContent(data);
    }

    // Delete deletes the conversation in its runtime, after asking.
    async void Delete(BindingListedConversation listed)
    {
        var dialog = new ContentDialog
        {
            Title = "删除会话？",
            Content = Ui.Text("「" + listed.Conversation.Title + "」将从它的 Agent 运行时中删除，无法撤销。"),
            PrimaryButtonText = "删除",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };
        if (await ShowDialog(dialog) != ContentDialogResult.Primary)
        {
            return;
        }
        var id = listed.Conversation.Id;
        if (OpenID == id)
        {
            CloseConversation();
        }
        Change(application => application.DeleteConversation(listed.Conversation.RuntimeId, id));
    }

    // EditProject edits a project's name and folders; conversations in any
    // of its folders, and in git worktrees of its repository, belong to it.
    async void EditProject(string path)
    {
        if (listing.Projects.FirstOrDefault(project => project.Path == path) is not { } project)
        {
            return;
        }
        var folders = new List<string> { project.Path };
        folders.AddRange(project.Folders.Where(folder => folder != project.Path));
        var primary = project.Path;
        var name = new TextBox { Header = "名称", Text = project.Name ?? "", PlaceholderText = Listing.LastPart(primary) };
        var list = new ListView { SelectionMode = ListViewSelectionMode.Single, MaxHeight = 200 };
        var add = new TextBox { PlaceholderText = device.IsLocal ? "文件夹的绝对路径" : "那台设备上的绝对路径" };
        var addButton = new Button { Content = "添加" };
        var browse = new Button { Content = "浏览…", Visibility = device.IsLocal ? Visibility.Visible : Visibility.Collapsed };
        var remove = new Button { Content = "移除" };
        var makePrimary = new Button { Content = "设为主文件夹" };
        var error = Ui.Error();
        void Show()
        {
            var selected = list.SelectedIndex;
            list.Items.Clear();
            foreach (var folder in folders)
            {
                list.Items.Add(new ListViewItem { Content = Ui.TwoLines(folder, folder == primary ? "主文件夹：新会话建议的工作目录" : "") });
            }
            list.SelectedIndex = Math.Min(selected, folders.Count - 1);
            name.PlaceholderText = Listing.LastPart(primary);
        }
        void Chosen()
        {
            var folder = list.SelectedIndex >= 0 ? folders[list.SelectedIndex] : null;
            remove.IsEnabled = folder != null && folder != primary;
            makePrimary.IsEnabled = folder != null && folder != primary;
        }
        void Add(string folder)
        {
            folder = folder.Trim();
            if (folder.Length == 0 || folders.Contains(folder))
            {
                return;
            }
            if (!Path.IsPathFullyQualified(folder))
            {
                Ui.Show(error, "请填写绝对路径");
                return;
            }
            Ui.Show(error, "");
            folders.Add(folder);
            add.Text = "";
            Show();
        }
        list.SelectionChanged += (_, _) => Chosen();
        addButton.Click += (_, _) => Add(add.Text);
        browse.Click += async (_, _) =>
        {
            if (await PickFolder() is { } picked)
            {
                Add(picked);
            }
        };
        remove.Click += (_, _) =>
        {
            if (list.SelectedIndex >= 0 && folders[list.SelectedIndex] != primary)
            {
                folders.RemoveAt(list.SelectedIndex);
                Show();
            }
        };
        makePrimary.Click += (_, _) =>
        {
            if (list.SelectedIndex >= 0)
            {
                primary = folders[list.SelectedIndex];
                Show();
                Chosen();
            }
        };
        Show();
        Chosen();
        var addRow = new Grid { ColumnSpacing = 8 };
        addRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        addRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        addRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        addRow.Children.Add(add);
        Grid.SetColumn(addButton, 1);
        addRow.Children.Add(addButton);
        Grid.SetColumn(browse, 2);
        addRow.Children.Add(browse);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(makePrimary);
        actions.Children.Add(remove);
        var panel = new StackPanel { Spacing = 12, MinWidth = 440 };
        panel.Children.Add(name);
        panel.Children.Add(Ui.Labeled("文件夹", new Border { Style = Ui.Style("CardStyle"), CornerRadius = new CornerRadius(4), Child = list }));
        panel.Children.Add(actions);
        panel.Children.Add(Ui.Field(addRow, "工作目录在任一文件夹中的会话都归入此项目。", error));
        if (project.Worktrees.Length > 0)
        {
            var worktrees = new StackPanel { Spacing = 2 };
            foreach (var worktree in project.Worktrees)
            {
                worktrees.Children.Add(Ui.Muted(worktree));
            }
            panel.Children.Add(Ui.Labeled("按所属仓库归入的 Git worktree", worktrees));
        }
        panel.Children.Add(Ui.Muted("项目的名称与文件夹只是 Velune 的列表记录，不改变会话的工作目录或执行位置。"));
        var dialog = new ContentDialog
        {
            Title = "编辑项目",
            Content = new ScrollViewer { Content = panel, Padding = new Thickness(0, 0, 16, 0), MaxHeight = 520 },
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 600.0;
        if (await ShowDialog(dialog) != ContentDialogResult.Primary)
        {
            return;
        }
        var shownName = name.Text.Trim();
        var others = folders.Where(folder => folder != primary).ToArray();
        var chosenPrimary = primary;
        Change(application => application.SetProject(path, chosenPrimary, shownName.Length > 0 ? shownName : null, others));
    }
}
