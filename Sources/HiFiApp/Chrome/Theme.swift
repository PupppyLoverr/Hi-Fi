import Cocoa
import SwiftUI
import HiFiCore

// MARK: - Hi-Fi theme: the Cosmos design language
//
// Tokens mirror Cosmos (crates/theme): dark separates planes by tiny lightness
// steps (#060606 shell → #0d/#0e surfaces → #16/#1e raised), light uses
// white planes + hairline borders + soft shadows. Radii ladder 16/10/6,
// spacing 4/8/12/16. Accent presets match cosmos's AccentPreset table.

enum HFAppearance { case dark, light }

struct HFPalette {
    let isDark: Bool

    // planes
    let background: NSColor     // window backdrop        #060606 / #ffffff
    let shell: NSColor          // sidebar plane          #0d0d0d / #f3f3f5
    let surfaceCard: NSColor    // cards, chips           #0e0e0e / #ffffff
    let surfaceDialog: NSColor  // raised panel           #161616 / #ffffff
    let surfaceOverlay: NSColor // highest surface        #1e1e1e / #ffffff
    let raised: NSColor         // inputs, hover-target   #343438 / #ededf0

    // text ladder
    let text: NSColor
    let muted: NSColor
    let faint: NSColor

    // washes
    let elementHover: NSColor
    let elementActive: NSColor

    // hairlines
    let border: NSColor
    let borderStrong: NSColor

    // accents + signal
    let accent: NSColor
    let accentStrong: NSColor
    let accentWash: NSColor
    let onAccent: NSColor
    let danger: NSColor
    let warning: NSColor
    let success: NSColor
    let terminalBackground: NSColor

    init(isDark: Bool, accentHex: String) {
        self.isDark = isDark
        if isDark {
            background = HF.rgb(0x060606)
            shell = HF.rgb(0x0d0d0d)
            surfaceCard = HF.rgb(0x0e0e0e)
            surfaceDialog = HF.rgb(0x161616)
            surfaceOverlay = HF.rgb(0x1e1e1e)
            raised = HF.rgb(0x262628)
            text = HF.rgb(0xe8e8ea)
            muted = HF.rgb(0xa9a9ae)
            faint = HF.rgb(0x85858a)
            elementHover = NSColor.white.withAlphaComponent(0.05)
            elementActive = NSColor.white.withAlphaComponent(0.09)
            border = NSColor.white.withAlphaComponent(0.075)
            borderStrong = NSColor.white.withAlphaComponent(0.13)
            danger = HF.rgb(0xf87171)
            warning = HF.rgb(0xfacc15)
            success = HF.rgb(0x34d399)
            terminalBackground = HF.rgb(0x090909)
        } else {
            background = HF.rgb(0xffffff)
            shell = HF.rgb(0xf3f3f5)
            surfaceCard = HF.rgb(0xffffff)
            surfaceDialog = HF.rgb(0xffffff)
            surfaceOverlay = HF.rgb(0xffffff)
            raised = HF.rgb(0xededf0)
            text = HF.rgb(0x303035)
            muted = HF.rgb(0x62626a)
            faint = HF.rgb(0x797981)
            elementHover = NSColor.black.withAlphaComponent(0.045)
            elementActive = NSColor.black.withAlphaComponent(0.08)
            border = NSColor.black.withAlphaComponent(0.08)
            borderStrong = NSColor.black.withAlphaComponent(0.14)
            danger = HF.rgb(0xdc2626)
            warning = HF.rgb(0xa16207)
            success = HF.rgb(0x15803d)
            terminalBackground = HF.rgb(0xfafafa)
        }
        accent = HF.rgb(hex: accentHex) ?? HF.rgb(0x8b7cf6)
        accentStrong = accent.highlight(withLevel: 0.14) ?? accent
        accentWash = accent.withAlphaComponent(isDark ? 0.16 : 0.10)
        onAccent = isDark ? HF.rgb(0xffffff) : HF.rgb(0xffffff)
    }

