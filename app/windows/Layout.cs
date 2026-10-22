using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.UI.Dispatching;

namespace Velune;

// The window layout as data: where the tool windows are and how each window's
// editor splits into panes of tabs. It is this app's own presentation state
// (see TDD.md); the core never sees it.

enum Area { Left, Right, Bottom }

enum Tool { List, Outline, Composer }

// Where a tool window is: docked in an area, floating over its window, a
// window of its own, or (the outline) a button on each pane's tab bar, or
// (the composer) in each pane.
enum Place { Dock, Floating, Window, Button, InPane }

// Bounds are in physical pixels, as the window manager places windows.
sealed record Bounds(int X, int Y, int Width, int Height);

sealed class ToolState
{
    public Place Place { get; set; }
    public Area Area { get; set; }
    public bool Shown { get; set; } = true;
    // Its share of its area, among the area's tools.
    public double Share { get; set; } = 1;
    public Bounds? Bounds { get; set; }
}

sealed class AreaState
{
    // Width of a side area, height of the bottom one, in effective pixels.
    public double Size { get; set; }
    public bool Collapsed { get; set; }
}

sealed class ToolLayout
{
    public const double SideSize = 280;
    public const double BottomSize = 200;

    public Dictionary<Tool, ToolState> Tools { get; set; } = [];
    public Dictionary<Area, AreaState> Areas { get; set; } = [];
    // The areas “只显示编辑区” collapsed, to open again; null when it is off.
    public List<Area>? EditorOnly { get; set; }

    public static ToolLayout Default() => new()
    {
        Tools = new()
        {
            [Tool.List] = new ToolState { Place = Place.Dock, Area = Area.Left },
            [Tool.Outline] = new ToolState { Place = Place.Button, Area = Area.Right },
            [Tool.Composer] = new ToolState { Place = Place.InPane, Area = Area.Bottom },
        },
        Areas = new()
        {
            [Area.Left] = new AreaState { Size = SideSize },
            [Area.Right] = new AreaState { Size = SideSize, Collapsed = true },
            [Area.Bottom] = new AreaState { Size = BottomSize, Collapsed = true },
        },
    };

    // Complete fills what a saved layout lacks with the defaults.
    public ToolLayout Complete()
    {
        var defaults = Default();
        foreach (var (tool, state) in defaults.Tools)
        {
            Tools.TryAdd(tool, state);
        }
        foreach (var (area, state) in defaults.Areas)
        {
            Areas.TryAdd(area, state);
        }
        return this;
    }

    public ToolLayout Copy() => JsonSerializer.Deserialize<ToolLayout>(JsonSerializer.Serialize(this, LayoutStore.Options), LayoutStore.Options)!.Complete();

    // The usual tool of each area, which showing an empty area brings.
    public static Tool Usual(Area area) => area switch
    {
        Area.Left => Tool.List,
        Area.Right => Tool.Outline,
        _ => Tool.Composer,
    };

    public bool Docked(Tool tool, Area area) => Tools[tool] is { Place: Place.Dock, Shown: true } state && state.Area == area;

    // Visible tells whether a docked tool shows: shown and its area open.
    public bool Visible(Tool tool) => Tools[tool] is { Place: Place.Dock, Shown: true } state && !Areas[state.Area].Collapsed;

    // InArea are the tools shown in an area, in the order they stack.
    public List<Tool> InArea(Area area) => [.. Enum.GetValues<Tool>().Where(tool => Docked(tool, area))];
}

// A tab in a saved editor: a conversation, or an unsent one when its ID is empty.
sealed record TabState(string RuntimeId, string ConversationId, string Title);

[JsonPolymorphic(TypeDiscriminatorPropertyName = "kind")]
[JsonDerivedType(typeof(PaneState), "pane")]
[JsonDerivedType(typeof(SplitState), "split")]
abstract class EditorState;

sealed class PaneState : EditorState
{
    public List<TabState> Tabs { get; set; } = [];
    public int Selected { get; set; }
}

sealed class SplitState : EditorState
{
    // Vertical splits stack their children top to bottom.
    public bool Vertical { get; set; }
    public List<EditorState> Children { get; set; } = [];
    public List<double> Ratios { get; set; } = [];
}

sealed class WindowState
{
    public string Device { get; set; } = "";
    public Bounds? Bounds { get; set; }
    public bool Maximized { get; set; }
    public bool Pinned { get; set; }
    public EditorState Editor { get; set; } = new PaneState();
    // The focused pane, by its place among the panes in reading order.
    public int Focus { get; set; }
    public ToolLayout Tools { get; set; } = ToolLayout.Default();
}

sealed class SavedLayout
{
    public const int CurrentSchema = 1;
    public int Schema { get; set; } = CurrentSchema;
    // The latest tool layout, which a window without its own starts from.
    public ToolLayout Tools { get; set; } = ToolLayout.Default();
    public List<WindowState> Windows { get; set; } = [];
}

// LayoutStore keeps the layout in VELUNE_HOME/windows-layout.json. A file of
// another schema, or one that does not read, gives the defaults, and the next
// change overwrites it.
sealed class LayoutStore(string home, DispatcherQueue dispatcher)
{
    public static readonly JsonSerializerOptions Options = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
    };

    readonly string path = Path.Combine(home, "windows-layout.json");
    DispatcherQueueTimer? timer;

    public SavedLayout Read()
    {
        try
        {
            var saved = JsonSerializer.Deserialize<SavedLayout>(File.ReadAllText(path), Options);
            if (saved is { Schema: SavedLayout.CurrentSchema })
            {
                saved.Tools.Complete();
                foreach (var window in saved.Windows)
                {
                    window.Tools.Complete();
                }
                return saved;
            }
        }
        catch (Exception failure) when (failure is IOException or UnauthorizedAccessException or JsonException or NotSupportedException)
        {
        }
        return new SavedLayout();
    }

    // Save writes the layout a moment after the latest change.
    public void Save(Func<SavedLayout> layout)
    {
        if (timer == null)
        {
            timer = dispatcher.CreateTimer();
            timer.Interval = TimeSpan.FromSeconds(1);
            timer.IsRepeating = false;
            timer.Tick += (_, _) => Flush();
        }
        pending = layout;
        timer.Stop();
        timer.Start();
    }

    Func<SavedLayout>? pending;

    // Flush writes a pending change now, as the app quits.
    public void Flush()
    {
        if (pending is not { } layout)
        {
            return;
        }
        pending = null;
        try
        {
            Directory.CreateDirectory(home);
            var temporary = path + ".tmp";
            File.WriteAllText(temporary, JsonSerializer.Serialize(layout(), Options));
            File.Move(temporary, path, true);
        }
        catch (Exception failure) when (failure is IOException or UnauthorizedAccessException)
        {
            // The layout is a convenience; the next change tries again.
        }
    }
}
