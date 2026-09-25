import SwiftUI
import HiFiCore

// Sidebar — Cosmos shell plane (#0d0d0d dark / #f3f3f5 light) with hairline
// separators, Solar icons, space-accent focused rows and frosted bottom bar.

struct SidebarView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var theme: HiFiTheme

    init(store: BrowserStore) {
        self.store = store
        self.theme = store.theme
    }

    var body: some View {
        let p = theme.palette
        VStack(spacing: 0) {
            trafficLightSpacer
            ScrollView {
                VStack(alignment: .leading, spacing: HFSpace.sm) {
                    pinnedRow
                    todaySection
                    ForEach(store.activeSpace.groups.filter { !$0.pinnedSection && !$0.name.isEmpty }) { g in
                        groupSection(g)
                    }
                    newGroupRow
                }
                .padding(.horizontal, HFSpace.sm)
                .padding(.top, HFSpace.xs)
            }
            Divider().overlay(p.sBorder)
            spaceBar
        }
        .background(p.sShell)
    }

    // MARK: header (under traffic lights)

    private var trafficLightSpacer: some View {
        let p = theme.palette
        return HStack(spacing: HFSpace.xs) {
            Color.clear.frame(width: 64) // traffic lights live here
            Text(store.activeSpace.name)
                .font(.system(size: 11.5, weight: .semibold))
                .foregroundStyle(p.sFaint)
                .lineLimit(1)
            Spacer()
            SidebarIconButton(icon: "command", fallback: "command", tip: "Command bar (⌘T)", theme: theme) {
                store.commandBarVisible = true
            }
            SidebarIconButton(icon: "plus", fallback: "plus", tip: "New tab (⌘T · URL)", theme: theme) {
                _ = store.openTab("hifi://newtab")
            }
        }
        .padding(.horizontal, HFSpace.sm)
        .frame(height: 38)
    }

    // MARK: pinned strip

    private var pinnedRow: some View {
        let p = theme.palette
        let pinned = store.activeSpace.groups[0].tabs
        return Group {
            if !pinned.isEmpty {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 34), spacing: HFSpace.xs)], spacing: HFSpace.xs) {
                    ForEach(pinned) { t in
                        PinnedCell(tab: t, store: store)
                    }
                }
                .padding(.bottom, HFSpace.xs)
                Rectangle().fill(p.sBorder).frame(height: 1)
            }
        }
    }

    // MARK: today / unnamed

    private var todaySection: some View {
        Group {
            if let g = store.activeSpace.groups.first(where: { !$0.pinnedSection && $0.name.isEmpty }) ?? store.activeSpace.groups.dropFirst().first {
                ForEach(g.tabs) { t in
                    TabRow(tab: t, group: g, store: store)
                }
            }
        }
    }

    // MARK: named groups

    private func groupSection(_ g: HiFiCore.Group) -> some View {
        let p = theme.palette
        let accent = g.id == store.activeSpace.activeGroupID ? p.sAccent : p.sFaint
        return VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: HFSpace.xs) {
                Circle().fill(accent).frame(width: 5, height: 5)
                Text(g.name.uppercased())
                    .font(.system(size: 10, weight: .semibold))
                    .tracking(0.6)
                    .foregroundStyle(g.id == store.activeSpace.activeGroupID ? p.sMuted : p.sFaint)
                Spacer()
                Text("\(g.tabs.count)")
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundStyle(p.sFaint.opacity(0.8))
            }
            .padding(.top, HFSpace.xs)
            .padding(.bottom, 2)
            .contextMenu {
                Button("Rename…") { renameGroup(g) }
                Divider()
                Button("Delete group", role: .destructive) { store.deleteGroup(id: g.id) }
            }
            ForEach(g.tabs) { t in
                TabRow(tab: t, group: g, store: store)
            }
        }
    }

    private var newGroupRow: some View {
        let p = theme.palette
        return Button {
            _ = store.createGroup(name: "New group")
        } label: {
            HStack(spacing: HFSpace.xs) {
                HFIconView(name: "add-circle", fallback: "plus.circle", size: 13, color: p.sFaint)
                Text("New group")
                    .font(.system(size: 11.5))
                    .foregroundStyle(p.sFaint)
                Spacer()
            }
            .padding(.horizontal, HFSpace.sm)
            .padding(.vertical, 5)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(0.85)
    }

    // MARK: space switcher (bottom bar)

    private var spaceBar: some View {
        let p = theme.palette
        return HStack(spacing: HFSpace.xs) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: HFSpace.xs) {
                    ForEach(store.state.spaces) { s in
                        SpacePill(space: s, active: s.id == store.activeSpace.id, palette: p) {
                            _ = store.switchSpace(s.id)
                        }
                    }
                    Button {
                        let name = promptText(title: "New space", placeholder: "Space name") ?? ""
                        if !name.isEmpty { _ = store.createSpace(name: name) }
                    } label: {
                        HFIconView(name: "plus", fallback: "plus", size: 11, color: p.sFaint)
                            .frame(width: 20, height: 20)
                            .background(p.sHover, in: RoundedRectangle(cornerRadius: HFRadius.control))
                    }
                    .buttonStyle(.plain)
                    .help("New space")
                }
            }
            Spacer(minLength: 0)
            DownloadsButton(store: store)
            SidebarIconButton(icon: "settings-minimalistic", fallback: "gearshape", tip: "Settings", theme: theme) {
                _ = store.openTab("hifi://settings")
            }
        }
        .padding(.horizontal, HFSpace.sm)
        .padding(.vertical, 7)
        .background(p.sShell)
    }

    // MARK: helpers

    private func renameGroup(_ g: HiFiCore.Group) {
        if let name = promptText(title: "Rename group", placeholder: g.name), !name.isEmpty {
            var g2 = g; g2.name = name
            store.applyGroup(g2)
        }
    }

    private func promptText(title: String, placeholder: String) -> String? {
        let a = NSAlert()
        a.messageText = title
        let f = NSTextField(frame: NSRect(x: 0, y: 0, width: 240, height: 24))
        f.placeholderString = placeholder
        a.accessoryView = f
        a.addButton(withTitle: "OK"); a.addButton(withTitle: "Cancel")
        return a.runModal() == .alertFirstButtonReturn ? f.stringValue : nil
    }
}

