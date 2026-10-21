using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// RouteEditor edits one model route in the settings page: its name, strategy,
// session stickiness and ordered members. Each member shows its provider's
// sign-in and the quota windows that apply to its model, and says when the
// route will not use it.
sealed class RouteEditor : UserControl
{
    static readonly BindingRoutingStrategy[] strategies = [BindingRoutingStrategy.Smart, BindingRoutingStrategy.Order, BindingRoutingStrategy.Rotate, BindingRoutingStrategy.Weight];

    readonly RoutesPage page;
    readonly string recordKey;
    readonly StackPanel body = new() { Spacing = 4, MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Stretch };
    readonly InfoBar failure = Ui.FailureBar();
    readonly BreadcrumbBar trail = new();
    readonly StackPanel disclosures = new() { Spacing = 4 };
    readonly TextBox name = new() { MinWidth = 280, PlaceholderText = "例如 Codex 双订阅" };
    readonly TextBlock nameError = Ui.Error(), membersError = Ui.Error();
    readonly ComboBox strategy = new() { MinWidth = 160 };
    readonly TextBlock strategyDetail = Ui.Muted("");
    readonly ToggleSwitch sticky = new() { OnContent = "开", OffContent = "关" };
    readonly StackPanel memberCards = new() { Spacing = 4 };
    readonly DropDownButton add = new() { Content = "添加成员" };
    readonly Button save = new() { Content = "保存", Style = Ui.Style("AccentButtonStyle") };
    readonly List<string> members;
    readonly Dictionary<string, uint> weights;
    readonly Dictionary<string, BindingProviderQuotaStatus> quotas = [];
    readonly List<MemberCard> cards = [];
    readonly DispatcherTimer timer = new();
    bool reading;
    bool dirty;

    internal RouteEditor(RoutesPage page, BindingModelRoute? route)
    {
        this.page = page;
        recordKey = route?.RecordKey ?? "";
        members = [.. route?.Members ?? []];
        weights = new Dictionary<string, uint>(route?.Weights ?? []);

        trail.ItemClicked += (_, e) =>
        {
            if (e.Index == 0)
            {
                Leave();
            }
        };
        body.Children.Add(trail);
        body.Children.Add(failure);
        body.Children.Add(disclosures);

        body.Children.Add(Ui.Heading("路由"));
        name.Text = route?.Name ?? "";
        name.TextChanged += (_, _) =>
        {
            dirty = true;
            ShowTrail();
        };
        body.Children.Add(Ui.Card(Ui.Field(Ui.TwoLines("名称", "与提供商模型一起出现在会话的模型选择中。", true), "", nameError), name));
        foreach (var option in strategies)
        {
            strategy.Items.Add(RoutesPage.StrategyName(option));
        }
        // A new route picks smartly and keeps conversations on their member.
        strategy.SelectedIndex = Array.IndexOf(strategies, route?.Strategy ?? BindingRoutingStrategy.Smart);
        strategy.SelectionChanged += (_, _) =>
        {
            dirty = true;
            ShowStrategy();
            ShowMembers();
        };
        var strategyText = new StackPanel { Spacing = 2 };
        strategyText.Children.Add(Ui.Text("策略"));
        strategyText.Children.Add(strategyDetail);
        body.Children.Add(Ui.Card(strategyText, strategy));
        sticky.IsOn = route?.Sticky ?? true;
        sticky.Toggled += (_, _) => dirty = true;
        body.Children.Add(Ui.Card(Ui.TwoLines("会话保持", "同一会话继续使用上次成功的成员，不越过健康检查与使用上限；只在应用运行期间保留。", true), sticky));

        var heading = new Grid();
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        heading.Children.Add(Ui.Heading("成员"));
        add.Flyout = new MenuFlyout();
        add.Flyout.Opening += (_, _) => FillAddMenu((MenuFlyout)add.Flyout);
        add.VerticalAlignment = VerticalAlignment.Bottom;
        add.Margin = new Thickness(0, 0, 0, 4);
        Grid.SetColumn(add, 1);
        heading.Children.Add(add);
        body.Children.Add(heading);
        body.Children.Add(Ui.Muted("按优先顺序排列。当前成员在开始回应前失败且属于可切换的失败时，改用下一个；直接选择提供商模型的请求不切换。聚合多个订阅不合并额度，每个成员仍受其账户规则约束。"));
        body.Children.Add(membersError);
        body.Children.Add(memberCards);

        var cancel = new Button { Content = "取消" };
        cancel.Click += (_, _) => Leave();
        save.Click += (_, _) => Save();
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        actions.Children.Add(cancel);
        actions.Children.Add(save);
        var layout = new Grid();
        layout.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        layout.Children.Add(new ScrollViewer { Content = new Border { Padding = new Thickness(24, 12, 24, 20), Child = body } });
        var footer = new Border { Padding = new Thickness(24, 12, 24, 16), Child = actions };
        Grid.SetRow(footer, 1);
        layout.Children.Add(footer);
        Content = layout;

        ShowTrail();
        ShowStrategy();
        ShowMembers();
        timer.Tick += (_, _) => ReadQuotas();
        Loaded += (_, _) => ReadQuotas();
        Unloaded += (_, _) => timer.Stop();
    }

