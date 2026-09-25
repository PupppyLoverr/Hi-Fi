import SwiftUI
import AppKit
import Foundation
import UniformTypeIdentifiers
import HiFiCore

// MARK: - hifi://newtab — Cosmos-style start page

final class NewTabPaneView: PaneHostView {
    init(tab: HiFiCore.Tab, store: BrowserStore) {
        super.init(frame: .zero)
        let host = NSHostingView(rootView: NewTabView(store: store))
        host.translatesAutoresizingMaskIntoConstraints = false
        addSubview(host)
        NSLayoutConstraint.activate([
            host.leadingAnchor.constraint(equalTo: leadingAnchor),
            host.trailingAnchor.constraint(equalTo: trailingAnchor),
            host.topAnchor.constraint(equalTo: topAnchor),
            host.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError() }
}

struct NewTabView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var theme: HiFiTheme
    @State private var query = ""
    @State private var clock = Date()
    @State private var bgImage: NSImage?
    private let ticker = Timer.publish(every: 30, on: .main, in: .common).autoconnect()

    init(store: BrowserStore) {
        self.store = store; self.theme = store.theme
    }

    var body: some View {
        let p = theme.palette
        ZStack {
            backgroundLayer(p)

            VStack(spacing: 0) {
                Spacer(minLength: 0).frame(height: 90)

                // wordmark — Hi-Fi identity (app icon), not the cosmos mark
                VStack(spacing: 8) {
                    if let logo = NSImage(named: "AppIcon") {
                        Image(nsImage: logo).resizable()
                            .frame(width: 52, height: 52)
                            .clipShape(RoundedRectangle(cornerRadius: 12))
                            .shadow(color: .black.opacity(0.3), radius: 10, y: 3)
                    } else {
                        Text("Hi-Fi")
                            .font(.system(size: 34, weight: .bold, design: .rounded))
                            .foregroundStyle(p.sText)
                    }
                    Text(clock, style: .time)
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(p.sFaint)
                }

                // search
                HStack(spacing: 10) {
                    HFIconView(name: "magnifer", fallback: "magnifyingglass",
                               size: 15, color: p.sFaint)
                    TextField("Search or type an address", text: $query)
                        .textFieldStyle(.plain)
                        .font(.system(size: 15))
                        .foregroundStyle(p.sText)
                        .onSubmit { open(query) }
                    if !query.isEmpty {
                        Button { query = "" } label: {
                            HFIconView(name: "close", fallback: "xmark", size: 11, color: p.sFaint)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, HFSpace.lg)
                .frame(maxWidth: 520, maxHeight: 46)
                .background(cardBackground(p))
                .padding(.top, 34)

                // quick actions
                HStack(spacing: HFSpace.sm) {
                    ActionChip(icon: "terminal", sf: "terminal", title: "Terminal", p: p) {
                        _ = store.openTab("hifi://terminal")
                    }
                    ActionChip(icon: "bot", sf: "sparkles", title: "Agent", p: p) {
                        _ = store.openTab("hifi://agent")
                    }
                    ActionChip(icon: "git-branch", sf: "arrow.triangle.branch", title: "Diff", p: p) {
                        _ = store.openTab("hifi://diff")
                    }
                    ActionChip(icon: "split-columns", sf: "rectangle.split.2x1", title: "Split", p: p) {
                        if let id = store.focusedTabID { _ = store.splitTab(anchorID: id, side: .right) }
                    }
                    ActionChip(icon: "tuning", sf: "slider.horizontal.3", title: "Settings", p: p) {
                        _ = store.openTab("hifi://settings")
                    }
                }
                .padding(.top, HFSpace.md)

                // recents
                recents(p)

                Spacer(minLength: 0)

                hints(p)
            }
            .padding(.horizontal, 40)
        }
        .onReceive(ticker) { clock = $0 }
        .onAppear { loadBackground() }
        .onChange(of: store.settings.backgroundImage) { _ in loadBackground() }
    }

    // MARK: layers

    private func backgroundLayer(_ p: HFPalette) -> some View {
        ZStack {
            p.sBackground
            if let bgImage {
                Image(nsImage: bgImage).resizable().aspectRatio(contentMode: .fill)
                    .blur(radius: CGFloat(store.settings.backgroundBlur) / 10)
                    .opacity(0.9)
                    .overlay(p.sBackground.opacity(Double(store.settings.backgroundDim) / 100))
                    .clipped()
            } else {
                // subtle accent aurora, same language as cosmos's quiet backdrops
                RadialGradient(colors: [p.sAccent.opacity(p.isDark ? 0.10 : 0.06), .clear],
                               center: .top, startRadius: 40, endRadius: 520)
                RadialGradient(colors: [p.sAccent.opacity(p.isDark ? 0.05 : 0.03), .clear],
                               center: .bottomTrailing, startRadius: 20, endRadius: 480)
            }
        }
        .ignoresSafeArea()
    }

    private func cardBackground(_ p: HFPalette) -> some View {
        RoundedRectangle(cornerRadius: HFRadius.bubble)
            .fill(p.sCard.opacity(bgImage != nil ? 0.72 : 1))
            .background(
                RoundedRectangle(cornerRadius: HFRadius.bubble)
                    .fill(.regularMaterial.opacity(0.5))
            )
            .overlay(
                RoundedRectangle(cornerRadius: HFRadius.bubble)
                    .strokeBorder(p.sBorder, lineWidth: 1)
            )
            .shadow(color: .black.opacity(p.isDark ? 0.4 : 0.08), radius: 18, y: 6)
    }

    private func recents(_ p: HFPalette) -> some View {
        let entries = store.historyEntries(matching: "", limit: 6)
        return Group {
            if !entries.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text("RECENT")
                        .font(.system(size: 9.5, weight: .semibold))
                        .tracking(0.8)
                        .foregroundStyle(p.sFaint)
                        .padding(.bottom, 5)
                    ForEach(Array(entries.enumerated()), id: \.offset) { _, e in
                        Button { open(e.url) } label: {
                            HStack(spacing: 9) {
                                if let img = FaviconCache.shared.image(for: e.url) {
                                    Image(nsImage: img).resizable().frame(width: 13, height: 13)
                                } else {
                                    HFIconView(name: "clock-circle", fallback: "clock",
                                               size: 12, color: p.sFaint)
                                }
                                Text(e.title.isEmpty ? e.url : e.title)
                                    .font(.system(size: 12))
                                    .foregroundStyle(p.sMuted).lineLimit(1)
                                Spacer()
                                Text(host(of: e.url))
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundStyle(p.sFaint.opacity(0.75)).lineLimit(1)
                            }
                            .padding(.horizontal, 10).padding(.vertical, 6)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(HoverFill(p: p))
                    }
                }
                .padding(12)
                .frame(maxWidth: 520)
                .background(cardBackground(p))
                .padding(.top, HFSpace.lg)
            }
        }
    }

    private func hints(_ p: HFPalette) -> some View {
        HStack(spacing: 14) {
            kbd(p, "⌘T", "command")
            kbd(p, "⌘L", "address")
            kbd(p, "⌘\\", "sidebar")
            kbd(p, "⌘⏎", "split")
            kbd(p, "⌃⇥", "cycle")
        }
        .padding(.bottom, 22)
    }

    private func kbd(_ p: HFPalette, _ key: String, _ label: String) -> some View {
        HStack(spacing: 4) {
            Text(key)
                .font(.system(size: 9.5, weight: .medium, design: .monospaced))
                .foregroundStyle(p.sMuted)
                .padding(.horizontal, 5).padding(.vertical, 2.5)
                .background(p.sHover, in: RoundedRectangle(cornerRadius: 4))
                .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(p.sBorder, lineWidth: 0.5))
            Text(label)
                .font(.system(size: 9.5))
                .foregroundStyle(p.sFaint)
        }
    }

    private func host(of url: String) -> String {
        URL(string: url)?.host ?? url
    }

    private func open(_ q: String) {
        _ = store.openTab(q.isEmpty ? "hifi://newtab" : q)
        query = ""
    }

    private func loadBackground() {
        guard let path = store.settings.backgroundImage else { bgImage = nil; return }
        bgImage = NSImage(contentsOfFile: path)
    }
}

private struct HoverFill: ButtonStyle {
    let p: HFPalette
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(
                RoundedRectangle(cornerRadius: HFRadius.control)
                    .fill(configuration.isPressed ? p.sActive : Color.clear)
            )
    }
}

