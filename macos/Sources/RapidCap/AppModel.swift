// The capture state machine and everything the views read.
//
// The same states as `rapidcap_capture::CaptureState`, owned here rather than
// in Rust: on macOS the core is a library of blocking calls, and sequencing
// them - overlay, countdown, record, finalize - is UI work.

import AppKit
import Observation
import SwiftUI

enum CaptureKind: String {
    case region, window, video, gif

    var noun: String {
        switch self {
        case .region: "Region"
        case .window: "Window"
        case .video: "Video"
        case .gif: "GIF"
        }
    }

    var isRecording: Bool { self == .video || self == .gif }
}

enum CaptureState: Equatable {
    case idle
    case selecting(CaptureKind)
    case countdown(CaptureKind, Int)
    case recording(CaptureKind)
    case paused(CaptureKind)
    case finalizing(CaptureKind)

    var kind: CaptureKind? {
        switch self {
        case .idle: nil
        case let .selecting(kind), let .countdown(kind, _), let .recording(kind),
             let .paused(kind), let .finalizing(kind): kind
        }
    }

    var isBusy: Bool { self != .idle }
}

enum Screen: Hashable {
    case panel, settings, gallery
}

/// Each window has one fixed size; nothing resizes to its content.
enum PanelSize {
    static func of(_ screen: Screen) -> CGSize {
        switch screen {
        case .panel: CGSize(width: 372, height: 256)
        case .settings: CGSize(width: 440, height: 540)
        case .gallery: CGSize(width: 540, height: 460)
        }
    }
}

@MainActor
@Observable
final class AppModel {
    var settings: Settings
    let paths: Paths
    let version: String
    var state: CaptureState = .idle {
        // Out of screenshots and recordings only while a capture runs, so
        // ordinary screenshot tools can still photograph the panel.
        didSet {
            panel?.sharingType = state.isBusy ? .none : .readOnly
            if state != oldValue { onStateChange?(state) }
        }
    }
    /// Shows a window; set by the app delegate, which owns them.
    @ObservationIgnored var onOpen: ((Screen) -> Void)?
    /// Lets the menu bar icon follow the capture state.
    @ObservationIgnored var onStateChange: ((CaptureState) -> Void)?
    var error: String?
    var lastSaved: URL?
    var recordingStarted: Date?
    var pausedTotal: TimeInterval = 0
    private var pausedAt: Date?
    /// Bumped whenever the capture folder may have changed - a capture landed,
    /// or Recent captures was opened - so the gallery lists it again.
    var capturesVersion = 0

    @ObservationIgnored private var recording: RecordingHandle?
    @ObservationIgnored private var countdownTask: Task<Void, Never>?
    @ObservationIgnored private var savedClearTask: Task<Void, Never>?
    @ObservationIgnored private var hidPanel = false
    /// Bumped by every recording start and every cancel. A start that comes
    /// back from FFmpeg checks it, so a slow start from a cancelled session
    /// is never mistaken for the countdown of a newer one.
    @ObservationIgnored private var session = 0
    @ObservationIgnored private var audio: SystemAudio?
    /// Background work Quit has to wait for: an FFmpeg start or a save.
    @ObservationIgnored private var unsettled = 0
    @ObservationIgnored private var toggling = false
    @ObservationIgnored private var onSettled: (() -> Void)?
    @ObservationIgnored let overlay = OverlayController()
    @ObservationIgnored let hud = HudController()
    @ObservationIgnored let frame = FrameController()
    @ObservationIgnored weak var panel: NSWindow?

    init() {
        do {
            (settings, paths, version) = try Core.load()
        } catch {
            settings = Settings.fallback
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            paths = Paths(
                capture_root: home + "/Documents/RapidCap/Screenshots",
                settings_file: home + "/Library/Application Support/RapidCap/settings.json",
                temp_dir: home + "/Library/Caches/RapidCap/Temp",
                thumb_dir: home + "/Library/Caches/RapidCap/Thumbnails"
            )
            version = "?"
            self.error = "Settings could not be read: \(error.localizedDescription)"
        }
    }

