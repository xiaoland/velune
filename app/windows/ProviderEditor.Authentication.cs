using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The editor's sign-in: an API key kept here, a subscription account signed
// in through the core's own flow, or an existing Claude Code login the
// provider refers to. The ARK Coding Plan also keeps separate usage query
// credentials.
sealed partial class ProviderEditor
{
    readonly PasswordBox key = new() { MinWidth = 280 };
    readonly Button reveal = new() { Content = "显示" };
    readonly TextBlock keyError = Ui.Error();
    readonly TextBlock signedIn = Ui.Muted("");
    readonly StackPanel signInActions = new() { Orientation = Orientation.Horizontal, Spacing = 8 };
    readonly ComboBox accountSource = new() { MinWidth = 280 };
    // The Claude Code logins accountSource offers after a new account: the
    // user's own, then each Claude Code runtime's.
    readonly List<BindingLinkedClaudeLogin> logins = [];
    readonly StackPanel quotaAuthentication = new() { Spacing = 4 };
    readonly TextBox quotaID = new() { MinWidth = 280 };
    readonly PasswordBox quotaSecret = new() { MinWidth = 280 };
    readonly CheckBox quotaConfirmed = new() { Content = "此次保存时确认此来源对应当前模型 Key 的个人 Coding Plan" };
    readonly TextBlock quotaStatus = Ui.Muted(""), quotaError = Ui.Error();
    readonly Button clearQuota = new() { Content = "清除额度查询认证" };
    bool clearingQuota;

    void BuildAuthentication()
    {
        switch (authenticationSource)
        {
            case BindingProviderAuthenticationSource.ApiKey:
                Section("认证");
                var keyRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                keyRow.Children.Add(key);
                keyRow.Children.Add(reveal);
                key.PasswordChanged += (_, _) => Changed();
                reveal.Click += (_, _) => RevealKey();
                body.Children.Add(Ui.Card(Ui.Field(Ui.TwoLines("API key", "保存在这台电脑的应用私有配置中。", true), "", keyError), keyRow));
                break;
            case BindingProviderAuthenticationSource.CodexOAuth:
            case BindingProviderAuthenticationSource.ClaudeCli:
            case BindingProviderAuthenticationSource.RuntimeProvider:
                Section("认证");
                if (authenticationSource == BindingProviderAuthenticationSource.ClaudeCli)
                {
                    BuildAccountSource();
                }
                var status = new StackPanel { Spacing = 2 };
                status.Children.Add(Ui.Text(authenticationSource switch
                {
                    BindingProviderAuthenticationSource.CodexOAuth => "ChatGPT 账户",
                    BindingProviderAuthenticationSource.ClaudeCli => "Claude 账户",
                    _ => "认证来源",
                }));
                status.Children.Add(signedIn);
                body.Children.Add(Ui.Card(status, signInActions));
                break;
        }
        if (quotaSource == BindingProviderQuotaSource.ArkCodingUsage)
        {
            BuildQuotaAuthentication();
        }
        ShowAuthentication();
    }

    // BuildAccountSource offers where a Claude Code provider's login comes
    // from: a new account signed in through Velune, or an existing login that
    // Claude Code keeps and renews, which Velune refers to without copying.
    void BuildAccountSource()
    {
        accountSource.Items.Add("在 Velune 中登录新账户");
        accountSource.Items.Add("本机 Claude Code 登录");
        logins.Add(new BindingLinkedClaudeLogin.UserDefault());
        foreach (var runtime in page.Runtimes)
        {
            if (page.RuntimeTypes.FirstOrDefault(type => type.Id == runtime.TypeId)?.FamilyId == "claude-code")
            {
                accountSource.Items.Add("Claude Code 运行时「" + runtime.Name + "」的登录");
                logins.Add(new BindingLinkedClaudeLogin.Runtime(runtime.Id));
            }
        }
        accountSource.SelectedIndex = SavedSource();
        accountSource.SelectionChanged += (_, _) =>
        {
            Changed();
            ShowAuthentication();
        };
        body.Children.Add(Ui.Card(Ui.TwoLines("账户来源", "沿用的登录由 Claude Code 保存与续期；Velune 不复制凭据，也不退出它。", true), accountSource));
    }

