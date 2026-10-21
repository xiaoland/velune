using System.Security.Cryptography;
using Microsoft.UI.Xaml.Media.Imaging;
using Velune.Bindings;

namespace Velune;

// A device a view works on: Local for this PC, else a paired device's peer
// ID. The interface is the same for each; only the application it calls changes.
public readonly record struct Device(string PeerId)
{
    public static readonly Device Local = new("");
    public bool IsLocal => PeerId.Length == 0;
}

// This PC's execution core and the remote service opened with it. Every call
// into the bindings blocks, so views make them on the thread pool and await
// the result back on the UI thread.
sealed class Core
{
    public string Home { get; }
    public string Resources { get; }
    public VeluneApplication Local { get; }
    // View tells this app's windows apart on a controlled device; one per process.
    readonly string view;
    readonly object gate = new();
    VeluneRemoteEndpoint? endpoint;
    readonly Dictionary<string, VeluneApplication> connections = [];

    Core(string home, string resources, VeluneApplication local)
    {
        Home = home;
        Resources = resources;
        Local = local;
        view = "windows-" + Convert.ToHexStringLower(RandomNumberGenerator.GetBytes(16));
    }

    public static Core Open()
    {
        var home = ApplicationHome();
        // Runtime helpers ship in resources beside the executable.
        var resources = Path.Combine(AppContext.BaseDirectory, "resources");
        if (!Directory.Exists(resources))
        {
            throw new InvalidOperationException("缺少运行时资源目录：" + resources);
        }
        var local = VeluneApplication.Open(new BindingOptions(home, resources));
        return new Core(home, resources, local);
    }

    // VELUNE_HOME, else %USERPROFILE%\.velune.
    static string ApplicationHome()
    {
        var configured = Environment.GetEnvironmentVariable("VELUNE_HOME");
        if (!string.IsNullOrEmpty(configured))
        {
            if (!Path.IsPathFullyQualified(configured))
            {
                throw new InvalidOperationException("VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。");
            }
            return configured;
        }
        var profile = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        if (string.IsNullOrEmpty(profile))
        {
            throw new InvalidOperationException("找不到用户目录");
        }
        return Path.Combine(profile, ".velune");
    }

    public static string DeviceName()
    {
        var name = Environment.GetEnvironmentVariable("COMPUTERNAME")?.Trim();
        return string.IsNullOrEmpty(name) ? Environment.MachineName : name;
    }

    // OpenEndpoint opens the remote service once; incoming control must be
    // reachable whenever the app runs.
    public VeluneRemoteEndpoint OpenEndpoint()
    {
        lock (gate)
        {
            if (endpoint != null)
            {
                return endpoint;
            }
            var directory = Path.Combine(Home, "remote");
            Directory.CreateDirectory(directory);
            endpoint = VeluneRemoteEndpoint.Open(new BindingRemoteOptions(
                IdentityFile: Path.Combine(directory, "device-identity"),
                StateFile: Path.Combine(directory, "remote-state.json"),
                DeviceName: DeviceName(),
                Relay: new BindingRelayConfig.DevelopmentPublic(),
                Discovery: new BindingDiscoveryConfig.DevelopmentPublic(),
                PathPolicy: BindingRemotePathPolicy.Automatic,
                BindAddresses: [],
                RequestTimeoutMs: 30_000), Local);
            return endpoint;
        }
    }

    public VeluneRemoteEndpoint Remote()
    {
        lock (gate)
        {
            return endpoint ?? throw new BindingException.Unavailable();
        }
    }

    // Application is a device's application: this PC's own core, or a paired
    // device's through the same named methods.
    public VeluneApplication Application(Device device)
    {
        if (device.IsLocal)
        {
            return Local;
        }
        lock (gate)
        {
            if (connections.TryGetValue(device.PeerId, out var known))
            {
                return known;
            }
        }
        var connection = Remote().Connect(device.PeerId, view);
        lock (gate)
        {
            if (connections.TryGetValue(device.PeerId, out var existing))
            {
                connection.Dispose();
                return existing;
            }
            connections[device.PeerId] = connection;
            return connection;
        }
    }

    // ForgetConnection drops a removed device's connection; the remote service
    // closes it with the device.
    public void ForgetConnection(string peer)
    {
        lock (gate)
        {
            connections.Remove(peer);
        }
    }

    // Shutdown closes connections and the remote service; running
    // conversations stay with their owner.
    public void Shutdown()
    {
        VeluneApplication[] open;
        VeluneRemoteEndpoint? remote;
        lock (gate)
        {
            open = [.. connections.Values];
            connections.Clear();
            remote = endpoint;
        }
        foreach (var connection in open)
        {
            Try(connection.Shutdown);
        }
        if (remote != null)
        {
            Try(remote.Shutdown);
        }
        Try(Local.Shutdown);

        static void Try(Action shutdown)
        {
            try { shutdown(); } catch (BindingException) { }
        }
    }

    // Describe is a message the user can act on; details stay in the core's
    // own diagnostics.
    public static string Describe(Exception failure) => failure switch
    {
        BindingException.Closed => "连接已关闭",
        BindingException.Unavailable => "远程控制服务不可用",
        BindingException.Diagnostic diagnostic when diagnostic.operationId.Length > 0 =>
            $"{diagnostic.detail}（诊断编号：{diagnostic.operationId}）",
        BindingException.Diagnostic diagnostic => diagnostic.detail,
        _ => failure.Message,
    };

    // Brand is Velune's mark from the optical master for its size, for a
    // light or dark background; without the file the app shows no mark.
    public static SvgImageSource Brand(string master, bool dark)
    {
        var source = new SvgImageSource();
        var path = Path.Combine(AppContext.BaseDirectory, "Assets", "Brand", $"velune-{master}-{(dark ? "dark" : "graphite")}.svg");
        if (File.Exists(path))
        {
            _ = source.SetSourceAsync(new MemoryStream(File.ReadAllBytes(path)).AsRandomAccessStream());
        }
        return source;
    }
}
