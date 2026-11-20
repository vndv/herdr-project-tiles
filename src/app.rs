//! `project-tiles app …`: a JSON backend for the Herdr.app mod (`~/.config/herdr-app/mods/project-tiles`).
//!
//! The mod draws the UI and owns catalog edits; this binary does what needs a real process:
//! git (process group, no TTY, timeouts, no GIT_SSH_COMMAND), the language scan, the cache,
//! path normalization and the import merge. Every command prints one JSON object on stdout.

use crate::catalog::CatalogStore;
use crate::git::{self, RemoteOp};
use crate::langs;
use crate::meta::{Meta, Metadata};
use crate::settings::{Paths, Settings};
use crate::text;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn run(args: &[String]) -> Result<Value, String> {
    let paths = Paths::from_env();
    let sub = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];
    match sub {
        "paths" => Ok(json!({
            "config_dir": paths.config_dir,
            "catalog": paths.catalog(),
            "settings": paths.settings(),
            "cache": paths.cache(),
            "vscode_catalog": Paths::vscode_catalog(),
        })),
        "meta" => meta(&paths, rest),
        "git" => git_op(&paths, rest),
        "resolve" => resolve(rest),
        "import" => import(&paths, rest),
        _ => Err(format!(
            "unknown app command `{sub}`; use paths | meta | git | resolve | import"
        )),
    }
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn values<'a>(args: &'a [String], name: &str) -> Vec<&'a str> {
    args.windows(2)
        .filter(|w| w[0] == name)
        .map(|w| w[1].as_str())
        .collect()
}

/// Meta as the mod wants it: language colours resolved, the sidebar-style git summary.
pub fn meta_json(m: &Meta) -> Value {
    let langs: Vec<Value> = m
        .languages
        .iter()
        .map(|(n, p)| json!({ "name": n, "pct": p, "color": langs::color_of(n) }))
        .collect();
    json!({
        "path": m.path,
        "availability": m.availability,
        "git": m.git,
        "git_checked": m.git_checked,
        "languages": langs,
        "languages_scanned": m.languages_scanned,
        "fetched_at": m.fetched_at,
        "remote_at": m.remote_at,
        "languages_at": m.languages_at,
        "git_token": crate::presence::git_token(m),
        "links": m.git.as_ref().map(git::links),
    })
}

/// `meta [--cached] [--force] [--fetch] [--id ID]…`: refreshes what is stale (or everything with
/// `--force`), waits for it, prints `{projects: {id: meta}}`. `--cached` only reads the cache.
fn meta(paths: &Paths, args: &[String]) -> Result<Value, String> {
    let store = CatalogStore::open(paths.catalog());
    let settings = Settings::load(&paths.settings());
    let md = Metadata::new(Some(paths.cache()), Arc::new(|_: &str| {}));
    let only = values(args, "--id");
    let projects: Vec<_> = store
        .data
        .projects
        .iter()
        .filter(|p| only.is_empty() || only.contains(&p.id.as_str()))
        .cloned()
        .collect();
    if !flag(args, "--cached") {
        let fetch = flag(args, "--fetch") && settings.fetch_remote;
        for p in &projects {
            md.refresh(p, &settings, flag(args, "--force"), Some(fetch));
        }
        let deadline = Instant::now() + Duration::from_secs(280);
        while md.busy_count() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(30));
        }
        md.flush();
    }
    let out: serde_json::Map<String, Value> = projects
        .iter()
        .filter_map(|p| md.get(&p.id).map(|m| (p.id.clone(), meta_json(&m))))
        .collect();
    Ok(json!({ "projects": out, "error": store.error }))
}

