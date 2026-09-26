//! Well-known filesystem locations.

use std::path::PathBuf;

/// Data directory layout: `~/Library/Application Support/HiFi/` on macOS,
/// `~/.local/share/hifi` on Linux, `%APPDATA%\hifi` on Windows.
#[derive(Debug, Clone)]
pub struct HifiPaths {
    pub root: PathBuf,
}

impl HifiPaths {
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        let root = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("~").join("Library/Application Support"))
            .join("HiFi");
        #[cfg(target_os = "linux")]
        let root = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("~").join(".local/share"))
            .join("hifi");
        #[cfg(target_os = "windows")]
        let root = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("~").join("AppData\\Roaming"))
            .join("hifi");
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let root = PathBuf::from(".").join("hifi-data");
        Self { root }
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub fn socket_file(&self) -> PathBuf {
        self.root.join("ipc.sock")
    }

    pub fn keymap_file(&self) -> PathBuf {
        self.root.join("keymap.toml")
    }

    pub fn history_file(&self) -> PathBuf {
        self.root.join("history.json")
    }

    /// Notes surface documents — one HTML file per notes tab.
    pub fn notes_dir(&self) -> PathBuf {
        self.root.join("notes")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.notes_dir())
    }
}
