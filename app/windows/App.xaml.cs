using Microsoft.UI.Xaml;
using Velune.Bindings;

namespace Velune;

// Velune for Windows: a full device that runs agents on this PC and controls
// paired devices through the same interface.
public partial class App : Application
{
    MainWindow? window;

    public App() => InitializeComponent();

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        Core core;
        try
        {
            core = Core.Open();
        }
        catch (Exception failure)
        {
            var message = failure is BindingException binding ? Core.Describe(binding) : failure.Message;
            new FailureWindow("Velune 无法启动：" + message).Activate();
            return;
        }
        window = new MainWindow(core);
        // Closing the main window quits; agents on this PC stop with the app.
        window.Closed += (_, _) =>
        {
            core.Shutdown();
            Exit();
        };
        window.Activate();
    }
}
