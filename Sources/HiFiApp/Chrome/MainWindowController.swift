import Cocoa
import SwiftUI
import Combine
import HiFiCore


/// One window: visual-effect sidebar + content area whose layout mirrors the
/// active group's SplitNode tree. Command bar / find bar / toasts float above.
@MainActor
final class MainWindowController: NSWindowController, NSWindowDelegate {

    let store: BrowserStore
    private let sidebarView: NSHostingView<SidebarView>
    private let contentArea = NSView()
    let paneRegistry = PaneRegistry()
    private var keyMonitor: Any?
    private var cancellables = Set<AnyCancellable>()
    private var commandBar: CommandBarController?
    private var toastLabel: NSTextField?
    private var toastView: NSVisualEffectView?
    private var findBar: FindBarController?
    private var lastContentKey = ""

    private let splitView = NSSplitView()   // [sidebar | content]
    private let sidebarWidth: CGFloat = 224

    init(store: BrowserStore) {
        self.store = store
        sidebarView = NSHostingView(rootView: SidebarView(store: store))
        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1240, height: 800),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered, defer: false)
        w.title = "Hi-Fi"
        w.titlebarAppearsTransparent = true
        w.titleVisibility = .hidden
        w.isMovableByWindowBackground = true
        w.minSize = NSSize(width: 640, height: 420)
        w.isReleasedWhenClosed = false
        super.init(window: w)
        w.delegate = self
        layoutViews()
        installKeyMonitor()
        observeStore()
        restoreFrame()
        rebuildContent()
    }

    required init?(coder: NSCoder) { fatalError() }

    // MARK: - layout

    private func layoutViews() {
        guard let content = window?.contentView else { return }
        content.wantsLayer = true

        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(splitView)
        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: content.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])

        let sidebarWrap = NSVisualEffectView()
        sidebarWrap.material = .sidebar
        sidebarWrap.blendingMode = .behindWindow
        sidebarView.translatesAutoresizingMaskIntoConstraints = false
        sidebarWrap.addSubview(sidebarView)
        NSLayoutConstraint.activate([
            sidebarView.leadingAnchor.constraint(equalTo: sidebarWrap.leadingAnchor),
            sidebarView.trailingAnchor.constraint(equalTo: sidebarWrap.trailingAnchor),
            sidebarView.topAnchor.constraint(equalTo: sidebarWrap.topAnchor),
            sidebarView.bottomAnchor.constraint(equalTo: sidebarWrap.bottomAnchor),
        ])

        contentArea.wantsLayer = true
        splitView.addArrangedSubview(sidebarWrap)
        splitView.addArrangedSubview(contentArea)
        splitView.setHoldingPriority(.defaultHigh, forSubviewAt: 0)
        sidebarWrap.translatesAutoresizingMaskIntoConstraints = false
        sidebarWrap.widthAnchor.constraint(equalToConstant: sidebarWidth).isActive = true

        let cb = CommandBarController(store: store)
        commandBar = cb
        cb.attach(to: window!)

        findBar = FindBarController(store: store, registry: paneRegistry)
        findBar?.attach(to: window!)

        let tv = NSVisualEffectView()
        tv.material = .hudWindow; tv.state = .active
        tv.wantsLayer = true; tv.layer?.cornerRadius = 10
        let label = NSTextField(labelWithString: "")
        label.textColor = .labelColor
        label.font = .systemFont(ofSize: 12)
        label.translatesAutoresizingMaskIntoConstraints = false
        tv.addSubview(label)
        tv.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: tv.leadingAnchor, constant: 14),
            label.trailingAnchor.constraint(equalTo: tv.trailingAnchor, constant: -14),
            label.topAnchor.constraint(equalTo: tv.topAnchor, constant: 8),
            label.bottomAnchor.constraint(equalTo: tv.bottomAnchor, constant: -8),
        ])
        tv.isHidden = true
        content.addSubview(tv)
        NSLayoutConstraint.activate([
            tv.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -18),
            tv.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -18),
        ])
        toastView = tv; toastLabel = label
    }

    private func observeStore() {
        store.$state
            .combineLatest(store.$focusedTabID)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                self?.rebuildContentIfStructureChanged()
                self?.syncTitleBar()
            }
            .store(in: &cancellables)
        store.$toastMessage
            .receive(on: DispatchQueue.main)
            .sink { [weak self] msg in
                self?.toastLabel?.stringValue = msg ?? ""
                self?.toastView?.isHidden = (msg == nil)
            }
            .store(in: &cancellables)
        store.$commandBarVisible
            .receive(on: DispatchQueue.main)
            .sink { [weak self] v in v ? self?.commandBar?.show() : self?.commandBar?.hide() }
            .store(in: &cancellables)
        store.$findBarVisible
            .receive(on: DispatchQueue.main)
            .sink { [weak self] v in v ? self?.findBar?.show() : self?.findBar?.hide() }
            .store(in: &cancellables)
        store.$state.map(\.sidebarCollapsed).removeDuplicates()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] c in self?.splitView.arrangedSubviews.first?.isHidden = c }
            .store(in: &cancellables)
    }

    private func restoreFrame() {
        guard let desc = store.state.windowFrame else { return }
        window?.setFrame(NSRectFromString(desc), display: true)
    }

    private func syncTitleBar() {
        window?.title = "Hi-Fi — \(store.activeSpace.name)"
    }

    // MARK: - content area = split tree of the active group

    /// Only the structure (which tabs tile where) triggers a view rebuild;
    /// title/url/progress churn does not.
    private func effectiveRoot(_ g: HiFiCore.Group) -> SplitNode? {
        if let layout = g.layout {
            if let focused = g.focusedTabID, !layout.contains(focused),
               g.tabs.contains(where: { $0.id == focused }) {
                return .leaf(focused) // focused tab isn't in the split — show it solo
            }
            return layout
        }
        return g.focusedTabID.map { SplitNode.leaf($0) }
    }

    private func contentKey() -> String {
        let g = store.activeGroup ?? store.activeSpace.groups.first
        let layoutKey = g.flatMap { effectiveRoot($0) }.map { Self.nodeKey($0) } ?? "-"
        return "\(store.state.activeSpaceID ?? "")|\(g?.id ?? "")|\(layoutKey)"
    }
    private static func nodeKey(_ n: SplitNode) -> String {
        switch n {
        case .leaf(let t): return "L\(t)"
        case .split(let d, let f, let s, _):
            return "S\(d == .horizontal ? "H" : "V")(\(nodeKey(f)),\(nodeKey(s)))"
        }
    }

    private func rebuildContentIfStructureChanged() {
        let key = contentKey()
        guard key != lastContentKey else { return }
        lastContentKey = key
        rebuildContent()
    }

    private func rebuildContent() {
        guard let group = store.activeGroup ?? store.activeSpace.groups.first else { return }
        let root = effectiveRoot(group)
        contentArea.subviews.forEach { $0.removeFromSuperview() }
        guard let root else {
            let empty = NSTextField(labelWithString: "No tabs — ⌘T to open one")
            empty.textColor = .tertiaryLabelColor
            empty.font = .systemFont(ofSize: 14)
            empty.translatesAutoresizingMaskIntoConstraints = false
            contentArea.addSubview(empty)
            NSLayoutConstraint.activate([
                empty.centerXAnchor.constraint(equalTo: contentArea.centerXAnchor),
                empty.centerYAnchor.constraint(equalTo: contentArea.centerYAnchor),
            ])
            return
        }
        let tree = build(root, for: group)
        tree.translatesAutoresizingMaskIntoConstraints = false
        contentArea.addSubview(tree)
        NSLayoutConstraint.activate([
            tree.leadingAnchor.constraint(equalTo: contentArea.leadingAnchor),
            tree.trailingAnchor.constraint(equalTo: contentArea.trailingAnchor),
            tree.topAnchor.constraint(equalTo: contentArea.topAnchor),
            tree.bottomAnchor.constraint(equalTo: contentArea.bottomAnchor),
        ])
        DispatchQueue.main.async { [weak self] in self?.applyRatios(root, in: tree) }
        if let ft = group.focusedTabID { focusPane(ft) }
    }

    /// Map the model tree to nested NSSplitViews; leaves come from the registry
    /// so panes (webviews, terminals) survive re-layout.
    private func build(_ node: SplitNode, for group: HiFiCore.Group) -> NSView {
        switch node {
        case .leaf(let tabID):
            return paneRegistry.view(for: tabID, in: self)
        case .split(let dir, let first, let second, _):
            let sv = FocusSplitView()
            sv.isVertical = (dir == .horizontal)
            sv.dividerStyle = .thin
            sv.delegate = sv
            sv.addArrangedSubview(build(first, for: group))
            sv.addArrangedSubview(build(second, for: group))
            sv.onResize = { [weak self] in self?.persistRatios(for: group) }
            return sv
        }
    }

    private func applyRatios(_ node: SplitNode, in view: NSView) {
        guard case .split(let dir, let first, let second, let ratio) = node,
              let sv = view as? NSSplitView, sv.arrangedSubviews.count == 2 else { return }
        let total = dir == .horizontal ? sv.bounds.width : sv.bounds.height
        guard total > 1 else {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
                self?.applyRatios(node, in: view)
            }
            return
        }
        sv.setPosition(total * CGFloat(ratio), ofDividerAt: 0)
        applyRatios(first, in: sv.arrangedSubviews[0])
        applyRatios(second, in: sv.arrangedSubviews[1])
    }

    /// Write live divider positions back into the group's SplitNode tree.
    /// Reads the live group so a stale build-time snapshot can't resurrect
    /// tabs/layout that were moved or removed since the view was built.
    func persistRatios(for group: HiFiCore.Group) {
        guard var live = store.group(group.id), let root = live.layout else { return }
        let synced = syncRatios(node: root, view: contentArea.subviews.first)
        if synced != root {
            live.layout = synced
            store.applyGroup(live)
        }
    }

    private func syncRatios(node: SplitNode, view: NSView?) -> SplitNode {
        guard case .split(let dir, let f, let s, let ratio) = node,
              let sv = view as? NSSplitView, sv.arrangedSubviews.count == 2 else { return node }
        let total = dir == .horizontal ? sv.bounds.width : sv.bounds.height
        let firstSize = dir == .horizontal ? sv.arrangedSubviews[0].bounds.width
                                           : sv.arrangedSubviews[0].bounds.height
        let r = total > 1 ? firstSize / total : ratio
        return .split(direction: dir,
                      first: syncRatios(node: f, view: sv.arrangedSubviews[0]),
                      second: syncRatios(node: s, view: sv.arrangedSubviews[1]),
                      ratio: Double(r))
    }

    // MARK: - pane focus / registry

    func focusPane(_ tabID: String?) {
        guard let tabID, let pane = paneRegistry.existing(tabID) else { return }
        window?.makeFirstResponder(pane.firstResponderView)
    }

    func webPane(for tabID: String) -> WebPaneView? { paneRegistry.webPane(for: tabID) }
    func dropPane(_ tabID: String) { paneRegistry.drop(tabID) }

    // MARK: - key handling

    private func installKeyMonitor() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, self.window?.isKeyWindow == true else { return event }
            // key events arrive on the main thread; assumeIsolated keeps this synchronous
            let consumed = MainActor.assumeIsolated { self.handleKey(event) }
            return consumed ? nil : event
        }
    }

    func handleKey(_ event: NSEvent) -> Bool {
        if event.keyCode == 53 { // esc
            if store.commandBarVisible { store.commandBarVisible = false; return true }
            if store.findBarVisible { store.findBarVisible = false; return true }
            return false
        }
        guard let chord = Self.chord(for: event),
              let action = Keymap.first(where: { $0.0 == chord })?.1 else { return false }
        performKeyChord(action)
        return true
    }

    static func chord(for e: NSEvent) -> KeyChord? {
        guard var key = e.charactersIgnoringModifiers?.lowercased() else { return nil }
        if e.keyCode == 48 { key = "tab" }
        if e.keyCode == 36 || e.keyCode == 76 { key = "\r" }
        if e.keyCode == 51 || e.keyCode == 117 { key = "delete" }
        let m = e.modifierFlags.intersection(.deviceIndependentFlagsMask)
        return KeyChord(key, cmd: m.contains(.command), shift: m.contains(.shift),
                        ctrl: m.contains(.control), opt: m.contains(.option))
    }

    func performKeyChord(_ action: KeyAction) {
        switch action {
        case .commandBar:
            store.commandBarMode = .command
            store.commandBarVisible.toggle()
        case .addressBar:
            store.commandBarMode = .url
            store.commandBarVisible = true
        case .closeTab:
            if let id = store.focusedTabID ?? store.activeGroup?.focusedTabID {
                dropPane(id)
                store.closeTab(id)
            }
        case .nextTab:  store.cycleTab(1)
        case .prevTab:  store.cycleTab(-1)
        case .nextSpace: store.cycleSpace(1)
        case .prevSpace: store.cycleSpace(-1)
        case .toggleSidebar: store.sidebarCollapsed.toggle()
        case .splitRight:
            if let id = store.focusedTabID { _ = store.splitTab(anchorID: id, side: .right) }
        case .splitDown:
            if let id = store.focusedTabID { _ = store.splitTab(anchorID: id, side: .below) }
        case .findInPage:
            store.findBarVisible.toggle()
        case .reload:
            if let id = store.focusedTabID { paneRegistry.webPane(for: id)?.reload() }
        case .settings:
            _ = store.openTab("hifi://settings")
        case .newTab:
            _ = store.openTab("hifi://newtab")
        case .goBack:
            if let id = store.focusedTabID { paneRegistry.webPane(for: id)?.goBack() }
        case .goForward:
            if let id = store.focusedTabID { paneRegistry.webPane(for: id)?.goForward() }
        case .pinJump1, .pinJump2, .pinJump3, .pinJump4, .pinJump5,
             .pinJump6, .pinJump7, .pinJump8, .pinJump9:
            let idx = KeyAction.allCases.firstIndex(of: action)!
                    - KeyAction.allCases.firstIndex(of: .pinJump1)!
            store.pinJump(idx)
        }
    }

    func windowWillClose(_ notification: Notification) {
        store.persistNow()
    }

    func windowDidBecomeKey(_ notification: Notification) {
        if let id = store.focusedTabID { paneRegistry.refreshOnFocus(tabID: id) }
    }
}

/// NSSplitView that reports user drags back up.
final class FocusSplitView: NSSplitView, NSSplitViewDelegate {
    var onResize: (() -> Void)?
    func splitViewDidResizeSubviews(_ notification: Notification) { onResize?() }
    func splitView(_ splitView: NSSplitView, canCollapseSubview subview: NSView) -> Bool { false }
}
