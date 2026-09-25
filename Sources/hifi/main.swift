import Foundation
import HiFiCore

// MARK: - hifi CLI
// Talks to the running Hi-Fi app over its UNIX socket.
// If the app isn't running, launches it and waits for the socket.

struct CLI {
    var jsonMode = false
    var launchIfNeeded = true

    mutating func run(_ argv: [String]) -> Int32 {
        var args = Array(argv.dropFirst())
        args = args.filter { arg in
            if arg == "--json" { jsonMode = true; return false }
            if arg == "--no-launch" { launchIfNeeded = false; return false }
            return true
        }
        guard let cmd = args.first else { usage(); return 64 }
        args.removeFirst()

        do {
            let request = try buildRequest(cmd: cmd, args: args)
            let response = try send(request)
            return printResult(response)
        } catch let e as CLIError {
            FileHandle.standardError.write(Data("hifi: \(e.message)\n".utf8))
            return 1
        } catch {
            FileHandle.standardError.write(Data("hifi: \(error)\n".utf8))
            return 1
        }
    }

    // MARK: request building

    struct CLIError: Error { let message: String }
    static func fail(_ m: String) -> CLIError { CLIError(message: m) }

    func need(_ value: String?, _ what: String) throws -> String {
        guard let v = value else { throw CLI.fail("missing \(what)") }
        return v
    }

    /// pull `--flag VALUE` or `--flag=VALUE` out of args
    func opt(_ args: inout [String], _ name: String) -> String? {
        for (i, a) in args.enumerated() {
            if a == "--\(name)" {
                args.remove(at: i)
                guard i < args.count else { return "" }
                return args.remove(at: i)
            }
            if a.hasPrefix("--\(name)=") {
                args.remove(at: i)
                return String(a.dropFirst(name.count + 3))
            }
        }
        return nil
    }
    func flag(_ args: inout [String], _ name: String) -> Bool {
        if let i = args.firstIndex(of: "--\(name)") { args.remove(at: i); return true }
        return false
    }
    func sizeOpt(_ args: inout [String]) throws -> Double? {
        guard let s = opt(&args, "size") else { return nil }
        let trimmed = s.hasSuffix("%") ? String(s.dropLast()) : s
        guard let v = Double(trimmed) else { throw CLI.fail("bad --size \(s)") }
        return v > 1 ? v / 100.0 : v
    }