    // MARK: windows

    /// Settings and Recent captures are windows of their own, beside the
    /// panel. One window with screens inside it lost clicks on its Back
    /// button, so there is no Back button any more.
    func open(_ screen: Screen) { onOpen?(screen) }

    // MARK: settings

    func update(_ change: (inout Settings) -> Void) {
        var next = settings
        change(&next)
        guard next != settings else { return }
        settings = next
        do {
            try Core.save(next)
        } catch {
            self.error = "Settings were not saved: \(error.localizedDescription)"
        }
    }

    // MARK: commands

    func trigger(_ kind: CaptureKind) {
        switch state {
        case .idle:
            begin(kind)
        case let .selecting(current):
            overlay.close()
            state = .idle
            if current != kind { begin(kind) } else { restorePanel() }
        case let .recording(current), let .paused(current):
            if current == kind { stop() }
        case let .countdown(current, _):
            if current == kind { cancel() }
        case .finalizing:
            break
        }
    }

    func togglePause() {
        // One pause or resume at a time: a second press while FFmpeg is still
        // closing or opening a segment would ask it to do the same twice.
        guard let recording, !toggling else { return }
        let mine = session
        switch state {
        case let .recording(kind):
            toggling = true
            perform { try recording.pause() } then: { [weak self] ok in
                guard let self else { return }
                toggling = false
                // Failed, or stopped, cancelled or replaced while FFmpeg paused.
                guard ok, mine == session, state == .recording(kind) else { return }
                audio?.pause()
                pausedAt = Date()
                state = .paused(kind)
            }
        case let .paused(kind):
            toggling = true
            perform { try recording.resume() } then: { [weak self] ok in
                guard let self else { return }
                toggling = false
                guard ok, mine == session, state == .paused(kind) else { return }
                audio?.resume()
                if let pausedAt { pausedTotal += Date().timeIntervalSince(pausedAt) }
                pausedAt = nil
                state = .recording(kind)
            }
        default:
            break
        }
    }

    func cancel() {
        session += 1
        audio?.cancel()
        audio = nil
        switch state {
        case .selecting:
            overlay.close()
            state = .idle
            restorePanel()
        case .countdown:
            countdownTask?.cancel()
            hud.close()
            frame.close()
            state = .idle
            restorePanel()
        case .recording, .paused:
            let handle = recording
            recording = nil
            Task.detached { handle?.cancel() }
            hud.close()
            frame.close()
            state = .idle
            restorePanel()
        default:
            break
        }
    }

    /// For Quit. Stops a recording on the spot, and answers whether the app
    /// can go now; if not, `then` runs once a start or a save in flight has
    /// finished, so neither leaves FFmpeg or a half-written file behind.
    func prepareToQuit(then: @escaping () -> Void) -> Bool {
        if let handle = recording {
            recording = nil
            handle.cancel()
        }
        cancel()
        guard unsettled > 0 else { return true }
        onSettled = then
        return false
    }

    private func settle() {
        unsettled -= 1
        if unsettled == 0, let onSettled {
            self.onSettled = nil
            onSettled()
        }
    }

    /// `--e2e` only: the recording flow from the countdown on, for a region
    /// given here instead of picked in the overlay.
    func record(_ kind: CaptureKind, at points: CGRect) {
        error = nil
        state = .selecting(kind)
        startCountdown(kind, Pick(points: points, processName: nil))
    }

    func elapsed(at now: Date) -> TimeInterval {
        guard let recordingStarted else { return 0 }
        let paused = pausedTotal + (pausedAt.map { now.timeIntervalSince($0) } ?? 0)
        return max(0, now.timeIntervalSince(recordingStarted) - paused)
    }

    // MARK: flow

