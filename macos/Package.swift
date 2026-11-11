// swift-tools-version: 6.2
// The native macOS app: SwiftUI with Liquid Glass over the Rust capture core.
//
// The core is `crates/ffi`, built as a static library first; scripts/
// package-macos.sh builds both and passes the library directory with -L.
import PackageDescription

let package = Package(
    name: "RapidCap",
    platforms: [.macOS("26.0")],
    targets: [
        .systemLibrary(name: "CRapidCap", path: "Sources/CRapidCap"),
        .executableTarget(
            name: "RapidCap",
            dependencies: ["CRapidCap"],
            path: "Sources/RapidCap",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("Carbon"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("QuickLookThumbnailing"),
                .linkedFramework("ScreenCaptureKit"),
            ]
        ),
    ]
)
