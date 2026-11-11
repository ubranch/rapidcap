// Region and window selection: one borderless window per display, dimmed,
// with the selection cut out of the dim.

import AppKit

/// What the user picked, in both coordinate systems the app needs: AppKit
/// points for the frame and HUD windows, core pixels for the capture.
struct Pick: Sendable {
    /// AppKit global points, bottom-left origin.
    let points: CGRect
    let processName: String?

    var pixels: PixelRect {
        let primary = NSScreen.screens.first
        let height = primary?.frame.maxY ?? 0
        let scale = primary?.backingScaleFactor ?? 2
        let top = height - points.maxY
        return PixelRect(
            x: Int32((points.minX * scale).rounded()),
            y: Int32((top * scale).rounded()),
            width: UInt32((points.width * scale).rounded()),
            height: UInt32((points.height * scale).rounded())
        )
    }
}

/// Core Graphics top-left points to AppKit bottom-left points.
func appKitRect(fromCG rect: CGRect) -> CGRect {
    let height = NSScreen.screens.first?.frame.maxY ?? 0
    return CGRect(x: rect.minX, y: height - rect.maxY, width: rect.width, height: rect.height)
}

private final class KeyWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

@MainActor
final class OverlayController {
    private var windows: [NSWindow] = []
    private var done: ((Pick?) -> Void)?
    fileprivate var pickWindow = false
    fileprivate var dragStart: CGPoint?
    fileprivate var selection: CGRect?
    fileprivate var hovered: (rect: CGRect, name: String)?

    func open(pickWindow: Bool, done: @escaping (Pick?) -> Void) {
        close()
        self.pickWindow = pickWindow
        self.done = done
        dragStart = nil
        selection = nil
        hovered = nil
        for screen in NSScreen.screens {
            let window = KeyWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
            window.level = .screenSaver
            window.isOpaque = false
            window.backgroundColor = .clear
            window.hasShadow = false
            window.ignoresMouseEvents = false
            window.acceptsMouseMovedEvents = true
            window.sharingType = .none
            window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
            window.contentView = OverlayView(controller: self, screenFrame: screen.frame)
            window.setFrame(screen.frame, display: true)
            window.orderFrontRegardless()
            windows.append(window)
        }
        NSApp.activate()
        let mouse = NSEvent.mouseLocation
        let under = windows.first { $0.frame.contains(mouse) } ?? windows.first
        under?.makeKey()
        under?.makeFirstResponder(under?.contentView)
        NSCursor.crosshair.push()
        if pickWindow { updateHover(at: mouse) }
    }

    func close() {
        guard !windows.isEmpty else { return }
        NSCursor.pop()
        windows.forEach { $0.orderOut(nil) }
        windows.removeAll()
    }

    fileprivate func finish(_ pick: Pick?) {
        let done = self.done
        self.done = nil
        close()
        done?(pick)
    }

    fileprivate func redraw() {
        windows.forEach { $0.contentView?.needsDisplay = true }
    }

    fileprivate func updateHover(at point: CGPoint) {
        hovered = Self.window(at: point)
        redraw()
    }

    /// The topmost ordinary window under a point, excluding our own.
    private static func window(at point: CGPoint) -> (rect: CGRect, name: String)? {
        let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
            return nil
        }
        let ownPid = ProcessInfo.processInfo.processIdentifier
        for info in list {
            guard (info[kCGWindowLayer as String] as? Int) == 0,
                  (info[kCGWindowOwnerPID as String] as? Int32) != ownPid,
                  let boundsDict = info[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: boundsDict),
                  bounds.width > 40, bounds.height > 40
            else { continue }
            let rect = appKitRect(fromCG: bounds)
            if rect.contains(point) {
                return (rect, info[kCGWindowOwnerName as String] as? String ?? "Window")
            }
        }
        return nil
    }
}

private final class OverlayView: NSView {
    unowned let controller: OverlayController
    let screenFrame: CGRect

