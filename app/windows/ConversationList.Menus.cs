using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;
using Windows.ApplicationModel.DataTransfer;

namespace Velune;

// Managing the conversation list: searching it, how it groups, filters and
// sorts, organizing conversations (pin, read, project, archive), group order
// and projects, and deleting. Organization is Velune's own record of the
// list; renaming, deleting and Codex's archive act on the native session.
sealed partial class ConversationList
{
    readonly DispatcherTimer searchTimer = new() { Interval = TimeSpan.FromMilliseconds(300) };
    // The latest search's result, while the search box has text.
    BindingConversationSearch? search;

    void AttachBrowser()
    {
        searchTimer.Tick += (_, _) =>
        {
            searchTimer.Stop();
            Search();
        };
        searchBox.TextChanged += (_, e) =>
        {
            if (e.Reason == AutoSuggestionBoxTextChangeReason.UserInput)
            {
                searchTimer.Stop();
                searchTimer.Start();
            }
        };
        searchBox.QuerySubmitted += (_, _) =>
        {
            searchTimer.Stop();
            Search();
        };
        displayMenu.Opening += (_, _) => FillDisplayMenu();
        Unloaded += (_, _) => searchTimer.Stop();
    }

    bool Searching => search != null && searchBox.Text.Trim().Length > 0;

    // Shown is what the list shows: the search's result while searching.
    (IReadOnlyList<BindingConversationGroup> Groups, IReadOnlyList<BindingListedConversation> Conversations) Shown() =>
        Searching
            ? (search!.Groups, search.Conversations.Where(listed => listed.Conversation.ParentConversationId == null).ToList())
            : (Listing.Groups, Listing.Conversations);

    // ListChanged shows a new list of the device and searches it again.
    public void ListChanged()
    {
        Render();
        if (Searching)
        {
            Search();
        }
    }

    // Search matches the device's latest list; an empty box shows the list.
    async void Search()
    {
        var text = searchBox.Text.Trim();
        if (text.Length == 0)
        {
            search = null;
            Render();
            return;
        }
        var target = Device;
        try
        {
            var found = await Task.Run(() => window.Shell.Core.Application(target).SearchConversations(text));
            if (target == Device && searchBox.Text.Trim() == text)
            {
                search = found;
                Render();
            }
        }
        catch (Exception failure)
        {
            Problems.Report("搜索会话", failure, target);
        }
    }

    void ClearSearch()
    {
        searchBox.Text = "";
        search = null;
    }

    void Change(string source, Func<VeluneApplication, BindingConfigurationSnapshot> work) => Listings.Change(Device, source, work);

    void SavePreferences(BindingConversationBrowserPreferences preferences)
    {
        Listing.Preferences = preferences;
        Change("更改会话列表的显示", application => application.SetConversationBrowserPreferences(preferences));
    }

    void Organize(List<BindingListedConversation> chosen, BindingConversationOrganizationChange change)
    {
        var references = chosen.Select(listed => new BindingConversationReference(listed.Conversation.RuntimeId, listed.Conversation.Id)).ToArray();
        Change("整理会话", application => application.OrganizeConversations(references, change));
    }

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
    // reading it again, and where the list is.
    void FillDisplayMenu()
    {
        displayMenu.Items.Clear();
        if (Listing.Preferences is { } preferences)
        {
            FillBrowsing(preferences);
            displayMenu.Items.Add(new MenuFlyoutSeparator());
        }
        displayMenu.Items.Add(MenuItem("重新读取", window.Reread));
        displayMenu.Items.Add(new MenuFlyoutSeparator());
        displayMenu.Items.Add(window.Tools.PlaceMenu(Tool.List, "布局"));
    }

