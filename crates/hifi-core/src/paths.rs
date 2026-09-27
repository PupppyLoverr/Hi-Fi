//! Well-known filesystem locations.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
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

    pub fn token_file(&self) -> PathBuf {
        self.root.join("ipc.token")
    }

    pub fn write_ipc_token(&self) -> std::io::Result<String> {
        self.ensure_dirs()?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(self.token_file())?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        std::fs::set_permissions(self.token_file(), std::fs::Permissions::from_mode(0o600))?;
        Ok(token)
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

    /// Agent chat transcripts — one JSON file per chat tab.
    pub fn chats_dir(&self) -> PathBuf {
        self.root.join("chats")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        std::fs::create_dir_all(self.notes_dir())?;
        std::fs::create_dir_all(self.chats_dir())
    }
}
