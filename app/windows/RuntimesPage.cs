using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// This PC's Agent runtime instances: a list to add, import, edit and delete them.
sealed partial class RuntimesPage : UserControl
{
    const string DefaultGateway = "default";

    readonly Core core;
    readonly SettingsWindow window;
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    // Tells a runtime's type is no longer supported; failures go to 问题.
    readonly InfoBar failure = Ui.FailureBar();
    readonly Button add = Ui.IconButton("\uE710", "添加 Agent 运行时实例");
    readonly Button remove = Ui.IconButton("\uE738", "删除运行时实例");
    readonly Button import = new() { Content = "快速导入…" };
    readonly Button edit = new() { Content = "编辑…" };
    // Offers the library's resources to a runtime just added.
    readonly InfoBar offer = new() { Severity = InfoBarSeverity.Informational, IsClosable = true };
    BindingRuntimeInstance[] instances = [];
    bool listed;
    BindingRuntimeTypeDescriptor[] kinds = [];
    bool busy;

    internal RuntimesPage(Core core, SettingsWindow window, Action openLibrary)
    {
        this.core = core;
        this.window = window;
        var library = new Button { Content = "打开资源库" };
        library.Click += (_, _) =>
        {
            offer.IsOpen = false;
            openLibrary();
        };
        offer.ActionButton = library;
        add.Click += (_, _) => Edit(null);
        remove.Click += (_, _) => Delete();
        import.Click += (_, _) => Import();
        edit.Click += (_, _) => Edit(Chosen());
        list.SelectionChanged += (_, _) => ShowActions();
        list.DoubleTapped += (_, _) => Edit(Chosen());
        list.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter && Chosen() != null)
            {
                e.Handled = true;
                Edit(Chosen());
            }
        };
        Content = Ui.ListPage("Agent 运行时", new StackPanel { Spacing = 8, Children = { failure, offer } }, list, [add, import, remove], [edit]);
        Reload();
    }

    BindingRuntimeInstance? Chosen() => list.SelectedIndex is var index and >= 0 && index < instances.Length ? instances[index] : null;

    void ShowActions()
    {
        add.IsEnabled = Addable().Any();
        remove.IsEnabled = Chosen() != null && !busy;
        import.IsEnabled = !busy;
        edit.IsEnabled = Chosen() != null;
    }

    async void Reload()
    {
        try
        {
            var snapshot = await Task.Run(core.Local.List);
            kinds = snapshot.RuntimeTypes;
            SetInstances(snapshot.RuntimeInstances);
        }
        catch (Exception problem)
        {
            Problems.Report("读取 Agent 运行时", problem);
        }
    }

    void SetInstances(BindingRuntimeInstance[] shown)
    {
        var selected = Chosen()?.Id;
        var added = listed ? shown.Where(runtime => !instances.Any(known => known.Id == runtime.Id)).ToList() : [];
        listed = true;
        instances = shown;
        if (added.Count > 0)
        {
            Offer(added);
        }
        list.Items.Clear();
        foreach (var runtime in instances)
        {
            var row = Ui.TwoLines(runtime.Name, Summary(runtime));
            row.Padding = new Thickness(4, 6, 4, 6);
            var item = new ListViewItem { Content = row };
            item.ContextFlyout = Ui.Menu(
                Ui.MenuItem("编辑…", () => Edit(runtime)),
                Ui.MenuItem(runtime.Enabled ? "停用" : "启用", () => Upsert(runtime with { Enabled = !runtime.Enabled })),
                Ui.MenuItem("删除…", () =>
                {
                    list.SelectedItem = item;
                    Delete();
                }));
            list.Items.Add(item);
        }
        list.SelectedIndex = Array.FindIndex(instances, runtime => runtime.Id == selected) is var index and >= 0
            ? index
            : Math.Min(list.SelectedIndex, instances.Length - 1);
        Ui.Placeholder(list, "还没有 Agent 运行时。用＋手动添加，或快速导入已安装的。");
        ShowActions();
    }

    string Summary(BindingRuntimeInstance runtime)
    {
        var parts = new List<string> { Kind(runtime.TypeId)?.Name ?? runtime.TypeId };
        if (!runtime.Enabled)
        {
            parts.Add("已停用");
        }
        return string.Join(" · ", parts);
    }

    // Offer tells, after runtimes are added, that the library's resources
    // can be enabled on them.
    async void Offer(List<BindingRuntimeInstance> added)
    {
        try
        {
            var count = (await Task.Run(core.Local.ListResourceTemplates)).Length;
            if (count > 0)
            {
                offer.Message = $"资源库中有 {count} 份资源，可以启用于“{string.Join("”“", added.Select(runtime => runtime.Name))}”。";
                offer.IsOpen = true;
            }
        }
        catch (Exception problem)
        {
            Problems.Report("读取资源库", problem);
        }
    }

    BindingRuntimeTypeDescriptor? Kind(string id) => kinds.FirstOrDefault(kind => kind.Id == id);

    // Addable are the types an instance can be added of.
    IEnumerable<BindingRuntimeTypeDescriptor> Addable() => kinds;

    // Write runs one change against this PC's core, then shows the runtimes
    // it reports; false keeps a dialog open with what failed.
    async Task<bool> Write(string source, Func<VeluneApplication, BindingRuntimeUpdate> work, TextBlock? shownIn = null)
    {
        if (busy)
        {
            return false;
        }
        busy = true;
        ShowActions();
        try
        {
            var update = await Task.Run(() => work(core.Local));
            kinds = update.RuntimeTypes;
            SetInstances(update.RuntimeInstances);
            return true;
        }
        catch (Exception problem)
        {
            // A dialog covers the title bar's way to 问题, so it says why too.
            if (shownIn != null)
            {
                Ui.Show(shownIn, Core.Describe(problem));
            }
            Problems.Report(source, problem);
            return false;
        }
        finally
        {
            busy = false;
            ShowActions();
        }
    }

    async void Upsert(BindingRuntimeInstance runtime) => await Write("保存运行时「" + runtime.Name + "」", application => application.UpsertRuntime(runtime));

    async void Delete()
    {
        if (Chosen() is not { } runtime || busy)
        {
            return;
        }
        if (await Ui.Confirm(window, "删除运行时实例？", "只删除此配置实例；Agent 运行时自己的会话、资源和配置文件不会被删除。", "删除"))
        {
            await Write("删除运行时「" + runtime.Name + "」", application => application.DeleteRuntime(runtime.Id));
        }
    }

    // Edit adds a runtime instance by hand (existing is null), starting from
    // template's type and settings when given, or edits one: its name,
    // whether it runs, its type and the type's settings, its HOME, an ACP
    // Agent's launch, and what its new conversations start with.
    async void Edit(BindingRuntimeInstance? existing, BindingRuntimeInstance? template = null)
    {
        var addable = Addable().ToList();
        var kind = existing != null ? Kind(existing.TypeId)
            : addable.FirstOrDefault(candidate => candidate.Id == template?.TypeId) ?? addable.FirstOrDefault();
        if (kind == null)
        {
            Ui.Show(failure, existing == null ? "" : "这个运行时的类型已不受支持");
            return;
        }
        if (await ReadExtras(existing, kind) is not { } extras)
        {
            return;
        }
        var home = new HomeSection(window, extras.Execution, extras.ManagedHome);
        var launch = new LaunchSection(extras.Launch);
        var starts = new StartsSection(core, existing, kind, extras.Choices);
        var panel = new StackPanel { Spacing = 16, MinWidth = 440 };
        var name = new TextBox { Header = "实例名称", Text = existing?.Name ?? (template?.TypeId == kind.Id ? template.Name : kind.Name) };
        var enabled = new ToggleSwitch { Header = "启用此运行时", IsOn = existing?.Enabled ?? true };
        var fields = new StackPanel { Spacing = 12 };
        var formError = Ui.Error();
        var values = new Dictionary<string, Func<string>>();
        var errors = new Dictionary<string, TextBlock>();
        var views = new Dictionary<string, FrameworkElement>();
        void ShowDirectory()
        {
            if (views.TryGetValue(RuntimeDirectoryKey, out var directory))
            {
                directory.Visibility = home.Managed ? Visibility.Collapsed : Visibility.Visible;
            }
        }
        void ShowFields(BindingRuntimeTypeDescriptor shown)
        {
            kind = shown;
            fields.Children.Clear();
            values.Clear();
            errors.Clear();
            views.Clear();
            launch.Element.Visibility = shown.LaunchOptions ? Visibility.Visible : Visibility.Collapsed;
            if (shown.Fields.Length > 0)
            {
                fields.Children.Add(Ui.Text("实例配置", "BodyStrongTextBlockStyle"));
            }
            // A template fills only its own type's fields.
            var source = existing ?? (template?.TypeId == shown.Id ? template : null);
            foreach (var field in shown.Fields)
            {
                var value = source?.Settings.GetValueOrDefault(field.Key) ?? field.Value;
                errors[field.Key] = Ui.Error();
                var view = Ui.Field(FieldControl(field, value, values), field.Help ?? "", errors[field.Key]);
                views[field.Key] = view;
                fields.Children.Add(view);
            }
            ShowDirectory();
        }
        home.Changed += ShowDirectory;
        panel.Children.Add(name);
        panel.Children.Add(Ui.Field(enabled, "停用后不读取其历史，也不创建或执行新任务；原生会话数据保留。"));
        if (existing == null)
        {
            var type = new ComboBox { Header = "类型", HorizontalAlignment = HorizontalAlignment.Stretch };
            foreach (var candidate in addable)
            {
                type.Items.Add(candidate.Name);
            }
            type.SelectedIndex = addable.IndexOf(kind);
            type.SelectionChanged += (_, _) =>
            {
                ShowFields(addable[type.SelectedIndex]);
                name.Text = addable[type.SelectedIndex].Name;
                // Permission modes are the type's.
                var at = panel.Children.IndexOf(starts.Element);
                starts = new StartsSection(core, null, kind, null);
                if (at >= 0)
                {
                    panel.Children[at] = starts.Element;
                }
            };
            panel.Children.Add(type);
        }
        else
        {
            panel.Children.Add(Ui.Labeled("类型", new TextBlock { Text = kind.Name }));
        }
        ShowFields(kind);
        panel.Children.Add(fields);
        panel.Children.Add(home.Element);
        panel.Children.Add(launch.Element);
        if (existing != null && kind.AgentInfo)
        {
            panel.Children.Add(AgentInfo(existing));
        }
        panel.Children.Add(starts.Element);
        panel.Children.Add(formError);

        var dialog = new ContentDialog
        {
            Title = existing == null ? "添加 Agent 运行时" : "编辑 Agent 运行时",
            Content = new ScrollViewer { Content = panel, Padding = new Thickness(0, 0, 16, 0), MaxHeight = 480 },
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 560.0;
        dialog.PrimaryButtonClick += async (_, e) =>
        {
            var deferral = e.GetDeferral();
            // A stopped runtime needs none of its required fields, nor one
            // with a managed HOME its runtime directory.
            var missing = enabled.IsOn
                ? kind.Fields.FirstOrDefault(field => field.Required && values[field.Key]().Trim().Length == 0 && !(field.Key == RuntimeDirectoryKey && home.Managed))
                : null;
            foreach (var field in kind.Fields)
            {
                Ui.Show(errors[field.Key], field == missing ? "请填写“" + field.Label + "”" : "");
            }
            var execution = home.Read();
            var launched = kind.LaunchOptions ? launch.Read() : null;
            if (missing != null || execution == null || (kind.LaunchOptions && launched == null))
            {
                e.Cancel = true;
                deferral.Complete();
                return;
            }
            var started = starts.Read();
            var lastChosen = extras.Choices?.LastChosen ?? new BindingTurnChoice(null, null, null);
            // An edited instance's other settings stay as they are.
            var settings = new Dictionary<string, string>(existing?.Settings ?? []);
            foreach (var field in kind.Fields)
            {
                var value = values[field.Key]().Trim();
                if (value.Length > 0)
                {
                    settings[field.Key] = value;
                }
                else
                {
                    settings.Remove(field.Key);
                }
            }
            var shownName = name.Text.Trim();
            var runtime = existing == null
                ? new BindingRuntimeInstance(enabled.IsOn, Guid.NewGuid().ToString(), shownName.Length > 0 ? shownName : kind.Name, kind.Id, DefaultGateway, settings)
                : existing with { Enabled = enabled.IsOn, Name = shownName.Length > 0 ? shownName : kind.Name, Settings = settings };
            e.Cancel = !await Write("保存运行时「" + runtime.Name + "」", application =>
            {
                var update = launched != null
                    ? application.UpsertAcpRuntime(runtime, execution, launched)
                    : application.UpsertRuntimeWithExecution(runtime, execution);
                if (started != null)
                {
                    application.SetRuntimeComposerChoices(new BindingRuntimeComposerChoices(runtime.Id, lastChosen, started));
                }
                return update;
            }, formError);
            deferral.Complete();
        };
        await window.ShowDialog(dialog);
    }

    // FieldControl is a setting's control; values reads what it holds.
    FrameworkElement FieldControl(BindingSettingField field, string value, Dictionary<string, Func<string>> values)
    {
        if (field.Kind == BindingSettingKind.Choice)
        {
            var choice = new ComboBox { Header = field.Label, HorizontalAlignment = HorizontalAlignment.Stretch, PlaceholderText = "选择" };
            foreach (var option in field.Options)
            {
                choice.Items.Add(option.Label);
            }
            choice.SelectedIndex = Array.FindIndex(field.Options, option => option.Id == value);
            values[field.Key] = () => choice.SelectedIndex >= 0 ? field.Options[choice.SelectedIndex].Id : "";
            return choice;
        }
        var input = new TextBox { Header = field.Label, Text = value };
        values[field.Key] = () => input.Text;
        if (field.Kind == BindingSettingKind.Text)
        {
            return input;
        }
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var browse = new Button { Content = "浏览…", VerticalAlignment = VerticalAlignment.Bottom };
        browse.Click += async (_, _) =>
        {
            var path = field.Kind == BindingSettingKind.DirectoryPath ? await Native.PickFolder(window) : await Native.PickFile(window);
            if (path != null)
            {
                input.Text = path;
            }
        };
        Grid.SetColumn(browse, 1);
        row.Children.Add(input);
        row.Children.Add(browse);
        return row;
    }

    // Import lists the runtimes discovery finds, to add them one by one.
    async void Import()
    {
        var status = Ui.Text("正在发现运行时…");
        var found = new StackPanel { Spacing = 8 };
        var panel = new StackPanel { Spacing = 12, MinWidth = 480 };
        panel.Children.Add(Ui.Muted("在 PATH、npm／pnpm 全局目录，以及 Codex、Claude 桌面应用与编辑器扩展的安装位置中查找 Pi、Codex 与 Claude Code，只读取公开版本，不导入登录信息。"));
        panel.Children.Add(status);
        panel.Children.Add(found);
        var dialog = new ContentDialog
        {
            Title = "快速导入 Agent 运行时",
            Content = new ScrollViewer { Content = panel, Padding = new Thickness(0, 0, 16, 0), MaxHeight = 480 },
            PrimaryButtonText = "完成",
            SecondaryButtonText = "重新发现",
            DefaultButton = ContentDialogButton.Primary,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 640.0;
        async void Discover()
        {
            status.Text = "正在发现运行时…";
            found.Children.Clear();
            dialog.IsSecondaryButtonEnabled = false;
            try
            {
                var candidates = await Task.Run(() => Discovery.Discover(core.Local));
                status.Text = candidates.Length == 0 ? "未发现运行时，可以手动添加。" : $"发现 {candidates.Length} 个运行时。";
                foreach (var candidate in candidates)
                {
                    found.Children.Add(CandidateRow(candidate, dialog));
                }
            }
            catch (Exception problem)
            {
                status.Text = Core.Describe(problem);
            }
            dialog.IsSecondaryButtonEnabled = true;
        }
        dialog.SecondaryButtonClick += (_, e) =>
        {
            e.Cancel = true;
            Discover();
        };
        dialog.Opened += (_, _) => Discover();
        await window.ShowDialog(dialog);
    }

    Border CandidateRow(BindingRuntimeDiscoveryCandidate candidate, ContentDialog importing)
    {
        var lines = Ui.TwoLines(candidate.Runtime.Name + (candidate.Version is { } version ? " " + version : ""), candidate.Runtime.Settings.GetValueOrDefault("binary", ""));
        if (!candidate.Supported)
        {
            lines.Children.Add(Ui.Muted(candidate.Detail));
        }
        FrameworkElement action;
        if (candidate.AlreadyConfigured)
        {
            action = Ui.Muted("已配置");
        }
        else if (!candidate.Supported && Addable().Any(kind => kind.Id == candidate.Runtime.TypeId))
        {
            // A known version that discovery could not complete, such as a
            // runtime directory that does not exist yet: finish it by hand.
            var manual = new Button { Content = "手动添加…" };
            manual.Click += (_, _) =>
            {
                // One dialog at a time: the form opens once this one closes.
                importing.Hide();
                Edit(null, candidate.Runtime);
            };
            action = manual;
        }
        else
        {
            var addButton = new Button { Content = "添加", IsEnabled = candidate.Supported };
            addButton.Click += async (_, _) =>
            {
                addButton.IsEnabled = false;
                if (await Write("导入运行时「" + candidate.Runtime.Name + "」", application => application.UpsertRuntime(candidate.Runtime)))
                {
                    addButton.Content = "已添加";
                }
                else
                {
                    addButton.IsEnabled = true;
                }
            };
            action = addButton;
        }
        return Ui.Card(lines, action);
    }
}
