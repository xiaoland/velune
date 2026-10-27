using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The editor's quota and usage limits: the windows and amounts the provider
// reports, as the core last observed them, and a used share per window at
// which every route and direct call stops sending new requests to it.
sealed partial class ProviderEditor
{
    readonly StackPanel quotaSection = new() { Spacing = 4 };
    readonly StackPanel quotaWindows = new() { Spacing = 4 };
    readonly TextBlock quotaState = Ui.Muted("");
    readonly Button refreshQuota = new() { Content = "刷新" };
    readonly Button quotaHistory = new() { Content = "历史…" };
    readonly Dictionary<string, (TextBlock Detail, NumberBox Limit)> windows = [];
    readonly ToggleSwitch accountQuota = new() { OnContent = "开", OffContent = "关" };
    readonly TextBlock accountTitle = Ui.Text(""), accountDetail = Ui.Muted("");
    Border? accountCard;
    BindingProviderQuotaStatus? lastQuota;
    DispatcherTimer? quotaTimer;
    bool reading;

    // BuildAccountQuota offers reading the account's quota with the API key
    // of a provider that calls a service's official address directly.
    void BuildAccountQuota()
    {
        if (subscription || authenticationSource != BindingProviderAuthenticationSource.ApiKey
            || (existing?.ExecutionBackend ?? template?.ExecutionBackend) is BindingExecutionBackend.ClaudeCli)
        {
            return;
        }
        accountQuota.IsOn = quotaSource == BindingProviderQuotaSource.ApiAccount;
        accountQuota.Toggled += (_, _) =>
        {
            if (building)
            {
                return;
            }
            quotaSource = accountQuota.IsOn ? BindingProviderQuotaSource.ApiAccount : BindingProviderQuotaSource.None;
            Changed();
            ShowAccountQuota();
        };
        endpoint.TextChanged += (_, _) => ShowAccountQuota();
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(accountTitle);
        text.Children.Add(accountDetail);
        accountCard = Ui.Card(text, accountQuota);
        body.Children.Add(accountCard);
        ShowAccountQuota();
    }

    // AccountService is the official service at the address, if it is one.
    string? AccountService() => accountCard == null ? null : VeluneBindingsMethods.ApiAccountServiceName(endpoint.Text.Trim());

    void ShowAccountQuota()
    {
        if (accountCard == null)
        {
            return;
        }
        var service = AccountService();
        accountCard.Visibility = service == null ? Visibility.Collapsed : Visibility.Visible;
        accountTitle.Text = "读取 " + service + " 账户额度";
        accountDetail.Text = service == "OpenRouter"
            ? "用这个 API key 向 OpenRouter 读取它的花费上限与剩余额度；没有设置上限的 Key 不报告额度。"
            : "用这个 API key 向 " + service + " 官方地址读取账户余额；提供商表示余额不足时暂停新请求。";
        quotaSection.Visibility = quotaSource != BindingProviderQuotaSource.None && (service != null || quotaSource != BindingProviderQuotaSource.ApiAccount)
            ? Visibility.Visible : Visibility.Collapsed;
    }

    void BuildQuota()
    {
        if (quotaSource == BindingProviderQuotaSource.None && accountCard == null)
        {
            return;
        }
        body.Children.Add(quotaSection);
        quotaSection.Children.Add(Ui.Heading("额度与使用上限"));
        ShowAccountQuota();
        var remaining = page.Usage == BindingUsageDisplay.Remaining;
        var about = remaining
            ? "为每个额度窗口设置至少保留的比例，剩余低于它时暂停新请求；对所有路由和直接调用共同生效。"
            : "为每个额度窗口设置已用比例上限，达到时暂停新请求；对所有路由和直接调用共同生效。";
        var state = new StackPanel { Spacing = 2 };
        state.Children.Add(Ui.Text(about + "上限依据供应商观测，不是计费硬预算。"));
        state.Children.Add(quotaState);
        refreshQuota.Click += (_, _) => ReadQuota(true);
        quotaHistory.Click += async (_, _) =>
        {
            if (existing is { } provider)
            {
                await QuotaHistory.Show(page.Window, page.Core, provider, lastQuota, page.Usage);
            }
        };
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(quotaHistory);
        actions.Children.Add(refreshQuota);
        quotaSection.Children.Add(Ui.Card(state, actions));
        quotaSection.Children.Add(quotaWindows);
        ShowQuotaState(null);
    }

    // CanReadQuota tells whether the provider is saved with a quota source
    // and signed in, the only time its quota is read.
    bool CanReadQuota => existing is { Authentication.Configured: true } saved && saved.QuotaSource != BindingProviderQuotaSource.None;

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
        refreshQuota.Visibility = quotaHistory.Visibility = CanReadQuota ? Visibility.Visible : Visibility.Collapsed;
        quotaState.Text = existing == null || existing.QuotaSource == BindingProviderQuotaSource.None ? "保存并完成认证后，这里显示额度窗口。"
            : !CanReadQuota ? (authenticationSource == BindingProviderAuthenticationSource.ApiKey ? "配置 API key 后显示额度。" : "登录后显示额度窗口。")
            : status == null ? "正在读取额度…"
            : status.Problem is { Length: > 0 } problem ? problem
            : status.Freshness switch
            {
                BindingQuotaFreshness.Unsupported => "此提供商不报告额度。",
                BindingQuotaFreshness.Unknown when status.Refreshing => "正在读取额度…",
                BindingQuotaFreshness.Unknown => "额度未知。",
                BindingQuotaFreshness.Stale => "额度观测已过期，将重新读取。",
                _ when status.Available == false => "当前暂停新请求：已达到使用上限、余额不足，或设有上限而缺少最新额度。",
                _ => Quota.Balances(status),
            };
        quotaState.Visibility = quotaState.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    void ShowQuota(BindingProviderQuotaStatus status)
    {
        lastQuota = status;
        ShowQuotaState(status);
        foreach (var window in status.Windows)
        {
            if (!windows.TryGetValue(window.Id, out var row))
            {
                row = AddWindow(window.Id);
            }
            row.Detail.Text = Quota.Detail(window, status.Freshness, page.Usage);
            row.Detail.Style = Ui.Style(Quota.Short(window, status.Freshness) ? "CautionTextStyle" : "MutedTextStyle");
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