    private func begin(_ kind: CaptureKind) {
        error = nil
        guard CGPreflightScreenCaptureAccess() else {
            CGRequestScreenCaptureAccess()
            error = "Allow RapidCap in System Settings › Privacy & Security › Screen Recording."
            return
        }
        state = .selecting(kind)
        if settings.hide_panel_while_capturing, let panel, panel.isVisible {
            panel.orderOut(nil)
            hidPanel = true
        }
        overlay.open(pickWindow: kind == .window) { [weak self] pick in
            guard let self else { return }
            guard let pick else {
                self.state = .idle
                self.restorePanel()
                return
            }
            if kind.isRecording {
                self.startCountdown(kind, pick)
            } else {
                self.shoot(kind, pick)
            }
        }
    }

    private func shoot(_ kind: CaptureKind, _ pick: Pick) {
        state = .finalizing(kind)
        Task { [weak self] in
            // The overlay has to be gone from the window server before the
            // frame is read, or its dim lands in the screenshot.
            try? await Task.sleep(for: .milliseconds(120))
            let result = await offMain { Result { try Core.screenshot(pick.pixels, processName: pick.processName) } }
            self?.finish(result)
        }
    }

    private func startCountdown(_ kind: CaptureKind, _ pick: Pick) {
        frame.show(around: pick.points, model: self)
        hud.show(model: self, near: pick.points)
        let seconds = settings.countdown_seconds
        countdownTask = Task { [weak self] in
            for remaining in stride(from: seconds, to: 0, by: -1) {
                self?.state = .countdown(kind, remaining)
                try? await Task.sleep(for: .seconds(1))
                if Task.isCancelled { return }
            }
            self?.startRecording(kind, pick)
        }
    }

    private func startRecording(_ kind: CaptureKind, _ pick: Pick) {
        state = .countdown(kind, 0)
        session += 1
        let mine = session
        unsettled += 1
        if kind == .video && settings.audio.enabled { attachAudio(mine) }
        Task { [weak self] in
            let result = await offMain { Result { try RecordingHandle(gif: kind == .gif, rect: pick.pixels) } }
            guard let self else { return }
            defer { self.settle() }
            // Cancelled while FFmpeg was starting, and maybe a new capture
            // begun since: this result belongs to nobody now.
            guard mine == self.session, case .countdown = self.state else {
                if case let .success(handle) = result { await offMain { handle.cancel() } }
                return
            }
            switch result {
            case let .success(handle):
                self.recording = handle
                self.recordingStarted = Date()
                self.pausedTotal = 0
                self.pausedAt = nil
                self.state = .recording(kind)
            case let .failure(error):
                self.audio?.cancel()
                self.audio = nil
                self.hud.close()
                self.frame.close()
                self.state = .idle
                self.error = error.localizedDescription
                self.restorePanel()
            }
        }
    }

    /// System audio, started beside FFmpeg but never waited for: if the
    /// system is slow to hand over sound - a permission prompt, say - the
    /// recording goes ahead and the track joins when it comes. The core lines
    /// the two up by their ends, so a late start only means silence first.
    private func attachAudio(_ mine: Int) {
        let settings = settings.audio
        Task { [weak self] in
            let result: Result<SystemAudio, Error>
            do {
                result = .success(try await SystemAudio.start(bitrate: settings.bitrate, channels: settings.channels))
            } catch {
                result = .failure(error)
            }
            guard let self else { return }
            let live: Bool = switch self.state {
            case .countdown, .recording, .paused: mine == self.session
            default: false
            }
            switch result {
            case let .success(recorder) where live:
                if case .paused = self.state { recorder.pause() }
                self.audio = recorder
            case let .success(recorder):
                recorder.cancel()
            case let .failure(problem) where live:
                self.error = "Recording without system audio: \(problem.localizedDescription)"
            case .failure:
                break
            }
        }
    }

