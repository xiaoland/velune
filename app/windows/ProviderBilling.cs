using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// A provider's independent, read-only organization reporting configuration.
sealed class ProviderBilling
{
    readonly VeluneApplication core;
    readonly string providerID;
    readonly BillingService service;
    readonly ToggleSwitch enabled = new() { Header = "查询组织用量与账单" };
    readonly TextBox organization = new() { Header = "组织 ID" };
    readonly ComboBox scope = new() { Header = "统计范围", HorizontalAlignment = HorizontalAlignment.Stretch };
    readonly TextBox scopeID = new();
    readonly PasswordBox key = new() { Header = "组织查询 Key", PlaceholderText = "与模型 Key 分开保存" };
    readonly CheckBox confirmed = new() { Content = "我确认查询凭据属于此组织，且模型 Key 对应所选范围" };
    readonly StackPanel fields = new() { Spacing = 12 };
    readonly StackPanel report = new() { Spacing = 8 };
    readonly TextBlock message = Ui.Muted("");
    readonly Button refresh = new() { Content = "刷新" };
    readonly DispatcherTimer timer = new();
    ProviderBillingStatus? status;
    bool loaded, active, busy;
    internal ContentDialog Dialog { get; }

    internal ProviderBilling(VeluneApplication core, string providerID, BillingService service)
    {
        this.core = core;
        this.providerID = providerID;
        this.service = service;
        var body = new StackPanel { Spacing = 16, Width = 560 };
        body.Children.Add(enabled);
        scope.Items.Add("整个组织");
        scope.Items.Add(service == BillingService.OpenAi ? "Project" : "Workspace");
        scope.Items.Add("API key");
        scope.SelectedIndex = 0;
        fields.Children.Add(organization);
        fields.Children.Add(scope);
        fields.Children.Add(scopeID);
        fields.Children.Add(key);
        fields.Children.Add(confirmed);
        fields.Children.Add(Ui.Muted(service == BillingService.OpenAi
            ? "使用有查询权限的 OpenAI Admin Key。"
            : "使用有组织查询权限的 Admin Key。普通 workspace 模型 Key 不适用；个人账户不支持 Admin API。"));
        body.Children.Add(fields);
        body.Children.Add(Ui.Muted("查询配置单独保存。关闭后删除查询凭据；报表不参与网关准入。组织总额包含此范围内所有客户端的消费，多个提供商引用同一范围时不要重复相加。"));
        body.Children.Add(message);
        body.Children.Add(refresh);
        body.Children.Add(report);
        body.Children.Add(Ui.Muted(service == BillingService.Anthropic
            ? "官方 Analytics 当前为 beta，通常延迟 4 小时，最长 24 小时；费用为折扣后、抵扣 credits 前金额。空记录也可能由范围 ID 不匹配导致。"
            : "费用来自组织 Costs；文本生成用量来自 Completions Usage，二者覆盖的服务可能不同。报表可能延迟。"));
        body.Children.Add(Ui.Muted("充值余额请在官方控制台查看；支出限额与速率上限分别展示，不推算剩余额度。"));
        Dialog = new ContentDialog {
            Title = "用量与账单", PrimaryButtonText = "保存查询配置", CloseButtonText = "完成",
            IsPrimaryButtonEnabled = false,
            Content = new ScrollViewer { Content = body, MaxHeight = 560 },
        };
        enabled.Toggled += (_, _) => Update();
        scope.SelectionChanged += (_, _) => Update();
        organization.TextChanged += (_, _) => Update();
        scopeID.TextChanged += (_, _) => Update();
        key.PasswordChanged += (_, _) => Update();
        confirmed.Checked += (_, _) => Update();
        confirmed.Unchecked += (_, _) => Update();
        refresh.Click += async (_, _) => { await Read(true); if (active) { Update(); } };
        Dialog.Loaded += async (_, _) => { active = true; await Read(false); if (active) { Update(); } };
        Dialog.Unloaded += (_, _) => { active = false; timer.Stop(); key.Password = ""; };
        timer.Tick += async (_, _) => { await Read(false); if (active) { Update(); } };
        Dialog.PrimaryButtonClick += async (_, args) =>
        {
            args.Cancel = true;
            var deferral = args.GetDeferral();
            busy = true; timer.Stop(); Update();
            var selected = scope.SelectedIndex == 0 ? (BillingScope)new BillingScope.Organization()
                : scope.SelectedIndex == 2 ? new BillingScope.ApiKey(scopeID.Text.Trim())
                : service == BillingService.OpenAi ? new BillingScope.Project(scopeID.Text.Trim()) : new BillingScope.Workspace(scopeID.Text.Trim());
            var configuration = enabled.IsOn ? new BillingConfiguration(service, organization.Text.Trim(), selected) : null;
            var edit = key.Password.Length == 0 ? (BillingAuthenticationEdit)new BillingAuthenticationEdit.Keep() : new BillingAuthenticationEdit.SetApiKey(key.Password);
            var binding = confirmed.IsChecked == true;
            try
            {
                var result = await Task.Run(() => core.SaveProviderBilling(ProvidersPage.DefaultGateway, providerID, configuration, edit, binding));
                if (!active) { return; }
                status = result; key.Password = ""; confirmed.IsChecked = false;
                Ui.Show(message, "查询配置已保存。");
                Render();
            }
            catch (Exception problem)
            {
                if (active) { Ui.Show(message, Core.Describe(problem)); }
            }
            finally
            {
                busy = false;
                if (active) { Schedule(); Update(); }
                deferral.Complete();
            }
        };
        Update();
    }