private struct ActionChip: View {
    let icon: String
    let sf: String
    let title: String
    let p: HFPalette
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            VStack(spacing: 5) {
                HFIconView(name: icon, fallback: sf, size: 15,
                           color: hovering ? p.sAccent : p.sMuted)
                Text(title)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(hovering ? p.sText : p.sMuted)
            }
            .frame(width: 64, height: 50)
            .background(
                RoundedRectangle(cornerRadius: HFRadius.panel)
                    .fill(hovering ? p.sAccentWash : p.sCard.opacity(0.85))
            )
            .overlay(
                RoundedRectangle(cornerRadius: HFRadius.panel)
                    .strokeBorder(hovering ? p.sAccent.opacity(0.4) : p.sBorder, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

// MARK: - hifi://settings — real settings page

final class SettingsPaneView: PaneHostView {
    init(tab: HiFiCore.Tab, store: BrowserStore) {
        super.init(frame: .zero)
        let host = NSHostingView(rootView: SettingsView(store: store))
        host.translatesAutoresizingMaskIntoConstraints = false
        addSubview(host)
        NSLayoutConstraint.activate([
            host.leadingAnchor.constraint(equalTo: leadingAnchor),
            host.trailingAnchor.constraint(equalTo: trailingAnchor),
            host.topAnchor.constraint(equalTo: topAnchor),
            host.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError() }
}

struct SettingsView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var theme: HiFiTheme

    init(store: BrowserStore) {
        self.store = store; self.theme = store.theme
    }

    var body: some View {
        let p = theme.palette
        ScrollView {
            VStack(alignment: .leading, spacing: HFSpace.lg) {
                header(p)
                appearanceSection(p)
                backgroundSection(p)
                sidebarSection(p)
                spacesSection(p)
                dataSection(p)
                cliSection(p)
                privacySection(p)
            }
            .padding(28)
            .frame(maxWidth: 560)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(p.sBackground)
    }

    // MARK: sections

    private func header(_ p: HFPalette) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("Settings")
                .font(.system(size: 22, weight: .semibold, design: .rounded))
                .foregroundStyle(p.sText)
            Text("Hi-Fi preferences — stored locally, synced nowhere.")
                .font(.system(size: 12))
                .foregroundStyle(p.sFaint)
        }
        .padding(.bottom, HFSpace.xs)
    }

    private func sectionCard(_ p: HFPalette, title: String, icon: String, sfIcon: String,
                             @ViewBuilder _ content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                HFIconView(name: icon, fallback: sfIcon, size: 13, color: p.sAccent)
                Text(title.uppercased())
                    .font(.system(size: 10.5, weight: .semibold)).tracking(0.7)
                    .foregroundStyle(p.sMuted)
            }
            content()
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: HFRadius.bubble)
                .fill(p.sCard)
        )
        .overlay(
            RoundedRectangle(cornerRadius: HFRadius.bubble)
                .strokeBorder(p.sBorder, lineWidth: 1)
        )
    }

