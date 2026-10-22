using Microsoft.UI.Xaml;
using Windows.Graphics;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// A window's tool windows and its three dock areas: where each tool is,
// which areas are open and how big. The tools share one set of states and
// operations; each tool's content is built once and moves between hosts.
sealed class ToolWindows
{
    const double MinSide = 180;
    const double MinBottom = 80;

    readonly WorkspaceWindow window;
    readonly Dictionary<Area, Border> hosts;
    readonly ColumnDefinition left;
    readonly ColumnDefinition right;
    readonly RowDefinition bottom;
    readonly Dictionary<Tool, FrameworkElement> contents;
    // The panels built for the areas; emptied before each rebuild so the
    // tools' content can move.
    readonly List<Border> slots = [];
    readonly List<Sash> sashes = [];
    // The tools floating or in windows of their own.
    readonly Dictionary<Tool, ToolHost> outside = [];
    readonly Grid body;
    readonly Grid center;

    public ToolLayout Layout { get; private set; }

    public ToolWindows(WorkspaceWindow window, ToolLayout layout, Grid body, Grid center, Dictionary<Area, Border> hosts,
        ColumnDefinition left, ColumnDefinition right, RowDefinition bottom, Dictionary<Tool, FrameworkElement> contents)
    {
        this.window = window;
        Layout = layout;
        this.body = body;
        this.center = center;
        this.hosts = hosts;
        this.left = left;
        this.right = right;
        this.bottom = bottom;
        this.contents = contents;
    }

    public static string Name(Tool tool) => tool switch
    {
        Tool.List => "会话列表",
        Tool.Outline => "会话大纲",
        _ => "输入区",
    };

    static string AreaName(Area area) => area switch
    {
        Area.Left => "左侧",
        Area.Right => "右侧",
        _ => "底部",
    };

    // AreaOpen tells whether an area shows: open, with a tool to show.
    public bool AreaOpen(Area area) => !Layout.Areas[area].Collapsed && Layout.InArea(area).Count > 0;

    // Build shows the layout: each area's tools stacked along it, each
    // with a title bar when it shares the area.
    public void Build()
    {
        foreach (var slot in slots)
        {
            slot.Child = null;
        }
        slots.Clear();
        foreach (var sash in sashes)
        {
            (sash.Parent as Panel)?.Children.Remove(sash);
        }
        sashes.Clear();
        foreach (var area in Enum.GetValues<Area>())
        {
            hosts[area].Child = null;
        }
        // A tool leaving its window, or changing between floating and a
        // window of its own, gets its content back first.
        foreach (var (tool, host) in outside.ToList())
        {
            if (!Outside(tool) || host.Place != Layout.Tools[tool].Place || host.Closing)
            {
                outside.Remove(tool);
                host.Replace();
            }
        }
        foreach (var area in Enum.GetValues<Area>())
        {
            var open = AreaOpen(area);
            var size = Layout.Areas[area].Size;
            var tools = Layout.InArea(area);
            // The composer alone in the bottom area is as tall as it is.
            var fitted = area == Area.Bottom && tools is [Tool.Composer];
            switch (area)
            {
                case Area.Left:
                    left.Width = new GridLength(open ? size : 0);
                    break;
                case Area.Right:
                    right.Width = new GridLength(open ? size : 0);
                    break;
                default:
                    bottom.Height = open ? fitted ? GridLength.Auto : new GridLength(size) : new GridLength(0);
                    break;
            }
            hosts[area].Visibility = open ? Visibility.Visible : Visibility.Collapsed;
            if (!open)
            {
                continue;
            }
            hosts[area].Child = Stack(area, tools);
            if (!fitted)
            {
                AreaSash(area);
            }
        }
        foreach (var tool in Enum.GetValues<Tool>().Where(tool => Outside(tool) && !outside.ContainsKey(tool)))
        {
            var state = Layout.Tools[tool];
            var host = new ToolHost(window, tool, state.Place, state.Bounds, contents[tool]);
            outside[tool] = host;
            host.AppWindow.Show(false);
        }
    }

    bool Outside(Tool tool) => Layout.Tools[tool] is { Shown: true, Place: Place.Floating or Place.Window };

