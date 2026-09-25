import Cocoa
import HiFiCore

/// UNIX-socket IPC server. Every `hifi` CLI command lands here, is routed
/// to the store on the main thread, and returns one JSON line.
@MainActor
final class IPCServer {
    private let store: BrowserStore
    private let server = UnixSocketServer(path: HiFiPaths.socketPath)

    init(store: BrowserStore) { self.store = store }

    func start() throws {
        server.onRequest = { [weak self] data in
            guard let self else { return Data() }
            return self.serve(data)
        }
        try server.start()
        HiFiLog.write("IPC listening on \(HiFiPaths.socketPath)")
    }

    func stop() { server.stop() }

    // Runs on the socket queue. UI ops bounce to main; async ops use a semaphore.
    private func serve(_ data: Data) -> Data {
        let line = data.split(separator: 0x0A).first.map { Data($0) } ?? data
        guard let req = try? JSONDecoder().decode(IPCRequest.self, from: line) else {
            return Self.encode(IPCResponse(id: "", ok: false, error: "bad request"))
        }
        var result: IPCResponse?
        let sem = DispatchSemaphore(value: 0)
        Task { @MainActor in
            result = await self.handle(req)
            sem.signal()
        }
        if sem.wait(timeout: .now() + 30) == .timedOut {
            return Self.encode(IPCResponse(id: req.id, ok: false, error: "timed out"))
        }
        return Self.encode(result ?? IPCResponse(id: req.id, ok: false, error: "no result"))
    }

    private static func encode(_ r: IPCResponse) -> Data {
        (try? r.encode()) ?? Data("{\"ok\":false}\n".utf8)
    }

    // MARK: - dispatch

    private func handle(_ req: IPCRequest) async -> IPCResponse {
        let a = req.args
        do {
            switch req.command {
            case IPCCommand.ping:
                return ok(req, .str("pong"))

            // ---------- tabs ----------
            case IPCCommand.tabOpen:
                guard let url = a["url"]?.string else { throw bad("url required") }
                var anchor: (id: String, side: SplitSide)?
                for (key, side) in [("right_of", SplitSide.right), ("left_of", .left),
                                    ("above", .above), ("below", .below)] {
                    if let v = a[key]?.string { anchor = (v, side); break }
                }
                let id = store.openTab(
                    url,
                    sideOf: anchor?.id, side: anchor?.side,
                    size: a["size"]?.numberValue,
                    background: a["background"] == .bool(true),
                    keepAnchor: a["keep_anchor_active"] == .bool(true),
                    groupID: a["group"]?.string,
                    projectPath: a["project_path"]?.string,
                    profileName: a["profile"]?.string)
                return ok(req, .str(id))

            case IPCCommand.tabList:
                return ok(req, store.tabListJSON(groupID: a["group"]?.string))

            case IPCCommand.tabFocus:
                guard let id = a["id"]?.string else { throw bad("id required") }
                store.focusTab(id)
                return ok(req, .str(id))

            case IPCCommand.tabClose:
                guard let id = a["id"]?.string else { throw bad("id required") }
                store.windowController?.dropPane(id)
                store.closeTab(id)
                return ok(req, .str(id))

            case IPCCommand.tabSplit:
                guard let id = a["id"]?.string else { throw bad("id required") }
                var other: String?
                var side: SplitSide = .right
                for (key, s) in [("right", SplitSide.right), ("left", .left),
                                 ("above", .above), ("below", .below)] {
                    if let v = a[key]?.string { other = v; side = s; break }
                }
                guard let newID = store.splitTab(
                    anchorID: id, side: side, with: other,
                    size: a["size"]?.numberValue,
                    moveBetweenGroups: a["move_between_groups"] == .bool(true))
                else { throw bad("split failed (need --move-between-groups?)") }
                return ok(req, .str(newID))

            // ---------- groups ----------
            case IPCCommand.groupCreate:
                guard let name = a["name"]?.string else { throw bad("name required") }
                let id = store.createGroup(name: name, moveTabID: a["tab"]?.string)
                return ok(req, .str(id))

            case IPCCommand.groupRename:
                guard let id = a["id"]?.string, let name = a["name"]?.string
                else { throw bad("id + name required") }
                store.renameGroup(id, name: name)
                return ok(req, .str(id))

            case IPCCommand.groupShow:
                guard let id = a["id"]?.string else { throw bad("id required") }
                guard let j = store.groupJSON(id) else { throw bad("group not found") }
                return ok(req, j)

            // ---------- spaces ----------
            case IPCCommand.spaceList:
                return ok(req, store.spaceListJSON())

            case IPCCommand.spaceCreate:
                guard let name = a["name"]?.string else { throw bad("name required") }
                return ok(req, .str(store.createSpace(name: name)))

            case IPCCommand.spaceSwitch:
                guard let id = a["id"]?.string else { throw bad("id required") }
                guard store.switchSpace(id) else { throw bad("space not found") }
                return ok(req, .str(id))

            // ---------- worktree / agent ----------
            case IPCCommand.worktreeCreate:
                guard let repo = a["repo_path"]?.string else { throw bad("repo_path required") }
                return try worktreeCreate(req, repo: repo, name: a["name"]?.string)

            case IPCCommand.agentStart:
                let id = store.openTab("hifi://agent",
                                       projectPath: a["project_path"]?.string
                                            ?? FileManager.default.currentDirectoryPath,
                                       harness: a["harness"]?.string)
                return ok(req, .str(id))

            // ---------- page control ----------
            case IPCCommand.pageSnapshot, IPCCommand.pageClick, IPCCommand.pageType,
                 IPCCommand.pagePress, IPCCommand.pageURL, IPCCommand.pageHTML,
                 IPCCommand.pageScreenshot:
                return try await pageCommand(req)

            default:
                throw bad("unknown command \(req.command)")
            }
        } catch let e as IPCError {
            return IPCResponse(id: req.id, ok: false, error: e.message)
        } catch {
            return IPCResponse(id: req.id, ok: false, error: "\(error)")
        }
    }

