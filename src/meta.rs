//! Per-project metadata (git + languages) with background workers and an on-disk cache.

use crate::catalog::{Project, write_atomic};
use crate::git::{self, GitInfo};
use crate::langs;
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub path: String,
    /// "offline" (volume not mounted) or "deleted".
    pub availability: Option<String>,
    pub git: Option<GitInfo>,
    /// The git part was looked at (distinguishes "not a repo" from "not yet known").
    pub git_checked: bool,
    pub languages: Vec<(String, f64)>,
    pub languages_scanned: bool,
    pub fetched_at: f64,
    pub languages_at: f64,
}

#[derive(Serialize, Deserialize, Default)]
struct CacheFile {
    version: u32,
    projects: HashMap<String, Meta>,
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Why a folder is unreachable: its volume is not mounted, or it is gone.
pub fn availability(path: &str) -> Option<&'static str> {
    if Path::new(path).is_dir() {
        return None;
    }
    Some(if volume_missing(path) {
        "offline"
    } else {
        "deleted"
    })
}

fn volume_missing(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let root = match parts.as_slice() {
        ["", "Volumes", v, ..] if !v.is_empty() => format!("/Volumes/{v}"),
        ["", "media", u, v, ..] if !v.is_empty() => format!("/media/{u}/{v}"),
        ["", "run", "media", u, v, ..] if !v.is_empty() => format!("/run/media/{u}/{v}"),
        ["", "mnt", v, ..] if !v.is_empty() => format!("/mnt/{v}"),
        _ => return false,
    };
    !Path::new(&root).exists()
}

type Notify = Arc<dyn Fn(&str) + Send + Sync>;

struct Inner {
    cache: HashMap<String, Meta>,
    busy: HashSet<String>,
    dirty: bool,
}

#[derive(Clone)]
pub struct Metadata {
    inner: Arc<Mutex<Inner>>,
    slots: Arc<(Mutex<usize>, Condvar)>,
    notify: Notify,
    cache_path: Option<PathBuf>,
}

const WORKERS: usize = 6;

