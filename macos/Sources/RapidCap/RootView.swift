// One window's content. The window itself is glass (see `GlassWindow`); this
// lays out what sits on it: a title row level with the traffic lights, then
// the screen. Panel, Settings and Recent captures are separate windows.

import SwiftUI

struct WindowRoot: View {
    let screen: Screen
    let model: AppModel
    let metrics: MetricsBox

    var body: some View {
        VStack(spacing: 0) {
            titleRow
            Group {
                switch screen {
                case .panel: PanelView(model: model)
                case .settings: SettingsView(model: model)
                case .gallery: GalleryView(model: model)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .ignoresSafeArea()
    }

    private var titleRow: some View {
        HStack(spacing: 6) {
            Text(title)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 6)
            switch screen {
            case .panel:
                IconButton(symbol: "square.grid.2x2", help: String(localized: "Recent captures")) {
                    model.open(.gallery)
                }
                IconButton(symbol: "gearshape", help: String(localized: "Settings")) { model.open(.settings) }
            case .gallery:
                IconButton(symbol: "folder", help: String(localized: "Open captures folder")) { model.openOutput() }
            case .settings:
                EmptyView()
            }
        }
        .padding(.leading, metrics.value.leading)
        .padding(.trailing, 10)
        // The row's middle sits on the traffic lights' middle.
        .frame(height: metrics.value.centerFromTop * 2)
        .frame(maxWidth: .infinity)
    }

    private var title: String {
        switch screen {
        case .panel: "RapidCap"
        case .settings: String(localized: "Settings")
        case .gallery: String(localized: "Recent captures")
        }
    }
}

/// Arrive slightly small and blurred and settle into focus, the way a glass
/// pane does when it lands; leave the same way.
struct GlassLand: Transition {
    var scale: CGFloat = 0.96

    func body(content: Content, phase: TransitionPhase) -> some View {
        content
            .blur(radius: phase.isIdentity ? 0 : 10)
            .scaleEffect(phase.isIdentity ? 1 : scale, anchor: .top)
            .opacity(phase.isIdentity ? 1 : 0)
    }
}

extension Transition where Self == GlassLand {
    static var glassLand: GlassLand { GlassLand() }
    static func glassLand(scale: CGFloat) -> GlassLand { GlassLand(scale: scale) }
}

/// A small round glass button: the title row's actions.
struct IconButton: View {
    let symbol: String
    let help: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 22, height: 22)
                .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .circle)
        .focusEffectDisabled()
        .help(help)
        .accessibilityLabel(help)
    }
}

/// A segmented control whose selection is a drop of tinted glass that slides
/// and stretches between the options.
struct LiquidSegmented<Value: Hashable>: View {
    let options: [(value: Value, label: String)]
    @Binding var selection: Value
    var height: CGFloat = 22
    var font: Font = .system(size: 11, weight: .semibold)
    @Namespace private var drop

    var body: some View {
        HStack(spacing: 2) {
                ForEach(options, id: \.value) { option in
                    let selected = option.value == selection
                    Button {
                        withAnimation(.spring(response: 0.34, dampingFraction: 0.72)) { selection = option.value }
                    } label: {
                        Text(option.label)
                            .font(font)
                            .foregroundStyle(selected ? Color.white : .secondary)
                            .padding(.horizontal, 10)
                            .frame(height: height)
                            .contentShape(.capsule)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .background {
                        if selected {
                            Capsule()
                                .fill(.clear)
                                .glassEffect(.regular.tint(.accentColor), in: .capsule)
                                .matchedGeometryEffect(id: "selection", in: drop)
                        }
                    }
                }
            }
        .padding(2)
        .glassEffect(.regular, in: .capsule)
        .fixedSize()
    }
}

/// `@State` without the macro. On the macOS 27 SDK `@State` is a macro whose
/// plugin ships only with Xcode; this builds with the Command Line Tools, and
/// SwiftUI finds the `State` inside it like any other dynamic property.
@propertyWrapper
struct Local<Value>: DynamicProperty {
    private let state: State<Value>

    init(wrappedValue: Value) { state = State(initialValue: wrappedValue) }

    var wrappedValue: Value {
        get { state.wrappedValue }
        nonmutating set { state.wrappedValue = newValue }
    }

    var projectedValue: Binding<Value> { state.projectedValue }
}