    void ShowTrail()
    {
        var shown = name.Text.Trim();
        trail.ItemsSource = new[] { "模型路由", shown.Length > 0 ? shown : recordKey.Length == 0 ? "新路由" : "路由" };
    }

    BindingRoutingStrategy Strategy => strategy.SelectedIndex >= 0 ? strategies[strategy.SelectedIndex] : BindingRoutingStrategy.Smart;

    void ShowStrategy() => strategyDetail.Text = Strategy switch
    {
        BindingRoutingStrategy.Smart => "需要重新选择时，按各成员最紧额度窗口距使用上限的余量排序。",
        BindingRoutingStrategy.Order => "总是先用排在前面的可用成员。",
        BindingRoutingStrategy.Rotate => "新会话依次轮流分配到各成员。",
        _ => "按权重比例分配新会话。",
    };

    // ProvidersChanged shows members again after the providers were read anew.
    internal void ProvidersChanged() => ShowMembers();

    // ShowMembers lays the member cards out again after the members changed.
    void ShowMembers()
    {
        cards.Clear();
        memberCards.Children.Clear();
        for (var i = 0; i < members.Count; i++)
        {
            var card = new MemberCard(this, members[i], i);
            cards.Add(card);
            memberCards.Children.Add(card.Element);
        }
        if (members.Count == 0)
        {
            memberCards.Children.Add(Ui.Muted("还没有成员。用“添加成员”从提供商的模型中选择。"));
        }
        ShowDisclosures();
    }

    // ShowDisclosures shows what the member subscriptions say about using them.
    void ShowDisclosures()
    {
        disclosures.Children.Clear();
        var seen = new HashSet<string>();
        foreach (var key in members)
        {
            if (page.Member(key)?.Provider.Authentication.Disclosure is { } disclosure && seen.Add(disclosure.Title))
            {
                disclosures.Children.Add(new InfoBar { Severity = InfoBarSeverity.Warning, IsOpen = true, IsClosable = false, Title = disclosure.Title, Message = disclosure.Detail });
            }
        }
    }

    // Protocol is the protocol members share, which the first one sets.
    BindingGatewayProtocol? Protocol => members.Select(page.Member).FirstOrDefault(member => member != null)?.Provider.Protocol;

    void FillAddMenu(MenuFlyout menu)
    {
        menu.Items.Clear();
        var protocol = Protocol;
        foreach (var provider in page.Providers.Where(provider => provider.Models.Length > 0))
        {
            var group = new MenuFlyoutSubItem { Text = provider.Name };
            var other = protocol != null && provider.Protocol != protocol;
            foreach (var model in provider.Models)
            {
                var item = new MenuFlyoutItem
                {
                    Text = model.Nickname == model.ProviderModelId ? model.ProviderModelId : $"{model.Nickname}（{model.ProviderModelId}）",
                    IsEnabled = !other && !members.Contains(model.RecordKey),
                };
                var key = model.RecordKey;
                item.Click += (_, _) =>
                {
                    members.Add(key);
                    dirty = true;
                    ShowMembers();
                    ReadQuotas();
                };
                group.Items.Add(item);
            }
            if (other)
            {
                group.Text += "（其他协议）";
            }
            menu.Items.Add(group);
        }
        if (menu.Items.Count == 0)
        {
            menu.Items.Add(new MenuFlyoutItem { Text = "还没有提供商模型", IsEnabled = false });
        }
    }

    void Move(int index, int by)
    {
        var target = index + by;
        if (target < 0 || target >= members.Count)
        {
            return;
        }
        (members[index], members[target]) = (members[target], members[index]);
        dirty = true;
        ShowMembers();
    }

    void Remove(int index)
    {
        weights.Remove(members[index]);
        members.RemoveAt(index);
        dirty = true;
        ShowMembers();
    }

    uint Weight(string key) => weights.TryGetValue(key, out var weight) && weight > 0 ? weight : 1;

    void SetWeight(string key, uint weight)
    {
        weights[key] = weight;
        dirty = true;
        foreach (var card in cards)
        {
            card.ShowShare();
        }
    }

