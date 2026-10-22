using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;

namespace Velune;

// How to open a conversation in a window's editor.
enum Opening { CurrentTab, NewTab, SplitRight, SplitDown }

// A tab: one conversation in a pane. It keeps its view once shown, so its
// draft, reading place and what is expanded go wherever the tab goes.
sealed class Tab
{
    public Target Target { get; set; }
    public ConversationView? View { get; set; }
    public TabViewItem Item { get; }
    // The tab's content. The tab bar shows a selected tab's content as it was
    // when selected, so the view changes inside this host, not the item.
    public Border Host { get; } = new();
    // The conversations this tab showed before and after, for 后退 and 前进.
    public Stack<Target> Back { get; } = [];
    public Stack<Target> Forward { get; } = [];
    // Stops the window showing the tab from following its view's changes.
    public Action? Unfollow { get; set; }
    // What the tab's icon shows: waiting, running or nothing.
    public string State { get; set; } = "";

    public Tab(Target target)
    {
        Target = target;
        Item = new TabViewItem { Tag = this, Content = Host };
    }

    public string Title => View?.Model.Title ?? (Target.Unsent ? "新会话" : Target.Title);

    public TabState Save() => new(Target.RuntimeID, Target.ConversationID, Title);
}

abstract class EditorNode
{
    public Split? Parent { get; set; }
}

// A split lays its children side by side, or top to bottom when vertical,
// in proportion to their ratios.
sealed class Split(bool vertical) : EditorNode
{
    public bool Vertical { get; } = vertical;
    public List<EditorNode> Children { get; } = [];
    public List<double> Ratios { get; } = [];
}

// A pane is one tab group: Windows' own tab bar, with ＋ for a new tab.
sealed class Pane : EditorNode
{
    public TabView View { get; }
    public ToggleButton OutlineButton { get; }
    // 关联 Agent's button, while it is one and the tab has linked conversations.
    public Button LinkedButton { get; }

    public Pane(Editor editor)
    {
        View = new TabView
        {
            TabWidthMode = TabViewWidthMode.Equal,
            CloseButtonOverlayMode = TabViewCloseButtonOverlayMode.OnPointerOver,
            IsAddTabButtonVisible = true,
            CanReorderTabs = true,
            // Tabs drag to other panes and windows, and out of all of them
            // to a window of their own.
            CanDragTabs = true,
            AllowDropTabs = true,
            AllowDrop = true,
            VerticalAlignment = VerticalAlignment.Stretch,
        };
        // The outline's button, while the outline is one: on the tab bar, the
        // pane's toolbar, since the tabs already show the title.
        OutlineButton = new ToggleButton
        {
            Content = new FontIcon { Glyph = "\uE8FD", FontSize = 14 },
            Padding = new Thickness(8, 6, 8, 6),
            Margin = new Thickness(4, 0, 8, 0),
            VerticalAlignment = VerticalAlignment.Center,
            Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
        };
        ToolTipService.SetToolTip(OutlineButton, "大纲");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(OutlineButton, "大纲");
        OutlineButton.Click += (_, _) => editor.OutlineClicked(this);
        LinkedButton = new Button
        {
            Content = new FontIcon { Glyph = "\uE716", FontSize = 14 },
            Padding = new Thickness(8, 6, 8, 6),
            VerticalAlignment = VerticalAlignment.Center,
            Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
        };
        ToolTipService.SetToolTip(LinkedButton, "关联 Agent");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(LinkedButton, "关联 Agent");
        LinkedButton.Click += (_, _) =>
        {
            editor.Focus(this);
            editor.Window.ShowLinked(this);
        };
        OutlineButton.Margin = new Thickness(0, 0, 8, 0);
        View.TabStripFooter = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 2,
            Margin = new Thickness(4, 0, 0, 0),
            Children = { LinkedButton, OutlineButton },
        };
        View.AddTabButtonClick += (_, _) => editor.NewTab(this, null);
        View.TabCloseRequested += (_, e) =>
        {
            if (e.Tab.Tag is Tab tab)
            {
                editor.Close(tab, false);
            }
        };
        View.SelectionChanged += (_, _) => editor.Selected(this);
        // Dragging a tab along the bar reorders it.
        View.TabItemsChanged += (_, _) => editor.Reordered();
        View.TabDragStarting += (_, e) => editor.Window.TabDragStarting(e);
        View.TabDroppedOutside += (_, e) => editor.Window.TabDroppedOutside(e);
        View.TabDragCompleted += (_, _) => DragSource.Clear();
        View.TabStripDragOver += (_, e) => editor.Window.DragOverPane(this, e, true);
        View.TabStripDrop += (_, e) => editor.Window.DropOnPane(this, e, true);
        View.DragOver += (_, e) => editor.Window.DragOverPane(this, e, false);
        View.Drop += (_, e) => editor.Window.DropOnPane(this, e, false);
        // Any press in the pane focuses it, even one a control handles.
        View.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, _) => editor.Focus(this)), true);
        View.GotFocus += (_, _) => editor.Focus(this);
    }

    public IEnumerable<Tab> Tabs => View.TabItems.OfType<TabViewItem>().Select(item => (Tab)item.Tag);

    // SelectedItem follows SelectedIndex only once the tab bar is loaded, so
    // a pane that is still being built reads its selection from the index.
    public Tab? Selected =>
        (View.SelectedItem ?? (View.SelectedIndex >= 0 && View.SelectedIndex < Count ? View.TabItems[View.SelectedIndex] : null)) is TabViewItem { Tag: Tab tab } ? tab : null;

    public int Count => View.TabItems.Count;
}

