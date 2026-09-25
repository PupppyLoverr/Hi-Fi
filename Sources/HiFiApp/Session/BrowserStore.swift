import Foundation
import Combine
import HiFiCore
#if canImport(AppKit)
import AppKit
#endif

/// The single domain controller. Owns spaces/groups/tabs/splits, persistence,
/// history, and every mutating operation the UI or IPC layer can perform.
@MainActor
final class BrowserStore: ObservableObject {

    // MARK: published state
    @Published private(set) var state = SessionState()
    @Published var focusedTabID: String?
    @Published var toastMessage: String?
    @Published var commandBarVisible = false
    @Published var commandBarMode: CommandBarMode = .command
    @Published var findBarVisible = false
    @Published var downloads: [DownloadItem] = []

    enum CommandBarMode { case command, url }

    /// Views live outside the model; the window controller registers pane hosts here.
    weak var windowController: MainWindowController?

    /// Sidebar visibility (persisted inside state).
    var sidebarCollapsed: Bool {
        get { state.sidebarCollapsed }
        set { state.sidebarCollapsed = newValue; persistSoon() }
    }

    /// General state mutator for controllers that can't reach inside `state`.
    func apply(_ f: (inout SessionState) -> Void) {
        f(&state)
        persistSoon()
    }

    /// Store a mutated group back into whichever space owns it.
    func applyGroup(_ g: Group) {
        for si in state.spaces.indices {
            if let gi = state.spaces[si].groups.firstIndex(where: { $0.id == g.id }) {
                state.spaces[si].groups[gi] = g
                persistSoon()
                return
            }
        }
    }

    private var history: [HistoryEntry] = []
    private var persistWork: DispatchWorkItem?
    private var toastWork: DispatchWorkItem?

    var spaceAccents = ["#5E5CE6", "#E8963C", "#3C9E8F", "#C94F6D", "#4C8DDA", "#8B78E6"]

    // MARK: - derived accessors

    var activeSpace: Space {
        state.spaces.first(where: { $0.id == state.activeSpaceID }) ?? state.spaces[0]
    }

    var activeGroup: Group? {
        let s = activeSpace
        return s.groups.first(where: { $0.id == s.activeGroupID })
            ?? s.groups.first(where: { !$0.pinnedSection })
    }

    func space(_ id: String?) -> Space? { state.spaces.first(where: { $0.id == id }) }
    func group(_ id: String?, in space: Space? = nil) -> Group? {
        (space ?? activeSpace).groups.first(where: { $0.id == id })
    }
    func groupOf(tabID: String) -> (space: Space, group: Group)? {
        for s in state.spaces {
            for g in s.groups where g.tabs.contains(where: { $0.id == tabID }) {
                return (s, g)
            }
        }
        return nil
    }
    func tab(_ id: String?) -> Tab? {
        guard let id else { return nil }
        for s in state.spaces {
            for g in s.groups {
                if let t = g.tabs.first(where: { $0.id == id }) { return t }
            }
        }
        return nil
    }

    var focusedTab: Tab? {
        if let id = focusedTabID, let t = tab(id) { return t }
        return activeGroup?.focusedTabID.flatMap { tab($0) }
    }

    // MARK: - bootstrap / persistence

    func bootstrap() {
        try? HiFiPaths.ensureSupportDir()
        if let data = try? Data(contentsOf: HiFiPaths.stateFile) {
            do {
                let dec = JSONDecoder(); dec.dateDecodingStrategy = .iso8601
                let decoded = try dec.decode(SessionState.self, from: data)
                if !decoded.spaces.isEmpty { state = decoded }
            } catch {
                HiFiLog.write("state decode failed: \(error)")
            }
        }
        if state.spaces.isEmpty {
            let space = Space(name: "Personal", accent: spaceAccents[0])
            state = SessionState(spaces: [space], activeSpaceID: space.id,
                                 profiles: [Profile(name: "Default")])
        }
        if let d = try? Data(contentsOf: HiFiPaths.historyFile),
           let h = try? { let d2 = JSONDecoder(); d2.dateDecodingStrategy = .iso8601
                          return try d2.decode([HistoryEntry].self, from: d) }() {
            history = h
        }
        // seed a newtab surface when the session is otherwise empty
        if activeGroup?.tabs.isEmpty != false {
            _ = openTab("hifi://newtab", background: false)
        }
        if focusedTabID == nil {
            focusedTabID = activeGroup?.focusedTabID ?? activeGroup?.tabs.first?.id
        }
    }

