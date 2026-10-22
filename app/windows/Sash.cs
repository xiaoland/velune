using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// Sash divides two parts of a window and resizes them: a hairline with a
// wider grip over it. Dragging changes the sizes live; the layout keeps them
// on release; a double click restores the default or even sizes.
sealed class Sash : Grid
{
    const double Grip = 6;

    // Across tells whether it divides left from right (dragged sideways);
    // line draws the hairline where nothing else divides the parts.
    public Sash(bool across, Action<double> drag, Action done, Action reset, bool line = false)
    {
        Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
        ProtectedCursor = InputSystemCursor.Create(across ? InputSystemCursorShape.SizeWestEast : InputSystemCursorShape.SizeNorthSouth);
        Canvas.SetZIndex(this, 10);
        if (across)
        {
            Width = Grip;
            HorizontalAlignment = HorizontalAlignment.Left;
            Margin = new Thickness(-Grip / 2, 0, 0, 0);
        }
        else
        {
            Height = Grip;
            VerticalAlignment = VerticalAlignment.Top;
            Margin = new Thickness(0, -Grip / 2, 0, 0);
        }
        if (line)
        {
            Children.Add(new Border
            {
                Style = Ui.Style("DividerStyle"),
                Width = across ? 1 : double.NaN,
                Height = across ? double.NaN : 1,
                HorizontalAlignment = across ? HorizontalAlignment.Center : HorizontalAlignment.Stretch,
                VerticalAlignment = across ? VerticalAlignment.Stretch : VerticalAlignment.Center,
            });
        }
        double? start = null;
        PointerPressed += (_, e) =>
        {
            if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
            {
                return;
            }
            var point = e.GetCurrentPoint(null).Position;
            start = across ? point.X : point.Y;
            CapturePointer(e.Pointer);
            e.Handled = true;
        };
        PointerMoved += (_, e) =>
        {
            if (start is { } from)
            {
                var point = e.GetCurrentPoint(null).Position;
                drag((across ? point.X : point.Y) - from);
                e.Handled = true;
            }
        };
        void Release()
        {
            if (start != null)
            {
                start = null;
                done();
            }
        }
        PointerReleased += (_, e) =>
        {
            ReleasePointerCapture(e.Pointer);
            Release();
            e.Handled = true;
        };
        PointerCaptureLost += (_, _) => Release();
        DoubleTapped += (_, e) =>
        {
            e.Handled = true;
            reset();
        };
    }
}
