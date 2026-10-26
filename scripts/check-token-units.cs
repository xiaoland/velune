using Velune;

static void Check(bool condition) { if (!condition) throw new Exception("Token 单位检查失败"); }
(uint Count, string Text)[] examples = [(0, "0"), (999, "999"), (1000, "1K"), (8192, "8.192K"), (272000, "272K"), (999999, "999.999K"), (1000000, "1M"), (1500000, "1.5M"), (100000000, "100M"), (uint.MaxValue, "4.294967295B")];
foreach (var (count, text) in examples)
{
    Check(TokenCount.Format(count) == text);
    if (count > 0) Check(TokenCount.Parse(text) == count);
}
(string Text, uint Count)[] inputs = [(" 1.5m ", 1500000), ("1 k", 1000), ("1,000", 1000), ("100,000,000", 100000000), ("0.001K", 1), ("1.0000", 1), ("4.294967295B", uint.MaxValue)];
foreach (var (text, count) in inputs) Check(TokenCount.Parse(text) == count);
foreach (var text in new[] { "", "0", "-1", "+1", "1.2", "0.0001K", "1e3", "1KiB", "1,5M", "1,00", "NaN", "∞", "4.294967296B", "4294967296", "1.00000000000000000001M" }) Check(TokenCount.Parse(text) == null);
for (ulong count = 1; count <= uint.MaxValue; count += 7919) Check(TokenCount.Parse(TokenCount.Format(count)) == count);
Check(TokenCount.Format(ulong.MaxValue) == "18446744073.709551615B");
Check(TokenCount.ValidLimits(" ", ""));
Check(TokenCount.ValidLimits("1M", "32K"));
Check(!TokenCount.ValidLimits("1K", "1M"));
Check(!TokenCount.ValidLimits("wrong", ""));
Check(TokenCount.Rate(1500) == "1.5K");
Console.WriteLine("C# token 单位与无损回读检查通过");
