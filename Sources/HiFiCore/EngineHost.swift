import Foundation

// MARK: - Cross-platform engine contract
//
// Core declares WHAT the engine can do, never HOW. The macOS target implements
// this with WKWebView; Linux with WebKitGTK; Windows with WebKit or Gecko.
// Nothing in HiFiCore imports WebKit.

public struct PageNode: Codable, Equatable, Sendable {
    public var ref: String        // "@e12"
    public var role: String       // a11y-ish role
    public var name: String       // accessible name / text
    public var tag: String
    public var href: String?
    public var value: String?
    public var rect: [Double]     // x y w h
    public init(ref: String, role: String, name: String, tag: String,
                href: String? = nil, value: String? = nil, rect: [Double] = []) {
        self.ref = ref; self.role = role; self.name = name; self.tag = tag
        self.href = href; self.value = value; self.rect = rect
    }
}

public struct PageSnapshot: Codable, Equatable, Sendable {
    public var url: String
    public var title: String
    public var nodes: [PageNode]
    public init(url: String, title: String, nodes: [PageNode]) {
        self.url = url; self.title = title; self.nodes = nodes
    }
}

/// Engine primitives every platform implements. macOS: WKWebView.
public protocol EngineHost: AnyObject, Sendable {
    func createTab(profileID: String?, url: String) -> String          // -> tab id
    func navigate(tabID: String, url: String)
    func goBack(tabID: String)
    func goForward(tabID: String)
    func reload(tabID: String)
    func snapshot(tabID: String) async throws -> PageSnapshot
    func evalJS(tabID: String, source: String) async throws -> JSONValue
    func screenshot(tabID: String) async throws -> Data // png
    func currentURL(tabID: String) -> String?
    func currentTitle(tabID: String) -> String?
}

// MARK: - Keybindings (engine-agnostic). The macOS layer maps these to NSEvent.

public enum KeyAction: String, CaseIterable, Sendable {
    case commandBar, addressBar, closeTab, nextTab, prevTab
    case nextSpace, prevSpace, toggleSidebar, splitRight, splitDown
    case findInPage, reload, settings, newTab, goBack, goForward
    case pinJump1, pinJump2, pinJump3, pinJump4, pinJump5
    case pinJump6, pinJump7, pinJump8, pinJump9
}

public struct KeyChord: Equatable, Sendable {
    public var key: String       // "t", "l", "\\", "enter", "]", "1"...
    public var cmd: Bool = false
    public var shift: Bool = false
    public var ctrl: Bool = false
    public var opt: Bool = false
    public init(_ key: String, cmd: Bool = false, shift: Bool = false,
                ctrl: Bool = false, opt: Bool = false) {
        self.key = key; self.cmd = cmd; self.shift = shift; self.ctrl = ctrl; self.opt = opt
    }
}

/// The v0 keymap. Single source of truth; other platforms port this table.
public let Keymap: [(KeyChord, KeyAction)] = [
    (KeyChord("t", cmd: true),            .commandBar),
    (KeyChord("l", cmd: true),            .addressBar),
    (KeyChord("w", cmd: true),            .closeTab),
    (KeyChord("]", cmd: true, shift: true), .nextTab),
    (KeyChord("[", cmd: true, shift: true), .prevTab),
    (KeyChord("tab", ctrl: true),         .nextSpace),
    (KeyChord("tab", shift: true, ctrl: true), .prevSpace),
    (KeyChord("\\", cmd: true),           .toggleSidebar),
    (KeyChord("\r", cmd: true),           .splitRight),
    (KeyChord("\r", cmd: true, shift: true), .splitDown),
    (KeyChord("f", cmd: true),            .findInPage),
    (KeyChord("r", cmd: true),            .reload),
    (KeyChord(",", cmd: true),            .settings),
    (KeyChord("n", cmd: true),            .newTab),
    (KeyChord("1", cmd: true), .pinJump1), (KeyChord("2", cmd: true), .pinJump2),
    (KeyChord("3", cmd: true), .pinJump3), (KeyChord("4", cmd: true), .pinJump4),
    (KeyChord("5", cmd: true), .pinJump5), (KeyChord("6", cmd: true), .pinJump6),
    (KeyChord("7", cmd: true), .pinJump7), (KeyChord("8", cmd: true), .pinJump8),
    (KeyChord("9", cmd: true), .pinJump9),
]
