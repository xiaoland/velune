using Velune.Bindings;

namespace Velune;

// How quota windows read, wherever the core's observation is shown.
static class Quota
{
    // Name names a quota window by the period its ID records.
    public static string Name(string id)
    {
        if (id.StartsWith("window:") && long.TryParse(id["window:".Length..], out var seconds) && seconds > 0)
        {
            return (seconds % 86400 == 0 ? $"{seconds / 86400} 天" : seconds % 3600 == 0 ? $"{seconds / 3600} 小时" : $"{seconds / 60} 分钟") + "窗口";
        }
        return id switch
        {
            "ark:session" => "会话额度",
            "ark:weekly" => "每周额度",
            "ark:monthly" => "每月额度",
            "kimi:limit_5h" => "5 小时窗口",
            "kimi:limit_7d" => "7 天窗口",
            "kimi:limit_month_total" => "本月总额度",
            "kimi:limit_month_code" => "本月编程额度",
            "openrouter:keyLimit" => "Key 花费上限",
            _ => id,
        };
    }

    // Detail is a window's use as the usage display shows it, its limit when
    // one is set, when it resets, and whether the observation is old.
    public static string Detail(BindingQuotaWindowObservation window, BindingQuotaFreshness freshness, BindingUsageDisplay usage, BindingQuotaLimit? limit = null)
    {
        var used = Math.Clamp(window.UsedPercent, 0, 100);
        var remaining = usage == BindingUsageDisplay.Remaining;
        var parts = new List<string> { remaining ? $"剩余 {100 - used:0.#}%" : $"已用 {used:0.#}%" };
        if (limit != null)
        {
            parts.Add(Ceiling(limit.MaxUsedPercent, usage, false));
        }
        if (window.ResetAtMs is { } reset)
        {
            parts.Add("重置于 " + Time(reset));
        }
        if (Short(window, freshness) && window.Pace?.RunsOutAtMs is { } empty)
        {
            parts.Add("按目前的平均速度约 " + Time(empty) + " 用尽");
        }
        if (window.ModelIds is { Length: > 0 } scoped)
        {
            parts.Add("适用于 " + string.Join("、", scoped));
        }
        if (freshness == BindingQuotaFreshness.Stale || (window.ExpiresAtMs is long expiry && expiry < DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()))
        {
            parts.Add("观测已过期");
        }
        return string.Join(" · ", parts);
    }

    // Short tells that, at the average rate so far, a freshly observed
    // window runs out before it resets.
    public static bool Short(BindingQuotaWindowObservation window, BindingQuotaFreshness freshness) =>
        freshness == BindingQuotaFreshness.Fresh && window.Pace?.Status == BindingQuotaPaceStatus.RunsOutBeforeReset;

    public static string Time(long ms) => DateTimeOffset.FromUnixTimeMilliseconds(ms).ToLocalTime().ToString("M月d日 HH:mm");

    // BalanceName names an amount the account reports.
    public static string BalanceName(string id) => id == "openrouter:keyLimitRemaining" ? "Key 剩余额度" : "余额";

    public static string Amount(double amount, string unit) => amount.ToString("N2") + " " + unit;

    // Ceiling is a used-percent limit as the usage display reads it; reached,
    // it says new requests pause.
    public static string Ceiling(byte limit, BindingUsageDisplay usage, bool reached) => (usage, reached) switch
    {
        (BindingUsageDisplay.Remaining, false) => $"保留 {100 - limit}%",
        (BindingUsageDisplay.Remaining, true) => $"剩余已不足保留的 {100 - limit}%，暂停新请求",
        (_, false) => $"上限 {limit}%",
        _ => $"已达上限 {limit}%，暂停新请求",
    };

    // Balances lists the amounts the account reports, such as its prepaid
    // balance; an account that reports neither windows nor amounts says so.
    public static string Balances(BindingProviderQuotaStatus status)
    {
        var amounts = status.Balances.Select(balance => BalanceName(balance.Id) + "：" + Amount(balance.Amount, balance.Unit)).ToList();
        if (amounts.Count == 0 && !status.Windows.Any() && status.Freshness == BindingQuotaFreshness.Fresh)
        {
            return "提供商未报告额度上限或余额。";
        }
        return string.Join(" · ", amounts);
    }

    // Applies tells whether a window counts for a model ID: a window scoped
    // to models names it, the others cover every model.
    public static bool Applies(BindingQuotaWindowObservation window, string modelID) =>
        window.ModelIds is not { Length: > 0 } scoped || scoped.Contains(modelID);
}
