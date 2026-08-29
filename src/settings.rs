use crate::model::DatabaseTarget;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark_mode: bool,
    pub page_size: usize,
    pub recent_files: Vec<PathBuf>,
    pub recent_databases: Vec<DatabaseTarget>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            page_size: 200,
            recent_files: Vec::new(),
            recent_databases: Vec::new(),
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
            .map(|mut value: Self| {
                for path in std::mem::take(&mut value.recent_files).into_iter().rev() {
                    value.remember(&DatabaseTarget::SQLite { path });
                }
                value
            })
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

    pub fn remember(&mut self, target: &DatabaseTarget) {
        let target = target.without_password();
        self.recent_databases.retain(|old| old != &target);
        self.recent_databases.insert(0, target);
        self.recent_databases.truncate(10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn password_is_never_serialized() {
        let target = DatabaseTarget::PostgreSQL {
            host: "db".into(),
            port: 5432,
            database: "app".into(),
            user: "me".into(),
            password: "secret".into(),
            ssl_mode: Default::default(),
        };
        assert!(!serde_json::to_string(&target).unwrap().contains("secret"));
    }
    #[test]
    fn legacy_recent_files_deserialize() {
        let settings: Settings = serde_json::from_str(r#"{"recent_files":["old.db"]}"#).unwrap();
        assert_eq!(settings.recent_files.len(), 1);
    }
}
