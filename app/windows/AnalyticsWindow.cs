using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Velune.Bindings;

namespace Velune;

// The 分析 window: what the upstream requests of this PC's LLM gateway
// reported, today or over the last 7 or 30 days. 概览 sums them up, 提供商与模型
// compares the sources and 请求 lists single requests. A report stays with
// the query it answered: while a refresh runs or after it fails the window
// keeps showing it, and a refresh keeps each page's place.
sealed class AnalyticsWindow : Window
{
    const uint RequestStep = 100;
    const uint RequestMost = 500;
    const string ReadKey = "analytics";
    const string TotalDefinition = "总量单位为 Token，仅合计输入和输出均已报告的请求。输入、输出列分别合计各自已报告部分，覆盖可能不同，不能直接相加比较。";
    const string SpeedDefinition = "有效输出速率 = 已报告输出 Token ÷ 上游派发至业务终态的请求耗时，包含等待；汇总按可计算请求加权，不是单次速率的平均值。";

    enum Page { Overview, Breakdown, Requests }

    // Scope narrows the report to a provider or one of its models.
    sealed record Scope(string Name, string? ProviderId, string? ModelRecordKey);

    readonly Core core;
    readonly SelectorBar pages = new();
    readonly ComboBox period = new() { MinWidth = 140 };
    readonly Button refresh = Ui.IconButton("\uE72C", "刷新");
    readonly TextBlock context = new() { Style = Ui.Style("SecondaryTextStyle"), TextTrimming = TextTrimming.CharacterEllipsis };
    readonly TextBlock updated = new() { Style = Ui.Style("SecondaryTextStyle") };
    readonly InfoBar notice = new() { IsClosable = false, Margin = new Thickness(0, 8, 0, 0) };
    readonly Grid host = new();
    readonly TextBlock waiting = new() { Style = Ui.Style("SecondaryTextStyle"), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };

    readonly ScrollViewer overview = new() { Padding = new Thickness(20, 16, 20, 20) };
    readonly SelectorBar trendMetric = new();

    readonly Grid breakdown = new();
    readonly SelectorBar breakdownMode = new();
    readonly ListView breakdownList = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    readonly Button showRequests = new() { Content = "查看请求…" };

    readonly Grid requests = new();
    readonly TextBlock filter = new() { VerticalAlignment = VerticalAlignment.Center };
    readonly Button clearFilter = new() { Content = "清除筛选" };
    readonly Button showDetail = new() { Content = "显示详情…" };
    readonly ListView requestList = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    readonly TextBlock requestCount = new() { Style = Ui.Style("SecondaryTextStyle"), VerticalAlignment = VerticalAlignment.Center };
    readonly Button more = new() { Content = "加载更多" };
    readonly TextBlock most = new() { Text = "最多展示最近 500 条", Style = Ui.Style("SecondaryTextStyle"), VerticalAlignment = VerticalAlignment.Center };

    Scope scope = new("全部提供商", null, null);
    uint requestLimit = RequestStep;
    // The report on show, the query and scope it answered, and when.
    BindingAnalyticsReport? report;
    BindingAnalyticsQuery? loaded;
    Scope loadedScope = new("全部提供商", null, null);
    DateTimeOffset loadedAt;
    bool loading;
    bool failed;
    string? reportedWarning;
    int asked;