// Editor is a window's editing area: a tree of splits whose leaves are panes.
// The window shows its root; one pane has the focus, which commands and the
// docked tools follow.
sealed class Editor
{
    const double MinPane = 160;

    readonly WorkspaceWindow window;
    readonly Border host;
    EditorNode root;

    public WorkspaceWindow Window => window;
    // The grids built for the splits, emptied before each rebuild so their
    // panes can move: an element has one parent at a time.
    readonly List<Grid> grids = [];

    public Pane Focused { get; private set; }
    // Restoring selects tabs before the window has this editor; it shows
    // them once it does.
    readonly bool ready;

    public Editor(WorkspaceWindow window, Border host, EditorState state, int focus)
    {
        this.window = window;
        this.host = host;
        root = Restore(state);
        root.Parent = null;
        var panes = Panes().ToList();
        Focused = focus >= 0 && focus < panes.Count ? panes[focus] : panes[0];
        Build();
        ready = true;
    }

    // An editor of one tab moved from another window, which keeps its view.
    public Editor(WorkspaceWindow window, Border host, Tab adopted)
    {
        this.window = window;
        this.host = host;
        var pane = new Pane(this);
        Add(pane, adopted, 0);
        pane.View.SelectedIndex = 0;
        root = pane;
        Focused = pane;
        Build();
        ready = true;
    }

    EditorNode Restore(EditorState state)
    {
        if (state is SplitState { Children.Count: > 1 } saved)
        {
            var split = new Split(saved.Vertical);
            for (var i = 0; i < saved.Children.Count; i++)
            {
                var child = Restore(saved.Children[i]);
                child.Parent = split;
                split.Children.Add(child);
                split.Ratios.Add(i < saved.Ratios.Count && saved.Ratios[i] > 0 ? saved.Ratios[i] : 1);
            }
            return split;
        }
        if (state is SplitState { Children.Count: 1 } single)
        {
            return Restore(single.Children[0]);
        }
        var pane = new Pane(this);
        var tabs = state is PaneState { Tabs.Count: > 0 } kept ? kept.Tabs : [new TabState("", "", "")];
        foreach (var tab in tabs)
        {
            Add(pane, new Tab(new Target(tab.RuntimeId, tab.ConversationId, tab.Title)), pane.Count);
        }
        pane.View.SelectedIndex = state is PaneState { } at ? Math.Clamp(at.Selected, 0, pane.Count - 1) : 0;
        return pane;
    }

