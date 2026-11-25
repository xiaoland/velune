using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// ProviderEditor edits one provider in the settings page. Its sections show
// what the provider's composition needs: the connection, its sign-in, its
// models, and its quota and usage limits. A new provider starts from a
// template; a subscription's protocol and address are its own.
sealed partial class ProviderEditor : UserControl
{
    readonly ProvidersPage page;
    // The saved provider; null until a new one is first saved.
    ProviderSummary? existing;
    readonly ProviderTemplate? template;
    readonly Protocol protocolID;
    readonly RequestProfile profile;
    readonly ProviderAuthenticationSource authenticationSource;
    // The account quota source follows its switch; the others, the template.
    ProviderQuotaSource quotaSource;
    readonly bool subscription;

    readonly StackPanel body = new() { Spacing = 4, MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Stretch };
    readonly BreadcrumbBar trail = new();
    readonly TextBox name = new() { PlaceholderText = "例如 DeepSeek", MinWidth = 280 };
    readonly ComboBox protocol = new() { MinWidth = 280 };
    readonly TextBox endpoint = new() { PlaceholderText = "例如 https://api.example.com/v1", MinWidth = 280 };
    readonly TextBox binary = new() { MinWidth = 280 };
    readonly TextBox codexClientVersion = new() { MinWidth = 280 };
    readonly TextBlock nameError = Ui.Error(), endpointError = Ui.Error(), binaryError = Ui.Error(), modelsError = Ui.Error();
    readonly Button save = new() { Content = "保存", Style = Ui.Style("AccentButtonStyle") };
    readonly Button cancel = new() { Content = "取消" };
    // Set while the editor fills its fields, which changes nothing.
    bool building = true;
    bool dirty;

    ExecutionBackend? ConnectionBackend => existing?.ExecutionBackend ?? template?.ExecutionBackend;
    bool UsesCli => ConnectionBackend is ExecutionBackend.ClaudeCli or ExecutionBackend.AntigravityCli;
    bool UsesAntigravity => ConnectionBackend is ExecutionBackend.AntigravityCli;

    internal ProviderEditor(ProvidersPage page, ProviderSummary? existing, ProviderTemplate? template)
    {
        this.page = page;
        this.existing = existing;
        this.template = template;
        protocolID = existing?.Protocol ?? template?.Protocol ?? Protocol.ChatCompletions;
        profile = existing?.RequestProfile ?? template?.RequestProfile ?? RequestProfile.Native;
        authenticationSource = existing?.AuthenticationSource ?? template?.AuthenticationSource ?? ProviderAuthenticationSource.ApiKey;
        quotaSource = existing?.QuotaSource ?? template?.QuotaSource ?? ProviderQuotaSource.None;
        subscription = VeluneBindingsMethods.ProviderSubscriptionTemplate(
            ConnectionBackend ?? new ExecutionBackend.Http(),
            profile, authenticationSource, quotaSource) != null;

        trail.ItemClicked += (_, e) =>
        {
            if (e.Index == 0)
            {
                Leave();
            }
        };
        body.Children.Add(trail);
        if ((existing?.Authentication.Disclosure ?? template?.Disclosure) is { } disclosure)
        {
            body.Children.Add(new InfoBar { Severity = InfoBarSeverity.Warning, IsOpen = true, IsClosable = false, Title = disclosure.Title, Message = disclosure.Detail });
        }
        BuildConnection();
        BuildAuthentication();
        BuildAccountQuota();
        BuildBilling();
        BuildModels();
        BuildQuota();

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        actions.Children.Add(cancel);
        actions.Children.Add(save);
        save.Click += async (_, _) =>
        {
            if (await Save())
            {
                page.Close();
            }
        };
        cancel.Click += (_, _) => Leave();
        var footer = new Border { Padding = new Thickness(24, 12, 24, 16), Child = actions };

        var layout = new Grid();
        layout.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var scroller = new ScrollViewer { Content = new Border { Padding = new Thickness(24, 12, 24, 20), Child = body } };
        layout.Children.Add(scroller);
        Grid.SetRow(footer, 1);
        layout.Children.Add(footer);
        Content = layout;
        ShowTrail();
        Loaded += (_, _) => StartQuota();
        Unloaded += (_, _) => StopQuota();
        building = false;
    }

    // Changed marks that leaving would lose an edit.
    void Changed()
    {
        if (!building)
        {
            dirty = true;
        }
    }

    void ShowTrail()
    {
        var shown = name.Text.Trim();
        trail.ItemsSource = new[] { "AI提供商", shown.Length > 0 ? shown : existing == null ? "新提供商" : existing.Name };
    }

    async void Leave()
    {
        if (dirty && !await Ui.Confirm(page.Window, "放弃未保存的更改？", "此提供商的修改尚未保存。", "放弃"))
        {
            return;
        }
        page.Close();
    }

    // Section starts a titled group of settings cards.
    void Section(string title) => body.Children.Add(Ui.Heading(title));