/// `git fetch|pull|push ID`: runs it in the project folder, then refreshes the local state.
fn git_op(paths: &Paths, args: &[String]) -> Result<Value, String> {
    let op = match args.first().map(String::as_str) {
        Some("fetch") => RemoteOp::Fetch,
        Some("pull") => RemoteOp::Pull,
        Some("push") => RemoteOp::Push,
        _ => return Err("usage: app git fetch|pull|push <project id>".into()),
    };
    let id = args.get(1).ok_or("missing project id")?;
    let store = CatalogStore::open(paths.catalog());
    let p = store
        .data
        .project(id)
        .ok_or_else(|| format!("no project {id}"))?
        .clone();
    if !git::is_repo(&p.path) {
        return Ok(json!({ "ok": false, "message": "not a git repository" }));
    }
    let o = git::remote(&p.path, op);
    let md = Metadata::new(Some(paths.cache()), Arc::new(|_: &str| {}));
    let mut m = md.get(&p.id).unwrap_or_default();
    m.path = p.path.clone();
    m.git = git::info(&p.path, false).map(|mut g| {
        // A successful fetch/pull/push means the counts reflect the remote now.
        g.fetched = o.ok()
            || m.git
                .as_ref()
                .is_some_and(|b| b.fetched && b.upstream == g.upstream);
        g
    });
    m.git_checked = true;
    m.fetched_at = crate::meta::now();
    if o.ok() {
        m.remote_at = m.fetched_at;
    }
    md.set(&p.id, m.clone());
    md.flush();
    Ok(
        json!({ "ok": o.ok(), "message": if o.ok() { String::new() } else { o.message() }, "meta": meta_json(&m) }),
    )
}

/// `resolve PATH [--base DIR]`: expands `~`, checks it is a folder, normalizes to the git root.
fn resolve(args: &[String]) -> Result<Value, String> {
    let raw = args.first().ok_or("missing path")?;
    let base = values(args, "--base")
        .first()
        .map(|s| s.to_string())
        .unwrap_or_else(text::home);
    let path = text::expand_path(raw, &base).to_string_lossy().to_string();
    if !std::path::Path::new(&path).is_dir() {
        return Ok(
            json!({ "ok": false, "path": path, "error": format!("no such folder: {path}") }),
        );
    }
    let root = git::toplevel(&path);
    Ok(json!({ "ok": true, "path": root, "name": crate::catalog::default_name(&root) }))
}

/// `import [--cwd DIR]…`: merges the VS Code catalog and open folders into the catalog.
fn import(paths: &Paths, args: &[String]) -> Result<Value, String> {
    let mut store = CatalogStore::open(paths.catalog());
    if let Some(e) = &store.error {
        return Err(format!("projects.json does not parse: {e}"));
    }
    let vscode = std::fs::read_to_string(Paths::vscode_catalog())
        .ok()
        .and_then(|s| crate::catalog::CatalogData::parse(&s).ok());
    let cwds: Vec<String> = values(args, "--cwd")
        .iter()
        .map(|s| s.to_string())
        .collect();
    let theme = crate::theme::Theme::load();
    let added = crate::ui::import_into(
        &mut store.data,
        vscode.as_ref(),
        &cwds,
        &text::home(),
        &|n| theme.pick_color(n),
        &git::toplevel,
    );
    if added > 0 {
        store.save().map_err(|e| e.to_string())?;
    }
    Ok(json!({ "added": added, "vscode": vscode.is_some() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_and_resolve() {
        let a: Vec<String> = ["--id", "x", "--fetch", "--id", "y"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(values(&a, "--id"), vec!["x", "y"]);
        assert!(flag(&a, "--fetch"));
        let dir = std::env::temp_dir().join(format!("pt-app-{}", text::new_id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let r = resolve(&[dir.join("sub").to_string_lossy().to_string()]).unwrap();
        assert_eq!(r["ok"], true);
        let r = resolve(&["/no/such/dir".to_string()]).unwrap();
        assert_eq!(r["ok"], false);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn meta_shape() {
        let m = Meta {
            languages: vec![("Rust".into(), 100.0)],
            git: Some(git::GitInfo {
                ahead: 2,
                ..Default::default()
            }),
            ..Default::default()
        };
        let v = meta_json(&m);
        assert_eq!(v["languages"][0]["color"], "#dea584");
        assert_eq!(v["git_token"], "↑2");
        assert_eq!(v["git"]["ahead"], 2);
    }
}
