using Markdig;
using Markdig.Extensions.Tables;
using Markdig.Extensions.TaskLists;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using MdBlock = Markdig.Syntax.Block;
using MdInline = Markdig.Syntax.Inlines.Inline;
using XamlBlock = Microsoft.UI.Xaml.Documents.Block;
using XamlInline = Microsoft.UI.Xaml.Documents.Inline;

namespace Velune;

// Markdown draws the model's Markdown with the app's own text elements:
// selectable rich text for prose, monospace cards for code.
static class Markdown
{
    static readonly MarkdownPipeline pipeline = new MarkdownPipelineBuilder()
        .UsePipeTables()
        .UseEmphasisExtras()
        .UseTaskLists()
        .UseAutoLinks()
        .Build();

    public static UIElement View(string text)
    {
        var panel = new StackPanel { Spacing = 8 };
        foreach (var block in Markdig.Markdown.Parse(text, pipeline))
        {
            panel.Children.Add(Block(block));
        }
        return panel;
    }

    static UIElement Block(MdBlock block) => block switch
    {
        HeadingBlock heading => Prose(Paragraph(heading.Inline, heading.Level switch
        {
            1 => 20,
            2 => 17,
            _ => 15,
        }, FontWeights.SemiBold)),
        ParagraphBlock paragraph => Prose(Paragraph(paragraph.Inline)),
        FencedCodeBlock or CodeBlock => Code(((LeafBlock)block).Lines.ToString()),
        QuoteBlock quote => Quote(quote),
        ListBlock list => List(list),
        ThematicBreakBlock => new Border { Height = 1, Margin = new Thickness(0, 4, 0, 4), Background = Brush("DividerStrokeColorDefaultBrush") },
        Table table => Table(table),
        HtmlBlock html => Prose(Plain(html.Lines.ToString())),
        ContainerBlock container => Container(container),
        LeafBlock leaf => Prose(Plain(leaf.Lines.ToString())),
        _ => new StackPanel(),
    };

    static StackPanel Container(ContainerBlock container)
    {
        var panel = new StackPanel { Spacing = 8 };
        foreach (var child in container)
        {
            panel.Children.Add(Block(child));
        }
        return panel;
    }

    static RichTextBlock Prose(params XamlBlock[] blocks)
    {
        var text = new RichTextBlock { IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap };
        foreach (var block in blocks)
        {
            text.Blocks.Add(block);
        }
        return text;
    }

    static Paragraph Plain(string text)
    {
        var paragraph = new Paragraph();
        paragraph.Inlines.Add(new Run { Text = text.TrimEnd() });
        return paragraph;
    }

    static Paragraph Paragraph(ContainerInline? inlines, double? size = null, Windows.UI.Text.FontWeight? weight = null)
    {
        var paragraph = new Paragraph();
        if (size is { } fontSize)
        {
            paragraph.FontSize = fontSize;
        }
        if (weight is { } fontWeight)
        {
            paragraph.FontWeight = fontWeight;
        }
        if (inlines != null)
        {
            Inlines(inlines, paragraph.Inlines);
        }
        return paragraph;
    }

    static void Inlines(ContainerInline container, InlineCollection into)
    {
        foreach (var inline in container)
        {
            if (Inline(inline) is { } converted)
            {
                into.Add(converted);
            }
        }
    }

    static XamlInline? Inline(MdInline inline)
    {
        switch (inline)
        {
            case LiteralInline literal:
                return new Run { Text = literal.Content.ToString() };
            case CodeInline code:
                return new Run { Text = code.Content, FontFamily = Mono() };
            case LineBreakInline lineBreak:
                return lineBreak.IsHard ? new LineBreak() : new Run { Text = " " };
            case EmphasisInline emphasis:
                Span span = emphasis.DelimiterChar switch
                {
                    '~' => new Span { TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough },
                    '*' or '_' when emphasis.DelimiterCount >= 2 => new Bold(),
                    '*' or '_' => new Italic(),
                    _ => new Span(),
                };
                Inlines(emphasis, span.Inlines);
                return span;
            case LinkInline { IsImage: true } image:
                return new Run { Text = image.FirstChild is LiteralInline alt ? alt.Content.ToString() : image.Url ?? "" };
            case LinkInline link:
                var hyperlink = new Hyperlink();
                if (Uri.TryCreate(link.Url, UriKind.Absolute, out var uri) && (uri.Scheme == Uri.UriSchemeHttp || uri.Scheme == Uri.UriSchemeHttps))
                {
                    hyperlink.NavigateUri = uri;
                }
                Inlines(link, hyperlink.Inlines);
                if (hyperlink.Inlines.Count == 0)
                {
                    hyperlink.Inlines.Add(new Run { Text = link.Url ?? "" });
                }
                return hyperlink;
            case AutolinkInline autolink:
                var auto = new Hyperlink();
                if (Uri.TryCreate(autolink.Url, UriKind.Absolute, out var target))
                {
                    auto.NavigateUri = target;
                }
                auto.Inlines.Add(new Run { Text = autolink.Url });
                return auto;
            case HtmlInline html:
                return new Run { Text = html.Tag };
            case HtmlEntityInline entity:
                return new Run { Text = entity.Transcoded.ToString() };
            case ContainerInline container:
                var group = new Span();
                Inlines(container, group.Inlines);
                return group;
            default:
                return null;
        }
    }

