// The recording HUD and the frame around the recorded region.
//
// Both are marked unshareable, so neither lands in the recording.

import AppKit
import SwiftUI


@MainActor
private func floatingWindow(_ rect: CGRect, mouse: Bool) -> NSPanel {
    let panel = NSPanel(contentRect: rect, styleMask: [.borderless, .nonactivatingPanel],
                        backing: .buffered, defer: false)
    panel.level = .statusBar
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = false
    panel.ignoresMouseEvents = !mouse
    panel.sharingType = .none
    panel.isMovableByWindowBackground = mouse
    panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
    panel.hidesOnDeactivate = false
    return panel
}

// MARK: frame

@MainActor
final class FrameController {
    private var window: NSPanel?
    private static let pad: CGFloat = 3

    func show(around rect: CGRect, model: AppModel) {
        close()
        let outer = rect.insetBy(dx: -Self.pad, dy: -Self.pad)
        let window = floatingWindow(outer, mouse: false)
        let host = NSHostingView(rootView: FrameView(model: model))
        host.sizingOptions = []
        window.contentView = host
        window.setFrame(outer, display: true)
        fadeIn(window)
        self.window = window
    }

    func close() {
        fadeOut(window)
        window = nil
    }
}

private struct FrameView: View {
    let model: AppModel

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(Color.accentColor.opacity(0.9), lineWidth: 2)
                .padding(1)
            if case let .countdown(_, seconds) = model.state, seconds > 0 {
                CountdownBadge(seconds: seconds)
                    .transition(.scale(scale: 0.6).combined(with: .opacity))
            }
        }
        .animation(.spring(response: 0.4, dampingFraction: 0.7), value: model.state)
    }
}

/// The big number in the middle of the region while the countdown runs. Each
/// tick rolls the digit and pulses the ring, so the wait reads as a timer and
/// not as a frozen screen.
private struct CountdownBadge: View {
    let seconds: Int

    var body: some View {
        Text("\(seconds)")
            .font(.system(size: 44, weight: .semibold, design: .rounded).monospacedDigit())
            .contentTransition(.numericText(countsDown: true))
            .frame(width: 92, height: 92)
            .glassEffect(.regular, in: .circle)
            .overlay {
                Circle()
                    .stroke(Color.accentColor.opacity(0.6), lineWidth: 2)
                    .phaseAnimator([false, true], trigger: seconds) { ring, expanding in
                        ring.scaleEffect(expanding ? 1.25 : 1).opacity(expanding ? 0 : 0.9)
                    } animation: { _ in .easeOut(duration: 0.8) }
            }
            .animation(.spring(response: 0.35, dampingFraction: 0.65), value: seconds)
    }
}

/// Windows arrive and leave with a short fade rather than popping.
@MainActor
private func fadeIn(_ window: NSWindow) {
    window.alphaValue = 0
    window.orderFrontRegardless()
    NSAnimationContext.runAnimationGroup { context in
        context.duration = 0.18
        window.animator().alphaValue = 1
    }
}

@MainActor
private func fadeOut(_ window: NSWindow?) {
    guard let window else { return }
    NSAnimationContext.runAnimationGroup({ context in
        context.duration = 0.15
        window.animator().alphaValue = 0
    }, completionHandler: {
        window.orderOut(nil)
    })
}

// MARK: HUD

@MainActor
final class HudController {
    private var window: NSPanel?
    static let size = CGSize(width: 164, height: 34)

    func show(model: AppModel, near rect: CGRect) {
        close()
        let screen = NSScreen.screens.first { $0.frame.intersects(rect) } ?? NSScreen.main
        let visible = screen?.visibleFrame ?? rect
        let size = Self.size
        var origin = CGPoint(x: rect.midX - size.width / 2, y: rect.minY - size.height - 10)
        if origin.y < visible.minY + 8 { origin.y = rect.maxY + 10 }
        if origin.y + size.height > visible.maxY - 8 { origin.y = rect.minY + 10 }
        origin.x = min(max(origin.x, visible.minX + 8), visible.maxX - size.width - 8)
        let frame = CGRect(origin: origin, size: size)
        let window = floatingWindow(frame, mouse: true)
        let host = NSHostingView(rootView: HudView(model: model))
        host.sizingOptions = []
        window.contentView = host
        window.setFrame(frame, display: true)
        fadeIn(window)
        self.window = window
    }

