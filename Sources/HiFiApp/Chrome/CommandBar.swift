import Cocoa
import SwiftUI
import HiFiCore

/// Shared state between the AppKit overlay controller and the SwiftUI view.
/// Selection/commit are driven by an NSEvent monitor (deterministic) rather
/// than SwiftUI key routing, which NSTextView first-responders can swallow.
@MainActor
final class CommandBarModel: ObservableObject {
    @Published var query = ""
    @Published var selected = 0

    struct Item: Identifiable {
        let id = UUID()
        let icon: String
        let title: String
        let detail: String
        let run: (BrowserStore) -> Void
    }

    func items(for store: BrowserStore) -> [Item] {
        var out: [Item] = []
        let q = query.trimmed

        if store.commandBarMode == .url {
            let current = store.focusedTab?.url ?? ""
            out.append(Item(icon: "globe", title: "Go to \(q.isEmpty ? current : q)",
                            detail: q.isEmpty ? "current page" : "") { s in
                if q.isEmpty {
                    if let id = s.focusedTabID, let pane = s.windowController?.webPane(for: id) {
                        pane.navigate(current)
                    }
                } else {
                    if let id = s.focusedTabID,
                       s.tab(id)?.kind == .web,
                       let pane = s.windowController?.webPane(for: id) {
                        pane.navigate(q)
                    } else {
                        _ = s.openTab(q)
                    }
                }
            })
            return out
        }

        // prefix commands
        if q.hasPrefix("worktree ") || q.hasPrefix("agent ") || q.hasPrefix("diff ") {
            let parts = q.split(separator: " ").map(String.init)
            if let head = parts.first, parts.count > 1 {
                let path = parts[1]
                let name = parts.count > 2 ? parts[2] : nil
                switch head {
                case "worktree":
                    out.append(Item(icon: "arrow.triangle.branch",
                                    title: "New worktree \(name ?? "")",
                                    detail: path) { s in
                        s.worktreeCreate(repoPath: path, name: name)
                    })
                case "agent":
                    out.append(Item(icon: "sparkles", title: "New agent",
                                    detail: path) { s in
                        _ = s.openTab("hifi://agent", projectPath: path)
                    })
                case "diff":
                    out.append(Item(icon: "doc.text.magnifyingglass", title: "New diff",
                                    detail: path) { s in
                        _ = s.openTab("hifi://diff", projectPath: path)
                    })
                default: break
                }
                return out
            }
        }

        // actions catalog first — typed commands should hit actions before
        // the generic open/search fallback.
        for a in Self.actions where q.isEmpty || a.title.lowercased().contains(q.lowercased()) {
            out.append(a)
        }

        // default open action
        if !q.isEmpty {
            let (kind, url) = SchemeParser.parse(q)
            out.append(Item(icon: "globe",
                            title: q.hasPrefix("hifi://") ? "Open \(q)" : "Open \(url)",
                            detail: kind == .web ? "" : "internal") { s in
                _ = s.openTab(q)
            })
        }

        // open tabs across spaces
        for sp in store.state.spaces {
            for g in sp.groups {
                for t in g.tabs {
                    let title = t.title.isEmpty ? t.url : t.title
                    if q.isEmpty || title.lowercased().contains(q.lowercased())
                        || t.url.lowercased().contains(q.lowercased()) {
                        out.append(Item(icon: "arrow.right.circle", title: "Switch to \(title)",
                                        detail: "\(sp.name) · \(t.url)") { s in
                            s.focusTab(t.id)
                        })
                    }
                }
            }
        }

        // history
        for h in store.historyEntries(matching: q, limit: 8) {
            out.append(Item(icon: "clock", title: h.title.isEmpty ? h.url : h.title,
                            detail: h.url) { s in
                _ = s.openTab(h.url)
            })
        }
        return out
    }

    static let actions: [Item] = [
        Item(icon: "plus.square", title: "New Tab", detail: "hifi://newtab") { _ = $0.openTab("hifi://newtab") },
        Item(icon: "terminal", title: "New Terminal", detail: "hifi://terminal") { _ = $0.openTab("hifi://terminal") },
        Item(icon: "sparkles", title: "New Agent", detail: "hifi://agent") { _ = $0.openTab("hifi://agent") },
        Item(icon: "doc.text.magnifyingglass", title: "New Diff", detail: "pick a repo path") { _ = $0.openTab("hifi://diff") },
        Item(icon: "rectangle.split.2x1", title: "Split Right", detail: "") { s in
            if let id = s.focusedTab?.id { _ = s.splitTab(anchorID: id, side: .right) }
        },
        Item(icon: "rectangle.split.1x2", title: "Split Down", detail: "") { s in
            if let id = s.focusedTab?.id { _ = s.splitTab(anchorID: id, side: .below) }
        },
        Item(icon: "sidebar.left", title: "Toggle Sidebar", detail: "") { $0.sidebarCollapsed.toggle() },
        Item(icon: "xmark.circle", title: "Close Tab", detail: "") { s in
            if let id = s.focusedTab?.id { s.closeTab(id) }
        },
        Item(icon: "arrow.triangle.branch", title: "New Worktree", detail: "worktree /path [name]") { _ in },
        Item(icon: "plus.rectangle", title: "New Space", detail: "") { s in
            _ = s.createSpace(name: "Space \(s.state.spaces.count + 1)")
        },
        Item(icon: "pin", title: "Pin Tab", detail: "") { s in
            if let id = s.focusedTab?.id { s.togglePin(id) }
        },
        Item(icon: "gear", title: "Open Settings", detail: "") { _ = $0.openTab("hifi://settings") },
    ]
}

