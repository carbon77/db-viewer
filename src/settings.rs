use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark_mode: bool,
    pub page_size: usize,
    pub recent_files: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            page_size: 200,
            recent_files: Vec::new(),
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        ProjectDirs::from("com", "RustDbViewer", "Rust DB Viewer")
            .map(|dirs| dirs.config_dir().join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn remember(&mut self, path: &Path) {
        self.recent_files.retain(|old| old != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(10);
    }
}
