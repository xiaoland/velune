using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The 资源库 window: this PC's library of instructions, skills, MCP servers
// and sub-agents. A category on the left; its resources and, under them,
// what the runtimes hold outside the library; the chosen one's editor and
// the runtimes it is enabled on to the right. There is one; opening it
// again brings it back.
sealed partial class LibraryWindow : Window
{
    const string ReadKey = "library";
    const string EffectNote = "写入后，运行时通常在新会话中加载这些内容；正在进行的会话何时使用新内容由运行时决定，Velune 不会重启会话。";

    static readonly (BindingRuntimeResourceKind Kind, string Name, string Glyph)[] Kinds =
    [
        (BindingRuntimeResourceKind.UserInstructions, "指令", "\uE8A5"),
        (BindingRuntimeResourceKind.Skills, "技能", "\uE945"),
        (BindingRuntimeResourceKind.McpServers, "MCP 服务器", "\uE968"),
        (BindingRuntimeResourceKind.NativeAgentDefinitions, "子 Agent", "\uE716"),
    ];

    readonly Core core;
    readonly NavigationView nav = new()
    {
        PaneDisplayMode = NavigationViewPaneDisplayMode.Left,
        IsSettingsVisible = false,
        IsBackButtonVisible = NavigationViewBackButtonVisible.Collapsed,
        IsPaneToggleButtonVisible = false,
        OpenPaneLength = 180,
    };
    readonly TextBox search = new() { PlaceholderText = "搜索" };
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single };
    readonly Button add = Ui.IconButton("\uE710", "新建资源");
    readonly Button remove = Ui.IconButton("\uE738", "删除资源");
    readonly Button duplicate = new() { Content = "复制" };
    readonly Button import = new() { Content = "从运行时导入…" };
    readonly ScrollViewer detail = new() { HorizontalScrollMode = ScrollMode.Disabled };
    readonly InfoBar notice = new() { IsClosable = true, Margin = new Thickness(12, 0, 12, 8) };

    BindingRuntimeResourceKind kind = BindingRuntimeResourceKind.UserInstructions;
    BindingResourceTemplateSummary[] templates = [];
    BindingResourceSyncStatus[] statuses = [];
    BindingUnmanagedRuntimeResource[] unmanaged = [];
    BindingRuntimeInstance[] runtimes = [];
    BindingGatewayConfig[] gateways = [];
    // The chosen row: a draft's key, or a runtime's entry outside the library.
    object? chosen;
    // The row the notice is about.
    object? noticed;
    bool busy;
    bool closed;
    int reads;

    public LibraryWindow(Core core)
    {
        this.core = core;
        var root = Ui.Chrome(this, "资源库");
        foreach (var (shown, name, glyph) in Kinds)
        {
            nav.MenuItems.Add(new NavigationViewItem { Content = name, Tag = shown, Icon = new FontIcon { Glyph = glyph } });
        }
        nav.SelectedItem = nav.MenuItems[0];
        nav.SelectionChanged += (_, e) =>
        {
            if (e.SelectedItemContainer?.Tag is BindingRuntimeResourceKind picked && picked != kind)
            {
                kind = picked;
                chosen = null;
                search.Text = "";
                // The library's resources show at once; what the runtimes
                // hold of this kind follows the read.
                unmanaged = [];
                ShowList();
                ShowDetail();
                Reload();
            }
        };

        var middle = new Grid { Width = 300, RowSpacing = 8, Padding = new Thickness(0, 8, 0, 12) };
        middle.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        search.Margin = new Thickness(12, 0, 12, 0);
        middle.Children.Add(search);
        Ui.Row(middle, list, new GridLength(1, GridUnitType.Star));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, Margin = new Thickness(12, 0, 12, 0) };
        actions.Children.Add(add);
        actions.Children.Add(remove);
        actions.Children.Add(duplicate);
        actions.Children.Add(import);
        Ui.Row(middle, actions, GridLength.Auto);

        var body = new Grid();
        body.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        body.Children.Add(middle);
        var right = new Grid();
        right.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        notice.Margin = new Thickness(24, 12, 24, 0);
        right.Children.Add(notice);
        Ui.Row(right, detail, new GridLength(1, GridUnitType.Star));
        var column = new Border { BorderThickness = new Thickness(1, 0, 0, 0), Style = Ui.Style("DividedStyle"), Child = right };
        Grid.SetColumn(column, 1);
        body.Children.Add(column);
        nav.Content = body;
        Ui.Row(root, nav, new GridLength(1, GridUnitType.Star));
        WorkspaceWindow.Size(this, 1040, 680, 820, 500);

        search.TextChanged += (_, _) => ShowList();
        list.SelectionChanged += (_, _) =>
        {
            if (list.SelectedItem is ListViewItem { Tag: { } tag } && !Equals(tag, chosen))
            {
                chosen = tag;
                ShowDetail();
            }
            ShowActions();
        };
        add.Click += (_, _) => Choose(Library.Add(Draft.New(kind)));
        duplicate.Click += (_, _) =>
        {
            if (Chosen is { } draft)
            {
                Choose(Library.Add(draft.Copy()));
            }
        };
        remove.Click += (_, _) => Delete();
        import.Click += (_, _) => ImportFromRuntime();
        Closed += (_, _) => closed = true;
        Activated += (_, e) =>
        {
            // Back in front, the runtimes may have changed what they hold.
            if (e.WindowActivationState != WindowActivationState.Deactivated && reads > 0)
            {
                Reload(keepDetail: true);
            }
        };
        Reload();
    }

    VeluneApplication App => core.Local;

    string KindName(BindingRuntimeResourceKind shown) => Kinds.First(entry => entry.Kind == shown).Name;

    // Chosen is the chosen resource's draft, when a resource is chosen.
    Draft? Chosen => chosen is string key && Library.Drafts.TryGetValue(key, out var draft) ? draft : null;

    string RuntimeName(string id) => runtimes.FirstOrDefault(runtime => runtime.Id == id)?.Name ?? "已移除的运行时";

    BindingResourceSyncState State(string templateId, string runtimeId) =>
        statuses.FirstOrDefault(status => status.TemplateId == templateId && status.RuntimeId == runtimeId)?.State
        ?? new BindingResourceSyncState.NotEnabled();

    // Reload reads the library, its states and the runtimes' other entries
    // of the category; keepDetail leaves an editor being typed into alone.
    async void Reload(bool keepDetail = false)
    {
        var shown = kind;
        var read = ++reads;
        try
        {
            var (snapshot, all, states, others) = await Task.Run(() =>
                (App.List(), App.ListResourceTemplates(), App.ResourceSyncStatus(), App.UnmanagedRuntimeResources(shown)));
            if (read != reads || closed)
            {
                return;
            }
            Problems.Recovered(ReadKey);
            runtimes = snapshot.RuntimeInstances;
            gateways = snapshot.Gateways;
            templates = all;
            statuses = states;
            unmanaged = others;
            // A saved draft without edits follows the library; a removed one goes.
            foreach (var (key, draft) in Library.Drafts.Where(entry => entry.Value.Id.Length > 0).ToList())
            {
                if (!templates.Any(template => template.Id == draft.Id))
                {
                    Library.Drafts.Remove(key);
                }
            }
            ShowList();
            if (!keepDetail || chosen is not string || Chosen is { Dirty: false })
            {
                ShowDetail();
            }
            else
            {
                ShowEnabled();
            }
        }
        catch (Exception failure)
        {
            if (read == reads && !closed)
            {
                Problems.Failing(ReadKey, "读取资源库", failure);
                Ui.Show(notice, "读取资源库失败，详见问题。");
            }
        }
    }

    void ShowList()
    {
        var query = search.Text.Trim();
        bool Matches(string text) => query.Length == 0 || text.Contains(query, StringComparison.CurrentCultureIgnoreCase);
        list.Items.Clear();
        foreach (var template in templates.Where(template => template.Kind == kind && Matches(template.Name)).OrderBy(template => template.Name, StringComparer.CurrentCulture))
        {
            var draft = Library.Drafts.GetValueOrDefault(template.Id);
            list.Items.Add(Row(template.Id, (draft?.Dirty == true ? "● " : "") + template.Name, Summary(template.Id)));
        }
        foreach (var (key, draft) in Library.Drafts.Where(entry => entry.Value.Id.Length == 0 && entry.Value.Kind == kind && Matches(entry.Value.Name)))
        {
            list.Items.Add(Row(key, "● " + (draft.Name.Trim() is { Length: > 0 } name ? name : "未命名"), "尚未保存"));
        }
        if (list.Items.Count == 0)
        {
            list.Items.Add(Heading(query.Length > 0 ? "没有匹配的资源。" : "资源库中的内容可以启用到多个运行时，并由 Velune 保持同步。"));
        }
        var others = unmanaged.Where(entry => Matches(entry.Path) || Matches(RuntimeName(entry.RuntimeId))).ToList();
        if (others.Count > 0)
        {
            list.Items.Add(Heading("运行时中的其他内容"));
            foreach (var entry in others)
            {
                list.Items.Add(Row(entry, entry.Path, RuntimeName(entry.RuntimeId)));
            }
        }
        list.SelectedItem = list.Items.OfType<ListViewItem>().FirstOrDefault(item => item.Tag != null && Equals(item.Tag, chosen));
        ShowActions();

        static ListViewItem Row(object tag, string title, string detail)
        {
            var row = new ListViewItem { Tag = tag, Content = Ui.TwoLines(title, detail), Padding = new Thickness(12, 6, 12, 6) };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, title.TrimStart('●', ' '));
            return row;
        }

        static ListViewItem Heading(string text) => new()
        {
            IsEnabled = false,
            IsTabStop = false,
            Padding = new Thickness(12, 12, 12, 4),
            Content = new TextBlock { Text = text, Style = Ui.Style("CaptionTextBlockStyle"), TextWrapping = TextWrapping.Wrap },
        };
    }

    // Summary tells how far a resource is enabled and whether any runtime
    // needs attention.
    string Summary(string templateId)
    {
        var mine = statuses.Where(status => status.TemplateId == templateId).Select(status => status.State).ToList();
        var enabled = mine.Count(state => state is not (BindingResourceSyncState.NotEnabled or BindingResourceSyncState.Unsupported));
        var parts = new List<string> { enabled > 0 ? $"已启用于 {enabled} 个运行时" : "未启用" };
        if (mine.Count(state => state is BindingResourceSyncState.Pending) is var pending and > 0)
        {
            parts.Add($"{pending} 个待同步");
        }
        if (mine.Any(state => state is BindingResourceSyncState.Drifted or BindingResourceSyncState.Missing))
        {
            parts.Add("原生内容有变化");
        }
        return string.Join(" · ", parts);
    }

    void ShowActions()
    {
        remove.IsEnabled = !busy && Chosen != null;
        duplicate.IsEnabled = !busy && Chosen != null;
        add.IsEnabled = !busy;
        import.IsEnabled = !busy;
    }

    // Choose shows a draft, reading the saved resource when there is no edit.
    void Choose(string key)
    {
        chosen = key;
        search.Text = "";
        ShowList();
        ShowDetail();
    }

    // Write runs one change against the library; false leaves what failed
    // in 问题 and in the window.
    async Task<T?> Write<T>(string source, Func<VeluneApplication, T> work) where T : class
    {
        busy = true;
        ShowActions();
        try
        {
            var done = await Task.Run(() => work(App));
            return closed ? null : done;
        }
        catch (Exception failure)
        {
            Problems.Report(source, failure);
            noticed = chosen;
            Ui.Show(notice, source + "失败：" + Core.Describe(failure));
            notice.Severity = InfoBarSeverity.Error;
            return null;
        }
        finally
        {
            busy = false;
            if (!closed)
            {
                ShowActions();
            }
        }
    }

    async Task<bool> Write(string source, Action<VeluneApplication> work) =>
        await Write(source, application =>
        {
            work(application);
            return (object)true;
        }) != null;

    void Inform(string message)
    {
        noticed = chosen;
        notice.Severity = InfoBarSeverity.Informational;
        Ui.Show(notice, message);
    }

    async void Delete()
    {
        if (chosen is not string key || Chosen is not { } draft)
        {
            return;
        }
        if (draft.Id.Length == 0)
        {
            Library.Drafts.Remove(key);
            chosen = null;
            ShowList();
            ShowDetail();
            return;
        }
        if (await Ask($"删除“{draft.Name}”？", "已启用它的运行时保留各自的原生内容，但不再由 Velune 同步。", "删除") != ContentDialogResult.Primary)
        {
            return;
        }
        if (await Write("删除资源“" + draft.Name + "”", application => application.DeleteResourceTemplate(draft.Id)))
        {
            Library.Drafts.Remove(key);
            chosen = null;
            Reload();
        }
    }

    // ImportFromRuntime adds, at once, what a runtime holds outside the
    // library; each stays enabled on that runtime.
    async void ImportFromRuntime()
    {
        if (runtimes.Length == 0)
        {
            Inform("还没有 Agent 运行时。");
            return;
        }
        var source = new ComboBox { Header = "运行时", MinWidth = 280 };
        foreach (var runtime in runtimes)
        {
            source.Items.Add(new ComboBoxItem { Content = runtime.Name, Tag = runtime.Id });
        }
        var candidates = new ListView { SelectionMode = ListViewSelectionMode.Multiple, MaxHeight = 320, MinWidth = 420 };
        var state = Ui.Muted("");
        var found = new Dictionary<BindingRuntimeResourceKind, BindingUnmanagedRuntimeResource[]>();
        var panel = new StackPanel { Spacing = 12 };
        panel.Children.Add(source);
        panel.Children.Add(Ui.Muted("选中的内容将加入资源库，并继续启用于该运行时；原生文件改写为资源库的形式。"));
        panel.Children.Add(candidates);
        panel.Children.Add(state);
        var dialog = new ContentDialog
        {
            Title = "从运行时导入",
            Content = panel,
            PrimaryButtonText = "导入",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
            IsPrimaryButtonEnabled = false,
        };
        async void ShowCandidates()
        {
            candidates.Items.Clear();
            if (source.SelectedItem is not ComboBoxItem { Tag: string runtimeId })
            {
                return;
            }
            state.Text = "正在读取…";
            try
            {
                if (found.Count == 0)
                {
                    foreach (var (shown, _, _) in Kinds)
                    {
                        found[shown] = await Task.Run(() => App.UnmanagedRuntimeResources(shown));
                    }
                }
                foreach (var (shown, name, _) in Kinds)
                {
                    foreach (var entry in found[shown].Where(entry => entry.RuntimeId == runtimeId))
                    {
                        var item = new ListViewItem { Tag = entry, Content = Ui.TwoLines(entry.Path, name) };
                        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, entry.Path);
                        candidates.Items.Add(item);
                    }
                }
                state.Text = candidates.Items.Count == 0 ? "没有可导入的内容。" : "";
            }
            catch (Exception failure)
            {
                state.Text = "读取失败：" + Core.Describe(failure);
            }
        }
        source.SelectionChanged += (_, _) => ShowCandidates();
        candidates.SelectionChanged += (_, _) =>
        {
            var count = candidates.SelectedItems.Count;
            dialog.IsPrimaryButtonEnabled = count > 0;
            dialog.PrimaryButtonText = count > 0 ? $"导入 {count} 项" : "导入";
        };
        source.SelectedIndex = 0;
        if (await Show(dialog) != ContentDialogResult.Primary)
        {
            return;
        }
        var chosenEntries = candidates.SelectedItems.OfType<ListViewItem>().Select(item => (BindingUnmanagedRuntimeResource)item.Tag).ToList();
        // What the library cannot hold is listed first; it is dropped.
        var notes = new List<string>();
        foreach (var entry in chosenEntries)
        {
            if (await Write("读取“" + entry.Path + "”", application => application.ImportRuntimeResource(entry.RuntimeId, entry.Kind, entry.Path)) is { Notes.Length: > 0 } preview)
            {
                notes.Add($"{entry.Path}：{string.Join("；", preview.Notes)}");
            }
        }
        if (notes.Count > 0 && await Ask("部分字段无法带入", "以下内容资源库无法表达，不会带入，原生文件将改写为资源库的形式：\n" + string.Join("\n", notes), "继续导入") != ContentDialogResult.Primary)
        {
            return;
        }
        var added = 0;
        var failed = new List<string>();
        foreach (var entry in chosenEntries)
        {
            try
            {
                await Task.Run(() => App.AdoptRuntimeResource(entry.RuntimeId, entry.Kind, entry.Path));
                added++;
            }
            catch (Exception failure)
            {
                Problems.Report("导入“" + entry.Path + "”", failure);
                failed.Add($"{entry.Path}（{Core.Describe(failure)}）");
            }
        }
        Inform(failed.Count == 0 ? $"已导入 {added} 项。" : $"已导入 {added} 项；未导入：{string.Join("、", failed)}");
        Reload();
    }

    readonly SemaphoreSlim dialogs = new(1, 1);

    async Task<ContentDialogResult> Show(ContentDialog dialog)
    {
        await dialogs.WaitAsync();
        try
        {
            dialog.XamlRoot = Content.XamlRoot;
            return await dialog.ShowAsync();
        }
        finally
        {
            dialogs.Release();
        }
    }

    // Ask is a question with one action, or two; the default is to cancel.
    Task<ContentDialogResult> Ask(string title, string message, string primary, string? secondary = null)
    {
        var dialog = new ContentDialog
        {
            Title = title,
            Content = new ScrollViewer { Content = Ui.Text(message), MaxHeight = 360 },
            PrimaryButtonText = primary,
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };
        if (secondary != null)
        {
            dialog.SecondaryButtonText = secondary;
        }
        return Show(dialog);
    }
}
