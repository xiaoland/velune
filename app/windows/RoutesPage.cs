using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// This PC's model routes: provider models chosen together under one name,
// which the gateway picks among by strategy and fails over between. A route
// opens in the page, where its members show whether the route can use them.
sealed class RoutesPage : UserControl
{
    internal readonly Core Core;
    internal readonly SettingsWindow Window;
    // EditProvider opens a provider's editor on the providers page.
    internal readonly Action<string> EditProvider;
    readonly ContentControl shown = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    readonly Grid routeList;
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    readonly Button remove = Ui.IconButton("\uE738", "删除模型路由");
    readonly Button edit = new() { Content = "编辑…" };
    RouteEditor? editor;
    BindingModelRoute[] routes = [];
    internal BindingProviderDefinition[] Providers { get; private set; } = [];
    internal BindingUsageDisplay Usage { get; private set; } = BindingUsageDisplay.Used;

    internal RoutesPage(Core core, SettingsWindow window, Action<string> editProvider)
    {
        Core = core;
        Window = window;
        EditProvider = editProvider;
        var add = Ui.IconButton("\uE710", "添加模型路由");
        add.Click += (_, _) => Open(null);
        remove.Click += (_, _) => Delete();
        edit.Click += (_, _) => Open(Chosen());
        list.SelectionChanged += (_, _) => ShowActions();
        list.DoubleTapped += (_, _) => Open(Chosen());
        list.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter && Chosen() != null)
            {
                e.Handled = true;
                Open(Chosen());
            }
        };
        routeList = Ui.ListPage("模型路由", null, list, [add, remove], [edit]);
        shown.Content = routeList;
        Content = shown;
        Reload();
    }

    BindingModelRoute? Chosen() => list.SelectedIndex is var index and >= 0 && index < routes.Length ? routes[index] : null;

    void ShowActions()
    {
        remove.IsEnabled = Chosen() != null;
        edit.IsEnabled = Chosen() != null;
    }

    // Reload reads routes and providers again, as a provider may have changed
    // on its page; an open route keeps its fields.
    internal async void Reload()
    {
        try
        {
            var snapshot = await Task.Run(() => Core.Local.List());
            Usage = snapshot.UsageDisplay;
            SetGateway(snapshot.Gateways);
        }
        catch (Exception problem)
        {
            Problems.Report("读取模型路由", problem);
        }
    }

    internal void SetGateway(BindingGatewayConfig[] gateways)
    {
        var gateway = gateways.FirstOrDefault(candidate => candidate.Id == ProvidersPage.DefaultGateway);
        Providers = gateway?.Providers ?? [];
        var selected = Chosen()?.RecordKey;
        routes = gateway?.Routes ?? [];
        list.Items.Clear();
        foreach (var route in routes)
        {
            var row = Ui.TwoLines(route.Name, Summary(route));
            row.Padding = new Thickness(4, 6, 4, 6);
            var item = new ListViewItem { Content = row };
            item.ContextFlyout = Ui.Menu(
                Ui.MenuItem("编辑…", () => Open(route)),
                Ui.MenuItem("删除…", () =>
                {
                    list.SelectedItem = item;
                    Delete();
                }));
            list.Items.Add(item);
        }
        list.SelectedIndex = Array.FindIndex(routes, route => route.RecordKey == selected) is var index and >= 0
            ? index
            : Math.Min(list.SelectedIndex, routes.Length - 1);
        Ui.Placeholder(list, "还没有模型路由。用＋把多个提供商模型组成一个路由，按策略选择并在失败时切换。");
        ShowActions();
        editor?.ProvidersChanged();
    }

    static string Summary(BindingModelRoute route) =>
        string.Join(" · ", new[] { StrategyName(route.Strategy), $"{route.Members.Length} 个成员", route.Sticky ? "会话保持" : "" }.Where(part => part.Length > 0));

    internal static string StrategyName(BindingRoutingStrategy strategy) => strategy switch
    {
        BindingRoutingStrategy.Smart => "智能",
        BindingRoutingStrategy.Order => "顺序",
        BindingRoutingStrategy.Rotate => "轮询",
        _ => "权重",
    };

    // Member is the provider and model a route member's record names; null
    // when the model is gone.
    internal (BindingProviderDefinition Provider, BindingProviderModel Model)? Member(string key)
    {
        foreach (var provider in Providers)
        {
            if (provider.Models.FirstOrDefault(model => model.RecordKey == key) is { } model)
            {
                return (provider, model);
            }
        }
        return null;
    }

    void Open(BindingModelRoute? route)
    {
        if (editor != null)
        {
            return;
        }
        editor = new RouteEditor(this, route);
        shown.Content = editor;
    }

    internal void Close()
    {
        editor = null;
        shown.Content = routeList;
        Reload();
    }

    async void Delete()
    {
        if (Chosen() is not { } route
            || !await Ui.Confirm(Window, "删除模型路由？", "删除路由「" + route.Name + "」。选用它的会话需要重新选择模型；成员提供商与模型保留。", "删除"))
        {
            return;
        }
        try
        {
            SetGateway((await Task.Run(() => Core.Local.DeleteModelRoute(ProvidersPage.DefaultGateway, route.RecordKey))).Gateways);
        }
        catch (Exception problem)
        {
            Problems.Report("删除模型路由「" + route.Name + "」", problem);
        }
    }
}
