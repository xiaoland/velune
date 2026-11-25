using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// The editor's quota and usage limits: the windows and amounts the provider
// reports, as the core last observed them, and a used share per window at
// which every route and direct call stops sending new requests to it.
sealed partial class ProviderEditor
{
    enum LimitControl { None, Edit, Remove }
    sealed record QuotaRow(Border Card, TextBlock Detail, LimitControl Control, FrameworkElement? Action);

    readonly StackPanel quotaSection = new() { Spacing = 4 };
    readonly StackPanel quotaWindows = new() { Spacing = 4 };
    readonly TextBlock quotaState = Ui.Muted("");
    readonly Button refreshQuota = new() { Content = "刷新" };
    readonly Button quotaHistory = new() { Content = "历史…" };
    readonly Dictionary<string, QuotaRow> windows = [];
    readonly Dictionary<string, byte> draftLimits = [];
    readonly ToggleSwitch accountQuota = new() { OnContent = "开", OffContent = "关" };
    readonly TextBlock accountTitle = Ui.Text(""), accountDetail = Ui.Muted("");
    Border? accountCard;
    QuotaObservation? lastQuota;
    DispatcherTimer? quotaTimer;
    bool reading;

    // BuildAccountQuota offers reading the account's quota with the API key
    // of a provider that calls a service's official address directly.
    void BuildAccountQuota()
    {
        if (subscription || authenticationSource != ProviderAuthenticationSource.ApiKey || UsesCli)
        {
            return;
        }
        accountQuota.IsOn = quotaSource == ProviderQuotaSource.ApiAccount;
        accountQuota.Toggled += (_, _) =>
        {
            if (building)
            {
                return;
            }
            quotaSource = accountQuota.IsOn ? ProviderQuotaSource.ApiAccount : ProviderQuotaSource.None;
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
        quotaSection.Visibility = quotaSource != ProviderQuotaSource.None && (service != null || quotaSource != ProviderQuotaSource.ApiAccount)
            ? Visibility.Visible : Visibility.Collapsed;
    }

    void BuildQuota()
    {
        foreach (var limit in existing?.QuotaLimits ?? [])
        {
            draftLimits[limit.WindowId] = limit.MaxUsedPercent;
        }
        if (!UsesAntigravity && quotaSource == ProviderQuotaSource.None && accountCard == null)
        {
            return;
        }
        body.Children.Add(quotaSection);
        quotaSection.Children.Add(Ui.Heading("额度与使用上限"));
        ShowAccountQuota();
        var remaining = page.Usage == UsageDisplay.Remaining;
        var about = remaining
            ? "为每个额度窗口设置至少保留的比例，剩余低于它时暂停新请求；对所有路由和直接调用共同生效。"
            : "为每个额度窗口设置已用比例上限，达到时暂停新请求；对所有路由和直接调用共同生效。";
        var state = new StackPanel { Spacing = 2 };
        state.Children.Add(Ui.Text(UsesAntigravity
            ? "额度按本机执行目标归属。上限适用于对应额度组的模型。已设上限的组失去模型映射时，暂停向此目标派发，直到恢复映射或保存移除上限。"
            : about + "上限依据供应商观测，不是计费硬预算。"));
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
        ShowQuotaWindows(null);
    }

    bool CanReadQuota => existing is { Authentication.Configured: true } saved
        && (UsesAntigravity
            ? saved.ExecutionBackend is ExecutionBackend.AntigravityCli { Binary: var path, SshDestination: null }
                && binary.Text.Trim() == path
            : saved.QuotaSource != ProviderQuotaSource.None);

    void StartQuota()
    {
        ShowQuotaState(lastQuota);
        if (UsesAntigravity)
        {
            ShowQuotaWindows(CanReadQuota ? lastQuota : null);
        }
        if (!IsLoaded || !CanReadQuota)
        {
            StopQuota();
            return;
        }
        quotaTimer ??= new DispatcherTimer();
        quotaTimer.Tick -= QuotaTick;
        quotaTimer.Tick += QuotaTick;
        ReadQuota(false);
    }

    void StopQuota() => quotaTimer?.Stop();

    void QuotaTick(object? sender, object e) => ReadQuota(false);

    // Reads the provider's observation and lets the core refresh it when due.
    async void ReadQuota(bool refresh)
    {
        if (reading || !IsLoaded || existing is not { } provider || !CanReadQuota || (refresh && UsesAntigravity && page.Busy))
        {
            return;
        }
        reading = true;
        quotaTimer?.Stop();
        refreshQuota.IsEnabled = false;
        QuotaObservation? status = null;
        try
        {
            status = await Task.Run(() => refresh
                ? page.Core.Local.RefreshProviderQuota(ProvidersPage.DefaultGateway, provider.Id).Observation
                : page.Core.Local.ProviderQuota(ProvidersPage.DefaultGateway, provider.Id).Observation);
            if (IsLoaded && existing is { } current && provider.Id == current.Id && provider.ExecutionBackend == current.ExecutionBackend)
            {
                ShowQuota(status);
                Problems.Recovered(Problems.Key("quota", Device.Local, provider.Id));
            }
        }
        catch (Exception problem)
        {
            if (IsLoaded && existing is { } current && provider.Id == current.Id && provider.ExecutionBackend == current.ExecutionBackend)
            {
                quotaState.Text = Core.Describe(problem);
                Problems.Failing(Problems.Key("quota", Device.Local, provider.Id), "读取提供商「" + provider.Name + "」的额度", problem);
            }
        }
        reading = false;
        if (IsLoaded && quotaTimer != null)
        {
            refreshQuota.IsEnabled = status?.Refreshing != true;
            if (!CanReadQuota)
            {
                return;
            }
            var soon = status?.Refreshing == true
                || (!UsesAntigravity && (status == null || status.Windows.Length == 0 && status.Freshness == QuotaFreshness.Unknown));
            quotaTimer.Interval = TimeSpan.FromSeconds(soon ? 2 : 30);
            quotaTimer.Start();
        }
    }

    void ShowQuotaState(QuotaObservation? status)
    {
        refreshQuota.Visibility = CanReadQuota ? Visibility.Visible : Visibility.Collapsed;
        quotaHistory.Visibility = CanReadQuota ? Visibility.Visible : Visibility.Collapsed;
        quotaState.Text = UsesAntigravity && !CanReadQuota ? "保存本机 agy 执行目标后读取额度。"
            : existing == null || (!UsesAntigravity && existing.QuotaSource == ProviderQuotaSource.None) ? "保存并完成认证后，这里显示额度窗口。"
            : !CanReadQuota ? (authenticationSource == ProviderAuthenticationSource.ApiKey ? "配置 API key 后显示额度。" : "登录后显示额度窗口。")
            : status == null ? "正在读取额度…"
            : status.Problem is { Length: > 0 } problem
                ? problem + (status.Freshness == QuotaFreshness.Stale ? " 上次观测已过期。" : "")
            : status.Freshness switch
            {
                QuotaFreshness.Unsupported => "此提供商不报告额度。",
                QuotaFreshness.Unknown when status.Refreshing => "正在读取额度…",
                QuotaFreshness.Unknown => "额度未知。",
                QuotaFreshness.Stale => "额度观测已过期，将重新读取。",
                _ when status.Available == false => Quota.LimitedObservation,
                _ => Quota.Balances(status),
            };
        quotaState.Visibility = quotaState.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    void ShowQuota(QuotaObservation status)
    {
        lastQuota = status;
        ShowQuotaState(status);
        ShowQuotaWindows(CanReadQuota ? status : null);
    }

    void ShowQuotaWindows(QuotaObservation? status)
    {
        var reported = status?.Windows ?? [];
        var now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        foreach (var window in reported)
        {
            var control = !UsesAntigravity || Quota.HasModelMapping(window)
                ? LimitControl.Edit
                : draftLimits.ContainsKey(window.Id) ? LimitControl.Remove : LimitControl.None;
            var row = WindowRow(window.Id, control);
            var freshness = UsesAntigravity ? (Quota.IsCurrent(window, now) ? QuotaFreshness.Fresh : QuotaFreshness.Stale)
                : status?.Freshness ?? QuotaFreshness.Unknown;
            row.Detail.Text = Quota.Detail(window, freshness, page.Usage)
                + (control == LimitControl.Remove ? " · " + UnmappedLimit(window.Id) : "");
            row.Detail.Style = Ui.Style(Quota.Short(window, freshness) ? "CautionTextStyle" : "MutedTextStyle");
        }
        if (UsesAntigravity)
        {
            var reportedIds = reported.Select(window => window.Id).ToHashSet();
            foreach (var id in draftLimits.Keys.Where(id => !reportedIds.Contains(id)))
            {
                var row = WindowRow(id, LimitControl.Remove);
                row.Detail.Text = UnmappedLimit(id);
                row.Detail.Style = Ui.Style("MutedTextStyle");
            }
            foreach (var id in windows.Keys.Where(id => !reportedIds.Contains(id) && !draftLimits.ContainsKey(id)).ToArray())
            {
                var card = windows[id].Card;
                windows.Remove(id);
                quotaWindows.Children.Remove(card);
            }
        }
    }

    string UnmappedLimit(string id) => Quota.Ceiling(draftLimits[id], page.Usage, false) + " · 当前未取得模型映射";

    void ClearQuotaWindows()
    {
        windows.Clear();
        quotaWindows.Children.Clear();
    }

    // Keep the current input whenever the window updates without changing its kind.
    QuotaRow WindowRow(string id, LimitControl control)
    {
        if (windows.TryGetValue(id, out var previous) && previous.Control == control)
        {
            return previous;
        }
        FrameworkElement? action = null;
        if (control == LimitControl.Edit)
        {
            var remaining = page.Usage == UsageDisplay.Remaining;
            var limit = new NumberBox
            {
                Minimum = remaining ? 0 : 1,
                Maximum = remaining ? 99 : 100,
                PlaceholderText = "不设上限",
                SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
                MinWidth = 140,
                Header = remaining ? "至少保留（%）" : "已用上限（%）",
                Value = draftLimits.TryGetValue(id, out var kept)
                    ? remaining ? 100 - kept : kept
                    : double.NaN,
            };
            limit.ValueChanged += (_, _) =>
            {
                if (!windows.TryGetValue(id, out var current) || !ReferenceEquals(current.Action, limit))
                {
                    return;
                }
                if (double.IsNaN(limit.Value))
                {
                    draftLimits.Remove(id);
                }
                else
                {
                    var used = remaining ? 100 - limit.Value : limit.Value;
                    draftLimits[id] = (byte)Math.Clamp(Math.Round(used), 1, 100);
                }
                Changed();
            };
            action = limit;
        }
        else if (control == LimitControl.Remove)
        {
            var remove = new Button { Content = "移除上限" };
            remove.Click += (_, _) =>
            {
                draftLimits.Remove(id);
                Changed();
                ShowQuotaWindows(CanReadQuota ? lastQuota : null);
            };
            action = remove;
        }
        var detail = Ui.Muted("");
        var lines = new StackPanel { Spacing = 2 };
        lines.Children.Add(Ui.Text(UsesAntigravity ? Quota.AntigravityName(id) : Quota.Name(id)));
        lines.Children.Add(detail);
        var row = new QuotaRow(Ui.Card(lines, action), detail, control, action);
        windows[id] = row;
        if (previous != null)
        {
            var index = quotaWindows.Children.IndexOf(previous.Card);
            quotaWindows.Children.RemoveAt(index);
            quotaWindows.Children.Insert(index, row.Card);
        }
        else
        {
            quotaWindows.Children.Add(row.Card);
        }
        return row;
    }

    // An absent or unscoped window keeps its saved limit until explicitly removed.
    QuotaLimit[] Limits() => [.. draftLimits.Select(pair => new QuotaLimit(pair.Key, pair.Value))];
}
