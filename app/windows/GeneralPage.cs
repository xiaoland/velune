using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// This PC's general settings: how usage reads, how much of the conversation
// list loads, and how a conversation shows. Each change saves at once.
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
    readonly InfoBar failure = Ui.FailureBar();
    readonly ComboBox usage = new() { Items = { "已用", "剩余" }, MinWidth = 120 };
    readonly NumberBox page = new() { Minimum = 1, Maximum = 500, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact, MinWidth = 120 };
    readonly NumberBox total = new() { Minimum = 1, Maximum = 5000, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact, MinWidth = 120 };
    readonly ComboBox outline = new() { Items = { "弹出面板", "就地折叠" }, MinWidth = 120 };
    readonly List<(BindingStatusLineItem Item, CheckBox Box)> status = [];
    // Set while the page shows what the device keeps, which is no change.
    bool showing;

    internal GeneralPage(Core core)
    {
        this.core = core;
        var panel = new StackPanel { Spacing = 4, Padding = new Thickness(24, 12, 24, 20), MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Left };
        panel.Children.Add(Ui.Text("通用", "SubtitleTextBlockStyle"));
        panel.Children.Add(failure);

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

        Content = new ScrollViewer { Content = panel };
        usage.SelectionChanged += (_, _) => Save(application => application.SetUsageDisplay(usage.SelectedIndex == 1 ? BindingUsageDisplay.Remaining : BindingUsageDisplay.Used));
        outline.SelectionChanged += (_, _) => Save(application => application.SetOutlinePresentation(outline.SelectedIndex == 1 ? BindingOutlinePresentation.InPlace : BindingOutlinePresentation.Popover));
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
            Ui.Show(failure, Core.Describe(problem));
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
            Ui.Show(failure, "");
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
            Load();
        }
    }
}