    void FillBrowsing(BindingConversationBrowserPreferences preferences)
    {
        displayMenu.Items.Add(MenuHeading("分组"));
        foreach (var (grouping, name) in new[] { (BindingConversationBrowserGrouping.None, "不分组"), (BindingConversationBrowserGrouping.Runtime, "按 Agent 运行时"), (BindingConversationBrowserGrouping.Project, "按项目") })
        {
            displayMenu.Items.Add(Radio("grouping", name, preferences.Grouping == grouping, () => SavePreferences(preferences with { Grouping = grouping })));
        }
        if (preferences.Grouping != BindingConversationBrowserGrouping.None)
        {
            displayMenu.Items.Add(new MenuFlyoutSeparator());
            displayMenu.Items.Add(MenuHeading("组排序"));
            displayMenu.Items.Add(Radio("groupSort", "最近活动", preferences.GroupSort == BindingConversationGroupSort.Activity,
                () => SavePreferences(preferences with { GroupSort = BindingConversationGroupSort.Activity })));
            // Turning manual fixes the order the list shows now.
            displayMenu.Items.Add(Radio("groupSort", "手动", preferences.GroupSort == BindingConversationGroupSort.Manual,
                () => SetManualOrder(GroupOrder())));
        }
        displayMenu.Items.Add(new MenuFlyoutSeparator());
        displayMenu.Items.Add(MenuHeading("会话排序"));
        displayMenu.Items.Add(Radio("sort", "更新时间", preferences.Sort == BindingConversationBrowserSort.Updated, () => SavePreferences(preferences with { Sort = BindingConversationBrowserSort.Updated })));
        displayMenu.Items.Add(Radio("sort", "创建时间", preferences.Sort == BindingConversationBrowserSort.Created, () => SavePreferences(preferences with { Sort = BindingConversationBrowserSort.Created })));
        displayMenu.Items.Add(Radio("direction", "最新在前", !preferences.OldestFirst, () => SavePreferences(preferences with { OldestFirst = false })));
        displayMenu.Items.Add(Radio("direction", "最早在前", preferences.OldestFirst, () => SavePreferences(preferences with { OldestFirst = true })));

        displayMenu.Items.Add(new MenuFlyoutSeparator());
        var runtimes = new MenuFlyoutSubItem { Text = "Agent 运行时：" + (preferences.RuntimeId is { } runtimeID ? Listing.Configuration.Runtime(runtimeID)?.Name ?? "已移除" : "全部") };
        runtimes.Items.Add(Radio("runtimeFilter", "全部", preferences.RuntimeId == null, () => SavePreferences(preferences with { RuntimeId = null })));
        foreach (var runtime in Listing.Configuration.Runtimes)
        {
            runtimes.Items.Add(Radio("runtimeFilter", runtime.Name, preferences.RuntimeId == runtime.Id, () => SavePreferences(preferences with { RuntimeId = runtime.Id })));
        }
        displayMenu.Items.Add(runtimes);
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
        foreach (var project in Listing.Projects)
        {
            var path = project.Path;
            projects.Items.Add(Radio("projectFilter", ProjectName(path), preferences.Project is BindingConversationBrowserProject.Path { } chosen && chosen.PathValue == path,
                () => SavePreferences(preferences with { Project = new BindingConversationBrowserProject.Path(path) })));
        }
        displayMenu.Items.Add(projects);

        displayMenu.Items.Add(new MenuFlyoutSeparator());
        var archived = new ToggleMenuFlyoutItem { Text = "显示已归档的会话", IsChecked = Listing.ShowsArchived };
        var showing = Listing.ShowsArchived;
        archived.Click += (_, _) => Change("显示已归档的会话", application => application.ShowArchivedConversations(!showing));
        displayMenu.Items.Add(archived);
    }

    // ProjectName is a project's name, else its primary folder's name.
    string ProjectName(string path) =>
        Listing.Projects.FirstOrDefault(project => project.Path == path)?.Name is { Length: > 0 } name ? name : Listing.LastPart(path);

    // GroupOrder is the order the list shows its groups in now, pinned aside.
    List<BindingConversationGroupKey> GroupOrder() =>
        [.. Listing.Groups.Select(group => group.Key).Where(key => key is not BindingConversationGroupKey.Pinned)];

    // SetManualOrder keeps groups in the given order, which only the user
    // changes; the other grouping keeps its own order.
    void SetManualOrder(List<BindingConversationGroupKey> order)
    {
        if (Listing.Preferences is not { } preferences)
        {
            return;
        }
        var grouping = preferences.Grouping;
        var others = preferences.GroupOrder.Where(key => grouping == BindingConversationBrowserGrouping.Runtime
            ? key is not BindingConversationGroupKey.Runtime
            : key is not BindingConversationGroupKey.Project);
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
        if (group.Key is not BindingConversationGroupKey.Pinned && !Searching && Listing.Preferences is { Grouping: not BindingConversationBrowserGrouping.None })
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
        if (menu.Items.Count == 0)
        {
            menu.Items.Add(MenuHeading(group.Key is BindingConversationGroupKey.Pinned ? "置顶的会话总在最前" : "不分组时没有组排序"));
        }
    }

