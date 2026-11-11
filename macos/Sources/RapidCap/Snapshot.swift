// `RapidCap --selftest` and `--e2e`: checks for a machine you are not
// sitting at.

import AppKit

/// `RapidCap --selftest`: one screenshot and a two-second recording through
/// the core, printed as JSON, then quit. Proves the FFI, the signature's
/// Screen Recording grant and FFmpeg in one go.
@MainActor
enum SelfTest {
    static var requested: Bool { CommandLine.arguments.contains("--selftest") }

    static func run() {
        Task { @MainActor in
            var report: [String: String] = [:]
            // The captures go on the clipboard; put back what was there.
            let clipboard = NSPasteboard.general.pasteboardItems?.map { item in
                item.types.reduce(into: [NSPasteboard.PasteboardType: Data]()) { $0[$1] = item.data(forType: $1) }
            } ?? []
            defer {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.writeObjects(clipboard.map { types in
                    let item = NSPasteboardItem()
                    for (type, data) in types { item.setData(data, forType: type) }
                    return item
                })
            }
            let rect = PixelRect(x: 0, y: 0, width: 128, height: 64)
            if !CommandLine.arguments.contains("--audio") {
                let shot = await offMain { Result { try Core.screenshot(rect, processName: nil) } }
                report["screenshot"] = describe(shot)
            }
            // Video with system audio, paused once in the middle: the path a
            // real recording takes, sound muxed in at the end.
            let sound = try? await SystemAudio.start(bitrate: 128_000, channels: 2)
            report["audio_started"] = sound == nil ? "no" : "yes"
            let started = await offMain { Result { try RecordingHandle(gif: false, rect: rect) } }
            switch started {
            case let .success(handle):
                try? await Task.sleep(for: .seconds(2))
                sound?.pause()
                _ = await offMain { Result { try handle.pause() } }
                try? await Task.sleep(for: .seconds(1))
                _ = await offMain { Result { try handle.resume() } }
                sound?.resume()
                try? await Task.sleep(for: .seconds(2))
                let track = try? await sound?.finish().get()
                report["audio_seconds"] = track.map { String(format: "%.2f", $0.seconds) } ?? "none"
                var saved = await offMain { Result { try handle.stop() } }
                if case let .success(file) = saved, let track {
                    let mixed = await offMain { Result { try Core.muxAudio(video: file.path, audio: track.url,
                                                                           seconds: track.seconds) } }
                    if case let .failure(problem) = mixed { saved = .failure(problem) }
                }
                report["video"] = describe(saved)
            case let .failure(error):
                report["video"] = "error: \(error.localizedDescription)"
            }
            let data = (try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])) ?? Data()
            print(String(decoding: data, as: UTF8.self))
            NSApp.terminate(nil)
        }
    }

    private static func describe(_ result: Result<Core.Saved, Error>) -> String {
        switch result {
        case let .success(saved):
            let size = (try? saved.path.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            return "\(saved.path.path) (\(size) bytes)"
        case let .failure(error):
            return "error: \(error.localizedDescription)"
        }
    }
}

/// `RapidCap --e2e`: the real recording flow - countdown, HUD, frame, FFmpeg,
/// system audio, pause, stop, mux - for a small corner of the screen, with no
/// mouse or keyboard input. Three takes: video with a pause, GIF with a pause,
/// then video again, since the freeze showed on a second recording. Each file
/// is checked with ffprobe and deleted; the main thread's CPU is measured
/// after each take. Prints JSON and quits.
@MainActor
enum EndToEnd {
    static var requested: Bool { CommandLine.arguments.contains("--e2e") }

    static func run(_ model: AppModel) {
        Task { @MainActor in
            let clipboard = NSPasteboard.general.pasteboardItems?.map { item in
                item.types.reduce(into: [NSPasteboard.PasteboardType: Data]()) { $0[$1] = item.data(forType: $1) }
            } ?? []
            var takes: [[String: String]] = []
            let top = NSScreen.screens.first?.frame.maxY ?? 0
            let corner = CGRect(x: 0, y: top - 64, width: 128, height: 64)
            for (kind, pause) in [(CaptureKind.video, true), (.gif, true), (.video, false)] {
                takes.append(await take(model, kind, pause: pause, at: corner))
            }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.writeObjects(clipboard.map { types in
                let item = NSPasteboardItem()
                for (type, data) in types { item.setData(data, forType: type) }
                return item
            })
            let data = (try? JSONSerialization.data(withJSONObject: takes, options: [.sortedKeys])) ?? Data()
            print(String(decoding: data, as: UTF8.self))
            NSApp.terminate(nil)
        }
    }

    private static func take(_ model: AppModel, _ kind: CaptureKind, pause: Bool, at rect: CGRect) async
        -> [String: String] {
        var report = ["kind": kind.rawValue]
        model.lastSaved = nil
        model.record(kind, at: rect)
        guard await until(20, { model.state == .recording(kind) }) else {
            report["failed"] = "never recorded: state \(model.state), error \(model.error ?? "-")"
            model.cancel()
            return report
        }
        try? await Task.sleep(for: .seconds(1.5))
        if pause {
            model.togglePause()
            report["paused"] = await until(5, { model.state == .paused(kind) }) ? "yes" : "no"
            try? await Task.sleep(for: .seconds(1))
            model.togglePause()
            report["resumed"] = await until(8, { model.state == .recording(kind) }) ? "yes" : "no"
        }
        try? await Task.sleep(for: .seconds(1.5))
        model.trigger(kind)
        guard await until(40, { model.state == .idle }) else {
            report["failed"] = "never saved: state \(model.state)"
            return report
        }
        report["error"] = model.error ?? "-"
        if let file = model.lastSaved {
            report["streams"] = probe(file)
            try? FileManager.default.removeItem(at: file)
        } else {
            report["failed"] = "no file"
        }
        report["idle_cpu"] = String(format: "%.0f%%", await cpu(over: 2) * 100)
        return report
    }

    private static func until(_ seconds: Double, _ done: () -> Bool) async -> Bool {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            if done() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return done()
    }

    /// The process's CPU use while it should be idle: a spinning main thread
    /// shows as close to 100%.
    private static func cpu(over seconds: Double) async -> Double {
        func used() -> Double {
            var usage = rusage()
            getrusage(RUSAGE_SELF, &usage)
            return Double(usage.ru_utime.tv_sec + usage.ru_stime.tv_sec)
                + Double(usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) / 1_000_000
        }
        let before = used()
        try? await Task.sleep(for: .seconds(seconds))
        return (used() - before) / seconds
    }

    private static func probe(_ file: URL) -> String {
        let tool = ["/opt/homebrew/bin/ffprobe", "/usr/local/bin/ffprobe"]
            .first { FileManager.default.isExecutableFile(atPath: $0) }
        guard let tool else { return "no ffprobe" }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: tool)
        process.arguments = ["-v", "error", "-count_frames", "-show_entries",
                             "stream=codec_type,codec_name,duration,nb_read_frames", "-of", "compact=p=0:nk=1",
                             file.path]
        let pipe = Pipe()
        process.standardOutput = pipe
        try? process.run()
        process.waitUntilExit()
        let text = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        return text.split(separator: "\n").joined(separator: " ; ")
    }
}
