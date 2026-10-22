using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Graphics;

namespace Velune;

// A tool window outside its workspace window's areas: floating over it as
// a tool palette (above it, minimized with it, off the taskbar), or a
// window of its own. Closing it hides the tool.
sealed class ToolHost : Window
{
    readonly WorkspaceWindow owner;
    readonly Border slot = new();
    // Closed by a change of the layout rather than by the user.
    bool replacing;
    bool closed;
    // A floating window snapped to its owner keeps its distance from the
    // owner's nearest edges as the owner moves or resizes.
    (bool FromRight, int X, bool FromBottom, int Y)? snap;

    public Tool Tool { get; }
    public Place Place { get; }

    public ToolHost(WorkspaceWindow owner, Tool tool, Place place, Bounds? bounds, FrameworkElement content)
    {
        this.owner = owner;
        Tool = tool;
        Place = place;
        var name = ToolWindows.Name(tool);
        Title = name;
        ExtendsContentIntoTitleBar = true;
        var titleBar = new TitleBar { Title = name, Height = 40 };
        var placeButton = new DropDownButton
        {
            Content = new FontIcon { Glyph = "\uE712", FontSize = 12 },
            Padding = new Thickness(8, 4, 8, 4),
            Margin = new Thickness(0, 0, 4, 0),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
        };
        ToolTipService.SetToolTip(placeButton, "位置");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(placeButton, name + "的位置");
        var menu = new MenuFlyout();
        menu.Opening += (_, _) =>
        {
            menu.Items.Clear();
            foreach (var item in owner.Tools.PlaceItems(tool))
            {
                menu.Items.Add(item);
            }
        };
        placeButton.Flyout = menu;
        titleBar.RightHeader = placeButton;
        var root = new Grid();
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.Children.Add(titleBar);
        Grid.SetRow(slot, 1);
        root.Children.Add(slot);
        Content = root;
        SetTitleBar(titleBar);
        if (place == Place.Floating && DesktopAcrylicController.IsSupported())
        {
            SystemBackdrop = new DesktopAcrylicBackdrop();
        }
        else if (place == Place.Window && MicaController.IsSupported())
        {
            SystemBackdrop = new MicaBackdrop();
        }
        else
        {
            root.Style = Ui.Style("WindowBackgroundStyle");
        }
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Velune.ico"));
        if (place == Place.Floating)
        {
            AppWindow.SetPresenter(OverlappedPresenter.CreateForToolWindow());
            AppWindow.IsShownInSwitchers = false;
            Native.SetOwner(this, owner);
        }
        Locate(bounds);
        slot.Child = content;
        Native.OnMoved(this, () => DispatcherQueue.TryEnqueue(() =>
        {
            if (!closed)
            {
                owner.Tools.Released(this);
            }
        }));
        Closed += (_, _) =>
        {
            closed = true;
            Native.ForgetMoved(this);
            slot.Child = null;
            if (!replacing)
            {
                owner.Tools.HostClosed(Tool, Bounds);
            }
        };
    }

    // Locate puts the window where it was, else beside its owner's right edge.
    void Locate(Bounds? bounds)
    {
        if (bounds is { } saved && DisplayArea.GetFromRect(new RectInt32(saved.X, saved.Y, saved.Width, saved.Height), DisplayAreaFallback.None) != null)
        {
            AppWindow.MoveAndResize(new RectInt32(saved.X, saved.Y, saved.Width, saved.Height));
            return;
        }
        var scale = Native.Scale(owner);
        var width = (int)(320 * scale);
        var height = (int)((Tool == Tool.Composer ? 260 : 480) * scale);
        var position = owner.AppWindow.Position;
        var size = owner.AppWindow.Size;
        AppWindow.MoveAndResize(new RectInt32(position.X + size.Width - width - (int)(24 * scale), position.Y + (int)(72 * scale), width, height));
    }

    public Bounds Bounds => new(AppWindow.Position.X, AppWindow.Position.Y, AppWindow.Size.Width, AppWindow.Size.Height);

    public bool Closing => closed;

    // Replace closes the window for a change of the layout; its content
    // leaves it first, to go where the layout puts it.
    public void Replace()
    {
        replacing = true;
        slot.Child = null;
        Close();
    }

    public void Snap(RectInt32 window, PointInt32 at)
    {
        AppWindow.Move(at);
        var fromRight = at.X + AppWindow.Size.Width / 2 > window.X + window.Width / 2;
        var fromBottom = at.Y + AppWindow.Size.Height / 2 > window.Y + window.Height / 2;
        snap = (fromRight, at.X - (fromRight ? window.X + window.Width : window.X), fromBottom, at.Y - (fromBottom ? window.Y + window.Height : window.Y));
    }

    public void Unsnap() => snap = null;

    public bool Snapped => snap != null;

    // OwnerMoved follows the owner when snapped to it.
    public void OwnerMoved(RectInt32 window)
    {
        if (snap is not var (fromRight, x, fromBottom, y) || closed)
        {
            return;
        }
        AppWindow.Move(new PointInt32((fromRight ? window.X + window.Width : window.X) + x, (fromBottom ? window.Y + window.Height : window.Y) + y));
    }
}