// MARK: - pieces

private struct SidebarIconButton: View {
    let icon: String
    var fallback: String = "questionmark"
    var tip: String = ""
    @ObservedObject var theme: HiFiTheme
    let action: () -> Void
    @State private var hovering = false

    init(icon: String, fallback: String = "questionmark", tip: String = "", theme: HiFiTheme, action: @escaping () -> Void) {
        self.icon = icon; self.fallback = fallback; self.tip = tip; self.theme = theme; self.action = action
    }

    var body: some View {
        let p = theme.palette
        Button(action: action) {
            HFIconView(name: icon, fallback: fallback, size: 13, color: hovering ? p.sText : p.sMuted)
                .frame(width: 22, height: 22)
                .background(hovering ? p.sHover : Color.clear,
                            in: RoundedRectangle(cornerRadius: HFRadius.control))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(tip)
    }
}

private struct PinnedCell: View {
    let tab: HiFiCore.Tab
    @ObservedObject var store: BrowserStore
    @ObservedObject var theme: HiFiTheme
    @State private var hovering = false

    init(tab: HiFiCore.Tab, store: BrowserStore) {
        self.tab = tab; self.store = store; self.theme = store.theme
    }

    var body: some View {
        let p = theme.palette
        let focused = store.focusedTabID == tab.id
        Button {
            store.focusTab(tab.id)
        } label: {
            ZStack {
                RoundedRectangle(cornerRadius: HFRadius.panel)
                    .fill(focused ? p.sAccentWash : (hovering ? p.sHover : p.sCard))
                    .overlay(
                        RoundedRectangle(cornerRadius: HFRadius.panel)
                            .strokeBorder(p.sBorder, lineWidth: focused ? 0 : 1)
                    )
                favicon
            }
            .frame(width: 34, height: 34)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(tab.title.isEmpty ? tab.url : tab.title)
        .contextMenu {
            Button("Unpin") { store.togglePin(tab.id) }
            Button("Close", role: .destructive) { store.closeTab(tab.id) }
        }
    }

    private var favicon: some View {
        let p = theme.palette
        return Group {
            if let img = FaviconCache.shared.image(for: tab.url) {
                Image(nsImage: img).resizable().frame(width: 16, height: 16)
            } else {
                HFIconView(name: "globe", fallback: "globe", size: 15, color: p.sMuted)
            }
        }
    }
}

private struct TabRow: View {
    let tab: HiFiCore.Tab
    let group: HiFiCore.Group
    @ObservedObject var store: BrowserStore
    @ObservedObject var theme: HiFiTheme
    @State private var hovering = false

    init(tab: HiFiCore.Tab, group: HiFiCore.Group, store: BrowserStore) {
        self.tab = tab; self.group = group; self.store = store; self.theme = store.theme
    }

