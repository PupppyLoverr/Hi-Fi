import Cocoa
import HiFiCore


/// A pane-hosted view that can take key focus.
class PaneHostView: NSView {
    var firstResponderView: NSView { self }
    /// Called when the window becomes key and this pane is focused.
    func refreshOnFocus() {}
}

/// Cache of live pane views by tab id so splits/re-layout never recreate
/// webviews or terminal processes.
@MainActor
final class PaneRegistry {
    private var panes: [String: PaneHostView] = [:]
    private var webPanes: [String: WebPaneView] = [:]

    func view(for tabID: String, in wc: MainWindowController) -> NSView {
        if let p = panes[tabID] { return p }
        guard let tab = wc.store.tab(tabID) else {
            let v = PaneHostView()
            v.wantsLayer = true
            v.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
            return v
        }
        let pane = makePane(for: tab, in: wc)
        panes[tabID] = pane
        if let w = pane as? WebPaneView { webPanes[tabID] = w }
        return pane
    }

    private func makePane(for tab: HiFiCore.Tab, in wc: MainWindowController) -> PaneHostView {
        switch tab.kind {
        case .web, .preview:
            return WebPaneView(tab: tab, store: wc.store)
        case .terminal:
            return TerminalPaneView(tab: tab, store: wc.store, agentMode: false)
        case .agent:
            return TerminalPaneView(tab: tab, store: wc.store, agentMode: true)
        case .diff:
            return DiffPaneView(tab: tab, store: wc.store)
        case .newtab:
            return NewTabPaneView(tab: tab, store: wc.store)
        case .settings:
            return SettingsPaneView(tab: tab, store: wc.store)
        }
    }

    func existing(_ tabID: String) -> PaneHostView? { panes[tabID] }
    func webPane(for tabID: String) -> WebPaneView? { webPanes[tabID] }
    func drop(_ tabID: String) {
        panes.removeValue(forKey: tabID)
        webPanes.removeValue(forKey: tabID)
    }
    func refreshOnFocus(tabID: String) { panes[tabID]?.refreshOnFocus() }
}
