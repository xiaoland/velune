using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// 关联 Agent: the conversations related to the focused tab's, by group: its
// parent when it is a native sub-agent, its native sub-agents, and the
// conversations it collaborated with through Velune. A double click opens
// one as the settings say; the menu offers the other ways.
sealed class LinkedAgentsTool : UserControl
{
    readonly WorkspaceWindow window;
    readonly StackPanel groups = new() { Spacing = 4, Padding = new Thickness(0, 4, 0, 8) };
    readonly TextBlock empty = new() { Style = Ui.Style("SecondaryTextStyle"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(12, 8, 12, 8) };
    ConversationView? view;
    // What is shown, so a new snapshot rebuilds the panel only on change.
    string shown = "";

    sealed record Linked(string ConversationID, string Title, string Detail);

    public LinkedAgentsTool(WorkspaceWindow window)
    {
        this.window = window;
        var root = new Grid();
        root.Children.Add(new ScrollViewer { Content = groups });
        root.Children.Add(empty);
        Content = root;
    }

    // Follow shows a view's linked conversations and follows its changes.
    public void Follow(ConversationView? next)
    {
        if (view != next)
        {
            if (view != null)
            {
                view.Model.Changed -= Render;
            }
            view = next;
            if (view != null)
            {
                view.Model.Changed += Render;
            }
            shown = "";
        }
        Render();
    }

    // Has tells whether a view's conversation has linked conversations.
    public static bool Has(ConversationView? view) =>
        view?.Model is { } model && (model.ParentID != null || model.Snapshot is { SubAgents.Length: > 0 } or { Collaborators.Length: > 0 });

    List<(string Title, List<Linked> Items)> Groups()
    {
        var found = new List<(string, List<Linked>)>();
        if (view?.Model is not { } model)
        {
            return found;
        }
        var active = window.Listing.Active;
        string State(string id, bool failed) =>
            active.TryGetValue(id, out var running) && running.PendingInteractions > 0 ? "等待处理"
            : running?.RunState == BindingRunState.Running ? "运行中"
            : failed ? "失败"
            : "已结束";
        if (model.ParentID is { } parent)
        {
            var title = window.Listing.Conversations.FirstOrDefault(listed => listed.Conversation.Id == parent)?.Conversation.Title ?? "父会话";
            found.Add(("父会话", [new Linked(parent, title, "")]));
        }
        if (model.Snapshot is { SubAgents.Length: > 0 } withSubAgents)
        {
            found.Add(("子 Agent", [.. withSubAgents.SubAgents.Select(agent =>
                new Linked(agent.ConversationId, agent.Title.Length > 0 ? agent.Title : "子 Agent", State(agent.ConversationId, agent.Failed)))]));
        }
        if (model.Snapshot is { Collaborators.Length: > 0 } withCollaborators)
        {
            found.Add(("协作", [.. withCollaborators.Collaborators.Select(collaborator =>
            {
                var direction = collaborator.Contacted && collaborator.ContactedBy ? "互相联系"
                    : collaborator.Contacted ? "联系了对方" : "对方联系了它";
                var detail = string.Join(" · ", new[] { collaborator.RuntimeName ?? "", direction, State(collaborator.ConversationId, false) }.Where(part => part.Length > 0));
                return new Linked(collaborator.ConversationId, collaborator.Title.Length > 0 ? collaborator.Title : "会话", detail);
            })]));
        }
        return found;
    }

    public void Render()
    {
        var found = Groups();
        empty.Text = view == null ? "没有打开的会话。" : "这个会话还没有关联的 Agent。";
        empty.Visibility = found.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        var key = string.Join("\n", found.SelectMany(group => group.Items.Select(item => group.Title + "|" + item.ConversationID + "|" + item.Title + "|" + item.Detail)));
        if (key == shown)
        {
            return;
        }
        shown = key;
        groups.Children.Clear();
        foreach (var (title, items) in found)
        {
            groups.Children.Add(new TextBlock { Text = title, Style = Ui.Style("BodyStrongTextBlockStyle"), Margin = new Thickness(12, 8, 12, 2) });
            var list = new ListView { SelectionMode = ListViewSelectionMode.Single };
            foreach (var item in items)
            {
                var row = new ListViewItem { Tag = item.ConversationID, Content = Ui.TwoLines(item.Title, item.Detail) };
                ToolTipService.SetToolTip(row, item.Title);
                var id = item.ConversationID;
                row.DoubleTapped += (_, e) =>
                {
                    e.Handled = true;
                    window.OpenLinked(id);
                };
                var menu = new MenuFlyout();
                foreach (var (opening, name) in new[]
                {
                    (BindingLinkedConversationOpening.CurrentTab, "在当前标签页中打开"),
                    (BindingLinkedConversationOpening.NewTab, "在新标签页中打开"),
                    (BindingLinkedConversationOpening.Sheet, "在浮层中打开"),
                    (BindingLinkedConversationOpening.NewWindow, "在新窗口中打开"),
                })
                {
                    menu.Items.Add(Ui.MenuItem(name, () => window.OpenLinked(id, opening)));
                }
                row.ContextFlyout = menu;
                list.Items.Add(row);
            }
            groups.Children.Add(list);
        }
    }
}
