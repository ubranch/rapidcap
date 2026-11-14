// Video and GIF through ScreenCaptureKit.
//
// FFmpeg's AVFoundation input takes the whole display and crops it in a
// filter, so every frame of every pixel is read whatever the region, and it
// starts late: each run lost up to a second and a half off the front of the
// take, more on a busy machine. Here the window server crops and scales before
// the frame leaves it, and AVAssetWriter writes it - H.264 for a video, ProRes
// 4444 for a GIF, whose palette pass the core runs on the file at stop. Files
// are named and filed by the core (`rc_recording_paths`), the same as an FFmpeg
// recording.

import AppKit
import AVFoundation
import ScreenCaptureKit

final class ScreenRecorder: NSObject, SCStreamOutput, Recorder, @unchecked Sendable {
    /// The picked region on one display: what the stream is set up with.
    struct Area: Sendable {
        let displayID: CGDirectDisplayID
        /// Display points, top-left origin, as `sourceRect` takes them.
        let rect: CGRect
        /// Pixels, even, as H.264 needs.
        let width: Int
        let height: Int
    }

    /// The display holding most of `points` (AppKit global points) and the
    /// part of `points` on it. Nil when no display holds any of it.
    @MainActor
    static func area(_ points: CGRect) -> Area? {
        let screen = NSScreen.screens.max { lhs, rhs in
            let left = lhs.frame.intersection(points), right = rhs.frame.intersection(points)
            return left.width * left.height < right.width * right.height
        }
        guard let screen,
              let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
        else { return nil }
        let clip = screen.frame.intersection(points)
        guard !clip.isNull else { return nil }
        let scale = screen.backingScaleFactor
        // Rounded down to even pixels, and the source shrunk to match, so the
        // picture is cropped by at most a pixel per axis and never stretched.
        let width = Int((clip.width * scale).rounded()) & ~1
        let height = Int((clip.height * scale).rounded()) & ~1
        guard width >= 2, height >= 2 else { return nil }
        let frame = screen.frame
        return Area(
            displayID: number.uint32Value,
            rect: CGRect(x: clip.minX - frame.minX, y: frame.maxY - clip.maxY,
                         width: CGFloat(width) / scale, height: CGFloat(height) / scale),
            width: width,
            height: height
        )
    }

    private let temp: URL
    private let final: URL
    private let gif: Bool
    private let queue = DispatchQueue(label: "com.inspire.rapidcap.screen-recorder")
    private let writer: AVAssetWriter
    private let input: AVAssetWriterInput
    private var stream: SCStream?

    // Touched only on `queue`. Times are on the host clock, the one
    // ScreenCaptureKit stamps its frames with.
    private var started = false
    private var last: CMTime?
    /// Total time spent paused, taken off every later frame so the file runs
    /// on without a gap.
    private var shift = CMTime.zero
    private var pausedAt: CMTime?
    /// Frames stamped before this were taken during the pause.
    private var resumedAt = CMTime.zero
    /// The newest frame seen while paused. The stream sends a frame only when
    /// the picture changes, so without it a still screen after a resume would
    /// show what was there before the pause.
    private var held: CMSampleBuffer?
    /// Set by stop and cancel: nothing is written after it.
    private var ended = false

