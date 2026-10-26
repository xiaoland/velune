using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Windows.System;
using Windows.UI.Core;

namespace Velune;

// What the palette lists: the commands, the conversations to open, or the
// next turn's choices.
enum PaletteMode { Commands, Conversations, Runtime, Model, Effort, Permission }

// One row of the palette. Run receives whether Ctrl+Enter chose it, which
// opens a conversation in a new tab.
sealed record PaletteItem(string Id, string Title, Action<bool> Run, string? Detail = null, string? Shortcut = null, bool Checked = false, bool Enabled = true);

// Matching typed text against titles, in order but not necessarily
// together: each word must match, and earlier, tighter matches rank higher.
static class FuzzyMatch
{
    public static int? Score(string query, IEnumerable<string?> fields)
    {
        var words = query.ToLowerInvariant().Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
        var texts = fields.OfType<string>().Select(field => field.ToLowerInvariant()).ToList();
        var total = 0;
        foreach (var word in words)
        {
            int? best = null;
            for (var i = 0; i < texts.Count; i++)
            {
                if (Score(word, texts[i]) is { } score && (best == null || score - i * 50 > best))
                {
                    best = score - i * 50;
                }
            }
            if (best is not { } found)
            {
                return null;
            }
            total += found;
        }
        return total;
    }

    static int? Score(string word, string text)
    {
        if (text.StartsWith(word, StringComparison.Ordinal))
        {
            return 3000 - text.Length;
        }
        var at = text.IndexOf(word, StringComparison.Ordinal);
        if (at >= 0)
        {
            return 2000 - at;
        }
        var gaps = 0;
        var index = 0;
        foreach (var character in word)
        {
            var found = text.IndexOf(character, index);
            if (found < 0)
            {
                return null;
            }
            gaps += found - index;
            index = found + 1;
        }
        return 1000 - gaps;
    }
}

