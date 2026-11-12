// Global hotkeys through Carbon's RegisterEventHotKey: the one system-wide
// shortcut API that needs no Accessibility grant.
//
// The defaults are the macOS column of `hotkey_specs` in the Rust app. Any
// chord can be rebound in Settings; the choice lives in UserDefaults, not in
// settings.json, because that file is shared with Windows, whose loader
// rejects keys it does not know.

import AppKit
import Carbon.HIToolbox
import Observation

enum HotkeyCommand: UInt32, CaseIterable {
    case region = 1, window, video, gif, pause

    var name: String {
        switch self {
        case .region: String(localized: "Capture region")
        case .window: String(localized: "Capture window")
        case .video: String(localized: "Record video")
        case .gif: String(localized: "Record GIF")
        case .pause: String(localized: "Pause · resume")
        }
    }

    var defaultChord: Chord {
        switch self {
        case .region: Chord(keyCode: UInt32(kVK_ANSI_E), modifiers: UInt32(optionKey), key: "E")
        case .window: Chord(keyCode: UInt32(kVK_ANSI_W), modifiers: UInt32(cmdKey | shiftKey), key: "W")
        case .video: Chord(keyCode: UInt32(kVK_ANSI_Q), modifiers: UInt32(optionKey), key: "Q")
        case .gif: Chord(keyCode: UInt32(kVK_ANSI_G), modifiers: UInt32(cmdKey | shiftKey), key: "G")
        case .pause: Chord(keyCode: UInt32(kVK_ANSI_P), modifiers: UInt32(cmdKey | shiftKey), key: "P")
        }
    }
}

/// A key plus Carbon modifier flags, and the key's printed name.
struct Chord: Codable, Equatable {
    var keyCode: UInt32
    var modifiers: UInt32
    var key: String

    /// `⌃⌥⇧⌘K`, in Apple's modifier order.
    var label: String {
        var text = ""
        if modifiers & UInt32(controlKey) != 0 { text += "⌃" }
        if modifiers & UInt32(optionKey) != 0 { text += "⌥" }
        if modifiers & UInt32(shiftKey) != 0 { text += "⇧" }
        if modifiers & UInt32(cmdKey) != 0 { text += "⌘" }
        return text + key
    }

    /// The same chord as a menu key equivalent, so menus print it the way
    /// macOS prints every other shortcut. Nil for keys a menu cannot show.
    var menuEquivalent: (key: String, modifiers: NSEvent.ModifierFlags)? {
        var flags: NSEvent.ModifierFlags = []
        if modifiers & UInt32(controlKey) != 0 { flags.insert(.control) }
        if modifiers & UInt32(optionKey) != 0 { flags.insert(.option) }
        if modifiers & UInt32(shiftKey) != 0 { flags.insert(.shift) }
        if modifiers & UInt32(cmdKey) != 0 { flags.insert(.command) }
        if key.count > 1, key.hasPrefix("F"), let n = Int(key.dropFirst()), (1...20).contains(n),
           let scalar = UnicodeScalar(NSF1FunctionKey + n - 1) {
            return (String(Character(scalar)), flags)
        }
        guard key.count == 1 else { return nil }
        return (key.lowercased(), flags)
    }

    /// From a key press in the recorder. Nil when it cannot be a global
    /// shortcut: a bare letter would fire every time you type it.
    init?(event: NSEvent) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        var carbon: UInt32 = 0
        if flags.contains(.control) { carbon |= UInt32(controlKey) }
        if flags.contains(.option) { carbon |= UInt32(optionKey) }
        if flags.contains(.shift) { carbon |= UInt32(shiftKey) }
        if flags.contains(.command) { carbon |= UInt32(cmdKey) }
        let code = UInt32(event.keyCode)
        let name = Chord.keyName(code, event)
        let isFunctionKey = Chord.functionKeys[code] != nil
        // Shift alone is still typing.
        let hasRealModifier = carbon & UInt32(controlKey | optionKey | cmdKey) != 0
        guard isFunctionKey || hasRealModifier, !name.isEmpty else { return nil }
        self.init(keyCode: code, modifiers: carbon, key: name)
    }

    init(keyCode: UInt32, modifiers: UInt32, key: String) {
        self.keyCode = keyCode
        self.modifiers = modifiers
        self.key = key
    }

    private static let functionKeys: [UInt32: String] = [
        UInt32(kVK_F1): "F1", UInt32(kVK_F2): "F2", UInt32(kVK_F3): "F3", UInt32(kVK_F4): "F4",
        UInt32(kVK_F5): "F5", UInt32(kVK_F6): "F6", UInt32(kVK_F7): "F7", UInt32(kVK_F8): "F8",
        UInt32(kVK_F9): "F9", UInt32(kVK_F10): "F10", UInt32(kVK_F11): "F11", UInt32(kVK_F12): "F12",
        UInt32(kVK_F13): "F13", UInt32(kVK_F14): "F14", UInt32(kVK_F15): "F15",
    ]

    private static let namedKeys: [UInt32: String] = [
        UInt32(kVK_Space): "Space", UInt32(kVK_Return): "↩", UInt32(kVK_Tab): "⇥",
        UInt32(kVK_Delete): "⌫", UInt32(kVK_ForwardDelete): "⌦", UInt32(kVK_LeftArrow): "←",
        UInt32(kVK_RightArrow): "→", UInt32(kVK_UpArrow): "↑", UInt32(kVK_DownArrow): "↓",
        UInt32(kVK_Home): "↖", UInt32(kVK_End): "↘", UInt32(kVK_PageUp): "⇞", UInt32(kVK_PageDown): "⇟",
    ]

    private static func keyName(_ code: UInt32, _ event: NSEvent) -> String {
        if let name = functionKeys[code] ?? namedKeys[code] { return name }
        // The unmodified character, so ⌥E reads "E" and not "´".
        return (event.charactersIgnoringModifiers ?? "").uppercased()
    }
}

