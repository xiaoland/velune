using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Velune.Bindings;

namespace Velune;

// The right of the 资源库 window: a resource's editor, saving, and the
// runtimes it is enabled on with what each needs; or a runtime's entry
// outside the library, read-only, to add to it.
sealed partial class LibraryWindow
{
    static readonly string[] Efforts = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

    // The shown resource's form: made anew with each panel, as an element
    // has one parent.
    StackPanel enabledRows = new();
    TextBlock formError = Ui.Error();
    Button save = new();
    Button revert = new();
    int shownDetail;

    void ShowDetail()
    {
        var shown = ++shownDetail;
        if (!Equals(chosen, noticed))
        {
            notice.IsOpen = false;
        }
        if (chosen is BindingUnmanagedRuntimeResource entry)
        {
            ShowOther(entry, shown);
            return;
        }
        if (chosen is not string key)
        {
            detail.Content = Placeholder(Kinds.First(entry => entry.Kind == kind).Name);
            return;
        }
        if (Library.Drafts.TryGetValue(key, out var draft) && (draft.Dirty || draft.Id.Length == 0))
        {
            ShowDraft(draft);
            return;
        }
        ShowSaved(key, shown);
    }

    static TextBlock Placeholder(string kindName) => new()
    {
        Text = $"选择左侧的{kindName}，或用＋新建。",
        Style = Ui.Style("SecondaryTextStyle"),
        Margin = new Thickness(24),
        TextWrapping = TextWrapping.Wrap,
    };

    // ShowSaved reads a resource without edits afresh, so it shows what the
    // library holds now.
    async void ShowSaved(string id, int shown)
    {
        try
        {
            var template = await Task.Run(() => App.ReadResourceTemplate(id));
            if (shown != shownDetail || closed)
            {
                return;
            }
            var draft = Draft.From(template);
            Library.Drafts[id] = draft;
            ShowDraft(draft);
        }
        catch (Exception failure)
        {
            if (closed)
            {
                return;
            }
            Problems.Report("读取资源", failure);
            detail.Content = new TextBlock { Text = "读取失败，详见问题。", Margin = new Thickness(24), Style = Ui.Style("SecondaryTextStyle") };
        }
    }

    void ShowDraft(Draft draft)
    {
        var panel = new StackPanel { Spacing = 16, Padding = new Thickness(24, 16, 24, 24), MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Left };
        void Touched()
        {
            if (!draft.Dirty)
            {
                draft.Dirty = true;
                ShowList();
            }
            ShowSaveState(draft);
        }
        var name = new TextBox { Header = "名称", Text = draft.Name };
        name.TextChanged += (_, _) =>
        {
            draft.Name = name.Text;
            Touched();
        };
        panel.Children.Add(Ui.Field(name, draft.Kind == BindingRuntimeResourceKind.UserInstructions
            ? "只在资源库中区分各份指令。"
            : "用作原生文件名或服务器名：只能含字母、数字、-、_、.，不超过 64 个字符。"));
        panel.Children.Add(Editor(draft, Touched, readOnly: false));
        formError = Ui.Error();
        panel.Children.Add(formError);

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        save = new Button { Content = "保存", Style = Ui.Style("AccentButtonStyle") };
        save.Click += (_, _) => Save();
        revert = new Button { Content = "还原" };
        revert.Click += (_, _) => Revert();
        actions.Children.Add(save);
        actions.Children.Add(revert);
        actions.Children.Add(new TextBlock { Text = "保存后同步到已启用的运行时", Style = Ui.Style("SecondaryTextStyle"), VerticalAlignment = VerticalAlignment.Center });
        panel.Children.Add(actions);
        ShowSaveState(draft);

        var enabledHeader = new Grid { Margin = new Thickness(0, 8, 0, 0) };
        enabledHeader.Children.Add(Ui.Text("启用于", "BodyStrongTextBlockStyle"));
        var all = new Button { Content = "全部启用", HorizontalAlignment = HorizontalAlignment.Right };
        all.Click += (_, _) => EnableAll();
        enabledHeader.Children.Add(all);
        panel.Children.Add(enabledHeader);
        enabledRows = new StackPanel { Spacing = 8 };
        panel.Children.Add(enabledRows);
        panel.Children.Add(Ui.Muted("启用时写入的是已保存的内容。有任务运行时不能更改启用状态；保存的修改会在“待同步”的运行时上稍后重试。"));
        panel.Children.Add(Ui.Muted(EffectNote));
        detail.Content = panel;
        ShowEnabled();
    }

