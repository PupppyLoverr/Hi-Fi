import Foundation

/// Stable, typed, human-debuggable identifiers.
public enum HiFiID {
    public static func tab() -> String      { "tab_"   + Self.short() }
    public static func group() -> String    { "group_" + Self.short() }
    public static func space() -> String    { "space_" + Self.short() }
    public static func profile() -> String  { "prof_"  + Self.short() }
    public static func worktree() -> String { "wt_"    + Self.short() }

    private static func short() -> String {
        String(UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(12))
    }
}