    private func appearanceSection(_ p: HFPalette) -> some View {
        sectionCard(p, title: "Appearance", icon: "palette-search", sfIcon: "paintpalette") {
            // theme mode
            HStack(spacing: HFSpace.sm) {
                ForEach([("system", "System"), ("dark", "Dark"), ("light", "Light")], id: \.0) { mode, label in
                    ModeChip(label: label,
                             active: store.settings.appearance == mode, p: p) {
                        store.updateSettings { $0.appearance = mode }
                    }
                }
                Spacer()
            }
            // accent swatches
            Text("Accent").font(.system(size: 11)).foregroundStyle(p.sMuted)
            HStack(spacing: HFSpace.sm) {
                // "space accent" = follow the current space's color
                Swatch(color: HF.rgb(hex: store.activeSpace.accent) ?? p.accent,
                       label: "Space", active: store.settings.accent.isEmpty, p: p) {
                    store.updateSettings { $0.accent = "" }
                }
                ForEach(HF.accentPresets, id: \.id) { preset in
                    Swatch(color: HF.rgb(theme.isDark ? preset.dark : preset.light),
                           label: preset.label,
                           active: store.settings.accent == preset.id, p: p) {
                        store.updateSettings { $0.accent = preset.id }
                    }
                }
            }
        }
    }