@MainActor
@Observable
final class Hotkeys {
    private(set) var chords: [HotkeyCommand: Chord] = [:]
    /// Commands whose chord another app already holds.
    private(set) var taken: Set<HotkeyCommand> = []

    @ObservationIgnored private var refs: [HotkeyCommand: EventHotKeyRef] = [:]
    @ObservationIgnored private static var handler: ((HotkeyCommand) -> Void)?
    @ObservationIgnored private var installed = false
    /// The command whose shortcut the Settings recorder is listening for.
    private(set) var recording: HotkeyCommand?
    @ObservationIgnored private var monitor: Any?

    static let shared = Hotkeys()

    private static let defaultsKey = "hotkeys"

    init() {
        let saved = UserDefaults.standard.data(forKey: Self.defaultsKey)
            .flatMap { try? JSONDecoder().decode([String: Chord].self, from: $0) } ?? [:]
        for command in HotkeyCommand.allCases {
            chords[command] = saved[String(command.rawValue)] ?? command.defaultChord
        }
    }

    func label(_ command: HotkeyCommand) -> String { chords[command]?.label ?? "" }

    func start(_ handler: @escaping (HotkeyCommand) -> Void) {
        Self.handler = handler
        if !installed {
            installed = true
            var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
            InstallEventHandler(GetApplicationEventTarget(), { _, event, _ in
                var id = EventHotKeyID()
                GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                                  nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
                if let command = HotkeyCommand(rawValue: id.id) {
                    DispatchQueue.main.async { Hotkeys.handler?(command) }
                }
                return noErr
            }, 1, &spec, nil, nil)
        }
        HotkeyCommand.allCases.forEach(register)
    }

    /// Bind a new chord, or `nil` to go back to the default.
    func set(_ command: HotkeyCommand, to chord: Chord?) {
        chords[command] = chord ?? command.defaultChord
        register(command)
        save()
    }

    /// Listen for a new chord for `command`. One recorder at a time: starting
    /// another ends this one. Every chord is released meanwhile, so pressing
    /// the one already bound records it instead of firing it.
    func record(_ command: HotkeyCommand, onKey: @escaping (NSEvent) -> Void) {
        endRecording()
        recording = command
        refs.values.forEach { UnregisterEventHotKey($0) }
        refs.removeAll()
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            onKey(event)
            return nil
        }
    }

    func endRecording() {
        guard recording != nil else { return }
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        recording = nil
        HotkeyCommand.allCases.forEach(register)
    }

    /// Which other command already uses this chord, if any.
    func owner(of chord: Chord, except command: HotkeyCommand) -> HotkeyCommand? {
        HotkeyCommand.allCases.first { $0 != command && chords[$0] == chord }
    }

    private func register(_ command: HotkeyCommand) {
        if let ref = refs.removeValue(forKey: command) { UnregisterEventHotKey(ref) }
        guard let chord = chords[command] else { return }
        var ref: EventHotKeyRef?
        let status = RegisterEventHotKey(chord.keyCode, chord.modifiers,
                                         EventHotKeyID(signature: OSType(0x5243_4150), id: command.rawValue),
                                         GetApplicationEventTarget(), 0, &ref)
        if status == noErr, let ref {
            refs[command] = ref
            taken.remove(command)
        } else {
            taken.insert(command)
        }
    }

    private func save() {
        var stored: [String: Chord] = [:]
        for (command, chord) in chords where chord != command.defaultChord {
            stored[String(command.rawValue)] = chord
        }
        UserDefaults.standard.set(try? JSONEncoder().encode(stored), forKey: Self.defaultsKey)
    }
}
