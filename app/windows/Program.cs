using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Velopack;

namespace Velune;

static class Program
{
    [STAThread]
    static void Main(string[] args)
    {
        // Install/update hooks must exit before WinUI, instance registration or
        // the core opens. Only an explicit user action may apply an update.
        VelopackApp.Build().SetAutoApplyOnStartup(false).Run();
        WinRT.ComWrappersSupport.InitializeComWrappers();
        Application.Start(_ =>
        {
            var dispatcher = DispatcherQueue.GetForCurrentThread();
            System.Threading.SynchronizationContext.SetSynchronizationContext(
                new DispatcherQueueSynchronizationContext(dispatcher));
            new App();
        });
    }
}
