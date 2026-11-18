using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

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
    string? conversationID;
    bool completedExpanded;

    sealed record Linked(string ConversationID, string Title, string Detail);
    sealed record Group(string Title, List<Linked> Items, bool Completed = false);

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
                window.Shell.Listings.Leave(view.Model.Device, Render);
            }
            view = next;
            conversationID = null;
            completedExpanded = false;
            shown = "";
            if (view != null)
            {
                view.Model.Changed += Render;
                window.Shell.Listings.Follow(view.Model.Device, Render);
            }
        }
        Render();
    }

    // Has tells whether a view's conversation has linked conversations.
    public static bool Has(ConversationView? view) =>
        view?.Model is { } model && (model.ParentID != null || model.Snapshot is { SubAgents.Length: > 0 } or { Collaborators.Length: > 0 });

    List<Group> Groups()
    {
        var found = new List<Group>();
        if (view?.Model is not { } model)
        {
            return found;
        }
        var listing = window.Shell.Listings.Of(model.Device);
        var active = listing.Active;
        string State(string id, bool failed) =>
            active.TryGetValue(id, out var running) && running.PendingInteractions > 0 ? "等待处理"
            : running?.RunState == RunState.Running ? "运行中"
            : failed ? "失败"
            : "已结束";
        if (model.ParentID is { } parent)
        {
            var title = listing.Conversations.FirstOrDefault(listed => listed.Conversation.Id == parent)?.Conversation.Title.DisplayText() ?? "父会话";
            found.Add(new("父会话", [new Linked(parent, title, "")]));
        }
        if (model.Snapshot is { SubAgents.Length: > 0 } withSubAgents)
        {
            var running = new List<Linked>();
            var completed = new List<Linked>();
            foreach (var agent in withSubAgents.SubAgents)
            {
                var activity = active.GetValueOrDefault(agent.ConversationId);
                var waiting = activity is { PendingInteractions: > 0 };
                var working = activity?.RunState is RunState.Running or RunState.Stopping or RunState.Elsewhere;
                var detail = waiting ? "等待处理" : activity?.RunState == RunState.Stopping ? "正在停止"
                    : working ? "运行中" : agent.Failed ? "失败" : "已结束";
                var row = new Linked(agent.ConversationId, agent.Title.Length > 0 ? agent.Title : "子 Agent", detail);
                (waiting || working ? running : completed).Add(row);
            }
            if (running.Count > 0) found.Add(new("子 Agent", running));
            if (completed.Count > 0) found.Add(new($"已结束的子 Agent（{completed.Count}）", completed, Completed: true));
        }
        if (model.Snapshot is { Collaborators.Length: > 0 } withCollaborators)
        {
            found.Add(new("协作", [.. withCollaborators.Collaborators.Select(collaborator =>
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
        if (window.IsClosed) return;
        var nextID = view?.Model.Snapshot?.Conversation.Id ?? view?.Model.Target.ConversationID;
        if (conversationID != nextID)
        {
            conversationID = nextID;
            completedExpanded = false;
            shown = "";
        }
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
        foreach (var group in found)
        {
            if (group.Completed)
            {
                var disclosure = new Expander
                {
                    Header = group.Title,
                    IsExpanded = completedExpanded,
                    Content = completedExpanded ? Rows(group.Items) : null,
                    HorizontalAlignment = HorizontalAlignment.Stretch,
                    HorizontalContentAlignment = HorizontalAlignment.Stretch,
                    Margin = new Thickness(12, 4, 12, 2),
                };
                disclosure.Expanding += (_, _) =>
                {
                    completedExpanded = true;
                    disclosure.Content ??= Rows(group.Items);
                };
                disclosure.Collapsed += (_, _) => completedExpanded = false;
                groups.Children.Add(disclosure);
                continue;
            }
            groups.Children.Add(new TextBlock { Text = group.Title, Style = Ui.Style("BodyStrongTextBlockStyle"), Margin = new Thickness(12, 8, 12, 2) });
            groups.Children.Add(Rows(group.Items));
        }
    }

    ListView Rows(List<Linked> items)
    {
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
                (LinkedConversationOpening.CurrentTab, "在当前标签页中打开"),
                (LinkedConversationOpening.NewTab, "在新标签页中打开"),
                (LinkedConversationOpening.Sheet, "在浮层中打开"),
                (LinkedConversationOpening.NewWindow, "在新窗口中打开"),
            })
            {
                menu.Items.Add(Ui.MenuItem(name, () => window.OpenLinked(id, opening)));
            }
            row.ContextFlyout = menu;
            list.Items.Add(row);
        }
        return list;
    }
}