    void BuildConnection()
    {
        Section("连接");
        name.Text = existing?.Name ?? template?.Name ?? "";
        name.TextChanged += (_, _) =>
        {
            Changed();
            ShowTrail();
        };
        body.Children.Add(Ui.Card(Ui.Field(Ui.Text("名称"), "", nameError), name));

        foreach (var descriptor in page.Protocols)
        {
            protocol.Items.Add(descriptor.Name);
        }
        protocol.SelectedIndex = Array.FindIndex(page.Protocols, descriptor => descriptor.Id == protocolID);
        protocol.SelectionChanged += (_, _) => Changed();
        endpoint.Text = existing?.Endpoint ?? template?.Endpoint ?? "";
        endpoint.TextChanged += (_, _) => Changed();
        if (subscription)
        {
            // A subscription connects as its template does.
            body.Children.Add(Ui.Card(Ui.TwoLines("协议", "由订阅决定。"), Ui.Text(page.ProtocolName(protocolID))));
            if (!UsesCli)
            {
                body.Children.Add(Ui.Card(Ui.TwoLines("接口地址", "由订阅决定。"), Ui.Text(endpoint.Text)));
            }
        }
        else
        {
            body.Children.Add(Ui.Card(Ui.TwoLines("协议", "提供商 API 使用的协议；与运行时协议不同时由网关转换。", true), protocol));
            body.Children.Add(Ui.Card(Ui.Field(Ui.Text("接口地址"), "", endpointError), endpoint));
        }
        if (UsesCli)
        {
            BuildBinary();
        }
        if (profile == RequestProfile.CodexResponses)
        {
            const string label = "Codex 客户端版本";
            codexClientVersion.Text = existing?.CodexClientVersion ?? "";
            codexClientVersion.PlaceholderText = $"自动（{VeluneBindingsMethods.DefaultCodexClientVersion()}）";
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(codexClientVersion, label);
            codexClientVersion.TextChanged += (_, _) => Changed();
            body.Children.Add(Ui.Card(Ui.TwoLines(label, "留空时采用适配器支持的最新版本。"), codexClientVersion));
        }
    }

    // A CLI executes the provider's requests in place of an HTTP service address.
    void BuildBinary()
    {
        var previousSshTarget = (ConnectionBackend as ExecutionBackend.AntigravityCli)?.SshDestination;
        if (previousSshTarget != null)
        {
            body.Children.Add(Ui.Card(Ui.TwoLines("原执行目标", $"此提供商配置了 SSH {previousSshTarget}。在 Windows 上，请选择本机 agy 并保存。", true)));
        }
        binary.Text = ConnectionBackend switch
        {
            ExecutionBackend.ClaudeCli { Binary: var saved } => saved,
            ExecutionBackend.AntigravityCli { Binary: var saved, SshDestination: null } => saved,
            _ => "",
        };
        var title = UsesAntigravity ? "Antigravity CLI 程序" : "Claude Code 程序";
        var command = template?.ExecutableCommand ?? (UsesAntigravity ? "agy" : "claude");
        binary.PlaceholderText = $"{command} 可执行文件的绝对路径";
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(binary, title);
        var found = Discovery.Installed(UsesAntigravity ? "antigravity" : "claude-code", command);
        if (previousSshTarget == null && binary.Text.Length == 0 && found.Count > 0)
        {
            binary.Text = found[0];
        }
        binary.TextChanged += (_, _) =>
        {
            Changed();
            if (UsesAntigravity) StartQuota();
        };
        var choose = new MenuFlyout();
        foreach (var path in found)
        {
            choose.Items.Add(Ui.MenuItem(path, () => binary.Text = path));
        }
        if (found.Count == 0)
        {
            choose.Items.Add(new MenuFlyoutItem { Text = "没有在 PATH 中发现", IsEnabled = false });
        }
        var discovered = new DropDownButton { Content = "已发现", Flyout = choose };
        var browse = new Button { Content = "选择…" };
        browse.Click += async (_, _) =>
        {
            if (await Native.PickFile(page.Window) is { } path)
            {
                binary.Text = path;
            }
        };
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        row.Children.Add(binary);
        row.Children.Add(discovered);
        row.Children.Add(browse);
        var detail = UsesAntigravity ? "填写本机 agy 程序的绝对路径。" : "由本机官方 CLI 登录与执行请求；须为绝对路径。";
        body.Children.Add(Ui.Card(Ui.Field(Ui.TwoLines(title, detail, true), "", binaryError), row));
    }