    init(controller: OverlayController, screenFrame: CGRect) {
        self.controller = controller
        self.screenFrame = screenFrame
        super.init(frame: CGRect(origin: .zero, size: screenFrame.size))
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .activeAlways, .inVisibleRect],
            owner: self
        ))
    }

    private func global(_ event: NSEvent) -> CGPoint { NSEvent.mouseLocation }

    override func mouseMoved(with event: NSEvent) {
        if controller.pickWindow { controller.updateHover(at: global(event)) }
    }

    override func mouseDown(with event: NSEvent) {
        if controller.pickWindow { return }
        controller.dragStart = global(event)
        controller.selection = nil
    }

    override func mouseDragged(with event: NSEvent) {
        guard !controller.pickWindow, let start = controller.dragStart else { return }
        // Kept on the display the drag began on. The recorder captures one
        // display, so a region across two would lose its other half.
        let screen = NSScreen.screens.first { $0.frame.contains(start) }?.frame ?? screenFrame
        let mouse = global(event)
        let point = CGPoint(x: min(max(mouse.x, screen.minX), screen.maxX),
                            y: min(max(mouse.y, screen.minY), screen.maxY))
        controller.selection = CGRect(
            x: min(start.x, point.x), y: min(start.y, point.y),
            width: abs(point.x - start.x), height: abs(point.y - start.y)
        )
        controller.redraw()
    }

    override func mouseUp(with event: NSEvent) {
        if controller.pickWindow {
            if let hovered = controller.hovered {
                controller.finish(Pick(points: hovered.rect, processName: hovered.name))
            }
            return
        }
        guard let selection = controller.selection, selection.width >= 8, selection.height >= 8 else {
            controller.dragStart = nil
            controller.selection = nil
            controller.redraw()
            return
        }
        controller.finish(Pick(points: selection.integral, processName: nil))
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { controller.finish(nil) } // Escape
    }

    override func rightMouseDown(with event: NSEvent) { controller.finish(nil) }

    override func draw(_ dirtyRect: NSRect) {
        let hole = (controller.pickWindow ? controller.hovered?.rect : controller.selection)
            .map { $0.offsetBy(dx: -screenFrame.minX, dy: -screenFrame.minY) }
        let scrim = NSBezierPath(rect: bounds)
        if let hole {
            scrim.append(NSBezierPath(roundedRect: hole, xRadius: 10, yRadius: 10))
            scrim.windingRule = .evenOdd
        }
        NSColor.black.withAlphaComponent(0.42).setFill()
        scrim.fill()

        guard let hole else {
            drawHint(controller.pickWindow ? "Click a window to capture it · Esc to cancel"
                : "Drag to select · Esc to cancel")
            return
        }
        let edge = NSBezierPath(roundedRect: hole.insetBy(dx: -1, dy: -1), xRadius: 11, yRadius: 11)
        edge.lineWidth = 2
        NSColor.controlAccentColor.setStroke()
        edge.stroke()

        let label: String
        if controller.pickWindow {
            label = controller.hovered?.name ?? ""
        } else {
            let scale = window?.backingScaleFactor ?? 2
            label = "\(Int(hole.width * scale)) × \(Int(hole.height * scale))"
        }
        drawBadge(label, below: hole)
    }

    private func attributes() -> [NSAttributedString.Key: Any] {
        [.font: NSFont.systemFont(ofSize: 12, weight: .medium), .foregroundColor: NSColor.white]
    }

    private func drawBadge(_ text: String, below rect: CGRect) {
        let string = NSAttributedString(string: text, attributes: attributes())
        let size = string.size()
        var badge = CGRect(x: rect.midX - size.width / 2 - 10, y: rect.minY - size.height - 18,
                           width: size.width + 20, height: size.height + 8)
        if badge.minY < 8 { badge.origin.y = rect.minY + 10 }
        NSColor.black.withAlphaComponent(0.6).setFill()
        NSBezierPath(roundedRect: badge, xRadius: badge.height / 2, yRadius: badge.height / 2).fill()
        string.draw(at: CGPoint(x: badge.minX + 10, y: badge.minY + 4))
    }

    private func drawHint(_ text: String) {
        let mouse = NSEvent.mouseLocation
        guard screenFrame.contains(mouse) else { return }
        let string = NSAttributedString(string: text, attributes: attributes())
        let size = string.size()
        let badge = CGRect(x: bounds.midX - size.width / 2 - 14, y: bounds.maxY - 120,
                           width: size.width + 28, height: size.height + 14)
        NSColor.black.withAlphaComponent(0.6).setFill()
        NSBezierPath(roundedRect: badge, xRadius: badge.height / 2, yRadius: badge.height / 2).fill()
        string.draw(at: CGPoint(x: badge.minX + 14, y: badge.minY + 7))
    }
}
