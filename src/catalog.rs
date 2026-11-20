//! `projects.json`: the catalog in the Project Tiles format. Unknown fields are kept.

use crate::text::{self, norm};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Logo {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl Logo {
    pub fn auto() -> Logo {
        Logo {
            kind: "auto".into(),
            value: None,
        }
    }
    pub fn emoji(&self) -> Option<&str> {
        match (self.kind.as_str(), &self.value) {
            ("emoji", Some(v)) if !v.trim().is_empty() => Some(v.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Show {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub languages: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<Logo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub languages: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(
        rename = "collectionId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub collection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<Show>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Project {
    pub fn new(path: &str) -> Project {
        Project {
            id: text::new_id(),
            name: default_name(path),
            path: path.to_string(),
            logo: Some(Logo::auto()),
            ..Default::default()
        }
    }
    pub fn emoji(&self) -> Option<&str> {
        self.logo.as_ref().and_then(|l| l.emoji())
    }
    /// Emoji or initials.
    pub fn badge(&self) -> String {
        match self.emoji() {
            Some(e) => text::clamp_graphemes(e, 3),
            None => text::initials(&self.name),
        }
    }
    pub fn manual_languages(&self) -> &[String] {
        self.languages.as_deref().unwrap_or(&[])
    }
}

pub fn default_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Collection {
    pub fn is_collapsed(&self) -> bool {
        self.collapsed.unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogData {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub collections: Vec<Collection>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn one() -> u32 {
    1
}

impl Default for CatalogData {
    fn default() -> Self {
        CatalogData {
            version: 1,
            collections: Vec::new(),
            projects: Vec::new(),
            extra: Map::new(),
        }
    }
}

impl CatalogData {
    /// Parses and repairs: drops entries without path/id, fills missing ids and names.
    pub fn parse(s: &str) -> Result<CatalogData, serde_json::Error> {
        let mut d: CatalogData = serde_json::from_str(s)?;
        d.collections.retain(|c| !c.id.is_empty());
        d.projects.retain(|p| !p.path.is_empty());
        for p in &mut d.projects {
            if p.id.is_empty() {
                p.id = text::new_id();
            }
            if p.name.trim().is_empty() {
                p.name = default_name(&p.path);
            }
        }
        Ok(d)
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        s.push('\n');
        s
    }

    pub fn find_path(&self, path: &str) -> Option<usize> {
        let k = norm(path);
        self.projects.iter().position(|p| norm(&p.path) == k)
    }
    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }
    pub fn project_mut(&mut self, id: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.id == id)
    }
    pub fn collection(&self, id: Option<&str>) -> Option<&Collection> {
        let id = id?;
        self.collections.iter().find(|c| c.id == id)
    }
    pub fn collection_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.collections.iter_mut().find(|c| c.id == id)
    }
    pub fn collection_by_name(&self, name: &str) -> Option<&Collection> {
        let n = name.trim().to_lowercase();
        self.collections
            .iter()
            .find(|c| c.name.trim().to_lowercase() == n)
    }
    /// Finds a collection by name (case-insensitive) or creates it; `None` for an empty name.
    pub fn ensure_collection(
        &mut self,
        name: &str,
        color: impl Fn(&str) -> String,
    ) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(c) = self.collection_by_name(name) {
            return Some(c.id.clone());
        }
        let c = Collection {
            id: text::new_id(),
            name: name.to_string(),
            color: Some(color(name)),
            collapsed: Some(false),
            ..Default::default()
        };
        let id = c.id.clone();
        self.collections.push(c);
        Some(id)
    }
    /// Whether the project's collection id points at an existing collection.
    pub fn is_grouped(&self, p: &Project) -> bool {
        self.collection(p.collection_id.as_deref()).is_some()
    }
}

/// The catalog file with change detection by mtime.
pub struct CatalogStore {
    pub path: PathBuf,
    pub data: CatalogData,
    mtime: Option<SystemTime>,
    /// Last load error (unparsable hand edit), shown in the status line.
    pub error: Option<String>,
}

impl CatalogStore {
    pub fn open(path: PathBuf) -> CatalogStore {
        let mut s = CatalogStore {
            path,
            data: CatalogData::default(),
            mtime: None,
            error: None,
        };
        s.reload();
        s
    }

    fn current_mtime(&self) -> Option<SystemTime> {
        fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }

    /// Re-reads the file when its mtime changed. Returns true when the data changed.
    /// An unparsable file keeps the current state.
    pub fn reload(&mut self) -> bool {
        let mt = self.current_mtime();
        if mt.is_some() && mt == self.mtime {
            return false;
        }
        self.mtime = mt;
        if mt.is_none() {
            return false;
        }
        match fs::read_to_string(&self.path)
            .map_err(|e| e.to_string())
            .and_then(|s| CatalogData::parse(&s).map_err(|e| e.to_string()))
        {
            Ok(d) => {
                self.error = None;
                if d != self.data {
                    self.data = d;
                    return true;
                }
                false
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        write_atomic(&self.path, self.data.to_json().as_bytes())?;
        self.mtime = self.current_mtime();
        self.error = None;
        Ok(())
    }
}

/// Writes through a temp file in the same directory and renames it into place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VSCODE: &str = r##"{
  "version": 1,
  "collections": [
    {"id": "c1", "name": "Work", "description": "job", "color": "#ff8800", "collapsed": true}
  ],
  "projects": [
    {"id": "p1", "name": "Shop", "path": "/Users/me/shop", "description": "d",
     "logo": {"kind": "emoji", "value": "🛒"}, "languages": ["Go"], "color": "#123456",
     "collectionId": "c1", "show": {"git": false}, "futureField": 42},
    {"id": "p2", "name": "WS", "path": "/Users/me/x.code-workspace", "logo": {"kind": "image", "value": "a.png"}},
    {"name": "", "path": "/tmp/noid"},
    {"id": "bad"}
  ]
}"##;

    #[test]
    fn round_trip_keeps_fields() {
        let d = CatalogData::parse(VSCODE).unwrap();
        assert_eq!(d.projects.len(), 3);
        let p = &d.projects[0];
        assert_eq!(p.emoji(), Some("🛒"));
        assert_eq!(p.collection_id.as_deref(), Some("c1"));
        assert_eq!(p.show.as_ref().unwrap().git, Some(false));
        assert_eq!(p.extra.get("futureField"), Some(&serde_json::json!(42)));
        assert_eq!(d.projects[2].name, "noid");
        assert_eq!(d.projects[2].id.len(), 36);
        assert!(d.collections[0].is_collapsed());
        let back = CatalogData::parse(&d.to_json()).unwrap();
        assert_eq!(back, d);
        let v: Value = serde_json::from_str(&d.to_json()).unwrap();
        assert_eq!(v["projects"][0]["collectionId"], "c1");
        assert_eq!(v["projects"][0]["futureField"], 42);
        assert!(v["projects"][0].get("collection_id").is_none());
    }

    #[test]
    fn lookups() {
        let mut d = CatalogData::parse(VSCODE).unwrap();
        assert_eq!(d.find_path("/users/me/shop/"), Some(0));
        assert_eq!(d.collection_by_name(" work ").unwrap().id, "c1");
        let id = d.ensure_collection("New", |_| "#000000".into()).unwrap();
        assert_eq!(d.collections.len(), 2);
        assert_eq!(d.ensure_collection("new", |_| "#111111".into()), Some(id));
        assert_eq!(d.ensure_collection("  ", |_| String::new()), None);
    }

    #[test]
    fn store_reload_and_bad_file() {
        let dir = std::env::temp_dir().join(format!("pt-cat-{}", text::new_id()));
        let path = dir.join("projects.json");
        let mut s = CatalogStore::open(path.clone());
        assert!(s.data.projects.is_empty());
        s.data.projects.push(Project::new("/tmp/a"));
        s.save().unwrap();
        assert!(!s.reload());
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "{ not json").unwrap();
        // Force a different mtime even on coarse file systems.
        let f = fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        assert!(!s.reload());
        assert!(s.error.is_some());
        assert_eq!(s.data.projects.len(), 1);
        fs::remove_dir_all(dir).ok();
    }
}