    // Draft is the provider the fields describe.
    ProviderDraft Draft()
    {
        var version = codexClientVersion.Text.Trim();
        return new(
            Id: existing?.Id ?? "",
            Name: name.Text.Trim(),
            Protocol: subscription || protocol.SelectedIndex < 0 ? protocolID : page.Protocols[protocol.SelectedIndex].Id,
            Endpoint: UsesCli ? "" : endpoint.Text.Trim(),
            ExecutionBackend: ConnectionBackend switch
            {
                ExecutionBackend.AntigravityCli => new ExecutionBackend.AntigravityCli(binary.Text.Trim(), null),
                ExecutionBackend.ClaudeCli => new ExecutionBackend.ClaudeCli(binary.Text.Trim()),
                _ => new ExecutionBackend.Http(),
            },
            RequestProfile: profile,
            AuthenticationSource: authenticationSource,
            // The account source reads only from an official address; another keeps none.
            QuotaSource: quotaSource == ProviderQuotaSource.ApiAccount && AccountService() == null ? ProviderQuotaSource.None : quotaSource,
            Models: [.. models.Select(model => model.Value())],
            QuotaLimits: Limits(),
            CodexClientVersion: profile == RequestProfile.CodexResponses && version.Length > 0 ? version : null);
    }

    // Check shows what is missing; false keeps the editor as it is.
    bool Check(ProviderDraft draft)
    {
        Ui.Show(nameError, draft.Name.Length == 0 ? "请填写名称" : "");
        var endpointMissing = !UsesCli && draft.Endpoint.Length == 0;
        var path = binary.Text.Trim();
        var binaryMissing = UsesCli && (!Path.IsPathFullyQualified(path) || (UsesAntigravity && path.Any(char.IsControl)));
        Ui.Show(endpointError, endpointMissing ? "请填写接口地址" : "");
        Ui.Show(binaryError, binaryMissing ? (UsesAntigravity ? "请填写 agy 程序的绝对路径" : "请选择 Claude Code 程序的绝对路径") : "");
        var missingID = models.Any(model => model.ID.Length == 0);
        var invalidTokens = models.Any(model => !model.ValidTokens);
        Ui.Show(modelsError, draft.Models.Length == 0 ? "请添加至少一个模型" : missingID ? "每个模型都需要模型 ID" : invalidTokens ? TokenCount.InputError : "");
        var keyMissing = authenticationSource == ProviderAuthenticationSource.ApiKey && key.Password.Trim().Length == 0 && !(existing?.Authentication.Configured ?? false);
        Ui.Show(keyError, keyMissing ? "请填写 API key" : "");
        var incomplete = QuotaIncomplete();
        return draft.Name.Length > 0 && !endpointMissing && !binaryMissing && draft.Models.Length > 0 && !missingID && !invalidTokens && !keyMissing && !incomplete;
    }

    // Save saves the fields together with any sign-in and usage query edit,
    // and keeps the editor on the saved provider; false when it did not save.
    async Task<bool> Save()
    {
        var draft = Draft();
        if (!Check(draft) || page.Busy)
        {
            return false;
        }
        var authentication = AuthenticationEdit();
        var quotaEdit = quotaSource == ProviderQuotaSource.ArkCodingUsage ? QuotaEdit() : null;
        page.Busy = true;
        save.IsEnabled = false;
        try
        {
            // A new provider is the one the save adds.
            var previous = existing == null
                ? (await Task.Run(() => page.Core.Local.List())).Gateways.FirstOrDefault(gateway => gateway.Id == ProvidersPage.DefaultGateway)?.Providers.Select(provider => provider.Id).ToHashSet() ?? []
                : [];
            var update = await Task.Run(() => page.Core.Local.SaveProvider(ProvidersPage.DefaultGateway, draft, authentication, quotaEdit));
            var saved = page.SetGateway(update.Gateways);
            var previousBackend = existing?.ExecutionBackend;
            existing = existing == null
                ? saved.FirstOrDefault(provider => !previous.Contains(provider.Id))
                : saved.FirstOrDefault(provider => provider.Id == existing.Id);
            if (UsesAntigravity && previousBackend != existing?.ExecutionBackend)
            {
                lastQuota = null;
                ClearQuotaWindows();
            }
            dirty = false;
            Refresh();
            return existing != null;
        }
        catch (Exception problem)
        {
            Problems.Report("保存提供商「" + (existing?.Name ?? "新提供商") + "」", problem);
            return false;
        }
        finally
        {
            page.Busy = false;
            save.IsEnabled = true;
        }
    }

    // Refresh shows the saved provider's sign-in, models and quota after it changed.
    void Refresh()
    {
        if (existing == null)
        {
            return;
        }
        building = true;
        key.Password = "";
        ShowAuthentication();
        foreach (var model in models)
        {
            model.Keep(existing.Models);
        }
        ShowQuotaAuthentication();
        building = false;
        StartQuota();
    }

    // Saved replaces the provider the editor shows with what the core keeps now.
    void Saved(GatewaySummary[] gateways)
    {
        var saved = page.SetGateway(gateways);
        if (existing != null)
        {
            existing = saved.FirstOrDefault(provider => provider.Id == existing.Id) ?? existing;
            Refresh();
        }
    }
}
