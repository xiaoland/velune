using Microsoft.UI.Xaml;
using Velune.Bindings;

namespace Velune;

// Velune for Windows: a full device that runs agents on this PC and controls
// paired devices through the same interface.
public partial class App : Application
{
    MainWindow? window;

    public App()
    {
        // WinUI ends the process on an unhandled exception with only a stowed
        // exception code, so keep its trace where a report can find it.
        AppDomain.CurrentDomain.UnhandledException += (_, e) => Record(e.ExceptionObject);
        UnhandledException += (_, e) => Record(e.Exception);
        InitializeComponent();
    }

    static void Record(object failure)
    {
        try
        {
            var folder = Path.Combine(Path.GetTempPath(), "Velune");
            Directory.CreateDirectory(folder);
            File.AppendAllText(Path.Combine(folder, "crash.log"), $"{DateTimeOffset.Now:O}\n{failure}\n\n");
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

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
        // The process ends with its last window.
        window.Closed += (_, _) => core.Shutdown();
        window.Activate();
    }
}