    public AnalyticsWindow(Core core, Action openProblems)
    {
        this.core = core;
        var root = Ui.Chrome(this, "分析");

        foreach (var (page, name) in new[] { (Page.Overview, "概览"), (Page.Breakdown, "提供商与模型"), (Page.Requests, "请求") })
        {
            pages.Items.Add(new SelectorBarItem { Text = name, Tag = page });
        }
        pages.SelectedItem = pages.Items[0];
        pages.SelectionChanged += (_, _) => ShowPage();
        foreach (var (days, name) in new[] { (1, "今天"), (7, "最近 7 天"), (30, "最近 30 天") })
        {
            period.Items.Add(new ComboBoxItem { Content = name, Tag = days });
        }
        period.SelectedIndex = 1;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(period, "时间范围");
        period.SelectionChanged += (_, _) => Load();
        refresh.Click += (_, _) => Load();

        var header = new Grid { ColumnSpacing = 8, Padding = new Thickness(16, 0, 16, 8) };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(pages);
        period.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(period, 1);
        header.Children.Add(period);
        refresh.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(refresh, 2);
        header.Children.Add(refresh);
        Ui.Row(root, header, GridLength.Auto);

        var about = new Grid { Padding = new Thickness(20, 0, 20, 8) };
        about.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        about.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        about.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        about.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        about.Children.Add(context);
        Grid.SetColumn(updated, 1);
        about.Children.Add(updated);
        var problems = new Button { Content = "问题" };
        problems.Click += (_, _) => openProblems();
        notice.ActionButton = problems;
        Grid.SetRow(notice, 1);
        Grid.SetColumnSpan(notice, 2);
        about.Children.Add(notice);
        Ui.Row(root, about, GridLength.Auto);

        BuildBreakdown();
        BuildRequests();
        host.Children.Add(overview);
        host.Children.Add(breakdown);
        host.Children.Add(requests);
        host.Children.Add(waiting);
        Ui.Row(root, Ui.Divided(host), new GridLength(1, GridUnitType.Star));
        WorkspaceWindow.Size(this, 860, 620, 680, 460);

        foreach (var (name, speed) in new[] { ("Token 用量", false), ("有效输出速率", true) })
        {
            trendMetric.Items.Add(new SelectorBarItem { Text = name, Tag = speed });
        }
        trendMetric.SelectedItem = trendMetric.Items[0];
        trendMetric.SelectionChanged += (_, _) => ShowOverview();
        Load();
        ShowPage();
    }

    Page Shown => (Page)pages.SelectedItem.Tag;

    void ShowPage()
    {
        var any = report != null;
        overview.Visibility = any && Shown == Page.Overview ? Visibility.Visible : Visibility.Collapsed;
        breakdown.Visibility = any && Shown == Page.Breakdown ? Visibility.Visible : Visibility.Collapsed;
        requests.Visibility = any && Shown == Page.Requests ? Visibility.Visible : Visibility.Collapsed;
        waiting.Visibility = any ? Visibility.Collapsed : Visibility.Visible;
        waiting.Text = failed ? "读取失败。" : "正在读取…";
    }

    // Load asks for the chosen period and scope; only the latest answer shows.
    async void Load()
    {
        var boundaries = Boundaries((int)((ComboBoxItem)period.SelectedItem).Tag);
        var query = new BindingAnalyticsQuery(boundaries[0], boundaries[^1], boundaries, scope.ProviderId, scope.ModelRecordKey, requestLimit);
        var wanted = scope;
        var ask = ++asked;
        loading = true;
        ShowState();
        try
        {
            var answer = await Task.Run(() => core.Local.AnalyticsQuery(query));
            if (ask != asked)
            {
                return;
            }
            Problems.Recovered(ReadKey);
            var sameScope = loaded is { } before && before.ProviderId == query.ProviderId && before.ModelRecordKey == query.ModelRecordKey;
            report = answer;
            loaded = query;
            loadedScope = wanted;
            loadedAt = DateTimeOffset.Now;
            failed = false;
            if (answer.StorageWarning is { } warning && warning != reportedWarning)
            {
                Problems.Report("保存用量记录", answer.DroppedCount > 0 ? $"{warning}（{answer.DroppedCount} 条未保存）" : warning);
            }
            reportedWarning = answer.StorageWarning;
            ShowOverview();
            ShowBreakdown();
            ShowRequests(sameScope);
        }
        catch (Exception failure)
        {
            if (ask != asked)
            {
                return;
            }
            failed = true;
            Problems.Failing(ReadKey, "读取分析", failure);
        }
        finally
        {
            if (ask == asked)
            {
                loading = false;
                ShowState();
                ShowPage();
            }
        }
    }

    // Boundaries are local midnights from days ago to now: one bucket a day,
    // the last one ending now.
    static long[] Boundaries(int days)
    {
        var now = DateTime.Now;
        var boundaries = new List<long>();
        for (var day = DateTime.Today.AddDays(1 - days); day < now; day = day.AddDays(1))
        {
            boundaries.Add(new DateTimeOffset(day).ToUnixTimeMilliseconds());
        }
        boundaries.Add(new DateTimeOffset(now).ToUnixTimeMilliseconds());
        return [.. boundaries];
    }

