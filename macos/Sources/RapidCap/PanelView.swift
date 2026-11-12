// The main panel: countdown, the four capture modes, and the footer.

import SwiftUI

struct PanelView: View {
    let model: AppModel

    var body: some View {
        GlassEffectContainer(spacing: 8) {
            VStack(spacing: 8) {
                header
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)],
                          spacing: 8) {
                    ModeCard(model: model, kind: .region, symbol: "rectangle.dashed",
                             title: String(localized: "Region"), shortcut: .region)
                    ModeCard(model: model, kind: .window, symbol: "macwindow",
                             title: String(localized: "Window"), shortcut: .window)
                    ModeCard(model: model, kind: .video, symbol: "video",
                             title: String(localized: "Video"), shortcut: .video)
                    ModeCard(model: model, kind: .gif, symbol: "photo.stack",
                             title: String(localized: "GIF"), shortcut: .gif)
                }
                footer
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 12)
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("Countdown")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
            LiquidSegmented(
                options: [(0, String(localized: "Off")), (3, "3s"), (5, "5s")],
                selection: Binding(
                    get: { model.settings.countdown_seconds },
                    set: { value in
                        if !model.state.isBusy { model.update { $0.countdown_seconds = value } }
                    }
                )
            )
            Spacer(minLength: 4)
            Text("\(model.settings.video.fps) fps · GIF \(model.settings.gif.fps)")
                .font(.system(size: 10, weight: .medium).monospacedDigit())
                .foregroundStyle(.secondary)
                .padding(.horizontal, 8)
                .frame(height: 20)
                .glassEffect(.regular, in: .capsule)
        }
        // Locked while busy without `.disabled`, for the same reason as the
        // mode cards: the key-view loop keeps the same members throughout.
        .allowsHitTesting(!model.state.isBusy)
        .opacity(model.state.isBusy ? 0.6 : 1)
    }

    @ViewBuilder
    private var footer: some View {
        if let error = model.error {
            HStack(spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text(error)
                    .font(.system(size: 11))
                    .lineLimit(2)
                    .truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .help(error)
                Button("Dismiss") { model.error = nil }
                    .buttonStyle(.glass)
                    .controlSize(.small)
            }
            .padding(.horizontal, 10)
            .frame(minHeight: 30)
            .glassEffect(.regular.tint(.orange.opacity(0.18)), in: .rect(cornerRadius: 12))
        } else {
            // One chip whose content changes, never two swapped in and out.
            // Swapping focusable buttons under a transition, while a capture
            // starts, sent SwiftUI's keyboard-focus bookkeeping into a loop
            // that never settled: the app froze on the next recording.
            HStack(spacing: 6) {
                let saved = model.lastSaved
                Chip(symbol: saved == nil ? "folder" : "checkmark.circle.fill",
                     text: saved?.lastPathComponent ?? String(localized: "Captures"), prominent: saved != nil) {
                    if let saved { NSWorkspace.shared.open(saved) } else { model.openOutput() }
                }
                .help(saved.map { String(localized: "Open \($0.path)") } ?? homeRelative(model.paths.capture_root))
                Spacer(minLength: 4)
                StatusWell(model: model)
            }
            .frame(height: 26)
        }
    }
}

private struct ModeCard: View {
    let model: AppModel
    let kind: CaptureKind
    let symbol: String
    let title: String
    let shortcut: HotkeyCommand

    @Local private var hovering = false

    private var active: Bool { model.state.kind == kind }
    private var recording: Bool {
        model.state == .recording(kind) || model.state == .paused(kind)
    }
    private var dimmed: Bool { model.state.isBusy && !active }

