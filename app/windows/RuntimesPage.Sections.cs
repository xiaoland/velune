using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The parts of a runtime instance besides its type's fields: where it runs
// and with which HOME, an ACP Agent's launch arguments and what it says of
// itself, and what new conversations on it start with.
sealed partial class RuntimesPage
{
    // The core derives the runtime directory from a HOME Velune manages.
    const string RuntimeDirectoryKey = "agentDir";

    // What an instance's editor reads before it opens.
    sealed record Extras(BindingRuntimeExecution Execution, BindingAcpLaunch? Launch, BindingRuntimeComposerChoices? Choices, string? ManagedHome);

    static readonly BindingRuntimeExecution LocalInherit = new(new BindingRuntimeTransport.Local(), new BindingRuntimeHome.Inherit(), null);

    async Task<Extras?> ReadExtras(BindingRuntimeInstance? existing, BindingRuntimeTypeDescriptor kind)
    {
        if (existing == null)
        {
            return new Extras(LocalInherit, new BindingAcpLaunch([], []), null, null);
        }
        try
        {
            return await Task.Run(() =>
            {
                var application = core.Local;
                var execution = application.RuntimeExecution(existing.Id);
                var launch = kind.LaunchOptions ? application.AcpLaunch(existing.Id) : null;
                var choices = application.ComposerChoices().Runtimes.FirstOrDefault(choices => choices.RuntimeId == existing.Id);
                var home = execution.Home is BindingRuntimeHome.Managed ? application.RuntimeHomeDirectory(existing.Id) : null;
                return new Extras(execution, launch, choices, home);
            });
        }
        catch (Exception problem)
        {
            Problems.Report("读取运行时「" + existing.Name + "」的设置", problem);
            return null;
        }
    }

    static Expander Section(string title, string summary, UIElement content) => new()
    {
        Header = Ui.TwoLines(title, summary),
        Content = content,
        HorizontalAlignment = HorizontalAlignment.Stretch,
        HorizontalContentAlignment = HorizontalAlignment.Stretch,
    };

    // HomeSection chooses the instance's HOME: the user's, a directory, or
    // one Velune manages. Running over SSH is set up on a Mac; here such an
    // instance keeps how it runs.
    sealed class HomeSection
    {
        static readonly string[] names = ["继承当前用户的 HOME", "指定目录", "由 Velune 管理"];
        readonly BindingRuntimeExecution saved;
        readonly ComboBox choice = new() { Header = "HOME", HorizontalAlignment = HorizontalAlignment.Stretch };
        readonly TextBox path = new() { Header = "HOME 目录", PlaceholderText = "绝对路径" };
        readonly Grid pathRow = new() { ColumnSpacing = 8 };
        readonly TextBlock error = Ui.Error();

        public Expander Element { get; }
        public event Action? Changed;
        public bool Managed => choice.SelectedIndex == 2;
        bool Remote => saved.Transport is BindingRuntimeTransport.Ssh;

        public HomeSection(Window window, BindingRuntimeExecution saved, string? managedHome)
        {
            this.saved = saved;
            foreach (var name in names)
            {
                choice.Items.Add(name);
            }
            choice.SelectedIndex = saved.Home switch
            {
                BindingRuntimeHome.Existing => 1,
                BindingRuntimeHome.Managed => 2,
                _ => 0,
            };
            path.Text = saved.Home is BindingRuntimeHome.Existing existing ? existing.Path : "";
            pathRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            pathRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var browse = new Button { Content = "浏览…", VerticalAlignment = VerticalAlignment.Bottom };
            browse.Click += async (_, _) =>
            {
                if (await Native.PickFolder(window) is { } picked)
                {
                    path.Text = picked;
                }
            };
            Grid.SetColumn(browse, 1);
            pathRow.Children.Add(path);
            pathRow.Children.Add(browse);

            var panel = new StackPanel { Spacing = 8 };
            if (Remote)
            {
                panel.Children.Add(Ui.Muted("此实例经 SSH 在另一台机器上运行；运行位置与远端 HOME 在 Mac 上设置，这里保持不变。"));
                choice.IsEnabled = false;
            }
            panel.Children.Add(Ui.Field(choice, "Velune 管理时为此实例单独准备 HOME，不复制你原有的凭据或会话；这不是安全隔离。"));
            panel.Children.Add(pathRow);
            panel.Children.Add(error);
            if (managedHome != null && saved.Home is BindingRuntimeHome.Managed)
            {
                var shown = Directory.Exists(managedHome);
                var reveal = new HyperlinkButton { Content = "在资源管理器中显示", IsEnabled = shown };
                reveal.Click += (_, _) => System.Diagnostics.Process.Start("explorer.exe", $"\"{managedHome}\"");
                panel.Children.Add(Ui.Labeled("位置", new TextBlock { Text = managedHome + (shown ? "" : "（首次运行时创建）"), IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap }));
                panel.Children.Add(reveal);
            }
            choice.SelectionChanged += (_, _) =>
            {
                Show();
                Changed?.Invoke();
            };
            Element = Section("运行环境", Remote ? "经 SSH 运行" : names[Math.Max(choice.SelectedIndex, 0)], panel);
            Show();
        }

