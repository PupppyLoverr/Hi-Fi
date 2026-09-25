import Cocoa
import HiFiCore

/// Cmd+F overlay: find-in-page via WebKit's window.find.
@MainActor
final class FindBarController: NSObject {
    private weak var store: BrowserStore?
    private weak var registry: PaneRegistry?
    private var bar: NSView?
    private let field = NSTextField()
    private weak var window: NSWindow?

    init(store: BrowserStore, registry: PaneRegistry) {
        self.store = store; self.registry = registry
    }

    func attach(to window: NSWindow) {
        self.window = window
        guard let content = window.contentView else { return }
        let p = store?.theme.palette ?? HFPalette(isDark: true, accentHex: "#8b7cf6")
        let fx = NSView()
        fx.wantsLayer = true; fx.layer?.cornerRadius = HFRadius.panel
        fx.layer?.borderWidth = 1
        fx.layer?.borderColor = p.borderStrong.cgColor
        fx.layer?.backgroundColor = p.surfaceDialog.cgColor
        fx.layer?.masksToBounds = true

        let icon = NSImageView()
        icon.image = HFIcon.image("magnifer", fallback: "magnifyingglass", size: 13)
        icon.contentTintColor = p.faint
        icon.translatesAutoresizingMaskIntoConstraints = false
        fx.addSubview(icon)

        field.placeholderString = "Find in page"
        field.font = .systemFont(ofSize: 12.5)
        field.isBezeled = false; field.focusRingType = .none
        field.drawsBackground = false
        field.textColor = p.text
        field.target = self
        field.action = #selector(commitAction)
        field.delegate = self
        field.translatesAutoresizingMaskIntoConstraints = false

        func iconButton(_ img: NSImage?, action: Selector) -> NSButton {
            let b = NSButton(image: img ?? NSImage(), target: self, action: action)
            b.isBordered = false; b.bezelStyle = .regularSquare
            b.contentTintColor = p.muted
            b.translatesAutoresizingMaskIntoConstraints = false
            b.widthAnchor.constraint(equalToConstant: 20).isActive = true
            fx.addSubview(b)
            return b
        }
        let prev = iconButton(HFIcon.svg("alt-arrow-up") ?? NSImage(systemSymbolName: "chevron.up", accessibilityDescription: nil), action: #selector(findPrev))
        let next = iconButton(HFIcon.svg("alt-arrow-down") ?? NSImage(systemSymbolName: "chevron.down", accessibilityDescription: nil), action: #selector(findNext))
        let close = iconButton(HFIcon.svg("close") ?? NSImage(systemSymbolName: "xmark", accessibilityDescription: nil), action: #selector(closeAction))
        _ = next // silence
        fx.addSubview(field)
        fx.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(fx)
        NSLayoutConstraint.activate([
            fx.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -18),
            fx.topAnchor.constraint(equalTo: content.topAnchor, constant: 40),
            icon.leadingAnchor.constraint(equalTo: fx.leadingAnchor, constant: 10),
            icon.centerYAnchor.constraint(equalTo: fx.centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 14),
            field.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 6),
            field.topAnchor.constraint(equalTo: fx.topAnchor, constant: 7),
            field.bottomAnchor.constraint(equalTo: fx.bottomAnchor, constant: -7),
            field.widthAnchor.constraint(equalToConstant: 190),
            prev.leadingAnchor.constraint(equalTo: field.trailingAnchor, constant: 6),
            prev.centerYAnchor.constraint(equalTo: fx.centerYAnchor),
            next.leadingAnchor.constraint(equalTo: prev.trailingAnchor, constant: 4),
            next.centerYAnchor.constraint(equalTo: fx.centerYAnchor),
            close.leadingAnchor.constraint(equalTo: next.trailingAnchor, constant: 4),
            close.trailingAnchor.constraint(equalTo: fx.trailingAnchor, constant: -8),
            close.centerYAnchor.constraint(equalTo: fx.centerYAnchor),
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
    @objc private func closeAction() { store?.findBarVisible = false }

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