    void ShowSaveState(Draft draft)
    {
        save.IsEnabled = !busy && (draft.Dirty || draft.Id.Length == 0);
        revert.IsEnabled = !busy && draft.Dirty && draft.Id.Length > 0;
    }

    void Revert()
    {
        if (chosen is string key && Chosen is { Id.Length: > 0 } draft)
        {
            Library.Drafts.Remove(key);
            ShowList();
            ShowSaved(draft.Id, ++shownDetail);
        }
    }

    async void Save()
    {
        if (chosen is not string key || Chosen is not { } draft)
        {
            return;
        }
        BindingResourceTemplate template;
        try
        {
            template = draft.Template();
        }
        catch (FormatException wrong)
        {
            Ui.Show(formError, wrong.Message);
            return;
        }
        Ui.Show(formError, "");
        BindingResourceSaveResult? result;
        busy = true;
        ShowActions();
        ShowSaveState(draft);
        try
        {
            result = await Task.Run(() => App.SaveResourceTemplate(template));
        }
        catch (Exception failure)
        {
            if (closed)
            {
                return;
            }
            // What the core rejects is about the form: say it there too.
            Problems.Report("保存资源“" + template.Name + "”", failure);
            Ui.Show(formError, Core.Describe(failure));
            return;
        }
        finally
        {
            busy = false;
            if (!closed)
            {
                ShowActions();
                ShowSaveState(draft);
            }
        }
        // Saved even if the window closed meanwhile: the draft is the library's now.
        Library.Drafts.Remove(key);
        draft.Id = result.Summary.Id;
        draft.Dirty = false;
        Library.Drafts[draft.Id] = draft;
        if (closed)
        {
            return;
        }
        chosen = draft.Id;
        Report("已保存。", "已保存，但以下运行时未能同步：", result.Reports);
        Reload(keepDetail: true);
        ShowDraft(draft);
    }

    // Report sums up what writing did on each runtime: what it could not
    // write goes to 问题 as well.
    void Report(string lead, string failedLead, IEnumerable<BindingTemplateApplyReport> reports)
    {
        var lines = new List<string>();
        var failed = false;
        foreach (var report in reports)
        {
            var runtime = RuntimeName(report.RuntimeId);
            var outcome = report.Outcome switch
            {
                BindingTemplateApplyOutcome.Create => "已写入",
                BindingTemplateApplyOutcome.Update => "已更新",
                BindingTemplateApplyOutcome.Unchanged => "无变化",
                BindingTemplateApplyOutcome.Conflict => "有同名的原生内容，未写入",
                BindingTemplateApplyOutcome.Skipped skipped => "跳过：" + skipped.Reason,
                BindingTemplateApplyOutcome.Failed failure => "失败：" + failure.Reason,
                _ => "",
            };
            if (report.Outcome is BindingTemplateApplyOutcome.Failed or BindingTemplateApplyOutcome.Skipped)
            {
                failed = true;
                Problems.Report("同步到“" + runtime + "”", outcome);
            }
            lines.Add($"{runtime}：{outcome}" + (report.Notes.Length > 0 ? "（" + string.Join("；", report.Notes) + "）" : ""));
        }
        noticed = chosen;
        notice.Severity = failed ? InfoBarSeverity.Warning : InfoBarSeverity.Success;
        Ui.Show(notice, lines.Count > 0 ? (failed ? failedLead : lead) + "\n" + string.Join("\n", lines) + (failed ? "" : "\n" + EffectNote) : lead);
    }