        void Show()
        {
            pathRow.Visibility = choice.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
            Ui.Show(error, "");
        }

        // Read is the execution to save, or null after saying what is wrong.
        public BindingRuntimeExecution? Read()
        {
            if (Remote)
            {
                return saved;
            }
            var chosen = path.Text.Trim();
            if (choice.SelectedIndex == 1 && !Path.IsPathFullyQualified(chosen))
            {
                Ui.Show(error, "请填写 HOME 目录的绝对路径");
                Element.IsExpanded = true;
                return null;
            }
            BindingRuntimeHome home = choice.SelectedIndex switch
            {
                1 => new BindingRuntimeHome.Existing(chosen),
                2 => new BindingRuntimeHome.Managed(null),
                _ => new BindingRuntimeHome.Inherit(),
            };
            return saved with { Home = home };
        }
    }

    // LaunchSection holds an ACP Agent's launch arguments and environment.
    sealed class LaunchSection
    {
        readonly TextBox arguments = new() { Header = "启动参数", AcceptsReturn = true, TextWrapping = TextWrapping.NoWrap, MinHeight = 64, PlaceholderText = "每行一个参数" };
        readonly TextBox environment = new() { Header = "环境变量", AcceptsReturn = true, TextWrapping = TextWrapping.NoWrap, MinHeight = 64, PlaceholderText = "每行一个：名称=值" };
        readonly TextBlock error = Ui.Error();

        public Expander Element { get; }

        public LaunchSection(BindingAcpLaunch? launch)
        {
            arguments.Text = string.Join("\r", launch?.Args ?? []);
            environment.Text = string.Join("\r", (launch?.Environment ?? []).OrderBy(entry => entry.Key, StringComparer.Ordinal).Select(entry => entry.Key + "=" + entry.Value));
            var panel = new StackPanel { Spacing = 8 };
            panel.Children.Add(Ui.Field(arguments, "按原样传给 Agent 程序，不经过 shell。"));
            panel.Children.Add(Ui.Field(environment, "只加给这个 Agent 的进程。"));
            panel.Children.Add(error);
            Element = Section("启动参数", launch is { Args.Length: > 0 } or { Environment.Count: > 0 } ? $"{launch.Args.Length} 个参数，{launch.Environment.Count} 个环境变量" : "无", panel);
        }

        static IEnumerable<string> Lines(TextBox box) => box.Text.Split('\r', '\n').Where(line => line.Trim().Length > 0);

        // Read is the launch to save, or null after saying what is wrong.
        public BindingAcpLaunch? Read()
        {
            var variables = new Dictionary<string, string>();
            foreach (var line in Lines(environment))
            {
                var at = line.IndexOf('=');
                var name = at > 0 ? line[..at].Trim() : "";
                if (name.Length == 0)
                {
                    Ui.Show(error, "环境变量要写成“名称=值”：" + line.Trim());
                    Element.IsExpanded = true;
                    return null;
                }
                variables[name] = line[(at + 1)..];
            }
            Ui.Show(error, "");
            return new BindingAcpLaunch([.. Lines(arguments)], variables);
        }
    }

