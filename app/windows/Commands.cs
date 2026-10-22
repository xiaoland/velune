using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Input;
using Windows.System;

namespace Velune;

// Velune's own window commands. Their shortcuts follow Windows and VS Code
// and Chrome on Windows (see TDD.md); sending and steering belong to the
// composer and are not here.
enum Command
{
    NewConversation,
    NewTab,
    NewWindow,
    CloseTab,
    CloseWindow,
    SplitRight,
    SplitDown,
    PreviousPane,
    NextPane,
    PreviousTab,
    NextTab,
    Back,
    Forward,
    ToggleLeft,
    ToggleRight,
    ToggleBottom,
    EditorOnly,
    ShowList,
    ShowOutline,
    FocusComposer,
    FindConversation,
    Rename,
    Settings,
}

readonly record struct Shortcut(VirtualKeyModifiers Modifiers, VirtualKey Key)
{
    public override string ToString()
    {
        var parts = new List<string>();
        if (Modifiers.HasFlag(VirtualKeyModifiers.Control))
        {
            parts.Add("Ctrl");
        }
        if (Modifiers.HasFlag(VirtualKeyModifiers.Menu))
        {
            parts.Add("Alt");
        }
        if (Modifiers.HasFlag(VirtualKeyModifiers.Shift))
        {
            parts.Add("Shift");
        }
        parts.Add(Key switch
        {
            (VirtualKey)188 => ",",
            (VirtualKey)219 => "[",
            (VirtualKey)220 => "\\",
            (VirtualKey)221 => "]",
            VirtualKey.Left => "←",
            VirtualKey.Right => "→",
            >= VirtualKey.Number0 and <= VirtualKey.Number9 => ((int)Key - (int)VirtualKey.Number0).ToString(),
            _ => Key.ToString(),
        });
        return string.Join("+", parts);
    }
}

sealed class Commands
{
    const VirtualKeyModifiers Ctrl = VirtualKeyModifiers.Control;
    const VirtualKeyModifiers Shift = VirtualKeyModifiers.Shift;
    const VirtualKeyModifiers Alt = VirtualKeyModifiers.Menu;

    public static readonly IReadOnlyDictionary<Command, (string Name, Shortcut? Default)> Table = new Dictionary<Command, (string, Shortcut?)>
    {
        [Command.NewConversation] = ("新建会话", new(Ctrl, VirtualKey.N)),
        [Command.NewTab] = ("新建标签页", new(Ctrl, VirtualKey.T)),
        [Command.NewWindow] = ("新建窗口", new(Ctrl | Shift, VirtualKey.N)),
        [Command.CloseTab] = ("关闭标签页", new(Ctrl, VirtualKey.W)),
        [Command.CloseWindow] = ("关闭窗口", new(Ctrl | Shift, VirtualKey.W)),
        [Command.SplitRight] = ("向右拆分", new(Ctrl, (VirtualKey)220)),
        [Command.SplitDown] = ("向下拆分", new(Ctrl | Shift, (VirtualKey)220)),
        [Command.PreviousPane] = ("上一个分屏", new(Ctrl, (VirtualKey)219)),
        [Command.NextPane] = ("下一个分屏", new(Ctrl, (VirtualKey)221)),
        [Command.PreviousTab] = ("上一个标签页", new(Ctrl | Shift, VirtualKey.Tab)),
        [Command.NextTab] = ("下一个标签页", new(Ctrl, VirtualKey.Tab)),
        [Command.Back] = ("后退", new(Alt, VirtualKey.Left)),
        [Command.Forward] = ("前进", new(Alt, VirtualKey.Right)),
        [Command.ToggleLeft] = ("显示／隐藏左侧区域", new(Ctrl, VirtualKey.B)),
        [Command.ToggleRight] = ("显示／隐藏右侧区域", new(Ctrl | Alt, VirtualKey.B)),
        [Command.ToggleBottom] = ("显示／隐藏底部区域", new(Ctrl, VirtualKey.J)),
        [Command.EditorOnly] = ("只显示编辑区", new(Ctrl | Shift, VirtualKey.E)),
        [Command.ShowList] = ("显示／隐藏会话列表", new(Ctrl, VirtualKey.Number1)),
        [Command.ShowOutline] = ("显示／隐藏会话大纲", new(Ctrl, VirtualKey.Number2)),
        [Command.FocusComposer] = ("转到输入区", new(Ctrl, VirtualKey.Number3)),
        [Command.FindConversation] = ("搜索会话", new(Ctrl, VirtualKey.F)),
        [Command.Rename] = ("重命名会话", new(VirtualKeyModifiers.None, VirtualKey.F2)),
        [Command.Settings] = ("设置", new(Ctrl, (VirtualKey)188)),
    };

    readonly Dictionary<Command, Shortcut?> bound = Table.ToDictionary(entry => entry.Key, entry => entry.Value.Default);

    public string Shortcut(Command command) => bound[command]?.ToString() ?? "";

    // Install puts the commands' shortcuts on a window's root.
    public void Install(UIElement root, Action<Command> run)
    {
        root.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        root.KeyboardAccelerators.Clear();
        foreach (var (command, shortcut) in bound)
        {
            if (shortcut is not { } keys)
            {
                continue;
            }
            var accelerator = new KeyboardAccelerator { Key = keys.Key, Modifiers = keys.Modifiers };
            accelerator.Invoked += (_, e) =>
            {
                e.Handled = true;
                run(command);
            };
            root.KeyboardAccelerators.Add(accelerator);
        }
    }
}
