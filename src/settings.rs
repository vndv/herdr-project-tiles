//! `settings.json` (snake_case keys, same as the Python version).

use crate::catalog::write_atomic;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub columns: u32,
    pub tile_size: String,
    pub show_path: bool,
    pub show_description: bool,
    pub show_languages: bool,
    pub show_git: bool,
    pub fetch_remote: bool,
    pub metadata_cache_minutes: f64,
    pub language_cache_hours: f64,
    pub max_files_scanned: usize,
    pub open_mode: String,
    pub language_ignore_dirs: Vec<String>,
}

pub const IGNORE_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    "bower_components",
    "vendor",
    "Library",
    "Temp",
    "Logs",
    "obj",
    "bin",
    "build",
    "Build",
    "dist",
    "out",
    "target",
    ".idea",
    ".vs",
    ".vscode",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    ".next",
    ".nuxt",
    "coverage",
    ".gradle",
    "Pods",
    "DerivedData",
    ".dart_tool",
    ".cache",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".terraform",
    "dbt_packages",
    "logs",
];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            columns: 3,
            tile_size: "normal".into(),
            show_path: true,
            show_description: true,
            show_languages: true,
            show_git: true,
            fetch_remote: true,
            metadata_cache_minutes: 10.0,
            language_cache_hours: 12.0,
            max_files_scanned: 15000,
            open_mode: "workspace".into(),
            language_ignore_dirs: IGNORE_DIRS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Settings {
    pub fn compact(&self) -> bool {
        self.tile_size == "compact"
    }
    pub fn columns(&self) -> usize {
        self.columns.clamp(1, 6) as usize
    }

    /// Reads the file; a missing file is created with the defaults, a broken one is ignored.
    pub fn load(path: &Path) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => {
                let d = Settings::default();
                let _ = write_atomic(path, pretty(&serde_json::to_value(&d).unwrap()).as_bytes());
                d
            }
        }
    }

    /// Writes one key into the file, keeping everything else (including unknown keys) as is.
    pub fn save_key(&self, path: &Path, key: &str) -> std::io::Result<()> {
        let mut cur: Map<String, Value> = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| match serde_json::to_value(Settings::default()) {
                Ok(Value::Object(m)) => m,
                _ => Map::new(),
            });
        if let Ok(Value::Object(mine)) = serde_json::to_value(self)
            && let Some(v) = mine.get(key)
        {
            cur.insert(key.to_string(), v.clone());
        }
        write_atomic(path, pretty(&Value::Object(cur)).as_bytes())
    }
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

/// Where the plugin keeps its files.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Paths {
        let home = crate::text::home();
        let config_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(&home).join(".config/herdr/plugins/config/vndv.project-tiles")
            });
        let state_dir = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(&home).join(".local/state/herdr/plugins/vndv.project-tiles")
            });
        Paths {
            config_dir,
            state_dir,
        }
    }
    pub fn catalog(&self) -> PathBuf {
        self.config_dir.join("projects.json")
    }
    pub fn settings(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }
    pub fn cache(&self) -> PathBuf {
        self.state_dir.join("metadata-cache.json")
    }
    pub fn vscode_catalog() -> PathBuf {
        let home = crate::text::home();
        if cfg!(target_os = "macos") {
            Path::new(&home).join("Library/Application Support/project-tiles/projects.json")
        } else {
            let base =
                std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
            Path::new(&base).join("project-tiles/projects.json")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_and_save_key() {
        let s: Settings =
            serde_json::from_str(r#"{"columns": 9, "tile_size": "compact", "x": 1}"#).unwrap();
        assert_eq!(s.columns(), 6);
        assert!(s.compact());
        assert!(s.fetch_remote);
        let dir = std::env::temp_dir().join(format!("pt-set-{}", crate::text::new_id()));
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, r#"{"columns": 2, "custom": true}"#).unwrap();
        let mut s = Settings::load(&p);
        assert_eq!(s.columns, 2);
        s.tile_size = "compact".into();
        s.columns = 5; // not saved: only tile_size is written
        s.save_key(&p, "tile_size").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["tile_size"], "compact");
        assert_eq!(v["columns"], 2);
        assert_eq!(v["custom"], true);
        std::fs::remove_dir_all(dir).ok();
    }
}