    // Share is a member's part of the weights, as a percentage.
    double Share(string key) => 100.0 * Weight(key) / members.Sum(member => (double)Weight(member));

    // ReadQuotas reads the core's quota observation for the members' signed-in
    // providers: soon while one reads or has none, then every half minute.
    async void ReadQuotas()
    {
        if (reading || !IsLoaded)
        {
            return;
        }
        reading = true;
        timer.Stop();
        var soon = false;
        var providers = members.Select(page.Member).Where(member => member != null).Select(member => member!.Value.Provider)
            .Where(provider => provider.QuotaSource != BindingProviderQuotaSource.None && provider.Authentication.Configured)
            .DistinctBy(provider => provider.Id).ToList();
        foreach (var provider in providers)
        {
            try
            {
                var status = await Task.Run(() => page.Core.Local.ProviderQuota(ProvidersPage.DefaultGateway, provider.Id));
                quotas[provider.Id] = status;
                soon |= status.Refreshing || status.Windows.Length == 0 && status.Freshness == BindingQuotaFreshness.Unknown;
            }
            catch (Exception problem)
            {
                quotas.Remove(provider.Id);
                Ui.Show(failure, Core.Describe(problem));
            }
        }
        foreach (var card in cards)
        {
            card.ShowStatus();
        }
        reading = false;
        if (IsLoaded && providers.Count > 0)
        {
            timer.Interval = TimeSpan.FromSeconds(soon ? 2 : 30);
            timer.Start();
        }
    }

