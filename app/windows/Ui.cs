using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Velune;

// The pieces the settings pages share.
static class Ui
{
    public static Style Style(string key) => (Style)Application.Current.Resources[key];

    public static TextBlock Text(string text, string style = "BodyTextBlockStyle") => new()
    {
        Text = text,
        Style = Style(style),
        TextWrapping = TextWrapping.Wrap,
    };

    public static TextBlock Muted(string text)
    {
        return Text(text, "MutedTextStyle");
    }

    public static TextBlock Error() => new()
    {
        Style = Style("ErrorTextStyle"),
        TextWrapping = TextWrapping.Wrap,
        Visibility = Visibility.Collapsed,
    };

    public static void Show(TextBlock error, string message)
    {
        error.Text = message;
        error.Visibility = message.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    // Field is a control with its explanation and what is wrong with it below.
    public static StackPanel Field(FrameworkElement control, string description = "", TextBlock? error = null)
    {
        var field = new StackPanel { Spacing = 4 };
        field.Children.Add(control);
        if (description.Length > 0)
        {
            field.Children.Add(Muted(description));
        }
        if (error != null)
        {
            field.Children.Add(error);
        }
        return field;
    }

    // Labeled is a control that has no header of its own, under its label.
    public static StackPanel Labeled(string label, FrameworkElement control)
    {
        var labeled = new StackPanel { Spacing = 4 };
        labeled.Children.Add(new TextBlock { Text = label });
        labeled.Children.Add(control);
        return labeled;
    }

    // Card is one settings row: what it is on the left, its controls on the right.
    public static Border Card(FrameworkElement body, FrameworkElement? trailing = null)
    {
        var row = new Grid { ColumnSpacing = 16 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        body.VerticalAlignment = VerticalAlignment.Center;
        row.Children.Add(body);
        if (trailing != null)
        {
            trailing.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(trailing, 1);
            row.Children.Add(trailing);
        }
        return new Border
        {
            Padding = new Thickness(16, 12, 16, 12),
            MinHeight = 64,
            CornerRadius = new CornerRadius(4),
            Style = Style("CardStyle"),
            Child = row,
        };
    }

    // TwoLines shows a title over a muted detail, trimmed to one line unless the
    // detail explains rather than summarizes.
    public static StackPanel TwoLines(string title, string detail, bool wrap = false)
    {
        var lines = new StackPanel { Spacing = 2 };
        lines.Children.Add(new TextBlock { Text = title, TextTrimming = TextTrimming.CharacterEllipsis });
        if (detail.Length > 0)
        {
            var muted = Muted(detail);
            if (!wrap)
            {
                muted.TextWrapping = TextWrapping.NoWrap;
                muted.TextTrimming = TextTrimming.CharacterEllipsis;
            }
            lines.Children.Add(muted);
        }
        return lines;
    }

    public static TextBlock Heading(string text)
    {
        var heading = Text(text, "BodyStrongTextBlockStyle");
        heading.Margin = new Thickness(0, 16, 0, 4);
        return heading;
    }

    public static Button IconButton(string glyph, string label)
    {
        var button = new Button { Content = new FontIcon { Glyph = glyph, FontSize = 14 }, Padding = new Thickness(8, 6, 8, 6) };
        ToolTipService.SetToolTip(button, label);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, label);
        return button;
    }

    public static MenuFlyoutItem MenuItem(string text, Action chosen)
    {
        var item = new MenuFlyoutItem { Text = text };
        item.Click += (_, _) => chosen();
        return item;
    }

    // ListPage is a settings page of one bordered list and, below it, its
    // actions: ＋ and − first, the others after.
    public static Grid ListPage(string title, InfoBar failure, ListView list, IEnumerable<UIElement> leading, IEnumerable<UIElement> trailing)
    {
        var page = new Grid { Padding = new Thickness(24, 12, 24, 20), RowSpacing = 12 };
        page.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        page.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        page.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        page.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        page.Children.Add(Text(title, "SubtitleTextBlockStyle"));
        Grid.SetRow(failure, 1);
        page.Children.Add(failure);
        var frame = new Border
        {
            CornerRadius = new CornerRadius(8),
            Style = Style("CardStyle"),
            Padding = new Thickness(0, 4, 0, 4),
            Child = list,
        };
        Grid.SetRow(frame, 2);
        page.Children.Add(frame);
        var actions = new Grid();
        actions.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        actions.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var left = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        foreach (var element in leading)
        {
            left.Children.Add(element);
        }
        var right = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        foreach (var element in trailing)
        {
            right.Children.Add(element);
        }
        Grid.SetColumn(right, 1);
        actions.Children.Add(left);
        actions.Children.Add(right);
        Grid.SetRow(actions, 3);
        page.Children.Add(actions);
        return page;
    }

    public static InfoBar FailureBar() => new() { Severity = InfoBarSeverity.Error, IsClosable = true };

    public static void Show(InfoBar bar, string failure)
    {
        bar.Message = failure;
        bar.IsOpen = failure.Length > 0;
    }

    // Confirm asks before a removal; the default answer is to keep.
    public static async Task<bool> Confirm(SettingsWindow window, string title, string message, string action)
    {
        var dialog = new ContentDialog
        {
            Title = title,
            Content = Text(message),
            PrimaryButtonText = action,
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };
        return await window.ShowDialog(dialog) == ContentDialogResult.Primary;
    }

    // Placeholder shows text over an empty list.
    public static void Placeholder(ListView list, string text)
    {
        list.Header = list.Items.Count == 0 ? new TextBlock
        {
            Text = text,
            Margin = new Thickness(16, 12, 16, 12),
            Style = Style("SecondaryTextStyle"),
            TextWrapping = TextWrapping.Wrap,
        } : null;
    }

    public static FlyoutBase? Menu(params MenuFlyoutItemBase[] items)
    {
        var menu = new MenuFlyout();
        foreach (var item in items)
        {
            menu.Items.Add(item);
        }
        return menu;
    }
}
