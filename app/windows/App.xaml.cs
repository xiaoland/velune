using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using System.Security.Cryptography;
using System.Text;
using Velune.Bindings;

namespace Velune;

// Velune for Windows: a full device that runs agents on this PC and controls
// paired devices through the same interface.
public partial class App : Application
{
    Shell? shell;

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

    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        try
        {
            var dispatcher = Microsoft.UI.Dispatching.DispatcherQueue.GetForCurrentThread();
            var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
            var home = Path.GetFullPath(Core.ApplicationHome()).TrimEnd(Path.DirectorySeparatorChar).ToUpperInvariant();
            var key = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(home)));
            var instance = AppInstance.FindOrRegisterForKey("velune-" + key);
            if (!instance.IsCurrent)
            {
                await instance.RedirectActivationToAsync(activation);
                Exit();
                return;
            }
            var pending = new Queue<AppActivationArguments>();
            instance.Activated += (_, incoming) => dispatcher.TryEnqueue(() =>
            {
                if (shell == null) pending.Enqueue(incoming); else Activate(incoming);
            });
            var core = await Task.Run(Core.Open);
            shell = new Shell(core, dispatcher);
            shell.Start();
            // An isolated home must not replace the user's system URL handler.
            if (string.IsNullOrEmpty(Environment.GetEnvironmentVariable("VELUNE_HOME")))
            {
                try { ActivationRegistrationManager.RegisterForProtocolActivation("agents", "", "Velune", Environment.ProcessPath!); }
                catch (Exception failure) { Problems.Report("注册 xURL 地址", failure); }
            }
            Activate(activation);
            while (pending.TryDequeue(out var incoming)) Activate(incoming);
        }
        catch (Exception failure)
        {
            var message = failure is BindingException binding ? Core.Describe(binding) : failure.Message;
            new FailureWindow("Velune 无法启动：" + message).Activate();
            return;
        }
    }

    void Activate(AppActivationArguments activation)
    {
        try { shell?.Activate(activation); }
        catch (Exception failure) { Problems.Report("打开外部请求", failure); }
    }
}
