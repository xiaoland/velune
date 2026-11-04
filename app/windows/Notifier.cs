using System.Security.Cryptography;
using System.Text;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;

namespace Velune;

// Notifier tells, by Windows' notifications, that a Velune turn ended (not
// one the user stopped) or that a conversation waits for the user, unless
// the conversation shows in front. A conversation has one notification,
// the latest. Clicking one opens its conversation in the window used last.
// While Velune is in the background and more conversations wait or end,
// its taskbar button flashes until it comes back: Windows gives numbered
// badges only to packaged apps. Whether notifications show is the
// system's setting.
sealed class Notifier(Shell shell, DispatcherQueue dispatcher)
{
    bool registered;
    // Pending interactions per conversation, to notice new ones.
    readonly Dictionary<(Device, string), uint> pending = [];
    // Conversations whose turn ended while Velune was in the background.
    readonly HashSet<(Device, string)> endedAway = [];
    // How many conversations wanted the user at the last look.
    int wanting;
    // Quota alerts come from this PC's core; the cursor is the last one read.
    DispatcherTimer? quotaTimer;
    ulong quotaCursor;
    bool readingQuota;

    // Shell routes cold and redirected activation; this receives live clicks.
    public void Start()
    {
        try
        {
            if (!AppNotificationManager.IsSupported())
            {
                return;
            }
            AppNotificationManager.Default.NotificationInvoked += (_, e) => dispatcher.TryEnqueue(() => Open(e.Arguments));
            try
            {
                AppNotificationManager.Default.Register("Velune", new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", "Velune.ico")));
            }
            catch (ArgumentException)
            {
                AppNotificationManager.Default.Register();
            }
            registered = true;
            quotaTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(3) };
            quotaTimer.Tick += (_, _) => ReadQuotaAlerts();
            quotaTimer.Start();
        }
        catch (Exception failure) when (failure is System.Runtime.InteropServices.COMException or InvalidOperationException or UnauthorizedAccessException)
        {
            Problems.Report("启用系统通知", failure);
        }
    }

    // Stop lets a later click launch Velune again rather than reach this run.
    public void Stop()
    {
        quotaTimer?.Stop();
        quotaTimer = null;
        if (registered)
        {
            registered = false;
            AppNotificationManager.Default.Unregister();
        }
    }

    // Polled takes a device's latest list: what waits and what ended since
    // the last read. The first read of a device only sets where to start.
    public void Polled(Device device, Listing listing, FinishedTurn[] ended, bool first)
    {
        var waiting = new HashSet<string>();
        foreach (var active in listing.Active.Values)
        {
            var key = (device, active.ConversationId);
            if (active.PendingInteractions > 0)
            {
                waiting.Add(active.ConversationId);
            }
            pending.TryGetValue(key, out var before);
            if (active.PendingInteractions > before)
            {
                Notify(device, listing, active.ContextRuntimeId, active.ConversationId, "等待你的确认");
            }
            pending[key] = active.PendingInteractions;
        }
        foreach (var key in pending.Keys.Where(key => key.Item1 == device && !waiting.Contains(key.Item2)).ToList())
        {
            pending.Remove(key);
        }
        if (!first)
        {
            foreach (var turn in ended.Where(turn => !turn.Silent))
            {
                var state = turn.Outcome switch
                {
                    TurnOutcome.Completed => "已完成",
                    TurnOutcome.Failed => "失败",
                    _ => null,
                };
                if (state == null)
                {
                    continue;
                }
                Notify(device, listing, turn.ContextRuntimeId, turn.ConversationId, state);
                if (!shell.InFront)
                {
                    endedAway.Add((device, turn.ConversationId));
                }
            }
        }
        Attend();
    }

    // Returned clears what ended while Velune was in the background.
    public void Returned()
    {
        endedAway.Clear();
        Attend();
    }

