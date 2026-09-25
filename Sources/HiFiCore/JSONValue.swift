import Foundation

/// Type-erased JSON used on the IPC wire so the CLI and app share one schema.
public enum JSONValue: Codable, Equatable, Sendable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case array([JSONValue])
    case object([String: JSONValue])
    case null

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self)  { self = .string(s); return }
        if let b = try? c.decode(Bool.self)    { self = .bool(b); return }
        if let n = try? c.decode(Double.self)  { self = .number(n); return }
        if let a = try? c.decode([JSONValue].self) { self = .array(a); return }
        if let o = try? c.decode([String: JSONValue].self) { self = .object(o); return }
        self = .null
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .string(let s): try c.encode(s)
        case .number(let n): try c.encode(n)
        case .bool(let b):   try c.encode(b)
        case .array(let a):  try c.encode(a)
        case .object(let o): try c.encode(o)
        case .null:          try c.encodeNil()
        }
    }

    public var string: String? {
        if case .string(let s) = self { return s }
        if case .number(let n) = self { return n.truncatingRemainder(dividingBy: 1) == 0 ? String(Int(n)) : String(n) }
        if case .bool(let b) = self { return String(b) }
        return nil
    }
    public var object: [String: JSONValue]? { if case .object(let o) = self { return o }; return nil }
    public var array: [JSONValue]?          { if case .array(let a) = self { return a }; return nil }

    public static func str(_ s: String) -> JSONValue { .string(s) }
}