    void Update()
    {
        fields.Visibility = enabled.IsOn ? Visibility.Visible : Visibility.Collapsed;
        scopeID.Visibility = scope.SelectedIndex == 0 ? Visibility.Collapsed : Visibility.Visible;
        scopeID.Header = scope.SelectedIndex == 2 ? "API key ID（非密钥）" : service == BillingService.OpenAi ? "Project ID" : "Workspace ID";
        Dialog.IsPrimaryButtonEnabled = loaded && !busy && (!enabled.IsOn ||
            organization.Text.Trim().Length > 0 && (scope.SelectedIndex == 0 || scopeID.Text.Trim().Length > 0) &&
            (key.Password.Length > 0 || status?.Authentication.Configured == true) && confirmed.IsChecked == true);
        refresh.IsEnabled = !busy && status?.Configuration != null && status.Refreshing == false;
    }

    async Task Read(bool force)
    {
        if (busy || !active) { return; }
        busy = true; timer.Stop(); Update();
        try
        {
            var result = await Task.Run(() => core.ProviderBilling(ProvidersPage.DefaultGateway, providerID, force));
            if (!active) { return; }
            status = result;
            if (!loaded)
            {
                if (result.Configuration is { } configuration)
                {
                    enabled.IsOn = true; organization.Text = configuration.OrganizationId;
                    (scope.SelectedIndex, scopeID.Text) = configuration.Scope switch {
                        BillingScope.Project item => (1, item.Id),
                        BillingScope.Workspace item => (1, item.Id),
                        BillingScope.ApiKey item => (2, item.Id),
                        _ => (0, ""),
                    };
                }
                loaded = true;
                key.PlaceholderText = result.Authentication.Configured ? "留空保留已保存的查询 Key" : "与模型 Key 分开保存";
            }
            Render();
        }
        catch (Exception problem)
        {
            if (active) { Ui.Show(message, Core.Describe(problem)); }
        }
        finally
        {
            busy = false;
            if (active) { Schedule(); Update(); }
        }
    }
    void Schedule()
    {
        timer.Interval = TimeSpan.FromSeconds(status?.Refreshing == true ? 2 : 30);
        timer.Start();
    }
    void Render()
    {
        report.Children.Clear();
        if (status?.Configuration == null) { return; }
        report.Children.Add(Ui.Heading(status.Refreshing ? "正在查询…" : status.Stale ? "上次报表已过期" : "本月报表（UTC）"));
        if (status.Problem is { } problem) { report.Children.Add(Ui.Muted(problem)); }
        if (status.Report is not { } data) { return; }
        var range = data.Configuration.Scope switch {
            BillingScope.Project item => "Project " + item.Id,
            BillingScope.Workspace item => "Workspace " + item.Id,
            BillingScope.ApiKey item => "API key " + item.Id,
            _ => "整个组织",
        };
        report.Children.Add(Ui.Muted(data.Configuration.OrganizationId + " · " + range));
        report.Children.Add(Ui.Muted($"统计区间：{Date(data.StartingAtMs)} 至 {Date(data.EndingAtMs)}"));
        report.Children.Add(Ui.Muted("本地抓取：" + Date(data.ObservedAtMs)));
        report.Children.Add(Ui.Muted("官方数据更新：" + (data.DataRefreshedAtMs is { } at ? Date(at) : "未报告")));
        if (data.Costs is { } costs)
        {
            if (costs.Length == 0) { report.Children.Add(Ui.Muted("费用：此范围暂无记录")); }
            foreach (var cost in costs) { report.Children.Add(Ui.Text($"本月费用：{cost.Currency} {cost.Amount:N2}")); }
        }
        if (data.Usage is { } usage)
        {
            if (usage.Requests == 0 && usage.InputTokens == 0 && usage.OutputTokens == 0) { report.Children.Add(Ui.Muted("用量：此范围暂无记录")); }
            else
            {
                report.Children.Add(Ui.Text($"{(service == BillingService.OpenAi ? "文本生成请求" : "请求数")}：{usage.Requests:N0}"));
                report.Children.Add(Ui.Text($"输入 Token（含缓存）：{usage.InputTokens:N0} · 输出 Token：{usage.OutputTokens:N0}"));
                if (usage.CachedInputTokens is { } cached) { report.Children.Add(Ui.Muted($"其中缓存读取：{cached:N0}")); }
                if (usage.CacheCreationInputTokens is { } created) { report.Children.Add(Ui.Muted($"其中缓存创建：{created:N0}")); }
            }
        }
        if (data.SpendLimit is { } spend)
        {
            report.Children.Add(Ui.Text($"官方月度支出限额：{spend.Currency} {spend.Amount:N2}"));
            report.Children.Add(Ui.Muted(spend.Enforced ? "官方已启用此限制；不表示当前已耗尽。" : "官方未启用此限制。"));
        }
        if (data.RateLimits is { } limits)
        {
            var rows = new StackPanel { Spacing = 8 };
            foreach (var limit in limits)
            {
                rows.Children.Add(Ui.Text($"{limit.Name} · {limit.LimitType}：{limit.Value:N0}"));
                if (limit.ModelIds.Length > 0) { rows.Children.Add(Ui.Muted(string.Join(", ", limit.ModelIds))); }
            }
            report.Children.Add(new Expander { Header = $"配置速率上限（{limits.Length}）", Content = rows, HorizontalAlignment = HorizontalAlignment.Stretch });
        }
        foreach (var issue in data.Problems) { report.Children.Add(Ui.Muted(issue)); }
    }
    static string Date(long milliseconds) => DateTimeOffset.FromUnixTimeMilliseconds(milliseconds).ToString("yyyy-MM-dd HH:mm 'UTC'");
}
