// Settings: presets first, everyday switches, rebindable shortcuts, and the
// encoder knobs folded away under Advanced. Writes the same settings.json the
// Windows app does; the choices are its tables, so a file is valid on both.

import SwiftUI

// MARK: presets

/// A whole-app quality profile: one tap sets every knob the Advanced section
/// exposes. Detected back from the values, so hand-tuning shows as Custom.
private struct Preset: Identifiable {
    let id: String
    let name: String
    let tagline: String
    let symbol: String
    let apply: (inout Settings) -> Void

    func matches(_ settings: Settings) -> Bool {
        var copy = settings
        apply(&copy)
        return copy == settings
    }
}

private let presets: [Preset] = [
    Preset(id: "studio", name: "Studio", tagline: "Sharpest frames, smoothest motion",
           symbol: "sparkles") {
        $0.video.fps = 60
        $0.video.bitrate = 6_000_000
        $0.video.preset = "p7"
        $0.video.tune = "hq"
        $0.gif.fps = 20
        $0.gif.dither = "sierra2_4a"
        $0.gif.palette_stats_mode = "full"
        $0.audio.bitrate = 192_000
        $0.screenshot.png_to_jpeg_threshold_bytes = 8 << 20
    },
    Preset(id: "everyday", name: "Everyday", tagline: "Crisp enough, light enough",
           symbol: "dial.medium") {
        $0.video.fps = 30
        $0.video.bitrate = 3_000_000
        $0.video.preset = "p7"
        $0.video.tune = "hq"
        $0.gif.fps = 15
        $0.gif.dither = "sierra2_4a"
        $0.gif.palette_stats_mode = "full"
        $0.audio.bitrate = 128_000
        $0.screenshot.png_to_jpeg_threshold_bytes = 2 << 20
    },
    Preset(id: "featherlight", name: "Featherlight", tagline: "Tiny files for chat and email",
           symbol: "leaf") {
        $0.video.fps = 24
        $0.video.bitrate = 1_500_000
        $0.video.preset = "p5"
        $0.video.tune = "hq"
        $0.gif.fps = 10
        $0.gif.dither = "bayer"
        $0.gif.palette_stats_mode = "diff"
        $0.audio.bitrate = 96_000
        $0.screenshot.png_to_jpeg_threshold_bytes = 1 << 20
    },
]

private struct Quality: Hashable {
    let name: String
    let bitrate: Int
    let preset: String
    let tune: String
}

private let qualities = [
    Quality(name: "Standard", bitrate: 1_500_000, preset: "p5", tune: "hq"),
    Quality(name: "High", bitrate: 3_000_000, preset: "p7", tune: "hq"),
    Quality(name: "Maximum", bitrate: 6_000_000, preset: "p7", tune: "hq"),
]

private let dithers: [(id: String, name: String, blurb: String)] = [
    ("sierra2_4a", "Sierra 2-4A", "Smooth gradients at 256 colours"),
    ("sierra2", "Sierra 2", "Wider kernel, a little softer"),
    ("floyd_steinberg", "Floyd-Steinberg", "Classic kernel, a bit grainier"),
    ("heckbert", "Heckbert", "Smallest kernel, coarser grain"),
    ("bayer", "Bayer", "Fixed pattern, smaller files"),
    ("none", "None", "No dithering, gradients band"),
]

// MARK: settings

struct SettingsView: View {
    let model: AppModel
    @Local private var advanced = false