    void ShowState()
    {
        refresh.IsEnabled = !loading;
        if (loaded is { } query)
        {
            var from = Local(query.FromMs);
            var to = Local(query.ToMs);
            context.Text = $"{from:yyyy-MM-dd}–{to:yyyy-MM-dd} · {TimeZoneInfo.Local.DisplayName} · {loadedScope.Name}";
            ToolTipService.SetToolTip(context, $"{from:yyyy-MM-dd HH:mm} 至 {to:yyyy-MM-dd HH:mm}（不含结束时刻）");
            updated.Text = "更新于 " + loadedAt.ToLocalTime().ToString("HH:mm");
        }
        else
        {
            context.Text = "";
            updated.Text = "";
        }
        if (report != null && loading)
        {
            Notice(InfoBarSeverity.Informational, "正在读取新范围，当前仍显示上次结果。", false);
        }
        else if (failed)
        {
            Notice(InfoBarSeverity.Error, report != null ? "读取失败，保留上次结果。" : "读取失败。", true);
        }
        else if (report?.StorageWarning != null)
        {
            Notice(InfoBarSeverity.Warning, "部分用量记录未能保存。", true);
        }
        else
        {
            notice.IsOpen = false;
        }
    }

    void Notice(InfoBarSeverity severity, string message, bool problems)
    {
        notice.Severity = severity;
        notice.Message = message;
        notice.ActionButton.Visibility = problems ? Visibility.Visible : Visibility.Collapsed;
        notice.IsOpen = true;
    }