    public EditorState Save() => Save(root);

    static EditorState Save(EditorNode node) => node switch
    {
        Split split => new SplitState { Vertical = split.Vertical, Children = [.. split.Children.Select(Save)], Ratios = [.. split.Ratios] },
        Pane pane => new PaneState { Tabs = [.. pane.Tabs.Select(tab => tab.Save())], Selected = Math.Max(pane.View.SelectedIndex, 0) },
        _ => new PaneState(),
    };

    public int FocusIndex => Panes().ToList().IndexOf(Focused);

    // Panes are the panes in reading order.
    public IEnumerable<Pane> Panes() => Panes(root);

    static IEnumerable<Pane> Panes(EditorNode node) => node switch
    {
        Pane pane => [pane],
        Split split => split.Children.SelectMany(Panes),
        _ => [],
    };

    public IEnumerable<Tab> Tabs() => Panes().SelectMany(pane => pane.Tabs);

    public Pane? PaneOf(Tab tab) => Panes().FirstOrDefault(pane => pane.View.TabItems.Contains(tab.Item));

    // Find is the tab showing a conversation, if one does.
    public Tab? Find(string conversationID) => Tabs().FirstOrDefault(tab => !tab.Target.Unsent && tab.Target.ConversationID == conversationID);

    // Build shows the tree in the window.
    public void Build()
    {
        foreach (var grid in grids)
        {
            grid.Children.Clear();
        }
        grids.Clear();
        host.Child = null;
        host.Child = Element(root);
    }

    FrameworkElement Element(EditorNode node)
    {
        if (node is Pane pane)
        {
            return pane.View;
        }
        var split = (Split)node;
        var grid = new Grid();
        grids.Add(grid);
        for (var i = 0; i < split.Children.Count; i++)
        {
            var size = new GridLength(split.Ratios[i], GridUnitType.Star);
            if (split.Vertical)
            {
                grid.RowDefinitions.Add(new RowDefinition { Height = size, MinHeight = MinPane / 2 });
            }
            else
            {
                grid.ColumnDefinitions.Add(new ColumnDefinition { Width = size, MinWidth = MinPane });
            }
            var child = Element(split.Children[i]);
            Place(child, split.Vertical, i);
            grid.Children.Add(child);
            if (i > 0)
            {
                var sash = SplitSash(grid, split, i);
                Place(sash, split.Vertical, i);
                grid.Children.Add(sash);
            }
        }
        return grid;
    }

    static void Place(FrameworkElement element, bool vertical, int index)
    {
        if (vertical)
        {
            Grid.SetRow(element, index);
        }
        else
        {
            Grid.SetColumn(element, index);
        }
    }

    // SplitSash resizes the two children either side of it.
    Sash SplitSash(Grid grid, Split split, int index)
    {
        double[]? start = null;
        double Size(int at) => split.Vertical ? grid.RowDefinitions[at].ActualHeight : grid.ColumnDefinitions[at].ActualWidth;
        void Set(int at, double value)
        {
            var length = new GridLength(value, GridUnitType.Star);
            if (split.Vertical)
            {
                grid.RowDefinitions[at].Height = length;
            }
            else
            {
                grid.ColumnDefinitions[at].Width = length;
            }
        }
        return new Sash(!split.Vertical, delta =>
        {
            if (start == null)
            {
                // Sizes in pixels from here on, so stars and pixels agree.
                start = [.. Enumerable.Range(0, split.Children.Count).Select(Size)];
                for (var i = 0; i < start.Length; i++)
                {
                    Set(i, start[i]);
                }
            }
            var pair = start[index - 1] + start[index];
            var before = Math.Clamp(start[index - 1] + delta, Math.Min(MinPane, pair / 2), Math.Max(pair - MinPane, pair / 2));
            Set(index - 1, before);
            Set(index, pair - before);
        }, () =>
        {
            start = null;
            var sizes = Enumerable.Range(0, split.Children.Count).Select(Size).ToList();
            var total = sizes.Sum();
            if (total > 0)
            {
                for (var i = 0; i < sizes.Count; i++)
                {
                    split.Ratios[i] = sizes[i] / total * split.Children.Count;
                }
            }
            window.LayoutChanged();
        }, () =>
        {
            var even = (split.Ratios[index - 1] + split.Ratios[index]) / 2;
            split.Ratios[index - 1] = even;
            split.Ratios[index] = even;
            Set(index - 1, even);
            Set(index, even);
            window.LayoutChanged();
        }, line: true);
    }