    // MARK: - page commands with permission gate

    private func pageCommand(_ req: IPCRequest) async throws -> IPCResponse {
        let a = req.args
        let tabID = a["tab"]?.string ?? store.focusedTabID
        HiFiLog.write("pageCommand \(req.command) tab=\(tabID ?? "nil")")
        guard let tabID, let tab = store.tab(tabID) else {
            throw bad("no such tab")
        }
        guard tab.kind == .web || tab.kind == .preview else {
            throw bad("tab \(tabID) is not a web page")
        }
        guard let (space, group) = store.groupOf(tabID: tabID) else { throw bad("orphaned tab") }

        // permission: one prompt per group
        let allowed = store.ensureAgentPermission(groupID: group.id) {
            let alert = NSAlert()
            alert.messageText = "Allow agent control?"
            alert.informativeText = "A CLI client wants to read and act on “\(tab.title)” in “\(space.name) / \(group.name.isEmpty ? "Tabs" : group.name)”. Allow for this group from now on?"
            alert.addButton(withTitle: "Allow")
            alert.addButton(withTitle: "Deny")
            return alert.runModal() == .alertFirstButtonReturn
        }
        guard allowed else { throw bad("agent control denied for this group") }

        guard let pane = store.windowController?.webPane(for: tabID) else {
            throw bad("web pane not loaded (focus the tab first)")
        }
        HiFiLog.write("pageCommand pane ok, evalJS")

        switch req.command {
        case IPCCommand.pageSnapshot:
            let raw = try await evalString(pane, JSBridge.snapshot)
            guard let data = raw.data(using: .utf8),
                  let j = try? JSONDecoder().decode(JSONValue.self, from: data) else {
                throw bad("snapshot parse failed")
            }
            return ok(req, j)

        case IPCCommand.pageClick:
            guard let t = a["target"]?.string else { throw bad("target required") }
            return try await simple(pane, req, JSBridge.click(target: t))

        case IPCCommand.pageType:
            guard let t = a["target"]?.string else { throw bad("target required") }
            let text = a["text"]?.string ?? ""
            return try await simple(pane, req, JSBridge.typeText(target: t, text: text))

        case IPCCommand.pagePress:
            guard let k = a["key"]?.string else { throw bad("key required") }
            return try await simple(pane, req, JSBridge.press(key: k))

        case IPCCommand.pageURL:
            return ok(req, .str(try await evalString(pane, JSBridge.currentURL)))

        case IPCCommand.pageHTML:
            return ok(req, .str(try await evalString(pane, JSBridge.pageHTML)))

        case IPCCommand.pageScreenshot:
            guard let path = a["path"]?.string else { throw bad("path required") }
            // WKWebView suspends rendering while occluded — takeSnapshot never
            // completes then, so raise the window first.
            if let win = store.windowController?.window {
                NSApp.activate(ignoringOtherApps: true)
                win.makeKeyAndOrderFront(nil)
                try await Task.sleep(nanoseconds: 200_000_000)
            }
            let png = try await pane.snapshotPNG()
            let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
            try png.write(to: url)
            return ok(req, .str(url.path))

        default:
            throw bad("unreachable")
        }
    }

    private func evalString(_ pane: WebPaneView, _ js: String) async throws -> String {
        let r = try await pane.evalJS(js)
        if let s = r as? String { return s }
        return String(describing: r ?? "")
    }

    private func simple(_ pane: WebPaneView, _ req: IPCRequest, _ js: String) async throws -> IPCResponse {
        let r = try await evalString(pane, js)
        if r.hasPrefix("ERR:") { throw bad(String(r.dropFirst(4))) }
        return ok(req, .str(r))
    }

    // MARK: - worktrees

    private func worktreeCreate(_ req: IPCRequest, repo: String, name: String?) throws -> IPCResponse {
        guard let result = store.worktreeCreate(repoPath: repo, name: name) else {
            throw bad("worktree create failed — is \(repo) a git repo?")
        }
        return ok(req, .object([
            "worktree": .str(result.path),
            "group": .str(result.groupID ?? ""),
            "space": .str(result.spaceID ?? ""),
        ]))
    }

    // MARK: - helpers

    private struct IPCError: Error { let message: String }
    private func bad(_ m: String) -> IPCError { IPCError(message: m) }
    private func ok(_ req: IPCRequest, _ v: JSONValue) -> IPCResponse {
        IPCResponse(id: req.id, ok: true, result: v)
    }
}

extension JSONValue {
    var numberValue: Double? {
        if case .number(let n) = self { return n }
        return nil
    }
}
