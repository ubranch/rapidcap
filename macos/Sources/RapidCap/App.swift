// App entry: the glass panel window, the menu bar item and the hotkeys.

import AppKit
import SwiftUI

@main
struct Main {
    static func main() {
        if CommandLine.arguments.contains("--probe") {
            probe()
            return
        }
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.regular)
        app.run()
    }

    /// The packaging smoke test: load the core and print what it found.
    @MainActor
    static func probe() {
        do {
            let (_, paths, version) = try Core.load()
            let payload: [String: Any] = [
                "app_id": "com.inspire.rapidcap",
                "version": version,
                "output": paths.capture_root,
                "ui": "swiftui",
                "hotkeys": HotkeyCommand.allCases.map { Hotkeys.shared.label($0) },
            ]
            let data = try JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
            print(String(decoding: data, as: UTF8.self))
        } catch {
            FileHandle.standardError.write(Data("probe failed: \(error.localizedDescription)\n".utf8))
            exit(1)
        }
    }
}

/// Where the traffic lights sit, measured off the real buttons so the title
/// row lines up with them on whatever macOS draws them at.
struct TitlebarMetrics: Equatable {
    var centerFromTop: CGFloat = 16
    var leading: CGFloat = 78
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var model: AppModel!
    private var window: NSWindow!
    private var statusItem: NSStatusItem!
    private let metrics = MetricsBox()

    func applicationDidFinishLaunching(_ notification: Notification) {
        guard ensureSingleInstance() else {
            NSApp.terminate(nil)
            return
        }
        model = AppModel()
        buildMenu()
        buildWindow()
        buildStatusItem()
        Hotkeys.shared.start { [weak self] command in self?.handle(command) }
        model.onOpen = { [weak self] screen in self?.open(screen) }
        show()
        if SelfTest.requested { SelfTest.run() }
        if EndToEnd.requested { EndToEnd.run(model) }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        show()
        return true
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        // A second copy quits before it has a model at all.
        guard model != nil else { return .terminateNow }
        let ready = model.prepareToQuit { NSApp.reply(toApplicationShouldTerminate: true) }
        return ready ? .terminateNow : .terminateLater
    }

    // MARK: windows

    private func buildWindow() {
        let window = makeWindow(.panel)
        window.center()
        self.window = window
        model.panel = window
        measureTitlebar()
    }

    /// The panel, Settings or Recent captures. Each is built once, on first
    /// use, and closing one only hides it.
    private func makeWindow(_ screen: Screen) -> NSWindow {
        let title = switch screen {
        case .panel: "RapidCap"
        case .settings: "Settings"
        case .gallery: "Recent captures"
        }
        let window = GlassWindow.make(title: title, size: PanelSize.of(screen),
                                      root: WindowRoot(screen: screen, model: model, metrics: metrics))
        window.delegate = self
        return window
    }

    private var others: [Screen: NSWindow] = [:]

    private func open(_ screen: Screen) {
        guard screen != .panel else { return show() }
        let window = others[screen] ?? {
            let made = makeWindow(screen)
            // Beside the panel when there is room, centred otherwise.
            if let panel = self.window, let visible = panel.screen?.visibleFrame {
                var origin = CGPoint(x: panel.frame.maxX + 12, y: panel.frame.maxY - made.frame.height)
                if origin.x + made.frame.width > visible.maxX { origin.x = panel.frame.minX - 12 - made.frame.width }
                origin.y = max(origin.y, visible.minY)
                if origin.x >= visible.minX { made.setFrameOrigin(origin) } else { made.center() }
            } else {
                made.center()
            }
            others[screen] = made
            return made
        }()
        if screen == .gallery { model.capturesVersion += 1 }
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    private func measureTitlebar() {
        guard let close = window.standardWindowButton(.closeButton),
              let zoom = window.standardWindowButton(.zoomButton),
              let content = window.contentView
        else { return }
        let closeFrame = close.convert(close.bounds, to: content)
        let zoomFrame = zoom.convert(zoom.bounds, to: content)
        let fromTop = content.isFlipped ? closeFrame.midY : content.bounds.height - closeFrame.midY
        metrics.value = TitlebarMetrics(centerFromTop: fromTop, leading: zoomFrame.maxX + 12)
    }


    func windowShouldClose(_ sender: NSWindow) -> Bool {
        // Every window hides rather than closes; the app lives in the menu bar.
        // Hiding does not end SwiftUI views, so a shortcut recorder left
        // listening would keep swallowing every key press.
        Hotkeys.shared.endRecording()
        sender.orderOut(nil)
        return false
    }

    func windowShouldZoom(_ window: NSWindow, toFrame newFrame: NSRect) -> Bool { false }

    private func show() {
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    // MARK: commands

    private func handle(_ command: HotkeyCommand) {
        switch command {
        case .region: model.trigger(.region)
        case .window: model.trigger(.window)
        case .video: model.trigger(.video)
        case .gif: model.trigger(.gif)
        case .pause: model.togglePause()
        }
    }

    private func buildMenu() {
        let main = NSMenu()
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "About RapidCap", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
                        keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Settings…", action: #selector(openSettings), keyEquivalent: ",").target = self
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Hide RapidCap", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(withTitle: "Quit RapidCap", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        main.addItem(appItem)

        let windowItem = NSMenuItem()
        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        windowMenu.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)),
                           keyEquivalent: "m")
        windowItem.submenu = windowMenu
        main.addItem(windowItem)
        NSApp.mainMenu = main
    }

    /// Settings is its own window, the way a Mac app's is: taller than the
    /// panel, fixed in size, and reachable with ⌘, from anywhere in the app.
    @objc private func openSettings() { open(.settings) }

    // MARK: status item

    private func buildStatusItem() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = StatusMark.image(recording: false)
        model.onStateChange = { [weak self] state in
            let recording = state.kind?.isRecording == true && state != .idle
            self?.statusItem.button?.image = StatusMark.image(recording: recording)
        }
        let menu = NSMenu()
        menu.delegate = self
        statusItem.menu = menu
    }

    @objc private func menuRegion() { model.trigger(.region) }
    @objc private func menuWindow() { model.trigger(.window) }
    @objc private func menuVideo() { model.trigger(.video) }
    @objc private func menuGif() { model.trigger(.gif) }
    @objc private func menuShow() { show() }
    @objc private func menuGallery() { open(.gallery) }
    @objc private func menuFolder() { model.openOutput() }
}

