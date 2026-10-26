using System.Globalization;
using System.Text.RegularExpressions;

namespace Velune;

static class TokenCount
{
    internal const string InputError = "Token 数须为正整数，可用 K、M、B（如 272K、1.5M），且不超过 4.294967295B；最大输出不能超过上下文窗口。";

    // 显示值也会填入编辑器，保留余数才能让未修改的配置无损保存。
    internal static string Format(ulong count)
    {
        foreach (var (scale, suffix) in new[] { (1_000_000_000UL, "B"), (1_000_000UL, "M"), (1_000UL, "K") })
        {
            if (count >= scale)
            {
                return ((decimal)count / scale).ToString("0.#########", CultureInfo.InvariantCulture) + suffix;
            }
        }
        return count.ToString(CultureInfo.InvariantCulture);
    }

    internal static string Rate(double value)
    {
        var (scale, suffix) = value >= 1_000_000_000 ? (1_000_000_000, "B") : value >= 1_000_000 ? (1_000_000, "M") : value >= 1_000 ? (1_000, "K") : (1, "");
        return (value / scale).ToString("0.##", CultureInfo.InvariantCulture) + suffix;
    }

    internal static uint? Parse(string text)
    {
        var value = text.Trim();
        if (!Regex.IsMatch(value, @"\A(?:[0-9]+|[0-9]{1,3}(?:,[0-9]{3})+)(?:\.[0-9]+)? *[kKmMbB]?\z")) return null;
        var power = char.ToUpperInvariant(value[^1]) switch { 'K' => 3, 'M' => 6, 'B' => 9, _ => 0 };
        if (power > 0) value = value[..^1];
        var parts = value.Trim().Replace(",", "").Split('.');
        var fraction = parts.Length == 2 ? parts[1].TrimEnd('0') : "";
        if (fraction.Length > power) return null;
        var digits = parts[0] + fraction + new string('0', power - fraction.Length);
        return uint.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out var count) && count > 0 ? count : null;
    }

    internal static bool ValidOptional(string value) => string.IsNullOrWhiteSpace(value) || Parse(value) != null;

    internal static bool ValidLimits(string context, string output) => ValidOptional(context) && ValidOptional(output)
        && !(Parse(context) is { } window && Parse(output) is { } maximum && maximum > window);
}