    var body: some View {
        ScrollView {
            GlassEffectContainer(spacing: 12) {
                VStack(spacing: 14) {
                    presetPicker
                    section("Capture") {
                        row("Countdown", "Before a recording starts") {
                            LiquidSegmented(options: [(0, "Off"), (3, "3s"), (5, "5s")],
                                            selection: bind(\.countdown_seconds))
                        }
                        divider
                        row("Hide panel while capturing", "It never lands in a recording either way") {
                            Toggle("", isOn: bind(\.hide_panel_while_capturing)).toggleStyle(.switch)
                        }
                        divider
                        row("Copy screenshots", "Put each screenshot on the clipboard") {
                            Toggle("", isOn: bind(\.screenshot.copy_to_clipboard)).toggleStyle(.switch)
                        }
                        divider
                        row("System audio", "Record what your Mac plays into videos") {
                            Toggle("", isOn: bind(\.audio.enabled)).toggleStyle(.switch)
                        }
                    }
                    section("Shortcuts") {
                        ForEach(Array(HotkeyCommand.allCases.enumerated()), id: \.element) { index, command in
                            if index > 0 { divider }
                            row(command.name, Hotkeys.shared.taken.contains(command)
                                ? "Taken by another app – pick another" : nil) {
                                ShortcutField(command: command)
                            }
                        }
                    }
                    advancedSection
                    section("Save to") {
                        HStack(spacing: 10) {
                            Image(systemName: "folder.fill")
                                .foregroundStyle(Color.accentColor)
                            Text(homeRelative(model.paths.capture_root))
                                .font(.system(size: 12, design: .monospaced))
                                .lineLimit(1)
                                .truncationMode(.head)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Button("Open") { model.openOutput() }
                                .buttonStyle(.glass)
                                .controlSize(.small)
                        }
                        .frame(minHeight: 34)
                    }
                    VStack(spacing: 2) {
                        Text("RapidCap \(model.version) · settings schema \(model.settings.schema_version)")
                        Text(homeRelative(model.paths.settings_file))
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    .font(.system(size: 10.5))
                    .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 16)
                .padding(.top, 4)
            }
        }
        .scrollIndicators(.never)
        .disabled(model.state.isBusy)
    }

    // MARK: presets

    private var presetPicker: some View {
        let current = presets.first { $0.matches(model.settings) }
        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                sectionTitle("Quality")
                Spacer()
                if current == nil {
                    Text("Custom")
                        .font(.system(size: 10.5, weight: .semibold))
                        .foregroundStyle(Color.accentColor)
                        .padding(.trailing, 4)
                        .transition(.opacity.combined(with: .scale))
                }
            }
            HStack(spacing: 8) {
                ForEach(presets) { preset in
                    let selected = current?.id == preset.id
                    Button {
                        withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
                            model.update(preset.apply)
                        }
                    } label: {
                        VStack(alignment: .leading, spacing: 6) {
                            Image(systemName: preset.symbol)
                                .font(.system(size: 16, weight: .medium))
                                .foregroundStyle(selected ? Color.accentColor : .primary)
                                .symbolEffect(.bounce, value: selected)
                            Text(preset.name)
                                .font(.system(size: 13, weight: .semibold))
                            Text(preset.tagline)
                                .font(.system(size: 10.5))
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .padding(11)
                        .frame(maxWidth: .infinity, minHeight: 96, alignment: .topLeading)
                        .contentShape(.rect(cornerRadius: 14))
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .glassEffect(selected ? .regular.tint(.accentColor.opacity(0.3)).interactive()
                        : .regular.interactive(), in: .rect(cornerRadius: 14))
                }
            }
        }
        .animation(.smooth, value: current?.id)
    }

    // MARK: advanced

    private var advancedSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) { advanced.toggle() }
            } label: {
                HStack(spacing: 6) {
                    sectionTitle("Advanced")
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(advanced ? 90 : 0))
                    Spacer()
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()

            if advanced {
                VStack(spacing: 2) {
                    subhead("Video")
                    row("Frame rate", nil) {
                        Picker("", selection: bind(\.video.fps)) {
                            ForEach([24, 30, 60], id: \.self) { Text("\($0) fps").tag($0) }
                        }
                        .fixedSize()
                    }
                    divider
                    row("Quality", String(format: "%g Mbps", Double(model.settings.video.bitrate) / 1_000_000)) {
                        Picker("", selection: Binding(
                            get: { qualities.first { $0.bitrate == model.settings.video.bitrate }?.name ?? "High" },
                            set: { name in
                                guard let quality = qualities.first(where: { $0.name == name }) else { return }
                                model.update {
                                    $0.video.bitrate = quality.bitrate
                                    $0.video.preset = quality.preset
                                    $0.video.tune = quality.tune
                                }
                            }
                        )) {
                            ForEach(qualities, id: \.name) { Text($0.name).tag($0.name) }
                        }
                        .fixedSize()
                    }
                    subhead("Audio")
                    row("Bitrate", "System audio in videos") {
                        Picker("", selection: bind(\.audio.bitrate)) {
                            ForEach([96_000, 128_000, 192_000], id: \.self) { Text("\($0 / 1000) kbps").tag($0) }
                        }
                        .fixedSize()
                    }
                    subhead("GIF")
                    row("Frame rate", nil) {
                        Picker("", selection: bind(\.gif.fps)) {
                            ForEach([10, 15, 20], id: \.self) { Text("\($0) fps").tag($0) }
                        }
                        .fixedSize()
                    }
                    divider
                    row("Dithering", dithers.first { $0.id == model.settings.gif.dither }?.blurb) {
                        Picker("", selection: bind(\.gif.dither)) {
                            ForEach(dithers, id: \.id) { Text($0.name).tag($0.id) }
                        }
                        .fixedSize()
                    }
                    divider
                    row("Palette", "One for the clip, or one per frame") {
                        Picker("", selection: bind(\.gif.palette_stats_mode)) {
                            Text("Whole clip").tag("full")
                            Text("Changes only").tag("diff")
                            Text("Per frame").tag("single")
                        }
                        .fixedSize()
                    }
                    subhead("Screenshots")
                    row("Save as JPEG above", "Large PNGs become JPEG") {
                        Picker("", selection: bind(\.screenshot.png_to_jpeg_threshold_bytes)) {
                            ForEach([1, 2, 4, 8], id: \.self) { Text("\($0) MB").tag($0 << 20) }
                        }
                        .fixedSize()
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .glassEffect(.regular, in: .rect(cornerRadius: 14))
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
    }

    // MARK: building blocks

    private var divider: some View { Divider().opacity(0.3) }

    private func bind<T: Equatable>(_ path: WritableKeyPath<Settings, T>) -> Binding<T> {
        Binding(
            get: { model.settings[keyPath: path] },
            set: { value in withAnimation(.smooth) { model.update { $0[keyPath: path] = value } } }
        )
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(title.uppercased())
            .font(.system(size: 10, weight: .semibold))
            .tracking(0.5)
            .foregroundStyle(.secondary)
            .padding(.leading, 10)
    }

    private func subhead(_ title: String) -> some View {
        Text(title)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 8)
            .padding(.bottom, 2)
    }

    private func section(_ title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            sectionTitle(title)
            VStack(spacing: 2) { content() }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .glassEffect(.regular, in: .rect(cornerRadius: 14))
        }
    }

    private func row(_ title: String, _ detail: String?, @ViewBuilder control: () -> some View) -> some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(.system(size: 12.5))
                    .lineLimit(1)
                if let detail {
                    Text(detail)
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .contentTransition(.opacity)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            control()
                .labelsHidden()
                .controlSize(.small)
        }
        .frame(minHeight: 34)
    }
}

