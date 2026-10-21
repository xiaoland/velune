using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// The editor's models: each one's model ID and parameters, filled by hand or
// from a model template, which is copied and not linked.
sealed partial class ProviderEditor
{
    readonly List<ModelRow> models = [];
    readonly StackPanel modelRows = new() { Spacing = 4 };

    void BuildModels()
    {
        var heading = new Grid();
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        heading.Children.Add(Ui.Heading("模型"));
        var add = new Button { Content = "添加新模型" };
        add.Click += (_, _) =>
        {
            AddModel(null, true);
            Changed();
        };
        var pick = new Button { Content = "从模板中选取…" };
        pick.Click += async (_, _) =>
        {
            foreach (var model in await ProviderFlows.PickModels(page))
            {
                AddModel(model, false);
                Changed();
            }
        };
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(0, 0, 0, 4) };
        buttons.Children.Add(add);
        buttons.Children.Add(pick);
        Grid.SetColumn(buttons, 1);
        heading.Children.Add(buttons);
        body.Children.Add(heading);
        body.Children.Add(modelsError);
        body.Children.Add(modelRows);
        foreach (var model in existing?.Models ?? [])
        {
            AddModel(model, false);
        }
    }

    void AddModel(BindingProviderModel? model, bool expanded)
    {
        var row = new ModelRow(this, model);
        row.Element.IsExpanded = expanded;
        models.Add(row);
        modelRows.Children.Add(row.Element);
    }

    void RemoveModel(ModelRow row)
    {
        models.Remove(row);
        modelRows.Children.Remove(row.Element);
        Changed();
    }

    // ModelRow is one model's expander: its model ID and parameters in the
    // header, its fields inside.
    sealed class ModelRow
    {
        readonly ProviderEditor editor;
        // The model's record, which routes and choices name; empty until saved.
        string recordKey;
        readonly string? icon;
        public Expander Element { get; } = new() { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        readonly TextBlock title = new() { TextTrimming = TextTrimming.CharacterEllipsis };
        readonly TextBlock detail = Ui.Muted("");
        readonly TextBox id = new() { Header = "模型 ID", PlaceholderText = "提供商 API 规定的标识" };
        readonly TextBox nickname = new() { Header = "昵称", PlaceholderText = "留空时显示模型 ID" };
        readonly NumberBox context = Count("上下文窗口（token）");
        readonly NumberBox output = Count("最大输出（token）");
        readonly TextBox levels = new() { Header = "支持的推理强度", PlaceholderText = "例如 low, medium, high；留空表示不声明" };

        public ModelRow(ProviderEditor editor, BindingProviderModel? model)
        {
            this.editor = editor;
            recordKey = model?.RecordKey ?? "";
            icon = model?.Icon;
            id.Text = model?.ProviderModelId ?? "";
            nickname.Text = model != null && model.Nickname != model.ProviderModelId ? model.Nickname : "";
            context.Value = model?.ContextWindow ?? double.NaN;
            output.Value = model?.MaxOutputTokens ?? double.NaN;
            levels.Text = model?.ReasoningLevels is { } declared ? string.Join(", ", declared) : "";
            detail.TextWrapping = TextWrapping.NoWrap;
            detail.TextTrimming = TextTrimming.CharacterEllipsis;
            var header = new StackPanel { Spacing = 2, Padding = new Thickness(0, 6, 0, 6) };
            header.Children.Add(title);
            header.Children.Add(detail);
            Element.Header = header;

            var fields = new Grid { ColumnSpacing = 12, RowSpacing = 12 };
            fields.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            fields.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            for (var i = 0; i < 3; i++)
            {
                fields.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            }
            Place(fields, id, 0, 0);
            Place(fields, nickname, 0, 1);
            Place(fields, context, 1, 0);
            Place(fields, output, 1, 1);
            Place(fields, levels, 2, 0);
            Grid.SetColumnSpan(levels, 2);

            var asTemplate = new Button { Content = "存为模板…" };
            asTemplate.Click += (_, _) => editor.SaveAsTemplate(Value());
            var remove = new Button { Content = "删除模型" };
            remove.Click += (_, _) => editor.RemoveModel(this);
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
            actions.Children.Add(asTemplate);
            actions.Children.Add(remove);
            var content = new StackPanel { Spacing = 12 };
            content.Children.Add(fields);
            content.Children.Add(actions);
            Element.Content = content;

            id.TextChanged += (_, _) => Edited();
            nickname.TextChanged += (_, _) => Edited();
            levels.TextChanged += (_, _) => Edited();
            context.ValueChanged += (_, _) => Edited();
            output.ValueChanged += (_, _) => Edited();
            ShowHeader();
        }

        static NumberBox Count(string header) => new()
        {
            Header = header,
            Minimum = 1,
            PlaceholderText = "未知",
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Hidden,
            NumberFormatter = new Windows.Globalization.NumberFormatting.DecimalFormatter { FractionDigits = 0, IsGrouped = true },
        };

        static void Place(Grid grid, FrameworkElement element, int row, int column)
        {
            Grid.SetRow(element, row);
            Grid.SetColumn(element, column);
            grid.Children.Add(element);
        }

        void Edited()
        {
            editor.Changed();
            ShowHeader();
        }

        public string ID => id.Text.Trim();

        // ShowHeader keeps the model ID and its declared reasoning in sight
        // while the row is folded.
        void ShowHeader()
        {
            var shownNickname = nickname.Text.Trim();
            title.Text = shownNickname.Length > 0 ? shownNickname : ID.Length > 0 ? ID : "新模型";
            var parts = new List<string>();
            if (shownNickname.Length > 0 && ID.Length > 0)
            {
                parts.Add(ID);
            }
            if (Optional(context) is { } window)
            {
                parts.Add("上下文 " + Tokens(window));
            }
            if (Optional(output) is { } most)
            {
                parts.Add("输出 " + Tokens(most));
            }
            parts.Add(Levels() is { } declared ? "推理 " + string.Join("/", declared) : "未声明推理强度");
            detail.Text = string.Join(" · ", parts);
        }

        static uint? Optional(NumberBox box) => double.IsNaN(box.Value) || box.Value < 1 ? null : (uint)Math.Min(box.Value, uint.MaxValue);

        string[]? Levels()
        {
            var declared = levels.Text.Split([',', '，', ' ', '、'], StringSplitOptions.RemoveEmptyEntries);
            return declared.Length > 0 ? declared : null;
        }

        public BindingProviderModel Value() => new(
            RecordKey: recordKey,
            ProviderModelId: ID,
            Nickname: nickname.Text.Trim() is { Length: > 0 } shown ? shown : ID,
            Icon: icon,
            ContextWindow: Optional(context),
            MaxOutputTokens: Optional(output),
            ReasoningLevels: Levels());

        // Keep takes the record a save gave a new model.
        public void Keep(BindingProviderModel[] saved)
        {
            if (recordKey.Length == 0 && saved.FirstOrDefault(model => model.ProviderModelId == ID) is { } kept)
            {
                recordKey = kept.RecordKey;
            }
        }
    }

    // Tokens is a token count in thousands or millions, as model cards write it.
    internal static string Tokens(uint count) => count switch
    {
        >= 1_000_000 when count % 1_000_000 == 0 => $"{count / 1_000_000}M",
        >= 1_000 when count % 1_000 == 0 => $"{count / 1_000}K",
        >= 1_000 => $"{count / 1000.0:0.#}K",
        _ => count.ToString(),
    };

    async void SaveAsTemplate(BindingProviderModel model)
    {
        var template = new BindingModelTemplate("", model.Nickname, model.ProviderModelId, model.Nickname, model.Icon, model.ContextWindow, model.MaxOutputTokens, model.ReasoningLevels);
        await ModelTemplatesList.Edit(page, template, true);
    }
}
