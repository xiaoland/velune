using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;

namespace Velune;

enum SettingsPage { General, Providers, Routes, Runtimes, Devices, Keep }

// The settings window: this PC's general settings, providers, model routes and runtimes, and
// devices. There is one; opening it again brings the open one back.
public sealed partial class SettingsWindow : Window
{
    readonly GeneralPage general;
    readonly ProvidersPage providers;
    readonly RoutesPage routes;
    readonly RuntimesPage runtimes;
    readonly DevicesPage devices;
    readonly SemaphoreSlim dialogs = new(1, 1);

    internal SettingsWindow(Core core, Hub hub, Action<Device> open)
    {
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(SettingsTitleBar);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Velune.ico"));
        if (MicaController.IsSupported())
        {
            SystemBackdrop = new MicaBackdrop();
        }
        else
        {
            Root.Style = Ui.Style("WindowBackgroundStyle");
        }
        WorkspaceWindow.Size(this, 820, 640, 600, 440);
        ShowMark();
        Root.ActualThemeChanged += (_, _) => ShowMark();

        general = new GeneralPage(core);
        providers = new ProvidersPage(core, this);
        routes = new RoutesPage(core, this, EditProvider);
        runtimes = new RuntimesPage(core, this);
        devices = new DevicesPage(core, hub, this, open);
        Navigation.SelectionChanged += (_, e) =>
        {
            Page.Content = (e.SelectedItemContainer?.Tag as string) switch
            {
                "general" => general,
                "providers" => providers,
                "routes" => routes,
                "runtimes" => runtimes,
                _ => devices,
            };
            // Routes name providers' models, which either page may have changed.
            if (ReferenceEquals(Page.Content, routes))
            {
                routes.Reload();
            }
            else if (ReferenceEquals(Page.Content, providers))
            {
                providers.Reload();
            }
        };
        Navigation.SelectedItem = Navigation.MenuItems[0];
    }

    void ShowMark()
    {
        TitleIcon.ImageSource = Core.Brand("small", Root.ActualTheme == ElementTheme.Dark);
    }

    internal void Show(SettingsPage page)
    {
        if (page != SettingsPage.Keep)
        {
            Navigation.SelectedItem = Navigation.MenuItems[(int)page];
        }
        Activate();
    }

    // EditProvider opens a provider's editor, as a route member asks.
    void EditProvider(string providerID)
    {
        Navigation.SelectedItem = Navigation.MenuItems[(int)SettingsPage.Providers];
        providers.EditProvider(providerID);
    }

    // HubChanged shows the remote service's new state.
    internal void HubChanged() => devices.HubChanged();

    internal async Task<ContentDialogResult> ShowDialog(ContentDialog dialog)
    {
        await dialogs.WaitAsync();
        try
        {
            dialog.XamlRoot = Root.XamlRoot;
            return await dialog.ShowAsync();
        }
        finally
        {
            dialogs.Release();
        }
    }
}