    void Notify(Device device, Listing listing, string runtimeID, string conversationID, string state)
    {
        if (!registered || shell.Viewing(device, conversationID))
        {
            return;
        }
        var title = listing.Conversations.FirstOrDefault(listed => listed.Conversation.Id == conversationID)?.Conversation.Title.DisplayText() is { Length: > 0 } named ? named : "会话";
        var where = device.IsLocal ? "" : "（" + shell.Hub.DeviceName(device) + "）";
        var notification = new AppNotificationBuilder()
            .AddArgument("device", device.PeerId)
            .AddArgument("runtime", runtimeID)
            .AddArgument("conversation", conversationID)
            .AddArgument("title", title)
            .AddText(title)
            .AddText(state + where)
            .BuildNotification();
        // The same tag replaces the conversation's earlier notification.
        notification.Tag = Tag(device, conversationID);
        notification.Group = "conversations";
        try
        {
            AppNotificationManager.Default.Show(notification);
        }
        catch (System.Runtime.InteropServices.COMException failure)
        {
            Problems.Report("显示系统通知", failure, device);
        }
    }

    // ReadQuotaAlerts announces the quota conditions this PC's core raised
    // since the last read: a usage limit reached, a window used up, or an
    // account the provider calls unavailable. Each shows once.
    async void ReadQuotaAlerts()
    {
        if (readingQuota || quotaTimer == null)
        {
            return;
        }
        readingQuota = true;
        try
        {
            var since = quotaCursor;
            var (alerts, snapshot) = await Task.Run(() =>
            {
                var read = shell.Core.Local.QuotaAlerts(since);
                return (read, read.Alerts.Length > 0 ? shell.Core.Local.List() : null);
            });
            quotaCursor = alerts.Cursor;
            Problems.Recovered(Problems.Key("quota-alerts", Device.Local, ""));
            if (snapshot != null && registered && quotaTimer != null)
            {
                foreach (var alert in alerts.Alerts)
                {
                    NotifyQuota(alert, snapshot);
                }
            }
        }
        catch (Exception problem)
        {
            Problems.Failing(Problems.Key("quota-alerts", Device.Local, ""), "读取额度提醒", problem);
        }
        readingQuota = false;
    }

    void NotifyQuota(QuotaAlert alert, ConfigurationSnapshot snapshot)
    {
        var provider = snapshot.Gateways.FirstOrDefault(gateway => gateway.Id == alert.GatewayId)?.Providers.FirstOrDefault(provider => provider.Id == alert.ProviderId);
        if (provider == null)
        {
            return;
        }
        var window = alert.WindowId is { } id ? Quota.Name(id) : "";
        var detail = alert.Kind switch
        {
            QuotaAlertKind.CapReached => window + Quota.Ceiling(alert.MaxUsedPercent ?? 100, snapshot.UsageDisplay, true) + "，直到额度回到上限以内。",
            QuotaAlertKind.Exhausted => window + "已用尽，重置前暂停向它发送新请求。",
            _ => "提供商表示此账户当前不可用（例如余额不足），暂停新请求。",
        };
        var notification = new AppNotificationBuilder()
            .AddText(provider.Name)
            .AddText("额度提醒：" + detail)
            .BuildNotification();
        // One notification per provider, condition and window.
        notification.Tag = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(alert.ProviderId + "\n" + alert.Kind + "\n" + alert.WindowId)))[..32];
        notification.Group = "quota";
        try
        {
            AppNotificationManager.Default.Show(notification);
        }
        catch (System.Runtime.InteropServices.COMException failure)
        {
            Problems.Report("显示系统通知", failure);
        }
    }

    public void Open(IDictionary<string, string> arguments)
    {
        if (arguments.TryGetValue("conversation", out var id) && arguments.TryGetValue("runtime", out var runtimeID))
        {
            var device = new Device(arguments.TryGetValue("device", out var peer) ? peer : "");
            shell.OpenNotified(device, new Target(runtimeID, id, arguments.TryGetValue("title", out var title) ? title : ""));
        }
    }

    // Attend flashes the taskbar button when more conversations want the
    // user while Velune is in the background.
    void Attend()
    {
        var count = pending.Count(entry => entry.Value > 0) + endedAway.Count(key => !pending.ContainsKey(key));
        if (count > wanting && !shell.InFront)
        {
            shell.Flash();
        }
        wanting = count;
    }

    // Tag names a conversation's notification within Windows' 64 characters.
    static string Tag(Device device, string conversationID) =>
        Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(device.PeerId + "\n" + conversationID)))[..32];
}
