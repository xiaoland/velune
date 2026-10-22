using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// ProviderEditor edits one provider in the settings page. Its sections show
// what the provider's composition needs: the connection, its sign-in, its
// models, and its quota and usage limits. A new provider starts from a
// template; a subscription's protocol and address are its own.
sealed partial class ProviderEditor : UserControl
{
    readonly ProvidersPage page;
    // The saved provider; null until a new one is first saved.
    BindingProviderDefinition? existing;
    readonly BindingProviderTemplate? template;
    readonly BindingGatewayProtocol protocolID;
    readonly BindingRequestProfile profile;
    readonly BindingProviderAuthenticationSource authenticationSource;
    readonly BindingProviderQuotaSource quotaSource;
    readonly bool subscription;

    readonly StackPanel body = new() { Spacing = 4, MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Stretch };
    readonly BreadcrumbBar trail = new();
    readonly TextBox name = new() { PlaceholderText = "例如 DeepSeek", MinWidth = 280 };
    readonly ComboBox protocol = new() { MinWidth = 280 };
    readonly TextBox endpoint = new() { PlaceholderText = "例如 https://api.example.com/v1", MinWidth = 280 };
    readonly TextBox binary = new() { PlaceholderText = "claude 可执行文件的绝对路径", MinWidth = 280 };
    readonly TextBlock nameError = Ui.Error(), endpointError = Ui.Error(), binaryError = Ui.Error(), modelsError = Ui.Error();
    readonly Button save = new() { Content = "保存", Style = Ui.Style("AccentButtonStyle") };
    readonly Button cancel = new() { Content = "取消" };
    // Set while the editor fills its fields, which changes nothing.
    bool building = true;
    bool dirty;

    internal ProviderEditor(ProvidersPage page, BindingProviderDefinition? existing, BindingProviderTemplate? template)
    {
        this.page = page;
        this.existing = existing;
        this.template = template;
        protocolID = existing?.Protocol ?? template?.Protocol ?? BindingGatewayProtocol.ChatCompletionsV1;
        profile = existing?.RequestProfile ?? template?.RequestProfile ?? BindingRequestProfile.Native;
        authenticationSource = existing?.AuthenticationSource ?? template?.AuthenticationSource ?? BindingProviderAuthenticationSource.ApiKey;
        quotaSource = existing?.QuotaSource ?? template?.QuotaSource ?? BindingProviderQuotaSource.None;
        subscription = template?.Kind == BindingProviderTemplateKind.Subscription || page.IsSubscription(profile, authenticationSource, quotaSource);

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
        var cli = (existing?.ExecutionBackend ?? template?.ExecutionBackend) is BindingExecutionBackend.ClaudeCli;
        if (subscription)
        {
            // A subscription connects as its template does.
            body.Children.Add(Ui.Card(Ui.TwoLines("协议", "由订阅决定。"), Ui.Text(page.ProtocolName(protocolID))));
            if (!cli)
            {
                body.Children.Add(Ui.Card(Ui.TwoLines("接口地址", "由订阅决定。"), Ui.Text(endpoint.Text)));
            }
        }
        else
        {
            body.Children.Add(Ui.Card(Ui.TwoLines("协议", "提供商 API 使用的协议；与运行时协议不同时由网关转换。", true), protocol));
            body.Children.Add(Ui.Card(Ui.Field(Ui.Text("接口地址"), "", endpointError), endpoint));
        }
        if (cli)
        {
            BuildBinary();
        }
    }

    // BuildBinary lays out the local Claude Code CLI that runs the
    // provider's requests in place of a service address.
    void BuildBinary()
    {
        binary.Text = (existing?.ExecutionBackend ?? template?.ExecutionBackend) is BindingExecutionBackend.ClaudeCli { Binary: var saved } ? saved : "";
        var found = Discovery.Installed("claude-code", template?.ExecutableCommand ?? "claude");
        if (binary.Text.Length == 0 && found.Count > 0)
        {
            binary.Text = found[0];
        }
        binary.TextChanged += (_, _) => Changed();
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
        body.Children.Add(Ui.Card(Ui.Field(Ui.TwoLines("Claude Code 程序", "由本机官方 CLI 登录与执行请求；须为绝对路径。", true), "", binaryError), row));
    }

    // Draft is the provider the fields describe.
    BindingProviderDraft Draft() => new(
        Id: existing?.Id ?? "",
        Name: name.Text.Trim(),
        Protocol: subscription || protocol.SelectedIndex < 0 ? protocolID : page.Protocols[protocol.SelectedIndex].Id,
        Endpoint: endpoint.Text.Trim(),
        ExecutionBackend: (existing?.ExecutionBackend ?? template?.ExecutionBackend) is BindingExecutionBackend.ClaudeCli
            ? new BindingExecutionBackend.ClaudeCli(binary.Text.Trim())
            : new BindingExecutionBackend.Http(),
        RequestProfile: profile,
        AuthenticationSource: authenticationSource,
        QuotaSource: quotaSource,
        Models: [.. models.Select(model => model.Value())],
        QuotaLimits: Limits());

    // Check shows what is missing; false keeps the editor as it is.
    bool Check(BindingProviderDraft draft)
    {
        var cli = draft.ExecutionBackend is BindingExecutionBackend.ClaudeCli;
        Ui.Show(nameError, draft.Name.Length == 0 ? "请填写名称" : "");
        var endpointMissing = !cli && draft.Endpoint.Length == 0;
        var binaryMissing = cli && !Path.IsPathFullyQualified(binary.Text.Trim());
        Ui.Show(endpointError, endpointMissing ? "请填写接口地址" : "");
        Ui.Show(binaryError, binaryMissing ? "请选择 Claude Code 程序的绝对路径" : "");
        var missingID = models.Any(model => model.ID.Length == 0);
        Ui.Show(modelsError, draft.Models.Length == 0 ? "请添加至少一个模型" : missingID ? "每个模型都需要模型 ID" : "");
        var keyMissing = authenticationSource == BindingProviderAuthenticationSource.ApiKey && key.Password.Trim().Length == 0 && !(existing?.Authentication.Configured ?? false);
        Ui.Show(keyError, keyMissing ? "请填写 API key" : "");
        var incomplete = QuotaIncomplete();
        return draft.Name.Length > 0 && !endpointMissing && !binaryMissing && draft.Models.Length > 0 && !missingID && !keyMissing && !incomplete;
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
        var quotaEdit = quotaSource == BindingProviderQuotaSource.ArkCodingUsage ? QuotaEdit() : null;
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
            existing = existing == null
                ? saved.FirstOrDefault(provider => !previous.Contains(provider.Id))
                : saved.FirstOrDefault(provider => provider.Id == existing.Id);
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
    void Saved(BindingGatewayConfig[] gateways)
    {
        var saved = page.SetGateway(gateways);
        if (existing != null)
        {
            existing = saved.FirstOrDefault(provider => provider.Id == existing.Id) ?? existing;
            Refresh();
        }
    }
}
