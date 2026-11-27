using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// One model's policy for a single runtime. Each save edits that entry independently.
static class ModelCompactionDialog
{
    public static bool Supports(RuntimeKind kind) => kind is RuntimeKind.Codex166 or RuntimeKind.ClaudeCode or RuntimeKind.Pi or RuntimeKind.DeepSeek;

    public static async Task ShowManaged(Core core, SettingsWindow window, string recordKey, string modelName, Func<bool> canEdit)
    {
        var snapshot = await core.Call(Device.Local, NativeOperation.ConfigurationRead, app => app.List());
        if (!canEdit()) return;
        var model = new ModelSelection.Managed(recordKey);
        var gateway = snapshot.Gateways.FirstOrDefault(item => item.Id == ProvidersPage.DefaultGateway);
        var providerKey = gateway?.Routes.FirstOrDefault(route => route.RecordKey == recordKey)?.Members.FirstOrDefault() ?? recordKey;
        var protocol = gateway?.Providers.FirstOrDefault(provider => provider.Models.Any(item => item.RecordKey == providerKey))?.Protocol;
        bool Compatible(RuntimeInstance runtime) => runtime.GatewayId == ProvidersPage.DefaultGateway && protocol is { } value
            && snapshot.RuntimeTypes.Any(kind => kind.Id == runtime.TypeId && kind.SupportedProviderProtocols.Contains(value));
        var runtimes = snapshot.RuntimeInstances.Where(runtime => Supports(runtime.TypeId) && (Compatible(runtime)
            || VeluneBindingsMethods.RuntimeModelAutoCompactions(runtime).Any(entry => Equals(entry.Model, model)))).ToArray();
        await Show(window.ShowDialog, modelName, model, runtimes, canEdit, (runtime, setting) =>
            core.Call(Device.Local, NativeOperation.ConfigurationWrite, app => app.SetRuntimeModelAutoCompaction(runtime.Id, model, setting)), Compatible);
    }

