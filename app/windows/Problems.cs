using Microsoft.UI.Dispatching;
using Velune.Bindings;

namespace Velune;

// One problem: an operation that failed, or a read that keeps failing, with
// all the device said about it.
sealed record Problem(DateTimeOffset Time, string Source, string Device, string Detail, string Kind, string Code, string Phase, string OperationID, string? Key)
{
    // Ongoing problems are reads that fail on every try; they go when one succeeds.
    public bool Ongoing => Key != null;

    public string Text => string.Join("\n", new[]
    {
        $"{Source}（{Device}，{Time.ToLocalTime():yyyy-MM-dd HH:mm:ss}）",
        Detail,
        Kind.Length > 0 ? "类别：" + Kind : "",
        Code.Length > 0 ? "代码：" + Code : "",
        Phase.Length > 0 ? "阶段：" + Phase : "",
        OperationID.Length > 0 ? "诊断编号：" + OperationID : "",
    }.Where(line => line.Length > 0));
}

// Problems are this run's failures, for the 问题 window, instead of each
// view keeping its own: an operation's failure until cleared, a failing read
// once per source until it recovers. They stay in memory, the newest 100.
static class Problems
{
    const int Limit = 100;
    static readonly List<Problem> entries = [];
    static DispatcherQueue? queue;
    static Func<Device, string> name = device => device.IsLocal ? "本机" : device.PeerId;

    public static event Action? Changed;

    // Entries are the problems, newest first.
    public static IReadOnlyList<Problem> Entries => entries;

    public static void Start(DispatcherQueue dispatcher, Func<Device, string> deviceName)
    {
        queue = dispatcher;
        name = deviceName;
    }

    // Report records an operation that failed.
    public static void Report(string source, Exception failure, Device? device = null) =>
        OnThread(() => Add(Make(source, failure, device, null)));

    // Report records an operation that failed without an exception to tell.
    public static void Report(string source, string detail, Device? device = null) =>
        OnThread(() => Add(new Problem(DateTimeOffset.Now, source, name(device ?? Device.Local), detail, "", "", "", "", null)));

    // Failing records a read that failed; while it keeps failing it stays one
    // problem, with the latest detail.
    public static void Failing(string key, string source, Exception failure, Device? device = null) =>
        OnThread(() =>
        {
            var problem = Make(source, failure, device, key);
            var at = entries.FindIndex(entry => entry.Key == key);
            if (at < 0)
            {
                Add(problem);
            }
            else if (entries[at] with { Time = problem.Time } != problem)
            {
                entries[at] = problem with { Time = entries[at].Time };
                Changed?.Invoke();
            }
        });

    // Recovered removes a read's problem once it succeeds.
    public static void Recovered(string key) =>
        OnThread(() =>
        {
            if (entries.RemoveAll(entry => entry.Key == key) > 0)
            {
                Changed?.Invoke();
            }
        });

    public static void Remove(Problem problem)
    {
        if (entries.Remove(problem))
        {
            Changed?.Invoke();
        }
    }

    public static void Clear()
    {
        if (entries.Count > 0)
        {
            entries.Clear();
            Changed?.Invoke();
        }
    }

    // Key names a device's read, so each device's reads fail apart.
    public static string Key(string read, Device device, string what = "") => $"{read}|{device.PeerId}|{what}";

    static Problem Make(string source, Exception failure, Device? shown, string? key)
    {
        var time = DateTimeOffset.Now;
        var device = shown ?? Device.Local;
        return failure switch
        {
            BindingException.Diagnostic diagnostic => new(time, source, name(device), diagnostic.detail, diagnostic.kind.ToString(), diagnostic.code, diagnostic.phase, diagnostic.operationId, key),
            BindingException binding => new(time, source, name(device), Core.Describe(binding), binding.GetType().Name, "", "", "", key),
            _ => new(time, source, name(device), failure.Message, failure.GetType().Name, "", "", "", key),
        };
    }

    static void Add(Problem problem)
    {
        entries.Insert(0, problem);
        if (entries.Count > Limit)
        {
            entries.RemoveRange(Limit, entries.Count - Limit);
        }
        Changed?.Invoke();
    }

    static void OnThread(Action work)
    {
        if (queue == null || queue.HasThreadAccess)
        {
            work();
        }
        else
        {
            queue.TryEnqueue(() => work());
        }
    }
}