// MARK: shortcut recorder

/// Click, press the new chord. Esc cancels, ⌫ goes back to the default.
private struct ShortcutField: View {
    let command: HotkeyCommand
    @Local private var note: String? = nil

    private var recording: Bool { Hotkeys.shared.recording == command }

    var body: some View {
        HStack(spacing: 6) {
            if let note {
                Text(note)
                    .font(.system(size: 10))
                    .foregroundStyle(.orange)
                    .transition(.opacity.combined(with: .move(edge: .trailing)))
            }
            Button(action: toggle) {
                Text(recording ? "Press keys…" : Hotkeys.shared.label(command))
                    .font(.system(size: 11.5, weight: .medium, design: .rounded))
                    .foregroundStyle(recording ? Color.accentColor : .primary)
                    .strikethrough(!recording && Hotkeys.shared.taken.contains(command))
                    .contentTransition(.numericText())
                    .padding(.horizontal, 10)
                    .frame(minWidth: 78, minHeight: 24)
                    .contentShape(.capsule)
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()
            .glassEffect(recording ? .regular.tint(.accentColor.opacity(0.3)).interactive() : .regular.interactive(),
                         in: .capsule)
            .help("Click, then press a new shortcut. Esc cancels, Delete resets.")
        }
        .animation(.smooth(duration: 0.2), value: recording)
        .animation(.smooth(duration: 0.2), value: note)
        .onDisappear(perform: stop)
    }

    private func toggle() {
        recording ? stop() : start()
    }

    private func start() {
        note = nil
        Hotkeys.shared.record(command, onKey: handle)
    }

    private func stop() {
        if recording { Hotkeys.shared.endRecording() }
    }

    private func handle(_ event: NSEvent) {
        let bare = event.modifierFlags.intersection([.command, .option, .control, .shift]).isEmpty
        if event.keyCode == 53 { // Escape
            stop()
            return
        }
        if bare, event.keyCode == 51 || event.keyCode == 117 { // Delete, Forward Delete
            stop()
            Hotkeys.shared.set(command, to: nil)
            return
        }
        guard let chord = Chord(event: event) else {
            flash("Add ⌘, ⌥ or ⌃")
            return
        }
        if let owner = Hotkeys.shared.owner(of: chord, except: command) {
            flash("Used by \(owner.name)")
            return
        }
        stop()
        Hotkeys.shared.set(command, to: chord)
        if Hotkeys.shared.taken.contains(command) { flash("Taken by another app") }
    }

    private func flash(_ text: String) {
        note = text
        Task {
            try? await Task.sleep(for: .seconds(2.5))
            if note == text { note = nil }
        }
    }
}
