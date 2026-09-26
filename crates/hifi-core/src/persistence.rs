//! `state.json` load/save.

use crate::models::WorkspaceState;
use std::path::Path;

#[derive(Debug)]
pub struct StateFile {
    pub path: std::path::PathBuf,
}

impl StateFile {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> WorkspaceState {
        self.load_at(&self.path)
    }

    fn load_at(&self, path: &Path) -> WorkspaceState {
        let Ok(text) = std::fs::read_to_string(path) else {
            return WorkspaceState::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Atomic write: serialize to a sibling temp file then rename.
    pub fn save(&self, state: &WorkspaceState) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
        std::fs::rename(&tmp, &self.path)
    }
}
