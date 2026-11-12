// Swift face of the Rust capture core (crates/ffi).
//
// Every call answers JSON; this file is the only place that knows it. The
// settings structs mirror `rapidcap_capture::Settings` field for field, because
// the Rust loader rejects unknown keys and the file is shared with Windows.

import CRapidCap
import Foundation

struct ScreenshotSettings: Codable, Equatable {
    var png_to_jpeg_threshold_bytes: Int
    var jpeg_quality: Int
    var copy_to_clipboard: Bool
}

struct VideoSettings: Codable, Equatable {
    var fps: Int
    var bitrate: Int
    var preset: String
    var tune: String
}

struct AudioSettings: Codable, Equatable {
    var enabled: Bool
    var microphone: String?
    var bitrate: Int
    var channels: Int
}

struct GifSettings: Codable, Equatable {
    var fps: Int
    var palette_stats_mode: String
    var dither: String
}

struct Settings: Codable, Equatable {
    var schema_version: Int
    var screenshot: ScreenshotSettings
    var video: VideoSettings
    var audio: AudioSettings
    var gif: GifSettings
    var countdown_seconds: Int
    var hide_panel_while_capturing: Bool

    // `microphone: null` has to be written out, not dropped: the Rust side
    // reads a missing key and a null the same way, but an explicit null keeps
    // the file diff-stable against what the Windows app writes.
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(schema_version, forKey: .schema_version)
        try container.encode(screenshot, forKey: .screenshot)
        try container.encode(video, forKey: .video)
        try container.encode(audio, forKey: .audio)
        try container.encode(gif, forKey: .gif)
        try container.encode(countdown_seconds, forKey: .countdown_seconds)
        try container.encode(hide_panel_while_capturing, forKey: .hide_panel_while_capturing)
    }
}

extension AudioSettings {
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(enabled, forKey: .enabled)
        try container.encode(microphone, forKey: .microphone)
        try container.encode(bitrate, forKey: .bitrate)
        try container.encode(channels, forKey: .channels)
    }
}

struct Paths: Codable {
    var capture_root: String
    var settings_file: String
    var temp_dir: String
    var thumb_dir: String
}

struct CoreError: Error, LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// A region in global Core Graphics device pixels, which is what the core's
/// `PhysicalRegion` means on macOS: top-left origin on the main display,
/// times its backing scale.
struct PixelRect: Equatable, Sendable {
    var x: Int32
    var y: Int32
    var width: UInt32
    var height: UInt32
}

enum Core {
    private static func take(_ pointer: UnsafeMutablePointer<CChar>?) -> [String: Any] {
        guard let pointer else { return ["error": String(localized: "the capture core returned nothing")] }
        defer { rc_free(pointer) }
        let data = Data(String(cString: pointer).utf8)
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            ?? ["error": String(localized: "the capture core returned unreadable JSON")]
    }

    private static func check(_ answer: [String: Any]) throws -> [String: Any] {
        if let message = answer["error"] as? String { throw CoreError(message: message) }
        return answer
    }

    private static func decode<T: Decodable>(_ type: T.Type, _ value: Any?) throws -> T {
        let data = try JSONSerialization.data(withJSONObject: value ?? NSNull())
        return try JSONDecoder().decode(type, from: data)
    }

    static func load() throws -> (Settings, Paths, String) {
        let answer = try check(take(rc_load()))
        return (
            try decode(Settings.self, answer["settings"]),
            try decode(Paths.self, answer["paths"]),
            // The app's own version; the core crate's lags behind the bundle.
            Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
                ?? answer["version"] as? String ?? "?"
        )
    }

    static func save(_ settings: Settings) throws {
        let data = try JSONEncoder().encode(settings)
        let text = String(decoding: data, as: UTF8.self)
        _ = try check(take(text.withCString { rc_save_settings($0) }))
    }

    struct Saved: Sendable {
        let path: URL
        let note: String?
    }

    static func screenshot(_ rect: PixelRect, processName: String?) throws -> Saved {
        let answer: [String: Any]
        if let processName {
            answer = take(processName.withCString {
                rc_screenshot(rect.x, rect.y, rect.width, rect.height, $0)
            })
        } else {
            answer = take(rc_screenshot(rect.x, rect.y, rect.width, rect.height, nil))
        }
        let checked = try check(answer)
        return Saved(
            path: URL(fileURLWithPath: checked["path"] as? String ?? ""),
            note: (checked["clipboard_error"] as? String).map { String(localized: "not copied: \($0)") }
        )
    }

    /// Put the system-audio track into a finished video, in place.
    static func muxAudio(video: URL, audio: URL, seconds: Double) throws {
        _ = try check(take(video.path.withCString { videoPath in
            audio.path.withCString { rc_mux_audio(videoPath, $0, seconds) }
        }))
    }

    static func copyFile(_ url: URL) throws {
        _ = try check(take(url.path.withCString { rc_copy_file($0) }))
    }
}

/// One FFmpeg recording. Every call blocks on FFmpeg, so callers run them off
/// the main thread.
final class RecordingHandle: @unchecked Sendable {
    private var raw: OpaquePointer?
    /// One call at a time. Pause, stop and cancel run on different detached
    /// tasks; without this a stop could free the Rust handle while a pause
    /// was still using it.
    private let lock = NSLock()

    init(gif: Bool, rect: PixelRect) throws {
        var error: UnsafeMutablePointer<CChar>?
        raw = rc_record_start(gif ? 1 : 0, rect.x, rect.y, rect.width, rect.height, &error)
        if raw == nil {
            var message = String(localized: "recording did not start")
            if let error {
                message = (try? JSONSerialization.jsonObject(with: Data(String(cString: error).utf8))
                    as? [String: Any])?["error"] as? String ?? message
                rc_free(error)
            }
            throw CoreError(message: message)
        }
    }

    private func call(_ body: (OpaquePointer?) -> UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
        guard raw != nil else { throw CoreError(message: String(localized: "the recording has already ended")) }
        guard let pointer = body(raw) else {
            throw CoreError(message: String(localized: "the capture core returned nothing"))
        }
        defer { rc_free(pointer) }
        let answer = (try? JSONSerialization.jsonObject(with: Data(String(cString: pointer).utf8)))
            as? [String: Any] ?? [:]
        if let message = answer["error"] as? String { throw CoreError(message: message) }
        return answer
    }

    func pause() throws { try lock.withLock { _ = try call { rc_record_pause($0) } } }
    func resume() throws { try lock.withLock { _ = try call { rc_record_resume($0) } } }

    func stop() throws -> Core.Saved {
        lock.lock()
        defer {
            raw = nil
            lock.unlock()
        }
        let answer = try call { rc_record_stop($0) }
        return Core.Saved(
            path: URL(fileURLWithPath: answer["path"] as? String ?? ""),
            note: (answer["clipboard_error"] as? String).map { String(localized: "not copied: \($0)") }
        )
    }

    func cancel() {
        lock.withLock {
            rc_record_cancel(raw)
            raw = nil
        }
    }
}
