// Recent captures: the capture folder and its YYYY-MM subfolders, newest
// first, with Quick Look thumbnails.

import QuickLookThumbnailing
import SwiftUI

struct CaptureFile: Identifiable, Hashable {
    let url: URL
    let date: Date
    var id: URL { url }

    var kind: Filter {
        switch url.pathExtension.lowercased() {
        case "mp4", "mov": .video
        case "gif": .gif
        default: .images
        }
    }
}

enum Filter: String, CaseIterable {
    case all = "All", images = "Images", video = "Video", gif = "GIF"

    var title: String {
        switch self {
        case .all: String(localized: "All")
        case .images: String(localized: "Images")
        case .video: String(localized: "Video")
        case .gif: String(localized: "GIF")
        }
    }
}

private let extensions: Set<String> = ["png", "jpg", "jpeg", "gif", "mp4", "mov"]

func listCaptures(in root: URL) -> [CaptureFile] {
    let manager = FileManager.default
    var folders = [root]
    if let children = try? manager.contentsOfDirectory(at: root, includingPropertiesForKeys: [.isDirectoryKey]) {
        folders += children.filter { (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true }
    }
    var files: [CaptureFile] = []
    for folder in folders {
        let items = (try? manager.contentsOfDirectory(
            at: folder, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]
        )) ?? []
        for item in items where extensions.contains(item.pathExtension.lowercased()) {
            let date = (try? item.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate)
                ?? .distantPast
            files.append(CaptureFile(url: item, date: date))
        }
    }
    return files.sorted { $0.date > $1.date }
}

struct GalleryView: View {
    let model: AppModel
    @Local private var files: [CaptureFile] = []
    @Local private var filter: Filter = .all

    private var shown: [CaptureFile] {
        filter == .all ? files : files.filter { $0.kind == filter }
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                LiquidSegmented(options: Filter.allCases.map { ($0, $0.title) }, selection: $filter,
                                height: 24)
                Spacer()
                Text(shown.count == 1 ? String(localized: "1 capture") : String(localized: "\(shown.count) captures"))
                    .font(.system(size: 10.5, weight: .medium).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .contentTransition(.numericText())
            }
            .padding(.horizontal, 12)

            if shown.isEmpty {
                EmptyGallery(model: model, filtered: !files.isEmpty)
            } else {
                ScrollView {
                    LazyVGrid(columns: [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10),
                                        GridItem(.flexible(), spacing: 10)], spacing: 10) {
                        ForEach(shown) { file in
                            Tile(file: file, model: model) { reload() }
                                .transition(.glassLand(scale: 0.9))
                        }
                    }
                    .animation(.spring(response: 0.4, dampingFraction: 0.8), value: shown)
                    .padding(.horizontal, 12)
                    .padding(.bottom, 12)
                }
                .scrollIndicators(.automatic)
            }
        }
        .onAppear(perform: reload)
        .onChange(of: model.capturesVersion) { reload() }
    }

    private func reload() {
        let root = model.captureRoot
        Task.detached {
            let found = listCaptures(in: root)
            await MainActor.run { files = found }
        }
    }
}

private struct EmptyGallery: View {
    let model: AppModel
    let filtered: Bool

    var body: some View {
        VStack(spacing: 4) {
            Spacer(minLength: 0)
            Image(systemName: filtered ? "line.3.horizontal.decrease.circle" : "camera.viewfinder")
                .font(.system(size: 22, weight: .light))
                .foregroundStyle(.secondary)
            Text(filtered ? String(localized: "Nothing of this kind yet") : String(localized: "No captures yet"))
                .font(.system(size: 13, weight: .semibold))
                .padding(.top, 2)
            Text("Saved to \(homeRelative(model.paths.capture_root))")
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.head)
            if !filtered {
                VStack(spacing: 3) {
                    ForEach([HotkeyCommand.region, .window, .video], id: \.self) { command in
                        HStack {
                            Text(command.name).font(.system(size: 11)).foregroundStyle(.secondary)
                            Spacer()
                            Text(Hotkeys.shared.label(command))
                                .font(.system(size: 10.5, weight: .medium, design: .rounded))
                                .padding(.horizontal, 6)
                                .frame(height: 18)
                                .background(.white.opacity(0.08), in: .capsule)
                        }
                    }
                }
                .frame(maxWidth: 220)
                .padding(.top, 6)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .glassEffect(.regular, in: .rect(cornerRadius: 12))
        .padding(.horizontal, 12)
        .padding(.bottom, 12)
    }
}

private struct Tile: View {
    let file: CaptureFile
    let model: AppModel
    let changed: () -> Void
    @Local private var thumbnail: NSImage? = nil
    @Local private var hovering: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ZStack {
                Rectangle().fill(.white.opacity(0.05))
                if let thumbnail {
                    Image(nsImage: thumbnail)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                }
                // Always in the view tree, shown by opacity. Inserting and
                // removing buttons on every hover makes SwiftUI rebuild its
                // key-view loop each time - with Full Keyboard Access on, the
                // loop behind the app's freezes.
                ZStack {
                    Rectangle().fill(.black.opacity(0.35))
                    HStack(spacing: 4) {
                        action("doc.on.doc", String(localized: "Copy")) { try? Core.copyFile(file.url) }
                        action("folder", String(localized: "Show in Finder")) { model.reveal(file.url) }
                        action("trash", String(localized: "Move to Trash")) {
                            try? FileManager.default.trashItem(at: file.url, resultingItemURL: nil)
                            changed()
                        }
                    }
                }
                .opacity(hovering ? 1 : 0)
                .allowsHitTesting(hovering)
                if !hovering && file.kind != .images {
                    VStack {
                        HStack {
                            Text(file.kind == .gif ? "GIF" : "VIDEO")
                                .font(.system(size: 8, weight: .bold))
                                .padding(.horizontal, 5)
                                .frame(height: 14)
                                .glassEffect(.regular, in: .capsule)
                            Spacer()
                        }
                        Spacer()
                    }
                    .padding(5)
                }
            }
            .frame(height: 92)
            .clipped()

            VStack(alignment: .leading, spacing: 2) {
                Text(file.url.lastPathComponent)
                    .font(.system(size: 11.5, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(file.date, format: .relative(presentation: .named))
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
        }
        .clipShape(.rect(cornerRadius: 12))
        .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 12))
        .scaleEffect(hovering ? 1.03 : 1)
        .shadow(color: .black.opacity(hovering ? 0.22 : 0), radius: 12, y: 5)
        .zIndex(hovering ? 1 : 0)
        .onHover { inside in
            withAnimation(.spring(response: 0.3, dampingFraction: 0.68)) { hovering = inside }
        }
        .onTapGesture(count: 2) { NSWorkspace.shared.open(file.url) }
        .help(file.url.lastPathComponent)
        .task(id: file.url) { thumbnail = await makeThumbnail(file.url) }
        .animation(.easeOut(duration: 0.12), value: hovering)
    }

    private func action(_ symbol: String, _ help: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 28, height: 28)
                .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
        .glassEffect(.regular.interactive(), in: .circle)
        .help(help)
    }
}

private func makeThumbnail(_ url: URL) async -> NSImage? {
    let request = QLThumbnailGenerator.Request(
        fileAt: url, size: CGSize(width: 220, height: 140), scale: 2, representationTypes: .thumbnail
    )
    guard let thumbnail = try? await QLThumbnailGenerator.shared.generateBestRepresentation(for: request) else {
        return nil
    }
    return thumbnail.nsImage
}