    private init(temp: URL, final: URL, area: Area, fps: Int, bitrate: Int, gif: Bool) throws {
        self.temp = temp
        self.final = final
        self.gif = gif
        try? FileManager.default.removeItem(at: temp)
        writer = try AVAssetWriter(outputURL: temp, fileType: gif ? .mov : .mp4)
        writer.shouldOptimizeForNetworkUse = !gif
        let colour: [String: Any] = [
            AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
            AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
            AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
        ]
        // A GIF's frames are kept at full colour for its palette pass: ProRes
        // 4444 does not halve the colour of text edges the way H.264 does.
        let settings: [String: Any] = gif ? [
            AVVideoCodecKey: AVVideoCodecType.proRes4444,
            AVVideoWidthKey: area.width,
            AVVideoHeightKey: area.height,
            AVVideoColorPropertiesKey: colour,
        ] : [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: area.width,
            AVVideoHeightKey: area.height,
            AVVideoColorPropertiesKey: colour,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: bitrate,
                AVVideoExpectedSourceFrameRateKey: fps,
                AVVideoMaxKeyFrameIntervalKey: fps * 2,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
                AVVideoAllowFrameReorderingKey: false,
            ],
        ]
        input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
        input.expectsMediaDataInRealTime = true
        super.init()
        writer.add(input)
        guard writer.startWriting() else {
            throw writer.error ?? CoreError(message: String(localized: "the video file could not be opened"))
        }
    }

    /// `fps` and `bitrate` from the video or the GIF settings; a GIF has no
    /// bitrate and takes 0.
    static func start(_ area: Area, fps: Int, bitrate: Int, gif: Bool) async throws -> ScreenRecorder {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let display = content.displays.first(where: { $0.displayID == area.displayID }) else {
            throw CoreError(message: String(localized: "the display to record is gone"))
        }
        let config = SCStreamConfiguration()
        config.sourceRect = area.rect
        config.width = area.width
        config.height = area.height
        config.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(max(1, fps)))
        config.showsCursor = false
        config.capturesAudio = false
        if gif {
            config.pixelFormat = kCVPixelFormatType_32BGRA
            config.colorSpaceName = CGColorSpace.sRGB
        } else {
            config.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
            config.colorMatrix = kCVImageBufferYCbCrMatrix_ITU_R_709_2
            config.colorSpaceName = CGColorSpace.itur_709
        }
        config.queueDepth = 6

        let paths = try Core.recordingPaths(gif: gif)
        let recorder = try ScreenRecorder(temp: paths.temp, final: paths.final, area: area, fps: fps,
                                          bitrate: bitrate, gif: gif)
        // Our own windows - the frame, the HUD - stay out of the picture.
        // `sharingType = .none` alone does not keep them out of this stream.
        let own = content.applications.filter { $0.processID == getpid() }
        let filter = SCContentFilter(display: display, excludingApplications: own, exceptingWindows: [])
        let stream = SCStream(filter: filter, configuration: config, delegate: nil)
        do {
            try stream.addStreamOutput(recorder, type: .screen, sampleHandlerQueue: recorder.queue)
            try await stream.startCapture()
        } catch {
            recorder.cancel()
            throw error
        }
        recorder.stream = stream
        return recorder
    }

    private static func now() -> CMTime { CMClockGetTime(CMClockGetHostTimeClock()) }

    func pause() throws {
        queue.sync {
            if pausedAt == nil { pausedAt = Self.now() }
        }
    }

    func resume() throws {
        queue.sync {
            guard let pausedAt, !ended else { return }
            let now = Self.now()
            shift = shift + (now - pausedAt)
            self.pausedAt = nil
            resumedAt = now
            if let held {
                append(held, at: now - shift)
                self.held = nil
            }
        }
    }

    func stop() throws -> Core.Saved {
        let end: CMTime? = queue.sync {
            guard !ended else { return nil }
            ended = true
            held = nil
            guard started, let last else { return nil }
            // A still screen sends no frames, so the file is held open to the
            // moment Stop was pressed (or the pause it was pressed in).
            let end = max((pausedAt ?? Self.now()) - shift, last)
            writer.endSession(atSourceTime: end)
            input.markAsFinished()
            return end
        }
        stopStream()
        guard end != nil else {
            discard()
            throw CoreError(message: String(localized: "no picture arrived from the screen"))
        }
        let done = DispatchSemaphore(value: 0)
        writer.finishWriting { done.signal() }
        done.wait()
        guard writer.status == .completed else {
            discard()
            throw writer.error ?? CoreError(message: String(localized: "the video could not be written"))
        }
        do {
            if gif {
                // On failure the core keeps the capture and names it in the error.
                try Core.encodeGif(temp, to: final)
                discard()
            } else {
                try FileManager.default.moveItem(at: temp, to: final)
            }
        } catch let error as CoreError {
            throw error
        } catch {
            throw CoreError(message: String(localized: "finalize \(temp.path) to \(final.path) failed: \(error.localizedDescription)"))
        }
        var note: String?
        do {
            try Core.copyFile(final)
        } catch {
            note = String(localized: "not copied: \(error.localizedDescription)")
        }
        return Core.Saved(path: final, note: note)
    }

    func cancel() {
        queue.sync {
            ended = true
            held = nil
            if writer.status == .writing { writer.cancelWriting() }
        }
        stream?.stopCapture(completionHandler: nil)
        discard()
    }

    /// Stop the stream and wait for it, so no frame is in flight when the
    /// file is closed.
    private func stopStream() {
        guard let stream else { return }
        let done = DispatchSemaphore(value: 0)
        stream.stopCapture { _ in done.signal() }
        done.wait()
    }

    private func discard() {
        try? FileManager.default.removeItem(at: temp)
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, buffer.isValid, !ended, Self.isComplete(buffer) else { return }
        if pausedAt != nil {
            held = buffer
            return
        }
        let time = buffer.presentationTimeStamp
        // Taken during the pause but delivered after the resume.
        guard time >= resumedAt else { return }
        append(buffer, at: time - shift)
    }

    /// On `queue`. Timestamps only go forward: AVAssetWriter fails the whole
    /// file on one that does not.
    private func append(_ buffer: CMSampleBuffer, at time: CMTime) {
        if let last, time <= last { return }
        var sample = buffer
        if time != buffer.presentationTimeStamp {
            var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: time,
                                            decodeTimeStamp: .invalid)
            var copy: CMSampleBuffer?
            CMSampleBufferCreateCopyWithNewTiming(allocator: nil, sampleBuffer: buffer, sampleTimingEntryCount: 1,
                                                  sampleTimingArray: &timing, sampleBufferOut: &copy)
            guard let copy else { return }
            sample = copy
        }
        if !started {
            writer.startSession(atSourceTime: time)
            started = true
        }
        if input.isReadyForMoreMediaData, input.append(sample) {
            last = time
        }
    }

    /// A frame with new picture in it. The stream also sends idle and blank
    /// frames, which carry no image to write.
    private static func isComplete(_ buffer: CMSampleBuffer) -> Bool {
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, createIfNecessary: false)
            as? [[SCStreamFrameInfo: Any]],
            let raw = attachments.first?[.status] as? Int,
            let status = SCFrameStatus(rawValue: raw)
        else { return false }
        return status == .complete
    }
}
