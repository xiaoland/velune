using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Input;
using Windows.System;

namespace Velune;

// Velune's own window commands. Their shortcuts follow Windows and VS Code
// and Chrome on Windows (see TDD.md); sending and steering belong to the
// composer and are not here. The last two act from other apps too.
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
    ShowLinked,
    FindConversation,
    Rename,
    Settings,
    Problems,
    Analytics,
    Library,
    CommandPalette,
    OpenConversation,
    NextAttention,
    FocusTranscript,
    ShortcutHelp,
    Attach,
    ChooseRuntime,
    ChooseModel,
    ChooseEffort,
    ChoosePermission,
    Continue,
    Stop,
    GlobalShow,
    GlobalNewConversation,
}

// How the composer sends: Enter, with Shift+Enter for a new line, or
// Ctrl+Enter, with Enter for a new line. Either way the next modifier up
// steers the running turn.
enum SendMode
{
    Enter,
    CtrlEnter,
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
        if (Modifiers.HasFlag(VirtualKeyModifiers.Windows))
        {
            parts.Add("Win");
        }
        parts.Add(Key switch
        {
            (VirtualKey)188 => ",",
            (VirtualKey)191 => "/",
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

// The commands' shortcuts as the user set them, for every window, and the
// composer's send mode. Only changes are kept, in
// VELUNE_HOME/windows-shortcuts.json (schema 1, null for a removed shortcut,
// no send mode for Enter); another schema, or a file that does not read,
// gives the defaults and the next change overwrites it.
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
        [Command.ShowLinked] = ("显示／隐藏关联 Agent", new(Ctrl, VirtualKey.Number4)),
        [Command.FindConversation] = ("搜索会话", new(Ctrl, VirtualKey.F)),
        [Command.Rename] = ("重命名会话", new(VirtualKeyModifiers.None, VirtualKey.F2)),
        [Command.Settings] = ("设置", new(Ctrl, (VirtualKey)188)),
        [Command.Problems] = ("问题", new(Ctrl | Shift, VirtualKey.M)),
        [Command.Analytics] = ("分析", null),
        [Command.Library] = ("资源库", null),
        [Command.CommandPalette] = ("命令面板", new(Ctrl, VirtualKey.K)),
        [Command.OpenConversation] = ("打开会话…", new(Ctrl, VirtualKey.P)),
        [Command.NextAttention] = ("转到待处理的会话", new(Ctrl | Shift, VirtualKey.A)),
        [Command.FocusTranscript] = ("转到消息", new(Ctrl, VirtualKey.Number0)),
        [Command.ShortcutHelp] = ("快捷键速查", new(Ctrl, (VirtualKey)191)),
        [Command.Attach] = ("添加附件…", new(Ctrl, VirtualKey.O)),
        // Not Ctrl+Alt: on many layouts it is AltGr, which types characters.
        [Command.ChooseRuntime] = ("选择运行时…", new(Ctrl | Shift, VirtualKey.R)),
        [Command.ChooseModel] = ("选择模型…", new(Ctrl | Shift, VirtualKey.O)),
        [Command.ChooseEffort] = ("选择推理强度…", new(Ctrl | Shift, VirtualKey.I)),
        [Command.ChoosePermission] = ("选择权限模式…", new(Ctrl | Shift, VirtualKey.P)),
        [Command.Continue] = ("继续", null),
        [Command.Stop] = ("停止生成", null),
        [Command.GlobalShow] = ("显示 Velune（全局）", null),
        [Command.GlobalNewConversation] = ("新建会话（全局）", null),
    };

    // Global commands act while another app is in front, through the system.
    public static bool IsGlobal(Command command) => command is Command.GlobalShow or Command.GlobalNewConversation;

    sealed record Keys(int Modifiers, int Key);

    sealed class Saved
    {
        public int Schema { get; set; } = 1;
        public Dictionary<Command, Keys?> Shortcuts { get; set; } = [];
        public SendMode? SendMode { get; set; }
    }

    readonly string path;
    readonly Dictionary<Command, Shortcut?> changed = [];

    public SendMode SendMode { get; private set; } = SendMode.Enter;

    // The global commands whose shortcut the system or another app holds.
    public HashSet<Command> GlobalFailed { get; } = [];

    // Changed runs after a shortcut changes, for the windows to take it.
    public event Action? Changed;

    public Commands(string home)
    {
        path = Path.Combine(home, "windows-shortcuts.json");
        try
        {
            if (JsonSerializer.Deserialize<Saved>(File.ReadAllText(path), LayoutStore.Options) is { Schema: 1 } saved)
            {
                SendMode = saved.SendMode ?? SendMode.Enter;
                foreach (var (command, keys) in saved.Shortcuts.Where(entry => Table.ContainsKey(entry.Key)))
                {
                    changed[command] = keys is { } set ? new Shortcut((VirtualKeyModifiers)set.Modifiers, (VirtualKey)set.Key) : null;
                }
            }
        }
        catch (Exception failure) when (failure is IOException or UnauthorizedAccessException or JsonException or NotSupportedException)
        {
        }
    }

    public Shortcut? Bound(Command command) => changed.TryGetValue(command, out var shortcut) ? shortcut : Table[command].Default;

    public bool IsChanged(Command command) => changed.ContainsKey(command);

    public void SetSendMode(SendMode mode)
    {
        SendMode = mode;
        Write();
    }

    // The composer's keys as the send mode has them.
    public string SendKey => SendMode == SendMode.Enter ? "Enter" : "Ctrl+Enter";
    public string SteerKey => SendMode == SendMode.Enter ? "Ctrl+Enter" : "Ctrl+Shift+Enter";
    public string NewLineKey => SendMode == SendMode.Enter ? "Shift+Enter" : "Enter";

    public string Shortcut(Command command) => Bound(command)?.ToString() ?? "";

    // Bind gives a command a shortcut, or none; it returns why it cannot.
    public string? Bind(Command command, Shortcut? shortcut)
    {
        if (shortcut is { } keys)
        {
            var modified = keys.Modifiers.HasFlag(VirtualKeyModifiers.Control) || keys.Modifiers.HasFlag(VirtualKeyModifiers.Menu);
            if (!modified && keys.Key is not (>= VirtualKey.F1 and <= VirtualKey.F12))
            {
                return "需要带 Ctrl 或 Alt，或使用 F1–F12。";
            }
            if (Reserved(keys) is { } owner)
            {
                return $"{keys} 已用于{owner}。";
            }
            if (Table.Keys.Where(other => other != command && Bound(other) == keys).Select(other => (Command?)other).FirstOrDefault() is { } taken)
            {
                return $"{keys} 已用于“{Table[taken].Name}”。";
            }
        }
        if (shortcut == Table[command].Default)
        {
            changed.Remove(command);
        }
        else
        {
            changed[command] = shortcut;
        }
        Write();
        return null;
    }

    // Reserved names what owns a shortcut Velune's commands cannot take:
    // the composer's sending, text editing and Windows itself.
    static string? Reserved(Shortcut keys)
    {
        const VirtualKeyModifiers control = VirtualKeyModifiers.Control;
        return (keys.Modifiers, keys.Key) switch
        {
            (control, VirtualKey.Enter) => "输入框的发送与插入本轮",
            (control | VirtualKeyModifiers.Shift, VirtualKey.Enter) => "输入框的插入本轮",
            (VirtualKeyModifiers.Menu, VirtualKey.Up) => "输入框的编辑最后一条排队消息",
            (control, VirtualKey.Back) => "审批卡片的取消请求与文字编辑",
            (control, VirtualKey.C or VirtualKey.V or VirtualKey.X or VirtualKey.A or VirtualKey.Z or VirtualKey.Y) => "文字编辑",
            (VirtualKeyModifiers.Menu, VirtualKey.F4) => "Windows 关闭窗口",
            (control, VirtualKey.F4) => "标签栏关闭标签",
            _ => null,
        };
    }

    public void Reset(Command command)
    {
        changed.Remove(command);
        Write();
    }

    public void ResetAll()
    {
        changed.Clear();
        Write();
    }

    void Write()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            var saved = new Saved
            {
                Shortcuts = changed.ToDictionary(entry => entry.Key, entry => entry.Value is { } keys ? new Keys((int)keys.Modifiers, (int)keys.Key) : null),
                SendMode = SendMode == SendMode.Enter ? null : SendMode,
            };
            var temporary = path + ".tmp";
            File.WriteAllText(temporary, JsonSerializer.Serialize(saved, LayoutStore.Options));
            File.Move(temporary, path, true);
        }
        catch (Exception failure) when (failure is IOException or UnauthorizedAccessException)
        {
            // The shortcut works until the app quits; the next change tries again.
        }
        Changed?.Invoke();
    }

    // Install puts the commands' shortcuts on a window's root.
    public void Install(UIElement root, Action<Command> run)
    {
        root.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        root.KeyboardAccelerators.Clear();
        foreach (var command in Table.Keys.Where(command => !IsGlobal(command)))
        {
            if (Bound(command) is not { } keys)
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