    // SavedSource is accountSource's entry for the saved login.
    int SavedSource() => existing?.Authentication.LinkedLogin is { } linked && logins.IndexOf(linked) is var index and >= 0 ? index + 1 : 0;

    bool SourceChanged => authenticationSource == BindingProviderAuthenticationSource.ClaudeCli && accountSource.SelectedIndex != SavedSource();

    // Linked tells whether the provider uses, or will use, an existing Claude Code login.
    bool Linked => authenticationSource == BindingProviderAuthenticationSource.ClaudeCli && accountSource.SelectedIndex > 0;

    // NeedsLogin tells whether saving a new provider should go on to sign in.
    bool NeedsLogin => authenticationSource == BindingProviderAuthenticationSource.CodexOAuth
        || (authenticationSource == BindingProviderAuthenticationSource.ClaudeCli && !Linked);

    void ShowAuthentication()
    {
        var authentication = existing?.Authentication;
        if (authenticationSource == BindingProviderAuthenticationSource.ApiKey)
        {
            var saved = authentication?.Configured ?? false;
            key.PlaceholderText = saved ? "已保存；留空保持不变" : "";
            reveal.Visibility = saved && key.PasswordRevealMode != PasswordRevealMode.Visible ? Visibility.Visible : Visibility.Collapsed;
            return;
        }
        signInActions.Children.Clear();
        if (existing == null || SourceChanged)
        {
            signedIn.Text = Linked ? "保存后沿用此登录。" : existing == null ? "保存后登录。" : "保存后需在 Velune 中重新登录。";
            if (NeedsLogin)
            {
                signInActions.Children.Add(Action("保存并登录…", Login));
            }
            return;
        }
        var label = authentication!.AccountLabel is { Length: > 0 } account ? "：" + account : "";
        var from = authentication.Provenance?.DisplayName is { Length: > 0 } provenance ? provenance + " · " : "";
        signedIn.Text = from + (authentication.Configured
            ? (authentication.Method == BindingAuthenticationMethod.OAuth ? "已登录" : "已配置") + label
            : authentication.Method == BindingAuthenticationMethod.OAuth ? "未登录" : "未配置");
        foreach (var action in authentication.Actions)
        {
            // Only a signed-in account signs out.
            if (action.Id == "logout" && !authentication.Configured)
            {
                continue;
            }
            // An action this app does not know is not offered.
            Action? chosen = action.Id switch
            {
                "login" => Login,
                "logout" => Logout,
                "inspect" => Inspect,
                _ => null,
            };
            if (chosen != null)
            {
                signInActions.Children.Add(Action(action.Id == "logout" ? "退出登录…" : action.Label, chosen));
            }
        }
    }

    static Button Action(string label, Action chosen)
    {
        var button = new Button { Content = label };
        button.Click += (_, _) => chosen();
        return button;
    }

    BindingAuthenticationEdit AuthenticationEdit()
    {
        if (authenticationSource == BindingProviderAuthenticationSource.ApiKey && key.Password.Trim() is { Length: > 0 } value)
        {
            return new BindingAuthenticationEdit.SetApiKey(value);
        }
        if (SourceChanged)
        {
            return accountSource.SelectedIndex > 0
                ? new BindingAuthenticationEdit.UseClaudeCodeLogin(logins[accountSource.SelectedIndex - 1])
                : new BindingAuthenticationEdit.Clear();
        }
        return new BindingAuthenticationEdit.Keep();
    }

