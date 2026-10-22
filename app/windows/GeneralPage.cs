using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Velune.Bindings;
using Windows.System;
using Windows.UI.Core;

namespace Velune;

// This PC's general settings: how usage reads, how much of the conversation
// list loads, how a conversation shows and opens linked ones, and the
// shortcuts, which are this app's own. Each change saves at once.
sealed class GeneralPage : UserControl
{
    static readonly (BindingStatusLineItem Item, string Name)[] statusItems =
    [
        (BindingStatusLineItem.ContextUsage, "上下文用量"),
        (BindingStatusLineItem.SessionTokens, "本会话累计 token"),
        (BindingStatusLineItem.WorkingDirectory, "工作目录与分支"),
        (BindingStatusLineItem.RateLimits, "账户额度"),
        (BindingStatusLineItem.SessionCost, "费用估算"),
    ];

    readonly Core core;
    readonly ComboBox usage = new() { Items = { "已用", "剩余" }, MinWidth = 120 };
    readonly NumberBox page = new() { Minimum = 1, Maximum = 500, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact, MinWidth = 120 };
    readonly NumberBox total = new() { Minimum = 1, Maximum = 5000, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact, MinWidth = 120 };
    readonly ComboBox outline = new() { Items = { "弹出面板", "就地折叠" }, MinWidth = 120 };
    static readonly (BindingLinkedConversationOpening Opening, string Name)[] openings =
    [
        (BindingLinkedConversationOpening.CurrentTab, "当前标签页"),
        (BindingLinkedConversationOpening.NewTab, "新标签页"),
        (BindingLinkedConversationOpening.Sheet, "浮层"),
        (BindingLinkedConversationOpening.NewWindow, "新窗口"),
    ];
    readonly ComboBox linked = new() { MinWidth = 120 };
    readonly Commands commands;
    // The command whose shortcut is being recorded, with its row's parts.
    (Command Command, Button Button, TextBlock Error, Action Show)? recording;
    readonly List<(BindingStatusLineItem Item, CheckBox Box)> status = [];
    // Set while the page shows what the device keeps, which is no change.
    bool showing;

    internal GeneralPage(Core core, Commands commands)
    {
        this.core = core;
        this.commands = commands;
        var panel = new StackPanel { Spacing = 4, Padding = new Thickness(24, 12, 24, 20), MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Left };
        panel.Children.Add(Ui.Text("通用", "SubtitleTextBlockStyle"));

        panel.Children.Add(Ui.Heading("用量"));
        panel.Children.Add(Ui.Card(Ui.TwoLines("用量显示", "提供商额度、账户限额与会话上下文显示为已用或剩余。只改变显示，额度上限仍按已用比例生效。", true), usage));

        panel.Children.Add(Ui.Heading("会话列表"));
        panel.Children.Add(Ui.Card(Ui.TwoLines("每次加载", "每个分组一次读取的会话数，组内用“加载更多”继续。", true), page));
        panel.Children.Add(Ui.Card(Ui.TwoLines("总数上限", "整个列表先读取的会话数，其余在展开分组时读取。", true), total));

        panel.Children.Add(Ui.Heading("会话"));
        panel.Children.Add(Ui.Card(Ui.TwoLines("大纲按钮", "弹出面板列出用户消息；就地折叠把消息列表收起为只剩用户消息。", true), outline));
        var boxes = new StackPanel { Spacing = 0 };
        foreach (var (item, name) in statusItems)
        {
            var box = new CheckBox { Content = name };
            box.Click += (_, _) => SaveStatus();
            boxes.Children.Add(box);
            status.Add((item, box));
        }
        panel.Children.Add(Ui.Card(Ui.TwoLines("状态栏", "输入区操作行右侧常驻显示的项；只显示运行时记录中已有的项。点按状态栏可查看全部。", true), boxes));
        foreach (var (_, name) in openings)
        {
            linked.Items.Add(name);
        }
        panel.Children.Add(Ui.Card(Ui.TwoLines("关联会话", "从转录中的协作者、子 Agent 或“打开父会话”打开会话的方式。浮层可再转到标签页或新窗口。", true), linked));

        panel.Children.Add(Ui.Heading("快捷键"));
        panel.Children.Add(Ui.Muted("点按快捷键后按下新的组合即可更改，Backspace 移除，Esc 取消。发送（Ctrl+Enter）与插入本轮（Ctrl+Shift+Enter）属于输入框，不可更改。"));
        panel.Children.Add(Shortcuts());

        Content = new ScrollViewer { Content = panel };
        usage.SelectionChanged += (_, _) => Save(application => application.SetUsageDisplay(usage.SelectedIndex == 1 ? BindingUsageDisplay.Remaining : BindingUsageDisplay.Used));
        outline.SelectionChanged += (_, _) => Save(application => application.SetOutlinePresentation(outline.SelectedIndex == 1 ? BindingOutlinePresentation.InPlace : BindingOutlinePresentation.Popover));
        linked.SelectionChanged += (_, _) =>
        {
            if (linked.SelectedIndex >= 0)
            {
                var opening = openings[linked.SelectedIndex].Opening;
                Save(application => application.SetLinkedConversationOpening(opening));
            }
        };
        page.ValueChanged += (_, _) => SaveLimits();
        total.ValueChanged += (_, _) => SaveLimits();
        Load();
    }

