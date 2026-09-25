import Foundation

/// One JSON object per line on the UNIX socket.
public struct IPCRequest: Codable, Sendable {
    public var id: String
    public var command: String
    public var args: [String: JSONValue]

    public init(command: String, args: [String: JSONValue] = [:]) {
        self.id = UUID().uuidString
        self.command = command
        self.args = args
    }

    public func encode() throws -> Data {
        var d = try JSONEncoder().encode(self)
        d.append(0x0A)
        return d
    }
}

public struct IPCResponse: Codable, Sendable {
    public var id: String
    public var ok: Bool
    public var result: JSONValue?
    public var error: String?

    public init(id: String, ok: Bool, result: JSONValue? = nil, error: String? = nil) {
        self.id = id; self.ok = ok; self.result = result; self.error = error
    }

    public func encode() throws -> Data {
        var d = try JSONEncoder().encode(self)
        d.append(0x0A)
        return d
    }
}

/// Command names shared by the CLI and the app's dispatcher.
public enum IPCCommand {
    public static let ping           = "app.ping"
    public static let tabOpen        = "tab.open"
    public static let tabList        = "tab.list"
    public static let tabFocus       = "tab.focus"
    public static let tabClose       = "tab.close"
    public static let tabSplit       = "tab.split"
    public static let groupCreate    = "group.create"
    public static let groupRename    = "group.rename"
    public static let groupShow      = "group.show"
    public static let spaceList      = "space.list"
    public static let spaceCreate    = "space.create"
    public static let spaceSwitch    = "space.switch"
    public static let worktreeCreate = "worktree.create"
    public static let agentStart     = "agent.start"
    public static let pageSnapshot   = "page.snapshot"
    public static let pageClick      = "page.click"
    public static let pageType       = "page.type"
    public static let pagePress      = "page.press"
    public static let pageURL        = "page.url"
    public static let pageHTML       = "page.html"
    public static let pageScreenshot = "page.screenshot"
}
