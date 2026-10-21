using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The provider page's dialogs: choosing what to add, signing in, importing
// from an Agent runtime, and picking models from templates.
static class ProviderFlows
{
    internal abstract record Choice
    {
        internal sealed record Template(BindingProviderTemplate Chosen) : Choice;
        internal sealed record Import : Choice;
    }

    // ChooseTemplate asks what a new provider starts from: a subscription,
    // whose connection is fixed, an API provider, or an Agent runtime's own
    // providers. Null when the user cancels.
    public static async Task<Choice?> ChooseTemplate(ProvidersPage page)
    {
        var choices = new List<Choice>();
        var list = new ListView { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true, MaxHeight = 360 };
        void Group(string title, IEnumerable<(Choice Choice, string Name, string Detail)> entries)
        {
            var shown = entries.ToList();
            if (shown.Count == 0)
            {
                return;
            }
            var heading = Ui.Text(title, "BodyStrongTextBlockStyle");
            heading.Margin = new Thickness(12, 8, 12, 2);
            list.Items.Add(new ListViewItem { Content = heading, IsEnabled = false, MinHeight = 0 });
            choices.Add(null!);
            foreach (var (choice, name, detail) in shown)
            {
                var row = Ui.TwoLines(name, detail, true);
                row.Padding = new Thickness(4, 6, 4, 6);
                list.Items.Add(new ListViewItem { Content = row });
                choices.Add(choice);
            }
        }
        Group("订阅", page.Templates.Where(template => template.Kind == BindingProviderTemplateKind.Subscription)
            .Select(template => ((Choice)new Choice.Template(template), template.Name, template.Summary)));
        Group("API", page.Templates.Where(template => template.Kind == BindingProviderTemplateKind.Api)
            .Select(template => ((Choice)new Choice.Template(template), template.Name, template.Summary)));
        Group("导入", [(new Choice.Import(), "从 Agent 运行时导入…", "读取 Codex 或 Claude Code 运行时已配置的提供商、模型与认证来源，逐项选择导入。")]);

        var disclosure = new InfoBar { Severity = InfoBarSeverity.Warning, IsClosable = false };
        var panel = new StackPanel { Spacing = 12, MinWidth = 440 };
        panel.Children.Add(list);
        panel.Children.Add(disclosure);
        var dialog = new ContentDialog
        {
            Title = "添加 AI 提供商",
            Content = panel,
            PrimaryButtonText = "继续",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
            IsPrimaryButtonEnabled = false,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 560.0;
        Choice? Chosen() => list.SelectedIndex >= 0 ? choices[list.SelectedIndex] : null;
        list.SelectionChanged += (_, _) =>
        {
            dialog.IsPrimaryButtonEnabled = Chosen() != null;
            // A subscription says what using it means before it is added.
            if (Chosen() is Choice.Template { Chosen.Disclosure: { } shown })
            {
                disclosure.Title = shown.Title;
                disclosure.Message = shown.Detail;
                disclosure.IsOpen = true;
            }
            else
            {
                disclosure.IsOpen = false;
            }
        };
        var chosen = false;
        list.DoubleTapped += (_, _) =>
        {
            if (Chosen() != null)
            {
                chosen = true;
                dialog.Hide();
            }
        };
        var result = await page.Window.ShowDialog(dialog);
        return result == ContentDialogResult.Primary || chosen ? Chosen() : null;
    }

    // Login signs a saved provider in through the core's flow: it opens the
    // sign-in page, shows a device code, or asks what the flow asks. It
    // returns the gateways as the core keeps them afterwards; a failure to
    // cancel or read them is thrown to the caller.
    public static async Task<BindingGatewayConfig[]> Login(Core core, SettingsWindow window, string providerID, string name)
    {
        var application = core.Local;
        var status = Ui.Text("正在开始登录…");
        var progress = new ProgressRing { IsActive = true, Width = 20, Height = 20 };
        var statusRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        statusRow.Children.Add(progress);
        statusRow.Children.Add(status);
        var notices = new StackPanel { Spacing = 8 };
        var prompts = new StackPanel { Spacing = 8 };
        var outcome = Ui.Error();
        var panel = new StackPanel { Spacing = 16, MinWidth = 400 };
        panel.Children.Add(statusRow);
        panel.Children.Add(notices);
        panel.Children.Add(prompts);
        panel.Children.Add(outcome);
        var dialog = new ContentDialog
        {
            Title = "登录「" + name + "」",
            Content = new ScrollViewer { Content = panel, MaxHeight = 420 },
            CloseButtonText = "取消",
        };
        dialog.Resources["ContentDialogMaxWidth"] = 520.0;

        BindingGatewayConfig[]? saved = null;
        var running = true;
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(500) };
        var polling = false;

        void Finish(string? error, bool cancelled)
        {
            running = false;
            timer.Stop();
            progress.IsActive = false;
            progress.Visibility = Visibility.Collapsed;
            prompts.Children.Clear();
            if (error == null)
            {
                status.Text = "已登录。";
                dialog.Hide();
                return;
            }
            status.Text = cancelled ? "已取消登录。" : "登录没有完成。";
            Ui.Show(outcome, cancelled ? "" : error);
            dialog.CloseButtonText = "关闭";
        }

        void Show(BindingAuthenticationProgress shown)
        {
            if (shown.Gateways is { } gateways)
            {
                saved = gateways;
            }
            foreach (var update in shown.Events)
            {
                switch (update.TypeId)
                {
                    case "notify" when update.Notification is { } notice:
                        Notice(notice);
                        break;
                    case "prompt" when update.Prompt is { } asked && update.Id is { } id:
                        Prompt(id, asked);
                        break;
                    case "result":
                        Finish(update.Ok == true ? null : update.Error ?? "登录失败", update.Cancelled == true);
                        break;
                }
            }
            if (!shown.Running && running)
            {
                Finish(saved != null ? null : "登录进程已结束", false);
            }
        }

        void Notice(BindingAuthenticationNotification notice)
        {
            if (notice.Url is { Length: > 0 } url)
            {
                status.Text = "已在浏览器中打开登录页面，请在那里完成登录。";
                Open(url);
                var again = new HyperlinkButton { Content = "再次打开登录页面", Padding = new Thickness(0) };
                again.Click += (_, _) => Open(url);
                notices.Children.Add(again);
            }
            if (notice.UserCode is { Length: > 0 } code)
            {
                var codeText = new TextBlock { Text = code, FontSize = 24, FontFamily = (Microsoft.UI.Xaml.Media.FontFamily)Application.Current.Resources["MonoFont"], IsTextSelectionEnabled = true };
                var copy = new Button { Content = "复制" };
                copy.Click += (_, _) =>
                {
                    var data = new Windows.ApplicationModel.DataTransfer.DataPackage();
                    data.SetText(code);
                    Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(data);
                };
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
                row.Children.Add(codeText);
                row.Children.Add(copy);
                notices.Children.Add(Ui.Labeled("验证码", row));
            }
            if (notice.VerificationUri is { Length: > 0 } verification && notice.Url != verification)
            {
                var link = new HyperlinkButton { Content = verification, Padding = new Thickness(0) };
                link.Click += (_, _) => Open(verification);
                notices.Children.Add(link);
            }
            foreach (var text in new[] { notice.Instructions, notice.Text })
            {
                if (text is { Length: > 0 })
                {
                    notices.Children.Add(Ui.Text(text));
                }
            }
        }

        void Prompt(string id, BindingAuthenticationPrompt asked)
        {
            prompts.Children.Clear();
            Func<string> value;
            FrameworkElement input;
            if (asked.Options is { Length: > 0 } options)
            {
                var choice = new ComboBox { Header = asked.Text, HorizontalAlignment = HorizontalAlignment.Stretch };
                foreach (var option in options)
                {
                    choice.Items.Add(option.Label);
                }
                choice.SelectedIndex = 0;
                value = () => choice.SelectedIndex >= 0 ? options[choice.SelectedIndex].Id : "";
                input = choice;
            }
            else
            {
                var text = new TextBox { Header = asked.Text };
                value = () => text.Text;
                input = text;
            }
            var reply = new Button { Content = "继续", Style = Ui.Style("AccentButtonStyle") };
            reply.Click += async (_, _) =>
            {
                reply.IsEnabled = false;
                var answer = value();
                try
                {
                    prompts.Children.Clear();
                    Show(await Task.Run(() => application.AuthenticationReply(id, answer)));
                }
                catch (Exception problem)
                {
                    Finish(Core.Describe(problem), false);
                }
            };
            prompts.Children.Add(input);
            prompts.Children.Add(reply);
        }

        timer.Tick += async (_, _) =>
        {
            if (polling || !running)
            {
                return;
            }
            polling = true;
            try
            {
                Show(await Task.Run(application.AuthenticationPoll));
            }
            catch (Exception problem)
            {
                Finish(Core.Describe(problem), false);
            }
            polling = false;
        };
        dialog.Opened += async (_, _) =>
        {
            try
            {
                Show(await Task.Run(() => application.AuthenticationStart(ProvidersPage.DefaultGateway, providerID)));
                if (running)
                {
                    status.Text = notices.Children.Count == 0 ? "正在等待登录完成…" : status.Text;
                    timer.Start();
                }
            }
            catch (Exception problem)
            {
                Finish(Core.Describe(problem), false);
            }
        };
        await window.ShowDialog(dialog);
        timer.Stop();
        if (running)
        {
            // Closing the dialog while it runs cancels the sign-in.
            running = false;
            saved = (await Task.Run(application.AuthenticationCancel)).Gateways ?? saved;
        }
        return saved ?? (await Task.Run(() => application.List())).Gateways;
    }

