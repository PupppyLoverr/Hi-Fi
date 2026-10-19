//! Native page hosts. Each platform keeps the engine it ships with:
//! WKWebView on macOS, WebView2 on Windows and WebKitGTK (offscreen, in a
//! helper process) on Linux. All three expose the same `WebPaneHost` API
//! and report through the same `WebEvent` channel.

use std::sync::mpsc::{SendError, Sender};

pub type WakeSender = futures::channel::mpsc::UnboundedSender<()>;

#[derive(Clone)]
pub struct WebEventSender {
    tx: Sender<WebEvent>,
    wake: WakeSender,
}

impl WebEventSender {
    pub fn new(tx: Sender<WebEvent>, wake: WakeSender) -> Self {
        Self { tx, wake }
    }

    pub fn send(&self, event: WebEvent) -> Result<(), SendError<WebEvent>> {
        let result = self.tx.send(event);
        let _ = self.wake.unbounded_send(());
        result
    }
}

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
    ContextMenu {
        tab: String,
        x: f64,
        y: f64,
        href: String,
        src: String,
        selection: String,
    },
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

/// Decode an IPC payload shared by browser context menus and Notes saves.
pub fn parse_ipc_event(tab: &str, body: &str) -> Option<WebEvent> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    (value.get("type")?.as_str() == Some("ctx")).then(|| WebEvent::ContextMenu {
        tab: tab.into(),
        x: value
            .get("x")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default(),
        y: value
            .get("y")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default(),
        href: value
            .get("href")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into(),
        src: value
            .get("src")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into(),
        selection: value
            .get("sel")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into(),
    })
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