    var body: some View {
        Button { if !dimmed { model.trigger(kind) } } label: {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Image(systemName: recording ? "stop.fill" : symbol)
                        .contentTransition(.symbolEffect(.replace))
                        .symbolEffect(.bounce, value: hovering)
                        .font(.system(size: 15, weight: .medium))
                        .foregroundStyle(active ? Color.accentColor : .primary)
                        .frame(width: 20, height: 20, alignment: .leading)
                    Spacer()
                    Text(Hotkeys.shared.label(shortcut))
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .frame(height: 17)
                        .background(.white.opacity(0.08), in: .capsule)
                        .strikethrough(Hotkeys.shared.taken.contains(shortcut))
                        .help(Hotkeys.shared.taken.contains(shortcut)
                              ? String(localized: "Another app uses this shortcut") : String(localized: "Shortcut"))
                }
                Spacer(minLength: 4)
                Text(label)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .contentTransition(.numericText())
            }
            .padding(10)
            .frame(maxWidth: .infinity, minHeight: 70, maxHeight: 70, alignment: .leading)
            .contentShape(.rect(cornerRadius: 12))
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
        .glassEffect(active ? .regular.tint(.accentColor.opacity(0.28)).interactive() : .regular.interactive(),
                     in: .rect(cornerRadius: 12))
        .scaleEffect(hovering && !dimmed ? 1.025 : 1)
        .shadow(color: .black.opacity(hovering && !dimmed ? 0.18 : 0), radius: 10, y: 4)
        .opacity(dimmed ? 0.45 : 1)
        // Not `.disabled`: that takes the card out of the key-view loop and
        // puts it back on every capture, which is the churn that loop chokes on.
        .allowsHitTesting(!dimmed)
        .onHover { inside in
            withAnimation(.spring(response: 0.3, dampingFraction: 0.65)) { hovering = inside }
        }
        .animation(.spring(response: 0.4, dampingFraction: 0.75), value: model.state)
        .accessibilityLabel(label)
    }

    private var label: String {
        switch model.state {
        case .selecting(kind):
            kind == .window ? String(localized: "Click a window…") : String(localized: "Drag a region…")
        case let .countdown(current, seconds) where current == kind:
            seconds > 0 ? String(localized: "Starts in \(seconds)") : String(localized: "Starting…")
        case .recording(kind): String(localized: "Stop")
        case .paused(kind): String(localized: "Paused · Stop")
        case .finalizing(kind): String(localized: "Saving…")
        default: title
        }
    }
}

struct Chip: View {
    let symbol: String
    let text: String
    let prominent: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: symbol)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(prominent ? Color.accentColor : .secondary)
                Text(text)
                    .font(.system(size: 11, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 110)
                    .fixedSize()
            }
            .padding(.horizontal, 10)
            .frame(height: 26)
            .contentShape(.capsule)
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
        .glassEffect(prominent ? .regular.tint(.accentColor.opacity(0.22)).interactive() : .regular.interactive(),
                     in: .capsule)
    }
}

private struct StatusWell: View {
    let model: AppModel

    var body: some View {
        // Ticks only while it shows a running clock; idle it does not redraw.
        TimelineView(.animation(minimumInterval: 0.5, paused: !ticking)) { context in
            HStack(spacing: 7) {
                if case .recording = model.state {
                    RecordingDot(active: true, now: context.date)
                } else {
                    Circle()
                        .fill(dot)
                        .frame(width: 6, height: 6)
                }
                Text(text(at: context.date))
                    .font(.system(size: 11, weight: .medium).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 4)
            .frame(height: 26)
        }
    }

    private var ticking: Bool {
        switch model.state {
        case .recording, .paused: true
        default: false
        }
    }

    private var dot: Color {
        switch model.state {
        case .idle: model.lastSaved == nil ? .green.opacity(0.8) : .accentColor
        case .paused: .orange
        default: .secondary
        }
    }

    private func text(at now: Date) -> String {
        switch model.state {
        case .idle: model.lastSaved == nil ? String(localized: "Ready") : String(localized: "Saved")
        case .selecting: String(localized: "Esc to cancel")
        case let .countdown(_, seconds): seconds > 0 ? String(localized: "In \(seconds)s") : String(localized: "Starting")
        case .recording: clock(model.elapsed(at: now))
        case .paused: String(localized: "Paused \(clock(model.elapsed(at: now)))")
        case .finalizing: String(localized: "Saving…")
        }
    }
}