    public void Focus(Pane pane)
    {
        if (ready && Focused != pane)
        {
            Focused = pane;
            window.FocusChanged();
        }
    }

    public void Selected(Pane pane)
    {
        if (!ready)
        {
            return;
        }
        if (pane.Selected is { } tab)
        {
            window.Show(tab);
        }
        if (pane == Focused)
        {
            window.FocusChanged();
        }
        window.LayoutChanged();
    }

    public void Reordered()
    {
        if (ready)
        {
            window.LayoutChanged();
        }
    }

    public void OutlineClicked(Pane pane)
    {
        Focus(pane);
        if (pane.Selected?.View is { } view)
        {
            view.OutlineClicked(pane.OutlineButton);
        }
        else
        {
            pane.OutlineButton.IsChecked = false;
        }
    }

    void Add(Pane pane, Tab tab, int at)
    {
        // The menu acts in the window the tab is in now.
        var menu = new MenuFlyout();
        menu.Opening += (_, _) => window.FillTabMenu(menu, tab);
        tab.Item.ContextFlyout = menu;
        // A restored tab shows its saved title until its view reads the conversation.
        tab.Item.Header ??= tab.Title;
        pane.View.TabItems.Insert(Math.Clamp(at, 0, pane.Count), tab.Item);
    }

    // NewTab opens a tab after the pane's selected one, unsent when no target.
    public Tab NewTab(Pane pane, Target? target)
    {
        var tab = new Tab(target ?? new Target("", "", ""));
        var at = pane.Selected is { } selected ? pane.View.TabItems.IndexOf(selected.Item) + 1 : pane.Count;
        Add(pane, tab, at);
        pane.View.SelectedItem = tab.Item;
        Focus(pane);
        window.LayoutChanged();
        return tab;
    }

    // Select shows a tab and focuses its pane.
    public void Select(Tab tab)
    {
        if (PaneOf(tab) is { } pane)
        {
            pane.View.SelectedItem = tab.Item;
            Focus(pane);
        }
    }

    // Close closes a tab. A pane closes with its last tab; the window keeps
    // a new tab instead of an empty editor, unless asked to close with it.
    public void Close(Tab tab, bool closeWindow)
    {
        if (PaneOf(tab) is not { } pane)
        {
            return;
        }
        var last = pane.Count == 1 && root == pane;
        // A conversation window closes with its last tab.
        if (last && (closeWindow || window.IsConversationWindow))
        {
            window.Close();
            return;
        }
        tab.View?.Close();
        tab.View = null;
        pane.View.TabItems.Remove(tab.Item);
        if (pane.Count == 0)
        {
            if (root == pane)
            {
                NewTab(pane, null);
            }
            else
            {
                Remove(pane);
            }
        }
        window.LayoutChanged();
    }

    public void CloseOthers(Tab tab)
    {
        foreach (var other in Tabs().Where(other => other != tab).ToList())
        {
            Close(other, false);
        }
    }

    // Detach takes a tab out of its pane, keeping its view, to put it elsewhere.
    Pane? Detach(Tab tab)
    {
        if (PaneOf(tab) is not { } pane)
        {
            return null;
        }
        pane.View.TabItems.Remove(tab.Item);
        return pane;
    }

