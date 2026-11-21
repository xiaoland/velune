using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// A local facility editor. The core owns persistence, credentials and network generations.
sealed class RemoteNetworkSettings : UserControl
{
    readonly Core core;
    readonly StackPanel panel = new() { Spacing = 10, MinWidth = 360, MaxWidth = 600 };
    readonly ComboBox mode = new() { Items = { "公共中继", "自定义中继", "关闭中继" }, SelectedIndex = 0 };
    readonly TextBox urls = new() { Header = "中继地址，每行一个", PlaceholderText = "https://relay.example.com/", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 72 };
    readonly StackPanel credentials = new() { Spacing = 8 };
    readonly List<CredentialRow> entries = [];
    readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
    readonly InfoBar failure = Ui.FailureBar();
    readonly DispatcherTimer refresh = new() { Interval = TimeSpan.FromSeconds(2) };
    readonly Button save;
    readonly Button retry;
    readonly Button add;
    NetworkState? loaded;
    bool refreshing;
    internal bool Busy { get; private set; }

    internal RemoteNetworkSettings(Core core)
    {
        this.core = core;
        panel.Children.Add(Ui.Muted("用于这台电脑的远程连接。两端均需配置受限中继的访问凭据；地址查找继续使用现有设置。"));
        panel.Children.Add(mode);
        panel.Children.Add(urls);
        mode.SelectionChanged += (_, _) => urls.Visibility = mode.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
        urls.Visibility = Visibility.Collapsed;
        panel.Children.Add(Ui.Muted("关闭中继后，需要能直连对方设备。"));
        panel.Children.Add(Ui.Heading("中继访问凭据"));
        panel.Children.Add(Ui.Muted("凭据只发往指定地址。留空保持已有令牌；移除须勾选后保存。中继候选与访问资格分别设置。"));
        panel.Children.Add(credentials);
        add = Button("添加凭据", () => { AddCredential("", false); UpdateButtons(); });
        panel.Children.Add(add);
        save = Button("保存并应用", Save);
        retry = Button("重试已保存设置", () => Run(async () =>
        {
            var state = await Task.Run(() => core.Remote().RetryNetwork());
            if (IsLoaded) Show(state);
        }));
        panel.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { save, retry } });
        panel.Children.Add(Button("重新载入（丢弃编辑）", Reload));
        panel.Children.Add(status);
        Content = new StackPanel { Spacing = 8, Children = { failure, new ScrollViewer { Content = panel, MaxHeight = 560 } } };
        Loaded += (_, _) => { if (loaded == null) Reload(); refresh.Start(); };
        Unloaded += (_, args) =>
        {
            // Child controls can unload while this editor remains mounted.
            if (IsLoaded && !ReferenceEquals(args.OriginalSource, this)) return;
            refresh.Stop(); foreach (var entry in entries) entry.Token.Password = "";
        };
        refresh.Tick += (_, _) => Refresh();
        UpdateButtons();
    }

    static Button Button(string title, Action action)
    {
        var button = new Button { Content = title };
        button.Click += (_, _) => action();
        return button;
    }

    void Reload() => Run(async () =>
    {
        var state = await Task.Run(() => core.Remote().ReadNetworkState());
        if (IsLoaded) Fill(state);
    });

    void Fill(NetworkState state)
    {
        loaded = state;
        mode.SelectedIndex = state.Saved?.Relay switch { RelayConfig.Custom => 1, RelayConfig.Disabled => 2, _ => 0 };
        urls.Text = state.Saved?.Relay is RelayConfig.Custom custom ? string.Join("\n", custom.Urls) : "";
        foreach (var entry in entries) entry.Token.Password = "";
        entries.Clear(); credentials.Children.Clear();
        foreach (var entry in state.Saved?.Credentials ?? []) AddCredential(entry.Url, entry.Configured);
        Show(state); UpdateButtons();
    }

    void AddCredential(string url, bool configured)
    {
        var entry = new CredentialRow(url, configured);
        entries.Add(entry); credentials.Children.Add(entry.Panel);
    }

    void Save() => Run(async () =>
    {
        var selected = mode.SelectedIndex;
        var addresses = urls.Text.Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).ToList();
        if (selected == 1 && addresses.Count == 0) throw new InvalidOperationException("请填写至少一个中继地址");
        RelayConfig relay = selected switch { 1 => new RelayConfig.Custom([.. addresses]), 2 => new RelayConfig.Disabled(), _ => new RelayConfig.DevelopmentPublic() };
        var edits = new List<RelayCredentialEdit>();
        foreach (var entry in entries)
        {
            var remove = entry.Remove.IsChecked == true;
            if (remove && !entry.Configured) continue;
            CredentialEdit edit = remove ? new CredentialEdit.Remove()
                : entry.Token.Password.Length > 0 ? new CredentialEdit.Set(entry.Token.Password)
                : entry.Configured ? new CredentialEdit.Keep()
                : throw new InvalidOperationException("请填写新凭据的 Bearer 令牌，或移除该条目");
            edits.Add(new RelayCredentialEdit(entry.Url.Text.Trim(), edit));
        }
        var expected = loaded?.Saved?.Revision ?? 0;
        var state = await Task.Run(() => core.Remote().ApplyRelaySettings(expected, relay, [.. edits]));
        if (IsLoaded) Fill(state);
    });

    // Neither status refresh nor retry changes the form's revision or discards edits.
    async void Refresh()
    {
        if (!IsLoaded) { refresh.Stop(); return; }
        if (Busy || refreshing) return;
        refreshing = true;
        try
        {
            var state = await Task.Run(() => core.Remote().ReadNetworkState());
            if (IsLoaded && !Busy) Show(state);
        }
        catch (Exception problem) { if (IsLoaded) Ui.Show(failure, Core.Describe(problem)); }
        finally { refreshing = false; }
    }

    async void Run(Func<Task> body)
    {
        if (!IsLoaded || Busy) return;
        Busy = true; IsEnabled = false; Ui.Show(failure, "");
        try { await body(); }
        catch (Exception problem)
        {
            Problems.Report("本机连接设置", problem);
            if (IsLoaded) Ui.Show(failure, Core.Describe(problem));
        }
        finally
        {
            Busy = false;
            if (IsLoaded) { IsEnabled = true; UpdateButtons(); }
        }
    }

    void UpdateButtons()
    {
        save.IsEnabled = loaded != null;
        retry.IsEnabled = loaded != null;
        add.IsEnabled = entries.Count < 64;
    }

    void Show(NetworkState state)
    {
        var phase = state.Phase switch
        {
            NetworkApplicationPhase.Unapplied => "尚未应用",
            NetworkApplicationPhase.Applying => "正在应用已保存设置",
            NetworkApplicationPhase.Applied => "已保存的设置已应用；设施可达状态如下",
            _ => "设置已保存，应用失败，可重试" + (state.ErrorCode is { } code ? $"（{code}）" : ""),
        };
        status.Text = string.Join("\n", new[] { phase }.Concat(state.Facilities.Select(facility =>
        {
            var kind = facility.Kind == FacilityKind.HomeRelay ? "中继" : "地址查找";
            var health = facility.Reachability switch
            {
                FacilityReachability.Connecting => "连接中",
                FacilityReachability.Reachable => "可达",
                FacilityReachability.Unavailable => "不可达",
                FacilityReachability.AuthenticationDenied => "认证被拒绝",
                _ => "尚未确认",
            };
            return $"{kind} {facility.Url}：{health}";
        })));
    }

    sealed class CredentialRow
    {
        internal readonly StackPanel Panel = new() { Spacing = 6 };
        internal readonly TextBox Url;
        internal readonly PasswordBox Token;
        internal readonly CheckBox Remove = new() { Content = "移除此凭据" };
        internal readonly bool Configured;
        internal CredentialRow(string url, bool configured)
        {
            Configured = configured;
            Url = new TextBox { Header = "认证服务地址", Text = url, IsReadOnly = configured, PlaceholderText = "https://relay.example.com/" };
            Token = new PasswordBox { Header = configured ? "新 Bearer 令牌（留空保持）" : "Bearer 令牌" };
            Remove.Checked += (_, _) => { Token.IsEnabled = false; Url.IsEnabled = false; };
            Remove.Unchecked += (_, _) => { Token.IsEnabled = true; Url.IsEnabled = true; };
            Panel.Children.Add(Url); Panel.Children.Add(Token); Panel.Children.Add(Remove);
        }
    }
}
