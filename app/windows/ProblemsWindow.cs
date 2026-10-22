using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;

namespace Velune;

// The 问题 window: this run's problems, newest first, each with all its
// detail to read and copy. There is one; opening it again brings it back.
sealed class ProblemsWindow : Window
{
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single };
    readonly TextBlock detail = new() { IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap };
    readonly TextBlock state = new() { Style = Ui.Style("SecondaryTextStyle"), Margin = new Thickness(0, 0, 0, 8) };
    readonly TextBlock empty = new() { Text = "没有问题。", Style = Ui.Style("SecondaryTextStyle"), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
    readonly Button copy = new() { Content = "复制" };
    readonly Button copyAll = new() { Content = "全部复制" };
    readonly Button remove = new() { Content = "移除" };
    readonly Button clear = new() { Content = "全部清除" };
    readonly Grid body = new() { ColumnSpacing = 0 };

    public ProblemsWindow()
    {
        Title = "问题";
        ExtendsContentIntoTitleBar = true;
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Velune.ico"));
        var root = new Grid();
        if (MicaController.IsSupported())
        {
            SystemBackdrop = new MicaBackdrop();
        }
        else
        {
            root.Style = Ui.Style("WindowBackgroundStyle");
        }
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        var titleBar = new TitleBar { Title = "问题", Height = 48 };
        root.Children.Add(titleBar);
        SetTitleBar(titleBar);

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Margin = new Thickness(16, 0, 16, 12) };
        actions.Children.Add(copy);
        actions.Children.Add(remove);
        actions.Children.Add(copyAll);
        actions.Children.Add(clear);
        Grid.SetRow(actions, 1);
        root.Children.Add(actions);

        body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(300) });
        body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        body.Children.Add(list);
        var pane = new StackPanel { Padding = new Thickness(20, 4, 20, 20) };
        pane.Children.Add(state);
        pane.Children.Add(detail);
        var reading = new ScrollViewer { Content = pane };
        Grid.SetColumn(reading, 1);
        body.Children.Add(reading);
        var content = new Grid { BorderThickness = new Thickness(0, 1, 0, 0), BorderBrush = (Brush)Application.Current.Resources["DividerStrokeColorDefaultBrush"] };
        content.Children.Add(body);
        content.Children.Add(empty);
        Grid.SetRow(content, 2);
        root.Children.Add(content);
        Content = root;
        WorkspaceWindow.Size(this, 820, 520, 520, 320);

        list.SelectionChanged += (_, _) => ShowSelected();
        copy.Click += (_, _) =>
        {
            if (Selected is { } problem)
            {
                Copy(problem.Text);
            }
        };
        copyAll.Click += (_, _) => Copy(string.Join("\n\n", Problems.Entries.Select(problem => problem.Text)));
        remove.Click += (_, _) =>
        {
            if (Selected is { } problem)
            {
                Problems.Remove(problem);
            }
        };
        clear.Click += (_, _) => Problems.Clear();
        Problems.Changed += Render;
        Closed += (_, _) => Problems.Changed -= Render;
        Render();
    }

    Problem? Selected => (list.SelectedItem as ListViewItem)?.Tag as Problem;

    void Render()
    {
        var selected = Selected;
        list.Items.Clear();
        foreach (var problem in Problems.Entries)
        {
            var when = problem.Time.ToLocalTime().ToString(problem.Time.Date == DateTimeOffset.Now.Date ? "HH:mm:ss" : "MM-dd HH:mm");
            var row = new ListViewItem { Tag = problem, Content = Ui.TwoLines(problem.Source, $"{problem.Device} · {when}{(problem.Ongoing ? " · 仍在发生" : "")}") };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, problem.Source);
            list.Items.Add(row);
        }
        var any = list.Items.Count > 0;
        body.Visibility = any ? Visibility.Visible : Visibility.Collapsed;
        empty.Visibility = any ? Visibility.Collapsed : Visibility.Visible;
        copyAll.IsEnabled = any;
        clear.IsEnabled = any;
        var keep = list.Items.OfType<ListViewItem>().FirstOrDefault(row => selected != null && (row.Tag as Problem)?.Key is { } key ? key == selected.Key : ReferenceEquals(row.Tag, selected));
        list.SelectedItem = keep ?? list.Items.FirstOrDefault();
        ShowSelected();
    }

    void ShowSelected()
    {
        var problem = Selected;
        copy.IsEnabled = problem != null;
        remove.IsEnabled = problem != null;
        detail.Text = problem?.Text ?? "";
        state.Text = problem is { Ongoing: true } ? "这项读取仍然失败；成功后自动移除。" : "";
        state.Visibility = state.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    static void Copy(string text)
    {
        var package = new DataPackage();
        package.SetText(text);
        Clipboard.SetContent(package);
    }
}

// ProblemsButton is the title bar's way to the 问题 window, with how many
// there are; it shows only while there are some.
sealed class ProblemsButton : Button
{
    readonly TextBlock count = new() { VerticalAlignment = VerticalAlignment.Center };

    public ProblemsButton(Action open)
    {
        Content = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6,
            Children = { new FontIcon { Glyph = "\uE7BA", FontSize = 14 }, count },
        };
        Click += (_, _) => open();
        Loaded += (_, _) =>
        {
            Problems.Changed += Show;
            Show();
        };
        Unloaded += (_, _) => Problems.Changed -= Show;
        Show();
    }

    void Show()
    {
        var number = Problems.Entries.Count;
        count.Text = number.ToString();
        Visibility = number > 0 ? Visibility.Visible : Visibility.Collapsed;
        var label = $"问题（{number}）";
        ToolTipService.SetToolTip(this, label);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(this, label);
    }
}
