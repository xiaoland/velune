using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace Velune;

// QuotaHistory shows a provider's retained quota over time: a window's used
// share or an amount, in 96 bins that each read as their latest observation.
// Only observations Velune made are shown; a bin without one stays empty
// rather than carrying a value forward.
sealed class QuotaHistory
{
    const int Bins = 96;
    const double ChartHeight = 180;
    static readonly (string Label, TimeSpan Span)[] Ranges = [("24 小时", TimeSpan.FromDays(1)), ("7 天", TimeSpan.FromDays(7)), ("30 天", TimeSpan.FromDays(30))];

    // A series: a window's used share, or an amount in its unit.
    sealed record Metric(bool Window, string Id, string[]? Models, string Unit)
    {
        public string Key => (Window ? "w" : "b") + "\n" + Id + "\n" + string.Join(",", Models ?? []) + "\n" + Unit;
        public string Title => Window
            ? Quota.Name(Id) + (Models is { Length: > 0 } scoped ? "（仅 " + string.Join("、", scoped) + "）" : "")
            : Quota.BalanceName(Id) + "（" + Unit + "）";
    }

    sealed class Bin
    {
        public DateTimeOffset Start, End, LatestAt;
        public double Latest, Low, High;
        public int Count;
    }

    readonly Core core;
    readonly ProviderSummary provider;
    readonly QuotaObservation? status;
    readonly bool remaining;
    readonly ComboBox metricBox = new() { MinWidth = 240 };
    readonly ComboBox rangeBox = new() { MinWidth = 120 };
    readonly Grid chart = new() { Height = ChartHeight, ColumnSpacing = 1 };
    readonly TextBlock scale = Ui.Muted(""), from = Ui.Muted(""), state = Ui.Muted("");
    readonly TextBlock summary = Ui.Text("");
    readonly ContentDialog dialog;
    List<Metric> metrics = [];
    QuotaHistoryPoint[] points = [];
    // Set while the metric list refills, whose selection changes mean nothing.
    bool open = true, clearing, filling;

