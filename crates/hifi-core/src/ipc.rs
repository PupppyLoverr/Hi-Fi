//! IPC wire protocol — newline-delimited JSON on the unix socket.
//!
//! Request:  `{"id": "…", "method": "tab.open", "params": {…}}`
//! Response: `{"id": "…", "ok": true, "result": {…}}` or `"error": "…"`

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl IpcResponse {
    pub fn ok(id: impl Into<String>, result: Value) -> Self {
        Self {
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

/// Method names — kept identical to the Swift build so scripts port unchanged.
pub mod methods {
    pub const PING: &str = "ping";
    pub const TAB_OPEN: &str = "tab.open";
    pub const TAB_CLOSE: &str = "tab.close";
    pub const TAB_LIST: &str = "tab.list";
    pub const TAB_FOCUS: &str = "tab.focus";
    pub const TAB_RELOAD: &str = "tab.reload";
    pub const TAB_NAVIGATE: &str = "tab.navigate";
    pub const TAB_BACK: &str = "tab.back";
    pub const TAB_FORWARD: &str = "tab.forward";
    pub const TAB_PIN: &str = "tab.pin";
    pub const TAB_EXEC: &str = "tab.exec";
    pub const TAB_SNAPSHOT: &str = "tab.snapshot";
    pub const TAB_CLICK: &str = "tab.click";
    pub const TAB_TYPE: &str = "tab.type";
    pub const TAB_SCREENSHOT: &str = "tab.screenshot";
    pub const GROUP_LIST: &str = "group.list";
    pub const GROUP_CREATE: &str = "group.create";
    pub const SPACE_LIST: &str = "space.list";
    pub const SPACE_CREATE: &str = "space.create";
    pub const SPACE_SWITCH: &str = "space.switch";
    pub const WORKTREE_CREATE: &str = "worktree.create";
    pub const SPLIT_SET: &str = "split.set";
    pub const WINDOW_NEW: &str = "window.new";
    pub const BROWSER_USE: &str = "browser.use";
    pub const DOCK_OPEN: &str = "dock.open";
    pub const DOCK_TAB: &str = "dock.tab";
    pub const DOCK_UNDOCK: &str = "dock.undock";
    pub const DOCK_TOGGLE: &str = "dock.toggle";
    pub const DOCK_LIST: &str = "dock.list";
}

#[cfg(unix)]
pub mod client {
    //! Blocking client used by the `hifi` CLI.
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    pub fn call(
        socket: &Path,
        method: &str,
        params: Value,
        timeout_secs: u64,
    ) -> anyhow::Result<IpcResponse> {
        let stream = UnixStream::connect(socket)
            .map_err(|e| anyhow::anyhow!("cannot reach Hi-Fi (is it running?): {e}"))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(timeout_secs)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;
        let req = IpcRequest {
            id: uuid::Uuid::new_v4().simple().to_string(),
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        stream.try_clone()?.write_all(line.as_bytes())?;
        let mut reader = BufReader::new(stream);
        let mut buf = String::new();
        reader.read_line(&mut buf)?;
        if buf.is_empty() {
            anyhow::bail!("Hi-Fi closed the connection without a reply");
        }
        Ok(serde_json::from_str(&buf)?)
    }
}