impl Metadata {
    pub fn new(cache_path: Option<PathBuf>, notify: Notify) -> Metadata {
        let cache = cache_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<CacheFile>(&s).ok())
            .filter(|c| c.version == 2)
            .map(|c| c.projects)
            .unwrap_or_default();
        Metadata {
            inner: Arc::new(Mutex::new(Inner {
                cache,
                busy: HashSet::new(),
                dirty: false,
            })),
            slots: Arc::new((Mutex::new(0), Condvar::new())),
            notify,
            cache_path,
        }
    }

    pub fn get(&self, id: &str) -> Option<Meta> {
        self.inner.lock().unwrap().cache.get(id).cloned()
    }
    pub fn set(&self, id: &str, m: Meta) {
        let mut g = self.inner.lock().unwrap();
        g.cache.insert(id.to_string(), m);
        g.dirty = true;
    }
    pub fn is_busy(&self, id: &str) -> bool {
        self.inner.lock().unwrap().busy.contains(id)
    }
    pub fn busy_count(&self) -> usize {
        self.inner.lock().unwrap().busy.len()
    }
    pub fn drop_project(&self, id: &str) {
        let mut g = self.inner.lock().unwrap();
        g.cache.remove(id);
        g.dirty = true;
    }

    /// Starts a background refresh when the cached data is older than its TTL (or `force`).
    /// `fetch` overrides `settings.fetch_remote`.
    pub fn refresh(&self, p: &Project, s: &Settings, force: bool, fetch: Option<bool>) {
        let t = now();
        {
            let mut g = self.inner.lock().unwrap();
            if g.busy.contains(&p.id) {
                return;
            }
            let m = g.cache.get(&p.id);
            let moved = m.map(|m| m.path != p.path).unwrap_or(true);
            let git_due = force
                || moved
                || t - m.map(|m| m.fetched_at).unwrap_or(0.0) > s.metadata_cache_minutes * 60.0;
            let lang_due = force
                || moved
                || t - m.map(|m| m.languages_at).unwrap_or(0.0) > s.language_cache_hours * 3600.0;
            if !git_due && !lang_due {
                return;
            }
            g.busy.insert(p.id.clone());
            drop(g);
            let me = self.clone();
            let p = p.clone();
            let s = s.clone();
            let fetch = fetch.unwrap_or(s.fetch_remote);
            std::thread::spawn(move || {
                me.acquire();
                me.work(&p, &s, git_due, lang_due, fetch);
                me.release();
                me.inner.lock().unwrap().busy.remove(&p.id);
                (me.notify)(&p.id);
            });
        }
    }

    fn acquire(&self) {
        let (m, cv) = &*self.slots;
        let mut n = m.lock().unwrap();
        while *n >= WORKERS {
            n = cv.wait(n).unwrap();
        }
        *n += 1;
    }
    fn release(&self) {
        let (m, cv) = &*self.slots;
        *m.lock().unwrap() -= 1;
        cv.notify_one();
    }

    fn publish(&self, id: &str, m: Meta) {
        self.set(id, m);
        (self.notify)(id);
    }

    fn work(&self, p: &Project, s: &Settings, git_due: bool, lang_due: bool, fetch: bool) {
        let prev = self.get(&p.id).unwrap_or_default();
        let mut m = prev.clone();
        m.path = p.path.clone();
        if let Some(a) = availability(&p.path) {
            m.availability = Some(a.to_string());
            if a == "deleted" {
                m.git = None;
                m.languages.clear();
            }
            self.publish(&p.id, m);
            return;
        }
        m.availability = None;
        let want_git = git_due && s.show_git && p.show.as_ref().and_then(|x| x.git).unwrap_or(true);
        if !p.manual_languages().is_empty() {
            m.languages = langs::from_names(p.manual_languages());
            m.languages_scanned = true;
            m.languages_at = now();
        }
        if want_git {
            // Quick, local phase first so the tile fills right away.
            let mut g = git::info(&p.path, false);
            if let (Some(g), Some(before)) = (g.as_mut(), prev.git.as_ref())
                && g.upstream.is_some()
                && g.upstream == before.upstream
            {
                g.fetched = before.fetched;
            }
            let has_upstream = g.as_ref().is_some_and(|g| g.upstream.is_some());
            m.git = g;
            m.git_checked = true;
            m.fetched_at = now();
            self.publish(&p.id, m.clone());
            if fetch && has_upstream {
                if let Some(g) = git::info(&p.path, true) {
                    m.git = Some(g);
                }
                m.fetched_at = now();
                self.publish(&p.id, m.clone());
            }
        }
        if lang_due && s.show_languages && p.manual_languages().is_empty() {
            let ignore: HashSet<String> = s.language_ignore_dirs.iter().cloned().collect();
            m.languages = langs::scan(&p.path, &ignore, s.max_files_scanned);
            m.languages_scanned = true;
            m.languages_at = now();
        }
        self.publish(&p.id, m);
    }

    /// Writes the cache when it changed.
    pub fn flush(&self) {
        let Some(path) = &self.cache_path else { return };
        let data = {
            let mut g = self.inner.lock().unwrap();
            if !g.dirty {
                return;
            }
            g.dirty = false;
            CacheFile {
                version: 2,
                projects: g.cache.clone(),
            }
        };
        if let Ok(s) = serde_json::to_string(&data) {
            let _ = write_atomic(path, s.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_kinds() {
        assert_eq!(availability("/"), None);
        assert_eq!(availability("/Volumes/NoSuchDisk-pt/x"), Some("offline"));
        assert_eq!(availability("/tmp/no-such-dir-pt-test"), Some("deleted"));
    }

    #[test]
    fn refresh_scans_and_caches() {
        let dir = std::env::temp_dir().join(format!("pt-meta-{}", crate::text::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "print(1)").unwrap();
        let cache = dir.join("cache.json");
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let tx = Mutex::new(tx);
        let md = Metadata::new(
            Some(cache.clone()),
            Arc::new(move |id: &str| {
                let _ = tx.lock().unwrap().send(id.to_string());
            }),
        );
        let p = Project::new(&dir.to_string_lossy());
        let s = Settings {
            fetch_remote: false,
            ..Settings::default()
        };
        md.refresh(&p, &s, false, None);
        md.refresh(&p, &s, false, None); // ignored while busy
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while md.is_busy(&p.id) && std::time::Instant::now() < deadline {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(50));
        }
        let m = md.get(&p.id).unwrap();
        assert!(m.git_checked);
        assert!(m.git.is_none());
        assert_eq!(m.languages[0].0, "Python");
        md.flush();
        let again = Metadata::new(Some(cache), Arc::new(|_: &str| {}));
        assert_eq!(again.get(&p.id), Some(m));
        std::fs::remove_dir_all(dir).ok();
    }
}