    // Release takes a tab out to put it in another pane or window. A pane
    // left empty closes; an empty editor keeps a new tab, or a conversation
    // window closes.
    public void Release(Tab tab)
    {
        if (Detach(tab) is not { } pane)
        {
            return;
        }
        if (pane.Count == 0)
        {
            if (root != pane)
            {
                Remove(pane);
            }
            else if (window.IsConversationWindow)
            {
                window.Close();
                return;
            }
            else
            {
                NewTab(pane, null);
            }
        }
        window.FocusChanged();
        window.LayoutChanged();
    }

    // Insert puts a tab in a pane at an index and shows it there.
    public void Insert(Pane pane, Tab tab, int at)
    {
        Add(pane, tab, at);
        pane.View.SelectedItem = tab.Item;
        Focus(pane);
        window.Show(tab);
        window.FocusChanged();
        window.LayoutChanged();
    }

    // SplitWith puts a tab in a new pane beside a pane: after it, to the
    // right or below, or before it.
    public void SplitWith(Pane pane, bool vertical, Tab tab, bool before = false)
    {
        var added = new Pane(this);
        Add(added, tab, 0);
        added.View.SelectedIndex = 0;
        if (pane.Parent is { } parent && parent.Vertical == vertical)
        {
            var at = parent.Children.IndexOf(pane);
            var half = parent.Ratios[at] / 2;
            parent.Ratios[at] = half;
            parent.Children.Insert(before ? at : at + 1, added);
            parent.Ratios.Insert(before ? at : at + 1, half);
            added.Parent = parent;
        }
        else
        {
            var split = new Split(vertical) { Parent = pane.Parent };
            Replace(pane, split);
            split.Children.AddRange(before ? [added, pane] : [pane, added]);
            split.Ratios.AddRange([1, 1]);
            pane.Parent = split;
            added.Parent = split;
        }
        Build();
        Focus(added);
        window.Show(tab);
        window.LayoutChanged();
    }

    // MoveToSplit moves a tab of a pane with others into a new pane beside it.
    public void MoveToSplit(Tab tab, bool vertical)
    {
        if (PaneOf(tab) is not { Count: > 1 } pane)
        {
            return;
        }
        Detach(tab);
        SplitWith(pane, vertical, tab);
    }

    void Replace(EditorNode old, EditorNode replacement)
    {
        if (old.Parent is { } parent)
        {
            parent.Children[parent.Children.IndexOf(old)] = replacement;
            replacement.Parent = parent;
        }
        else
        {
            root = replacement;
            replacement.Parent = null;
        }
    }

    // Remove takes an empty pane out of the tree; a split left with one
    // child gives way to it.
    void Remove(Pane pane)
    {
        if (pane.Parent is not { } parent)
        {
            return;
        }
        var at = parent.Children.IndexOf(pane);
        parent.Children.RemoveAt(at);
        parent.Ratios.RemoveAt(at);
        if (parent.Children.Count == 1)
        {
            Replace(parent, parent.Children[0]);
        }
        Build();
        if (Focused == pane)
        {
            Focus(Panes().ElementAt(Math.Max(0, Math.Min(at, Panes().Count() - 1))));
        }
    }

    // Step moves the focus to the next or previous pane.
    public void StepPane(int by)
    {
        var panes = Panes().ToList();
        var at = panes.IndexOf(Focused);
        var next = panes[((at + by) % panes.Count + panes.Count) % panes.Count];
        Focus(next);
        next.Selected?.View?.FocusInput();
    }

    // StepTab selects the next or previous tab of the focused pane.
    public void StepTab(int by)
    {
        var count = Focused.Count;
        if (count > 1)
        {
            Focused.View.SelectedIndex = ((Focused.View.SelectedIndex + by) % count + count) % count;
        }
    }

    public void CloseAll()
    {
        foreach (var tab in Tabs().ToList())
        {
            tab.View?.Close();
            tab.View = null;
        }
    }
}
