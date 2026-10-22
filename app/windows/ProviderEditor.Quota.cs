using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The editor's quota and usage limits: the windows the provider reports, as
// the core last observed them, and a used share per window at which every
// route and direct call stops sending new requests to it.
sealed partial class ProviderEditor
{
    readonly StackPanel quotaWindows = new() { Spacing = 4 };
    readonly TextBlock quotaState = Ui.Muted("");
    readonly Button refreshQuota = new() { Content = "刷新" };
    readonly Dictionary<string, (TextBlock Detail, NumberBox Limit)> windows = [];
    DispatcherTimer? quotaTimer;
    bool reading;

    void BuildQuota()
    {
        if (quotaSource == BindingProviderQuotaSource.None)
        {
            return;
        }
        Section("额度与使用上限");
        var remaining = page.Usage == BindingUsageDisplay.Remaining;
        var about = remaining
            ? "为每个额度窗口设置至少保留的比例，剩余低于它时暂停新请求；对所有路由和直接调用共同生效。"
            : "为每个额度窗口设置已用比例上限，达到时暂停新请求；对所有路由和直接调用共同生效。";
        var state = new StackPanel { Spacing = 2 };
        state.Children.Add(Ui.Text(about + "上限依据供应商观测，不是计费硬预算。"));
        state.Children.Add(quotaState);
        refreshQuota.Click += (_, _) => ReadQuota(true);
        body.Children.Add(Ui.Card(state, refreshQuota));
        body.Children.Add(quotaWindows);
        ShowQuotaState(null);
    }

    // CanReadQuota tells whether the provider is saved and signed in, the
    // only time its quota is read.
    bool CanReadQuota => quotaSource != BindingProviderQuotaSource.None && existing is { Authentication.Configured: true };

    void StartQuota()
    {
        if (!IsLoaded || !CanReadQuota)
        {
            ShowQuotaState(null);
            return;
        }
        quotaTimer ??= new DispatcherTimer();
        quotaTimer.Tick -= QuotaTick;
        quotaTimer.Tick += QuotaTick;
        ReadQuota(false);
    }

    void StopQuota() => quotaTimer?.Stop();

    void QuotaTick(object? sender, object e) => ReadQuota(false);

    // ReadQuota reads the core's observation, which decides itself whether
    // to sample: soon while it reads or has none, then every half minute.
    async void ReadQuota(bool refresh)
    {
        if (reading || existing is not { } provider || !CanReadQuota)
        {
            return;
        }
        reading = true;
        quotaTimer?.Stop();
        refreshQuota.IsEnabled = false;
        BindingProviderQuotaStatus? status = null;
        try
        {
            status = await Task.Run(() => refresh
                ? page.Core.Local.RefreshProviderQuota(ProvidersPage.DefaultGateway, provider.Id)
                : page.Core.Local.ProviderQuota(ProvidersPage.DefaultGateway, provider.Id));
            ShowQuota(status);
            Problems.Recovered(Problems.Key("quota", Device.Local, provider.Id));
        }
        catch (Exception problem)
        {
            quotaState.Text = Core.Describe(problem);
            Problems.Failing(Problems.Key("quota", Device.Local, provider.Id), "读取提供商「" + provider.Name + "」的额度", problem);
        }
        reading = false;
        refreshQuota.IsEnabled = true;
        if (IsLoaded && quotaTimer != null)
        {
            var soon = status == null || status.Refreshing || status.Windows.Length == 0 && status.Freshness == BindingQuotaFreshness.Unknown;
            quotaTimer.Interval = TimeSpan.FromSeconds(soon ? 2 : 30);
            quotaTimer.Start();
        }
    }

    void ShowQuotaState(BindingProviderQuotaStatus? status)
    {
        refreshQuota.Visibility = CanReadQuota ? Visibility.Visible : Visibility.Collapsed;
        quotaState.Text = existing == null ? "保存并完成认证后，这里显示额度窗口。"
            : !CanReadQuota ? "登录后显示额度窗口。"
            : status == null ? "正在读取额度…"
            : status.Problem is { Length: > 0 } problem ? problem
            : status.Freshness switch
            {
                BindingQuotaFreshness.Unsupported => "此提供商不报告额度。",
                BindingQuotaFreshness.Unknown when status.Refreshing => "正在读取额度…",
                BindingQuotaFreshness.Unknown => "额度未知。",
                BindingQuotaFreshness.Stale => "额度观测已过期，将重新读取。",
                _ when status.Available == false => "当前暂停新请求：已达到使用上限，或设有上限而缺少最新额度。",
                _ => "",
            };
        quotaState.Visibility = quotaState.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    void ShowQuota(BindingProviderQuotaStatus status)
    {
        ShowQuotaState(status);
        foreach (var window in status.Windows)
        {
            if (!windows.TryGetValue(window.Id, out var row))
            {
                row = AddWindow(window.Id);
            }
            row.Detail.Text = Quota.Detail(window, status.Freshness, page.Usage);
        }
    }

    // AddWindow shows one quota window with its limit, which keeps what the
    // user typed while the window's observation updates.
    (TextBlock Detail, NumberBox Limit) AddWindow(string id)
    {
        var remaining = page.Usage == BindingUsageDisplay.Remaining;
        var limit = new NumberBox
        {
            Minimum = remaining ? 0 : 1,
            Maximum = remaining ? 99 : 100,
            PlaceholderText = "不设上限",
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            MinWidth = 140,
            Header = remaining ? "至少保留（%）" : "已用上限（%）",
        };
        limit.Value = existing?.QuotaLimits.FirstOrDefault(saved => saved.WindowId == id) is { } kept
            ? remaining ? 100 - kept.MaxUsedPercent : kept.MaxUsedPercent
            : double.NaN;
        limit.ValueChanged += (_, _) => Changed();
        var detail = Ui.Muted("");
        var lines = new StackPanel { Spacing = 2 };
        lines.Children.Add(Ui.Text(Quota.Name(id)));
        lines.Children.Add(detail);
        quotaWindows.Children.Add(Ui.Card(lines, limit));
        windows[id] = (detail, limit);
        return windows[id];
    }

    // Limits are the saved limits with the shown windows' edits; a window
    // not reported now keeps its limit.
    BindingQuotaLimit[] Limits()
    {
        var remaining = page.Usage == BindingUsageDisplay.Remaining;
        var limits = (existing?.QuotaLimits ?? []).Where(saved => !windows.ContainsKey(saved.WindowId)).ToList();
        foreach (var (id, row) in windows)
        {
            if (!double.IsNaN(row.Limit.Value))
            {
                var used = remaining ? 100 - row.Limit.Value : row.Limit.Value;
                limits.Add(new BindingQuotaLimit(id, (byte)Math.Clamp(Math.Round(used), 1, 100)));
            }
        }
        return [.. limits];
    }
}