    static void Open(string url)
    {
        if (Uri.TryCreate(url, UriKind.Absolute, out var uri) && (uri.Scheme == Uri.UriSchemeHttps || uri.Scheme == Uri.UriSchemeHttp))
        {
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(uri.AbsoluteUri) { UseShellExecute = true });
        }
    }

    // Import reads an Agent runtime's own providers, shows what can be
    // imported and why not, and imports only what the user ticks.
    public static async Task Import(ProvidersPage page)
    {
        var application = page.Core.Local;
        var sources = page.Runtimes
            .Select(runtime => (Runtime: runtime, Type: page.ImportTypes.FirstOrDefault(type => type.Id == runtime.TypeId)))
            .Where(entry => entry.Type != null)
            .ToList();
        var panel = new StackPanel { Spacing = 12, MinWidth = 480 };
        var dialog = new ContentDialog
        {
            Title = "从 Agent 运行时导入",
            Content = new ScrollViewer { Content = panel, MaxHeight = 480, Padding = new Thickness(0, 0, 16, 0) },
            PrimaryButtonText = "导入",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
            IsPrimaryButtonEnabled = false,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 620.0;
        if (sources.Count == 0)
        {
            panel.Children.Add(Ui.Text("没有可读取提供商配置的 Agent 运行时。先在“Agent 运行时”中添加 Codex 或 Claude Code。"));
            dialog.PrimaryButtonText = "";
            dialog.CloseButtonText = "关闭";
            await page.Window.ShowDialog(dialog);
            return;
        }
        var source = new ComboBox { Header = "来源", HorizontalAlignment = HorizontalAlignment.Stretch };
        foreach (var (runtime, type) in sources)
        {
            source.Items.Add(runtime.Name + "（" + type!.Name + "）");
        }
        var status = Ui.Muted("");
        var candidates = new StackPanel { Spacing = 8 };
        var replace = new CheckBox { Content = "替换已导入的同一提供商", Visibility = Visibility.Collapsed };
        var outcome = Ui.Error();
        panel.Children.Add(source);
        panel.Children.Add(status);
        panel.Children.Add(candidates);
        panel.Children.Add(replace);
        panel.Children.Add(outcome);

        BindingProviderImportPreview? preview = null;
        BindingProviderImportSource? read = null;
        var chosen = new Dictionary<string, List<(string Key, CheckBox Box)>>();
        var imported = new HashSet<string>();

        void ShowChosen()
        {
            var any = chosen.Values.Any(models => models.Any(model => model.Box.IsChecked == true));
            dialog.IsPrimaryButtonEnabled = any;
            replace.Visibility = chosen.Any(entry => imported.Contains(entry.Key) && entry.Value.Any(model => model.Box.IsChecked == true))
                ? Visibility.Visible
                : Visibility.Collapsed;
        }

        async void Read()
        {
            var (runtime, type) = sources[source.SelectedIndex];
            var asked = new BindingProviderImportSource("harness", type!.FamilyId, runtime.Id, null, []);
            read = asked;
            preview = null;
            chosen.Clear();
            imported.Clear();
            candidates.Children.Clear();
            dialog.IsPrimaryButtonEnabled = false;
            status.Text = "正在读取…";
            try
            {
                var shown = await Task.Run(() => application.PreviewProviderImport(ProvidersPage.DefaultGateway, asked));
                if (!ReferenceEquals(read, asked))
                {
                    return;
                }
                preview = shown;
                status.Text = shown.SourceLabel;
                foreach (var warning in shown.Warnings)
                {
                    candidates.Children.Add(new InfoBar { Severity = InfoBarSeverity.Warning, IsOpen = true, IsClosable = false, Message = warning });
                }
                foreach (var provider in shown.Providers)
                {
                    candidates.Children.Add(Candidate(provider));
                }
                if (shown.Providers.Length == 0)
                {
                    candidates.Children.Add(Ui.Text("这个运行时没有可导入的提供商。"));
                }
                ShowChosen();
            }
            catch (Exception problem)
            {
                if (ReferenceEquals(read, asked))
                {
                    status.Text = "";
                    Ui.Show(outcome, Core.Describe(problem));
                }
            }
        }

        // Candidate is one provider and its models, none ticked: reading does
        // not choose. Why an item cannot be imported is shown with it.
        Border Candidate(BindingImportProviderCandidate provider)
        {
            var models = new List<(string Key, CheckBox Box)>();
            chosen[provider.Id] = models;
            if (provider.AlreadyImported)
            {
                imported.Add(provider.Id);
            }
            var all = new CheckBox { IsThreeState = false, IsEnabled = provider.CanImport && provider.Models.Any(model => model.CanImport) };
            var heading = new StackPanel { Spacing = 2 };
            heading.Children.Add(Ui.Text(provider.Name + (provider.AlreadyImported ? "（已导入）" : ""), "BodyStrongTextBlockStyle"));
            var detail = new List<string> { provider.Protocol };
            if (provider.Endpoint is { Length: > 0 } address)
            {
                detail.Add(address);
            }
            if (provider.CredentialStatus.Length > 0)
            {
                detail.Add(provider.CredentialStatus);
            }
            heading.Children.Add(Ui.Muted(string.Join(" · ", detail)));
            foreach (var issue in provider.Issues)
            {
                heading.Children.Add(Ui.Muted(issue));
            }
            all.Content = heading;
            var list = new StackPanel { Spacing = 0 };
            var syncing = false;
            foreach (var model in provider.Models)
            {
                var lines = new StackPanel { Spacing = 0 };
                lines.Children.Add(new TextBlock { Text = model.Name.Length > 0 && model.Name != model.ProviderModelId ? $"{model.Name}（{model.ProviderModelId}）" : model.ProviderModelId });
                var parameters = new List<string>();
                if (model.ContextWindow is { } window)
                {
                    parameters.Add("上下文 " + ProviderEditor.Tokens(window));
                }
                if (model.MaxOutputTokens is { } output)
                {
                    parameters.Add("输出 " + ProviderEditor.Tokens(output));
                }
                if (model.ReasoningLevels.Length > 0)
                {
                    parameters.Add("推理 " + string.Join("/", model.ReasoningLevels));
                }
                parameters.AddRange(model.Issues);
                if (parameters.Count > 0)
                {
                    lines.Children.Add(Ui.Muted(string.Join(" · ", parameters)));
                }
                var box = new CheckBox { Content = lines, IsEnabled = provider.CanImport && model.CanImport };
                box.Click += (_, _) =>
                {
                    syncing = true;
                    var ticked = models.Count(entry => entry.Box.IsChecked == true);
                    all.IsChecked = ticked == 0 ? false : ticked == models.Count(entry => entry.Box.IsEnabled) ? true : null;
                    syncing = false;
                    ShowChosen();
                };
                models.Add((model.CandidateKey, box));
                list.Children.Add(box);
            }
            all.Click += (_, _) =>
            {
                if (syncing)
                {
                    return;
                }
                var tick = all.IsChecked == true;
                foreach (var (_, box) in models.Where(entry => entry.Box.IsEnabled))
                {
                    box.IsChecked = tick;
                }
                ShowChosen();
            };
            var expander = new Expander
            {
                Header = $"{provider.Models.Length} 个模型",
                Content = list,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
            };
            var card = new StackPanel { Spacing = 8 };
            card.Children.Add(all);
            if (provider.Models.Length > 0)
            {
                card.Children.Add(expander);
            }
            return new Border { Padding = new Thickness(12), CornerRadius = new CornerRadius(4), Style = Ui.Style("CardStyle"), Child = card };
        }

        source.SelectionChanged += (_, _) => Read();
        source.SelectedIndex = 0;
        dialog.PrimaryButtonClick += async (_, e) =>
        {
            var deferral = e.GetDeferral();
            var selections = chosen
                .Select(entry => new BindingImportSelection(entry.Key, [.. entry.Value.Where(model => model.Box.IsChecked == true).Select(model => model.Key)]))
                .Where(selection => selection.CandidateKeys.Length > 0)
                .ToArray();
            if (preview is not { } shown || read is not { } asked || selections.Length == 0)
            {
                e.Cancel = true;
                deferral.Complete();
                return;
            }
            var replacing = replace.IsChecked == true;
            try
            {
                var result = await Task.Run(() => application.ApplyProviderImport(ProvidersPage.DefaultGateway, asked, shown.Token, selections, replacing));
                page.SetGateway(result.Gateways);
                page.ShowFailure(result.SkippedProviderIds.Length > 0
                    ? $"已导入 {result.ImportedProviderIds.Length} 个提供商；{result.SkippedProviderIds.Length} 个已存在而未替换。"
                    : "");
            }
            catch (Exception problem)
            {
                Ui.Show(outcome, Core.Describe(problem));
                e.Cancel = true;
            }
            deferral.Complete();
        };
        await page.Window.ShowDialog(dialog);
    }

    // PickModels lets the user tick model templates, and entries of the
    // public catalog once read, as editable copies for a provider's models.
    public static async Task<List<BindingProviderModel>> PickModels(ProvidersPage page)
    {
        var entries = page.ModelTemplates
            .Select(template => (Name: template.Name, Detail: Parameters(template.SuggestedProviderModelId, template.ContextWindow, template.MaxOutputTokens, template.ReasoningLevels),
                Model: new BindingProviderModel("", template.SuggestedProviderModelId, template.Nickname, template.Icon, template.ContextWindow, template.MaxOutputTokens, template.ReasoningLevels)))
            .ToList();
        var search = new AutoSuggestBox { PlaceholderText = "搜索", QueryIcon = new SymbolIcon(Symbol.Find) };
        var list = new ListView { SelectionMode = ListViewSelectionMode.Multiple, Height = 320 };
        var catalog = new Button { Content = "读取公共模型目录" };
        var status = Ui.Muted("公共目录的条目只作参数起点，不说明此提供商支持该模型；填入后可逐项修改。");
        var shown = new List<int>();
        var picked = new HashSet<int>();
        // Set while the list is refilled, which leaves the picks as they are.
        var filling = false;
        void Show()
        {
            var query = search.Text.Trim();
            filling = true;
            list.Items.Clear();
            shown.Clear();
            for (var i = 0; i < entries.Count; i++)
            {
                var (name, detail, model) = entries[i];
                if (query.Length > 0 && !name.Contains(query, StringComparison.OrdinalIgnoreCase) && !model.ProviderModelId.Contains(query, StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }
                var row = Ui.TwoLines(name, detail);
                row.Padding = new Thickness(0, 4, 0, 4);
                var item = new ListViewItem { Content = row };
                list.Items.Add(item);
                shown.Add(i);
                if (picked.Contains(i))
                {
                    list.SelectedItems.Add(item);
                }
            }
            filling = false;
            Ui.Placeholder(list, entries.Count == 0 ? "还没有模型模板。可在模型的“存为模板…”保存，或读取公共模型目录。" : "没有符合的条目。");
        }
        search.TextChanged += (_, _) => Show();
        catalog.Click += async (_, _) =>
        {
            catalog.IsEnabled = false;
            status.Text = "正在读取公共模型目录…";
            try
            {
                var fetched = await Task.Run(page.Core.Local.FetchPublicModelCatalog);
                entries.AddRange(fetched.Select(entry => (Name: entry.Name.Length > 0 ? entry.Name : entry.ModelId,
                    Detail: "目录 · " + entry.SourceProviderName + " · " + Parameters(entry.ModelId, entry.ContextWindow, entry.MaxOutputTokens, entry.ReasoningLevels),
                    Model: new BindingProviderModel("", entry.ModelId, entry.Name.Length > 0 ? entry.Name : entry.ModelId, null, entry.ContextWindow, entry.MaxOutputTokens, entry.ReasoningLevels))));
                status.Text = $"已读取 {fetched.Length} 个目录条目。条目只作参数起点，不说明此提供商支持该模型。";
                catalog.Visibility = Visibility.Collapsed;
                Show();
            }
            catch (Exception problem)
            {
                status.Text = Core.Describe(problem);
                catalog.IsEnabled = true;
            }
        };
        Show();
        var top = new Grid { ColumnSpacing = 8 };
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.Children.Add(search);
        Grid.SetColumn(catalog, 1);
        top.Children.Add(catalog);
        var panel = new StackPanel { Spacing = 12, MinWidth = 480 };
        panel.Children.Add(top);
        panel.Children.Add(list);
        panel.Children.Add(status);
        var dialog = new ContentDialog
        {
            Title = "从模板中选取模型",
            Content = panel,
            PrimaryButtonText = "添加",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
            IsPrimaryButtonEnabled = false,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 620.0;
        list.SelectionChanged += (_, e) =>
        {
            // Ticks survive a search that hides them.
            if (filling)
            {
                return;
            }
            foreach (var item in e.AddedItems)
            {
                picked.Add(shown[list.Items.IndexOf(item)]);
            }
            foreach (var item in e.RemovedItems)
            {
                if (list.Items.IndexOf(item) is var index and >= 0)
                {
                    picked.Remove(shown[index]);
                }
            }
            dialog.IsPrimaryButtonEnabled = picked.Count > 0;
        };
        if (await page.Window.ShowDialog(dialog) != ContentDialogResult.Primary)
        {
            return [];
        }
        return [.. picked.Order().Select(index => entries[index].Model)];
    }

    static string Parameters(string id, uint? context, uint? output, string[]? levels)
    {
        var parts = new List<string> { id };
        if (context is { } window)
        {
            parts.Add("上下文 " + ProviderEditor.Tokens(window));
        }
        if (output is { } most)
        {
            parts.Add("输出 " + ProviderEditor.Tokens(most));
        }
        if (levels is { Length: > 0 })
        {
            parts.Add("推理 " + string.Join("/", levels));
        }
        return string.Join(" · ", parts);
    }
}