    func persistSoon() {
        persistWork?.cancel()
        let w = DispatchWorkItem { [weak self] in self?.persistNow() }
        persistWork = w
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 0.4, execute: w)
    }

    func persistNow() {
        state.windowFrame = windowController?.window.flatMap { NSStringFromRect($0.frame) }
        let enc = JSONEncoder(); enc.outputFormatting = [.sortedKeys]
        enc.dateEncodingStrategy = .iso8601
        if let data = try? enc.encode(state) {
            try? HiFiPaths.ensureSupportDir()
            try? data.write(to: HiFiPaths.stateFile, options: .atomic)
        }
        saveHistory()
    }

    private func saveHistory() {
        let cutoff = Date().addingTimeInterval(-30 * 24 * 3600)
        history = history.filter { $0.visitedAt > cutoff }
        let enc = JSONEncoder(); enc.dateEncodingStrategy = .iso8601
        if let data = try? enc.encode(history) {
            try? data.write(to: HiFiPaths.historyFile, options: .atomic)
        }
    }

    func recordHistory(url: String, title: String) {
        guard url.hasPrefix("http") else { return }
        history.append(HistoryEntry(url: url, title: title, spaceID: state.activeSpaceID))
        if history.count > 5000 { history.removeFirst(history.count - 5000) }
        persistSoon()
    }

    func historyEntries(matching query: String, limit: Int = 30) -> [HistoryEntry] {
        let q = query.lowercased()
        let hits = history.reversed().filter {
            q.isEmpty || $0.url.lowercased().contains(q) || $0.title.lowercased().contains(q)
        }
        var seen = Set<String>()
        return hits.filter { seen.insert($0.url).inserted }.prefix(limit).map { $0 }
    }

    func toast(_ msg: String) {
        toastMessage = msg
        toastWork?.cancel()
        let w = DispatchWorkItem { [weak self] in
            self?.toastMessage = nil
        }
        toastWork = w
        DispatchQueue.main.asyncAfter(deadline: .now() + 4, execute: w)
    }

    // MARK: - tab operations

    @discardableResult
    func openTab(_ rawURL: String,
                 kind explicitKind: TabKind? = nil,
                 sideOf anchorID: String? = nil, side: SplitSide? = nil, size: Double? = nil,
                 background: Bool = false, keepAnchor: Bool = false,
                 groupID: String? = nil, projectPath: String? = nil,
                 profileName: String? = nil, pinned: Bool = false,
                 harness: String? = nil,
                 spaceID: String? = nil) -> String {

        let spaceIdx = state.spaces.firstIndex(where: { $0.id == (spaceID ?? state.activeSpaceID) }) ?? 0
        let (kind, url) = explicitKind != nil
            ? (explicitKind!, rawURL)
            : SchemeParser.parse(rawURL)
        let resolvedProjectPath = projectPath
            ?? URLComponents(string: url)?.queryItems?.first(where: { $0.name == "path" })?.value

        var groupIdx: Int
        if let gID = groupID,
           let gi = state.spaces[spaceIdx].groups.firstIndex(where: { $0.id == gID }) {
            groupIdx = gi
        } else if pinned {
            groupIdx = 0 // pinned section
        } else if let anchorID,
                  let gi = state.spaces[spaceIdx].groups.firstIndex(where: { $0.layout?.contains(anchorID) == true
                      || $0.tabs.contains(where: { $0.id == anchorID }) }) {
            groupIdx = gi
        } else {
            groupIdx = state.spaces[spaceIdx].groups.firstIndex(where: {
                $0.id == state.spaces[spaceIdx].activeGroupID
            }) ?? 1
        }

        // resolve a profile override to a real profile
        if let pn = profileName, !pn.isEmpty,
           !state.profiles.contains(where: { $0.name == pn }) {
            state.profiles.append(Profile(name: pn))
        }

        var tab = Tab(kind: kind, url: url, pinned: pinned,
                      projectPath: resolvedProjectPath, harness: harness)
        switch kind {
        case .web:      tab.title = URL(string: url)?.host ?? url
        case .terminal: tab.title = "Terminal"
        case .agent:    tab.title = "Agent"
        case .diff:     tab.title = "Diff"
        case .preview:  tab.title = "Preview"
        case .newtab:   tab.title = "New Tab"
        case .settings: tab.title = "Settings"
        }

        var group = state.spaces[spaceIdx].groups[groupIdx]
        group.tabs.append(tab)

        if let anchorID, let side {
            if group.layout == nil || group.layout?.contains(anchorID) != true {
                group.layout = .leaf(anchorID)
            }
            group.layout = group.layout!.inserting(tab.id, sideOf: anchorID,
                                                   side: side, ratio: size ?? 0.5)
            group.layout = group.layout?.compactedKeeping(group.tabs.map { $0.id })
        }

        if !(background || keepAnchor) {
            group.focusedTabID = tab.id
            state.spaces[spaceIdx].activeGroupID = group.id
            focusedTabID = tab.id
        } else if group.focusedTabID == nil {
            group.focusedTabID = group.tabs.first?.id
        }

        state.spaces[spaceIdx].groups[groupIdx] = group
        persistSoon()
        return tab.id
    }

    func closeTab(_ id: String) {
        for si in state.spaces.indices {
            for gi in state.spaces[si].groups.indices {
                var g = state.spaces[si].groups[gi]
                guard let ti = g.tabs.firstIndex(where: { $0.id == id }) else { continue }
                let wasPinnedSection = g.pinnedSection
                g.tabs.remove(at: ti)
                g.layout = g.layout?.removing(id)?.compactedKeeping(g.tabs.map { $0.id })
                if g.layout == nil, let first = g.tabs.first { g.layout = nil }
                if g.focusedTabID == id { g.focusedTabID = g.tabs.last?.id }
                // drop empty named groups (keep pinned + today shells)
                if g.tabs.isEmpty && !wasPinnedSection && !g.name.isEmpty {
                    state.spaces[si].groups.remove(at: gi)
                    if state.spaces[si].activeGroupID == g.id {
                        state.spaces[si].activeGroupID = state.spaces[si].groups.first?.id
                    }
                } else {
                    state.spaces[si].groups[gi] = g
                }
                if focusedTabID == id {
                    focusedTabID = activeGroup?.focusedTabID
                }
                persistSoon()
                return
            }
        }
    }

    func focusTab(_ id: String) {
        guard let (s, g) = groupOf(tabID: id) else { return }
        if s.id != state.activeSpaceID { state.activeSpaceID = s.id }
        var gg = g; gg.focusedTabID = id
        setGroup(gg, in: s)
        state.spaces[state.spaces.firstIndex(where: { $0.id == s.id })!].activeGroupID = g.id
        focusedTabID = id
        persistSoon()
    }

    private func setGroup(_ g: Group, in s: Space) {
        if let si = state.spaces.firstIndex(where: { $0.id == s.id }),
           let gi = state.spaces[si].groups.firstIndex(where: { $0.id == g.id }) {
            state.spaces[si].groups[gi] = g
        }
    }

    func updateTab(_ id: String, title: String? = nil, url: String? = nil) {
        for si in state.spaces.indices {
            for gi in state.spaces[si].groups.indices {
                if let ti = state.spaces[si].groups[gi].tabs.firstIndex(where: { $0.id == id }) {
                    if let title, !title.isEmpty { state.spaces[si].groups[gi].tabs[ti].title = title }
                    if let url { state.spaces[si].groups[gi].tabs[ti].url = url }
                    persistSoon()
                    return
                }
            }
        }
    }

    func togglePin(_ id: String) {
        guard let (s, g) = groupOf(tabID: id) else { return }
        var t = tab(id)!; t.pinned.toggle()
        // move between pinned section and today group
        let targetIdx = t.pinned ? 0 : (s.groups.firstIndex(where: { !$0.pinnedSection }) ?? 1)
        removeTabFromGroup(id, group: g, space: s)
        var dest = state.spaces[state.spaces.firstIndex(where: { $0.id == s.id })!].groups[targetIdx]
        dest.tabs.append(t)
        state.spaces[state.spaces.firstIndex(where: { $0.id == s.id })!].groups[targetIdx] = dest
        if t.pinned { focusTab(id) }
        persistSoon()
    }

    private func removeTabFromGroup(_ id: String, group g: Group, space s: Space) {
        var gg = g
        gg.tabs.removeAll { $0.id == id }
        gg.layout = gg.layout?.removing(id)?.compactedKeeping(gg.tabs.map { $0.id })
        if gg.focusedTabID == id { gg.focusedTabID = gg.tabs.last?.id }
        setGroup(gg, in: s)
    }

    // MARK: - splits

    @discardableResult
    func splitTab(anchorID: String, side: SplitSide, with otherID: String? = nil,
                  size: Double? = nil, moveBetweenGroups: Bool = false) -> String? {
        guard let (s, g) = groupOf(tabID: anchorID) else {
            HiFiLog.write("splitTab: no group for anchor \(anchorID)")
            return nil
        }
        var newID: String
        if let otherID {
            if let (os, og) = groupOf(tabID: otherID), og.id != g.id {
                guard moveBetweenGroups else { return nil } // must opt in
                removeTabFromGroup(otherID, group: og, space: os)
                var t = tab(otherID)!; t.pinned = false
                var gg = g; gg.tabs.append(t); setGroup(gg, in: s)
                newID = otherID
            } else {
                newID = otherID
            }
        } else {
            newID = openTab("hifi://newtab", background: true, groupID: g.id)
        }
        var gg = state.spaces.first(where: { $0.id == s.id })!.groups.first(where: { $0.id == g.id })!
        if gg.layout == nil { gg.layout = .leaf(anchorID) }
        if !gg.layout!.contains(anchorID) { gg.layout = .leaf(anchorID) }
        gg.layout = gg.layout!.removing(newID)?.compactedKeeping(gg.tabs.map { $0.id }) ?? gg.layout
        if gg.layout == nil { gg.layout = .leaf(anchorID) }
        gg.layout = gg.layout!.inserting(newID, sideOf: anchorID, side: side, ratio: size ?? 0.5)
        gg.focusedTabID = newID
        setGroup(gg, in: s)
        focusedTabID = newID
        persistSoon()
        HiFiLog.write("splitTab ok: \(newID) into group \(gg.id)")
        return newID
    }

    /// cycle through non-pinned tabs of the active group
    func cycleTab(_ delta: Int) {
        guard let g = activeGroup, !g.tabs.isEmpty else { return }
        let ids = g.tabs.map { $0.id }
        let cur = ids.firstIndex(of: g.focusedTabID ?? "") ?? 0
        let next = ids[(cur + delta + ids.count) % ids.count]
        focusTab(next)
    }

    func pinJump(_ index: Int) {
        let pinned = activeSpace.groups[0].tabs
        guard index < pinned.count else { return }
        focusTab(pinned[index].id)
    }

    // MARK: - groups

    @discardableResult
    func createGroup(name: String, moveTabID: String? = nil) -> String {
        var g = Group(name: name)
        if let moveTabID, let (s, og) = groupOf(tabID: moveTabID) {
            var t = tab(moveTabID)!; t.pinned = false
            removeTabFromGroup(moveTabID, group: og, space: s)
            g.tabs = [t]; g.focusedTabID = t.id
        }
        var sp = activeSpace
        sp.groups.append(g)
        sp.activeGroupID = g.id
        if let si = state.spaces.firstIndex(where: { $0.id == sp.id }) {
            state.spaces[si] = sp
        }
        if let ft = g.focusedTabID { focusedTabID = ft }
        persistSoon()
        return g.id
    }

    func renameGroup(_ id: String, name: String) {
        for si in state.spaces.indices {
            if let gi = state.spaces[si].groups.firstIndex(where: { $0.id == id }) {
                state.spaces[si].groups[gi].name = name
                persistSoon(); return
            }
        }
    }

    // MARK: - spaces / profiles

    @discardableResult
    func createSpace(name: String) -> String {
        let accent = spaceAccents[state.spaces.count % spaceAccents.count]
        var s = Space(name: name, accent: accent)
        state.spaces.append(s)
        state.activeSpaceID = s.id
        _ = openTab("hifi://newtab", groupID: s.groups[1].id, spaceID: s.id)
        persistSoon()
        return s.id
    }

    func switchSpace(_ idOrName: String) -> Bool {
        if let s = state.spaces.first(where: { $0.id == idOrName || $0.name == idOrName }) {
            state.activeSpaceID = s.id
            focusedTabID = activeGroup?.focusedTabID
            persistSoon()
            return true
        }
        return false
    }

    func cycleSpace(_ delta: Int) {
        guard !state.spaces.isEmpty else { return }
        let ids = state.spaces.map { $0.id }
        let cur = ids.firstIndex(of: state.activeSpaceID ?? "") ?? 0
        _ = switchSpace(ids[(cur + delta + ids.count) % ids.count])
    }

    // MARK: - JSON views for IPC

    func tabListJSON(groupID: String? = nil) -> JSONValue {
        var rows: [JSONValue] = []
        for s in state.spaces {
            for g in s.groups {
                if let groupID, g.id != groupID { continue }
                for t in g.tabs {
                    rows.append(.object([
                        "id": .str(t.id), "kind": .str(t.kind.rawValue),
                        "title": .str(t.title), "url": .str(t.url),
                        "group": .str(g.id), "space": .str(s.id),
                        "pinned": .bool(t.pinned)
                    ]))
                }
            }
        }
        return .object(["rows": .array(rows)])
    }

    func groupJSON(_ id: String) -> JSONValue? {
        for s in state.spaces {
            if let g = s.groups.first(where: { $0.id == id }) {
                return .object([
                    "id": .str(g.id), "name": .str(g.name), "space": .str(s.id),
                    "tabs": .array(g.tabs.map {
                        .object(["id": .str($0.id), "title": .str($0.title),
                                 "url": .str($0.url), "kind": .str($0.kind.rawValue)])
                    }),
                    "focused": g.focusedTabID.map { JSONValue.str($0) } ?? .null
                ])
            }
        }
        return nil
    }

    func spaceListJSON() -> JSONValue {
        .object(["rows": .array(state.spaces.map {
            .object(["id": .str($0.id), "name": .str($0.name),
                     "accent": .str($0.accent),
                     "active": .bool($0.id == state.activeSpaceID),
                     "groups": .number(Double($0.groups.count))])
        })])
    }

    func updateTabProjectPath(_ id: String, path: String) {
        for si in state.spaces.indices {
            for gi in state.spaces[si].groups.indices {
                if let ti = state.spaces[si].groups[gi].tabs.firstIndex(where: { $0.id == id }) {
                    state.spaces[si].groups[gi].tabs[ti].projectPath = path
                    persistSoon()
                    return
                }
            }
        }
    }

    // MARK: - downloads tray

    func registerDownload(filename: String, url: String, destination: String) {
        downloads.append(DownloadItem(filename: filename, url: url,
                                      progress: 0, done: false,
                                      destination: destination))
    }

    func finishDownload(url: String) {
        if let i = downloads.lastIndex(where: { $0.url == url && !$0.done }) {
            downloads[i].done = true
            downloads[i].progress = 1
        }
    }

    // MARK: - agent control permission

    func ensureAgentPermission(groupID: String, ask: () -> Bool) -> Bool {
        if state.agentAllowedGroups.contains(groupID) { return true }
        guard ask() else { return false }
        state.agentAllowedGroups.insert(groupID)
        persistSoon()
        return true
    }
}

extension SplitNode {
    /// Drop leaves whose tab no longer exists in `keep`.
    func compactedKeeping(_ keep: [String]) -> SplitNode? {
        switch self {
        case .leaf(let t): return keep.contains(t) ? self : nil
        case .split(let d, let f, let s, let r):
            let nf = f.compactedKeeping(keep)
            let ns = s.compactedKeeping(keep)
            switch (nf, ns) {
            case (nil, nil): return nil
            case (.some(let a), nil): return a
            case (nil, .some(let b)): return b
            case (.some(let a), .some(let b)): return .split(direction: d, first: a, second: b, ratio: r)
            }
        }
    }
}

struct DownloadItem: Equatable {
    let id = UUID()
    var filename: String
    var url: String
    var progress: Double
    var done: Bool
    var destination: String?
    static func == (a: DownloadItem, b: DownloadItem) -> Bool { a.id == b.id }
}