    func buildRequest(cmd: String, args rawArgs: [String]) throws -> IPCRequest {
        var args = rawArgs
        switch cmd {
        case "ping":
            return IPCRequest(command: IPCCommand.ping)

        case "tab":
            guard let sub = args.first else { throw CLI.fail("tab <open|list|focus|close|split>") }
            args.removeFirst()
            switch sub {
            case "open":
                let url = try need(args.first, "url")
                args.removeFirst()
                var a: [String: JSONValue] = ["url": .str(url)]
                if let v = opt(&args, "right-of") { a["right_of"] = .str(v) }
                if let v = opt(&args, "left-of")  { a["left_of"] = .str(v) }
                if let v = opt(&args, "above")    { a["above"] = .str(v) }
                if let v = opt(&args, "below")    { a["below"] = .str(v) }
                if let v = try sizeOpt(&args)     { a["size"] = .number(v) }
                if flag(&args, "background")      { a["background"] = .bool(true) }
                if flag(&args, "keep-anchor-active") { a["keep_anchor_active"] = .bool(true) }
                if let v = opt(&args, "group")    { a["group"] = .str(v) }
                if let v = opt(&args, "project-path") { a["project_path"] = .str(v) }
                if let v = opt(&args, "profile")  { a["profile"] = .str(v) }
                return IPCRequest(command: IPCCommand.tabOpen, args: a)

            case "list":
                var a: [String: JSONValue] = [:]
                if flag(&args, "window")       { a["window"] = .bool(true) }
                if let g = opt(&args, "group") { a["group"] = .str(g) }
                return IPCRequest(command: IPCCommand.tabList, args: a)

            case "focus":
                return IPCRequest(command: IPCCommand.tabFocus,
                                  args: ["id": .str(try need(args.first, "tab id"))])
            case "close":
                return IPCRequest(command: IPCCommand.tabClose,
                                  args: ["id": .str(try need(args.first, "tab id"))])
            case "split":
                let id = try need(args.first, "tab id"); args.removeFirst()
                var a: [String: JSONValue] = ["id": .str(id)]
                if let v = opt(&args, "right")  { a["right"] = .str(v) }
                if let v = opt(&args, "left")   { a["left"] = .str(v) }
                if let v = opt(&args, "above")  { a["above"] = .str(v) }
                if let v = opt(&args, "below")  { a["below"] = .str(v) }
                if let v = try sizeOpt(&args)   { a["size"] = .number(v) }
                if flag(&args, "move-between-groups") { a["move_between_groups"] = .bool(true) }
                return IPCRequest(command: IPCCommand.tabSplit, args: a)
            default:
                throw CLI.fail("unknown tab subcommand \(sub)")
            }

        case "group":
            guard let sub = args.first else { throw CLI.fail("group <create|rename|show>") }
            args.removeFirst()
            switch sub {
            case "create":
                let name = try need(args.first, "name"); args.removeFirst()
                var a: [String: JSONValue] = ["name": .str(name)]
                if let t = opt(&args, "tab") { a["tab"] = .str(t) }
                return IPCRequest(command: IPCCommand.groupCreate, args: a)
            case "rename":
                let id = try need(args.first, "id")
                let name = try need(args.dropFirst(2).first ?? (args.count > 1 ? args[1] : nil), "name")
                return IPCRequest(command: IPCCommand.groupRename,
                                  args: ["id": .str(id), "name": .str(name)])
            case "show":
                return IPCRequest(command: IPCCommand.groupShow,
                                  args: ["id": .str(try need(args.dropFirst().first ?? args.first, "id"))])
            default:
                throw CLI.fail("unknown group subcommand \(sub)")
            }

        case "space":
            guard let sub = args.first else { throw CLI.fail("space <list|create|switch>") }
            args.removeFirst()
            switch sub {
            case "list":
                return IPCRequest(command: IPCCommand.spaceList)
            case "create":
                return IPCRequest(command: IPCCommand.spaceCreate,
                                  args: ["name": .str(try need(args.first, "name"))])
            case "switch":
                return IPCRequest(command: IPCCommand.spaceSwitch,
                                  args: ["id": .str(try need(args.first, "id or name"))])
            default:
                throw CLI.fail("unknown space subcommand \(sub)")
            }

        case "worktree":
            guard args.first == "create" else { throw CLI.fail("worktree create <repo-path> [name]") }
            args.removeFirst()
            let repo = try need(args.first, "repo-path"); args.removeFirst()
            var a: [String: JSONValue] = ["repo_path": .str(repo)]
            if let n = args.first { a["name"] = .str(n) }
            return IPCRequest(command: IPCCommand.worktreeCreate, args: a)

        case "agent":
            guard args.first == "start" else { throw CLI.fail("agent start [--project-path P] [--harness H]") }
            args.removeFirst()
            var a: [String: JSONValue] = [:]
            if let v = opt(&args, "project-path") { a["project_path"] = .str(v) }
            if let v = opt(&args, "harness")      { a["harness"] = .str(v) }
            return IPCRequest(command: IPCCommand.agentStart, args: a)

        case "page":
            guard let sub = args.first else { throw CLI.fail("page <snapshot|click|type|press|url|html|screenshot>") }
            args.removeFirst()
            var a: [String: JSONValue] = [:]
            func rest() throws -> [String] { args }
            switch sub {
            case "snapshot", "url", "html":
                if let t = opt(&args, "tab") { a["tab"] = .str(t) }
                return IPCRequest(command: "page.\(sub)", args: a)
            case "screenshot":
                if let t = opt(&args, "tab") { a["tab"] = .str(t) }
                let path = try need(rest().last, "output path")
                a["path"] = .str(path)
                return IPCRequest(command: IPCCommand.pageScreenshot, args: a)
            case "click":
                if let t = opt(&args, "tab") { a["tab"] = .str(t) }
                a["target"] = .str(try need(args.first, "@eN or css"))
                return IPCRequest(command: IPCCommand.pageClick, args: a)
            case "type":
                if let t = opt(&args, "tab") { a["tab"] = .str(t) }
                let target = try need(args.first, "@eN or css"); args.removeFirst()
                a["target"] = .str(target)
                a["text"] = .str(args.joined(separator: " "))
                return IPCRequest(command: IPCCommand.pageType, args: a)
            case "press":
                if let t = opt(&args, "tab") { a["tab"] = .str(t) }
                a["key"] = .str(try need(args.first, "key"))
                return IPCRequest(command: IPCCommand.pagePress, args: a)
            default:
                throw CLI.fail("unknown page subcommand \(sub)")
            }

        default:
            throw CLI.fail("unknown command \(cmd)")
        }
    }

    // MARK: transport

