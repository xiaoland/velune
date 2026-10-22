using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics;

namespace Velune;

// What is being dragged within the app: a tab, a conversation from a list,
// or a list's group. Drags carry only the app's own format, so nothing
// outside the app takes them; what they carry is recorded here.
static class DragSource
{
    public const string Format = "Velune.Drag";

    public static (WorkspaceWindow Window, Tab Tab)? Tab { get; set; }
    public static (Device Device, Target Target)? Conversation { get; set; }
    public static (ConversationList List, string GroupID)? Group { get; set; }

    public static void Clear()
    {
        Tab = null;
        Conversation = null;
        Group = null;
    }

    public static void Carry(DataPackage data)
    {
        data.SetData(Format, "velune");
        data.RequestedOperation = DataPackageOperation.Move;
    }
}

// Where a drag over a pane would put what it carries: among the tabs, in
// the pane, or in a new pane at one of its edges.
enum DropZone { None, Strip, Center, Left, Right, Top, Bottom }

public sealed partial class WorkspaceWindow
{
    // The tab bar's height: drops there belong to the bar's own events.
    const double StripHeight = 40;

    internal bool IsConversationWindow { get; private set; }

    internal void TabDragStarting(TabViewTabDragStartingEventArgs e)
    {
        if (e.Tab.Tag is not Tab tab)
        {
            e.Cancel = true;
            return;
        }
        DragSource.Clear();
        DragSource.Tab = (this, tab);
        DragSource.Carry(e.Data);
    }

    // TabDroppedOutside makes a tab dropped outside every window a window of
    // its own where it was dropped.
    internal void TabDroppedOutside(TabViewTabDroppedOutsideEventArgs e)
    {
        if (e.Tab.Tag is Tab tab && Shell.WindowAt(Native.Cursor()) == null)
        {
            MoveToNewWindow(tab, Native.Cursor());
        }
        DragSource.Clear();
    }

    DropZone Zone(Pane pane, DragEventArgs e, bool strip)
    {
        if (strip)
        {
            return DropZone.Strip;
        }
        var point = e.GetPosition(pane.View);
        if (point.Y < StripHeight)
        {
            return DropZone.None;
        }
        var x = point.X / Math.Max(pane.View.ActualWidth, 1);
        var y = (point.Y - StripHeight) / Math.Max(pane.View.ActualHeight - StripHeight, 1);
        return x < 0.25 ? DropZone.Left
            : x > 0.75 ? DropZone.Right
            : y < 0.25 ? DropZone.Top
            : y > 0.75 ? DropZone.Bottom
            : DropZone.Center;
    }

    // Accepts tells whether a drop could go there: the same device, and
    // not where it already is.
    bool Accepts(Pane pane, DropZone zone)
    {
        if (zone == DropZone.None)
        {
            return false;
        }
        if (DragSource.Tab is var (source, tab))
        {
            if (source.Device != Device)
            {
                return false;
            }
            if (source == this && Editor.PaneOf(tab) == pane)
            {
                // The bar reorders its own tabs; a pane splits off one of several.
                return zone is not (DropZone.Strip or DropZone.Center) && pane.Count > 1;
            }
            return true;
        }
        return DragSource.Conversation is var (device, _) && device == Device;
    }

    static string Caption(DropZone zone) => zone switch
    {
        DropZone.Left => "在左侧拆分",
        DropZone.Right => "在右侧拆分",
        DropZone.Top => "在上方拆分",
        DropZone.Bottom => "在下方拆分",
        _ => "移到此处",
    };

    internal void DragOverPane(Pane pane, DragEventArgs e, bool strip)
    {
        if (!e.DataView.Contains(DragSource.Format))
        {
            return;
        }
        var zone = Zone(pane, e, strip);
        if (!Accepts(pane, zone))
        {
            return;
        }
        e.AcceptedOperation = DataPackageOperation.Move;
        e.DragUIOverride.Caption = Caption(zone);
        e.DragUIOverride.IsGlyphVisible = false;
        e.Handled = true;
    }