    public static FontFamily Mono() => (FontFamily)Application.Current.Resources["MonoFont"];

    public static Brush Brush(string key) => (Brush)Application.Current.Resources[key];

    // Code is preformatted text on a card, scrolled sideways rather than wrapped.
    public static UIElement Code(string text, double size = 13) => new Border
    {
        Background = Brush("CardBackgroundFillColorSecondaryBrush"),
        BorderBrush = Brush("CardStrokeColorDefaultBrush"),
        BorderThickness = new Thickness(1),
        CornerRadius = new CornerRadius(6),
        Child = new ScrollViewer
        {
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalScrollMode = ScrollMode.Auto,
            VerticalScrollMode = ScrollMode.Disabled,
            Content = new TextBlock
            {
                Text = text.TrimEnd('\n', '\r').Replace("\t", "    "),
                FontFamily = Mono(),
                FontSize = size,
                IsTextSelectionEnabled = true,
                Padding = new Thickness(12, 8, 12, 8),
            },
        },
    };

    static UIElement Quote(QuoteBlock quote) => new Border
    {
        BorderBrush = Brush("AccentFillColorDefaultBrush"),
        BorderThickness = new Thickness(3, 0, 0, 0),
        Padding = new Thickness(12, 0, 0, 0),
        Child = Container(quote),
        Opacity = 0.85,
    };

    static UIElement List(ListBlock list)
    {
        var panel = new StackPanel { Spacing = 4 };
        var number = int.TryParse(list.OrderedStart, out var start) ? start : 1;
        foreach (var item in list.OfType<ListItemBlock>())
        {
            var marker = list.IsOrdered ? $"{number++}." : "•";
            if (item.FirstOrDefault() is ParagraphBlock { Inline.FirstChild: TaskList task })
            {
                marker = task.Checked ? "☑" : "☐";
            }
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(list.IsOrdered ? 24 : 12) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var bullet = new TextBlock { Text = marker, HorizontalAlignment = list.IsOrdered ? HorizontalAlignment.Right : HorizontalAlignment.Center };
            var body = Container(item);
            Grid.SetColumn(body, 1);
            row.Children.Add(bullet);
            row.Children.Add(body);
            panel.Children.Add(row);
        }
        return panel;
    }

    static UIElement Table(Table table)
    {
        var grid = new Grid { BorderBrush = Brush("CardStrokeColorDefaultBrush"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(6) };
        var columns = table.OfType<TableRow>().Select(row => row.Count).DefaultIfEmpty(0).Max();
        for (var column = 0; column < columns; column++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        }
        var index = 0;
        foreach (var row in table.OfType<TableRow>())
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var column = 0;
            foreach (var cell in row.OfType<TableCell>())
            {
                var text = Prose([.. cell.OfType<ParagraphBlock>().Select(paragraph => (XamlBlock)Paragraph(paragraph.Inline, null, row.IsHeader ? FontWeights.SemiBold : null))]);
                var border = new Border
                {
                    Padding = new Thickness(8, 4, 8, 4),
                    BorderBrush = Brush("DividerStrokeColorDefaultBrush"),
                    BorderThickness = new Thickness(0, index == 0 ? 0 : 1, column == 0 ? 0 : 1, 0),
                    Child = text,
                };
                Grid.SetRow(border, index);
                Grid.SetColumn(border, column++);
                grid.Children.Add(border);
            }
            index++;
        }
        return new ScrollViewer
        {
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalScrollMode = ScrollMode.Auto,
            VerticalScrollMode = ScrollMode.Disabled,
            Content = grid,
        };
    }
}