    func close() {
        fadeOut(window)
        window = nil
    }
}

private struct HudView: View {
    let model: AppModel

    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.5)) { context in
            HStack(spacing: 4) {
                status(at: context.date)
                    .frame(maxWidth: .infinity)
                HudButton(symbol: isPaused ? "play.fill" : "pause.fill",
                          help: isPaused ? String(localized: "Resume") : String(localized: "Pause"),
                          prominent: false, enabled: canPause) { model.togglePause() }
                HudButton(symbol: canPause ? "stop.fill" : "xmark",
                          help: canPause ? String(localized: "Stop") : String(localized: "Cancel"),
                          prominent: canPause, enabled: true) {
                    if canPause { model.trigger(model.state.kind ?? .video) } else { model.cancel() }
                }
            }
            .padding(.leading, 10)
            .padding(.trailing, 5)
            .frame(width: HudController.size.width, height: HudController.size.height)
            .glassEffect(.regular, in: .capsule)
        }
    }

    private var isPaused: Bool {
        if case .paused = model.state { return true }
        return false
    }

    private var canPause: Bool {
        switch model.state {
        case .recording, .paused: true
        default: false
        }
    }

    @ViewBuilder
    private func status(at now: Date) -> some View {
        switch model.state {
        case let .countdown(kind, seconds):
            Text(seconds > 0 ? String(localized: "\(kind.noun) in \(seconds)") : String(localized: "Starting…"))
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
        case .recording, .paused:
            HStack(spacing: 6) {
                RecordingDot(active: !isPaused, now: now)
                Text(clock(model.elapsed(at: now)))
                    .font(.system(size: 12, weight: .semibold).monospacedDigit())
            }
        case let .finalizing(kind):
            Text("Saving \(kind.noun)…")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
        default:
            EmptyView()
        }
    }
}

/// A tap target, not a `Button`: the HUD never takes the keyboard, and a
/// button there gave SwiftUI's keyboard-focus bookkeeping a key-view loop to
/// rebuild on every tick of the clock.
private struct HudButton: View {
    let symbol: String
    let help: String
    let prominent: Bool
    let enabled: Bool
    let action: () -> Void

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 9.5, weight: .bold))
            .foregroundStyle(prominent ? Color.white : Color.primary)
            .frame(width: 24, height: 24)
            .contentShape(.circle)
            .glassEffect(prominent ? .regular.tint(.accentColor).interactive() : .regular.interactive(), in: .circle)
            .opacity(enabled ? 1 : 0.4)
            .onTapGesture { if enabled { action() } }
            .help(help)
            .accessibilityElement()
            .accessibilityLabel(help)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { if enabled { action() } }
    }
}

/// Blinks on the clock it sits beside: dimmed every other half second.
///
/// Not a `repeatForever` animation. One of those never finishes, so when the
/// dot was removed under an animated change - a recording ending - SwiftUI
/// kept the removal running, and the whole window updated every frame from
/// then on. With Full Keyboard Access on, each of those updates rebuilt the
/// key-view loop: the likely cause of the app freezing on a second recording.
struct RecordingDot: View {
    let active: Bool
    let now: Date

    private var dim: Bool { active && Int(now.timeIntervalSinceReferenceDate * 2) % 2 == 1 }

    var body: some View {
        Circle()
            .fill(active ? Color.accentColor : Color.secondary)
            .frame(width: 6, height: 6)
            .opacity(dim ? 0.35 : 1)
            .animation(.easeInOut(duration: 0.4), value: dim)
    }
}

func clock(_ seconds: TimeInterval) -> String {
    let whole = Int(seconds)
    return String(format: "%02d:%02d", whole / 60, whole % 60)
}
