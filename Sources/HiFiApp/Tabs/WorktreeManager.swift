import Foundation
import Cocoa
import HiFiCore

/// `hifi worktree create <repo> [name]`:
///   git worktree add ~/HiFi/worktrees/<repo>/<name>
///   → copy allowlisted ignored files (hifi.toml [worktree] copy_files)
///   → create a group in the current space
///   → run .hifi/setup.sh or [scripts] setup in background with HIFI_* env.
extension BrowserStore {

    @discardableResult
    func worktreeCreate(repoPath rawRepo: String, name: String?) -> Worktree? {
        let repo = (rawRepo as NSString).expandingTildeInPath
        guard Self.isGitRepo(repo) else {
            toast("not a git repo: \(repo)")
            return nil
        }
        let repoName = URL(fileURLWithPath: repo).lastPathComponent
        let wtName = name ?? Self.nextWorktreeName(repo: repo)
        let target = HiFiPaths.worktreeRoot
            .appendingPathComponent(repoName, isDirectory: true)
            .appendingPathComponent(wtName, isDirectory: true)

        if FileManager.default.fileExists(atPath: target.path) {
            toast("worktree exists: \(target.path)")
        } else {
            let branch = "hifi/\(wtName)"
            var args = ["worktree", "add", target.path]
            if Self.branchExists(repo: repo, branch: branch) {
                args.append(branch)
            } else {
                args += ["-b", branch]
            }
            let (rc, out) = Self.git(repo: repo, args: args)
            guard rc == 0 else {
                toast("git worktree add failed: \(out.trimmed)")
                HiFiLog.write("worktree add failed: \(out)")
                return nil
            }
        }

        // copy ignored local files into the fresh worktree
        copyAllowlist(repo: repo, target: target.path)

        let wt = Worktree(repoPath: repo, name: wtName, path: target.path,
                          spaceID: state.activeSpaceID)
        let gid = createGroup(name: wtName)
        var w = wt; w.groupID = gid
        apply { st in st.worktrees.append(w) }
        persistSoon()

        // open a terminal + preview-ish diff in the new group
        _ = openTab("hifi://terminal", background: true,
                    groupID: gid, projectPath: target.path)
        _ = openTab("hifi://diff", background: true,
                    groupID: gid, projectPath: target.path)

        runSetupScript(worktree: w, groupID: gid)
        toast("worktree \(wtName) ready")
        return w
    }

    private static func isGitRepo(_ path: String) -> Bool {
        let (rc, _) = git(repo: path, args: ["rev-parse", "--git-dir"])
        return rc == 0
    }

    private static func branchExists(repo: String, branch: String) -> Bool {
        let (rc, _) = git(repo: repo, args: ["show-ref", "--verify", "--quiet", "refs/heads/\(branch)"])
        return rc == 0
    }

    private static func nextWorktreeName(repo: String) -> String {
        let fm = FileManager.default
        let base = HiFiPaths.worktreeRoot.appendingPathComponent(
            URL(fileURLWithPath: repo).lastPathComponent)
        let existing = (try? fm.contentsOfDirectory(atPath: base.path)) ?? []
        var i = existing.count + 1
        var name = "wt-\(i)"
        while fm.fileExists(atPath: base.appendingPathComponent(name).path) {
            i += 1; name = "wt-\(i)"
        }
        return name
    }

    private func copyAllowlist(repo: String, target: String) {
        let config = MiniToml.load(projectPath: repo)
        let files = config?.array("worktree", "copy_files")
            ?? [".env", ".env.local", "lumen.toml", "hifi.toml"]
        let fm = FileManager.default
        for rel in files {
            let src = URL(fileURLWithPath: repo).appendingPathComponent(rel)
            let dst = URL(fileURLWithPath: target).appendingPathComponent(rel)
            guard fm.fileExists(atPath: src.path), !fm.fileExists(atPath: dst.path) else { continue }
            try? fm.createDirectory(at: dst.deletingLastPathComponent(),
                                    withIntermediateDirectories: true)
            try? fm.copyItem(at: src, to: dst)
        }
    }

    /// `.hifi/setup.sh` or `[scripts] setup` from hifi.toml — background,
    /// env-injected, output to toast + log file.
    private func runSetupScript(worktree wt: Worktree, groupID: String) {
        let config = MiniToml.load(projectPath: wt.path)
        let scriptPath = URL(fileURLWithPath: wt.path).appendingPathComponent(".hifi/setup.sh").path
        let inline = config?.string("scripts", "setup")
        var cmd: String?
        if FileManager.default.fileExists(atPath: scriptPath) {
            cmd = "bash \(scriptPath)"
        } else if let inline {
            cmd = inline
        }
        guard let cmd else { return }

        let env = [
            "HIFI_GROUP_ID=\(groupID)",
            "HIFI_TAB_ID=",
            "HIFI_WORKSPACE_NAME=\(wt.name)",
            "HIFI_WORKSPACE_PATH=\(wt.path)",
            "HIFI_SOCKET=\(HiFiPaths.socketPath)",
        ]
        DispatchQueue.global(qos: .utility).async {
            HiFiLog.write("setup \(wt.path): \(cmd)")
            let (rc, out) = Self.shell("cd '\(wt.path)' && \(cmd)", env: env)
            let tail = out.components(separatedBy: "\n").suffix(4).joined(separator: "\n")
            HiFiLog.write("setup done rc=\(rc)\n\(tail)")
            DispatchQueue.main.async {
                self.toast(rc == 0 ? "setup finished for \(wt.name)"
                                  : "setup failed (rc=\(rc)) — see hifi.log")
            }
        }
    }

    // MARK: process helpers

    @discardableResult
    static func git(repo: String, args: [String]) -> (Int32, String) {
        shell("/usr/bin/git -C '\(repo)' \(args.map { "'\($0)'" }.joined(separator: " "))", env: [])
    }

    @discardableResult
    static func shell(_ command: String, env: [String]) -> (Int32, String) {
        let p = Process()
        let out = Pipe()
        p.executableURL = URL(fileURLWithPath: "/bin/bash")
        p.arguments = ["-lc", command]
        p.standardOutput = out
        p.standardError = out
        var e = ProcessInfo.processInfo.environment
        for kv in env {
            let parts = kv.split(separator: "=", maxSplits: 1).map(String.init)
            if parts.count == 2 { e[parts[0]] = parts[1] }
        }
        p.environment = e
        guard (try? p.run()) != nil else { return (127, "spawn failed") }
        p.waitUntilExit()
        return (p.terminationStatus,
                String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self))
    }
}