    // RevealKey reads the saved key into the field to show and edit it.
    async void RevealKey()
    {
        if (existing is not { } provider)
        {
            return;
        }
        try
        {
            var saved = await Task.Run(() => page.Core.Local.ReadProviderApiKey(ProvidersPage.DefaultGateway, provider.Id));
            building = true;
            key.Password = saved;
            building = false;
            key.PasswordRevealMode = PasswordRevealMode.Visible;
            reveal.Visibility = Visibility.Collapsed;
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    // Login saves what changed, then signs in through the core's flow.
    async void Login()
    {
        if ((existing == null || dirty) && !await Save())
        {
            return;
        }
        if (existing is not { } provider)
        {
            return;
        }
        try
        {
            Saved(await ProviderFlows.Login(page.Core, page.Window, provider.Id, provider.Name));
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    async void Logout()
    {
        if (existing is not { } provider
            || !await Ui.Confirm(page.Window, "退出登录？", "「" + provider.Name + "」需要重新登录后才能使用。", "退出登录"))
        {
            return;
        }
        try
        {
            Saved((await Task.Run(() => page.Core.Local.AuthenticationLogout(ProvidersPage.DefaultGateway, provider.Id))).Gateways);
            Ui.Show(failure, "");
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    // Inspect asks the login's owner for its state, then shows what the core keeps.
    async void Inspect()
    {
        if (existing is not { } provider)
        {
            return;
        }
        try
        {
            var gateways = await Task.Run(() =>
            {
                page.Core.Local.AuthenticationInspect(ProvidersPage.DefaultGateway, provider.Id);
                return page.Core.Local.List().Gateways;
            });
            Saved(gateways);
            Ui.Show(failure, "");
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    // BuildQuotaAuthentication lays out the ARK Coding Plan's usage query
    // credentials, which the user links to the model key; the account is not checked.
    void BuildQuotaAuthentication()
    {
        Section("额度查询认证");
        body.Children.Add(Ui.Card(Ui.TwoLines("状态", "模型 API key 与此处 AK/SK 分别用于模型和额度查询。账户归属未自动核验。", true), quotaStatus));
        body.Children.Add(Ui.Card(Ui.Text("AccessKey ID"), quotaID));
        body.Children.Add(Ui.Card(Ui.Field(Ui.TwoLines("SecretAccessKey", "输入完整 AK/SK 替换；留空保留。改模型 Key 后需再次确认，有上限且未确认时暂停派发。上限按此来源观测执行，不是计费硬预算。", true), "", quotaError), quotaSecret));
        var confirm = new StackPanel { Spacing = 8 };
        confirm.Children.Add(quotaConfirmed);
        confirm.Children.Add(clearQuota);
        body.Children.Add(Ui.Card(confirm));
        quotaID.TextChanged += (_, _) => Changed();
        quotaSecret.PasswordChanged += (_, _) => Changed();
        quotaConfirmed.Click += (_, _) => Changed();
        clearQuota.Click += (_, _) =>
        {
            quotaID.Text = "";
            quotaSecret.Password = "";
            quotaConfirmed.IsChecked = false;
            clearingQuota = true;
            Changed();
            ShowQuotaAuthentication();
        };
        ShowQuotaAuthentication();
    }

    void ShowQuotaAuthentication()
    {
        if (quotaSource != BindingProviderQuotaSource.ArkCodingUsage)
        {
            return;
        }
        var described = existing?.QuotaAuthentication ?? new BindingQuotaAuthenticationDescription(false, false);
        quotaStatus.Text = clearingQuota ? "保存后清除"
            : described is { Configured: true, BindingConfirmed: true } ? "已由用户确认关联"
            : described.Configured ? "需要重新确认关联"
            : "尚未配置";
        clearQuota.Visibility = described.Configured && !clearingQuota ? Visibility.Visible : Visibility.Collapsed;
    }

    bool QuotaIncomplete()
    {
        var incomplete = quotaSource == BindingProviderQuotaSource.ArkCodingUsage
            && (quotaID.Text.Trim().Length == 0) != (quotaSecret.Password.Trim().Length == 0);
        Ui.Show(quotaError, incomplete ? "额度查询需要完整 AK/SK" : "");
        return incomplete;
    }

    // QuotaEdit is what saving does to the usage query credentials; null keeps them.
    BindingQuotaAuthenticationEdit? QuotaEdit()
    {
        var (id, secret) = (quotaID.Text.Trim(), quotaSecret.Password.Trim());
        var confirmed = quotaConfirmed.IsChecked == true;
        if (clearingQuota)
        {
            clearingQuota = false;
            return new BindingQuotaAuthenticationEdit.Clear();
        }
        if (id.Length > 0 && secret.Length > 0)
        {
            return new BindingQuotaAuthenticationEdit.SetAccessKey(id, secret, confirmed);
        }
        return confirmed ? new BindingQuotaAuthenticationEdit.ConfirmBinding() : null;
    }
}
