import Cocoa
import HiFiCore

/// Cmd+F overlay: find-in-page via WebKit's window.find.
@MainActor
final class FindBarController: NSObject {
    private weak var store: BrowserStore?
    private weak var registry: PaneRegistry?
    private var bar: NSVisualEffectView?
    private let field = NSTextField()
    private weak var window: NSWindow?

    init(store: BrowserStore, registry: PaneRegistry) {
        self.store = store; self.registry = registry
    }

    func attach(to window: NSWindow) {
        self.window = window
        guard let content = window.contentView else { return }
        let fx = NSVisualEffectView()
        fx.material = .popover; fx.state = .active
        fx.wantsLayer = true; fx.layer?.cornerRadius = 8
        fx.layer?.masksToBounds = true

        field.placeholderString = "Find in page"
        field.font = .systemFont(ofSize: 12)
        field.isBezeled = false; field.focusRingType = .none
        field.drawsBackground = false
        field.target = self
        field.action = #selector(commitAction)
        field.delegate = self
        field.translatesAutoresizingMaskIntoConstraints = false

        let next = NSButton(title: "↓", target: self, action: #selector(findNext))
        let prev = NSButton(title: "↑", target: self, action: #selector(findPrev))
        for b in [next, prev] {
            b.isBordered = false; b.font = .systemFont(ofSize: 13)
            b.translatesAutoresizingMaskIntoConstraints = false
            fx.addSubview(b)
        }
        fx.addSubview(field)
        fx.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(fx)
        NSLayoutConstraint.activate([
            fx.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -18),
            fx.topAnchor.constraint(equalTo: content.topAnchor, constant: 40),
            field.leadingAnchor.constraint(equalTo: fx.leadingAnchor, constant: 12),
            field.topAnchor.constraint(equalTo: fx.topAnchor, constant: 7),
            field.bottomAnchor.constraint(equalTo: fx.bottomAnchor, constant: -7),
            field.widthAnchor.constraint(equalToConstant: 200),
            prev.leadingAnchor.constraint(equalTo: field.trailingAnchor, constant: 6),
            prev.centerYAnchor.constraint(equalTo: fx.centerYAnchor),
            next.leadingAnchor.constraint(equalTo: prev.trailingAnchor, constant: 2),
            next.trailingAnchor.constraint(equalTo: fx.trailingAnchor, constant: -8),
            next.centerYAnchor.constraint(equalTo: fx.centerYAnchor),
        ])
        fx.isHidden = true
        bar = fx
    }

    func show() {
        bar?.isHidden = false
        window?.makeFirstResponder(field)
        field.selectText(nil)
    }
    func hide() {
        bar?.isHidden = true
        clearFind()
        window?.makeFirstResponder(nil)
    }

    @objc private func commitAction() { commit() }

    private func currentWeb() -> WebPaneView? {
        guard let id = store?.focusedTabID else { return nil }
        return registry?.webPane(for: id)
    }

    @objc private func commit() { find(backwards: false) }
    @objc private func findNext() { find(backwards: false) }
    @objc private func findPrev() { find(backwards: true) }

    private func find(backwards: Bool) {
        currentWeb()?.find(field.stringValue, backwards: backwards)
    }
    private func clearFind() {
        // move the selection past matches / clear highlight
        currentWeb()?.find("")
    }
}

extension FindBarController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        if let f = obj.object as? NSTextField, !f.stringValue.isEmpty {
            find(backwards: false)
        }
    }
    func control(_ control: NSControl, textView: NSTextView,
                 doCommandBy commandSelector: Selector) -> Bool {
        // shift-enter = previous
        if commandSelector == #selector(NSResponder.insertNewline(_:)),
           NSEvent.modifierFlags.contains(.shift) {
            find(backwards: true); return true
        }
        return false
    }
}