extension AppDelegate: NSMenuDelegate {
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        func item(_ title: String, _ action: Selector, _ symbol: String, _ key: HotkeyCommand? = nil) {
            let entry = NSMenuItem(title: title, action: action, keyEquivalent: "")
            entry.target = self
            entry.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)?
                .withSymbolConfiguration(.init(pointSize: 13, weight: .regular))
            // The global shortcut, printed on the right like any menu's.
            if let key, let shortcut = Hotkeys.shared.chords[key]?.menuEquivalent {
                entry.keyEquivalent = shortcut.key
                entry.keyEquivalentModifierMask = shortcut.modifiers
            }
            menu.addItem(entry)
        }
        let recordingVideo = model.state == .recording(.video) || model.state == .paused(.video)
        let recordingGif = model.state == .recording(.gif) || model.state == .paused(.gif)
        item("Capture region", #selector(menuRegion), "rectangle.dashed", .region)
        item("Capture window", #selector(menuWindow), "macwindow", .window)
        item(recordingVideo ? "Stop recording" : "Record video", #selector(menuVideo),
             recordingVideo ? "stop.fill" : "video", .video)
        item(recordingGif ? "Stop GIF" : "Record GIF", #selector(menuGif),
             recordingGif ? "stop.fill" : "photo.stack", .gif)
        menu.addItem(.separator())
        item("Show RapidCap", #selector(menuShow), "rectangle.on.rectangle")
        item("Recent captures", #selector(menuGallery), "square.grid.2x2")
        item("Open captures folder", #selector(menuFolder), "folder")
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit RapidCap", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        quit.image = NSImage(systemSymbolName: "power", accessibilityDescription: "Quit")?
            .withSymbolConfiguration(.init(pointSize: 13, weight: .regular))
        menu.addItem(quit)
    }
}

/// Holds the titlebar measurement where both AppKit and SwiftUI can reach it.
@MainActor
@Observable
final class MetricsBox {
    var value = TitlebarMetrics()
}

/// One instance per user: a second launch just shows the first one's panel.
@MainActor
private func ensureSingleInstance() -> Bool {
    let others = NSRunningApplication.runningApplications(withBundleIdentifier: "com.inspire.rapidcap")
        .filter { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }
    if let first = others.first, let url = first.bundleURL {
        // Opening the running app sends it a reopen event, which shows its
        // panel; activating alone would leave a closed panel closed.
        NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
        return false
    }
    return true
}

/// A fixed-size window that is glass all the way to its rounded edge.
///
/// The glass sits *behind* the SwiftUI content as a sibling rather than
/// hosting it as `NSGlassEffectView.contentView`. A click once froze the app
/// inside SwiftUI's key-view loop walk; keeping the hosting view a plain
/// subview keeps its responder chain the ordinary one.
@MainActor
enum GlassWindow {
    static func make(title: String, size: CGSize, root: some View) -> NSWindow {
        let window = NSWindow(
            contentRect: CGRect(origin: .zero, size: size),
            styleMask: [.titled, .closable, .miniaturizable, .fullSizeContentView],
            backing: .buffered, defer: false
        )
        window.title = title
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = true
        window.isOpaque = false
        window.backgroundColor = .clear
        window.isReleasedWhenClosed = false
        window.standardWindowButton(.zoomButton)?.isEnabled = false
        window.collectionBehavior = [.fullScreenAuxiliary]

        let container = NSView(frame: CGRect(origin: .zero, size: size))
        let glass = NSGlassEffectView(frame: container.bounds)
        glass.autoresizingMask = [.width, .height]
        // The glass has to round exactly where the window frame does. A
        // larger radius left the frame's own dark corner showing outside it.
        // A private key: asked only when the window answers to it, because
        // `value(forKey:)` on a key a class lacks raises and kills the app.
        let key = "_cornerRadius"
        let radius = window.responds(to: NSSelectorFromString(key))
            ? (window.value(forKey: key) as? NSNumber).map { CGFloat($0.doubleValue) } : nil
        glass.cornerRadius = radius ?? 16
        let host = NSHostingView(rootView: root)
        host.sizingOptions = []
        host.frame = container.bounds
        host.autoresizingMask = [.width, .height]
        container.addSubview(glass)
        container.addSubview(host)
        window.contentView = container

        // Fixed at one size. Min and max pin it against everything that
        // resizes windows from outside the app: a double-click on the title
        // bar, Window > Fill, tiling. The delegate refuses zoom as well.
        window.setContentSize(size)
        window.contentMinSize = size
        window.contentMaxSize = size
        return window
    }
}