    QuotaHistory(Core core, ProviderSummary provider, QuotaObservation? status, UsageDisplay usage)
    {
        this.core = core;
        this.provider = provider;
        this.status = status;
        remaining = usage == UsageDisplay.Remaining;
        foreach (var (label, _) in Ranges)
        {
            rangeBox.Items.Add(label);
        }
        rangeBox.SelectedIndex = 0;
        rangeBox.SelectionChanged += (_, _) => Load();
        metricBox.SelectionChanged += (_, _) =>
        {
            if (!filling)
            {
                Render();
            }
        };

        var choices = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        choices.Children.Add(metricBox);
        choices.Children.Add(rangeBox);
        var ends = new Grid();
        ends.Children.Add(from);
        ends.Children.Add(new TextBlock { Text = "现在", Style = Ui.Style("MutedTextStyle"), HorizontalAlignment = HorizontalAlignment.Right });
        var panel = new StackPanel { Spacing = 8, Width = 600 };
        panel.Children.Add(choices);
        panel.Children.Add(scale);
        panel.Children.Add(new Border { Style = Ui.Style("DividedStyle"), BorderThickness = new Thickness(0, 0, 0, 1), Child = chart });
        panel.Children.Add(ends);
        panel.Children.Add(summary);
        panel.Children.Add(state);
        panel.Children.Add(Ui.Muted("只记录 Velune 读取到的额度：查看额度或经网关使用这个提供商时才会读取。空白时段表示没有观测，不代表额度没有变化。悬停柱条查看观测时间。保留 30 天；更换认证后从新的记录开始。"));
        dialog = new ContentDialog
        {
            Title = "额度历史 · " + provider.Name,
            Content = panel,
            SecondaryButtonText = "清除历史…",
            IsSecondaryButtonEnabled = false,
            CloseButtonText = "完成",
            DefaultButton = ContentDialogButton.Close,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 720.0;
        dialog.SecondaryButtonClick += (_, e) =>
        {
            // The dialog stays open: the first press asks, the second clears.
            e.Cancel = true;
            if (!clearing)
            {
                clearing = true;
                dialog.SecondaryButtonText = "确认清除";
                state.Text = "再按“确认清除”删除保留的观测记录；当前额度、认证和使用上限不受影响。";
                return;
            }
            Clear();
        };
        dialog.Closed += (_, _) => open = false;
    }

    public static async Task Show(SettingsWindow window, Core core, ProviderSummary provider, QuotaObservation? status, UsageDisplay usage)
    {
        var history = new QuotaHistory(core, provider, status, usage);
        history.Load();
        await window.ShowDialog(history.dialog);
    }

    async void Load()
    {
        var span = Ranges[Math.Max(0, rangeBox.SelectedIndex)].Span;
        state.Text = "正在读取…";
        try
        {
            var since = DateTimeOffset.UtcNow.Subtract(span).ToUnixTimeMilliseconds();
            var read = await Task.Run(() => core.Local.ProviderQuotaHistory(ProvidersPage.DefaultGateway, provider.Id, since));
            if (!open)
            {
                return;
            }
            points = read.Points;
            state.Text = "";
            ShowMetrics();
        }
        catch (Exception problem)
        {
            if (open)
            {
                state.Text = "额度历史无法读取：" + Core.Describe(problem);
            }
        }
    }

    async void Clear()
    {
        dialog.IsSecondaryButtonEnabled = false;
        try
        {
            await Task.Run(() => core.Local.ClearProviderQuotaHistory(ProvidersPage.DefaultGateway, provider.Id));
            if (!open)
            {
                return;
            }
            clearing = false;
            dialog.SecondaryButtonText = "清除历史…";
            Load();
        }
        catch (Exception problem)
        {
            if (open)
            {
                state.Text = "清除失败：" + Core.Describe(problem);
                dialog.IsSecondaryButtonEnabled = true;
            }
        }
    }

    // ShowMetrics lists the current series in the order the provider reports
    // them, then those only history has, and keeps the chosen one.
    void ShowMetrics()
    {
        var chosen = metricBox.SelectedIndex >= 0 && metricBox.SelectedIndex < metrics.Count ? metrics[metricBox.SelectedIndex].Key : null;
        var all = new List<Metric>();
        void Add(Metric metric)
        {
            if (!all.Any(known => known.Key == metric.Key))
            {
                all.Add(metric);
            }
        }
        foreach (var current in status?.Windows ?? [])
        {
            Add(new Metric(true, current.Id, current.ModelIds, ""));
        }
        foreach (var current in status?.Balances ?? [])
        {
            Add(new Metric(false, current.Id, null, current.Unit));
        }
        foreach (var point in points)
        {
            foreach (var kept in point.Windows)
            {
                Add(new Metric(true, kept.Id, kept.ModelIds, ""));
            }
            foreach (var kept in point.Balances)
            {
                Add(new Metric(false, kept.Id, null, kept.Unit));
            }
        }
        metrics = all;
        filling = true;
        metricBox.Items.Clear();
        foreach (var metric in metrics)
        {
            metricBox.Items.Add(metric.Title);
        }
        metricBox.IsEnabled = metrics.Count > 0;
        var index = metrics.FindIndex(metric => metric.Key == chosen);
        metricBox.SelectedIndex = metrics.Count == 0 ? -1 : Math.Max(0, index);
        filling = false;
        dialog.IsSecondaryButtonEnabled = points.Length > 0;
        Render();
    }

    double? Value(QuotaHistoryPoint point, Metric metric)
    {
        if (metric.Window)
        {
            var models = string.Join(",", metric.Models ?? []);
            var window = point.Windows.FirstOrDefault(kept => kept.Id == metric.Id && string.Join(",", kept.ModelIds ?? []) == models);
            return window == null ? null : remaining ? 100 - Math.Clamp(window.UsedPercent, 0, 100) : Math.Clamp(window.UsedPercent, 0, 100);
        }
        return point.Balances.FirstOrDefault(kept => kept.Id == metric.Id && kept.Unit == metric.Unit)?.Amount;
    }

    string Format(Metric metric, double value) => metric.Window ? (remaining ? "剩余 " : "已用 ") + value.ToString("0") + "%" : Quota.Amount(value, metric.Unit);

    // Render draws the chosen series anew; elements are never moved between parents.
    void Render()
    {
        chart.Children.Clear();
        chart.ColumnDefinitions.Clear();
        summary.Text = "";
        var span = Ranges[Math.Max(0, rangeBox.SelectedIndex)].Span;
        var end = DateTimeOffset.Now;
        var start = end - span;
        from.Text = start.ToString("M月d日 HH:mm");
        // A pending clear keeps its prompt; otherwise the state is the chart's.
        void Note(string text)
        {
            if (!clearing)
            {
                state.Text = text;
            }
        }
        Note("");
        if (metricBox.SelectedIndex < 0 || metricBox.SelectedIndex >= metrics.Count)
        {
            scale.Text = "";
            Note(status?.Freshness == QuotaFreshness.Unsupported ? "这个提供商不报告额度。" : "还没有记录到额度。");
            return;
        }
        var metric = metrics[metricBox.SelectedIndex];
        var width = span / Bins;
        var bins = new Bin?[Bins];
        foreach (var point in points)
        {
            var at = DateTimeOffset.FromUnixTimeMilliseconds(point.ObservedAtMs).ToLocalTime();
            if (at < start || Value(point, metric) is not { } value)
            {
                continue;
            }
            var index = Math.Min(Bins - 1, (int)((at - start) / width));
            if (bins[index] is { } bin)
            {
                if (at >= bin.LatestAt)
                {
                    bin.Latest = value;
                    bin.LatestAt = at;
                }
                bin.Low = Math.Min(bin.Low, value);
                bin.High = Math.Max(bin.High, value);
                bin.Count++;
            }
            else
            {
                bins[index] = new Bin { Start = start + width * index, End = start + width * (index + 1), Latest = value, LatestAt = at, Low = value, High = value, Count = 1 };
            }
        }
        var shown = bins.OfType<Bin>().ToList();
        if (shown.Count == 0)
        {
            scale.Text = "";
            Note("所选时间范围内没有观测。");
            return;
        }
        var top = metric.Window ? 100 : Math.Max(1, shown.Max(bin => bin.High) * 1.1);
        scale.Text = metric.Window ? (remaining ? "剩余 %，最高 100" : "已用 %，最高 100") : metric.Unit + "，最高 " + top.ToString("N2");
        var bar = Ui.Style("ChartInputStyle");
        for (var index = 0; index < Bins; index++)
        {
            chart.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            if (bins[index] is not { } bin)
            {
                continue;
            }
            var column = new Grid { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
            column.Children.Add(new Rectangle { Style = bar, VerticalAlignment = VerticalAlignment.Bottom, Height = Math.Max(1, bin.Latest / top * ChartHeight) });
            var tip = Format(metric, bin.Latest) + "\n观测于 " + bin.LatestAt.ToString("M月d日 HH:mm");
            if (bin.Count > 1)
            {
                tip += "\n此区间 " + bin.Count + " 次观测，" + Format(metric, bin.Low) + " 至 " + Format(metric, bin.High);
            }
            ToolTipService.SetToolTip(column, tip);
            Grid.SetColumn(column, index);
            chart.Children.Add(column);
        }
        var last = shown[^1];
        var parts = new List<string> { "最近一次 " + Format(metric, last.Latest) + " · " + last.LatestAt.ToString("M月d日 HH:mm") };
        if (metric.Window && !remaining)
        {
            parts.Add("观测到的最高 " + Format(metric, shown.Max(bin => bin.High)));
        }
        else
        {
            parts.Add("观测到的最低 " + Format(metric, shown.Min(bin => bin.Low)));
        }
        var models = string.Join(",", metric.Models ?? []);
        if (metric.Window && status?.Windows.FirstOrDefault(current => current.Id == metric.Id && string.Join(",", current.ModelIds ?? []) == models)?.ResetAtMs is { } reset
            && reset > DateTimeOffset.UtcNow.ToUnixTimeMilliseconds())
        {
            parts.Add("下次重置 " + Quota.Time(reset));
        }
        summary.Text = string.Join("    ", parts);
    }
}