    internal void DropOnPane(Pane pane, DragEventArgs e, bool strip)
    {
        if (!e.DataView.Contains(DragSource.Format))
        {
            return;
        }
        var zone = Zone(pane, e, strip);
        if (!Accepts(pane, zone))
        {
            return;
        }
        e.Handled = true;
        e.AcceptedOperation = DataPackageOperation.Move;
        var index = strip ? InsertIndex(pane, e) : pane.Count;
        Tab tab;
        if (DragSource.Tab is var (source, dragged))
        {
            source.Editor.Release(dragged);
            if (source != this)
            {
                Adopt(dragged);
            }
            tab = dragged;
        }
        else if (DragSource.Conversation is var (_, target))
        {
            // A conversation this window shows already moves with its tab.
            if (Editor.Find(target.ConversationID) is { } open)
            {
                if (Editor.PaneOf(open) == pane && zone is DropZone.Strip or DropZone.Center)
                {
                    Editor.Select(open);
                    DragSource.Clear();
                    return;
                }
                Editor.Release(open);
                tab = open;
            }
            else
            {
                tab = new Tab(target);
            }
        }
        else
        {
            return;
        }
        DragSource.Clear();
        // Releasing may have closed the pane the drop was on.
        if (!Editor.Panes().Contains(pane))
        {
            pane = Editor.Focused;
            zone = DropZone.Center;
        }
        switch (zone)
        {
            case DropZone.Left or DropZone.Right:
                Editor.SplitWith(pane, false, tab, zone == DropZone.Left);
                break;
            case DropZone.Top or DropZone.Bottom:
                Editor.SplitWith(pane, true, tab, zone == DropZone.Top);
                break;
            default:
                Editor.Insert(pane, tab, Math.Min(index, pane.Count));
                break;
        }
    }

    // InsertIndex is where among a pane's tabs the pointer is.
    static int InsertIndex(Pane pane, DragEventArgs e)
    {
        var x = e.GetPosition(pane.View).X;
        var index = 0;
        foreach (var item in pane.View.TabItems.OfType<TabViewItem>())
        {
            if (item.ActualWidth > 0 && item.TransformToVisual(pane.View).TransformPoint(new Windows.Foundation.Point(0, 0)).X + item.ActualWidth / 2 < x)
            {
                index++;
            }
        }
        return index;
    }

    // Adopt takes a tab from another window of the same device, its view
    // following it here.
    void Adopt(Tab tab)
    {
        tab.Unfollow?.Invoke();
        tab.Unfollow = null;
        if (tab.View is { } view)
        {
            view.MoveTo(this);
            view.Model.Configuration = Listing.Configuration;
            Follow(tab);
        }
    }

    // MoveToNewWindow moves a tab to a conversation window at a point of the
    // screen, as big as its pane. The only tab of a conversation window
    // cannot leave it; the window moves there instead.
    internal void MoveToNewWindow(Tab tab, PointInt32? at)
    {
        if (Editor.PaneOf(tab) is not { } pane)
        {
            return;
        }
        var scale = Native.Scale(this);
        var size = new SizeInt32((int)Math.Max(pane.View.ActualWidth * scale, 480 * scale), (int)Math.Max(pane.View.ActualHeight * scale, 360 * scale));
        if (IsConversationWindow && Editor.Tabs().Count() == 1)
        {
            if (at is { } point)
            {
                AppWindow.Move(point);
            }
            return;
        }
        Editor.Release(tab);
        Shell.OpenConversationWindow(Device, tab, null, at is { } corner ? new Bounds(corner.X, corner.Y, size.Width, size.Height) : null);
    }

    // Contains tells whether a point of the screen is on this window.
    internal bool Contains(PointInt32 point)
    {
        if (closed || !AppWindow.IsVisible || AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized })
        {
            return false;
        }
        var position = AppWindow.Position;
        var size = AppWindow.Size;
        return point.X >= position.X && point.X < position.X + size.Width && point.Y >= position.Y && point.Y < position.Y + size.Height;
    }
}
