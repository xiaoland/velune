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
    CompactContext,
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

    // Each command's title, its English name, which the palette and the /
    // list also find it by without showing it (DESIGN.md, 检索别名), and its
    // default shortcut.
    public static readonly IReadOnlyDictionary<Command, (string Name, string English, Shortcut? Default)> Table = new Dictionary<Command, (string, string, Shortcut?)>
    {
        [Command.NewConversation] = ("新建会话", "New Conversation", new(Ctrl, VirtualKey.N)),
        [Command.NewTab] = ("新建标签页", "New Tab", new(Ctrl, VirtualKey.T)),
        [Command.NewWindow] = ("新建窗口", "New Window", new(Ctrl | Shift, VirtualKey.N)),
        [Command.CloseTab] = ("关闭标签页", "Close Tab", new(Ctrl, VirtualKey.W)),
        [Command.CloseWindow] = ("关闭窗口", "Close Window", new(Ctrl | Shift, VirtualKey.W)),
        [Command.SplitRight] = ("向右拆分", "Split Right", new(Ctrl, (VirtualKey)220)),
        [Command.SplitDown] = ("向下拆分", "Split Down", new(Ctrl | Shift, (VirtualKey)220)),
        [Command.PreviousPane] = ("上一个分屏", "Previous Pane", new(Ctrl, (VirtualKey)219)),
        [Command.NextPane] = ("下一个分屏", "Next Pane", new(Ctrl, (VirtualKey)221)),
        [Command.PreviousTab] = ("上一个标签页", "Previous Tab", new(Ctrl | Shift, VirtualKey.Tab)),
        [Command.NextTab] = ("下一个标签页", "Next Tab", new(Ctrl, VirtualKey.Tab)),
        [Command.Back] = ("后退", "Back", new(Alt, VirtualKey.Left)),
        [Command.Forward] = ("前进", "Forward", new(Alt, VirtualKey.Right)),
        [Command.ToggleLeft] = ("显示／隐藏左侧区域", "Toggle Left Area", new(Ctrl, VirtualKey.B)),
        [Command.ToggleRight] = ("显示／隐藏右侧区域", "Toggle Right Area", new(Ctrl | Alt, VirtualKey.B)),
        [Command.ToggleBottom] = ("显示／隐藏底部区域", "Toggle Bottom Area", new(Ctrl, VirtualKey.J)),
        [Command.EditorOnly] = ("只显示编辑区", "Editor Only", new(Ctrl | Shift, VirtualKey.E)),
        [Command.ShowList] = ("显示／隐藏会话列表", "Toggle Conversation List", new(Ctrl, VirtualKey.Number1)),
        [Command.ShowOutline] = ("显示／隐藏会话大纲", "Toggle Outline", new(Ctrl, VirtualKey.Number2)),
        [Command.FocusComposer] = ("转到输入区", "Go to Composer", new(Ctrl, VirtualKey.Number3)),
        [Command.ShowLinked] = ("显示／隐藏关联 Agent", "Toggle Linked Agents", new(Ctrl, VirtualKey.Number4)),
        [Command.FindConversation] = ("搜索会话", "Find Conversation", new(Ctrl, VirtualKey.F)),
        [Command.Rename] = ("重命名会话", "Rename Conversation", new(VirtualKeyModifiers.None, VirtualKey.F2)),
        [Command.Settings] = ("设置", "Settings", new(Ctrl, (VirtualKey)188)),
        [Command.Problems] = ("问题", "Problems", new(Ctrl | Shift, VirtualKey.M)),
        [Command.Analytics] = ("分析", "Analytics", null),
        [Command.Library] = ("资源库", "Resource Library", null),
        [Command.CommandPalette] = ("命令面板", "Command Palette", new(Ctrl, VirtualKey.K)),
        [Command.OpenConversation] = ("打开会话…", "Open Conversation", new(Ctrl, VirtualKey.P)),
        [Command.NextAttention] = ("转到待处理的会话", "Go to Next Conversation Needing Attention", new(Ctrl | Shift, VirtualKey.A)),
        [Command.FocusTranscript] = ("转到消息", "Go to Messages", new(Ctrl, VirtualKey.Number0)),
        [Command.ShortcutHelp] = ("快捷键速查", "Keyboard Shortcuts", new(Ctrl, (VirtualKey)191)),
        [Command.Attach] = ("添加附件…", "Attach Files", new(Ctrl, VirtualKey.O)),
        // Not Ctrl+Alt: on many layouts it is AltGr, which types characters.
        [Command.ChooseRuntime] = ("选择运行时…", "Choose Runtime", new(Ctrl | Shift, VirtualKey.R)),
        [Command.ChooseModel] = ("选择模型…", "Choose Model", new(Ctrl | Shift, VirtualKey.O)),
        [Command.ChooseEffort] = ("选择推理强度…", "Choose Reasoning Effort", new(Ctrl | Shift, VirtualKey.I)),
        [Command.ChoosePermission] = ("选择权限模式…", "Choose Permission Mode", new(Ctrl | Shift, VirtualKey.P)),
        [Command.Continue] = ("继续", "Continue", null),
        [Command.Stop] = ("停止生成", "Stop", null),
        [Command.GlobalShow] = ("显示 Velune（全局）", "Show Velune (Global)", null),
        [Command.GlobalNewConversation] = ("新建会话（全局）", "New Conversation (Global)", null),
        [Command.CompactContext] = ("压缩上下文", "Compact Context", null),
    };

    // Other words people look for a command by, in either language.
    static readonly IReadOnlyDictionary<Command, string[]> Keywords = new Dictionary<Command, string[]>
    {
        [Command.NewConversation] = ["chat", "session"],
        [Command.OpenConversation] = ["quick open", "switch conversation", "切换会话"],
        [Command.FindConversation] = ["search", "filter", "搜索"],
        [Command.Stop] = ["cancel", "interrupt", "abort", "中断", "取消"],
        [Command.Continue] = ["resume", "retry", "重试"],
        [Command.Attach] = ["attachment", "upload", "file", "image", "文件", "图片"],
        [Command.ChooseRuntime] = ["switch runtime", "agent", "harness", "切换运行时"],
        [Command.ChooseModel] = ["switch model", "llm", "切换模型"],
        [Command.ChooseEffort] = ["reasoning", "thinking", "切换推理强度", "思考"],
        [Command.ChoosePermission] = ["approval", "sandbox", "切换权限", "审批"],
        [Command.NextAttention] = ["pending", "waiting", "approval", "unread", "待处理", "未读"],
        [Command.SplitRight] = ["pane", "分屏"],
        [Command.SplitDown] = ["pane", "分屏"],
        [Command.NextPane] = ["split", "分屏"],
        [Command.PreviousPane] = ["split", "分屏"],
        [Command.CommandPalette] = ["commands"],
        [Command.FocusTranscript] = ["transcript", "消息列表"],
        [Command.ShowList] = ["sidebar", "侧栏"],
        [Command.FocusComposer] = ["input", "prompt", "输入框"],
        [Command.ShowLinked] = ["subagent", "子 Agent"],
        [Command.EditorOnly] = ["zen", "maximize", "专注"],
        [Command.Problems] = ["errors", "diagnostics", "issues", "错误"],
        [Command.ShortcutHelp] = ["keys", "keybindings", "cheat sheet", "help", "快捷键"],
        [Command.Settings] = ["preferences", "options", "选项"],
        [Command.Analytics] = ["usage", "tokens", "用量"],
        [Command.Library] = ["skills", "mcp", "instructions", "技能"],
    };

    // Velune's commands as / names them; any other /… goes to the runtime as typed.
    public static readonly (string Name, Command Command)[] Slash =
    [
        ("model", Command.ChooseModel),
        ("effort", Command.ChooseEffort),
        ("permission", Command.ChoosePermission),
        ("runtime", Command.ChooseRuntime),
        ("new", Command.NewConversation),
        ("continue", Command.Continue),
        ("stop", Command.Stop),
        ("attach", Command.Attach),
        ("open", Command.OpenConversation),
        ("find", Command.FindConversation),
        ("commands", Command.CommandPalette),
        ("keys", Command.ShortcutHelp),
    ];

    // SearchTerms are what a command is found by besides its title: its
    // English name, its keywords and its / name.
    public static string[] SearchTerms(Command command) =>
        [Table[command].English, .. Keywords.GetValueOrDefault(command, []), .. Slash.Where(entry => entry.Command == command).Select(entry => entry.Name)];

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
