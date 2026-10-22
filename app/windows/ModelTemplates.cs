using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The model templates a provider's models fill from: a suggested model ID and
// parameters, copied when used and never linked to the models made from them.
sealed class ModelTemplatesList : UserControl
{
    readonly ProvidersPage page;
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsDoubleTapEnabled = true };
    readonly Button remove = Ui.IconButton("\uE738", "删除模型模板");
    readonly Button edit = new() { Content = "编辑…" };
    BindingModelTemplate[] templates = [];

    internal ModelTemplatesList(ProvidersPage page)
    {
        this.page = page;
        var add = Ui.IconButton("\uE710", "添加模型模板");
        add.Click += async (_, _) => await Edit(page, new BindingModelTemplate("", "", "", "", null, null, null, null), true);
        remove.Click += (_, _) => Delete();
        edit.Click += (_, _) => EditChosen();
        list.SelectionChanged += (_, _) => ShowActions();
        list.DoubleTapped += (_, _) => EditChosen();
        list.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter && Chosen() != null)
            {
                e.Handled = true;
                EditChosen();
            }
        };
        Content = Ui.ListPage("模型模板", null, list, [add, remove], [edit]);
        ShowActions();
    }

    BindingModelTemplate? Chosen() => list.SelectedIndex is var index and >= 0 && index < templates.Length ? templates[index] : null;

    void ShowActions()
    {
        remove.IsEnabled = Chosen() != null;
        edit.IsEnabled = Chosen() != null;
    }

    async void EditChosen()
    {
        if (Chosen() is { } template)
        {
            await Edit(page, template, false);
        }
    }

    internal void Show(BindingModelTemplate[] shown)
    {
        var selected = Chosen()?.Id;
        templates = [.. shown.OrderBy(template => template.Name, StringComparer.CurrentCultureIgnoreCase)];
        list.Items.Clear();
        foreach (var template in templates)
        {
            var detail = new List<string> { template.SuggestedProviderModelId };
            if (template.ContextWindow is { } window)
            {
                detail.Add("上下文 " + ProviderEditor.Tokens(window));
            }
            if (template.ReasoningLevels is { Length: > 0 } levels)
            {
                detail.Add("推理 " + string.Join("/", levels));
            }
            var row = Ui.TwoLines(template.Name, string.Join(" · ", detail));
            row.Padding = new Thickness(4, 6, 4, 6);
            var item = new ListViewItem { Content = row };
            item.ContextFlyout = Ui.Menu(
                Ui.MenuItem("编辑…", async () => await Edit(page, template, false)),
                Ui.MenuItem("删除…", () =>
                {
                    list.SelectedItem = item;
                    Delete();
                }));
            list.Items.Add(item);
        }
        list.SelectedIndex = Array.FindIndex(templates, template => template.Id == selected);
        Ui.Placeholder(list, "还没有模型模板。用＋添加，或在提供商的模型中“存为模板…”。");
        ShowActions();
    }

    async void Delete()
    {
        if (Chosen() is not { } template
            || !await Ui.Confirm(page.Window, "删除模型模板？", "删除模板「" + template.Name + "」。已从它填入的模型不受影响。", "删除"))
        {
            return;
        }
        try
        {
            page.SetModelTemplates(await Task.Run(() => page.Core.Local.DeleteModelTemplate(template.Id)));
        }
        catch (Exception problem)
        {
            Problems.Report("删除模型模板「" + template.Name + "」", problem);
        }
    }

    // Edit edits a template in a dialog, adding it when isNew; the dialog
    // stays open until it saves or the user cancels.
    internal static async Task Edit(ProvidersPage page, BindingModelTemplate template, bool isNew)
    {
        var name = new TextBox { Header = "模板名称", Text = template.Name };
        var id = new TextBox { Header = "建议的模型 ID", Text = template.SuggestedProviderModelId, PlaceholderText = "填入提供商模型时可再修改" };
        var nickname = new TextBox { Header = "昵称", Text = template.Nickname == template.SuggestedProviderModelId ? "" : template.Nickname };
        var context = Count("上下文窗口（token）", template.ContextWindow);
        var output = Count("最大输出（token）", template.MaxOutputTokens);
        var levels = new TextBox { Header = "支持的推理强度", Text = template.ReasoningLevels is { } declared ? string.Join(", ", declared) : "", PlaceholderText = "例如 low, medium, high；留空表示不声明" };
        var nameError = Ui.Error();
        var formError = Ui.Error();
        var panel = new StackPanel { Spacing = 12, MinWidth = 420 };
        panel.Children.Add(Ui.Field(name, "", nameError));
        panel.Children.Add(id);
        panel.Children.Add(nickname);
        panel.Children.Add(context);
        panel.Children.Add(output);
        panel.Children.Add(levels);
        panel.Children.Add(Ui.Muted("模板只用于快速填写；从它填入的模型是独立副本，修改模板不改变已有模型。"));
        panel.Children.Add(formError);
        var dialog = new ContentDialog
        {
            Title = isNew ? "新建模型模板" : "编辑模型模板",
            Content = new ScrollViewer { Content = panel, Padding = new Thickness(0, 0, 16, 0), MaxHeight = 480 },
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        dialog.PrimaryButtonClick += async (_, e) =>
        {
            var deferral = e.GetDeferral();
            var shownName = name.Text.Trim();
            Ui.Show(nameError, shownName.Length == 0 ? "请填写模板名称" : "");
            if (shownName.Length == 0)
            {
                e.Cancel = true;
                deferral.Complete();
                return;
            }
            var suggested = id.Text.Trim();
            var declared = levels.Text.Split([',', '，', ' ', '、'], StringSplitOptions.RemoveEmptyEntries);
            var saved = template with
            {
                Id = isNew ? "" : template.Id,
                Name = shownName,
                SuggestedProviderModelId = suggested,
                Nickname = nickname.Text.Trim() is { Length: > 0 } shown ? shown : suggested,
                ContextWindow = Optional(context),
                MaxOutputTokens = Optional(output),
                ReasoningLevels = declared.Length > 0 ? declared : null,
            };
            try
            {
                page.SetModelTemplates(await Task.Run(() => page.Core.Local.SaveModelTemplate(saved)));
            }
            catch (Exception problem)
            {
                Ui.Show(formError, Core.Describe(problem));
                Problems.Report("保存模型模板", problem);
                e.Cancel = true;
            }
            deferral.Complete();
        };
        await page.Window.ShowDialog(dialog);
    }

    static NumberBox Count(string header, uint? value) => new()
    {
        Header = header,
        Minimum = 1,
        PlaceholderText = "未知",
        Value = value ?? double.NaN,
        SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Hidden,
    };

    static uint? Optional(NumberBox box) => double.IsNaN(box.Value) || box.Value < 1 ? null : (uint)Math.Min(box.Value, uint.MaxValue);
}