    // FillConversationMenu offers what may be done now with the conversation,
    // or with every chosen one when it is among them.
    void FillConversationMenu(MenuFlyout menu, string id)
    {
        menu.Items.Clear();
        if (Listed(id) is not { } clicked)
        {
            return;
        }
        var chosen = Chosen();
        if (!chosen.Any(listed => listed.Conversation.Id == id))
        {
            chosen = [clicked];
        }
        if (chosen is [var one])
        {
            var target = Target(one);
            menu.Items.Add(MenuItem("在新标签页中打开", () => window.Open(target, Opening.NewTab)));
            menu.Items.Add(MenuItem("在右侧拆分中打开", () => window.Open(target, Opening.SplitRight)));
            menu.Items.Add(MenuItem("在下方拆分中打开", () => window.Open(target, Opening.SplitDown)));
            menu.Items.Add(MenuItem("在新窗口中打开", () => window.Shell.OpenConversationWindow(Device, null, target, null)));
            menu.Items.Add(new MenuFlyoutSeparator());
            var rename = MenuItem("重命名…", () => Rename(one), one.Conversation.CanRename);
            rename.KeyboardAcceleratorTextOverride = "F2";
            menu.Items.Add(rename);
            menu.Items.Add(new MenuFlyoutSeparator());
        }
        else
        {
            menu.Items.Add(MenuHeading($"已选择 {chosen.Count} 个会话"));
        }
        var pinned = chosen.All(listed => listed.Pinned);
        menu.Items.Add(MenuItem(pinned ? "取消置顶" : "置顶", () => Organize(chosen, pinned
            ? new BindingConversationOrganizationChange.Unpin()
            : new BindingConversationOrganizationChange.Pin())));
        var unread = chosen.Any(listed => listed.Unread);
        menu.Items.Add(MenuItem(unread ? "标为已读" : "标为未读", () => Organize(chosen, unread
            ? new BindingConversationOrganizationChange.MarkRead()
            : new BindingConversationOrganizationChange.MarkUnread())));
        // Moving changes only where the list shows it, not where it runs.
        var move = new MenuFlyoutSubItem { Text = "移到项目" };
        foreach (var project in Listing.Projects)
        {
            var path = project.Path;
            move.Items.Add(Radio("move:" + id, ProjectName(path), chosen.All(listed => listed.Project == path),
                () => Organize(chosen, new BindingConversationOrganizationChange.MoveToProject(path))));
        }
        if (chosen.Any(listed => listed.Moved))
        {
            move.Items.Add(new MenuFlyoutSeparator());
            move.Items.Add(MenuItem("回到工作目录所在的项目", () => Organize(chosen, new BindingConversationOrganizationChange.MoveToProject(null))));
        }
        if (move.Items.Count == 0)
        {
            move.Items.Add(MenuHeading("还没有项目"));
        }
        menu.Items.Add(move);
        var archived = chosen.All(listed => listed.Archived);
        menu.Items.Add(MenuItem(archived ? "取消归档" : "归档", () => Organize(chosen, archived
            ? new BindingConversationOrganizationChange.Unarchive()
            : new BindingConversationOrganizationChange.Archive())));
        if (chosen is [var single])
        {
            menu.Items.Add(new MenuFlyoutSeparator());
            menu.Items.Add(MenuItem("拷贝会话 ID", () => Copy(single.Conversation.Id)));
            menu.Items.Add(MenuItem("拷贝链接", () => Copy($"velune://conversation?runtime={Uri.EscapeDataString(single.Conversation.RuntimeId)}&id={Uri.EscapeDataString(single.Conversation.Id)}")));
        }
        if (chosen.All(listed => listed.Conversation.CanDelete))
        {
            menu.Items.Add(new MenuFlyoutSeparator());
            menu.Items.Add(MenuItem("删除…", () => Delete(chosen)));
        }
    }

