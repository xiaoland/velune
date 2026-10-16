import AppKit
import SwiftUI

/// A divider's grip, as in VS Code's sashes: wider than the line it moves,
/// with the system's resize pointer; a double click puts the sizes back.
/// An AppKit view laid over what it divides, since a SwiftUI gesture there
/// loses the pointer to a transcript's AppKit views beside the line.
struct SplitSash: NSViewRepresentable {
    static let thickness: CGFloat = 8
    /// `.horizontal` between views side by side: it moves left and right.
    let axis: Axis
    /// The distance dragged since the press, along the axis; right and down are positive.
    let drag: (CGFloat) -> Void
    let end: () -> Void
    let reset: () -> Void

    func makeNSView(context: Context) -> SashView { SashView() }

    func updateNSView(_ view: SashView, context: Context) {
        if view.axis != axis {
            view.axis = axis
            view.window?.invalidateCursorRects(for: view)
        }
        view.drag = drag
        view.end = end
        view.reset = reset
    }

    final class SashView: NSView {
        var axis: Axis = .horizontal
        var drag: (CGFloat) -> Void = { _ in }
        var end: () -> Void = {}
        var reset: () -> Void = {}
        private var cursor: NSCursor { axis == .horizontal ? .resizeLeftRight : .resizeUpDown }

        override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

        override func resetCursorRects() { addCursorRect(bounds, cursor: cursor) }

        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            trackingAreas.forEach(removeTrackingArea)
            addTrackingArea(NSTrackingArea(rect: .zero, options: [.cursorUpdate, .activeInActiveApp, .inVisibleRect], owner: self))
        }

        override func cursorUpdate(with event: NSEvent) { cursor.set() }

        /// The drag is tracked here until the button comes up, as a split
        /// view's divider is, so the pointer keeps its shape over other views.
        override func mouseDown(with event: NSEvent) {
            if event.clickCount == 2 { reset(); return }
            guard let window else { return }
            let start = event.locationInWindow
            cursor.push()
            window.trackEvents(matching: [.leftMouseDragged, .leftMouseUp], timeout: NSEvent.foreverDuration, mode: .eventTracking) { event, stop in
                guard let event else { return }
                if event.type == .leftMouseUp { stop.pointee = true; return }
                let point = event.locationInWindow
                self.drag(self.axis == .horizontal ? point.x - start.x : start.y - point.y)
            }
            NSCursor.pop()
            end()
        }
    }
}

/// Grips for the dividers between views laid out along an axis, `lengths`
/// long with a one-point line between each two. Laid over the views, after
/// them, so the pointer finds them first.
struct SashOverlay: View {
    let axis: Axis
    let lengths: [CGFloat]
    /// The divider before view `index` dragged this far since the press.
    let drag: (_ index: Int, _ distance: CGFloat) -> Void
    let end: () -> Void
    let reset: () -> Void

    var body: some View {
        GeometryReader { geometry in
            let horizontal = axis == .horizontal
            ForEach(Array(lengths.indices.dropFirst()), id: \.self) { index in
                let line = lengths[..<index].reduce(0, +) + CGFloat(index - 1) + 0.5
                SplitSash(axis: axis, drag: { drag(index, $0) }, end: end, reset: reset)
                    .frame(width: horizontal ? SplitSash.thickness : geometry.size.width,
                           height: horizontal ? geometry.size.height : SplitSash.thickness)
                    .position(x: horizontal ? line : geometry.size.width / 2, y: horizontal ? geometry.size.height / 2 : line)
            }
        }
    }
}