    // Released docks a floating tool let go of near an edge of its window,
    // or snaps it to the window or another floating tool nearby; a window of
    // its own only keeps where it is.
    public void Released(ToolHost host)
    {
        var state = Layout.Tools[host.Tool];
        state.Bounds = host.Bounds;
        if (host.Place == Place.Floating)
        {
            var scale = Native.Scale(window);
            var frame = Frame(window.AppWindow);
            var cursor = Native.Cursor();
            var reach = (int)(80 * scale);
            var within = cursor.Y >= frame.Y && cursor.Y <= frame.Y + frame.Height + reach;
            Area? dock = within && Math.Abs(cursor.X - frame.X) <= reach ? Area.Left
                : within && Math.Abs(cursor.X - (frame.X + frame.Width)) <= reach ? Area.Right
                : cursor.X >= frame.X && cursor.X <= frame.X + frame.Width && Math.Abs(cursor.Y - (frame.Y + frame.Height)) <= reach ? Area.Bottom
                : null;
            if (dock is { } area)
            {
                Move(host.Tool, Place.Dock, area);
                return;
            }
            Snap(host, frame, (int)(14 * scale));
            state.Bounds = host.Bounds;
        }
        Changed();
    }

    static RectInt32 Frame(Microsoft.UI.Windowing.AppWindow window) =>
        new(window.Position.X, window.Position.Y, window.Size.Width, window.Size.Height);

    // Snap aligns a floating tool's edges with its window's, inside or out,
    // or with another snapped floating tool's, when they are close.
    void Snap(ToolHost host, RectInt32 frame, int reach)
    {
        var tool = Frame(host.AppWindow);
        int? x = null;
        int? y = null;
        void Near(ref int? at, int edge, int target, int place)
        {
            if (at == null && Math.Abs(edge - target) <= reach)
            {
                at = place;
            }
        }
        var right = frame.X + frame.Width;
        var bottom = frame.Y + frame.Height;
        Near(ref x, tool.X + tool.Width, frame.X, frame.X - tool.Width);
        Near(ref x, tool.X, right, right);
        Near(ref x, tool.X, frame.X, frame.X);
        Near(ref x, tool.X + tool.Width, right, right - tool.Width);
        Near(ref y, tool.Y, frame.Y, frame.Y);
        Near(ref y, tool.Y + tool.Height, bottom, bottom - tool.Height);
        Near(ref y, tool.Y, bottom, bottom);
        Near(ref y, tool.Y + tool.Height, frame.Y, frame.Y - tool.Height);
        foreach (var other in outside.Values.Where(other => other != host && other.Place == Place.Floating && other.Snapped))
        {
            var them = Frame(other.AppWindow);
            Near(ref x, tool.X, them.X + them.Width, them.X + them.Width);
            Near(ref x, tool.X + tool.Width, them.X, them.X - tool.Width);
            Near(ref y, tool.Y, them.Y, them.Y);
            Near(ref y, tool.Y + tool.Height, them.Y + them.Height, them.Y + them.Height - tool.Height);
        }
        if (x != null || y != null)
        {
            host.Snap(frame, new PointInt32(x ?? tool.X, y ?? tool.Y));
        }
        else
        {
            host.Unsnap();
        }
    }

    // OwnerMoved brings the snapped floating tools along with their window.
    public void OwnerMoved()
    {
        var frame = Frame(window.AppWindow);
        foreach (var host in outside.Values)
        {
            host.OwnerMoved(frame);
        }
    }

    // HostClosed hides a tool whose window the user closed; the composer
    // returns to each pane.
    public void HostClosed(Tool tool, Bounds bounds)
    {
        outside.Remove(tool);
        if (window.IsClosed)
        {
            return;
        }
        var state = Layout.Tools[tool];
        state.Bounds = bounds;
        if (tool == Tool.Composer)
        {
            state.Place = Place.InPane;
        }
        else
        {
            state.Shown = false;
        }
        Apply();
    }

    // CloseAll closes the tool windows with their window.
    public void CloseAll()
    {
        foreach (var host in outside.Values.ToList())
        {
            host.Replace();
        }
        outside.Clear();
    }

