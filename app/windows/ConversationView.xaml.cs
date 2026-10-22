using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Velune.Bindings;
using Windows.System;
using Windows.UI.Core;

namespace Velune;

// One open conversation: its transcript, the interaction it waits on and the
// composer with the next turn's action row.
public sealed partial class ConversationView : UserControl
{
    internal Conversation Model { get; }
    readonly WorkspaceWindow window;
    // The interaction shown, and the replies typed or chosen for it by question ID.
    (string Id, bool Busy)? shownInteraction;
    readonly Dictionary<string, string> answers = [];
    readonly Dictionary<string, string> chosen = [];

    internal ConversationView(Conversation model, WorkspaceWindow window)
    {
        InitializeComponent();
        Model = model;
        this.window = window;
        AttachTranscript();
        AttachComposer();
        ParentButton.Click += (_, _) =>
        {
            if (Model.ParentID is { } parent)
            {
                window.OpenLinked(parent);
            }
        };
        Input.PreviewKeyDown += InputKeyDown;
        SendButton.Click += (_, _) => Send();
        RuntimeButton.Click += (_, _) => window.OpenSettings(SettingsPage.Runtimes);
        StopButton.Click += (_, _) => Model.Stop();
        AgentMenu.Opening += (_, _) => FillAgentMenu();
        PermissionMenu.Opening += (_, _) => FillPermissionMenu();
        CwdMenu.Opening += (_, _) => FillCwdMenu();
        Model.Changed += Render;
        Loaded += (_, _) => Input.Focus(FocusState.Programmatic);
        Render();
    }

    // ComposerInPane puts the composer under the transcript, or takes it out
    // for the composer's tool window, which shows the focused tab's.
    internal void ComposerInPane(bool inPane)
    {
        if (ComposerArea.Parent is Border host)
        {
            host.Child = null;
        }
        if (inPane && !Body.Children.Contains(ComposerArea))
        {
            Body.Children.Add(ComposerArea);
        }
        else if (!inPane)
        {
            Body.Children.Remove(ComposerArea);
        }
    }

    internal void FocusInput() => Input.Focus(FocusState.Programmatic);

    public void Close()
    {
        FlushDraft();
        Model.Close();
    }