    private func backgroundSection(_ p: HFPalette) -> some View {
        sectionCard(p, title: "New tab background", icon: "photo", sfIcon: "photo") {
            HStack(spacing: HFSpace.sm) {
                Button {
                    pickImage()
                } label: {
                    Label(store.settings.backgroundImage == nil ? "Choose image…" : "Change image…",
                          systemImage: "photo")
                        .font(.system(size: 11.5))
                }
                .buttonStyle(.bordered)

                if store.settings.backgroundImage != nil {
                    Button("Remove") { store.updateSettings { $0.backgroundImage = nil } }
                        .buttonStyle(.bordered)
                        .foregroundStyle(p.sDanger)
                }
            }
            if let path = store.settings.backgroundImage {
                Text(path).font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(p.sFaint).lineLimit(1).truncationMode(.middle)
            }
            if store.settings.backgroundImage != nil {
                sliderRow(p, "Blur", value: Double(store.settings.backgroundBlur)) { v in
                    store.updateSettings { $0.backgroundBlur = Int(v) }
                }
                sliderRow(p, "Dim", value: Double(store.settings.backgroundDim)) { v in
                    store.updateSettings { $0.backgroundDim = Int(v) }
                }
            }
        }
    }

    private func sliderRow(_ p: HFPalette, _ label: String, value: Double,
                           onChange: @escaping (Double) -> Void) -> some View {
        HStack {
            Text(label).font(.system(size: 11)).foregroundStyle(p.sMuted).frame(width: 34, alignment: .leading)
            Slider(value: Binding(get: { value }, set: onChange), in: 0...100)
        }
    }

    private func sidebarSection(_ p: HFPalette) -> some View {
        sectionCard(p, title: "Sidebar", icon: "sidebar-minimalistic-left", sfIcon: "sidebar.left") {
            Toggle(isOn: Binding(
                get: { store.sidebarCollapsed },
                set: { _ in store.sidebarCollapsed.toggle() })) {
                Text("Collapsed by default").font(.system(size: 12)).foregroundStyle(p.sText)
            }
            .toggleStyle(.checkbox)
        }
    }

    private func spacesSection(_ p: HFPalette) -> some View {
        sectionCard(p, title: "Spaces", icon: "widget", sfIcon: "square.grid.2x2") {
            ForEach(store.state.spaces) { s in
                HStack(spacing: 7) {
                    Circle().fill(Color(nsColor: HF.rgb(hex: s.accent) ?? p.accent))
                        .frame(width: 6, height: 6)
                    Text(s.name).font(.system(size: 12)).foregroundStyle(p.sText)
                    Spacer()
                    Text("\(s.groups.map { $0.tabs.count }.reduce(0, +)) tabs")
                        .font(.system(size: 10, design: .monospaced)).foregroundStyle(p.sFaint)
                    if s.id == store.activeSpace.id {
                        Text("active").font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(p.sAccent)
                    }
                }
            }
        }
    }

