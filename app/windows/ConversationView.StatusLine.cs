using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// The composer's status line: the items the settings keep in sight, from the
// runtime's own turn records; an item with no record is left out, not
// estimated. The button shows every item there is.
public sealed partial class ConversationView
{
    // The working directory's git branch, read once per finished turn.
    (string Directory, string? Branch, int Turns)? branch;
    bool readingBranch;

    sealed record StatusItem(StatusLineItem Item, string Label, string Shown, string Detail);

    // StatusItems are the items the conversation has records for, in the
    // status line's order.
    List<StatusItem> StatusItems()
    {
        var items = new List<StatusItem>();
        if (Model.Snapshot is not { } snapshot)
        {
            return items;
        }
        var remaining = Model.Configuration.Usage == UsageDisplay.Remaining;
        var contextRemaining = Model.Configuration.ContextUsage == UsageDisplay.Remaining;
        var usage = snapshot.Usage;
        if (usage.ContextTokens is { } context)
        {
            if (usage.ContextWindow is { } window and > 0)
            {
                var used = Math.Clamp(100.0 * context / window, 0, 100);
                items.Add(new(StatusLineItem.ContextUsage, "上下文",
                    contextRemaining ? $"上下文剩余 {100 - used:0}%" : $"上下文 {used:0}%",
                    contextRemaining ? $"上下文剩余 {Count(window > context ? window - context : 0)} / {Count(window)} token。" : $"上下文占用 {Count(context)} / {Count(window)} token。"));
            }
            else
            {
                items.Add(new(StatusLineItem.ContextUsage, "上下文", $"上下文 {Count(context)}", "模型的上下文窗口未知，只显示最近一次请求的 token 数。"));
            }
        }
        if (usage.SessionTokens is { } tokens)
        {
            items.Add(new(StatusLineItem.SessionTokens, "本会话 token", $"{Count(tokens)} token", "本会话各轮记录的输入与输出 token 之和。"));
        }
        if (snapshot.Conversation.Cwd is { Length: > 0 } cwd)
        {
            var shownBranch = branch is { } known && known.Directory == cwd ? known.Branch : null;
            items.Add(new(StatusLineItem.WorkingDirectory, "工作目录",
                Listing.LastPart(cwd) + (shownBranch != null ? " · " + shownBranch : ""),
                cwd + (shownBranch != null ? "，分支 " + shownBranch : "")));
            ReadBranch(cwd, snapshot.TranscriptTurns.Length);
        }
        if (usage.RateLimits.Length > 0)
        {
            var limits = usage.RateLimits.Select(limit =>
                (limit.WindowMinutes is { } minutes ? Period(minutes) + " " : "") + (remaining ? $"剩余 {100 - Math.Min(limit.UsedPercent, 100)}%" : $"{limit.UsedPercent}%"));
            items.Add(new(StatusLineItem.RateLimits, "账户额度", string.Join(" · ", limits), "运行时最近报告的账户用量限额。"));
        }
        if (usage.SessionCostMicroUsd is { } cost)
        {
            items.Add(new(StatusLineItem.SessionCost, "费用估算", $"${cost / 1_000_000.0:0.00}", "运行时按其价格表估算的本会话费用；经网关或订阅时不代表实际计费。"));
        }
        return items;
    }

    void RenderStatus()
    {
        var items = StatusItems();
        var kept = items.Where(item => Model.Configuration.StatusLine.Contains(item.Item)).ToList();
        StatusText.Text = string.Join("  ", kept.Select(item => item.Shown));
        StatusButton.Visibility = Visible(items.Count > 0);
        if (kept.Count == 0 && items.Count > 0)
        {
            StatusText.Text = "状态";
        }
        ToolTipService.SetToolTip(StatusButton, kept.Count > 0
            ? string.Join("\n", kept.Select(item => item.Label + "：" + item.Detail)) + "\n点按查看全部状态。"
            : "点按查看会话状态。");
    }

    void ShowStatus()
    {
        var panel = new StackPanel { Spacing = 10, MinWidth = 260, MaxWidth = 360 };
        foreach (var item in StatusItems())
        {
            var lines = new StackPanel { Spacing = 2 };
            lines.Children.Add(Ui.Text(item.Label + "：" + item.Shown, "BodyStrongTextBlockStyle"));
            lines.Children.Add(Ui.Muted(item.Detail));
            panel.Children.Add(lines);
        }
        panel.Children.Add(Ui.Muted("在设置的“通用”中选择常驻显示的项。"));
        new Flyout { Content = panel, Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.Top }.ShowAt(StatusButton);
    }

    // ReadBranch reads the working directory's branch once per finished turn.
    async void ReadBranch(string cwd, int turns)
    {
        if (readingBranch || (branch is { } known && known.Directory == cwd && known.Turns == turns) || Model.Snapshot is not { } snapshot)
        {
            return;
        }
        readingBranch = true;
        try
        {
            var changes = await Model.WorkingChanges(snapshot.ContextRuntimeId, snapshot.Conversation.Id);
            branch = (cwd, changes.Branch, turns);
            RenderStatus();
        }
        catch (Exception)
        {
            // Outside a repository, or the device cannot tell; the directory shows alone.
            branch = (cwd, null, turns);
        }
        readingBranch = false;
    }

    static string Period(ulong minutes) =>
        minutes % 1440 == 0 ? $"{minutes / 1440} 天" : minutes % 60 == 0 ? $"{minutes / 60} 小时" : $"{minutes} 分钟";

    static string Count(ulong count) => TokenCount.Format(count);
}