    // SwiftUI accessors
    var sBackground: Color { Color(nsColor: background) }
    var sShell: Color { Color(nsColor: shell) }
    var sCard: Color { Color(nsColor: surfaceCard) }
    var sRaised: Color { Color(nsColor: raised) }
    var sText: Color { Color(nsColor: text) }
    var sMuted: Color { Color(nsColor: muted) }
    var sFaint: Color { Color(nsColor: faint) }
    var sHover: Color { Color(nsColor: elementHover) }
    var sActive: Color { Color(nsColor: elementActive) }
    var sBorder: Color { Color(nsColor: border) }
    var sBorderStrong: Color { Color(nsColor: borderStrong) }
    var sAccent: Color { Color(nsColor: accent) }
    var sAccentStrong: Color { Color(nsColor: accentStrong) }
    var sAccentWash: Color { Color(nsColor: accentWash) }
    var sDanger: Color { Color(nsColor: danger) }
    var sSuccess: Color { Color(nsColor: success) }
    var sWarning: Color { Color(nsColor: warning) }
    var sTerminalBg: Color { Color(nsColor: terminalBackground) }
}

// MARK: - helpers

enum HF {
    static func rgb(_ hex: Int) -> NSColor {
        NSColor(red: CGFloat((hex >> 16) & 0xff) / 255,
                green: CGFloat((hex >> 8) & 0xff) / 255,
                blue: CGFloat(hex & 0xff) / 255, alpha: 1)
    }

    /// "#rrggbb" or "#rgb" -> NSColor
    static func rgb(hex: String) -> NSColor? {
        var s = hex.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("#") { s.removeFirst() }
        guard s.count == 6 || s.count == 3 else { return nil }
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        guard let v = Int(s, radix: 16) else { return nil }
        return rgb(v)
    }

    // Cosmos AccentPreset table (dark / light).
    static let accentPresets: [(id: String, label: String, dark: Int, light: Int)] = [
        ("cosmos", "Cosmos", 0x8b7cf6, 0x5b43e8),
        ("orange", "Orange", 0xfb923c, 0xc2410c),
        ("amber",  "Amber",  0xfbbf24, 0xa16207),
        ("green",  "Green",  0x4ade80, 0x15803d),
        ("cyan",   "Cyan",   0x22d3ee, 0x0e7490),
        ("blue",   "Blue",   0x60a5fa, 0x2563eb),
        ("pink",   "Pink",   0xf472b6, 0xbe185d),
    ]

    static func accentHex(id: String, dark: Bool) -> String {
        guard let p = accentPresets.first(where: { $0.id == id }) else {
            return dark ? "#8b7cf6" : "#5b43e8"
        }
        return String(format: "#%06x", dark ? p.dark : p.light)
    }

    static func accentNSColor(id: String, dark: Bool) -> NSColor {
        guard let p = accentPresets.first(where: { $0.id == id }) else {
            return rgb(dark ? 0x8b7cf6 : 0x5b43e8)
        }
        return rgb(dark ? p.dark : p.light)
    }
}

// MARK: - Theme provider (observable, persisted via SessionState.settings)

@MainActor
final class HiFiTheme: ObservableObject {
    /// Resolved palette — recomputed when appearance or accent changes.
    @Published private(set) var palette: HFPalette
    @Published private(set) var isDark: Bool
    @Published private(set) var accentHex: String

    private var settings: HiFiCore.Settings
    private let spaceAccent: () -> String

    init(settings: HiFiCore.Settings, spaceAccent: @escaping () -> String) {
        self.settings = settings
        self.spaceAccent = spaceAccent
        let dark = HiFiTheme.resolveDark(settings.appearance)
        let hex = HiFiTheme.resolveAccent(settings, spaceAccent: spaceAccent, dark: dark)
        self.isDark = dark
        self.accentHex = hex
        self.palette = HFPalette(isDark: dark, accentHex: hex)
    }

