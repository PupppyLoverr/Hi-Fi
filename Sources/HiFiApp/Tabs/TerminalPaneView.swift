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

    override var firstResponderView: NSView { terminal }

    init(tab: HiFiCore.Tab, store: BrowserStore, agentMode: Bool) {
        self.tabID = tab.id
        self.store = store
        terminal = LocalProcessTerminalView(frame: .zero)
        super.init(frame: .zero)
        wantsLayer = true

        let cwd = tab.projectPath ?? FileManager.default.homeDirectoryForCurrentUser.path
        let (cmd, args, name) = agentMode
            ? Self.detectHarness(fallback: cwd)
            : (ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh", ["-l"], "shell")

        let header = makeHeader(tab: tab, cwd: cwd, name: name, agentMode: agentMode)
        header.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)

        terminal.translatesAutoresizingMaskIntoConstraints = false
        terminal.processDelegate = self
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

    private func makeHeader(tab: HiFiCore.Tab, cwd: String, name: String, agentMode: Bool) -> NSView {
        let bar = NSView()
        bar.wantsLayer = true
        bar.layer?.backgroundColor = NSColor.windowBackgroundColor.withAlphaComponent(0.4).cgColor
        let icon = NSTextField(labelWithString: agentMode ? "✦" : "❯")
        icon.font = .systemFont(ofSize: 11)
        icon.textColor = .secondaryLabelColor
        icon.translatesAutoresizingMaskIntoConstraints = false
        let title = NSTextField(labelWithString:
            agentMode ? "\(name)  ·  \(cwd)" : "\(cwd)")
        title.font = .monospacedSystemFont(ofSize: 10.5, weight: .regular)
        title.textColor = .secondaryLabelColor
        title.lineBreakMode = .byTruncatingMiddle
        title.translatesAutoresizingMaskIntoConstraints = false
        bar.addSubview(icon); bar.addSubview(title)
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: bar.leadingAnchor, constant: 10),
            icon.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            title.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 8),
            title.trailingAnchor.constraint(equalTo: bar.trailingAnchor, constant: -10),
            title.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
        ])
        return bar
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
        return (shell, ["-l"], "shell")
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
    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}
    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        store?.updateTab(tabID, title: title.isEmpty ? "Terminal" : title)
    }
}
