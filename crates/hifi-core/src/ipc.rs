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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
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
    /// Visible text of the page (agent reading surface).
    pub const TAB_READ: &str = "tab.read";
    pub const TAB_SCROLL: &str = "tab.scroll";
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

pub mod client {
    //! Blocking client used by the `hifi` CLI. Unix-domain socket on macOS
    //! and Linux; loopback TCP on Windows (port published next to the socket
    //! path, see [`port_file`]).
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;

    #[cfg(unix)]
    fn connect(socket: &Path) -> std::io::Result<std::os::unix::net::UnixStream> {
        std::os::unix::net::UnixStream::connect(socket)
    }

    #[cfg(not(unix))]
    fn connect(socket: &Path) -> std::io::Result<std::net::TcpStream> {
        let port: u16 = std::fs::read_to_string(port_file(socket))?
            .trim()
            .parse()
            .map_err(std::io::Error::other)?;
        std::net::TcpStream::connect(("127.0.0.1", port))
    }

    pub fn call(
        socket: &Path,
        method: &str,
        params: Value,
        timeout_secs: u64,
    ) -> anyhow::Result<IpcResponse> {
        call_as_agent(socket, method, params, timeout_secs, None)
    }

    pub fn call_as_agent(
        socket: &Path,
        method: &str,
        params: Value,
        timeout_secs: u64,
        agent: Option<&str>,
    ) -> anyhow::Result<IpcResponse> {
        let stream = connect(socket).map_err(|e| anyhow::anyhow!("Hi-Fi is not running: {e}"))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(timeout_secs)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;
        let token_path = socket
            .parent()
            .map(|parent| parent.join("ipc.token"))
            .ok_or_else(|| anyhow::anyhow!("Hi-Fi is not running"))?;
        let token = std::fs::read_to_string(token_path)
            .map_err(|_| anyhow::anyhow!("Hi-Fi is not running"))?
            .trim()
            .to_string();
        if token.is_empty() {
            anyhow::bail!("Hi-Fi is not running");
        }
        let req = IpcRequest {
            id: uuid::Uuid::new_v4().simple().to_string(),
            method: method.to_string(),
            params,
            token,
            agent: agent.map(str::to_string),
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

/// Where the Windows IPC server publishes its loopback port.
pub fn port_file(socket: &std::path::Path) -> std::path::PathBuf {
    socket.with_extension("port")
}

pub fn agent_may(
    method: &str,
    req_agent: Option<&str>,
    tab_agent_of: Option<&str>,
) -> Result<(), &'static str> {
    let Some(agent) = req_agent else {
        return Ok(());
    };
    use methods as m;
    if matches!(method, m::TAB_OPEN | m::TAB_LIST | m::PING) {
        return Ok(());
    }
    let tab_method = matches!(
        method,
        m::TAB_CLOSE
            | m::TAB_FOCUS
            | m::TAB_RELOAD
            | m::TAB_NAVIGATE
            | m::TAB_BACK
            | m::TAB_FORWARD
            | m::TAB_PIN
            | m::TAB_EXEC
            | m::TAB_SNAPSHOT
            | m::TAB_CLICK
            | m::TAB_TYPE
            | m::TAB_SCREENSHOT
            | m::TAB_READ
            | m::TAB_SCROLL
    );
    if method == m::TAB_EXEC {
        return Err("not permitted for agents");
    }
    if tab_method {
        return if tab_agent_of == Some(agent) {
            Ok(())
        } else {
            Err("tab is not owned by this agent")
        };
    }
    Err("not permitted for agents")
}

#[cfg(test)]
mod tests {
    use super::{agent_may, methods};

    #[test]
    fn agent_can_use_owned_tab_only() {
        assert!(agent_may(methods::TAB_READ, Some("chat"), Some("chat")).is_ok());
        assert_eq!(
            agent_may(methods::TAB_READ, Some("chat"), None),
            Err("tab is not owned by this agent")
        );
        assert_eq!(
            agent_may(methods::TAB_EXEC, Some("chat"), Some("chat")),
            Err("not permitted for agents")
        );
        assert!(agent_may(methods::TAB_LIST, Some("chat"), None).is_ok());
    }
}
