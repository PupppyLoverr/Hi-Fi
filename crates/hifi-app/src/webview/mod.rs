//! Native page hosts. Each platform keeps the engine it ships with:
//! WKWebView on macOS, WebView2 on Windows and WebKitGTK (offscreen, in a
//! helper process) on Linux. All three expose the same `WebPaneHost` API
//! and report through the same `WebEvent` channel.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::WebPaneHost;
#[cfg(target_os = "macos")]
pub use macos::WebPaneHost;
#[cfg(target_os = "windows")]
pub use windows::WebPaneHost;

/// Events a native page pushes to the app (engine callback → channel → GPUI).
#[derive(Debug)]
pub enum WebEvent {
    Title {
        tab: String,
        title: String,
    },
    Url {
        tab: String,
        url: String,
    },
    Loading {
        tab: String,
        loading: bool,
    },
    CanGo {
        tab: String,
        back: bool,
        forward: bool,
    },
    /// target=_blank / window.open — open a sibling tab.
    NewTab {
        url: String,
    },
    /// Keystroke swallowed while the webview had focus; re-dispatch in GPUI.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Keystroke {
        combo: String,
    },
    /// Editor surface posted its body over `window.ipc`.
    NotesSave {
        tab: String,
        html: String,
    },
    Error {
        tab: String,
        message: String,
    },
    /// A new offscreen frame is ready to composite.
    #[cfg(target_os = "linux")]
    Redraw,
    /// The page copied text; GPUI owns the system clipboard.
    #[cfg(target_os = "linux")]
    Clipboard {
        text: String,
    },
}

/// Browser chords the app keymap owns even while a page holds focus.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn is_browser_chord(combo: &str) -> bool {
    let combo = combo.replace("ctrl-", "cmd-");
    matches!(
        combo.as_str(),
        "cmd-t"
            | "cmd-w"
            | "cmd-l"
            | "cmd-["
            | "cmd-]"
            | "cmd-r"
            | "cmd-shift-r"
            | "cmd-,"
            | "cmd-f"
            | "cmd-b"
            | "cmd-k"
            | "cmd-shift-\\"
            | "cmd-tab"
            | "cmd-shift-tab"
            | "cmd-shift-["
            | "cmd-shift-]"
            | "f5"
    )
}
