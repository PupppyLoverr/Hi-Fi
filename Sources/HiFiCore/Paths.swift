import Foundation

/// On-disk locations for the app, CLI and user data.
public enum HiFiPaths {
    public static var supportDir: URL {
        let env = ProcessInfo.processInfo.environment["HIFI_DATA_DIR"]
        if let env, !env.isEmpty {
            return URL(fileURLWithPath: (env as NSString).expandingTildeInPath)
        }
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        return appSupport.appendingPathComponent("HiFi", isDirectory: true)
    }

    public static var socketPath: String {
        if let env = ProcessInfo.processInfo.environment["HIFI_SOCKET"], !env.isEmpty { return env }
        return supportDir.appendingPathComponent("ipc.sock").path
    }

    public static var stateFile: URL   { supportDir.appendingPathComponent("state.json") }
    public static var historyFile: URL { supportDir.appendingPathComponent("history.json") }
    public static var logFile: URL     { supportDir.appendingPathComponent("hifi.log") }

    public static var worktreeRoot: URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return home.appendingPathComponent("HiFi/worktrees", isDirectory: true)
    }

    public static func ensureSupportDir() throws {
        try FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
    }
}
