import SwiftUI
import AppKit
import HiFiCore


/// Vertical sidebar: pinned strip, ungrouped tabs, named groups, space bar.
struct SidebarView: View {
    @ObservedObject var store: BrowserStore
    @State private var newSpaceDraft: String?
    @State private var renamingGroup: String?
    @State private var renameText = ""
    @State private var showDownloads = false

    private var space: HiFiCore.Space { store.activeSpace }

    var body: some View {
        VStack(spacing: 0) {
            trafficLightSpacer
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    pinnedSection
                    todaySection
                    ForEach(space.groups.filter { !$0.pinnedSection && !$0.name.isEmpty }) { g in
                        groupSection(g)
                    }
                    newGroupRow
                }
                .padding(.horizontal, 8)
            }
            .scrollIndicators(.hidden)
            Divider()
            spaceBar
        }
        .frame(minWidth: 200)
        .popover(isPresented: $showDownloads) { downloadsPopover }
    }

    /// Room for the traffic lights: window chrome buttons live at top-left.
    private var trafficLightSpacer: some View {
        HStack(spacing: 6) {
            Text(space.name)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.leading, 74) // clear traffic lights
            Spacer()
            Button { store.commandBarMode = .command; store.commandBarVisible.toggle() } label: {
                Image(systemName: "command")
            }
            .buttonStyle(.plain).foregroundStyle(.secondary)
            Button { _ = store.openTab("hifi://newtab") } label: {
                Image(systemName: "plus")
            }
            .buttonStyle(.plain).foregroundStyle(.secondary)
        }
        .padding(.vertical, 8)
        .padding(.trailing, 8)
    }

    // MARK: pinned

    @ViewBuilder private var pinnedSection: some View {
        let pinned = space.groups[0].tabs
        if !pinned.isEmpty {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 36), spacing: 4)]) {
                ForEach(Array(pinned.enumerated()), id: \.element.id) { idx, t in
                    PinnedCell(tab: t, index: idx,
                               accent: accent,
                               focused: isFocused(t))
                        .onTapGesture { store.focusTab(t.id) }
                        .contextMenu { tabMenu(t) }
                }
            }
            .padding(.vertical, 4)
            Divider()
        }
    }

    // MARK: today list

    @ViewBuilder private var todaySection: some View {
        let g = space.groups.first(where: { !$0.pinnedSection && $0.name.isEmpty })
        if let g, !g.tabs.isEmpty {
            ForEach(g.tabs) { t in tabRow(t, group: g) }
        }
    }

    // MARK: named groups

    @ViewBuilder private func groupSection(_ g: HiFiCore.Group) -> some View {
        HStack(spacing: 4) {
            Circle().fill(accent).frame(width: 5, height: 5)
            if renamingGroup == g.id {
                TextField("Name", text: $renameText, onCommit: {
                    store.renameGroup(g.id, name: renameText)
                    renamingGroup = nil
                })
                .textFieldStyle(.plain).font(.system(size: 11, weight: .semibold))
            } else {
                Text(g.name.uppercased())
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .contextMenu {
                        Button("Rename") { renamingGroup = g.id; renameText = g.name }
                        Button("Delete Group") {
                            g.tabs.forEach { store.closeTab($0.id) }
                        }
                    }
                    .onTapGesture(count: 2) {
                        renamingGroup = g.id; renameText = g.name
                    }
            }
            Spacer()
        }
        .padding(.top, 10).padding(.bottom, 2)
        ForEach(g.tabs) { t in tabRow(t, group: g) }
    }

    private var newGroupRow: some View {
        Button {
            _ = store.createGroup(name: "Group \(space.groups.count - 1)")
        } label: {
            Label("New Group", systemImage: "folder.badge.plus")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
        }
        .buttonStyle(.plain)
        .padding(.top, 8)
    }

    // MARK: tab rows

    private func isFocused(_ t: HiFiCore.Tab) -> Bool {
        t.id == (store.focusedTabID ?? space.groups.first(where: {
            $0.id == space.activeGroupID
        })?.focusedTabID)
    }

    private func tabRow(_ t: HiFiCore.Tab, group g: HiFiCore.Group) -> some View {
        HStack(spacing: 7) {
            tabIcon(t)
            Text(t.title.isEmpty ? t.url : t.title)
                .font(.system(size: 12))
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 4)
            if isFocused(t) {
                Circle().fill(accent).frame(width: 5, height: 5)
            }
            Button { store.closeTab(t.id) } label: {
                Image(systemName: "xmark").font(.system(size: 8, weight: .bold))
            }
            .buttonStyle(.plain)
            .foregroundStyle(.tertiary)
            .opacity(0.7)
        }
        .padding(.horizontal, 7).padding(.vertical, 5)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isFocused(t) ? accent.opacity(0.18) : Color.clear)
        )
        .contentShape(Rectangle())
        .onTapGesture { store.focusTab(t.id) }
        .contextMenu { tabMenu(t) }
    }

    @ViewBuilder private func tabIcon(_ t: HiFiCore.Tab) -> some View {
        switch t.kind {
        case .web, .preview:
            Text(String((t.title.isEmpty ? t.url : t.title).prefix(1)).uppercased())
                .font(.system(size: 9, weight: .semibold))
                .frame(width: 16, height: 16)
                .background(RoundedRectangle(cornerRadius: 4).fill(accent.opacity(0.25)))
        case .terminal: Image(systemName: "terminal").font(.system(size: 11))
        case .agent:    Image(systemName: "sparkles").font(.system(size: 11))
        case .diff:     Image(systemName: "doc.text.magnifyingglass").font(.system(size: 11))
        case .newtab:   Image(systemName: "plus.square").font(.system(size: 11))
        case .settings: Image(systemName: "gear").font(.system(size: 11))
        }
    }

    @ViewBuilder private func tabMenu(_ t: HiFiCore.Tab) -> some View {
        Button(t.pinned ? "Unpin" : "Pin") { store.togglePin(t.id) }
        Button("Split Right") { _ = store.splitTab(anchorID: t.id, side: .right) }
        Button("Split Down")  { _ = store.splitTab(anchorID: t.id, side: .below) }
        Divider()
        Button("Close") { store.closeTab(t.id) }
    }

    // MARK: space bar

    private var spaceBar: some View {
        HStack(spacing: 6) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(store.state.spaces) { s in
                        Button { _ = store.switchSpace(s.id) } label: {
                            HStack(spacing: 4) {
                                Circle().fill(Color(hex: s.accent) ?? .accentColor)
                                    .frame(width: 7, height: 7)
                                Text(s.name).font(.system(size: 11))
                            }
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .background(
                                Capsule().fill(s.id == space.id
                                               ? Color(hex: s.accent)?.opacity(0.2) ?? .clear
                                               : .clear)
                            )
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            Button {
                newSpaceDraft = ""
            } label: { Image(systemName: "plus") }
                .buttonStyle(.plain).foregroundStyle(.secondary)
                .popover(isPresented: Binding(
                    get: { newSpaceDraft != nil },
                    set: { if !$0 { newSpaceDraft = nil } })) {
                    HStack {
                        TextField("Space name", text: Binding(
                            get: { newSpaceDraft ?? "" },
                            set: { newSpaceDraft = $0 }),
                            onCommit: {
                                if let n = newSpaceDraft, !n.isEmpty {
                                    _ = store.createSpace(name: n)
                                }
                                newSpaceDraft = nil
                            })
                        .frame(width: 140)
                    }.padding(10)
                }
            Button { showDownloads.toggle() } label: {
                Image(systemName: "arrow.down.circle")
            }
            .buttonStyle(.plain).foregroundStyle(.secondary)
            Button { _ = store.openTab("hifi://settings") } label: {
                Image(systemName: "gear")
            }
            .buttonStyle(.plain).foregroundStyle(.secondary)
        }
        .padding(.horizontal, 10).padding(.vertical, 7)
    }

    private var accent: Color { Color(hex: space.accent) ?? .accentColor }

    private var downloadsPopover: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Downloads").font(.headline)
            if store.downloads.isEmpty {
                Text("Nothing yet").foregroundStyle(.secondary).font(.system(size: 12))
            }
            ForEach(store.downloads, id: \.id) { d in
                HStack {
                    Image(systemName: d.done ? "checkmark.circle.fill" : "arrow.down.circle")
                    VStack(alignment: .leading) {
                        Text(d.filename).font(.system(size: 12)).lineLimit(1)
                        if let dest = d.destination {
                            Text(dest).font(.system(size: 10)).foregroundStyle(.secondary)
                        }
                    }
                    if !d.done {
                        ProgressView(value: d.progress).frame(width: 60)
                    }
                }
            }
        }
        .padding(12).frame(minWidth: 260)
    }
}

struct PinnedCell: View {
    let tab: HiFiCore.Tab
    let index: Int
    let accent: Color
    let focused: Bool

    var body: some View {
        Text(String((tab.title.isEmpty ? "?" : tab.title).prefix(1)).uppercased())
            .font(.system(size: 12, weight: .semibold))
            .frame(width: 34, height: 34)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(focused ? accent.opacity(0.35) : Color.secondary.opacity(0.12))
            )
            .overlay(alignment: .topLeading) {
                Text("\(index + 1)").font(.system(size: 7)).foregroundStyle(.secondary)
                    .padding(2)
            }
            .help("\(tab.title) — ⌘\(index + 1)")
    }
}

extension Color {
    init?(hex: String) {
        var h = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        if h.hasPrefix("#") { h.removeFirst() }
        guard h.count == 6, let v = UInt64(h, radix: 16) else { return nil }
        self.init(red: Double((v >> 16) & 0xFF) / 255,
                  green: Double((v >> 8) & 0xFF) / 255,
                  blue: Double(v & 0xFF) / 255)
    }
}