    async void Load()
    {
        try
        {
            var snapshot = await Task.Run(() => core.Local.List());
            showing = true;
            usage.SelectedIndex = snapshot.UsageDisplay == BindingUsageDisplay.Remaining ? 1 : 0;
            outline.SelectedIndex = snapshot.OutlinePresentation == BindingOutlinePresentation.InPlace ? 1 : 0;
            linked.SelectedIndex = Array.FindIndex(openings, entry => entry.Opening == snapshot.LinkedConversationOpening);
            page.Value = snapshot.ConversationBrowserLimits.Page;
            total.Value = snapshot.ConversationBrowserLimits.Total;
            foreach (var (item, box) in status)
            {
                box.IsChecked = snapshot.StatusLine.Contains(item);
            }
            showing = false;
        }
        catch (Exception problem)
        {
            showing = false;
            Problems.Report("读取通用设置", problem);
        }
    }

    // Shortcuts lists the commands with their shortcuts, each changed by
    // pressing the new one.
    Border Shortcuts()
    {
        var rows = new StackPanel { Spacing = 6 };
        var shows = new List<Action>();
        foreach (var (command, (name, _)) in Commands.Table)
        {
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(96) });
            var label = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
            label.Children.Add(new TextBlock { Text = name });
            var error = Ui.Error();
            label.Children.Add(error);
            row.Children.Add(label);
            var button = new Button { MinWidth = 160, HorizontalContentAlignment = HorizontalAlignment.Center };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, name + "的快捷键");
            Grid.SetColumn(button, 1);
            row.Children.Add(button);
            var reset = new HyperlinkButton { Content = "恢复默认" };
            Grid.SetColumn(reset, 2);
            row.Children.Add(reset);
            void Show()
            {
                button.Content = commands.Shortcut(command) is { Length: > 0 } keys ? keys : "无";
                reset.Visibility = commands.IsChanged(command) ? Visibility.Visible : Visibility.Collapsed;
            }
            button.Click += (_, _) =>
            {
                var again = recording?.Command == command;
                StopRecording();
                if (!again)
                {
                    recording = (command, button, error, Show);
                    button.Content = "按下新的快捷键…";
                }
            };
            button.PreviewKeyDown += (_, e) => Record(command, e);
            button.LostFocus += (_, _) =>
            {
                if (recording?.Command == command)
                {
                    StopRecording();
                }
            };
            reset.Click += (_, _) =>
            {
                commands.Reset(command);
                Ui.Show(error, "");
                Show();
            };
            Show();
            shows.Add(Show);
            rows.Children.Add(row);
        }
        var all = new Button { Content = "全部恢复默认", Margin = new Thickness(0, 8, 0, 0) };
        all.Click += (_, _) =>
        {
            StopRecording();
            commands.ResetAll();
            foreach (var show in shows)
            {
                show();
            }
        };
        rows.Children.Add(all);
        return new Border { Style = Ui.Style("CardStyle"), CornerRadius = new CornerRadius(4), Padding = new Thickness(16, 12, 16, 12), Child = rows };
    }

    void StopRecording()
    {
        if (recording is var (_, _, _, show))
        {
            recording = null;
            show();
        }
    }

    // Record takes the keys pressed while a shortcut is recorded.
    void Record(Command command, KeyRoutedEventArgs e)
    {
        if (recording is not var (recorded, _, error, show) || recorded != command)
        {
            return;
        }
        e.Handled = true;
        if (e.Key is VirtualKey.Control or VirtualKey.Shift or VirtualKey.Menu or VirtualKey.LeftControl or VirtualKey.RightControl
            or VirtualKey.LeftShift or VirtualKey.RightShift or VirtualKey.LeftMenu or VirtualKey.RightMenu or VirtualKey.LeftWindows or VirtualKey.RightWindows)
        {
            return;
        }
        bool Down(VirtualKey key) => InputKeyboardSource.GetKeyStateForCurrentThread(key).HasFlag(CoreVirtualKeyStates.Down);
        var modifiers = VirtualKeyModifiers.None;
        if (Down(VirtualKey.Control))
        {
            modifiers |= VirtualKeyModifiers.Control;
        }
        if (Down(VirtualKey.Menu))
        {
            modifiers |= VirtualKeyModifiers.Menu;
        }
        if (Down(VirtualKey.Shift))
        {
            modifiers |= VirtualKeyModifiers.Shift;
        }
        if (modifiers == VirtualKeyModifiers.None && e.Key == VirtualKey.Escape)
        {
            StopRecording();
            return;
        }
        var problem = modifiers == VirtualKeyModifiers.None && e.Key == VirtualKey.Back
            ? commands.Bind(command, null)
            : commands.Bind(command, new Shortcut(modifiers, e.Key));
        Ui.Show(error, problem ?? "");
        if (problem == null)
        {
            recording = null;
            show();
        }
    }

    // SaveStatus keeps the chosen items in the status line's own order.
    void SaveStatus()
    {
        var items = status.Where(entry => entry.Box.IsChecked == true).Select(entry => entry.Item).ToArray();
        Save(application => application.SetStatusLine(items));
    }

    void SaveLimits()
    {
        if (double.IsNaN(page.Value) || double.IsNaN(total.Value))
        {
            return;
        }
        var limits = new BindingConversationBrowserLimits((uint)page.Value, (uint)total.Value);
        Save(application => application.SetConversationBrowserLimits(limits));
    }

    async void Save<T>(Func<VeluneApplication, T> change)
    {
        if (showing)
        {
            return;
        }
        try
        {
            await Task.Run(() => change(core.Local));
        }
        catch (Exception problem)
        {
            Problems.Report("保存通用设置", problem);
            Load();
        }
    }
}
