// System audio for a video recording.
//
// FFmpeg's macOS input sees screens and microphones, never what the Mac
// plays, so the sound comes from ScreenCaptureKit instead: an audio-only
// stream written to an AAC file beside the video, and put into the video by
// the core (`rc_mux_audio`) once both have stopped. Pauses are cut out of the
// audio the same way the core cuts them out of the video.

import AVFoundation
import ScreenCaptureKit

final class SystemAudio: NSObject, SCStreamOutput, @unchecked Sendable {
    let url: URL
    private let queue = DispatchQueue(label: "com.inspire.rapidcap.system-audio")
    private let writer: AVAssetWriter
    private let input: AVAssetWriterInput
    private var stream: SCStream?

    // Touched only on `queue`.
    private var first: CMTime?
    private var lastEnd: CMTime?
    private var shift = CMTime.zero
    private var written = CMTime.zero
    private var paused = false
    private var resumed = false
    /// Set by `end()` and `cancel()`: no buffer after it is written, so the
    /// track stops when the video does and a late callback cannot start a
    /// new file after a cancel.
    private var ended = false

    private init(bitrate: Int, channels: Int) throws {
        url = FileManager.default.temporaryDirectory
            .appendingPathComponent("rapidcap-audio-\(UUID().uuidString).m4a")
        writer = try AVAssetWriter(outputURL: url, fileType: .m4a)
        input = AVAssetWriterInput(mediaType: .audio, outputSettings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48_000,
            AVNumberOfChannelsKey: channels,
            AVEncoderBitRateKey: bitrate,
        ])
        input.expectsMediaDataInRealTime = true
        writer.add(input)
    }

    static func start(bitrate: Int, channels: Int) async throws -> SystemAudio {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let display = content.displays.first else {
            throw CoreError(message: "no display to take system audio from")
        }
        let config = SCStreamConfiguration()
        config.capturesAudio = true
        config.excludesCurrentProcessAudio = true
        config.sampleRate = 48_000
        config.channelCount = channels
        // The stream always carries video too; keep that half as small and
        // as rare as it goes, since nothing reads it.
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(value: 1, timescale: 1)

        let recorder = try SystemAudio(bitrate: bitrate, channels: channels)
        let stream = SCStream(filter: SCContentFilter(display: display, excludingWindows: []),
                              configuration: config, delegate: nil)
        try stream.addStreamOutput(recorder, type: .audio, sampleHandlerQueue: recorder.queue)
        try await stream.startCapture()
        recorder.stream = stream
        return recorder
    }

    func pause() { queue.sync { paused = true } }

    func resume() {
        queue.sync {
            paused = false
            resumed = true
        }
    }

    /// Take no more sound. Called the moment Stop is pressed, alongside
    /// FFmpeg's own stop, so the two tracks end together.
    func end() { queue.sync { ended = true } }

    /// Close the file: the track and its length in seconds.
    func finish() async -> Result<(url: URL, seconds: Double), Error> {
        end()
        try? await stream?.stopCapture()
        let seconds: Double? = queue.sync {
            guard first != nil else { return nil }
            input.markAsFinished()
            return written.seconds
        }
        guard let seconds else {
            discard()
            return .failure(CoreError(message: "no sound arrived from the system"))
        }
        await writer.finishWriting()
        guard writer.status == .completed else {
            discard()
            return .failure(writer.error ?? CoreError(message: "the sound track could not be written"))
        }
        return .success((url, seconds))
    }

    func cancel() {
        let stream = stream
        Task { try? await stream?.stopCapture() }
        queue.sync {
            ended = true
            if first != nil { writer.cancelWriting() }
        }
        discard()
    }

    private func discard() {
        try? FileManager.default.removeItem(at: url)
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, buffer.isValid, !paused, !ended else { return }
        let start = buffer.presentationTimeStamp
        let length = buffer.duration.isValid
            ? buffer.duration
            : CMTime(value: CMTimeValue(buffer.numSamples), timescale: 48_000)
        if first == nil {
            guard writer.startWriting() else { return }
            writer.startSession(atSourceTime: start)
            first = start
        }
        // The time spent paused, taken out so the track runs on without a gap.
        if resumed, let lastEnd {
            shift = shift + (start - lastEnd)
            resumed = false
        }
        lastEnd = start + length

        var sample = buffer
        if shift != .zero {
            var timing = CMSampleTimingInfo(duration: length, presentationTimeStamp: start - shift,
                                            decodeTimeStamp: .invalid)
            var copy: CMSampleBuffer?
            CMSampleBufferCreateCopyWithNewTiming(allocator: nil, sampleBuffer: buffer, sampleTimingEntryCount: 1,
                                                  sampleTimingArray: &timing, sampleBufferOut: &copy)
            guard let copy else { return }
            sample = copy
        }
        if input.isReadyForMoreMediaData, input.append(sample), let first {
            written = start - shift + length - first
        }
    }
}