    static func resolveDark(_ appearance: String) -> Bool {
        switch appearance {
        case "dark": return true
        case "light": return false
        default:
            let name = NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua])
            return name == .darkAqua
        }
    }

    static func resolveAccent(_ s: HiFiCore.Settings, spaceAccent: () -> String, dark: Bool) -> String {
        if !s.accent.isEmpty { return HF.accentHex(id: s.accent, dark: dark) }
        let space = spaceAccent()
        return HF.rgb(hex: space) != nil ? space : HF.accentHex(id: "cosmos", dark: dark)
    }

    func update(_ s: HiFiCore.Settings) {
        settings = s
        recompute()
    }

    func recompute() {
        let dark = HiFiTheme.resolveDark(settings.appearance)
        let hex = HiFiTheme.resolveAccent(settings, spaceAccent: spaceAccent, dark: dark)
        isDark = dark
        accentHex = hex
        palette = HFPalette(isDark: dark, accentHex: hex)
    }
}

// MARK: - Solar icon loading (linear weight, currentColor, template)

enum HFIcon {
    private static var cache: [String: NSImage] = [:]
    private static var cacheLock = NSLock()

    /// Load a bundled Solar SVG, or nil. Names match Resources/icons/<name>.svg.
    static func svg(_ name: String) -> NSImage? {
        cacheLock.lock()
        if let img = cache[name] { cacheLock.unlock(); return img }
        cacheLock.unlock()
        var img: NSImage?
        if let url = Bundle.main.url(forResource: name, withExtension: "svg") ??
                       Bundle.main.url(forResource: name, withExtension: "svg", subdirectory: "icons"),
           let data = try? Data(contentsOf: url) {
            img = NSImage(data: data)
        }
        if img == nil,
           let path = Bundle.main.resourceURL?.appendingPathComponent("icons/\(name).svg"),
           let data = try? Data(contentsOf: path) {
            img = NSImage(data: data)
        }
        // debug/scratch builds without a bundle: look next to the binary
        if img == nil {
            let exe = ProcessInfo.processInfo.arguments[0]
            let guess = URL(fileURLWithPath: exe)
                .deletingLastPathComponent()
                .appendingPathComponent("icons/\(name).svg")
            if let data = try? Data(contentsOf: guess) { img = NSImage(data: data) }
        }
        if let img { img.isTemplate = true }
        cacheLock.lock()
        if let img { cache[name] = img }
        cacheLock.unlock()
        return img
    }

    /// Solar icon with SF Symbol fallback.
    static func image(_ name: String, fallback sf: String, size: CGFloat = 16) -> NSImage {
        if let svg = svg(name) {
            svg.size = NSSize(width: size, height: size)
            return svg
        }
        return NSImage(systemSymbolName: sf, accessibilityDescription: nil) ?? NSImage()
    }
}

/// SwiftUI wrapper: Solar icon tinted like Cosmos (text color drives the stroke).
struct HFIconView: View {
    let name: String
    var fallback: String = "questionmark"
    var size: CGFloat = 14
    var color: Color = Color(nsColor: .labelColor)

    var body: some View {
        if let img = HFIcon.svg(name) {
            Image(nsImage: img)
                .resizable()
                .renderingMode(.template)
                .foregroundStyle(color)
                .frame(width: size, height: size)
        } else {
            Image(systemName: fallback)
                .resizable()
                .renderingMode(.template)
                .foregroundStyle(color)
                .frame(width: size, height: size)
        }
    }
}

// MARK: - geometry

enum HFRadius {
    static let bubble: CGFloat = 16   // cards, big pills
    static let panel: CGFloat = 10    // panels, menus
    static let control: CGFloat = 6   // buttons, fields, chips
    static let round: CGFloat = 999
}

enum HFSpace {
    static let xs: CGFloat = 4
    static let sm: CGFloat = 8
    static let md: CGFloat = 12
    static let lg: CGFloat = 16
}