    void InputKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (ComposerKey(e))
        {
            e.Handled = true;
            return;
        }
        var control = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control).HasFlag(CoreVirtualKeyStates.Down);
        if (e.Key == VirtualKey.Enter && control)
        {
            e.Handled = true;
            Send();
        }
    }

    async void Send()
    {
        var text = Input.Text;
        if ((string.IsNullOrWhiteSpace(text) && Model.Attached.Count == 0) || Model.Busy || Model.Next.Length == 0 || Model.Uploading > 0)
        {
            return;
        }
        SetInput("");
        // Sending shows the whole list again, at its end.
        if (outlineFolded)
        {
            SetOutlineFolded(false, null);
        }
        ScrollToEnd();
        if (!await Model.Send(text) && Input.Text.Length == 0)
        {
            SetInput(text);
        }
    }

    void Render()
    {
        // A change of the setting meanwhile turns the folded outline back.
        if (outlineFolded && Model.Configuration.Outline != BindingOutlinePresentation.InPlace)
        {
            outlineFolded = false;
        }
        var subAgent = Model.ParentID != null;
        SubAgentBar.Visibility = Visible(subAgent);
        ComposerPanel.Visibility = Visible(!subAgent);
        Spinner.IsActive = Model.Loading;
        var noRuntime = Model.Target.Unsent && window.Listing.Loaded && Model.Configuration.Runtimes.Count == 0;
        Empty.Text = noRuntime ? "这台设备还没有可用的 Agent 运行时。Velune 通过 Agent 运行时执行会话。"
            : Model.Loading || Model.Messages.Count > 0 ? "" : Model.Snapshot == null ? "输入第一条消息开始会话。" : "还没有消息。";
        RuntimeButton.Visibility = Visible(noRuntime);
        RenderTranscript();
        FailureBar.Message = Model.Failure;
        FailureBar.IsOpen = Model.Failure.Length > 0;
        RenderInteraction();

        AgentButton.Visibility = Visible(Model.Next.Length > 0);
        AgentButton.Content = AgentTitle();
        PermissionButton.Visibility = Visible(Model.Next.Length > 0 && (Model.PermissionModes().Length > 0 || Model.SessionOptions.Length > 0));
        (PermissionButton.Content, var permissionTip) = PermissionTitle();
        ToolTipService.SetToolTip(PermissionButton, permissionTip);
        CwdButton.Visibility = Visible(Model.Target.Unsent);
        CwdButton.Content = Model.Cwd.Length > 0 ? Listing.LastPart(Model.Cwd) : "工作目录";
        ToolTipService.SetToolTip(CwdButton, Model.Cwd.Length > 0 ? Model.Cwd : "工作目录：留空时使用运行时的默认目录");
        RenderComposer();
    }

    static Visibility Visible(bool visible) => visible ? Visibility.Visible : Visibility.Collapsed;

    void RenderInteraction()
    {
        var interaction = Model.Interaction;
        if (interaction == null)
        {
            shownInteraction = null;
            InteractionCard.Visibility = Visibility.Collapsed;
            InteractionCard.Child = null;
            return;
        }
        if (shownInteraction is { } shown && shown.Id == interaction.Id && shown.Busy == Model.Busy)
        {
            return;
        }
        if (shownInteraction?.Id != interaction.Id)
        {
            answers.Clear();
            chosen.Clear();
        }
        shownInteraction = (interaction.Id, Model.Busy);
        InteractionCard.Child = interaction.Kind switch
        {
            BindingInteractionKind.Approval approval => Approval(interaction.Id, approval),
            BindingInteractionKind.UserInput input => Questions(interaction.Id, input.Questions),
            _ => null,
        };
        InteractionCard.Visibility = Visibility.Visible;
    }

    UIElement Approval(string interactionID, BindingInteractionKind.Approval approval)
    {
        var panel = new StackPanel { Spacing = 12 };
        panel.Children.Add(new TextBlock { Text = approval.Title, Style = (Style)Application.Current.Resources["BodyStrongTextBlockStyle"], TextWrapping = TextWrapping.Wrap });
        if (approval.Detail.Trim().Length > 0)
        {
            panel.Children.Add(new ScrollViewer { MaxHeight = 180, Content = Markdown.Code(approval.Detail, 12) });
        }
        var options = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        foreach (var option in approval.Options)
        {
            var button = new Button { Content = option.Label, IsEnabled = !Model.Busy };
            button.Click += (_, _) => Model.Reply(interactionID, new BindingRuntimeInteractionReply.Decision(option.Id));
            options.Children.Add(button);
        }
        panel.Children.Add(options);
        return panel;
    }

    UIElement Questions(string interactionID, BindingInteractionQuestion[] questions)
    {
        var panel = new StackPanel { Spacing = 12 };
        var submit = new Button { Content = "提交", Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
        void Check() => submit.IsEnabled = !Model.Busy && questions.All(question =>
            question.Options.Length == 0 ? answers.GetValueOrDefault(question.Id, "").Trim().Length > 0 : chosen.ContainsKey(question.Id));
        foreach (var question in questions)
        {
            panel.Children.Add(new TextBlock { Text = question.Text, Style = (Style)Application.Current.Resources["BodyStrongTextBlockStyle"], TextWrapping = TextWrapping.Wrap });
            if (question.Options.Length == 0)
            {
                if (question.Secret)
                {
                    var secret = new PasswordBox { Password = answers.GetValueOrDefault(question.Id, "") };
                    secret.PasswordChanged += (_, _) => { answers[question.Id] = secret.Password; Check(); };
                    panel.Children.Add(secret);
                }
                else
                {
                    var answer = new TextBox { Text = answers.GetValueOrDefault(question.Id, ""), TextWrapping = TextWrapping.Wrap };
                    answer.TextChanged += (_, _) => { answers[question.Id] = answer.Text; Check(); };
                    panel.Children.Add(answer);
                }
                continue;
            }
            var options = new RadioButtons { MaxColumns = 4 };
            foreach (var option in question.Options)
            {
                options.Items.Add(option.Label);
            }
            if (chosen.TryGetValue(question.Id, out var label))
            {
                options.SelectedItem = label;
            }
            options.SelectionChanged += (_, _) =>
            {
                if (options.SelectedItem is string picked)
                {
                    chosen[question.Id] = picked;
                }
                Check();
            };
            panel.Children.Add(options);
        }
        submit.Click += (_, _) => Model.Reply(interactionID, new BindingRuntimeInteractionReply.Answers(
            [.. questions.Select(question => new BindingInteractionAnswer(question.Id,
                [chosen.TryGetValue(question.Id, out var value) ? value : answers.GetValueOrDefault(question.Id, "")]))]));
        var cancel = new Button { Content = "取消", IsEnabled = !Model.Busy };
        cancel.Click += (_, _) => Model.Reply(interactionID, new BindingRuntimeInteractionReply.Cancel());
        panel.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { submit, cancel } });
        Check();
        return panel;
    }

    // AgentTitle names the next turn's runtime, model and, when its model
    // takes one, reasoning level.
    string AgentTitle()
    {
        var parts = new List<string> { Model.Configuration.Runtime(Model.Next)?.Name ?? "选择运行时", Model.ModelName() };
        if (Model.EffortOptions().Length > 0 && (Model.Effort() ?? Model.CurrentEffort()) is { } effort)
        {
            parts.Add(effort);
        }
        return string.Join(" · ", parts);
    }

    (string Title, string Tooltip) PermissionTitle()
    {
        if (Model.PermissionModes().Length == 0)
        {
            return ("会话选项", "ACP Agent 为此会话提供的选项");
        }
        var mode = Model.PermissionMode(Model.Choice().PermissionMode);
        var current = Model.PermissionMode(Model.CurrentSettings()?.PermissionMode);
        var title = (mode ?? current)?.Label ?? "运行时默认权限";
        var tooltip = mode != null
            ? "权限模式：" + mode.Label + "。" + mode.Detail
            : Model.HasTurns(Model.Next) ? "权限模式：沿用会话当前的模式" : "权限模式：由运行时使用自己的默认模式";
        return (title, tooltip);
    }

    static MenuFlyoutItem Item(string text, Action chosen, bool enabled = true)
    {
        var item = new MenuFlyoutItem { Text = text, IsEnabled = enabled };
        item.Click += (_, _) => chosen();
        return item;
    }

    static RadioMenuFlyoutItem Choice(string group, string text, bool isChecked, Action chosen)
    {
        var item = new RadioMenuFlyoutItem { Text = text, GroupName = group, IsChecked = isChecked };
        item.Click += (_, _) => chosen();
        return item;
    }

    static MenuFlyoutItem Heading(string text) => new() { Text = text, IsEnabled = false };

    // FillAgentMenu offers the next turn's runtime, model and reasoning
    // level, a submenu each, titled with the choice.
    void FillAgentMenu()
    {
        AgentMenu.Items.Clear();
        var runtimeName = Model.Configuration.Runtime(Model.Next)?.Name ?? "选择运行时";
        // An unchosen part follows the conversation's current one, or is the runtime's own.
        var unchosen = Model.HasTurns(Model.Next) ? "沿用当前" : "运行时默认";

        var runtimes = new MenuFlyoutSubItem { Text = "运行时：" + runtimeName };
        foreach (var runtime in Model.Configuration.Runtimes)
        {
            runtimes.Items.Add(Choice("runtime", runtime.Name, runtime.Id == Model.Next, () => Model.ChooseRuntime(runtime.Id)));
        }
        runtimes.Items.Add(new MenuFlyoutSeparator());
        runtimes.Items.Add(Item("Agent 运行时设置…", () => window.OpenSettings(SettingsPage.Runtimes)));
        AgentMenu.Items.Add(runtimes);

        var modelName = Model.ModelName();
        var models = new MenuFlyoutSubItem { Text = "模型：" + modelName };
        FillModels(models.Items, modelName);
        AgentMenu.Items.Add(models);

        var levels = Model.EffortOptions();
        if (levels.Length == 0)
        {
            return;
        }
        var effort = Model.Effort();
        var efforts = new MenuFlyoutSubItem { Text = "推理强度：" + (effort ?? Model.CurrentEffort() ?? unchosen) };
        var first = Model.CurrentEffort() is { } current ? "沿用当前（" + current + "）" : unchosen;
        efforts.Items.Add(Choice("effort", first, effort == null, () => Model.Choose(false, choice => choice with { Effort = null })));
        foreach (var level in levels)
        {
            efforts.Items.Add(Choice("effort", level, effort == level, () => Model.Choose(true, choice => choice with { Effort = level })));
        }
        AgentMenu.Items.Add(efforts);
    }

    // FillModels lists no choice first (the conversation's current model, or
    // the runtime's default), then the runtime's own models, before the
    // models Velune's providers serve.
    void FillModels(IList<MenuFlyoutItemBase> items, string modelName)
    {
        switch (Model.NextCatalog)
        {
            case { Loading: true }:
                items.Add(Heading("正在读取运行时的模型…"));
                break;
            case { Failure.Length: > 0 } failed:
                items.Add(Heading("无法读取运行时的模型：" + failed.Failure));
                break;
        }
        var chosenModel = Model.Choice().Model;
        var first = Model.HasTurns(Model.Next) && Model.NextModel() != null ? "沿用当前（" + modelName + "）" : "运行时默认";
        items.Add(Choice("model", first, chosenModel == null, () => Model.Choose(false, choice => choice with { Model = null })));
        var choices = Model.ModelChoices();
        void Pick(ModelChoice model) =>
            items.Add(Choice("model", model.Name, Equals(model.Selection, chosenModel), () => Model.Choose(true, choice => choice with { Model = model.Selection })));
        foreach (var model in choices.Where(model => !model.Managed))
        {
            Pick(model);
        }
        if (choices.Any(model => model.Managed))
        {
            items.Add(new MenuFlyoutSeparator());
            items.Add(Heading("Velune 提供商"));
            foreach (var model in choices.Where(model => model.Managed))
            {
                Pick(model);
            }
        }
        items.Add(new MenuFlyoutSeparator());
        items.Add(Item("重新读取模型", () => Model.LoadCatalog(Model.Next, true), Model.NextCatalog is not { Loading: true }));
        items.Add(Item("AI 提供商设置…", () => window.OpenSettings(SettingsPage.Providers)));
    }

    // FillPermissionMenu offers what the next turn may do, among the runtime
    // type's modes in its own names. A mode is the session's state in the
    // runtime: choosing none keeps the current one.
    void FillPermissionMenu()
    {
        PermissionMenu.Items.Clear();
        if (Model.PermissionModes().Length > 0)
        {
            FillPermissionModes();
        }
        FillSessionOptions();
    }

    void FillPermissionModes()
    {
        var mode = Model.PermissionMode(Model.Choice().PermissionMode);
        var current = Model.PermissionMode(Model.CurrentSettings()?.PermissionMode);
        var first = current != null ? "沿用当前（" + current.Label + "）" : Model.HasTurns(Model.Next) ? "沿用当前" : "运行时默认";
        PermissionMenu.Items.Add(Choice("permission", first, mode == null, () => Model.Choose(false, choice => choice with { PermissionMode = null })));
        foreach (var option in Model.PermissionModes())
        {
            var item = Choice("permission", option.Label, mode?.Id == option.Id, () => Model.Choose(true, choice => choice with { PermissionMode = option.Id }));
            ToolTipService.SetToolTip(item, option.Detail);
            PermissionMenu.Items.Add(item);
        }
    }

    // FillCwdMenu offers where an unsent conversation works. This PC's
    // folders are browsed; another device's are typed, as they exist only on it.
    void FillCwdMenu()
    {
        CwdMenu.Items.Clear();
        if (Model.Device.IsLocal)
        {
            CwdMenu.Items.Add(Item("选择目录…", Browse));
        }
        else
        {
            CwdMenu.Items.Add(Item("输入目录…", EnterCwd));
        }
        if (Model.Cwd.Length > 0)
        {
            CwdMenu.Items.Add(Item("使用运行时默认目录", () => SetCwd("")));
        }
        var projects = Model.Configuration.Projects.Take(5).ToList();
        if (projects.Count > 0)
        {
            CwdMenu.Items.Add(new MenuFlyoutSeparator());
            CwdMenu.Items.Add(Heading("最近使用"));
            foreach (var project in projects)
            {
                CwdMenu.Items.Add(Choice("cwd", project, project == Model.Cwd, () => SetCwd(project)));
            }
        }
    }

    void SetCwd(string cwd)
    {
        Model.Cwd = cwd;
        Render();
    }

    async void Browse()
    {
        if (await window.PickFolder() is { } path)
        {
            SetCwd(path);
        }
    }

    // EnterCwd takes an absolute path on the device the conversation runs
    // on; the device checks it when the conversation starts.
    async void EnterCwd()
    {
        var input = new TextBox { Text = Model.Cwd, PlaceholderText = "绝对路径" };
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "新会话的工作目录",
            Content = new StackPanel
            {
                Spacing = 12,
                Children =
                {
                    new TextBlock { Text = "输入那台设备上的绝对路径；留空使用运行时的默认目录。", TextWrapping = TextWrapping.Wrap },
                    input,
                },
            },
            PrimaryButtonText = "使用",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            SetCwd(input.Text.Trim());
        }
    }
}
