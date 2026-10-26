using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;
using Microsoft.Windows.Storage.Pickers;

namespace Velune;

// What the windows ask of Windows itself.
static partial class Native
{
    [LibraryImport("user32.dll")]
    private static partial uint GetDpiForWindow(nint window);

    [StructLayout(LayoutKind.Sequential)]
    struct Point
    {
        public int X;
        public int Y;
    }

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetCursorPos(out Point point);

    [LibraryImport("comctl32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static unsafe partial bool SetWindowSubclass(nint window, delegate* unmanaged[Stdcall]<nint, uint, nint, nint, nuint, nuint, nint> procedure, nuint id, nuint data);

    [LibraryImport("comctl32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static unsafe partial bool RemoveWindowSubclass(nint window, delegate* unmanaged[Stdcall]<nint, uint, nint, nint, nuint, nuint, nint> procedure, nuint id);

    [LibraryImport("comctl32.dll")]
    private static partial nint DefSubclassProc(nint window, uint message, nint wParam, nint lParam);

    [LibraryImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static partial nint SetWindowLongPtr(nint window, int index, nint value);

    [StructLayout(LayoutKind.Sequential)]
    struct FlashInfo
    {
        public uint Size;
        public nint Window;
        public uint Flags;
        public uint Count;
        public uint Timeout;
    }

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool FlashWindowEx(ref FlashInfo info);

    // The taskbar button flashes until the window comes to the front.
    const uint FlashTrayUntilForeground = 0x2 | 0xC;

    const uint ExitSizeMove = 0x0232;
    const int OwnerIndex = -8;

    // The windows told when the user stops moving or sizing them.
    static readonly Dictionary<nint, Action> movedWindows = [];

    static nint Handle(Window window) => WinRT.Interop.WindowNative.GetWindowHandle(window);

    // Flash asks for attention on the window's taskbar button until the
    // user brings it to the front.
    public static void Flash(Window window)
    {
        var info = new FlashInfo { Size = (uint)Marshal.SizeOf<FlashInfo>(), Window = Handle(window), Flags = FlashTrayUntilForeground };
        FlashWindowEx(ref info);
    }

    // SetOwner keeps a window above its owner, minimized with it and off the
    // taskbar, as a tool palette.
    public static void SetOwner(Window window, Window owner) => SetWindowLongPtr(Handle(window), OwnerIndex, Handle(owner));

    // OnMoved calls done each time the user lets go of the window after
    // moving or sizing it; the call comes from the window procedure, so done
    // only queues its work.
    public static unsafe void OnMoved(Window window, Action done)
    {
        var handle = Handle(window);
        movedWindows[handle] = done;
        SetWindowSubclass(handle, &Subclass, 1, 0);
    }

    public static unsafe void ForgetMoved(Window window)
    {
        var handle = Handle(window);
        if (movedWindows.Remove(handle))
        {
            RemoveWindowSubclass(handle, &Subclass, 1);
        }
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    static nint Subclass(nint window, uint message, nint wParam, nint lParam, nuint id, nuint data)
    {
        if (message == ExitSizeMove && movedWindows.TryGetValue(window, out var done))
        {
            done();
        }
        return DefSubclassProc(window, message, wParam, lParam);
    }

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool RegisterHotKey(nint window, int id, uint modifiers, uint key);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool UnregisterHotKey(nint window, int id);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool SetForegroundWindow(nint window);

    const uint HotKeyMessage = 0x0312;
    const uint NoRepeat = 0x4000;
    // The window the system-wide shortcuts are registered on, how many, and
    // what a press does; the call comes from the window procedure, so it only
    // queues its work.
    static nint hotKeyWindow;
    static int hotKeyCount;
    static Action<int>? hotKeyPressed;

    // SetHotKeys registers system-wide shortcuts on a window, replacing those
    // registered before; it returns the indexes the system or another app holds.
    public static unsafe List<int> SetHotKeys(Window? window, IReadOnlyList<Shortcut> keys, Action<int> pressed)
    {
        if (hotKeyWindow != 0)
        {
            for (var i = 0; i < hotKeyCount; i++)
            {
                UnregisterHotKey(hotKeyWindow, i + 1);
            }
            RemoveWindowSubclass(hotKeyWindow, &HotKeySubclass, 2);
            hotKeyWindow = 0;
            hotKeyCount = 0;
        }
        var failed = new List<int>();
        if (window == null || keys.Count == 0)
        {
            return failed;
        }
        hotKeyPressed = pressed;
        hotKeyWindow = Handle(window);
        hotKeyCount = keys.Count;
        SetWindowSubclass(hotKeyWindow, &HotKeySubclass, 2, 0);
        for (var i = 0; i < keys.Count; i++)
        {
            var modifiers = keys[i].Modifiers;
            // RegisterHotKey's flags: Alt 1, Ctrl 2, Shift 4, Win 8.
            var flags = NoRepeat
                | (modifiers.HasFlag(Windows.System.VirtualKeyModifiers.Menu) ? 1u : 0)
                | (modifiers.HasFlag(Windows.System.VirtualKeyModifiers.Control) ? 2u : 0)
                | (modifiers.HasFlag(Windows.System.VirtualKeyModifiers.Shift) ? 4u : 0)
                | (modifiers.HasFlag(Windows.System.VirtualKeyModifiers.Windows) ? 8u : 0);
            if (!RegisterHotKey(hotKeyWindow, i + 1, flags, (uint)keys[i].Key))
            {
                failed.Add(i);
            }
        }
        return failed;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    static nint HotKeySubclass(nint window, uint message, nint wParam, nint lParam, nuint id, nuint data)
    {
        if (message == HotKeyMessage && wParam > 0)
        {
            hotKeyPressed?.Invoke((int)wParam - 1);
        }
        return DefSubclassProc(window, message, wParam, lParam);
    }

    // Foreground brings the window in front of other apps' windows, which a
    // global shortcut's press allows.
    public static void Foreground(Window window) => SetForegroundWindow(Handle(window));

    // Cursor is where the pointer is on the screen, in physical pixels as
    // windows are placed.
    public static Windows.Graphics.PointInt32 Cursor() =>
        GetCursorPos(out var point) ? new(point.X, point.Y) : new(0, 0);

    // Scale is the window's display scale, for sizes given in effective pixels.
    public static double Scale(Window window) =>
        GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(window)) / 96.0;

    public static async Task<string?> PickFolder(Window window)
    {
        var picker = new FolderPicker(window.AppWindow.Id);
        return (await picker.PickSingleFolderAsync())?.Path;
    }

    public static async Task<string?> PickFile(Window window)
    {
        var picker = new FileOpenPicker(window.AppWindow.Id);
        picker.FileTypeFilter.Add("*");
        return (await picker.PickSingleFileAsync())?.Path;
    }

    public static async Task<string[]> PickFiles(Window window)
    {
        var picker = new FileOpenPicker(window.AppWindow.Id);
        picker.FileTypeFilter.Add("*");
        return [.. (await picker.PickMultipleFilesAsync()).Select(file => file.Path)];
    }
}
