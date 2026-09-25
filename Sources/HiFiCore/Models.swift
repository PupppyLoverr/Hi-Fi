import Foundation

// MARK: - Tab kinds

public enum TabKind: String, Codable, Sendable {
    case web, terminal, agent, diff, preview, newtab, settings
}

public enum TabKindError: Error { case badScheme }

/// Parse a `hifi://` scheme or plain URL into kind + payload.
public enum SchemeParser {
    public static let builtinSchemes: Set<String> = [
        "newtab", "terminal", "agent", "diff", "preview", "settings"
    ]

    public static func parse(_ raw: String) -> (kind: TabKind, url: String) {
        if raw.hasPrefix("hifi://") {
            let rest = String(raw.dropFirst("hifi://".count))
            let host = rest.split(separator: "/", maxSplits: 1).first
                .flatMap { $0.split(separator: "?", maxSplits: 1).first.map(String.init) } ?? rest
            switch host {
            case "terminal": return (.terminal, raw)
            case "agent":    return (.agent, raw)
            case "diff":     return (.diff, raw)
            case "preview":  return (.preview, raw)
            case "settings": return (.settings, raw)
            default:         return (.newtab, "hifi://newtab")
            }
        }
        if raw.hasPrefix("http://") || raw.hasPrefix("https://") || raw.hasPrefix("file://") {
            return (.web, raw)
        }
        // Bare words -> search; bare domain-looking -> https.
        if raw.contains(" ") {
            let q = raw.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? raw
            return (.web, "https://www.google.com/search?q=\(q)")
        }
        return (.web, "https://\(raw)")
    }
}

// MARK: - Domain model

public struct Tab: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var kind: TabKind
    public var url: String
    public var title: String
    public var pinned: Bool
    public var projectPath: String?
    public var harness: String?
    public var createdAt: Date

    public init(id: String = HiFiID.tab(), kind: TabKind, url: String, title: String = "",
                pinned: Bool = false, projectPath: String? = nil, harness: String? = nil) {
        self.id = id; self.kind = kind; self.url = url; self.title = title
        self.pinned = pinned; self.projectPath = projectPath; self.harness = harness
        self.createdAt = Date()
    }
}

public enum SplitDirection: String, Codable, Sendable {
    case horizontal // left | right
    case vertical   // above / below
}

/// Binary split tree over tab ids. Persisted with its group.
public indirect enum SplitNode: Codable, Equatable, Sendable {
    case leaf(String) // tab id
    case split(direction: SplitDirection, first: SplitNode, second: SplitNode, ratio: Double)

    enum CodingKeys: String, CodingKey { case leaf, split, direction, first, second, ratio }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .leaf(let t): try c.encode(t, forKey: .leaf)
        case .split(let d, let f, let s, let r):
            try c.encode(true, forKey: .split)
            try c.encode(d, forKey: .direction)
            try c.encode(f, forKey: .first)
            try c.encode(s, forKey: .second)
            try c.encode(r, forKey: .ratio)
        }
    }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        if let t = try c.decodeIfPresent(String.self, forKey: .leaf) {
            self = .leaf(t); return
        }
        let d = try c.decode(SplitDirection.self, forKey: .direction)
        let f = try c.decode(SplitNode.self, forKey: .first)
        let s = try c.decode(SplitNode.self, forKey: .second)
        let r = try c.decodeIfPresent(Double.self, forKey: .ratio) ?? 0.5
        self = .split(direction: d, first: f, second: s, ratio: r)
    }

    /// All leaf tab ids, left-to-right / top-to-bottom.
    public var tabIDs: [String] {
        switch self {
        case .leaf(let t): return [t]
        case .split(_, let f, let s, _): return f.tabIDs + s.tabIDs
        }
    }

    public func contains(_ tabID: String) -> Bool { tabIDs.contains(tabID) }

    /// Split `target` leaf so `newTab` sits `side` of it at `ratio` (fraction for the FIRST child).
    public func inserting(_ newTab: String, sideOf target: String, side: SplitSide, ratio: Double = 0.5) -> SplitNode {
        switch self {
        case .leaf(let t):
            guard t == target else { return self }
            return SplitNode.makeSplit(target: target, newTab: newTab, side: side, ratio: ratio)
        case .split(let d, let f, let s, let r):
            return .split(direction: d,
                          first: f.inserting(newTab, sideOf: target, side: side, ratio: ratio),
                          second: s.inserting(newTab, sideOf: target, side: side, ratio: ratio),
                          ratio: r)
        }
    }

    static func makeSplit(target: String, newTab: String, side: SplitSide, ratio: Double) -> SplitNode {
        switch side {
        case .right:
            return .split(direction: .horizontal, first: .leaf(target), second: .leaf(newTab), ratio: 1 - ratio)
        case .left:
            return .split(direction: .horizontal, first: .leaf(newTab), second: .leaf(target), ratio: ratio)
        case .below:
            return .split(direction: .vertical, first: .leaf(target), second: .leaf(newTab), ratio: 1 - ratio)
        case .above:
            return .split(direction: .vertical, first: .leaf(newTab), second: .leaf(target), ratio: ratio)
        }
    }

    /// Remove a leaf, collapsing single-child splits. Returns nil if the tree becomes empty.
    public func removing(_ tabID: String) -> SplitNode? {
        switch self {
        case .leaf(let t): return t == tabID ? nil : self
        case .split(let d, let f, let s, let r):
            guard let nf = f.removing(tabID) else { return s.removing(tabID) == nil ? nil : s }
            guard let ns = s.removing(tabID) else { return f.removing(tabID) == nil ? nil : f }
            // both children still exist
            switch (nf, ns) {
            default: return .split(direction: d, first: nf, second: ns, ratio: r)
            }
        }
    }
}

