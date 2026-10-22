using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// The outline as a tool window: the focused tab's user messages; choosing
// one goes there.
sealed class OutlineTool : UserControl
{
    readonly ListView list = new() { SelectionMode = ListViewSelectionMode.Single, IsItemClickEnabled = true };
    readonly TextBlock empty = new() { Style = Ui.Style("SecondaryTextStyle"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(12, 8, 12, 8) };
    ConversationView? view;
    // The entries shown, by outline ID, so a new snapshot rebuilds only on change.
    List<(string Id, string Text)> shown = [];

    public OutlineTool()
    {
        var root = new Grid();
        root.Children.Add(list);
        root.Children.Add(empty);
        Content = root;
        list.ItemClick += (_, e) =>
        {
            if (e.ClickedItem is ListViewItem { Tag: string id })
            {
                view?.GoToEntry(id);
            }
        };
    }

    // Follow shows a view's outline and follows its changes.
    public void Follow(ConversationView? next)
    {
        if (view == next)
        {
            Render();
            return;
        }
        if (view != null)
        {
            view.Model.Changed -= Render;
        }
        view = next;
        if (view != null)
        {
            view.Model.Changed += Render;
        }
        shown = [];
        Render();
    }

    public void Render()
    {
        var entries = view?.OutlineEntries() ?? [];
        empty.Text = view == null ? "没有打开的会话。" : "还没有用户消息。";
        empty.Visibility = entries.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        if (entries.SequenceEqual(shown))
        {
            return;
        }
        shown = entries;
        list.Items.Clear();
        foreach (var (id, text) in entries)
        {
            var item = new ListViewItem
            {
                Tag = id,
                Content = new TextBlock { Text = text, TextTrimming = TextTrimming.CharacterEllipsis },
            };
            ToolTipService.SetToolTip(item, text);
            list.Items.Add(item);
        }
    }
}
