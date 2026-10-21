using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// This PC's gateway providers and the model templates their models fill from.
// A provider is added from a template or imported from an Agent runtime, and
// edited in the page itself, where its sections disclose what it needs.
sealed class ProvidersPage : UserControl
{
    internal const string DefaultGateway = "default";

    internal readonly Core Core;
    internal readonly SettingsWindow Window;
    readonly Grid root = new();
    readonly SelectorBar tabs = new();
    readonly ContentControl shown = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    readonly Grid providerList;
    readonly ModelTemplatesList templateList;
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    readonly InfoBar failure = Ui.FailureBar();
    readonly Button remove = Ui.IconButton("\uE738", "删除 AI 提供商");
    readonly Button edit = new() { Content = "编辑…" };
    ProviderEditor? editor;
    BindingProviderDefinition[] providers = [];
    internal BindingProtocolDescriptor[] Protocols { get; private set; } = [];
    internal BindingProviderTemplate[] Templates { get; private set; } = [];
    internal BindingModelTemplate[] ModelTemplates { get; private set; } = [];
    internal BindingRuntimeInstance[] Runtimes { get; private set; } = [];
    internal BindingRuntimeTypeDescriptor[] RuntimeTypes { get; private set; } = [];
    internal BindingRuntimeTypeDescriptor[] ImportTypes { get; private set; } = [];
    internal BindingUsageDisplay Usage { get; private set; } = BindingUsageDisplay.Used;
    internal bool Busy { get; set; }