    static void Copy(string text)
    {
        var data = new DataPackage();
        data.SetText(text);
        Clipboard.SetContent(data);
    }

    // Delete deletes the conversations in their runtimes, after asking, and
    // closes the tabs that show them.
    async void Delete(List<BindingListedConversation> chosen)
    {
        var dialog = new ContentDialog
        {
            Title = chosen.Count == 1 ? "删除会话？" : $"删除 {chosen.Count} 个会话？",
            Content = Ui.Text((chosen.Count == 1 ? "「" + chosen[0].Conversation.Title + "」" : "所选会话") + "将从它的 Agent 运行时中删除，无法撤销。"),
            PrimaryButtonText = "删除",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };
        if (await window.ShowDialog(dialog) != ContentDialogResult.Primary)
        {
            return;
        }
        foreach (var listed in chosen)
        {
            window.Shell.Deleted(Device, listed.Conversation.Id);
            var runtimeID = listed.Conversation.RuntimeId;
            var id = listed.Conversation.Id;
            Change("删除会话", application => application.DeleteConversation(runtimeID, id));
        }
    }

    // EditProject edits a project's name and folders; conversations in any
    // of its folders, and in git worktrees of its repository, belong to it.
    async void EditProject(string path)
    {
        if (Listing.Projects.FirstOrDefault(project => project.Path == path) is not { } project)
        {
            return;
        }
        var folders = new List<string> { project.Path };
        folders.AddRange(project.Folders.Where(folder => folder != project.Path));
        var primary = project.Path;
        var name = new TextBox { Header = "名称", Text = project.Name ?? "", PlaceholderText = Listing.LastPart(primary) };
        var folderList = new ListView { SelectionMode = ListViewSelectionMode.Single, MaxHeight = 200 };
        var local = Device.IsLocal;
        var add = new TextBox { PlaceholderText = local ? "文件夹的绝对路径" : "那台设备上的绝对路径" };
        var addButton = new Button { Content = "添加" };
        var browse = new Button { Content = "浏览…", Visibility = local ? Visibility.Visible : Visibility.Collapsed };
        var remove = new Button { Content = "移除" };
        var makePrimary = new Button { Content = "设为主文件夹" };
        var error = Ui.Error();
        void Show()
        {
            var selected = folderList.SelectedIndex;
            folderList.Items.Clear();
            foreach (var folder in folders)
            {
                folderList.Items.Add(new ListViewItem { Content = Ui.TwoLines(folder, folder == primary ? "主文件夹：新会话建议的工作目录" : "") });
            }
            folderList.SelectedIndex = Math.Min(selected, folders.Count - 1);
            name.PlaceholderText = Listing.LastPart(primary);
        }
        void Chosen()
        {
            var folder = folderList.SelectedIndex >= 0 ? folders[folderList.SelectedIndex] : null;
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
        folderList.SelectionChanged += (_, _) => Chosen();
        addButton.Click += (_, _) => Add(add.Text);
        browse.Click += async (_, _) =>
        {
            try
            {
                if (await window.PickFolder() is { } picked)
                {
                    Add(picked);
                }
            }
            catch (Exception failure)
            {
                Ui.Show(error, failure.Message);
            }
        };
        remove.Click += (_, _) =>
        {
            if (folderList.SelectedIndex >= 0 && folders[folderList.SelectedIndex] != primary)
            {
                folders.RemoveAt(folderList.SelectedIndex);
                Show();
            }
        };
        makePrimary.Click += (_, _) =>
        {
            if (folderList.SelectedIndex >= 0)
            {
                primary = folders[folderList.SelectedIndex];
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
        panel.Children.Add(Ui.Labeled("文件夹", new Border { Style = Ui.Style("CardStyle"), CornerRadius = new CornerRadius(4), Child = folderList }));
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
        if (await window.ShowDialog(dialog) != ContentDialogResult.Primary)
        {
            return;
        }
        var shownName = name.Text.Trim();
        var others = folders.Where(folder => folder != primary).ToArray();
        var chosenPrimary = primary;
        Change("保存项目", application => application.SetProject(path, chosenPrimary, shownName.Length > 0 ? shownName : null, others));
    }
}
