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