/// Spotlight-style overlay: URLs, search, tab jump across spaces, actions.
@MainActor
final class CommandBarController {
    private let store: BrowserStore
    private let model = CommandBarModel()
    private var hosting: NSHostingView<CommandBarView>?
    private var container: NSVisualEffectView?
    private weak var window: NSWindow?
    private var keyMonitor: Any?

    init(store: BrowserStore) { self.store = store }

    func attach(to window: NSWindow) {
        self.window = window
        guard let content = window.contentView else { return }
        let fx = NSVisualEffectView()
        fx.material = .popover
        fx.state = .active
        fx.wantsLayer = true
        fx.layer?.cornerRadius = 12
        fx.layer?.borderWidth = 0.5
        fx.layer?.borderColor = NSColor.separatorColor.withAlphaComponent(0.4).cgColor
        fx.layer?.masksToBounds = true

        let hv = NSHostingView(rootView: CommandBarView(store: store, model: model))
        hv.translatesAutoresizingMaskIntoConstraints = false
        fx.addSubview(hv)
        NSLayoutConstraint.activate([
            hv.leadingAnchor.constraint(equalTo: fx.leadingAnchor),
            hv.trailingAnchor.constraint(equalTo: fx.trailingAnchor),
            hv.topAnchor.constraint(equalTo: fx.topAnchor),
            hv.bottomAnchor.constraint(equalTo: fx.bottomAnchor),
        ])

        fx.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(fx)
        NSLayoutConstraint.activate([
            fx.widthAnchor.constraint(equalToConstant: 560),
            fx.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            fx.topAnchor.constraint(equalTo: content.topAnchor, constant: 96),
            fx.heightAnchor.constraint(lessThanOrEqualToConstant: 420),
        ])
        fx.isHidden = true
        container = fx; hosting = hv

        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] ev in
            guard let self, self.container?.isHidden == false else { return ev }
            switch Int(ev.keyCode) {
            case 126: self.moveSelection(-1); return nil   // up
            case 125: self.moveSelection(1);  return nil   // down
            case 36, 76: self.commit(); return nil         // return/enter
            case 53:
                self.store.commandBarVisible = false
                return nil                                 // esc
            default: return ev
            }
        }
    }

    func show() {
        model.query = ""
        model.selected = 0
        container?.isHidden = false
        // Focus the real NSTextField inside the hosting view — @FocusState
        // alone doesn't steal focus from an NSTextView responder.
        DispatchQueue.main.async { [weak self] in
            guard let fx = self?.container, let field = fx.firstSubview(of: NSTextField.self)
            else { return }
            self?.window?.makeFirstResponder(field)
        }
    }

    func hide() {
        container?.isHidden = true
        window?.makeFirstResponder(store.windowController.flatMap {
            $0.paneRegistry.existing(store.focusedTabID ?? "")?.firstResponderView
        })
    }

    private func moveSelection(_ delta: Int) {
        let count = model.items(for: store).count
        guard count > 0 else { return }
        model.selected = max(0, min(count - 1, model.selected + delta))
    }

    private func commit() {
        let items = model.items(for: store)
        HiFiLog.write("cmdbar commit q=\(model.query) sel=\(model.selected) first=\(items.first?.title ?? "none") n=\(items.count)")
        guard items.indices.contains(model.selected) else { return }
        items[model.selected].run(store)
        model.query = ""
        model.selected = 0
        store.commandBarVisible = false
    }
}

struct CommandBarView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var model: CommandBarModel

    private var items: [CommandBarModel.Item] { model.items(for: store) }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: store.commandBarMode == .url ? "globe" : "command")
                    .foregroundStyle(.secondary)
                TextField(store.commandBarMode == .url
                          ? "Enter address"
                          : "Type a command, URL, or search",
                          text: $model.query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 15))
                    .onSubmit { submit() }
                    .onChange(of: model.query) { _ in model.selected = 0 }
            }
            .padding(.horizontal, 14).padding(.vertical, 11)

            Divider()

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(Array(items.prefix(14).enumerated()), id: \.element.id) { idx, item in
                            Button { choose(idx) } label: {
                                HStack(spacing: 10) {
                                    Image(systemName: item.icon)
                                        .frame(width: 16).foregroundStyle(.secondary)
                                    Text(item.title).font(.system(size: 13)).lineLimit(1)
                                    Spacer()
                                    Text(item.detail).font(.system(size: 11))
                                        .foregroundStyle(.tertiary).lineLimit(1)
                                }
                                .padding(.horizontal, 14).padding(.vertical, 7)
                                .background(idx == model.selected
                                            ? Color.accentColor.opacity(0.15) : .clear)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .id(idx)
                        }
                    }
                }
                .frame(maxHeight: 320)
                .onChange(of: model.selected) { idx in
                    withAnimation(.linear(duration: 0.05)) { proxy.scrollTo(idx) }
                }
            }
        }
    }

    private func choose(_ idx: Int) { model.selected = idx; submit() }

    private func submit() {
        HiFiLog.write("cmdbar submit q=\(model.query) sel=\(model.selected) first=\(items.first?.title ?? "none") n=\(items.count)")
        guard items.indices.contains(model.selected) else { return }
        items[model.selected].run(store)
        model.query = ""
        model.selected = 0
        store.commandBarVisible = false
    }
}

private extension NSView {
    func firstSubview<T: NSView>(of type: T.Type) -> T? {
        if let s = self as? T { return s }
        for sub in subviews {
            if let m = sub.firstSubview(of: type) { return m }
        }
        return nil
    }
}