    func send(_ request: IPCRequest) throws -> IPCResponse {
        let path = HiFiPaths.socketPath
        let payload = try request.encode()
        do {
            let data = try UnixSocketClient.request(path: path, payload: payload)
            return try JSONDecoder().decode(IPCResponse.self, from: data)
        } catch {
            guard launchIfNeeded else { throw error }
            try launchApp()
            // wait up to 8s for the socket
            let deadline = Date().addingTimeInterval(8)
            while Date() < deadline {
                if let data = try? UnixSocketClient.request(path: path, payload: payload, timeout: 2),
                   let resp = try? JSONDecoder().decode(IPCResponse.self, from: data) {
                    return resp
                }
                Thread.sleep(forTimeInterval: 0.25)
            }
            throw CLI.fail("no response from Hi-Fi within 8s (app may be busy — check ~/Library/Application Support/HiFi/hifi.log)")
        }
    }

    func launchApp() throws {
        let candidates = [
            ProcessInfo.processInfo.environment["HIFI_APP"],
            Bundle.main.bundleURL.path.hasSuffix(".app") ? Bundle.main.bundleURL.path : nil,
            // CLI inside .app bundle: <App>.app/Contents/MacOS/hifi -> app root
            Bundle.main.bundleURL
                .deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().path.hasSuffix(".app")
                ? Bundle.main.bundleURL
                    .deletingLastPathComponent().deletingLastPathComponent()
                    .deletingLastPathComponent().path
                : nil,
            "/Applications/Hi-Fi.app",
            FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("Applications/Hi-Fi.app").path,
            // dev build location used by scripts/bundle.sh
            FileManager.default.currentDirectoryPath + "/build/Hi-Fi.app"
        ].compactMap { $0 }
        for app in candidates {
            if FileManager.default.fileExists(atPath: app) {
                let p = Process()
                p.executableURL = URL(fileURLWithPath: "/usr/bin/open")
                p.arguments = [app] // no -n: activate the running instance if there is one
                try? p.run()
                return
            }
        }
        // try bundle id
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        p.arguments = ["-b", "com.hifi.browser"]
        try? p.run()
    }

    // MARK: output

    func printResult(_ r: IPCResponse) -> Int32 {
        if jsonMode {
            var out: [String: JSONValue] = ["ok": .bool(r.ok)]
            if let res = r.result { out["result"] = res }
            if let err = r.error  { out["error"] = .str(err) }
            if let data = try? JSONEncoder().encode(JSONValue.object(out)) {
                print(String(decoding: data, as: UTF8.self))
            }
            return r.ok ? 0 : 1
        }
        if !r.ok {
            FileHandle.standardError.write(Data("hifi: \(r.error ?? "error")\n".utf8))
            return 1
        }
        if let res = r.result { printHuman(res) }
        return 0
    }

    func printHuman(_ v: JSONValue) {
        switch v {
        case .string(let s): print(s)
        case .number(let n):
            print(n.truncatingRemainder(dividingBy: 1) == 0 ? String(Int(n)) : String(n))
        case .bool(let b):   print(b)
        case .array(let a):
            for item in a { printHuman(item) }
        case .object(let o):
            // compact table for tab/group/space lists
            if let fields = o["rows"]?.array {
                let cols = ["id", "kind", "name", "title", "url", "group", "space", "active", "pinned", "groups"]
                for row in fields {
                    switch row {
                    case .array(let cols2):
                        print(cols2.map { $0.string ?? "" }.joined(separator: "\t"))
                    case .object(let obj):
                        let known = cols.compactMap { obj[$0]?.string }
                        let extra = obj.keys.filter { !cols.contains($0) }.sorted()
                            .compactMap { "\($0)=\(obj[$0]?.string ?? "")" }
                        print((known + extra).joined(separator: "\t"))
                    default:
                        print(row.string ?? "")
                    }
                }
            } else if let data = try? JSONEncoder().encode(v) {
                print(String(decoding: data, as: UTF8.self))
            }
        case .null: break
        }
    }

    func usage() {
        let u = """
        hifi — Hi-Fi browser control

          hifi tab open <url|hifi://...> [--right-of ID|--left-of ID|--above ID|--below ID]
                   [--size 40%] [--background] [--keep-anchor-active]
                   [--group ID] [--project-path PATH] [--profile NAME]
          hifi tab list [--window] [--group ID]
          hifi tab focus ID | close ID
          hifi tab split ID [--right|--left|--above|--below ID] [--size 40%] [--move-between-groups]
          hifi group create NAME [--tab ID] | rename ID NAME | show ID
          hifi space list | create NAME | switch ID|NAME
          hifi worktree create <repo-path> [name]
          hifi agent start [--project-path PATH] [--harness claude|codex|cursor|shell]
          hifi page snapshot|url|html [--tab ID]
          hifi page click @eN|CSS | type @eN|CSS TEXT | press KEY
          hifi page screenshot [--tab ID] PATH
          hifi ping        # is the app up?

        Global: --json  (machine-readable output)  --no-launch (don't start the app)
        """
        print(u)
    }
}

var cli = CLI()
exit(cli.run(CommandLine.arguments))
