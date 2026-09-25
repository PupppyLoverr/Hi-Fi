import Cocoa
import HiFiCore

/// git status + git diff viewer for a worktree/repo path.
/// Left: changed-file list. Right: unified diff. Refresh on focus.
@MainActor
final class DiffPaneView: PaneHostView {

    private weak var store: BrowserStore?
    private let tabID: String
    private var repoPath: String
    private let fileList = NSTableView()
    private var files: [(status: String, path: String)] = []
    private let diffText = NSTextView()
    private let statusLabel = NSTextField(labelWithString: "")
    private let pathField = NSTextField()

    override var firstResponderView: NSView { diffText }

    init(tab: HiFiCore.Tab, store: BrowserStore) {
        self.store = store
        self.tabID = tab.id
        self.repoPath = tab.projectPath ?? ""
        super.init(frame: .zero)
        wantsLayer = true
        buildUI()
        refresh()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func buildUI() {
        // header
        let header = NSView()
        header.wantsLayer = true
        header.layer?.backgroundColor = NSColor.windowBackgroundColor.withAlphaComponent(0.4).cgColor
        header.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)

        pathField.placeholderString = "/path/to/repo"
        pathField.stringValue = repoPath
        pathField.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        pathField.isBezeled = false
        pathField.focusRingType = .none
        pathField.drawsBackground = false
        pathField.translatesAutoresizingMaskIntoConstraints = false
        pathField.target = self
        pathField.action = #selector(pathChanged)
        header.addSubview(pathField)

        let refreshBtn = NSButton(title: "⟳", target: self, action: #selector(refresh))
        refreshBtn.isBordered = false
        refreshBtn.font = .systemFont(ofSize: 13)
        refreshBtn.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(refreshBtn)

        statusLabel.font = .systemFont(ofSize: 10)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(statusLabel)

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            header.leadingAnchor.constraint(equalTo: leadingAnchor),
            header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 26),
            pathField.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 10),
            pathField.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            pathField.trailingAnchor.constraint(equalTo: statusLabel.leadingAnchor, constant: -8),
            statusLabel.trailingAnchor.constraint(equalTo: refreshBtn.leadingAnchor, constant: -8),
            statusLabel.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            refreshBtn.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -8),
            refreshBtn.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            refreshBtn.widthAnchor.constraint(equalToConstant: 24),
        ])

        // split: file list | diff
        let split = NSSplitView()
        split.isVertical = true
        split.dividerStyle = .thin
        split.translatesAutoresizingMaskIntoConstraints = false
        addSubview(split)
        NSLayoutConstraint.activate([
            split.topAnchor.constraint(equalTo: header.bottomAnchor),
            split.leadingAnchor.constraint(equalTo: leadingAnchor),
            split.trailingAnchor.constraint(equalTo: trailingAnchor),
            split.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        let leftScroll = NSScrollView()
        leftScroll.hasVerticalScroller = true
        let col = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("f"))
        fileList.addTableColumn(col)
        fileList.headerView = nil
        fileList.dataSource = self
        fileList.delegate = self
        fileList.rowSizeStyle = .small
        fileList.intercellSpacing = NSSize(width: 0, height: 2)
        fileList.target = self
        fileList.doubleAction = #selector(filePicked)
        leftScroll.documentView = fileList

        let rightScroll = NSScrollView()
        rightScroll.hasVerticalScroller = true
        diffText.isEditable = false
        diffText.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        diffText.textContainerInset = NSSize(width: 8, height: 8)
        diffText.isVerticallyResizable = true
        diffText.isHorizontallyResizable = false
        diffText.textContainer?.widthTracksTextView = true
        diffText.backgroundColor = .textBackgroundColor
        rightScroll.documentView = diffText

        split.addArrangedSubview(leftScroll)
        split.addArrangedSubview(rightScroll)
        DispatchQueue.main.async { [weak self, weak split] in
            guard let split else { return }
            self.map { _ in split.setPosition(220, ofDividerAt: 0) }
        }
    }

    @objc private func pathChanged() {
        repoPath = pathField.stringValue
        store?.apply { _ in }
        store?.updateTabProjectPath(tabID, path: repoPath)
        refresh()
    }

    @objc func refresh() {
        guard !repoPath.isEmpty else {
            statusLabel.stringValue = "set repo path ↵"
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let files = Self.gitStatus(path: self?.repoPath ?? "")
            let branch = Self.git(path: self?.repoPath ?? "", args: ["rev-parse", "--abbrev-ref", "HEAD"])
            DispatchQueue.main.async {
                guard let self else { return }
                self.files = files
                self.fileList.reloadData()
                self.statusLabel.stringValue =
                    files.isEmpty ? "clean · \(branch.trimmed)" : "\(files.count) changed · \(branch.trimmed)"
                if let first = files.first { self.showDiff(for: first.path) }
                else { self.diffText.string = "working tree clean\n" }
            }
        }
    }

    override func refreshOnFocus() { refresh() }

    @objc private func filePicked() {
        let row = fileList.clickedRow
        guard files.indices.contains(row) else { return }
        showDiff(for: files[row].path)
    }

    private func showDiff(for file: String) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            var out = Self.git(path: self.repoPath, args: ["diff", "--", file])
            if out.trimmed.isEmpty {
                out = Self.git(path: self.repoPath, args: ["diff", "--cached", "--", file])
            }
            if out.trimmed.isEmpty {
                out = Self.git(path: self.repoPath,
                               args: ["status", "--porcelain", "--", file]) + "\n(untracked or unchanged)"
            }
            let attributed = Self.colorize(out)
            DispatchQueue.main.async { self.diffText.textStorage?.setAttributedString(attributed) }
        }
    }

    // MARK: git helpers

    static func gitStatus(path: String) -> [(status: String, path: String)] {
        let out = git(path: path, args: ["status", "--porcelain"])
        return out.components(separatedBy: "\n").compactMap { line in
            guard line.count > 3 else { return nil }
            let st = String(line.prefix(2)).trimmed
            let p = String(line.dropFirst(3))
            return (st, p)
        }
    }

    static func git(path: String, args: [String]) -> String {
        let p = Process()
        let out = Pipe()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        p.arguments = ["-C", path] + args
        p.standardOutput = out
        p.standardError = FileHandle.nullDevice
        guard (try? p.run()) != nil else { return "" }
        p.waitUntilExit()
        return String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
    }

    /// +/- green/red highlighting for unified diffs.
    static func colorize(_ text: String) -> NSAttributedString {
        let attr = NSMutableAttributedString()
        let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        for line in text.components(separatedBy: "\n") {
            var color = NSColor.textColor
            if line.hasPrefix("+") && !line.hasPrefix("+++") { color = .systemGreen }
            else if line.hasPrefix("-") && !line.hasPrefix("---") { color = .systemRed }
            else if line.hasPrefix("@@") { color = .systemPurple }
            else if line.hasPrefix("diff ") || line.hasPrefix("index ") { color = .secondaryLabelColor }
            attr.append(NSAttributedString(string: line + "\n",
                                           attributes: [.font: font, .foregroundColor: color]))
        }
        return attr
    }
}

extension DiffPaneView: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int { files.count }
    func tableView(_ tableView: NSTableView,
                   viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let f = files[row]
        let cell = NSTextField(labelWithString: " \(f.status)  \(f.path)")
        cell.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        cell.textColor = f.status.contains("?") ? .systemOrange : .labelColor
        cell.lineBreakMode = .byTruncatingMiddle
        return cell
    }
    func tableViewSelectionDidChange(_ notification: Notification) {
        let row = fileList.selectedRow
        guard files.indices.contains(row) else { return }
        showDiff(for: files[row].path)
    }
}

extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