    Grid Stack(Area area, List<Tool> tools)
    {
        var across = area == Area.Bottom;
        var grid = new Grid();
        for (var i = 0; i < tools.Count; i++)
        {
            var tool = tools[i];
            var share = new GridLength(Math.Max(Layout.Tools[tool].Share, 0.1), GridUnitType.Star);
            if (across)
            {
                grid.ColumnDefinitions.Add(new ColumnDefinition { Width = share, MinWidth = 120 });
            }
            else
            {
                grid.RowDefinitions.Add(new RowDefinition { Height = share, MinHeight = 80 });
            }
            var panel = Panel(tool, tools.Count > 1);
            if (across)
            {
                Grid.SetColumn(panel, i);
            }
            else
            {
                Grid.SetRow(panel, i);
            }
            grid.Children.Add(panel);
            if (i > 0)
            {
                var sash = ShareSash(grid, tools, i, across);
                if (across)
                {
                    Grid.SetColumn(sash, i);
                }
                else
                {
                    Grid.SetRow(sash, i);
                }
                grid.Children.Add(sash);
            }
        }
        return grid;
    }

    // Panel is one tool in its area: its title bar when the area has more
    // than one, over its content.
    Grid Panel(Tool tool, bool titled)
    {
        var panel = new Grid();
        panel.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        panel.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        if (titled)
        {
            var bar = new Grid { Padding = new Thickness(12, 2, 4, 2), ColumnSpacing = 2 };
            bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            bar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            bar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            bar.Children.Add(new TextBlock { Text = Name(tool), Style = Ui.Style("CaptionTextBlockStyle"), VerticalAlignment = VerticalAlignment.Center });
            var place = new DropDownButton
            {
                Content = new FontIcon { Glyph = "\uE712", FontSize = 12 },
                Padding = new Thickness(6, 4, 6, 4),
                Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent),
                BorderThickness = new Thickness(0),
            };
            var menu = new MenuFlyout();
            menu.Opening += (_, _) =>
            {
                menu.Items.Clear();
                foreach (var item in PlaceItems(tool))
                {
                    menu.Items.Add(item);
                }
            };
            place.Flyout = menu;
            ToolTipService.SetToolTip(place, "位置");
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(place, Name(tool) + "的位置");
            Grid.SetColumn(place, 1);
            bar.Children.Add(place);
            var hide = Ui.IconButton("\uE711", "隐藏" + Name(tool));
            hide.Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
            hide.BorderThickness = new Thickness(0);
            hide.Click += (_, _) => SetShown(tool, false);
            Grid.SetColumn(hide, 2);
            bar.Children.Add(hide);
            panel.Children.Add(bar);
        }
        var slot = new Border { Child = contents[tool] };
        slots.Add(slot);
        Grid.SetRow(slot, 1);
        panel.Children.Add(slot);
        return panel;
    }

    Sash ShareSash(Grid grid, List<Tool> tools, int index, bool across)
    {
        double[]? start = null;
        double Size(int at) => across ? grid.ColumnDefinitions[at].ActualWidth : grid.RowDefinitions[at].ActualHeight;
        void Set(int at, double value)
        {
            var length = new GridLength(value, GridUnitType.Star);
            if (across)
            {
                grid.ColumnDefinitions[at].Width = length;
            }
            else
            {
                grid.RowDefinitions[at].Height = length;
            }
        }
        var sash = new Sash(across, delta =>
        {
            if (start == null)
            {
                start = [.. Enumerable.Range(0, tools.Count).Select(Size)];
                for (var i = 0; i < start.Length; i++)
                {
                    Set(i, start[i]);
                }
            }
            var pair = start[index - 1] + start[index];
            var before = Math.Clamp(start[index - 1] + delta, Math.Min(80, pair / 2), Math.Max(pair - 80, pair / 2));
            Set(index - 1, before);
            Set(index, pair - before);
        }, () =>
        {
            start = null;
            var sizes = Enumerable.Range(0, tools.Count).Select(Size).ToList();
            var total = sizes.Sum();
            for (var i = 0; total > 0 && i < tools.Count; i++)
            {
                Layout.Tools[tools[i]].Share = sizes[i] / total * tools.Count;
            }
            Changed();
        }, () =>
        {
            foreach (var tool in tools)
            {
                Layout.Tools[tool].Share = 1;
            }
            Changed();
            Build();
        }, line: true);
        return sash;
    }

    // AreaSash resizes an area from its inner edge.
    void AreaSash(Area area)
    {
        double? start = null;
        var state = Layout.Areas[area];
        double Current() => area switch
        {
            Area.Left => left.ActualWidth,
            Area.Right => right.ActualWidth,
            _ => bottom.ActualHeight,
        };
        void Set(double size)
        {
            switch (area)
            {
                case Area.Left:
                    left.Width = new GridLength(size);
                    break;
                case Area.Right:
                    right.Width = new GridLength(size);
                    break;
                default:
                    bottom.Height = new GridLength(size);
                    break;
            }
        }
        var sash = new Sash(area != Area.Bottom, delta =>
        {
            start ??= Current();
            var limit = area == Area.Bottom ? Math.Max(center.ActualHeight - 160, MinBottom) : Math.Max(body.ActualWidth / 2, MinSide);
            var grown = area == Area.Left ? delta : -delta;
            Set(Math.Clamp(start.Value + grown, area == Area.Bottom ? MinBottom : MinSide, limit));
        }, () =>
        {
            start = null;
            state.Size = Current();
            Changed();
        }, () =>
        {
            state.Size = area == Area.Bottom ? ToolLayout.BottomSize : ToolLayout.SideSize;
            Set(state.Size);
            Changed();
        });
        switch (area)
        {
            case Area.Left:
                Grid.SetColumn(sash, 1);
                body.Children.Add(sash);
                break;
            case Area.Right:
                Grid.SetColumn(sash, 2);
                body.Children.Add(sash);
                break;
            default:
                Grid.SetRow(sash, 1);
                center.Children.Add(sash);
                break;
        }
        sashes.Add(sash);
    }

    void Changed()
    {
        window.ToolsChanged();
    }

    // Apply makes a change of the layout show and keeps it.
    void Apply()
    {
        Build();
        window.ToolsChanged();
        window.PlaceComposers();
    }

    // Move puts a tool at a place: docked in an area, or the outline's button
    // or the composer in each pane. A tool moved to a collapsed area opens it.
    public void Move(Tool tool, Place place, Area area)
    {
        var state = Layout.Tools[tool];
        state.Place = place;
        state.Area = area;
        state.Shown = true;
        state.Share = 1;
        if (place == Place.Dock)
        {
            Layout.Areas[area].Collapsed = false;
            Layout.EditorOnly = null;
        }
        Apply();
    }

    public void SetShown(Tool tool, bool shown)
    {
        var state = Layout.Tools[tool];
        if (state.Place is Place.Button or Place.InPane)
        {
            // A button or the composer in each pane has nothing to hide; hiding
            // one docks it hidden where it was last docked.
            state.Place = Place.Dock;
        }
        state.Shown = shown;
        if (shown && state.Place == Place.Dock)
        {
            Layout.Areas[state.Area].Collapsed = false;
            Layout.EditorOnly = null;
        }
        Apply();
    }

    // ToggleArea opens or collapses an area; opening an empty one brings its
    // usual tool.
    public void ToggleArea(Area area)
    {
        var state = Layout.Areas[area];
        if (AreaOpen(area))
        {
            state.Collapsed = true;
        }
        else
        {
            state.Collapsed = false;
            if (Layout.InArea(area).Count == 0)
            {
                var usual = Layout.Tools[ToolLayout.Usual(area)];
                usual.Place = Place.Dock;
                usual.Area = area;
                usual.Shown = true;
            }
        }
        Layout.EditorOnly = null;
        Apply();
    }

    // ToggleEditorOnly collapses every open area, or opens them again.
    public void ToggleEditorOnly()
    {
        if (Layout.EditorOnly is { } collapsed)
        {
            foreach (var area in collapsed)
            {
                Layout.Areas[area].Collapsed = false;
            }
            Layout.EditorOnly = null;
        }
        else
        {
            var open = Enum.GetValues<Area>().Where(AreaOpen).ToList();
            foreach (var area in open)
            {
                Layout.Areas[area].Collapsed = true;
            }
            Layout.EditorOnly = open;
        }
        Apply();
    }

    // ToggleTool shows or hides a docked tool; the outline as a button acts
    // as the button does.
    public void ToggleTool(Tool tool)
    {
        var state = Layout.Tools[tool];
        if (state.Place == Place.Dock)
        {
            SetShown(tool, !Layout.Visible(tool));
        }
        else if (state.Place is Place.Floating or Place.Window)
        {
            SetShown(tool, !state.Shown);
        }
        else if (tool == Tool.Outline)
        {
            window.Editor.OutlineClicked(window.Editor.Focused);
        }
    }

    // PlaceMenu is a tool's place, as a submenu.
    public MenuFlyoutSubItem PlaceMenu(Tool tool, string text)
    {
        var menu = new MenuFlyoutSubItem { Text = text };
        foreach (var item in PlaceItems(tool))
        {
            menu.Items.Add(item);
        }
        return menu;
    }

    public List<MenuFlyoutItemBase> PlaceItems(Tool tool)
    {
        var state = Layout.Tools[tool];
        var items = new List<MenuFlyoutItemBase>();
        RadioMenuFlyoutItem Radio(string text, bool chosen, Action choose)
        {
            var item = new RadioMenuFlyoutItem { Text = text, GroupName = "place:" + tool, IsChecked = chosen };
            item.Click += (_, _) => choose();
            return item;
        }
        foreach (var area in Enum.GetValues<Area>())
        {
            items.Add(Radio("停靠在" + AreaName(area), state.Place == Place.Dock && state.Area == area && state.Shown, () => Move(tool, Place.Dock, area)));
        }
        items.Add(Radio("浮动", state.Place == Place.Floating && state.Shown, () => Move(tool, Place.Floating, state.Area)));
        items.Add(Radio("独立窗口", state.Place == Place.Window && state.Shown, () => Move(tool, Place.Window, state.Area)));
        if (tool == Tool.Outline)
        {
            items.Add(Radio("标签栏上的按钮", state.Place == Place.Button, () => Move(tool, Place.Button, state.Area)));
        }
        if (tool == Tool.Composer)
        {
            items.Add(Radio("在每个面板中", state.Place == Place.InPane, () => Move(tool, Place.InPane, state.Area)));
        }
        if (tool == Tool.List || state.Place is Place.Dock or Place.Floating or Place.Window)
        {
            items.Add(Radio("隐藏", !state.Shown, () => SetShown(tool, false)));
        }
        return items;
    }

    // LayoutMenu is the whole window's layout, for the tab menu and the
    // title bar: each tool's place, the areas and only the editor.
    public List<MenuFlyoutItemBase> LayoutItems()
    {
        var items = new List<MenuFlyoutItemBase>();
        foreach (var tool in Enum.GetValues<Tool>())
        {
            items.Add(PlaceMenu(tool, Name(tool)));
        }
        items.Add(new MenuFlyoutSeparator());
        foreach (var area in Enum.GetValues<Area>())
        {
            var toggle = new ToggleMenuFlyoutItem { Text = "显示" + AreaName(area) + "区域", IsChecked = AreaOpen(area) };
            var at = area;
            toggle.Click += (_, _) => ToggleArea(at);
            toggle.KeyboardAcceleratorTextOverride = window.Commands.Shortcut(at switch
            {
                Area.Left => Command.ToggleLeft,
                Area.Right => Command.ToggleRight,
                _ => Command.ToggleBottom,
            });
            items.Add(toggle);
        }
        var editorOnly = new ToggleMenuFlyoutItem { Text = "只显示编辑区", IsChecked = Layout.EditorOnly != null };
        editorOnly.Click += (_, _) => ToggleEditorOnly();
        editorOnly.KeyboardAcceleratorTextOverride = window.Commands.Shortcut(Command.EditorOnly);
        items.Add(editorOnly);
        var pinned = new ToggleMenuFlyoutItem { Text = "保持在最前面", IsChecked = window.Pinned };
        pinned.Click += (_, _) => window.Pinned = !window.Pinned;
        items.Add(pinned);
        return items;
    }
}
