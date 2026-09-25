import Cocoa
import SwiftTerm
import HiFiCore

/// Real PTY tab. `agentMode` launches a detected coding-agent harness
/// (claude → codex → cursor-agent → $SHELL) instead of a bare shell.
@MainActor
final class TerminalPaneView: PaneHostView {

    private let terminal: LocalProcessTerminalView
    private let inputField = NSTextField()
    private let tabID: String
    private weak var store: BrowserStore?
    private var started = false
    private let dimsLabel = NSTextField(labelWithString: "")
    private var shellName = ""
    private var worktreeName = ""

    override var firstResponderView: NSView { terminal }

    init(tab: HiFiCore.Tab, store: BrowserStore, agentMode: Bool) {
        self.tabID = tab.id
        self.store = store
        terminal = LocalProcessTerminalView(frame: .zero)
        super.init(frame: .zero)
        wantsLayer = true

        let cwd = tab.projectPath ?? FileManager.default.homeDirectoryForCurrentUser.path
        let shellPath = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        let (cmd, args, name) = agentMode
            ? Self.detectHarness(fallback: cwd)
            : (shellPath, ["-l"], URL(fileURLWithPath: shellPath).lastPathComponent)

        let p = store.theme.palette
        shellName = name
        worktreeName = URL(fileURLWithPath: cwd).lastPathComponent
        let header = makeHeader(tab: tab, cwd: cwd, name: name, agentMode: agentMode)
        header.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)

        terminal.translatesAutoresizingMaskIntoConstraints = false
        terminal.processDelegate = self
        terminal.nativeBackgroundColor = p.terminalBackground
        terminal.nativeForegroundColor = p.text
        layer?.backgroundColor = p.terminalBackground.cgColor
        addSubview(terminal)

        inputField.placeholderString = "send a line to stdin…"
        inputField.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        inputField.translatesAutoresizingMaskIntoConstraints = false
        inputField.isBezeled = false
        inputField.focusRingType = .none
        inputField.drawsBackground = false
        inputField.target = self
        inputField.action = #selector(sendLine)
        if !agentMode { inputField.isHidden = true }
        addSubview(inputField)

        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor),
            header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.topAnchor.constraint(equalTo: topAnchor),
            header.heightAnchor.constraint(equalToConstant: 26),

            terminal.leadingAnchor.constraint(equalTo: leadingAnchor),
            terminal.trailingAnchor.constraint(equalTo: trailingAnchor),
            terminal.topAnchor.constraint(equalTo: header.bottomAnchor),

            inputField.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            inputField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            inputField.topAnchor.constraint(equalTo: terminal.bottomAnchor, constant: 4),
            inputField.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
            inputField.heightAnchor.constraint(equalToConstant: 20),
        ])

        let env = Self.terminalEnv(tab: tab, cwd: cwd)
        terminal.startProcess(executable: cmd, args: args, environment: env,
                              execName: nil, currentDirectory: cwd)
        started = true
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Radius-style header: mark icon, `name — -shell — cols×rows`.
    private func makeHeader(tab: HiFiCore.Tab, cwd: String, name: String, agentMode: Bool) -> NSView {
        let p = store!.theme.palette
        let bar = NSView()
        bar.wantsLayer = true
        bar.layer?.backgroundColor = p.surfaceCard.cgColor
        bar.layer?.borderWidth = 0

        // Solar mark for the harness (claude/codex/cursor), terminal glyph for shells
        let iconName = agentMode ? Self.markIcon(for: name) : "terminal"
        let icon = NSImageView()
        icon.image = HFIcon.image(iconName, fallback: agentMode ? "sparkles" : "terminal", size: 12)
        icon.contentTintColor = agentMode ? p.accent : p.muted
        icon.translatesAutoresizingMaskIntoConstraints = false

        let base = agentMode ? "\(worktreeName) — \(name)" : "\(worktreeName) — -\(name)"
        let title = NSTextField(labelWithString: base)
        title.font = .monospacedSystemFont(ofSize: 10.5, weight: .regular)
        title.textColor = p.faint
        title.lineBreakMode = .byTruncatingMiddle
        title.translatesAutoresizingMaskIntoConstraints = false
        dimsLabel.font = .monospacedSystemFont(ofSize: 10.5, weight: .regular)
        dimsLabel.textColor = p.faint
        dimsLabel.translatesAutoresizingMaskIntoConstraints = false
        bar.addSubview(icon); bar.addSubview(title); bar.addSubview(dimsLabel)
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: bar.leadingAnchor, constant: 10),
            icon.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            title.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 8),
            dimsLabel.leadingAnchor.constraint(equalTo: title.trailingAnchor, constant: 6),
            dimsLabel.trailingAnchor.constraint(equalTo: bar.trailingAnchor, constant: -10),
            dimsLabel.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            title.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
        ])
        return bar
    }

    static func markIcon(for harness: String) -> String {
        switch harness {
        case "claude":       return "claude-mark"
        case "codex":        return "openai-mark"
        case "cursor-agent", "cursor": return "cursor-mark"
        default:             return "bot"
        }
    }

    static func terminalEnv(tab: HiFiCore.Tab, cwd: String) -> [String] {
        var env = ProcessInfo.processInfo.environment
            .map { "\($0.key)=\($0.value)" }
        env += [
            "TERM=xterm-256color",
            "HIFI_TAB_ID=\(tab.id)",
            "HIFI_WORKSPACE_PATH=\(cwd)",
            "HIFI_WORKSPACE_NAME=\(URL(fileURLWithPath: cwd).lastPathComponent)",
            "HIFI_SHELL=1",
        ]
        return env
    }

    static func detectHarness(fallback cwd: String) -> (String, [String], String) {
        let paths = (ProcessInfo.processInfo.environment["PATH"] ?? "")
            .split(separator: ":").map(String.init)
        func exists(_ name: String) -> String? {
            for p in paths {
                let full = p + "/" + name
                if FileManager.default.isExecutableFile(atPath: full) { return full }
            }
            return nil
        }
        // detection order per spec: claude → codex → cursor → shell
        if let c = exists("claude")        { return (c, [], "claude") }
        if let c = exists("codex")         { return (c, [], "codex") }
        if let c = exists("cursor-agent")  { return (c, [], "cursor-agent") }
        if let c = exists("cursor")        { return (c, ["agent"], "cursor") }
        let shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        return (shell, ["-l"], URL(fileURLWithPath: shell).lastPathComponent)
    }

    @objc private func sendLine() {
        let line = inputField.stringValue
        guard !line.isEmpty else { return }
        inputField.stringValue = ""
        terminal.send(txt: line + "\n")
    }
}

extension TerminalPaneView: LocalProcessTerminalViewDelegate {
    func processTerminated(source: TerminalView, exitCode: Int32?) {
        store?.toast("process exited \(exitCode ?? 0)")
    }
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        if let d = directory { store?.updateTab(tabID, title: URL(fileURLWithPath: d).lastPathComponent) }
    }
    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {
        dimsLabel.stringValue = "— \(newCols)×\(newRows)"
    }
    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        store?.updateTab(tabID, title: title.isEmpty ? "Terminal" : title)
    }
}
