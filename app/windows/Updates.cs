using System.Threading;
using Microsoft.UI.Dispatching;
using Velopack;
using Velopack.Locators;
using Velopack.Sources;

namespace Velune;

enum UpdateState { Unavailable, Idle, Checking, Available, Downloading, Ready, Failed, Applying }

// One update session per application. Network work never closes a window or
// applies a package. Shell alone decides when the app can exit normally.
sealed class Updates
{
    public const string PackageId = "Velune";
    public const string Channel = "win-x64-beta";
    public const string ReleasesUrl = "https://github.com/xiaoland/velune/releases";
    readonly DispatcherQueue dispatcher;
    readonly UpdateManager manager;
    readonly CancellationTokenSource lifetime = new();
    DispatcherQueueTimer? timer;
    UpdateInfo? available;
    VelopackAsset? ready;
    bool stopped;

    public UpdateState State { get; private set; } = UpdateState.Unavailable;
    public string Failure { get; private set; } = "";
    public int Progress { get; private set; }
    public string Version => (ready?.Version ?? available?.TargetFullRelease.Version)?.ToString() ?? "";
    public bool CanRetryDownload => available != null;
    public bool CanApply => ready != null && State is (UpdateState.Ready or UpdateState.Failed);
    public event Action? Changed;

    public Updates(DispatcherQueue dispatcher) : this(dispatcher,
        new UpdateManager(new GithubSource("https://github.com/xiaoland/velune", null, prerelease: true),
            new UpdateOptions { ExplicitChannel = Channel }))
    {
    }

    internal Updates(DispatcherQueue dispatcher, UpdateManager manager)
    {
        this.dispatcher = dispatcher;
        this.manager = manager;
        // An isolated run must not change the shared installation. The raw
        // publish tree and portable packages use the Releases download entry.
        if (!string.IsNullOrEmpty(Environment.GetEnvironmentVariable("VELUNE_HOME"))
            || !manager.IsInstalled || manager.IsPortable || manager.AppId != PackageId)
        {
            return;
        }
        ready = manager.UpdatePendingRestart;
        State = ready == null ? UpdateState.Idle : UpdateState.Ready;
    }

    // Shortcuts and protocol activation target the install's stable launcher.
    public static string LaunchPath()
    {
        var root = VelopackLocator.Current.RootAppDir;
        var launcher = root == null ? null : Path.Combine(root, "Velune.exe");
        return launcher != null && File.Exists(launcher) ? launcher : Environment.ProcessPath!;
    }

    public void Start()
    {
        if (State == UpdateState.Unavailable || stopped || timer != null) return;
        timer = dispatcher.CreateTimer();
        timer.Interval = TimeSpan.FromHours(12);
        timer.Tick += (_, _) => _ = CheckAsync();
        timer.Start();
        _ = CheckAsync();
    }

    public async Task CheckAsync()
    {
        if (stopped || ready != null || available != null || State is not (UpdateState.Idle or UpdateState.Failed)) return;
        available = null;
        SetState(UpdateState.Checking);
        try
        {
            var found = await manager.CheckForUpdatesAsync();
            if (stopped) return;
            available = found;
            SetState(found == null ? UpdateState.Idle : UpdateState.Available);
        }
        catch (Exception failure)
        {
            Fail(failure);
        }
    }

    public async Task DownloadAsync()
    {
        if (stopped || available == null || State is not (UpdateState.Available or UpdateState.Failed)) return;
        Progress = 0;
        SetState(UpdateState.Downloading);
        try
        {
            await manager.DownloadUpdatesAsync(available, progress => dispatcher.TryEnqueue(() =>
            {
                if (stopped || State != UpdateState.Downloading) return;
                Progress = progress;
                Changed?.Invoke();
            }), lifetime.Token);
            if (stopped) return;
            ready = available.TargetFullRelease;
            SetState(UpdateState.Ready);
        }
        catch (Exception failure)
        {
            Fail(failure);
        }
    }

    // Velopack passes this exact process ID to Update.exe and waits for its
    // normal exit. Unlike ApplyUpdatesAndRestart, this does not exit the app.
    public bool WaitForExitAndRestart()
    {
        if (stopped || ready == null || State is not (UpdateState.Ready or UpdateState.Failed)) return false;
        try
        {
            manager.WaitExitThenApplyUpdates(ready, silent: false, restart: true);
            SetState(UpdateState.Applying);
            return true;
        }
        catch (Exception failure)
        {
            Fail(failure);
            return false;
        }
    }

    public void Stop()
    {
        stopped = true;
        timer?.Stop();
        lifetime.Cancel();
    }

    internal void Fail(Exception failure)
    {
        if (stopped) return;
        Failure = failure.Message;
        SetState(UpdateState.Failed);
    }

    void SetState(UpdateState state)
    {
        State = state;
        if (state != UpdateState.Failed) Failure = "";
        Changed?.Invoke();
    }
}