    // ShowEnabled lists the runtimes with the chosen resource's state on
    // each and what can be done about it.
    void ShowEnabled()
    {
        enabledRows.Children.Clear();
        if (Chosen is not { } draft)
        {
            return;
        }
        if (draft.Id.Length == 0)
        {
            enabledRows.Children.Add(Ui.Muted("创建后可以启用到运行时。"));
            return;
        }
        if (runtimes.Length == 0)
        {
            enabledRows.Children.Add(Ui.Muted("还没有 Agent 运行时。"));
            return;
        }
        foreach (var runtime in runtimes)
        {
            var state = State(draft.Id, runtime.Id);
            var row = new Grid { ColumnSpacing = 12 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var check = new CheckBox
            {
                Content = runtime.Name + (runtime.Enabled ? "" : "（已停用）"),
                IsChecked = state is not (BindingResourceSyncState.NotEnabled or BindingResourceSyncState.Unsupported),
                IsEnabled = !busy && state is not BindingResourceSyncState.Unsupported,
            };
            check.Click += (_, _) =>
            {
                if (check.IsChecked == true)
                {
                    Enable(draft, [runtime]);
                }
                else
                {
                    Disable(draft, runtime);
                }
            };
            var lines = new StackPanel { Spacing = 2 };
            lines.Children.Add(check);
            var (text, actions) = StateParts(draft, runtime, state);
            if (text.Length > 0)
            {
                var status = Ui.Muted(text);
                status.Margin = new Thickness(28, 0, 0, 0);
                if (state is BindingResourceSyncState.Drifted or BindingResourceSyncState.Missing)
                {
                    status.Style = Ui.Style("ErrorTextStyle");
                }
                lines.Children.Add(status);
            }
            row.Children.Add(lines);
            if (actions != null)
            {
                Grid.SetColumn(actions, 1);
                row.Children.Add(actions);
            }
            enabledRows.Children.Add(new Border { Style = Ui.Style("CardStyle"), CornerRadius = new CornerRadius(4), Padding = new Thickness(12, 8, 12, 8), Child = row });
        }
    }

    (string Text, FrameworkElement? Actions) StateParts(Draft draft, BindingRuntimeInstance runtime, BindingResourceSyncState state)
    {
        Button Action(string text, Action act)
        {
            var button = new Button { Content = text, IsEnabled = !busy };
            button.Click += (_, _) => act();
            return button;
        }
        DropDownButton Menu(string text, params MenuFlyoutItem[] items)
        {
            var menu = new MenuFlyout();
            foreach (var item in items)
            {
                menu.Items.Add(item);
            }
            return new DropDownButton { Content = text, Flyout = menu, IsEnabled = !busy };
        }
        return state switch
        {
            BindingResourceSyncState.Unsupported unsupported => (unsupported.Reason, null),
            BindingResourceSyncState.Synced => ("已同步", null),
            BindingResourceSyncState.Pending => ("待同步：运行时空闲后自动写入", Action("立即同步", () => Enable(draft, [runtime]))),
            BindingResourceSyncState.Drifted drifted => ("原生内容已被修改：" + string.Join("、", drifted.Paths), Menu("处理",
                Ui.MenuItem("比较…", () => Compare(draft, runtime)),
                Ui.MenuItem("用资源库的内容覆盖", () => Resolve(draft, runtime, BindingResolution.Overwrite)),
                Ui.MenuItem("用原生内容更新资源库…", () => TakeNative(draft, runtime)),
                Ui.MenuItem("不再同步，保留原生内容", () => Resolve(draft, runtime, BindingResolution.Unmanage)))),
            BindingResourceSyncState.Missing missing => ("原生内容已被删除：" + string.Join("、", missing.Paths), Menu("处理",
                Ui.MenuItem("用资源库的内容重新写入", () => Resolve(draft, runtime, BindingResolution.Overwrite)),
                Ui.MenuItem("不再同步", () => Resolve(draft, runtime, BindingResolution.Unmanage)))),
            _ => ("", null),
        };
    }

    async void EnableAll()
    {
        if (Chosen is not { Id.Length: > 0 } draft)
        {
            return;
        }
        var targets = runtimes.Where(runtime => State(draft.Id, runtime.Id) is BindingResourceSyncState.NotEnabled or BindingResourceSyncState.Pending).ToList();
        if (targets.Count == 0)
        {
            Inform("已启用于所有可用的运行时。");
            return;
        }
        await EnableOn(draft, targets);
    }

    async void Enable(Draft draft, BindingRuntimeInstance[] targets) => await EnableOn(draft, targets);

    // EnableOn writes the saved resource to runtimes; where a native entry
    // of the same name, or another instruction, is in the way, it asks
    // before replacing it.
    async Task EnableOn(Draft draft, IReadOnlyList<BindingRuntimeInstance> targets)
    {
        var reports = await Write("启用“" + draft.Name + "”", application =>
            application.ApplyResourceTemplate(draft.Id, [.. targets.Select(runtime => new BindingTemplateApplyTarget(runtime.Id, false))]));
        if (reports == null)
        {
            Reload(keepDetail: true);
            return;
        }
        var shown = reports.ToList();
        foreach (var report in reports.Where(report => report.Outcome is BindingTemplateApplyOutcome.Conflict))
        {
            var conflict = (BindingTemplateApplyOutcome.Conflict)report.Outcome;
            var runtime = RuntimeName(report.RuntimeId);
            var paths = string.Join("、", conflict.Unmanaged.Concat(conflict.Drifted));
            var message = draft.Kind == BindingRuntimeResourceKind.UserInstructions
                ? $"{runtime} 已有指令文件或已启用另一份指令（{paths}）。每个运行时只能启用一份指令，继续将替换它。"
                : $"{runtime} 中已有同名的原生内容（{paths}），启用后将被资源库的内容替换。";
            if (await Ask("覆盖已有的原生内容？", message, "覆盖") != ContentDialogResult.Primary)
            {
                continue;
            }
            if (await Write("覆盖“" + runtime + "”中的内容", application =>
                application.ApplyResourceTemplate(draft.Id, [new BindingTemplateApplyTarget(report.RuntimeId, true)])) is { } again)
            {
                shown.Remove(report);
                shown.AddRange(again);
            }
        }
        Report("已启用。", "未能全部启用：", shown);
        Reload(keepDetail: true);
    }

    // Disable stops syncing to a runtime, deleting what Velune wrote there
    // or leaving it as the runtime's own.
    async void Disable(Draft draft, BindingRuntimeInstance runtime)
    {
        var answer = await Ask($"停用于“{runtime.Name}”", "删除原生内容：从该运行时移除 Velune 写入的内容。\n保留原生内容：原生内容留在该运行时，但不再由 Velune 同步。", "删除原生内容", "保留原生内容");
        if (answer == ContentDialogResult.None)
        {
            ShowEnabled();
            return;
        }
        Resolve(draft, runtime, answer == ContentDialogResult.Primary ? BindingResolution.Remove : BindingResolution.Unmanage);
    }

    async void Resolve(Draft draft, BindingRuntimeInstance runtime, BindingResolution resolution)
    {
        var what = resolution switch
        {
            BindingResolution.Overwrite => "用资源库的内容覆盖“" + runtime.Name + "”",
            BindingResolution.Remove => "从“" + runtime.Name + "”删除",
            BindingResolution.Unmanage => "停止同步到“" + runtime.Name + "”",
            _ => "用“" + runtime.Name + "”的内容更新资源库",
        };
        if (await Write(what, application => application.ResolveResourceBinding(runtime.Id, draft.Id, resolution)))
        {
            Inform(what + "：已完成。" + (resolution == BindingResolution.Overwrite ? "\n" + EffectNote : ""));
        }
        Reload(keepDetail: true);
    }

    // TakeNative replaces the resource with the runtime's changed content,
    // after listing what the library cannot hold of it.
    async void TakeNative(Draft draft, BindingRuntimeInstance runtime)
    {
        if (await Write("读取“" + runtime.Name + "”中的内容", application => application.ResourceNativeImport(runtime.Id, draft.Id)) is not { } preview)
        {
            return;
        }
        var lines = new List<string> { $"资源库中的“{draft.Name}”将改为 {runtime.Name} 中的内容，并按保存同步到其他已启用的运行时。" };
        if (preview.Notes.Length > 0)
        {
            lines.Add("以下原生内容无法带入，将被丢弃：\n" + string.Join("\n", preview.Notes));
        }
        if (draft.Dirty)
        {
            lines.Add("此资源未保存的修改也将被替换。");
        }
        if (await Ask("用原生内容更新资源库？", string.Join("\n\n", lines), "更新资源库") != ContentDialogResult.Primary)
        {
            return;
        }
        if (await Write("用“" + runtime.Name + "”的内容更新资源库", application => application.ResolveResourceBinding(runtime.Id, draft.Id, BindingResolution.TakeNative)))
        {
            draft.Dirty = false;
            Inform($"已用 {runtime.Name} 中的内容更新“{draft.Name}”。");
        }
        Reload();
    }

    // Compare shows, file by file, the runtime's changed content beside
    // what overwriting would write.
    async void Compare(Draft draft, BindingRuntimeInstance runtime)
    {
        if (await Write("比较“" + runtime.Name + "”中的内容", application => application.ResourceDrift(runtime.Id, draft.Id)) is not { } files)
        {
            return;
        }
        var panel = new StackPanel { Spacing = 16 };
        foreach (var file in files)
        {
            panel.Children.Add(Ui.Text(file.Path, "BodyStrongTextBlockStyle"));
            if (file.Binary)
            {
                panel.Children.Add(Ui.Muted("二进制文件，不显示内容。"));
                continue;
            }
            var sides = new Grid { ColumnSpacing = 12 };
            sides.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            sides.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            sides.Children.Add(Side("当前原生内容", file.Native, "原生侧没有这个文件；覆盖将创建它。"));
            var library = Side("覆盖后（资源库）", file.Library, "资源库不再包含这个文件；覆盖将删除它。");
            Grid.SetColumn(library, 1);
            sides.Children.Add(library);
            panel.Children.Add(sides);
        }
        if (files.Length == 0)
        {
            panel.Children.Add(Ui.Muted("没有差异。"));
        }
        var dialog = new ContentDialog
        {
            Title = $"{runtime.Name} 中的“{draft.Name}”",
            Content = new ScrollViewer { Content = panel, MaxHeight = 480 },
            PrimaryButtonText = "用资源库的内容覆盖",
            SecondaryButtonText = "用原生内容更新资源库…",
            CloseButtonText = "关闭",
            DefaultButton = ContentDialogButton.Close,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 960.0;
        switch (await Show(dialog))
        {
            case ContentDialogResult.Primary:
                Resolve(draft, runtime, BindingResolution.Overwrite);
                break;
            case ContentDialogResult.Secondary:
                TakeNative(draft, runtime);
                break;
        }

        static StackPanel Side(string title, string? text, string absent)
        {
            var side = new StackPanel { Spacing = 4 };
            side.Children.Add(Ui.Muted(title));
            side.Children.Add(text == null ? Ui.Muted(absent) : new TextBox
            {
                Text = text,
                IsReadOnly = true,
                AcceptsReturn = true,
                TextWrapping = TextWrapping.NoWrap,
                FontFamily = new FontFamily("Cascadia Mono, Consolas"),
                MaxHeight = 320,
                MinWidth = 380,
            });
            return side;
        }
    }

    // ShowOther shows a runtime's entry outside the library as the library
    // would hold it, with what it would leave out, to add it.
    async void ShowOther(BindingUnmanagedRuntimeResource entry, int shown)
    {
        var runtime = RuntimeName(entry.RuntimeId);
        var panel = new StackPanel { Spacing = 16, Padding = new Thickness(24, 16, 24, 24), MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Left };
        panel.Children.Add(Ui.Text(entry.Path, "SubtitleTextBlockStyle"));
        panel.Children.Add(Ui.Muted($"{runtime} 中的 {entry.Path} 不在资源库中，Velune 不编辑它。加入资源库后即可编辑，并可启用到其他运行时。"));
        var adopt = new Button { Content = "加入资源库…", Style = Ui.Style("AccentButtonStyle"), IsEnabled = false };
        panel.Children.Add(adopt);
        detail.Content = panel;
        BindingResourceTemplateImport preview;
        try
        {
            preview = await Task.Run(() => App.ImportRuntimeResource(entry.RuntimeId, entry.Kind, entry.Path));
        }
        catch (Exception failure)
        {
            if (shown == shownDetail && !closed)
            {
                Problems.Report("读取“" + entry.Path + "”", failure);
                panel.Children.Add(Ui.Muted("读取失败，详见问题。"));
            }
            return;
        }
        if (shown != shownDetail || closed)
        {
            return;
        }
        if (preview.Notes.Length > 0)
        {
            panel.Children.Add(Ui.Muted("资源库无法表达，不会带入：" + string.Join("；", preview.Notes)));
        }
        panel.Children.Add(Editor(Draft.From(preview.Template), () => { }, readOnly: true));
        adopt.IsEnabled = true;
        adopt.Click += async (_, _) =>
        {
            var message = (preview.Notes.Length > 0
                ? $"以下内容资源库无法表达，不会带入；{runtime} 中的原生文件将改写为资源库的形式：\n{string.Join("\n", preview.Notes)}\n\n"
                : $"{runtime} 中的原生文件将改写为资源库的形式。\n\n") + "加入后它继续启用于该运行时，并由 Velune 保持同步。";
            if (await Ask("加入资源库", message, "加入资源库") != ContentDialogResult.Primary)
            {
                return;
            }
            if (await Write("把“" + entry.Path + "”加入资源库", application => application.AdoptRuntimeResource(entry.RuntimeId, entry.Kind, entry.Path)) is { } added)
            {
                chosen = added.Id;
                Reload();
            }
        };
    }

    // Editor is the fields of a draft's kind; every edit goes into the
    // draft at once.
    FrameworkElement Editor(Draft draft, Action touched, bool readOnly)
    {
        var panel = new StackPanel { Spacing = 12 };
        TextBox Text(string header, string value, Action<string> set, bool lines = false, string placeholder = "")
        {
            var box = new TextBox
            {
                Header = header,
                Text = value,
                IsReadOnly = readOnly,
                PlaceholderText = placeholder,
                AcceptsReturn = lines,
                TextWrapping = lines ? TextWrapping.Wrap : TextWrapping.NoWrap,
                MinHeight = lines ? 120 : 0,
                MaxHeight = lines ? 480 : double.PositiveInfinity,
            };
            ScrollViewer.SetVerticalScrollBarVisibility(box, ScrollBarVisibility.Auto);
            box.TextChanged += (_, _) =>
            {
                if (!readOnly)
                {
                    set(box.Text);
                    touched();
                }
            };
            return box;
        }
        switch (draft.Kind)
        {
            case BindingRuntimeResourceKind.UserInstructions:
                var body = Text("内容", draft.Body, value => draft.Body = value, lines: true);
                body.MinHeight = 280;
                panel.Children.Add(Ui.Field(body, "启用后整体写入运行时的用户指令文件；每个运行时只能启用一份指令。"));
                break;
            case BindingRuntimeResourceKind.Skills:
                var entry = Text("SKILL.md", draft.Body, value => draft.Body = value, lines: true);
                entry.FontFamily = new FontFamily("Cascadia Mono, Consolas");
                entry.MinHeight = 280;
                panel.Children.Add(Ui.Field(entry, "技能的入口，以 YAML frontmatter（name、description）开头。"));
                if (draft.Files.Length > 0)
                {
                    panel.Children.Add(Ui.Muted("其他文件随资源保留，在此不编辑：" + string.Join("、", draft.Files.Select(file => file.Path))));
                }
                break;
            case BindingRuntimeResourceKind.McpServers:
                panel.Children.Add(McpEditor(draft, touched, readOnly));
                break;
            default:
                panel.Children.Add(Text("描述", draft.Description, value => draft.Description = value, lines: true));
                var instructions = Text("指令", draft.Instructions, value => draft.Instructions = value, lines: true);
                instructions.MinHeight = 200;
                panel.Children.Add(instructions);
                panel.Children.Add(SubagentModelField(draft, touched, readOnly));
                var effort = new ComboBox { Header = "推理强度", MinWidth = 200, IsEnabled = !readOnly };
                effort.Items.Add(new ComboBoxItem { Content = "不指定", Tag = "" });
                foreach (var level in Efforts.Union(draft.Effort.Length > 0 ? [draft.Effort] : []))
                {
                    effort.Items.Add(new ComboBoxItem { Content = level, Tag = level });
                }
                effort.SelectedItem = effort.Items.OfType<ComboBoxItem>().First(item => (string)item.Tag == draft.Effort);
                effort.SelectionChanged += (_, _) =>
                {
                    draft.Effort = (string)((ComboBoxItem)effort.SelectedItem).Tag;
                    touched();
                };
                panel.Children.Add(Ui.Field(effort, "各运行时只写入它接受的级别，其余在写入报告中说明。"));
                panel.Children.Add(Ui.Field(Text("工具", draft.Tools, value => draft.Tools = value, placeholder: "例如 Read, Grep"), "以逗号分隔；留空沿用运行时的默认。"));
                panel.Children.Add(SkillChoices(draft, touched, readOnly));
                break;
        }
        return panel;
    }

    FrameworkElement SubagentModelField(Draft draft, Action touched, bool readOnly)
    {
        var field = new StackPanel { Spacing = 8 };
        var choice = new RadioButtons { Header = "模型", IsEnabled = !readOnly };
        foreach (var name in new[] { "沿用父会话", "原生模型 ID", "Velune 模型" })
        {
            choice.Items.Add(name);
        }
        choice.SelectedIndex = (int)draft.Model;
        var native = new TextBox { Text = draft.NativeModel, PlaceholderText = "运行时提供商的 model ID", IsReadOnly = readOnly };
        native.TextChanged += (_, _) =>
        {
            if (!readOnly)
            {
                draft.NativeModel = native.Text;
                touched();
            }
        };
        var velune = new ComboBox { MinWidth = 320, PlaceholderText = "选择 Velune 模型", IsEnabled = !readOnly };
        foreach (var provider in gateways.SelectMany(gateway => gateway.Providers))
        {
            foreach (var model in provider.Models)
            {
                velune.Items.Add(new ComboBoxItem { Content = $"{provider.Name} · {(model.Nickname.Length > 0 ? model.Nickname : model.ProviderModelId)}", Tag = model.RecordKey });
            }
        }
        if (draft.VeluneModel.Length > 0 && !velune.Items.OfType<ComboBoxItem>().Any(item => (string)item.Tag == draft.VeluneModel))
        {
            velune.Items.Add(new ComboBoxItem { Content = "已移除的模型", Tag = draft.VeluneModel });
        }
        velune.SelectedItem = velune.Items.OfType<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == draft.VeluneModel);
        velune.SelectionChanged += (_, _) =>
        {
            if (velune.SelectedItem is ComboBoxItem { Tag: string key })
            {
                draft.VeluneModel = key;
                touched();
            }
        };
        var help = Ui.Muted("");
        void Show()
        {
            native.Visibility = draft.Model == SubagentModel.Native ? Visibility.Visible : Visibility.Collapsed;
            velune.Visibility = draft.Model == SubagentModel.Velune ? Visibility.Visible : Visibility.Collapsed;
            help.Text = draft.Model switch
            {
                SubagentModel.Native => "写入原生定义；经 Velune 网关的轮次中，由 Velune 中唯一具有此 model ID 的模型提供。",
                SubagentModel.Velune => "不写入原生定义，只提供给经 Velune 网关的轮次；运行时用自己的提供商时看不到它。",
                _ => "子 Agent 使用父会话的模型。",
            };
        }
        choice.SelectionChanged += (_, _) =>
        {
            if (choice.SelectedIndex >= 0 && (SubagentModel)choice.SelectedIndex != draft.Model)
            {
                draft.Model = (SubagentModel)choice.SelectedIndex;
                touched();
                Show();
            }
        };
        Show();
        field.Children.Add(choice);
        field.Children.Add(native);
        field.Children.Add(velune);
        field.Children.Add(help);
        return field;
    }

    FrameworkElement SkillChoices(Draft draft, Action touched, bool readOnly)
    {
        var field = new StackPanel { Spacing = 4 };
        field.Children.Add(new TextBlock { Text = "技能" });
        var names = templates.Where(template => template.Kind == BindingRuntimeResourceKind.Skills).Select(template => template.Name)
            .Union(draft.Skills).OrderBy(name => name, StringComparer.CurrentCulture).ToList();
        if (names.Count == 0)
        {
            field.Children.Add(Ui.Muted("资源库中还没有技能。"));
        }
        foreach (var name in names)
        {
            var check = new CheckBox { Content = name, IsChecked = draft.Skills.Contains(name), IsEnabled = !readOnly };
            check.Click += (_, _) =>
            {
                draft.Skills.Remove(name);
                if (check.IsChecked == true)
                {
                    draft.Skills.Add(name);
                }
                touched();
            };
            field.Children.Add(check);
        }
        field.Children.Add(Ui.Muted("子 Agent 启动时载入所选技能；目前只有 Claude Code 支持，技能也需启用于同一运行时。"));
        return field;
    }

    // McpEditor edits a server's fields, or its JSON declaration; secrets
    // stay masked until shown.
    FrameworkElement McpEditor(Draft draft, Action touched, bool readOnly)
    {
        var panel = new StackPanel { Spacing = 12 };
        var fields = new StackPanel { Spacing = 12 };
        var json = new TextBox
        {
            Header = "JSON",
            AcceptsReturn = true,
            IsReadOnly = readOnly,
            FontFamily = new FontFamily("Cascadia Mono, Consolas"),
            MinHeight = 240,
            Visibility = Visibility.Collapsed,
        };
        var jsonNote = Ui.Muted("");
        var asJson = new ToggleSwitch { Header = "以 JSON 编辑", OnContent = "JSON", OffContent = "字段" };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(asJson, "以 JSON 编辑");
        void ShowFields()
        {
            fields.Children.Clear();
            var transport = new RadioButtons { Header = "连接方式", MaxColumns = 2, IsEnabled = !readOnly };
            transport.Items.Add("本地命令");
            transport.Items.Add("HTTP");
            transport.SelectedIndex = draft.Http ? 1 : 0;
            transport.SelectionChanged += (_, _) =>
            {
                if (transport.SelectedIndex >= 0 && (transport.SelectedIndex == 1) != draft.Http)
                {
                    draft.Http = transport.SelectedIndex == 1;
                    touched();
                    ShowFields();
                }
            };
            fields.Children.Add(transport);
            TextBox Line(string header, string value, Action<string> set, bool lines = false)
            {
                var box = new TextBox { Header = header, Text = value, IsReadOnly = readOnly, AcceptsReturn = lines, MinHeight = lines ? 80 : 0 };
                box.TextChanged += (_, _) =>
                {
                    if (!readOnly)
                    {
                        set(box.Text);
                        touched();
                    }
                };
                return box;
            }
            if (draft.Http)
            {
                fields.Children.Add(Line("URL", draft.Url, value => draft.Url = value));
                fields.Children.Add(Pairs("请求头", draft.Headers, touched, readOnly));
            }
            else
            {
                fields.Children.Add(Line("命令", draft.Command, value => draft.Command = value));
                fields.Children.Add(Ui.Field(Line("参数", draft.Args, value => draft.Args = value, lines: true), "每行一个，不经 shell。"));
                fields.Children.Add(Ui.Field(Line("工作目录", draft.Cwd, value => draft.Cwd = value), "可选；不支持的运行时在写入报告中说明。"));
                fields.Children.Add(Pairs("环境变量", draft.Env, touched, readOnly));
            }
            fields.Children.Add(Ui.Field(Line("超时（秒）", draft.Timeout, value => draft.Timeout = value), "一次工具调用最长的时间；可选。"));
        }
        asJson.Toggled += (_, _) =>
        {
            if (asJson.IsOn)
            {
                json.Text = draft.McpJson();
                jsonNote.Text = "可粘贴文档中的声明：单个服务器的对象，或含一个服务器的 mcpServers。超时在字段中设定。";
                json.Visibility = Visibility.Visible;
                fields.Visibility = Visibility.Collapsed;
                return;
            }
            if (!readOnly)
            {
                var (error, ignored) = draft.ReadMcpJson(json.Text);
                if (error != null)
                {
                    jsonNote.Text = error;
                    asJson.IsOn = true;
                    return;
                }
                jsonNote.Text = ignored.Length > 0 ? "未带入的字段：" + string.Join("、", ignored) : "";
                touched();
                ShowList();
            }
            json.Visibility = Visibility.Collapsed;
            fields.Visibility = Visibility.Visible;
            ShowFields();
        };
        ShowFields();
        panel.Children.Add(asJson);
        panel.Children.Add(fields);
        panel.Children.Add(json);
        panel.Children.Add(jsonNote);
        return panel;
    }

    // Pairs edits names and values; the values may be secrets and stay
    // masked unless the user looks.
    static FrameworkElement Pairs(string header, List<Pair> pairs, Action touched, bool readOnly)
    {
        var panel = new StackPanel { Spacing = 6 };
        panel.Children.Add(new TextBlock { Text = header });
        var rows = new StackPanel { Spacing = 6 };
        panel.Children.Add(rows);
        void ShowRows()
        {
            rows.Children.Clear();
            foreach (var pair in pairs)
            {
                var row = new Grid { ColumnSpacing = 8 };
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1.4, GridUnitType.Star) });
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                var key = new TextBox { Text = pair.Key, PlaceholderText = "名称", IsReadOnly = readOnly };
                key.TextChanged += (_, _) =>
                {
                    pair.Key = key.Text;
                    touched();
                };
                var value = new PasswordBox { Password = pair.Value, PlaceholderText = "值", PasswordRevealMode = PasswordRevealMode.Peek, IsEnabled = !readOnly };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(value, (pair.Key.Length > 0 ? pair.Key : header) + " 的值");
                value.PasswordChanged += (_, _) =>
                {
                    pair.Value = value.Password;
                    touched();
                };
                Grid.SetColumn(value, 1);
                var remove = Ui.IconButton("\uE711", "移除");
                remove.IsEnabled = !readOnly;
                remove.Click += (_, _) =>
                {
                    pairs.Remove(pair);
                    touched();
                    ShowRows();
                };
                Grid.SetColumn(remove, 2);
                row.Children.Add(key);
                row.Children.Add(value);
                row.Children.Add(remove);
                rows.Children.Add(row);
            }
        }
        ShowRows();
        if (!readOnly)
        {
            var add = new HyperlinkButton { Content = "添加" };
            add.Click += (_, _) =>
            {
                pairs.Add(new Pair());
                ShowRows();
            };
            panel.Children.Add(add);
        }
        return panel;
    }
}
