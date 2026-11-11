// The menu bar icon: the app icon in one colour - the tile as an outline,
// with its mark, two viewfinder corners round a record dot, solid inside it.
// A template image, so macOS tints it for light, dark and highlighted menu
// bars. A little smaller than the 18pt slot, as menu bar glyphs are. While a
// recording runs, the dot grows.
//
// Pixel-aligned rather than a scaled copy of the app icon: every edge sits on
// a half point, a whole device pixel on a Retina screen, and the corners are
// upright. The app icon's forward lean, at this size, only blurred them.

import AppKit

@MainActor
enum StatusMark {
    private static var cache: [Bool: NSImage] = [:]

    static func image(recording: Bool) -> NSImage {
        if let image = cache[recording] { return image }
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: true) { _ in
            guard let context = NSGraphicsContext.current?.cgContext else { return false }
            context.setFillColor(NSColor.black.cgColor)
            // The tile: a 14pt rounded square, its edge 1.5pt thick.
            let tile = CGMutablePath()
            tile.addRoundedRect(in: CGRect(x: 2, y: 2, width: 14, height: 14), cornerWidth: 4, cornerHeight: 4)
            tile.addRoundedRect(in: CGRect(x: 3.5, y: 3.5, width: 11, height: 11), cornerWidth: 2.5, cornerHeight: 2.5)
            context.addPath(tile)
            context.fillPath(using: .evenOdd)

            // The mark, symmetric about (9, 9).
            for bar in [CGRect(x: 5, y: 5, width: 3, height: 1.5),       // top-left, across
                        CGRect(x: 5, y: 5, width: 1.5, height: 3),       // top-left, down
                        CGRect(x: 10, y: 11.5, width: 3, height: 1.5),   // bottom-right, across
                        CGRect(x: 11.5, y: 10, width: 1.5, height: 3)] { // bottom-right, up
                context.fill(bar)
            }
            let radius: CGFloat = recording ? 1.75 : 1.25
            context.fillEllipse(in: CGRect(x: 9 - radius, y: 9 - radius, width: radius * 2, height: radius * 2))
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = recording ? "RapidCap – recording" : "RapidCap"
        cache[recording] = image
        return image
    }
}