    public static async Task Show(
        Func<ContentDialog, Task<ContentDialogResult>> show, string modelName, ModelSelection model,
        RuntimeInstance[] runtimes, Func<bool> canEdit, Func<RuntimeInstance, AutoCompactionSetting?, Task> save,
        Func<RuntimeInstance, bool>? compatible = null)
    {
        if (!canEdit()) return;
        var body = new StackPanel { Spacing = 8 };
        var dialog = new ContentDialog { Title = "自动压缩 · " + modelName, Content = body, CloseButtonText = "关闭" };
        if (runtimes.Length == 0)
        {
            body.Children.Add(Ui.Muted("这个模型当前没有支持自动压缩设置的运行时。"));
            await show(dialog);
            return;
        }

        var chooseRuntime = new ComboBox { Header = "适用运行时", HorizontalAlignment = HorizontalAlignment.Stretch };
        var context = new TextBlock { TextWrapping = TextWrapping.Wrap };
        var input = new TextBox { PlaceholderText = "运行时默认" };
        var detail = Ui.Muted("");
        var error = Ui.Error();
        var policies = runtimes.Select(runtime => VeluneBindingsMethods.RuntimeModelAutoCompactions(runtime)
            .FirstOrDefault(entry => Equals(entry.Model, model))?.Setting).ToArray();
        foreach (var (runtime, index) in runtimes.Select((runtime, index) => (runtime, index)))
        {
            chooseRuntime.Items.Add(runtime.Name + (policies[index] != null ? " · 已配置" : ""));
        }
        if (runtimes.Length > 1) body.Children.Add(chooseRuntime);
        else body.Children.Add(context);
        body.Children.Add(input);
        body.Children.Add(detail);
        body.Children.Add(error);

        void Update()
        {
            var index = chooseRuntime.SelectedIndex;
            if (index < 0 || index >= runtimes.Length) return;
            var runtime = runtimes[index];
            context.Text = "适用运行时：" + runtime.Name;
            (input.Header, detail.Text) = runtime.TypeId switch
            {
                RuntimeKind.Codex166 => ("压缩触发阈值（Token）", "Codex 会按实际模型窗口限制生效阈值。"),
                RuntimeKind.ClaudeCode => ("自动压缩窗口（Token）", "接受 100K 到 1M Token。Claude Code 会为输出与摘要预留空间。"),
                RuntimeKind.Pi => ("压缩预留量（Token）", "上下文剩余空间小于预留量时触发。"),
                RuntimeKind.DeepSeek => ("压缩触发比例（%）", "接受 1 到 100 的整数，须高于模型的保留比例。"),
                _ => ("", ""),
            };
            detail.Text = compatible?.Invoke(runtime) == false
                ? "此运行时当前无法使用该模型。留空可清除已保存的设置。"
                : detail.Text + " 留空恢复运行时默认，保存后从后续轮次生效。";
            input.Text = Format(policies[index]);
            Ui.Show(error, "");
        }

        chooseRuntime.SelectionChanged += (_, _) => Update();
        var saved = Array.FindIndex(policies, policy => policy != null);
        chooseRuntime.SelectedIndex = Math.Max(saved, 0);
        // A single runtime doesn't attach its ComboBox to the visual tree.
        Update();
        input.TextChanged += (_, _) => Ui.Show(error, "");
        dialog.PrimaryButtonText = "保存";
        dialog.CloseButtonText = "取消";
        dialog.PrimaryButtonClick += async (_, e) =>
        {
            var deferral = e.GetDeferral();
            try
            {
                if (!canEdit())
                {
                    e.Cancel = true;
                    Ui.Show(error, "当前模型已不可编辑。请关闭后重新打开。");
                    return;
                }
                var runtime = runtimes[chooseRuntime.SelectedIndex];
                var value = input.Text.Trim();
                if (value.Length > 0 && compatible?.Invoke(runtime) == false)
                {
                    e.Cancel = true;
                    Ui.Show(error, "请清空这一设置，或为该模型选择其他适用运行时。");
                    return;
                }
                var setting = Parse(runtime.TypeId, value);
                if (value.Length > 0 && setting == null)
                {
                    e.Cancel = true;
                    Ui.Show(error, runtime.TypeId == RuntimeKind.DeepSeek ? "请输入 1 到 100 的整数百分比。" : "请输入正整数 Token 数，可用 K、M 或 B。");
                    return;
                }
                await save(runtime, setting);
            }
            catch (Exception failure)
            {
                e.Cancel = true;
                if (canEdit()) Ui.Show(error, Core.Describe(failure));
            }
            finally { deferral.Complete(); }
        };
        await show(dialog);
    }

    static string Format(AutoCompactionSetting? setting) => setting switch
    {
        AutoCompactionSetting.Codex codex => TokenCount.Format(codex.TokenLimit),
        AutoCompactionSetting.Claude claude => TokenCount.Format(claude.WindowTokens),
        AutoCompactionSetting.Pi pi => TokenCount.Format(pi.ReserveTokens),
        AutoCompactionSetting.DeepSeek deepSeek => deepSeek.ThresholdPercent.ToString(CultureInfo.InvariantCulture),
        _ => "",
    };

    static AutoCompactionSetting? Parse(RuntimeKind kind, string text)
    {
        if (kind == RuntimeKind.DeepSeek)
        {
            var value = text.EndsWith('%') ? text[..^1].TrimEnd() : text;
            return byte.TryParse(value, NumberStyles.None, CultureInfo.InvariantCulture, out var percent) && percent is >= 1 and <= 100
                ? new AutoCompactionSetting.DeepSeek(percent) : null;
        }
        if (TokenCount.Parse(text) is not { } tokens) return null;
        return kind switch
        {
            RuntimeKind.Codex166 => new AutoCompactionSetting.Codex(tokens),
            RuntimeKind.ClaudeCode => new AutoCompactionSetting.Claude(tokens),
            RuntimeKind.Pi => new AutoCompactionSetting.Pi(tokens),
            _ => null,
        };
    }
}
