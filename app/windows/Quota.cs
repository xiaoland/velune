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
            _ => id,
        };
    }

    // Detail is a window's use as the usage display shows it, its limit when
    // one is set, when it resets, and whether the observation is old.
    public static string Detail(BindingProviderQuotaWindow window, BindingQuotaFreshness freshness, BindingUsageDisplay usage, BindingQuotaLimit? limit = null)
    {
        var used = Math.Clamp(window.UsedPercent, 0, 100);
        var remaining = usage == BindingUsageDisplay.Remaining;
        var parts = new List<string> { remaining ? $"剩余 {100 - used:0.#}%" : $"已用 {used:0.#}%" };
        if (limit != null)
        {
            parts.Add(remaining ? $"保留 {100 - limit.MaxUsedPercent}%" : $"上限 {limit.MaxUsedPercent}%");
        }
        if (window.ResetAtMs is { } reset)
        {
            parts.Add("重置于 " + DateTimeOffset.FromUnixTimeMilliseconds(reset).ToLocalTime().ToString("M月d日 HH:mm"));
        }
        if (window.ModelIds is { Length: > 0 } scoped)
        {
            parts.Add("适用于 " + string.Join("、", scoped));
        }
        if (freshness == BindingQuotaFreshness.Stale || window.ExpiresAtMs < DateTimeOffset.UtcNow.ToUnixTimeMilliseconds())
        {
            parts.Add("观测已过期");
        }
        return string.Join(" · ", parts);
    }

    // Applies tells whether a window counts for a model ID: a window scoped
    // to models names it, the others cover every model.
    public static bool Applies(BindingProviderQuotaWindow window, string modelID) =>
        window.ModelIds is not { Length: > 0 } scoped || scoped.Contains(modelID);
}
