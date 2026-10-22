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

    const uint ExitSizeMove = 0x0232;
    const int OwnerIndex = -8;

    // The windows told when the user stops moving or sizing them.
    static readonly Dictionary<nint, Action> movedWindows = [];

    static nint Handle(Window window) => WinRT.Interop.WindowNative.GetWindowHandle(window);

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