// The palette over the top of a window: a field and the rows it narrows.
// ↑↓ choose, Enter runs (Ctrl+Enter opens a conversation in a new tab), Esc
// or a click outside closes it, and the keyboard goes back where it was.
sealed class CommandPalette
{
    const double Width = 580;
    readonly Popup popup = new() { IsLightDismissEnabled = true };
    readonly TextBox query = new() { FontSize = 18, BorderThickness = new Thickness(0), Background = null, Padding = new Thickness(12, 8, 12, 8) };
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsItemClickEnabled = true, MaxHeight = 12 * 36, Padding = new Thickness(4, 4, 4, 6) };
    readonly TextBlock empty = new() { Margin = new Thickness(16, 12, 16, 14), HorizontalAlignment = HorizontalAlignment.Center };
    readonly Func<PaletteMode, List<PaletteItem>> source;
    readonly Action focusComposer;
    List<PaletteItem> all = [];
    List<PaletteItem> shown = [];
    UIElement? returnTo;
    // Set when a choice sends the keyboard on to the composer, and when an
    // item ran, which puts the keyboard where it wants.
    bool toComposer;
    bool ran;

    public PaletteMode? Mode { get; private set; }

    public CommandPalette(Func<PaletteMode, List<PaletteItem>> source, Action focusComposer)
    {
        this.source = source;
        this.focusComposer = focusComposer;
        empty.Style = Ui.Style("SecondaryTextStyle");
        var panel = new StackPanel { Width = Width };
        panel.Children.Add(query);
        panel.Children.Add(new Border { Height = 1, Style = Ui.Style("DividerStyle") });
        panel.Children.Add(empty);
        panel.Children.Add(list);
        popup.Child = new Border { Style = Ui.Style("PaletteStyle"), Child = panel };
        query.TextChanged += (_, _) => Filter(0);
        query.PreviewKeyDown += Key;
        list.ItemClick += (_, e) =>
        {
            if (list.Items.IndexOf(e.ClickedItem) is var index and >= 0)
            {
                Run(index, false);
            }
        };
        popup.Opened += (_, _) => query.Focus(FocusState.Programmatic);
        popup.Closed += (_, _) => Closed();
    }

    public bool IsOpen => popup.IsOpen;

    // Show opens the palette in a mode, or closes it when it shows that mode.
    public void Show(PaletteMode mode, XamlRoot root)
    {
        if (Mode == mode && popup.IsOpen)
        {
            Close(false);
            return;
        }
        if (!popup.IsOpen)
        {
            returnTo = FocusManager.GetFocusedElement(root) as UIElement;
        }
        Mode = mode;
        all = source(mode);
        query.PlaceholderText = Prompt(mode);
        query.Text = "";
        Filter(Math.Max(all.FindIndex(item => item.Checked), 0));
        popup.XamlRoot = root;
        popup.HorizontalOffset = Math.Max((root.Size.Width - Width) / 2, 0);
        popup.VerticalOffset = 48;
        popup.IsOpen = true;
        query.Focus(FocusState.Programmatic);
    }

    // Close closes it; the keyboard goes back where it was, or to the composer.
    public void Close(bool composer)
    {
        toComposer = composer;
        popup.IsOpen = false;
    }

    void Closed()
    {
        Mode = null;
        var back = returnTo;
        returnTo = null;
        if (ran)
        {
            ran = false;
            return;
        }
        if (!toComposer && back is Control control && control.XamlRoot != null && control.Focus(FocusState.Programmatic))
        {
            return;
        }
        toComposer = false;
        focusComposer();
    }

    static string Prompt(PaletteMode mode) => mode switch
    {
        PaletteMode.Commands => "输入命令",
        PaletteMode.Conversations => "打开会话：按标题、项目或目录搜索",
        PaletteMode.Runtime => "下一轮的运行时",
        PaletteMode.Model => "下一轮的模型",
        PaletteMode.Effort => "下一轮的推理强度",
        _ => "下一轮的权限模式",
    };

    void Filter(int selected)
    {
        var text = query.Text.Trim();
        shown = text.Length == 0 ? all : all
            .Select((item, index) => (Item: item, Index: index, Score: FuzzyMatch.Score(text, [item.Title, item.Detail])))
            .Where(entry => entry.Score != null)
            .OrderByDescending(entry => entry.Score).ThenBy(entry => entry.Index)
            .Select(entry => entry.Item)
            .ToList();
        list.Items.Clear();
        foreach (var item in shown)
        {
            list.Items.Add(Row(item));
        }
        empty.Text = Mode switch
        {
            PaletteMode.Commands => "没有匹配的命令",
            PaletteMode.Conversations => "没有匹配的会话",
            _ => "没有可选的项",
        };
        empty.Visibility = shown.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        list.Visibility = shown.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
        if (shown.Count > 0)
        {
            list.SelectedIndex = Math.Clamp(selected, 0, shown.Count - 1);
            list.ScrollIntoView(list.SelectedItem);
        }
    }

    Grid Row(PaletteItem item)
    {
        var row = new Grid { ColumnSpacing = 8, MinHeight = 28, Opacity = item.Enabled ? 1 : 0.5 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        if (Mode != PaletteMode.Commands)
        {
            row.Children.Add(new FontIcon { Glyph = "\uE73E", FontSize = 12, Opacity = item.Checked ? 1 : 0, VerticalAlignment = VerticalAlignment.Center });
        }
        var text = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        text.Children.Add(new TextBlock { Text = item.Title, TextTrimming = TextTrimming.CharacterEllipsis });
        if (item.Detail is { Length: > 0 } detail)
        {
            text.Children.Add(new TextBlock { Text = detail, Style = Ui.Style("SecondaryTextStyle"), TextTrimming = TextTrimming.CharacterEllipsis });
        }
        Grid.SetColumn(text, 1);
        row.Children.Add(text);
        if (item.Shortcut is { Length: > 0 } shortcut)
        {
            var keys = new TextBlock { Text = shortcut, Style = Ui.Style("SecondaryTextStyle"), VerticalAlignment = VerticalAlignment.Center };
            Grid.SetColumn(keys, 2);
            row.Children.Add(keys);
        }
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, item.Title);
        return row;
    }

    void Key(object sender, KeyRoutedEventArgs e)
    {
        var control = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control).HasFlag(CoreVirtualKeyStates.Down);
        switch (e.Key)
        {
            case VirtualKey.Up or VirtualKey.Down when shown.Count > 0:
                e.Handled = true;
                list.SelectedIndex = (list.SelectedIndex + (e.Key == VirtualKey.Up ? -1 : 1) + shown.Count) % shown.Count;
                list.ScrollIntoView(list.SelectedItem);
                break;
            case VirtualKey.Enter:
                e.Handled = true;
                Run(list.SelectedIndex, control);
                break;
            case VirtualKey.Escape:
                e.Handled = true;
                Close(false);
                break;
        }
    }

    void Run(int index, bool newTab)
    {
        if (index < 0 || index >= shown.Count)
        {
            return;
        }
        var item = shown[index];
        if (!item.Enabled)
        {
            return;
        }
        // Closed first, so what the item opens or focuses keeps the keyboard.
        ran = true;
        popup.IsOpen = false;
        item.Run(newTab);
    }
}