    internal ProvidersPage(Core core, SettingsWindow window)
    {
        Core = core;
        Window = window;
        var add = Ui.IconButton("\uE710", "添加 AI 提供商");
        add.Click += (_, _) => Add();
        remove.Click += (_, _) => Delete();
        edit.Click += (_, _) => Edit(Chosen());
        list.SelectionChanged += (_, _) => ShowActions();
        list.DoubleTapped += (_, _) => Edit(Chosen());
        list.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter && Chosen() != null)
            {
                e.Handled = true;
                Edit(Chosen());
            }
        };
        providerList = Ui.ListPage("AI提供商", failure, list, [add, remove], [edit]);
        templateList = new ModelTemplatesList(this);

        tabs.Items.Add(new SelectorBarItem { Text = "提供商", IsSelected = true });
        tabs.Items.Add(new SelectorBarItem { Text = "模型模板" });
        tabs.Margin = new Thickness(16, 4, 16, 0);
        tabs.SelectionChanged += (_, _) => ShowTab();
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.Children.Add(tabs);
        Grid.SetRow(shown, 1);
        root.Children.Add(shown);
        Content = root;
        ShowTab();
        Reload();
    }

    void ShowTab()
    {
        if (editor != null)
        {
            return;
        }
        shown.Content = tabs.SelectedItem == tabs.Items[1] ? templateList : providerList;
    }

    BindingProviderDefinition? Chosen() => list.SelectedIndex is var index and >= 0 && index < providers.Length ? providers[index] : null;

    void ShowActions()
    {
        remove.IsEnabled = Chosen() != null && !Busy;
        edit.IsEnabled = Chosen() != null;
    }

    // Reload reads what the page shows; the editor keeps its own fields.
    internal async void Reload()
    {
        try
        {
            var (snapshot, all) = await Task.Run(() => (Core.Local.List(), VeluneBindingsMethods.ProviderTemplates()));
            Templates = all;
            Protocols = [.. snapshot.Protocols.Where(protocol => protocol.Supported)];
            ModelTemplates = snapshot.ModelTemplates;
            Runtimes = snapshot.RuntimeInstances;
            RuntimeTypes = snapshot.RuntimeTypes;
            ImportTypes = snapshot.ProviderImportTypes;
            Usage = snapshot.UsageDisplay;
            SetGateway(snapshot.Gateways);
            templateList.Show(ModelTemplates);
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    // SetGateway shows the providers saved now; it returns them.
    internal BindingProviderDefinition[] SetGateway(BindingGatewayConfig[] gateways)
    {
        var selected = Chosen()?.Id;
        providers = gateways.FirstOrDefault(gateway => gateway.Id == DefaultGateway)?.Providers ?? [];
        list.Items.Clear();
        foreach (var provider in providers)
        {
            var row = Ui.TwoLines(provider.Name, Summary(provider));
            row.Padding = new Thickness(4, 6, 4, 6);
            var item = new ListViewItem { Content = row };
            item.ContextFlyout = Ui.Menu(
                Ui.MenuItem("编辑…", () => Edit(provider)),
                Ui.MenuItem("删除…", () =>
                {
                    list.SelectedItem = item;
                    Delete();
                }));
            list.Items.Add(item);
        }
        list.SelectedIndex = Array.FindIndex(providers, provider => provider.Id == selected) is var index and >= 0
            ? index
            : Math.Min(list.SelectedIndex, providers.Length - 1);
        Ui.Placeholder(list, "还没有 AI 提供商。用＋从模板添加，或从 Agent 运行时导入。");
        ShowActions();
        return providers;
    }

    // Summary is a provider's row detail: its models, where it connects, and
    // whether its sign-in is done.
    static string Summary(BindingProviderDefinition provider)
    {
        var parts = new List<string> { $"{provider.Models.Length} 个模型" };
        if (provider.ExecutionBackend is BindingExecutionBackend.ClaudeCli)
        {
            parts.Add("Claude Code CLI");
        }
        else if (provider.Endpoint.Length > 0)
        {
            parts.Add(provider.Endpoint);
        }
        if (provider.AuthenticationSource != BindingProviderAuthenticationSource.None && !provider.Authentication.Configured)
        {
            parts.Add(provider.Authentication.Method == BindingAuthenticationMethod.OAuth ? "未登录" : "未配置认证");
        }
        return string.Join(" · ", parts);
    }

    internal string ProtocolName(BindingGatewayProtocol protocol) =>
        Protocols.FirstOrDefault(descriptor => descriptor.Id == protocol)?.Name ?? $"协议 {protocol}";

    // IsSubscription tells whether a provider's connection is fixed by a
    // subscription template, so its protocol and address are not edited.
    internal bool IsSubscription(BindingRequestProfile profile, BindingProviderAuthenticationSource authentication, BindingProviderQuotaSource quota) =>
        Templates.Any(template => template.Kind == BindingProviderTemplateKind.Subscription
            && template.RequestProfile == profile && template.AuthenticationSource == authentication && template.QuotaSource == quota);

    async void Add()
    {
        if (Protocols.Length == 0 || Busy)
        {
            return;
        }
        switch (await ProviderFlows.ChooseTemplate(this))
        {
            case ProviderFlows.Choice.Template { Chosen: var template }:
                Open(new ProviderEditor(this, null, template));
                break;
            case ProviderFlows.Choice.Import:
                await ProviderFlows.Import(this);
                break;
        }
    }

    // EditProvider opens a saved provider's editor unless one is open.
    internal async void EditProvider(string providerID)
    {
        if (editor != null)
        {
            return;
        }
        try
        {
            var snapshot = await Task.Run(() => Core.Local.List());
            if (SetGateway(snapshot.Gateways).FirstOrDefault(provider => provider.Id == providerID) is { } found && editor == null)
            {
                tabs.SelectedItem = tabs.Items[0];
                Edit(found);
            }
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    void Edit(BindingProviderDefinition? existing)
    {
        if (existing != null)
        {
            Open(new ProviderEditor(this, existing, null));
        }
    }

    void Open(ProviderEditor opened)
    {
        editor = opened;
        tabs.Visibility = Visibility.Collapsed;
        shown.Content = opened;
    }

    // Close returns from the editor to the list.
    internal void Close()
    {
        editor = null;
        tabs.Visibility = Visibility.Visible;
        ShowTab();
        Reload();
    }

    async void Delete()
    {
        if (Chosen() is not { } provider || Busy)
        {
            return;
        }
        var message = "删除「" + provider.Name + "」及其模型配置。使用这些模型的运行时需要重新选择模型；原运行时文件与会话保留。";
        if (!await Ui.Confirm(Window, "删除 AI 提供商？", message, "删除"))
        {
            return;
        }
        Busy = true;
        ShowActions();
        try
        {
            SetGateway((await Task.Run(() => Core.Local.DeleteProvider(DefaultGateway, provider.Id))).Gateways);
            Ui.Show(failure, "");
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
        Busy = false;
        ShowActions();
    }

    internal void ShowFailure(string message) => Ui.Show(failure, message);

    internal void SetModelTemplates(BindingModelTemplate[] saved)
    {
        ModelTemplates = saved;
        templateList.Show(saved);
    }
}