    private func stop() {
        guard let handle = recording, let kind = state.kind else { return }
        recording = nil
        let sound = audio
        audio = nil
        state = .finalizing(kind)
        frame.close()
        unsettled += 1
        // Sound and picture stop at the same moment; closing the files can
        // then take as long as each needs.
        sound?.end()
        Task { [weak self] in
            async let track = sound?.finish()
            var result = await offMain { Result { try handle.stop() } }
            switch (result, await track) {
            case let (.success(saved), .success(track)?):
                let mixed = await offMain { Result { try Core.muxAudio(video: saved.path, audio: track.url,
                                                                       seconds: track.seconds) } }
                if case let .failure(problem) = mixed {
                    try? FileManager.default.removeItem(at: track.url)
                    result = .success(Core.Saved(path: saved.path,
                                                 note: "saved without sound: \(problem.localizedDescription)"))
                }
            case let (.success(saved), .failure(problem)?):
                result = .success(Core.Saved(path: saved.path,
                                             note: "saved without sound: \(problem.localizedDescription)"))
            case let (.failure, .success(track)?):
                try? FileManager.default.removeItem(at: track.url)
            default:
                break
            }
            self?.finish(result)
            self?.settle()
        }
    }

    private func finish(_ result: Result<Core.Saved, Error>) {
        hud.close()
        frame.close()
        state = .idle
        recordingStarted = nil
        switch result {
        case let .success(saved):
            lastSaved = saved.path
            capturesVersion += 1
            if let note = saved.note { error = "Saved, but \(note)" }
            savedClearTask?.cancel()
            savedClearTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(8))
                if !Task.isCancelled { self?.lastSaved = nil }
            }
        case let .failure(failure):
            error = failure.localizedDescription
        }
        restorePanel()
    }

    /// Blocking core work off the main thread; `done` runs either way, told
    /// whether it worked, so whoever waits on it is always released.
    private func perform(_ work: @escaping @Sendable () throws -> Void, then done: @escaping (Bool) -> Void) {
        Task { [weak self] in
            let result = await offMain { Result { try work() } }
            if case let .failure(error) = result { self?.error = error.localizedDescription }
            if case .success = result { done(true) } else { done(false) }
        }
    }

    /// Bring back the panel only if the capture hid it. A capture started from
    /// a hotkey or the menu bar with the panel closed leaves it closed.
    private func restorePanel() {
        guard hidPanel else { return }
        hidPanel = false
        panel?.orderFront(nil)
    }

    // MARK: folders

    var captureRoot: URL { URL(fileURLWithPath: paths.capture_root) }

    func openOutput() {
        try? FileManager.default.createDirectory(at: captureRoot, withIntermediateDirectories: true)
        NSWorkspace.shared.open(captureRoot)
    }

    func reveal(_ url: URL) {
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }
}

extension Settings {
    static let fallback = Settings(
        schema_version: 1,
        screenshot: .init(png_to_jpeg_threshold_bytes: 2_097_152, jpeg_quality: 90, copy_to_clipboard: true),
        video: .init(fps: 30, bitrate: 3_000_000, preset: "p7", tune: "hq"),
        audio: .init(enabled: true, microphone: nil, bitrate: 128_000, channels: 2),
        gif: .init(fps: 15, palette_stats_mode: "full", dither: "sierra2_4a"),
        countdown_seconds: 5,
        hide_panel_while_capturing: true
    )
}

/// `~/Documents/RapidCap/Screenshots`, the way a Mac user reads a path.
func homeRelative(_ path: String) -> String {
    let home = FileManager.default.homeDirectoryForCurrentUser.path
    return path.hasPrefix(home) ? "~" + path.dropFirst(home.count) : path
}

/// Run blocking core work off the main thread and come back with its result.
func offMain<T: Sendable>(_ work: @escaping @Sendable () -> T) async -> T {
    await Task.detached(priority: .userInitiated) { work() }.value
}