    async void Login(BindingProviderDefinition provider)
    {
        try
        {
            page.SetGateway(await ProviderFlows.Login(page.Core, page.Window, provider.Id, provider.Name));
            ReadQuotas();
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    async void Leave()
    {
        if (dirty && !await Ui.Confirm(page.Window, "放弃未保存的更改？", "此路由的修改尚未保存。", "放弃"))
        {
            return;
        }
        page.Close();
    }

    async void Save()
    {
        var shownName = name.Text.Trim();
        Ui.Show(nameError, shownName.Length == 0 ? "请填写名称" : "");
        Ui.Show(membersError, members.Count == 0 ? "请添加至少一个成员" : "");
        if (shownName.Length == 0 || members.Count == 0)
        {
            return;
        }
        // Weights belong to weighted routes, one for every member.
        var shownWeights = Strategy == BindingRoutingStrategy.Weight
            ? members.ToDictionary(key => key, Weight)
            : [];
        var route = new BindingModelRoute(recordKey, shownName, [.. members], Strategy, shownWeights, sticky.IsOn);
        save.IsEnabled = false;
        try
        {
            page.SetGateway((await Task.Run(() => page.Core.Local.SaveModelRoute(ProvidersPage.DefaultGateway, route))).Gateways);
            dirty = false;
            page.Close();
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
        save.IsEnabled = true;
    }

    // MemberCard is one member: its provider and model, its place and weight,
    // and whether the route can use it now.
    sealed class MemberCard
    {
        readonly RouteEditor editor;
        readonly string key;
        public Border Element { get; }
        readonly StackPanel status = new() { Spacing = 2 };
        readonly TextBlock share = Ui.Muted("");

        public MemberCard(RouteEditor editor, string key, int index)
        {
            this.editor = editor;
            this.key = key;
            var member = editor.page.Member(key);
            var lines = new StackPanel { Spacing = 2 };
            lines.Children.Add(Ui.Text(member is { } found
                ? $"{index + 1}. {found.Provider.Name} · {found.Model.Nickname}"
                : $"{index + 1}. 已不存在的模型"));
            if (member is { } named && named.Model.Nickname != named.Model.ProviderModelId)
            {
                lines.Children.Add(Ui.Muted(named.Model.ProviderModelId));
            }
            lines.Children.Add(status);

            var trailing = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Top };
            if (editor.Strategy == BindingRoutingStrategy.Weight)
            {
                var weight = new NumberBox { Minimum = 1, Maximum = 1000, Value = editor.Weight(key), SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact, MinWidth = 96 };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(weight, "权重");
                weight.ValueChanged += (_, _) =>
                {
                    if (!double.IsNaN(weight.Value))
                    {
                        editor.SetWeight(key, (uint)Math.Clamp(weight.Value, 1, 1000));
                    }
                };
                var weighting = new StackPanel { Spacing = 2 };
                weighting.Children.Add(weight);
                weighting.Children.Add(share);
                trailing.Children.Add(weighting);
            }
            var up = Ui.IconButton("\uE70E", "上移");
            up.IsEnabled = index > 0;
            up.Click += (_, _) => editor.Move(index, -1);
            var down = Ui.IconButton("\uE70D", "下移");
            down.IsEnabled = index < editor.members.Count - 1;
            down.Click += (_, _) => editor.Move(index, 1);
            var remove = Ui.IconButton("\uE711", "移除成员");
            remove.Click += (_, _) => editor.Remove(index);
            trailing.Children.Add(up);
            trailing.Children.Add(down);
            trailing.Children.Add(remove);

            var row = new Grid { ColumnSpacing = 16 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(lines);
            Grid.SetColumn(trailing, 1);
            row.Children.Add(trailing);
            Element = new Border { Padding = new Thickness(16, 12, 16, 12), CornerRadius = new CornerRadius(4), Style = Ui.Style("CardStyle"), Child = row };
            var menu = new List<MenuFlyoutItemBase>
            {
                Ui.MenuItem("上移", () => editor.Move(index, -1)),
                Ui.MenuItem("下移", () => editor.Move(index, 1)),
                Ui.MenuItem("移除", () => editor.Remove(index)),
            };
            if (member is { } owned)
            {
                var owner = owned.Provider.Id;
                menu.Add(new MenuFlyoutSeparator());
                menu.Add(Ui.MenuItem("编辑提供商…", () => editor.page.EditProvider(owner)));
                Element.DoubleTapped += (_, _) => editor.page.EditProvider(owner);
            }
            Element.ContextFlyout = Ui.Menu([.. menu]);
            ShowShare();
            ShowStatus();
        }

        public void ShowShare() => share.Text = $"占 {editor.Share(key):0.#}%";

        // ShowStatus shows the provider's sign-in, the quota windows that
        // apply to this model, and why the route would skip it now.
        public void ShowStatus()
        {
            status.Children.Clear();
            if (editor.page.Member(key) is not { } member)
            {
                status.Children.Add(Ui.Muted("此模型已从提供商中删除；保存前请移除。"));
                return;
            }
            var (provider, model) = member;
            var authentication = provider.Authentication;
            var reasons = new List<string>();
            if (provider.AuthenticationSource != BindingProviderAuthenticationSource.None)
            {
                var account = authentication.AccountLabel is { Length: > 0 } label ? "：" + label : "";
                var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                line.Children.Add(Ui.Muted(authentication.Configured
                    ? (authentication.Method == BindingAuthenticationMethod.OAuth ? "已登录" : "已配置认证") + account
                    : authentication.Method == BindingAuthenticationMethod.OAuth ? "未登录" : "未配置认证"));
                if (!authentication.Configured && authentication.Actions.Any(action => action.Id == "login"))
                {
                    var login = new HyperlinkButton { Content = "登录…", Padding = new Thickness(0) };
                    login.Click += (_, _) => editor.Login(provider);
                    line.Children.Add(login);
                }
                status.Children.Add(line);
                if (!authentication.Configured)
                {
                    reasons.Add(authentication.Method == BindingAuthenticationMethod.OAuth ? "登录前路由不会使用它。" : "配置认证前路由不会使用它。");
                }
            }
            var limited = provider.QuotaLimits.Length > 0;
            if (editor.quotas.TryGetValue(provider.Id, out var quota))
            {
                foreach (var window in quota.Windows.Where(window => Quota.Applies(window, model.ProviderModelId)))
                {
                    var limit = provider.QuotaLimits.FirstOrDefault(saved => saved.WindowId == window.Id);
                    status.Children.Add(Ui.Muted(Quota.Name(window.Id) + "：" + Quota.Detail(window, quota.Freshness, editor.page.Usage, limit)));
                }
                if (quota.Available == false)
                {
                    reasons.Add("已达到使用上限或暂停，路由暂不使用它。");
                }
                else if (limited && quota.Freshness != BindingQuotaFreshness.Fresh)
                {
                    reasons.Add("设有使用上限而缺少最新额度，路由暂不使用它。");
                }
            }
            else if (limited && authentication.Configured)
            {
                reasons.Add("设有使用上限而尚无额度观测，路由暂不使用它。");
            }
            if (limited && provider.QuotaSource == BindingProviderQuotaSource.ArkCodingUsage
                && provider.QuotaAuthentication is not { Configured: true, BindingConfirmed: true })
            {
                reasons.Add("额度查询认证未确认关联，设有上限时暂停派发。");
            }
            foreach (var reason in reasons.Distinct())
            {
                var warning = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
                warning.Children.Add(new FontIcon { Glyph = "\uE7BA", FontSize = 12, Style = Ui.Style("CautionIconStyle") });
                warning.Children.Add(Ui.Text(reason, "CaptionTextBlockStyle"));
                status.Children.Add(warning);
            }
        }
    }
}
