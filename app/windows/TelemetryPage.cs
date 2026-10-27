using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;
using Windows.ApplicationModel.DataTransfer;

namespace Velune;

sealed class TelemetryPage : UserControl
{
    readonly Core core;
    readonly Hub hub;
    readonly SettingsWindow window;
    readonly StackPanel panel = new() { Spacing = 8, Padding = new Thickness(24), MaxWidth = 900 };
    readonly ComboBox preference = new() { Items = { "未设置", "关闭", "开启" } };
    readonly TextBox endpoint = new() { PlaceholderText = "http://localhost:4317" };
    readonly Slider ratio = new() { Minimum = 0, Maximum = 100, StepFrequency = 1, Value = 100, MinWidth = 160 };
    readonly StackPanel headers = new() { Spacing = 8 };
    readonly List<(TextBox Name, PasswordBox Value)> metadata = [];
    readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
    readonly InfoBar failure = Ui.FailureBar();
    readonly ComboBox source = new() { DisplayMemberPath = "Name", SelectedValuePath = "PeerId", PlaceholderText = "选择设备", MinWidth = 180 };
    readonly StackPanel sharing = new() { Spacing = 8 };
    readonly TextBox invitation = new() { IsReadOnly = true, TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
    readonly DispatcherTimer refresh = new() { Interval = TimeSpan.FromSeconds(2) };
    ulong revision;
    string? sharingKey;
    bool visible;
    bool busy;
    int display;

    internal TelemetryPage(Core core, Hub hub, SettingsWindow window)
    {
        this.core = core; this.hub = hub; this.window = window;
        panel.Children.Add(Ui.Text("遥测", "SubtitleTextBlockStyle"));
        panel.Children.Add(Ui.Card(Ui.Text("本机遥测"), preference));
        panel.Children.Add(Ui.Card(Ui.Text("OTLP/gRPC 地址"), endpoint));
        panel.Children.Add(Ui.Card(Ui.Text("新链路采样率（%）"), ratio));
        panel.Children.Add(headers);
        panel.Children.Add(Button("添加认证字段", () => { if (metadata.Count < 16) AddHeader("authorization", ""); }));
        panel.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children =
        {
            Button("保存", () => Run(async () =>
            {
                var profile = Profile(); var expected = revision; var choice = (BindingTelemetryPreference)preference.SelectedIndex;
                var state = await Task.Run(() => { core.Telemetry.Save(expected, choice, profile); return core.Telemetry.Binding.Snapshot(); });
                if (visible) { revision = state.Revision; Show(state); }
            })),
            Button("发送测试事件", () => Run(async () =>
            {
                var profile = Profile() ?? throw new InvalidOperationException("请填写遥测地址");
                var result = await Task.Run(() => core.Telemetry.Binding.SendTest(profile));
                if (visible) status.Text = $"Traces：{TestText(result.Traces)}；Logs：{TestText(result.Logs)}；Metrics：{TestText(result.Metrics)}";
            })),
        }});
        panel.Children.Add(status); panel.Children.Add(failure);
        panel.Children.Add(Ui.Heading("从设备复制")); panel.Children.Add(source);
        panel.Children.Add(Button("复制遥测配置（含认证）", () => Run(async () =>
        {
            var peer = source.SelectedValue as string ?? throw new InvalidOperationException("请选择来源设备");
            var expected = revision;
            var result = await Task.Run(() =>
            {
                var profile = core.Remote().ReadTelemetryProfile(peer) ?? throw new InvalidOperationException("来源设备尚未保存遥测配置");
                core.Telemetry.Save(expected, core.Telemetry.Binding.Snapshot().Preference, profile);
                return (core.Telemetry.Binding.Snapshot(), profile);
            });
            if (!visible) return;
            Fill(result.Item1, result.profile);
            if (Uri.TryCreate(result.profile.Endpoint, UriKind.Absolute, out var uri) && uri.IsLoopback)
                status.Text = "已复制；来源地址为回环地址，请调整并测试";
        })));
        panel.Children.Add(Ui.Heading("允许读取本机配置（含认证）"));
        panel.Children.Add(sharing);
        panel.Children.Add(Button("生成遥测分享邀请", () => Run(async () =>
        {
            var created = await Task.Run(() => core.Remote().CreateTelemetryInvitation(300));
            if (visible) { invitation.Text = created.Encoded; invitation.Visibility = Visibility.Visible; }
        })));
        panel.Children.Add(invitation);
        panel.Children.Add(Button("复制邀请", () =>
        {
            if (invitation.Text.Length == 0) return;
            try { var data = new DataPackage(); data.SetText(invitation.Text); Clipboard.SetContent(data); }
            catch (Exception problem) { Ui.Show(failure, Core.Describe(problem)); }
        }));
        Content = new ScrollViewer { Content = panel };
        Loaded += (_, _) => { visible = true; display++; Load(); refresh.Start(); };
        Unloaded += (_, _) => { visible = false; display++; refresh.Stop(); metadata.Clear(); headers.Children.Clear(); invitation.Text = ""; };
        refresh.Tick += (_, _) => Refresh();
    }

    internal void Stop() { visible = false; display++; refresh.Stop(); }

    static Button Button(string name, Action action)
    {
        var button = new Button { Content = name };
        button.Click += (_, _) => action(); return button;
    }
    void AddHeader(string name, string value)
    {
        var key = new TextBox { Text = name, PlaceholderText = "Metadata 名称", MinWidth = 140 };
        var secret = new PasswordBox { Password = value, PlaceholderText = "值", MinWidth = 180 };
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        row.Children.Add(key); row.Children.Add(secret);
        row.Children.Add(Button("移除", () => { metadata.Remove((key, secret)); headers.Children.Remove(row); }));
        metadata.Add((key, secret)); headers.Children.Add(row);
    }
    BindingTelemetryProfile? Profile()
    {
        if (endpoint.Text.Trim().Length == 0) return null;
        var values = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var (key, secret) in metadata)
            if (!values.TryAdd(key.Text, secret.Password)) throw new InvalidOperationException("认证字段名称不能重复");
        return new BindingTelemetryProfile(endpoint.Text.Trim(), values, ratio.Value / 100);
    }
    void Load() => Run(async () =>
    {
        var result = await Task.Run(() => (core.Telemetry.Binding.Snapshot(), core.Telemetry.Binding.ProfileForEditing()));
        if (visible) Fill(result.Item1, result.Item2);
        RenderSharing();
    });
    void Fill(BindingTelemetrySnapshot state, BindingTelemetryProfile? profile)
    {
        revision = state.Revision; preference.SelectedIndex = (int)state.Preference;
        endpoint.Text = profile?.Endpoint ?? state.Endpoint ?? ""; ratio.Value = (profile?.SampleRatio ?? state.SampleRatio) * 100;
        metadata.Clear(); headers.Children.Clear();
        foreach (var (key, value) in profile?.Headers ?? []) AddHeader(key, value);
        Show(state);
    }
    async void Run(Func<Task> action)
    {
        if (busy || !visible) return;
        var ticket = display; busy = true; IsEnabled = false; Ui.Show(failure, "");
        try { await action(); }
        catch (Exception problem) { if (visible && ticket == display) Ui.Show(failure, Core.Describe(problem)); }
        finally { busy = false; if (visible) IsEnabled = true; }
    }
    async void Refresh()
    {
        if (busy || !visible) return;
        var ticket = display;
        try
        {
            var state = await Task.Run(() => core.Telemetry.Binding.Snapshot());
            if (!visible || ticket != display || busy) return;
            Show(state); RenderSharing();
        }
        catch (Exception problem) { if (visible && ticket == display) Ui.Show(failure, Core.Describe(problem)); }
    }
    void Show(BindingTelemetrySnapshot state)
    {
        status.Text = state.Status == "development_configuration_unavailable" ? "开发启动配置不可用，遥测未接入"
            : !state.ConfigurationAvailable ? "配置无法读取，遥测未启用；原文件已保留"
            : !state.Applied ? "已保存，尚未应用"
            : state.RustActiveRevision == null ? "已关闭"
            : $"{(state.DevelopmentOverride ? "开发启动配置 · " : "")}已应用 · 修订 {state.Revision} · Rust：{Health(state.RustExport)} · Windows：{Health(state.NativeExport)}";
    }
    static string Health(BindingTelemetryExportHealth health) => health.LastTransportResult switch { "rpc_completed" => "最近 RPC 完成", null => "尚无回执", _ => "最近导出失败" };
    static string TestText(string result) => result switch { "accepted" => "接收端已接受", "partially_rejected" => "部分拒绝", "unauthenticated" or "permission_denied" => "认证被拒绝", "timeout" => "超时", "unavailable" => "无法连接", _ => "被拒绝" };
    void RenderSharing()
    {
        if (!visible || hub.State is not { } state) return;
        var key = JsonSerializer.Serialize(new { state.Contacts, state.IncomingGrants, state.TelemetryProfileGrants, state.IncomingPairings });
        if (sharingKey == key) return;
        sharingKey = key;
        var selected = source.SelectedValue;
        source.ItemsSource = state.Contacts.Where(contact => contact.Confirmation is BindingContactConfirmation.Confirmed).ToArray();
        source.SelectedValue = selected;
        var names = state.Contacts.Where(contact => contact.Confirmation is BindingContactConfirmation.Confirmed).ToDictionary(contact => contact.PeerId, contact => contact.Name);
        foreach (var grant in state.IncomingGrants.Concat(state.TelemetryProfileGrants)) names[grant.PeerId] = grant.Name;
        var allowed = state.TelemetryProfileGrants.Select(grant => grant.PeerId).ToHashSet();
        sharing.Children.Clear();
        foreach (var (peer, name) in names.OrderBy(pair => pair.Value))
        {
            var granted = allowed.Contains(peer);
            sharing.Children.Add(Ui.Card(Ui.Text(name), Button(granted ? "撤销" : "允许", () => Run(async () =>
            {
                await Task.Run(() => { if (granted) core.Remote().RevokeTelemetryProfile(peer); else core.Remote().AllowTelemetryProfile(peer); });
            }))));
        }
        foreach (var pairing in state.IncomingPairings.Where(pairing => pairing.Permission == BindingPairingPermission.TelemetryProfileRead && pairing.Phase == BindingPairingPhase.AwaitingApproval))
        {
            invitation.Text = "";
            sharing.Children.Add(Ui.Card(Ui.TwoLines(pairing.Name + " · 遥测配置（含认证）", Hub.FormatCode(pairing.VerificationCode)),
                new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children =
                {
                    Button("核对一致，允许", () => Run(async () => { await Task.Run(() => core.Remote().ApprovePairing(pairing.AttemptId, pairing.PeerId)); })),
                    Button("拒绝", () => Run(async () => { await Task.Run(() => core.Remote().RejectPairing(pairing.AttemptId, pairing.PeerId)); })),
                }}));
        }
    }
}
