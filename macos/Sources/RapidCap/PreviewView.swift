// RapidCap's own preview window content: one capture - a picture, an
// animated GIF or a video - with the gallery's actions under it. Opening a
// capture used to hand it to Preview.app or QuickTime; now it stays here.

import AppKit
import AVKit
import SwiftUI

struct PreviewRoot: View {
    let model: AppModel
    let metrics: MetricsBox

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text(model.previewing?.lastPathComponent ?? "")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 6)
            }
            .padding(.leading, metrics.value.leading)
            .padding(.trailing, 10)
            .frame(height: metrics.value.centerFromTop * 2)
            .frame(maxWidth: .infinity)

            if let url = model.previewing {
                PreviewContent(url: url)
                    // A new capture is a new view: the old one's player is
                    // dismantled (paused, released) before the next starts.
                    .id(url)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .clipShape(.rect(cornerRadius: 12))
                    .padding(.horizontal, 12)
                actions(url)
            } else {
                Spacer(minLength: 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .ignoresSafeArea()
        // Esc closes the window. A real but invisible button, because a
        // hidden one takes no keyboard shortcut.
        .background {
            Button(String(localized: "Close")) { model.closePreview() }
                .keyboardShortcut(.cancelAction)
                .opacity(0)
                .frame(width: 0, height: 0)
                .accessibilityHidden(true)
        }
    }

    private func actions(_ url: URL) -> some View {
        HStack(spacing: 8) {
            IconButton(symbol: "doc.on.doc", help: String(localized: "Copy")) { try? Core.copyFile(url) }
            IconButton(symbol: "folder", help: String(localized: "Show in Finder")) { model.reveal(url) }
            IconButton(symbol: "trash", help: String(localized: "Move to Trash")) {
                // The gallery's delete: to the Trash, then let the gallery
                // list the folder again.
                guard (try? FileManager.default.trashItem(at: url, resultingItemURL: nil)) != nil else { return }
                model.capturesVersion += 1
                model.closePreview()
            }
            Spacer(minLength: 6)
            IconButton(symbol: "arrow.up.forward.app", help: String(localized: "Open with default app")) {
                NSWorkspace.shared.open(url)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 44)
    }
}

/// Picks the viewer by file type: the capture kinds RapidCap saves.
private struct PreviewContent: View {
    let url: URL

    var body: some View {
        switch url.pathExtension.lowercased() {
        case "mp4", "mov", "m4v":
            VideoPlayerView(url: url)
        case "gif":
            AnimatedImageView(url: url)
        default:
            if let image = NSImage(contentsOf: url) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Text(String(localized: "This file cannot be shown"))
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }
}

/// A looping GIF. SwiftUI's `Image` shows only the first frame; AppKit's
/// image view plays the frames itself, so no SwiftUI animation runs.
private struct AnimatedImageView: NSViewRepresentable {
    let url: URL

    func makeNSView(context: Context) -> NSImageView {
        let view = NSImageView()
        view.image = NSImage(contentsOf: url)
        view.animates = true
        view.imageScaling = .scaleProportionallyUpOrDown
        view.imageAlignment = .alignCenter
        // Let SwiftUI size it; the GIF's own size must not push the window.
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
        return view
    }

    func updateNSView(_ view: NSImageView, context: Context) {}
}

/// A video with the system's inline controls, playing once from the start.
/// The player is paused and dropped when the view goes - the window hid or
/// another capture replaced it - so no sound keeps playing behind it.
private struct VideoPlayerView: NSViewRepresentable {
    let url: URL

    func makeNSView(context: Context) -> AVPlayerView {
        let view = AVPlayerView()
        view.controlsStyle = .inline
        view.videoGravity = .resizeAspect
        let player = AVPlayer(url: url)
        view.player = player
        player.play()
        return view
    }

    func updateNSView(_ view: AVPlayerView, context: Context) {}

    static func dismantleNSView(_ view: AVPlayerView, coordinator: ()) {
        view.player?.pause()
        view.player = nil
    }
}
