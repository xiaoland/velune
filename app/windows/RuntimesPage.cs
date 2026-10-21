using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// This PC's Agent runtime instances: a list to add, import, edit and delete them.
sealed class RuntimesPage : UserControl
{
    const string DefaultGateway = "default";

    readonly Core core;
    readonly SettingsWindow window;
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    readonly InfoBar failure = Ui.FailureBar();
    readonly Button add = Ui.IconButton("\uE710", "添加 Agent 运行时实例");
    readonly Button remove = Ui.IconButton("\uE738", "删除运行时实例");
    readonly Button import = new() { Content = "快速导入…" };
    readonly Button edit = new() { Content = "编辑…" };
    BindingRuntimeInstance[] instances = [];
    BindingRuntimeTypeDescriptor[] kinds = [];
    bool busy;

    internal RuntimesPage(Core core, SettingsWindow window)
    {
        this.core = core;
        this.window = window;
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
        Content = Ui.ListPage("Agent 运行时", failure, list, [add, import, remove], [edit]);
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
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    void SetInstances(BindingRuntimeInstance[] shown)
    {
        var selected = Chosen()?.Id;
        instances = shown;
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

    BindingRuntimeTypeDescriptor? Kind(string id) => kinds.FirstOrDefault(kind => kind.Id == id);

    // Addable are the types set up here; an ACP Agent's launch arguments and
    // environment are not edited here.
    IEnumerable<BindingRuntimeTypeDescriptor> Addable() => kinds.Where(kind => !kind.LaunchOptions);

    // Write runs one change against this PC's core, then shows the runtimes
    // it reports; false keeps a dialog open with what failed.
    async Task<bool> Write(Func<VeluneApplication, BindingRuntimeUpdate> work, TextBlock? shownIn = null)
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
            Ui.Show(failure, "");
            return true;
        }
        catch (Exception problem)
        {
            if (shownIn != null)
            {
                Ui.Show(shownIn, Core.Describe(problem));
            }
            else
            {
                Ui.Show(failure, Core.Describe(problem));
            }
            return false;
        }
        finally
        {
            busy = false;
            ShowActions();
        }
    }

    async void Upsert(BindingRuntimeInstance runtime) => await Write(application => application.UpsertRuntime(runtime));

    async void Delete()
    {
        if (Chosen() is not { } runtime || busy)
        {
            return;
        }
        if (await Ui.Confirm(window, "删除运行时实例？", "只删除此配置实例；Agent 运行时自己的会话、资源和配置文件不会被删除。", "删除"))
        {
            await Write(application => application.DeleteRuntime(runtime.Id));
        }
    }

    // Edit adds a runtime instance by hand (existing is null) or edits one:
    // its name, whether it runs, its type and the type's settings.
    async void Edit(BindingRuntimeInstance? existing)
    {
        var addable = Addable().ToList();
        var kind = existing == null ? addable.FirstOrDefault() : Kind(existing.TypeId);
        if (kind == null)
        {
            Ui.Show(failure, existing == null ? "" : "这个运行时的类型已不受支持");
            return;
        }
        var panel = new StackPanel { Spacing = 16, MinWidth = 440 };
        var name = new TextBox { Header = "实例名称", Text = existing?.Name ?? kind.Name };
        var enabled = new ToggleSwitch { Header = "启用此运行时", IsOn = existing?.Enabled ?? true };
        var fields = new StackPanel { Spacing = 12 };
        var formError = Ui.Error();
        var values = new Dictionary<string, Func<string>>();
        var errors = new Dictionary<string, TextBlock>();
        void ShowFields(BindingRuntimeTypeDescriptor shown)
        {
            kind = shown;
            fields.Children.Clear();
            values.Clear();
            errors.Clear();
            if (shown.Fields.Length > 0)
            {
                fields.Children.Add(Ui.Text("实例配置", "BodyStrongTextBlockStyle"));
            }
            foreach (var field in shown.Fields)
            {
                var value = existing?.Settings.GetValueOrDefault(field.Key) ?? field.Value;
                errors[field.Key] = Ui.Error();
                fields.Children.Add(Ui.Field(FieldControl(field, value, values), field.Help ?? "", errors[field.Key]));
            }
        }
        panel.Children.Add(name);
        panel.Children.Add(Ui.Field(enabled, "停用后不读取其历史，也不创建或执行新任务；原生会话数据保留。"));
        if (existing == null)
        {
            var type = new ComboBox { Header = "类型", HorizontalAlignment = HorizontalAlignment.Stretch };
            foreach (var candidate in addable)
            {
                type.Items.Add(candidate.Name);
            }
            type.SelectedIndex = 0;
            type.SelectionChanged += (_, _) =>
            {
                ShowFields(addable[type.SelectedIndex]);
                name.Text = addable[type.SelectedIndex].Name;
            };
            panel.Children.Add(type);
        }
        else
        {
            panel.Children.Add(Ui.Labeled("类型", new TextBlock { Text = kind.Name }));
        }
        ShowFields(kind);
        panel.Children.Add(fields);
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
            // A stopped runtime needs none of its required fields.
            var missing = enabled.IsOn
                ? kind.Fields.FirstOrDefault(field => field.Required && values[field.Key]().Trim().Length == 0)
                : null;
            foreach (var field in kind.Fields)
            {
                Ui.Show(errors[field.Key], field == missing ? "请填写“" + field.Label + "”" : "");
            }
            if (missing != null)
            {
                e.Cancel = true;
                deferral.Complete();
                return;
            }
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
            e.Cancel = !await Write(application => application.UpsertRuntime(runtime), formError);
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
                    found.Children.Add(CandidateRow(candidate));
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

    Border CandidateRow(BindingRuntimeDiscoveryCandidate candidate)
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
        else
        {
            var addButton = new Button { Content = "添加", IsEnabled = candidate.Supported };
            addButton.Click += async (_, _) =>
            {
                addButton.IsEnabled = false;
                if (await Write(application => application.UpsertRuntime(candidate.Runtime)))
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