    private func dataSection(_ p: HFPalette) -> some View {
        sectionCard(p, title: "Data", icon: "hard-drive", sfIcon: "internaldrive") {
            kvRow(p, "State", HiFiPaths.stateFile.path)
            kvRow(p, "History", HiFiPaths.historyFile.path)
            kvRow(p, "Socket", HiFiPaths.socketPath)
            kvRow(p, "Profiles", "\(store.state.profiles.count)")
            HStack {
                Spacer()
                Button("Clear history") { store.clearHistory() }
                    .buttonStyle(.bordered)
                    .foregroundStyle(p.sDanger)
            }
        }
    }

    private func cliSection(_ p: HFPalette) -> some View {
        sectionCard(p, title: "CLI", icon: "terminal", sfIcon: "terminal") {
            Text("The `hifi` command line tool lives next to the app binary. Install it onto PATH:")
                .font(.system(size: 11.5)).foregroundStyle(p.sMuted)
            HStack {
                Button("Install hifi → /usr/local/bin") { installCLI() }
                    .buttonStyle(.borderedProminent)
                    .tint(Color(nsColor: p.accent))
                Text(Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/hifi").path)
                    .font(.system(size: 9.5, design: .monospaced))
                    .foregroundStyle(p.sFaint).lineLimit(1).truncationMode(.middle)
            }
        }
    }

    private func privacySection(_ p: HFPalette) -> some View {
        sectionCard(p, title: "Privacy", icon: "shield", sfIcon: "lock.shield") {
            Text("Local-first: state, history, and preferences live only in Application Support. No analytics, no telemetry, no accounts.")
                .font(.system(size: 11.5)).foregroundStyle(p.sMuted)
            Text("Agent browser control is off by default — grant it per group when prompted.")
                .font(.system(size: 11)).foregroundStyle(p.sFaint)
        }
    }

    // MARK: helpers

    private func kvRow(_ p: HFPalette, _ k: String, _ v: String) -> some View {
        HStack {
            Text(k).font(.system(size: 11)).foregroundStyle(p.sMuted).frame(width: 60, alignment: .leading)
            Text(v).font(.system(size: 10, design: .monospaced)).foregroundStyle(p.sFaint)
                .lineLimit(1).truncationMode(.middle)
        }
    }

    private func pickImage() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        if panel.runModal() == .OK, let url = panel.url {
            store.updateSettings { $0.backgroundImage = url.path }
        }
    }

    private func installCLI() {
        let src = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/hifi").path
        let dst = "/usr/local/bin/hifi"
        do {
            if FileManager.default.fileExists(atPath: dst) {
                try FileManager.default.removeItem(atPath: dst)
            }
            try FileManager.default.createSymbolicLink(atPath: dst, withDestinationPath: src)
            store.toast("hifi → /usr/local/bin")
        } catch {
            store.toast("install failed — try: sudo ln -sf \"\(src)\" \(dst)")
        }
    }
}

private struct ModeChip: View {
    let label: String
    let active: Bool
    let p: HFPalette
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.system(size: 11.5, weight: active ? .semibold : .regular))
                .foregroundStyle(active ? p.sText : p.sMuted)
                .padding(.horizontal, 14).padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: HFRadius.panel)
                        .fill(active ? p.sAccentWash : (hovering ? p.sHover : Color.clear))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: HFRadius.panel)
                        .strokeBorder(active ? p.sAccent : p.sBorder, lineWidth: 1)
                )
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

private struct Swatch: View {
    let color: NSColor
    let label: String
    let active: Bool
    let p: HFPalette
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            ZStack {
                Circle().fill(Color(nsColor: color))
                if active {
                    Image(systemName: "checkmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(.white)
                }
            }
            .frame(width: 22, height: 22)
            .overlay(
                Circle().strokeBorder(active ? Color(nsColor: color) : Color.clear,
                                      lineWidth: 2).padding(-3)
            )
            .scaleEffect(hovering ? 1.1 : 1)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(label)
    }
}