    var body: some View {
        let p = theme.palette
        let focused = store.focusedTabID == tab.id
        Button {
            store.focusTab(tab.id)
        } label: {
            HStack(spacing: 7) {
                leadingIcon
                // Radius-style "Kind/Title" for pane tabs, plain title for web
                Text(displayTitle)
                    .font(.system(size: 12))
                    .foregroundStyle(focused ? p.sText : p.sMuted)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 4)
                if hovering {
                    Button {
                        store.closeTab(tab.id)
                    } label: {
                        HFIconView(name: "close", fallback: "xmark", size: 10,
                                   color: p.sFaint)
                            .frame(width: 16, height: 16)
                    }
                    .buttonStyle(.plain)
                } else {
                    Circle()
                        .fill(focused ? p.sAccent : Color.clear)
                        .frame(width: 5, height: 5)
                }
            }
            .padding(.horizontal, HFSpace.sm)
            .padding(.vertical, 5)
            .background(
                RoundedRectangle(cornerRadius: HFRadius.control)
                    .fill(focused ? p.sAccentWash : (hovering ? p.sHover : Color.clear))
            )
            .overlay(alignment: .leading) {
                if focused {
                    RoundedRectangle(cornerRadius: 2)
                        .fill(p.sAccent)
                        .frame(width: 2.5, height: 14)
                        .offset(x: -2)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .contextMenu {
            Button(tab.pinned ? "Unpin" : "Pin") { store.togglePin(tab.id) }
            Divider()
            Button("Split right") { _ = store.splitTab(anchorID: tab.id, side: .right) }
            Button("Split down") { _ = store.splitTab(anchorID: tab.id, side: .below) }
            Divider()
            Button("Close", role: .destructive) { store.closeTab(tab.id) }
        }
    }

    /// "Agent/repo", "Terminal/zsh", "Diff/Hi-Fi" — kind prefix + live title.
    private var displayTitle: String {
        let title = tab.title
        switch tab.kind {
        case .web:
            return title.isEmpty ? tab.url : title
        case .newtab, .settings:
            return title
        case .terminal, .agent, .diff, .preview:
            let kind = tab.kind.rawValue.capitalized
            return (title.isEmpty || title == kind) ? kind : "\(kind)/\(title)"
        }
    }

    private var leadingIcon: some View {
        let p = theme.palette
        switch tab.kind {
        case .web:
            if let img = FaviconCache.shared.image(for: tab.url) {
                return AnyView(Image(nsImage: img).resizable().frame(width: 14, height: 14))
            }
            return AnyView(HFIconView(name: "globe", fallback: "globe", size: 13, color: p.sFaint))
        case .terminal:
            return AnyView(HFIconView(name: "terminal", fallback: "terminal", size: 13, color: p.sFaint))
        case .agent:
            return AnyView(HFIconView(name: "bot", fallback: "sparkles", size: 13, color: p.sFaint))
        case .diff:
            return AnyView(HFIconView(name: "git-branch", fallback: "arrow.triangle.branch", size: 13, color: p.sFaint))
        case .newtab:
            return AnyView(HFIconView(name: "home", fallback: "house", size: 13, color: p.sFaint))
        case .settings:
            return AnyView(HFIconView(name: "tuning", fallback: "slider.horizontal.3", size: 13, color: p.sFaint))
        case .preview:
            return AnyView(HFIconView(name: "eye", fallback: "eye", size: 13, color: p.sFaint))
        }
    }
}

private struct SpacePill: View {
    let space: HiFiCore.Space
    let active: Bool
    let palette: HFPalette
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        let p = palette
        let spaceAccent = HF.rgb(hex: space.accent) ?? p.accent
        Button(action: action) {
            HStack(spacing: 5) {
                Circle().fill(Color(nsColor: spaceAccent)).frame(width: 5, height: 5)
                Text(space.name)
                    .font(.system(size: 11, weight: active ? .semibold : .regular))
                    .foregroundStyle(active ? p.sText : p.sMuted)
            }
            .padding(.horizontal, HFSpace.sm)
            .padding(.vertical, 4)
            .background(
                active ? p.sActive : (hovering ? p.sHover : Color.clear),
                in: RoundedRectangle(cornerRadius: HFRadius.control)
            )
            .overlay(
                RoundedRectangle(cornerRadius: HFRadius.control)
                    .strokeBorder(active ? Color(nsColor: spaceAccent).opacity(0.35) : Color.clear, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

private struct DownloadsButton: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var theme: HiFiTheme
    @State private var hovering = false

    init(store: BrowserStore) {
        self.store = store; self.theme = store.theme
    }

    var body: some View {
        let p = theme.palette
        Menu {
            if store.downloads.isEmpty {
                Text("No downloads")
            } else {
                ForEach(store.downloads, id: \.id) { d in
                    Text(d.done ? "\(d.filename)" : "\(d.filename) — \(Int(d.progress * 100))%")
                }
            }
        } label: {
            HFIconView(name: "arrow-down", fallback: "arrow.down.circle", size: 13,
                       color: hovering ? p.sText : p.sMuted)
                .frame(width: 22, height: 22)
                .background(hovering ? p.sHover : Color.clear,
                            in: RoundedRectangle(cornerRadius: HFRadius.control))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .frame(width: 22)
        .onHover { hovering = $0 }
        .help("Downloads")
    }
}

extension Color {
    init(hex: String) {
        let ns = HF.rgb(hex: hex) ?? .labelColor
        self.init(nsColor: ns)
    }
}