public enum SplitSide: String, Codable, Sendable { case right, left, above, below }

public struct Group: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String          // "" = unnamed "today" list
    public var pinnedSection: Bool   // rendered as the pinned strip
    public var tabs: [Tab]
    public var layout: SplitNode?    // nil -> single tab mode
    public var focusedTabID: String?

    public init(id: String = HiFiID.group(), name: String = "", pinnedSection: Bool = false,
                tabs: [Tab] = [], layout: SplitNode? = nil) {
        self.id = id; self.name = name; self.pinnedSection = pinnedSection
        self.tabs = tabs; self.layout = layout
        self.focusedTabID = tabs.first?.id
    }
}

public struct Profile: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    /// Stable UUID feeding `WKWebsiteDataStore(forIdentifier:)`.
    public var identifier: UUID
    public init(id: String = HiFiID.profile(), name: String, identifier: UUID = UUID()) {
        self.id = id; self.name = name; self.identifier = identifier
    }
}

public struct Space: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var accent: String        // hex, e.g. "#5E5CE6"
    public var profileID: String?    // nil -> default data store
    public var groups: [Group]       // [pinnedSection, today, ...named]
    public var activeGroupID: String?

    public init(id: String = HiFiID.space(), name: String, accent: String, profileID: String? = nil) {
        self.id = id; self.name = name; self.accent = accent; self.profileID = profileID
        let pinned = Group(name: "Pinned", pinnedSection: true)
        let today  = Group(name: "")
        self.groups = [pinned, today]
        self.activeGroupID = today.id
    }
}

public struct Worktree: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var repoPath: String
    public var name: String
    public var path: String
    public var spaceID: String?
    public var groupID: String?
    public var createdAt: Date
    public init(id: String = HiFiID.worktree(), repoPath: String, name: String, path: String,
                spaceID: String? = nil, groupID: String? = nil) {
        self.id = id; self.repoPath = repoPath; self.name = name; self.path = path
        self.spaceID = spaceID; self.groupID = groupID; self.createdAt = Date()
    }
}

public struct HistoryEntry: Codable, Equatable, Sendable {
    public var url: String
    public var title: String
    public var visitedAt: Date
    public var spaceID: String?
    public init(url: String, title: String, spaceID: String? = nil) {
        self.url = url; self.title = title; self.visitedAt = Date(); self.spaceID = spaceID
    }
}

// MARK: - Persisted session

public struct SessionState: Codable, Sendable {
    public var spaces: [Space]
    public var activeSpaceID: String?
    public var profiles: [Profile]
    public var worktrees: [Worktree]
    /// group ids the user has granted agent browser-use control.
    public var agentAllowedGroups: Set<String>
    public var sidebarCollapsed: Bool
    public var windowFrame: String? // "x y w h"

    public init(spaces: [Space] = [], activeSpaceID: String? = nil,
                profiles: [Profile] = [], worktrees: [Worktree] = [],
                agentAllowedGroups: Set<String> = [], sidebarCollapsed: Bool = false,
                windowFrame: String? = nil) {
        self.spaces = spaces; self.activeSpaceID = activeSpaceID
        self.profiles = profiles; self.worktrees = worktrees
        self.agentAllowedGroups = agentAllowedGroups
        self.sidebarCollapsed = sidebarCollapsed; self.windowFrame = windowFrame
    }
}
