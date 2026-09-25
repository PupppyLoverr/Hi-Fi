import Foundation

/// Tiny TOML subset reader — enough for `hifi.toml` project config:
/// `[scripts]`, `[worktree]`, string values, and string arrays.
public struct MiniToml {
    /// [section: [key: value]] — arrays become "\u{1f}"-joined strings? No: keep typed.
    public private(set) var root: [String: [String: Value]] = [:]

    public enum Value: Equatable {
        case string(String)
        case array([String])
        case bool(Bool)
        case number(Double)
    }

    public init(_ text: String) {
        var section = ""
        for rawLine in text.components(separatedBy: .newlines) {
            var line = rawLine.trimmingCharacters(in: .whitespaces)
            // strip comments (respecting quotes)
            line = MiniToml.stripComment(line)
            guard !line.isEmpty else { continue }
            if line.hasPrefix("[") && line.hasSuffix("]") {
                section = String(line.dropFirst().dropLast()).trimmingCharacters(in: .whitespaces)
                if root[section] == nil { root[section] = [:] }
                continue
            }
            guard let eq = line.firstIndex(of: "=") else { continue }
            let key = line[line.startIndex..<eq].trimmingCharacters(in: .whitespaces)
                .trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
            let val = String(line[line.index(after: eq)...]).trimmingCharacters(in: .whitespaces)
            root[section, default: [:]][key] = MiniToml.parseValue(val)
        }
    }

    static func stripComment(_ s: String) -> String {
        var inSingle = false, inDouble = false
        for (i, ch) in s.enumerated() {
            if ch == "'" && !inDouble { inSingle.toggle() }
            if ch == "\"" && !inSingle { inDouble.toggle() }
            if ch == "#" && !inSingle && !inDouble {
                return String(s.prefix(i)).trimmingCharacters(in: .whitespaces)
            }
        }
        return s
    }

    static func parseValue(_ raw: String) -> Value {
        if raw.hasPrefix("[") && raw.hasSuffix("]") {
            let inner = raw.dropFirst().dropLast()
            let items = inner.split(separator: ",").map {
                String($0).trimmingCharacters(in: .whitespaces)
                    .trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
            }.filter { !$0.isEmpty }
            return .array(items)
        }
        if raw == "true" { return .bool(true) }
        if raw == "false" { return .bool(false) }
        if let n = Double(raw) { return .number(n) }
        return .string(raw.trimmingCharacters(in: CharacterSet(charactersIn: "\"'")))
    }

    public func string(_ section: String, _ key: String) -> String? {
        guard case .string(let s)? = root[section]?[key] else { return nil }
        return s
    }
    public func array(_ section: String, _ key: String) -> [String] {
        guard case .array(let a)? = root[section]?[key] else { return [] }
        return a
    }
    public func bool(_ section: String, _ key: String, default d: Bool = false) -> Bool {
        guard case .bool(let b)? = root[section]?[key] else { return d }
        return b
    }

    /// Load `hifi.toml` (or `.hifi.toml`) from a worktree path if present.
    public static func load(projectPath: String) -> MiniToml? {
        for name in ["hifi.toml", ".hifi.toml"] {
            let url = URL(fileURLWithPath: projectPath).appendingPathComponent(name)
            if let text = try? String(contentsOf: url, encoding: .utf8) {
                return MiniToml(text)
            }
        }
        return nil
    }
}

/// Append-only file logger (setup scripts, IPC errors). Never logs secrets —
/// callers must not pass credentials.
public enum HiFiLog {
    public static func write(_ line: String) {
        try? HiFiPaths.ensureSupportDir()
        let stamp = ISO8601DateFormatter().string(from: Date())
        let entry = "[\(stamp)] \(line)\n"
        let url = HiFiPaths.logFile
        if let h = try? FileHandle(forWritingTo: url) {
            defer { try? h.close() }
            _ = try? h.seekToEnd()
            try? h.write(contentsOf: Data(entry.utf8))
        } else {
            try? entry.write(to: url, atomically: false, encoding: .utf8)
        }
    }
}