    // AgentInfo reads, when asked, what a saved ACP Agent says of itself.
    Expander AgentInfo(BindingRuntimeInstance runtime)
    {
        var shown = new TextBlock { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
        var read = new Button { Content = "读取 Agent 信息" };
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(Ui.Muted("启动这个 Agent 并询问它的名称、版本、能力与登录方式。"));
        panel.Children.Add(read);
        panel.Children.Add(shown);
        read.Click += async (_, _) =>
        {
            read.IsEnabled = false;
            shown.Text = "正在读取…";
            try
            {
                var info = await Task.Run(() => core.Local.AcpRuntimeInfo(runtime.Id));
                static string Yes(bool value) => value ? "是" : "否";
                var lines = new List<string>
                {
                    info.AgentInfo is { } agent ? $"Agent：{agent.Title ?? agent.Name}（{agent.Name} {agent.Version}）" : "Agent 没有说明自己的名称与版本。",
                    $"加载会话：{Yes(info.Capabilities.LoadSession)}　列出会话：{Yes(info.Capabilities.ListSessions)}　恢复会话：{Yes(info.Capabilities.ResumeSession)}",
                    $"关闭会话：{Yes(info.Capabilities.CloseSession)}　删除会话：{Yes(info.Capabilities.DeleteSession)}",
                    info.AuthMethods.Length > 0
                        ? "登录方式：" + string.Join("、", info.AuthMethods.Select(method => method.Description is { Length: > 0 } about ? $"{method.Name}（{about}）" : method.Name))
                        : "登录方式：无",
                };
                shown.Text = string.Join("\n", lines);
            }
            catch (Exception problem)
            {
                shown.Text = Core.Describe(problem);
                Problems.Report("读取运行时「" + runtime.Name + "」的 Agent 信息", problem);
            }
            read.IsEnabled = true;
        };
        return Section("Agent 信息", "名称、版本、能力与登录方式", panel);
    }

    // StartsSection chooses what new conversations on the runtime start
    // with: the last choice made on it, its own defaults, or set ones.
    sealed class StartsSection
    {
        static readonly string[] names = ["上次的选择", "运行时默认", "指定"];
        readonly Core core;
        readonly BindingRuntimeInstance? runtime;
        readonly BindingPermissionMode[] modes;
        readonly BindingNewConversationChoice? saved;
        readonly ComboBox choice = new() { Header = "新会话开始时", HorizontalAlignment = HorizontalAlignment.Stretch };
        readonly ComboBox model = new() { Header = "模型", HorizontalAlignment = HorizontalAlignment.Stretch };
        readonly ComboBox effort = new() { Header = "推理强度", HorizontalAlignment = HorizontalAlignment.Stretch };
        readonly ComboBox permission = new() { Header = "权限", HorizontalAlignment = HorizontalAlignment.Stretch };
        readonly TextBlock state = new() { Style = Ui.Style("SecondaryTextStyle"), TextWrapping = TextWrapping.Wrap };
        readonly StackPanel chosenParts = new() { Spacing = 8 };
        // The models offered, first none: the runtime's default.
        List<(BindingModelSelection? Selection, string Name, string[] Efforts)> models = [(null, "运行时默认", [])];
        bool read;

        public Expander Element { get; }

        public StartsSection(Core core, BindingRuntimeInstance? runtime, BindingRuntimeTypeDescriptor kind, BindingRuntimeComposerChoices? choices)
        {
            this.core = core;
            this.runtime = runtime;
            modes = kind.PermissionModes;
            saved = choices?.NewConversation;
            foreach (var name in names)
            {
                choice.Items.Add(name);
            }
            var set = saved as BindingNewConversationChoice.Chosen;
            choice.SelectedIndex = set == null ? 0 : set.Choice is { Model: null, Effort: null, PermissionMode: null } ? 1 : 2;
            permission.Items.Add("运行时默认");
            foreach (var mode in modes)
            {
                permission.Items.Add(mode.Label);
            }
            permission.SelectedIndex = set?.Choice.PermissionMode is { } id ? Array.FindIndex(modes, mode => mode.Id == id) + 1 : 0;
            permission.Visibility = modes.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            if (set?.Choice.Model is { } chosen)
            {
                models.Add((chosen, Named(chosen), []));
            }
            ShowModels(set?.Choice.Model, set?.Choice.Effort);
            model.SelectionChanged += (_, _) => ShowEfforts(null);

            chosenParts.Children.Add(state);
            chosenParts.Children.Add(model);
            chosenParts.Children.Add(effort);
            chosenParts.Children.Add(permission);
            var panel = new StackPanel { Spacing = 8 };
            panel.Children.Add(Ui.Field(choice, "会话还没有在这个运行时上运行或选择过时，输入区从这里开始；之后按会话记住。"));
            panel.Children.Add(chosenParts);
            choice.SelectionChanged += (_, _) => Show();
            Element = Section("新会话", names[choice.SelectedIndex], panel);
            Show();
        }

        static string Named(BindingModelSelection selection) => selection switch
        {
            BindingModelSelection.Native native => native.ModelId,
            BindingModelSelection.Managed managed => managed.ModelRecordKey,
            _ => "模型",
        };

        void Show()
        {
            var setting = choice.SelectedIndex == 2;
            chosenParts.Visibility = setting ? Visibility.Visible : Visibility.Collapsed;
            if (!setting)
            {
                return;
            }
            if (runtime == null)
            {
                state.Text = "保存后可以指定模型；现在只能指定权限。";
                model.IsEnabled = false;
                effort.IsEnabled = false;
            }
            else if (!read)
            {
                read = true;
                ReadModels(runtime.Id);
            }
        }

        async void ReadModels(string runtimeID)
        {
            state.Text = "正在读取运行时的模型…";
            model.IsEnabled = false;
            try
            {
                var catalog = await Task.Run(() => core.Local.RuntimeModels(runtimeID));
                var keep = Selected();
                var keptEffort = effort.SelectedIndex > 0 ? effort.SelectedItem as string : null;
                models = [(null, "运行时默认", []), .. catalog.Select(entry => ((BindingModelSelection?)entry.Selection, entry.Name, entry.ReasoningEfforts))];
                if (keep is { } selection && !models.Any(entry => Equals(entry.Selection, selection)))
                {
                    models.Add((selection, Named(selection) + "（目录中没有）", []));
                }
                ShowModels(keep, keptEffort);
                state.Text = "";
            }
            catch (Exception problem)
            {
                state.Text = "无法读取模型：" + Core.Describe(problem);
            }
            model.IsEnabled = true;
        }

        BindingModelSelection? Selected() => model.SelectedIndex > 0 ? models[model.SelectedIndex].Selection : null;

        void ShowModels(BindingModelSelection? selected, string? chosenEffort)
        {
            model.Items.Clear();
            foreach (var entry in models)
            {
                model.Items.Add(entry.Name);
            }
            var at = models.FindIndex(entry => Equals(entry.Selection, selected));
            model.SelectedIndex = Math.Max(at, 0);
            ShowEfforts(chosenEffort);
        }

        void ShowEfforts(string? chosen)
        {
            var levels = model.SelectedIndex >= 0 && model.SelectedIndex < models.Count ? models[model.SelectedIndex].Efforts : [];
            if (chosen != null && !levels.Contains(chosen))
            {
                levels = [.. levels, chosen];
            }
            effort.Items.Clear();
            effort.Items.Add("运行时默认");
            foreach (var level in levels)
            {
                effort.Items.Add(level);
            }
            effort.SelectedIndex = chosen == null ? 0 : Array.IndexOf(levels, chosen) + 1;
            effort.Visibility = levels.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        }

        // Read is what to save, or null when nothing changed.
        public BindingNewConversationChoice? Read()
        {
            BindingNewConversationChoice chosen = choice.SelectedIndex switch
            {
                1 => new BindingNewConversationChoice.Chosen(new BindingTurnChoice(null, null, null)),
                2 => new BindingNewConversationChoice.Chosen(new BindingTurnChoice(
                    Selected(),
                    effort.SelectedIndex > 0 ? effort.SelectedItem as string : null,
                    permission.SelectedIndex > 0 ? modes[permission.SelectedIndex - 1].Id : null)),
                _ => new BindingNewConversationChoice.LastChosen(),
            };
            var unchanged = (saved ?? new BindingNewConversationChoice.LastChosen()) switch
            {
                BindingNewConversationChoice.LastChosen => chosen is BindingNewConversationChoice.LastChosen,
                BindingNewConversationChoice.Chosen before => chosen is BindingNewConversationChoice.Chosen after && before.Choice == after.Choice,
                _ => false,
            };
            return unchanged ? null : chosen;
        }
    }
}