    // 概览: reported tokens, performance, how the requests ended, the daily
    // trend and what the figures mean. Rebuilt in place, its scroll stays.
    void ShowOverview()
    {
        if (report is not { } shown)
        {
            return;
        }
        var totals = shown.Overview;
        var panel = new StackPanel { Spacing = 20 };

        var figures = new Grid { ColumnSpacing = 28 };
        figures.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(2, GridUnitType.Star) });
        figures.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var tokens = new StackPanel { Spacing = 12 };
        tokens.Children.Add(Ui.Text("已报告 Token", "BodyStrongTextBlockStyle"));
        var pair = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 32 };
        pair.Children.Add(Metric("输入", Tokens(totals.InputTokens), Coverage(totals.InputReportedCount, totals.RequestCount)));
        pair.Children.Add(Metric("输出", Tokens(totals.OutputTokens), Coverage(totals.OutputReportedCount, totals.RequestCount)));
        tokens.Children.Add(pair);
        figures.Children.Add(tokens);
        var performance = new StackPanel { Spacing = 12 };
        performance.Children.Add(Ui.Text("性能", "BodyStrongTextBlockStyle"));
        performance.Children.Add(Metric("有效输出速率", Speed(totals.OutputTokensPerSecond), $"{totals.EligibleSpeedCount} / {totals.RequestCount} 次请求可计算"));
        ToolTipService.SetToolTip(performance, SpeedDefinition);
        Grid.SetColumn(performance, 1);
        figures.Children.Add(performance);
        panel.Children.Add(figures);

        var ended = new StackPanel { Spacing = 4 };
        ended.Children.Add(Ui.Text($"{totals.RequestCount:N0} 次请求", "BodyStrongTextBlockStyle"));
        ended.Children.Add(Ui.Muted(Ended(totals)));
        panel.Children.Add(ended);

        if (loaded?.BucketBoundariesMs.Length > 2)
        {
            panel.Children.Add(Trend(shown));
        }
        else
        {
            panel.Children.Add(Ui.Muted("今日用量与性能见上方汇总。每日趋势可切换至最近 7 天或 30 天。"));
        }

        var notes = new StackPanel { Spacing = 8 };
        notes.Children.Add(Ui.Muted($"完整报告总量：{Tokens(totals.TotalTokens)} Token（{Coverage(totals.TotalReportedCount, totals.RequestCount)}）"));
        notes.Children.Add(Ui.Muted(TotalDefinition));
        notes.Children.Add(Ui.Muted(SpeedDefinition));
        notes.Children.Add(Ui.Muted("仅统计 Velune LLM 网关新请求的上游用量报告，不读取 Agent 运行时历史，也没有费用记录。— 表示未知，0 表示已报告为零。"));
        panel.Children.Add(new Expander
        {
            Header = "指标与数据来源说明",
            Content = notes,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
        });
        overview.Content = panel;
    }

    static StackPanel Metric(string title, string value, string coverage)
    {
        var metric = new StackPanel { Spacing = 4 };
        metric.Children.Add(Ui.Muted(title));
        metric.Children.Add(Ui.Text(value, "SubtitleTextBlockStyle"));
        metric.Children.Add(Ui.Text(coverage, "CaptionTextBlockStyle"));
        return metric;
    }

    // Trend draws each day's reported input and output, or its output rate.
    // Unreported days stay empty rather than zero.
    StackPanel Trend(BindingAnalyticsReport shown)
    {
        var speed = (bool)trendMetric.SelectedItem.Tag;
        var trend = new StackPanel { Spacing = 10 };
        var title = new Grid();
        title.Children.Add(Ui.Text("每日趋势", "BodyStrongTextBlockStyle"));
        trendMetric.HorizontalAlignment = HorizontalAlignment.Right;
        if (trendMetric.Parent is Panel previous)
        {
            previous.Children.Remove(trendMetric);
        }
        title.Children.Add(trendMetric);
        trend.Children.Add(title);

        var buckets = shown.Trend;
        var reading = Ui.Muted(speed ? "速率包含等待时间。" : "输入、输出按各自报告绘制；未报告不补零。");
        var any = buckets.Any(bucket => speed ? bucket.Totals.OutputTokensPerSecond != null : bucket.Totals.InputTokens != null || bucket.Totals.OutputTokens != null);
        if (!any)
        {
            var none = Ui.Muted(speed ? "暂无可计算的有效输出速率" : "暂无已报告的 Token 用量");
            none.Height = 160;
            none.TextAlignment = TextAlignment.Center;
            none.Padding = new Thickness(0, 64, 0, 0);
            trend.Children.Add(none);
            return trend;
        }
        const double height = 160;
        var input = (Brush)Application.Current.Resources["AccentFillColorDefaultBrush"];
        var output = (Brush)Application.Current.Resources["ControlStrongFillColorDefaultBrush"];
        var largest = buckets.Max(bucket => speed
            ? bucket.Totals.OutputTokensPerSecond ?? 0
            : Math.Max(bucket.Totals.InputTokens ?? 0, bucket.Totals.OutputTokens ?? 0));
        double Scale(double value) => largest > 0 ? Math.Max(1, value / largest * height) : 1;
        var chart = new Grid { ColumnSpacing = 4, Height = height + 22 };
        foreach (var bucket in buckets)
        {
            chart.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var day = Local(bucket.FromMs);
            var column = new Grid { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
            column.RowDefinitions.Add(new RowDefinition { Height = new GridLength(height) });
            column.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var bars = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Bottom };
            if (speed)
            {
                if (bucket.Totals.OutputTokensPerSecond is { } rate)
                {
                    bars.Children.Add(new Ellipse { Width = 8, Height = 8, Fill = input, Margin = new Thickness(0, 0, 0, Scale(rate) - 4) });
                }
            }
            else
            {
                foreach (var (value, fill) in new[] { (bucket.Totals.InputTokens, input), (bucket.Totals.OutputTokens, output) })
                {
                    if (value is { } reported)
                    {
                        bars.Children.Add(new Rectangle { Width = 8, Height = Scale(reported), Fill = fill, RadiusX = 2, RadiusY = 2, VerticalAlignment = VerticalAlignment.Bottom });
                    }
                }
            }
            column.Children.Add(bars);
            var label = new TextBlock { Text = day.ToString("MM-dd"), Style = Ui.Style("CaptionTextBlockStyle"), HorizontalAlignment = HorizontalAlignment.Center };
            Grid.SetRow(label, 1);
            column.Children.Add(label);
            var detail = $"{day:yyyy-MM-dd} · 输入：{Tokens(bucket.Totals.InputTokens)} · 输出：{Tokens(bucket.Totals.OutputTokens)} · 有效输出速率：{Speed(bucket.Totals.OutputTokensPerSecond)}";
            ToolTipService.SetToolTip(column, detail);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(column, detail);
            column.PointerEntered += (_, _) => reading.Text = detail;
            Grid.SetColumn(column, chart.ColumnDefinitions.Count - 1);
            chart.Children.Add(column);
        }
        // Too many days to label each: label every few.
        if (buckets.Length > 10)
        {
            foreach (var label in chart.Children.OfType<Grid>().Select(column => column.Children.OfType<TextBlock>().First()).Where((_, i) => i % 5 != 0))
            {
                label.Opacity = 0;
            }
        }
        trend.Children.Add(chart);
        if (!speed)
        {
            var legend = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 16 };
            foreach (var (name, fill) in new[] { ("输入", input), ("输出", output) })
            {
                var key = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
                key.Children.Add(new Rectangle { Width = 10, Height = 10, Fill = fill, RadiusX = 2, RadiusY = 2, VerticalAlignment = VerticalAlignment.Center });
                key.Children.Add(new TextBlock { Text = name, Style = Ui.Style("CaptionTextBlockStyle") });
                legend.Children.Add(key);
            }
            trend.Children.Add(legend);
        }
        trend.Children.Add(reading);
        return trend;
    }

    // 提供商与模型: each source's requests, tokens and rate, side by side.
    void BuildBreakdown()
    {
        breakdownMode.Items.Add(new SelectorBarItem { Text = "提供商", Tag = false });
        breakdownMode.Items.Add(new SelectorBarItem { Text = "模型", Tag = true });
        breakdownMode.SelectedItem = breakdownMode.Items[0];
        breakdownMode.SelectionChanged += (_, _) => ShowBreakdown();
        showRequests.Click += (_, _) => ShowRequestsOf(ChosenSource());
        breakdownList.SelectionChanged += (_, _) => showRequests.IsEnabled = ChosenSource() != null;
        breakdownList.DoubleTapped += (_, _) => ShowRequestsOf(ChosenSource());
        breakdownList.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter)
            {
                e.Handled = true;
                ShowRequestsOf(ChosenSource());
            }
        };
        var bar = new Grid { Padding = new Thickness(16, 8, 20, 8) };
        bar.Children.Add(breakdownMode);
        showRequests.HorizontalAlignment = HorizontalAlignment.Right;
        showRequests.IsEnabled = false;
        bar.Children.Add(showRequests);
        breakdown.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        breakdown.Children.Add(bar);
        Ui.Row(breakdown, breakdownList, new GridLength(1, GridUnitType.Star));
    }

    BindingAnalyticsBreakdown? ChosenSource() => (breakdownList.SelectedItem as ListViewItem)?.Tag as BindingAnalyticsBreakdown;

    void ShowBreakdown()
    {
        if (report is not { } shown)
        {
            return;
        }
        var models = (bool)breakdownMode.SelectedItem.Tag;
        breakdownList.Header = Columns(BreakdownWidths, models ? "模型" : "提供商", "请求", "输入", "输出", "有效输出速率");
        Keep(breakdownList, ChosenSource()?.Key, models == (breakdownList.Tag as bool?), () =>
        {
            breakdownList.Tag = models;
            foreach (var source in models ? shown.Models : shown.Providers)
            {
                FrameworkElement name = models
                    ? Ui.TwoLines(source.Name, shown.Providers.FirstOrDefault(provider => provider.ProviderId == source.ProviderId)?.Name ?? "未知提供商")
                    : new TextBlock { Text = source.Name, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center };
                ToolTipService.SetToolTip(name, source.Name);
                var totals = source.Totals;
                var row = Cells(BreakdownWidths, name,
                    Figure($"{totals.RequestCount:N0}"),
                    Figure(Tokens(totals.InputTokens), Coverage(totals.InputReportedCount, totals.RequestCount)),
                    Figure(Tokens(totals.OutputTokens), Coverage(totals.OutputReportedCount, totals.RequestCount)),
                    Figure(Speed(totals.OutputTokensPerSecond), $"{totals.EligibleSpeedCount} / {totals.RequestCount} 次请求可计算"));
                var item = new ListViewItem { Content = row, Tag = source, HorizontalContentAlignment = HorizontalAlignment.Stretch };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, source.Name);
                item.ContextFlyout = Ui.Menu(Ui.MenuItem("查看请求…", () => ShowRequestsOf(source)));
                breakdownList.Items.Add(item);
            }
            if (breakdownList.Items.Count == 0)
            {
                breakdownList.Header = Ui.Muted("这段时间没有经 Velune 网关的请求。");
                ((FrameworkElement)breakdownList.Header).Margin = new Thickness(20, 12, 20, 12);
            }
        }, item => ((item as ListViewItem)?.Tag as BindingAnalyticsBreakdown)?.Key);
        showRequests.IsEnabled = ChosenSource() != null;
    }

    static readonly GridLength[] BreakdownWidths =
    [
        new(1, GridUnitType.Star), new(70), new(110), new(110), new(150),
    ];

    // ShowRequestsOf narrows 请求 to a source and goes there.
    void ShowRequestsOf(BindingAnalyticsBreakdown? source)
    {
        if (source == null)
        {
            return;
        }
        scope = source.ModelRecordKey is { } model
            ? new Scope(source.Name, source.ProviderId, model)
            : new Scope(source.Name, source.ProviderId, null);
        requestLimit = RequestStep;
        pages.SelectedItem = pages.Items[(int)Page.Requests];
        Load();
    }

    // 请求: single requests, newest first, each with a read-only detail.
    void BuildRequests()
    {
        clearFilter.Click += (_, _) =>
        {
            scope = new Scope("全部提供商", null, null);
            requestLimit = RequestStep;
            Load();
        };
        showDetail.Click += (_, _) => Detail(ChosenRequest());
        more.Click += (_, _) =>
        {
            requestLimit = Math.Min(RequestMost, requestLimit + RequestStep);
            Load();
        };
        requestList.SelectionChanged += (_, _) => showDetail.IsEnabled = ChosenRequest() != null;
        requestList.DoubleTapped += (_, _) => Detail(ChosenRequest());
        requestList.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter)
            {
                e.Handled = true;
                Detail(ChosenRequest());
            }
        };
        var bar = new Grid { Padding = new Thickness(20, 8, 20, 8), ColumnSpacing = 8 };
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        bar.Children.Add(filter);
        Grid.SetColumn(clearFilter, 1);
        bar.Children.Add(clearFilter);
        Grid.SetColumn(showDetail, 3);
        showDetail.IsEnabled = false;
        bar.Children.Add(showDetail);
        requests.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        requests.Children.Add(bar);
        Ui.Row(requests, requestList, new GridLength(1, GridUnitType.Star));
        var footer = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, Padding = new Thickness(20, 8, 20, 10) };
        footer.Children.Add(requestCount);
        footer.Children.Add(more);
        footer.Children.Add(most);
        Ui.Row(requests, footer, GridLength.Auto);
    }

    BindingAnalyticsRequest? ChosenRequest() => (requestList.SelectedItem as ListViewItem)?.Tag as BindingAnalyticsRequest;

    static readonly GridLength[] RequestWidths = [new(130), new(1, GridUnitType.Star), new(80), new(110)];

    void ShowRequests(bool sameScope)
    {
        if (report is not { } shown)
        {
            return;
        }
        filter.Text = "筛选：" + loadedScope.Name;
        clearFilter.Visibility = loadedScope.ProviderId != null ? Visibility.Visible : Visibility.Collapsed;
        requestList.Header = Columns(RequestWidths, "时间", "提供商 / 模型", "结果", "输出 Token");
        Keep(requestList, ChosenRequest()?.RequestId, sameScope, () =>
        {
            foreach (var request in shown.Requests)
            {
                var source = Ui.TwoLines(request.ProviderName, request.ProviderModelId);
                var row = Cells(RequestWidths,
                    Figure(Local(request.StartedAtMs).ToString("MM-dd HH:mm"), alignRight: false),
                    source,
                    new TextBlock { Text = Outcome(request.Outcome), VerticalAlignment = VerticalAlignment.Center },
                    Figure(Tokens(request.OutputTokens)));
                var item = new ListViewItem { Content = row, Tag = request, HorizontalContentAlignment = HorizontalAlignment.Stretch };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, $"{request.ProviderName} {request.ProviderModelId} {Outcome(request.Outcome)}");
                item.ContextFlyout = Ui.Menu(Ui.MenuItem("显示详情…", () => Detail(request)));
                requestList.Items.Add(item);
            }
            if (requestList.Items.Count == 0)
            {
                requestList.Header = Ui.Muted("这段时间没有经 Velune 网关的请求。");
                ((FrameworkElement)requestList.Header).Margin = new Thickness(20, 12, 20, 12);
            }
        }, item => ((item as ListViewItem)?.Tag as BindingAnalyticsRequest)?.RequestId);
        showDetail.IsEnabled = ChosenRequest() != null;
        var total = shown.Overview.RequestCount;
        requestCount.Text = $"最近 {shown.Requests.Length} / {total} 条请求记录";
        var partial = total > (ulong)shown.Requests.Length;
        more.Visibility = partial && requestLimit < RequestMost ? Visibility.Visible : Visibility.Collapsed;
        most.Visibility = partial && requestLimit >= RequestMost ? Visibility.Visible : Visibility.Collapsed;
    }

    // Detail shows one request read-only: what it was and how it went, its
    // reasoning and cache usage, and its identifiers.
    async void Detail(BindingAnalyticsRequest? request)
    {
        if (request == null || Content.XamlRoot == null)
        {
            return;
        }
        var panel = new StackPanel { Spacing = 16 };
        panel.Children.Add(Facts(
            ("结果", Outcome(request.Outcome)),
            ("提供商", request.ProviderName),
            ("模型 ID", request.ProviderModelId),
            ("协议", Protocol(request.Protocol)),
            ("开始", Local(request.StartedAtMs).ToString("yyyy-MM-dd HH:mm:ss")),
            ("输入 Token", Tokens(request.InputTokens)),
            ("输出 Token", Tokens(request.OutputTokens)),
            ("有效输出速率", Speed(request.OutputTokensPerSecond)),
            ("请求耗时", Duration(request.ElapsedMs)),
            ("首输出延迟", Duration(request.FirstOutputMs))));
        var usage = new StackPanel { Spacing = 8 };
        usage.Children.Add(Facts(
            ("推理 Token", Tokens(request.ReasoningOutputTokens)),
            ("普通输入", Tokens(request.UncachedInputTokens)),
            ("缓存输入", Tokens(request.CachedInputTokens)),
            ("缓存读取", Tokens(request.CacheReadInputTokens)),
            ("缓存创建", Tokens(request.CacheCreationInputTokens))));
        usage.Children.Add(Ui.Muted("缓存字段按上游协议保留，不再次加到已归一的输入中；推理 Token 属于输出子集。"));
        panel.Children.Add(Section("推理与缓存用量", usage));
        panel.Children.Add(Section("请求信息", Facts(
            ("请求编号", request.RequestId),
            ("HTTP 状态", request.Status?.ToString() ?? "—"),
            ("业务终态耗时", Duration(request.TerminalElapsedMs)),
            ("用量报告", request.UsageReported ? request.UsageComplete ? "完整" : "部分" : "未报告"))));
        var notes = new StackPanel { Spacing = 8 };
        notes.Children.Add(Ui.Muted(SpeedDefinition));
        notes.Children.Add(Ui.Muted("首输出延迟从上游派发到首次正文、推理或工具内容；非流式响应未知。请求耗时涵盖完成接收、失败或取消；速率仅使用可确认的业务终态耗时。— 表示未知。"));
        panel.Children.Add(Section("指标说明", notes));
        var dialog = new ContentDialog
        {
            Title = "请求详情",
            Content = new ScrollViewer { Content = panel, MaxHeight = 460 },
            CloseButtonText = "完成",
            DefaultButton = ContentDialogButton.Close,
            XamlRoot = Content.XamlRoot,
        };
        await dialog.ShowAsync();

        static Expander Section(string title, UIElement content) => new()
        {
            Header = title,
            Content = content,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
        };
    }

    // Facts lists labelled values whose text can be selected and copied.
    static Grid Facts(params (string Label, string Value)[] facts)
    {
        var grid = new Grid { ColumnSpacing = 12, RowSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(96) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        foreach (var (label, value) in facts)
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var name = Ui.Muted(label);
            Grid.SetRow(name, grid.RowDefinitions.Count - 1);
            grid.Children.Add(name);
            var text = new TextBlock { Text = value, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap };
            Grid.SetRow(text, grid.RowDefinitions.Count - 1);
            Grid.SetColumn(text, 1);
            grid.Children.Add(text);
        }
        return grid;
    }

    // Keep rebuilds a list in place: the same scope keeps its selection and
    // scroll position, another starts at the top.
    static void Keep(ListView list, string? selected, bool same, Action rebuild, Func<object, string?> key)
    {
        var scroller = Ui.Descendant<ScrollViewer>(list);
        var offset = same ? scroller?.VerticalOffset ?? 0 : 0;
        list.Items.Clear();
        rebuild();
        if (same && selected != null && list.Items.FirstOrDefault(item => key(item) == selected) is { } keep)
        {
            list.SelectedItem = keep;
        }
        if (scroller != null)
        {
            list.UpdateLayout();
            scroller.ChangeView(null, offset, null, true);
        }
    }

    static Grid Columns(GridLength[] widths, params string[] titles) =>
        Cells(widths, [.. titles.Select((title, i) => (FrameworkElement)new TextBlock
        {
            Text = title,
            Style = Ui.Style("CaptionTextBlockStyle"),
            HorizontalAlignment = i == 0 || widths[i].IsStar ? HorizontalAlignment.Left : HorizontalAlignment.Right,
        })], header: true);

    static Grid Cells(GridLength[] widths, params FrameworkElement[] cells) => Cells(widths, cells, header: false);

    static Grid Cells(GridLength[] widths, FrameworkElement[] cells, bool header)
    {
        var row = new Grid { ColumnSpacing = 12, Padding = header ? new Thickness(16, 0, 12, 6) : new Thickness(0, 6, 0, 6) };
        for (var i = 0; i < widths.Length; i++)
        {
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = widths[i] });
            Grid.SetColumn(cells[i], i);
            row.Children.Add(cells[i]);
        }
        return row;
    }

    static TextBlock Figure(string value, string tip = "", bool alignRight = true)
    {
        var figure = new TextBlock
        {
            Text = value,
            HorizontalAlignment = alignRight ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Center,
        };
        if (tip.Length > 0)
        {
            ToolTipService.SetToolTip(figure, tip);
        }
        return figure;
    }

    static string Ended(BindingAnalyticsTotals totals)
    {
        var parts = new List<string> { $"{totals.CompletedCount} 次完成" };
        foreach (var (count, name) in new[] { (totals.FailedCount, "失败"), (totals.CancelledCount, "取消"), (totals.IncompleteCount, "未完成"), (totals.RejectedCount, "拒绝") })
        {
            if (count > 0)
            {
                parts.Add($"{count} 次{name}");
            }
        }
        return string.Join(" · ", parts);
    }

    static DateTime Local(long ms) => DateTimeOffset.FromUnixTimeMilliseconds(ms).LocalDateTime;
    static string Coverage(ulong known, ulong total) => $"{known} / {total} 次请求报告";
    static string Tokens(ulong? value) => value?.ToString("N0", CultureInfo.CurrentCulture) ?? "—";
    static string Speed(double? value) => value is { } rate && double.IsFinite(rate) ? rate.ToString("N1", CultureInfo.CurrentCulture) + " Token/s" : "—";

    static string Duration(ulong? ms) => ms is { } value
        ? value >= 60_000 ? $"{value / 60_000} 分 {value % 60_000 / 1000.0:0.00} 秒" : $"{value / 1000.0:0.00} 秒"
        : "—";

    static string Outcome(BindingAnalyticsOutcome outcome) => outcome switch
    {
        BindingAnalyticsOutcome.Completed => "完成",
        BindingAnalyticsOutcome.Incomplete => "未完成",
        BindingAnalyticsOutcome.Failed => "失败",
        BindingAnalyticsOutcome.Cancelled => "取消",
        BindingAnalyticsOutcome.Rejected => "拒绝",
        _ => outcome.ToString(),
    };

    static string Protocol(BindingAnalyticsProtocol protocol) => protocol switch
    {
        BindingAnalyticsProtocol.ChatCompletions => "OpenAI ChatCompletions",
        BindingAnalyticsProtocol.Responses => "OpenAI Responses",
        BindingAnalyticsProtocol.Messages => "Anthropic Messages",
        _ => protocol.ToString(),
    };
}
