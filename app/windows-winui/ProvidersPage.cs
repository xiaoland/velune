using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// This PC's gateway providers. HTTP templates retain their composition while
// API keys stay in the application's private configuration.
sealed class ProvidersPage : UserControl
{
    const string DefaultGateway = "default";

    readonly Core core;
    readonly SettingsWindow window;
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    readonly InfoBar failure = Ui.FailureBar();
    readonly Button remove = Ui.IconButton("\uE738", "删除 AI 提供商");
    readonly Button edit = new() { Content = "编辑…" };
    BindingProviderDefinition[] providers = [];
    BindingProtocolDescriptor[] protocols = [];
    BindingProviderTemplate[] templates = [];
    bool busy;

    internal ProvidersPage(Core core, SettingsWindow window)
    {
        this.core = core;
        this.window = window;
        var add = Ui.IconButton("\uE710", "添加 AI 提供商");
        add.Click += (_, _) => Edit(null);
        remove.Click += (_, _) => Delete();
        edit.Click += (_, _) => Edit(Chosen());
        list.SelectionChanged += (_, _) => ShowActions();
        list.DoubleTapped += (_, _) => Edit(Chosen());
        list.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter && Chosen() != null)
            {
                e.Handled = true;
                Edit(Chosen());
            }
        };
        Content = Ui.ListPage("AI提供商", failure, list, [add, remove], [edit]);
        Reload();
    }

    BindingProviderDefinition? Chosen() => list.SelectedIndex is var index and >= 0 && index < providers.Length ? providers[index] : null;

    void ShowActions()
    {
        remove.IsEnabled = Chosen() != null && !busy;
        edit.IsEnabled = Chosen() != null;
    }

    async void Reload()
    {
        try
        {
            var (snapshot, all) = await Task.Run(() => (core.Local.List(), VeluneBindingsMethods.ProviderTemplates()));
            templates = [.. all.Where(template => template.ExecutionBackend is BindingExecutionBackend.Http && template.AuthenticationSource == BindingProviderAuthenticationSource.ApiKey)];
            protocols = [.. snapshot.Protocols.Where(protocol => protocol.Supported)];
            SetGateway(snapshot.Gateways);
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
    }

    void SetGateway(BindingGatewayConfig[] gateways)
    {
        var selected = Chosen()?.Id;
        providers = gateways.FirstOrDefault(gateway => gateway.Id == DefaultGateway)?.Providers ?? [];
        list.Items.Clear();
        foreach (var provider in providers)
        {
            var row = Ui.TwoLines(provider.Name, $"{provider.Models.Length} 个模型 · {provider.Endpoint}");
            row.Padding = new Thickness(4, 6, 4, 6);
            var item = new ListViewItem { Content = row };
            item.ContextFlyout = Ui.Menu(
                Ui.MenuItem("编辑…", () => Edit(provider)),
                Ui.MenuItem("删除…", () =>
                {
                    list.SelectedItem = item;
                    Delete();
                }));
            list.Items.Add(item);
        }
        list.SelectedIndex = Array.FindIndex(providers, provider => provider.Id == selected) is var index and >= 0
            ? index
            : Math.Min(list.SelectedIndex, providers.Length - 1);
        Ui.Placeholder(list, "还没有 AI 提供商。用＋添加。");
        ShowActions();
    }

    string ProtocolName(BindingGatewayProtocol protocol) =>
        protocols.FirstOrDefault(descriptor => descriptor.Id == protocol)?.Name ?? $"协议 {protocol}";

    async void Delete()
    {
        if (Chosen() is not { } provider || busy)
        {
            return;
        }
        var message = "删除「" + provider.Name + "」及其模型配置。使用这些模型的运行时需要重新选择模型；原运行时文件与会话保留。";
        if (!await Ui.Confirm(window, "删除 AI 提供商？", message, "删除"))
        {
            return;
        }
        busy = true;
        ShowActions();
        try
        {
            SetGateway((await Task.Run(() => core.Local.DeleteProvider(DefaultGateway, provider.Id))).Gateways);
            Ui.Show(failure, "");
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
        busy = false;
        ShowActions();
    }

    // Edit adds a provider (existing is null) or edits one in a dialog that
    // stays open until it saves or the user cancels.
    async void Edit(BindingProviderDefinition? existing)
    {
        if (existing == null && protocols.Length == 0)
        {
            return;
        }
        var form = new ProviderForm(this, existing);
        var dialog = new ContentDialog
        {
            Title = existing == null ? "添加 AI 提供商" : "编辑 AI 提供商",
            Content = new ScrollViewer { Content = form.Panel, Padding = new Thickness(0, 0, 16, 0), MaxHeight = 480 },
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 560.0;
        dialog.PrimaryButtonClick += async (_, e) =>
        {
            var deferral = e.GetDeferral();
            e.Cancel = !await form.Save();
            deferral.Complete();
        };
        await window.ShowDialog(dialog);
    }

    // ProviderForm is the editor's fields; template is null for a custom
    // API, and only chosen when adding.
    sealed class ProviderForm
    {
        readonly ProvidersPage page;
        readonly BindingProviderDefinition? existing;
        BindingProviderTemplate? template;
        public StackPanel Panel { get; } = new() { Spacing = 16, MinWidth = 440 };
        readonly ComboBox templateBox = new() { Header = "模板", HorizontalAlignment = HorizontalAlignment.Stretch };
        readonly TextBox name = new() { Header = "名称", PlaceholderText = "例如 DeepSeek" };
        readonly ComboBox protocol = new() { Header = "协议", HorizontalAlignment = HorizontalAlignment.Stretch };
        readonly TextBox endpoint = new() { Header = "接口地址", PlaceholderText = "例如 https://api.example.com/v1" };
        readonly PasswordBox key = new() { Header = "API key" };
        readonly Button reveal = new() { Content = "显示" };
        readonly TextBox models = new() { Header = "模型 ID", PlaceholderText = "提供商 API 规定的标识，多个以逗号分隔" };
        readonly TextBlock nameError = Ui.Error(), endpointError = Ui.Error(), keyError = Ui.Error(), modelsError = Ui.Error(), formError = Ui.Error();
        readonly StackPanel quota = new() { Spacing = 12 };
        readonly TextBox quotaID = new() { Header = "AccessKey ID" };
        readonly PasswordBox quotaSecret = new() { Header = "SecretAccessKey" };
        readonly ToggleSwitch quotaConfirmed = new() { Header = "确认关联" };
        readonly TextBlock quotaStatus = new(), quotaError = Ui.Error();
        readonly Button clearQuota = new() { Content = "清除额度查询认证" };
        readonly TextBlock disclosure = Ui.Muted("");
        bool clearingQuota;

        public ProviderForm(ProvidersPage page, BindingProviderDefinition? existing)
        {
            this.page = page;
            this.existing = existing;
            foreach (var descriptor in page.protocols)
            {
                protocol.Items.Add(descriptor.Name);
            }
            if (existing == null)
            {
                templateBox.Items.Add("自定义 API");
                foreach (var candidate in page.templates)
                {
                    templateBox.Items.Add(candidate.Name);
                }
                templateBox.SelectedIndex = 0;
                templateBox.SelectionChanged += (_, _) => ChooseTemplate(templateBox.SelectedIndex > 0 ? page.templates[templateBox.SelectedIndex - 1] : null);
                Panel.Children.Add(templateBox);
                ChooseTemplate(null);
            }
            else
            {
                name.Text = existing.Name;
                endpoint.Text = existing.Endpoint;
                models.Text = string.Join(", ", existing.Models.Select(model => model.ProviderModelId));
                protocol.SelectedIndex = Array.FindIndex(page.protocols, descriptor => descriptor.Id == existing.Protocol);
            }
            Panel.Children.Add(Ui.Field(name, "", nameError));
            Panel.Children.Add(protocol);
            Panel.Children.Add(Ui.Field(endpoint, "", endpointError));
            if (ApiKey)
            {
                var keyRow = new Grid { ColumnSpacing = 8 };
                keyRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                keyRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                keyRow.Children.Add(key);
                if (KeySaved)
                {
                    key.PlaceholderText = "已保存；留空保持不变";
                    reveal.VerticalAlignment = VerticalAlignment.Bottom;
                    reveal.Click += (_, _) => RevealKey();
                    Grid.SetColumn(reveal, 1);
                    keyRow.Children.Add(reveal);
                }
                Panel.Children.Add(Ui.Field(keyRow, "保存在这台电脑的应用私有配置中。", keyError));
            }
            else
            {
                Panel.Children.Add(Ui.Labeled("认证", Ui.Muted("此提供商的认证请在 Mac 上管理。")));
            }
            Panel.Children.Add(Ui.Field(models, "", modelsError));
            BuildQuota();
            Panel.Children.Add(quota);
            Panel.Children.Add(disclosure);
            Panel.Children.Add(formError);
            ShowQuota();
        }

        // ApiKey tells whether the provider authenticates with an API key kept here.
        bool ApiKey => existing == null || existing.AuthenticationSource == BindingProviderAuthenticationSource.ApiKey;

        // KeySaved tells whether an edited provider has a key that an empty field keeps.
        bool KeySaved => existing?.Authentication.Configured ?? false;

        // QuotaSource is where the provider's usage is read: the edited
        // one's, or the chosen template's.
        BindingProviderQuotaSource QuotaSource => existing?.QuotaSource ?? template?.QuotaSource ?? BindingProviderQuotaSource.None;

        void ChooseTemplate(BindingProviderTemplate? chosen)
        {
            template = chosen;
            quotaID.Text = "";
            quotaSecret.Password = "";
            quotaConfirmed.IsOn = false;
            clearingQuota = false;
            name.Text = chosen?.Name ?? "";
            endpoint.Text = chosen?.Endpoint ?? "";
            protocol.SelectedIndex = chosen == null ? 0 : Array.FindIndex(page.protocols, descriptor => descriptor.Id == chosen.Protocol);
            // A subscription's protocol and endpoint are its own.
            var fixedEndpoint = chosen?.Kind == BindingProviderTemplateKind.Subscription;
            protocol.IsEnabled = !fixedEndpoint;
            endpoint.IsReadOnly = fixedEndpoint;
            disclosure.Text = chosen?.Disclosure is { } shown ? shown.Title + "：" + shown.Detail : "";
            disclosure.Visibility = disclosure.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            ShowQuota();
        }

        // BuildQuota lays out the ARK Coding Plan's usage query credentials,
        // which the user links to the model key; the account is not checked.
        void BuildQuota()
        {
            quota.Children.Add(Ui.Text("额度查询认证", "BodyStrongTextBlockStyle"));
            quota.Children.Add(Ui.Field(Ui.Labeled("状态", quotaStatus), "模型 API key 与此处 AK/SK 分别用于模型和额度查询。账户归属未自动核验。"));
            quota.Children.Add(quotaID);
            quota.Children.Add(Ui.Field(quotaSecret, "输入完整 AK/SK 替换；留空保留。改模型 Key 后需再次确认，有上限且未确认时暂停派发。上限按此来源观测执行，不是计费硬预算。", quotaError));
            quota.Children.Add(Ui.Field(quotaConfirmed, "此次保存时确认此来源对应当前模型 Key 的个人 Coding Plan"));
            clearQuota.Click += (_, _) =>
            {
                quotaID.Text = "";
                quotaSecret.Password = "";
                quotaConfirmed.IsOn = false;
                clearingQuota = true;
                ShowQuota();
            };
            quota.Children.Add(clearQuota);
        }

        void ShowQuota()
        {
            quota.Visibility = QuotaSource == BindingProviderQuotaSource.ArkCodingUsage ? Visibility.Visible : Visibility.Collapsed;
            var described = existing?.QuotaAuthentication ?? new BindingQuotaAuthenticationDescription(false, false);
            quotaStatus.Text = clearingQuota ? "保存后清除"
                : described is { Configured: true, BindingConfirmed: true } ? "已由用户确认关联"
                : described.Configured ? "需要重新确认关联"
                : "尚未配置";
            clearQuota.Visibility = described.Configured && !clearingQuota ? Visibility.Visible : Visibility.Collapsed;
        }

        // QuotaEdit is what saving does to the usage query credentials; null keeps them.
        BindingQuotaAuthenticationEdit? QuotaEdit()
        {
            var (id, secret) = (quotaID.Text.Trim(), quotaSecret.Password.Trim());
            if (clearingQuota)
            {
                return new BindingQuotaAuthenticationEdit.Clear();
            }
            if (id.Length > 0 && secret.Length > 0)
            {
                return new BindingQuotaAuthenticationEdit.SetAccessKey(id, secret, quotaConfirmed.IsOn);
            }
            return quotaConfirmed.IsOn ? new BindingQuotaAuthenticationEdit.ConfirmBinding() : null;
        }

        // RevealKey reads the saved key into the field to show and edit it.
        async void RevealKey()
        {
            var id = existing!.Id;
            try
            {
                key.Password = await Task.Run(() => page.core.Local.ReadProviderApiKey(DefaultGateway, id));
                key.PasswordRevealMode = PasswordRevealMode.Visible;
                reveal.Visibility = Visibility.Collapsed;
            }
            catch (Exception problem)
            {
                Ui.Show(formError, Core.Describe(problem));
            }
        }

        string[] ModelIDs() => models.Text.Split([',', '，', ' '], StringSplitOptions.RemoveEmptyEntries);

        // Save checks the fields and saves; false keeps the dialog open.
        public async Task<bool> Save()
        {
            var (shownName, shownEndpoint, shownKey) = (name.Text.Trim(), endpoint.Text.Trim(), key.Password.Trim());
            var ids = ModelIDs();
            Ui.Show(nameError, shownName.Length == 0 ? "请填写名称" : "");
            Ui.Show(endpointError, shownEndpoint.Length == 0 ? "请填写接口地址" : "");
            Ui.Show(keyError, ApiKey && shownKey.Length == 0 && !KeySaved ? "请填写 API key" : "");
            Ui.Show(modelsError, ids.Length == 0 ? "请填写至少一个模型 ID" : "");
            var incomplete = (quotaID.Text.Trim().Length == 0) != (quotaSecret.Password.Trim().Length == 0);
            Ui.Show(quotaError, incomplete ? "额度查询需要完整 AK/SK" : "");
            if (page.busy || protocol.SelectedIndex < 0 || shownName.Length == 0 || shownEndpoint.Length == 0 || ids.Length == 0
                || (ApiKey && shownKey.Length == 0 && !KeySaved) || incomplete)
            {
                return false;
            }
            var draft = new BindingProviderDraft(
                Id: existing?.Id ?? "",
                Name: shownName,
                Protocol: page.protocols[protocol.SelectedIndex].Id,
                Endpoint: shownEndpoint,
                ExecutionBackend: existing?.ExecutionBackend ?? template?.ExecutionBackend ?? new BindingExecutionBackend.Http(),
                RequestProfile: existing?.RequestProfile ?? template?.RequestProfile ?? BindingRequestProfile.Native,
                AuthenticationSource: existing?.AuthenticationSource ?? template?.AuthenticationSource ?? BindingProviderAuthenticationSource.ApiKey,
                QuotaSource: existing?.QuotaSource ?? template?.QuotaSource ?? BindingProviderQuotaSource.None,
                // A model kept by its ID keeps its record, which routes and choices name.
                Models: [.. ids.Select(id => existing?.Models.FirstOrDefault(model => model.ProviderModelId == id)
                    ?? new BindingProviderModel("", id, id, null, null, null, null))],
                QuotaLimits: existing?.QuotaLimits ?? []);
            BindingAuthenticationEdit authentication = ApiKey && shownKey.Length > 0
                ? new BindingAuthenticationEdit.SetApiKey(shownKey)
                : new BindingAuthenticationEdit.Keep();
            // The model key and the usage credentials are saved together.
            var quotaEdit = QuotaSource == BindingProviderQuotaSource.ArkCodingUsage ? QuotaEdit() : null;
            page.busy = true;
            try
            {
                var update = await Task.Run(() => page.core.Local.SaveProvider(DefaultGateway, draft, authentication, quotaEdit));
                page.SetGateway(update.Gateways);
                return true;
            }
            catch (Exception problem)
            {
                Ui.Show(formError, Core.Describe(problem));
                return false;
            }
            finally
            {
                page.busy = false;
                page.ShowActions();
            }
        }
    }
}
