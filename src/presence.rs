//! Which projects are open in herdr, their agents, and the Spaces sidebar tokens.

use crate::api::{AgentStatus, SOURCE, SessionSnapshot, WorkspaceInfo};
use crate::catalog::{CatalogData, Project};
use crate::meta::Meta;
use crate::socket::{ApiError, Client};
use crate::text::{norm, within};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// The catalog project a cwd belongs to (the deepest project path that contains it).
pub fn project_for_cwd<'a>(cat: &'a CatalogData, cwd: &str) -> Option<&'a Project> {
    cat.projects
        .iter()
        .filter(|p| within(cwd, &p.path))
        .max_by_key(|p| norm(&p.path).len())
}

/// Project id per workspace. Our own `pt_id` token wins while a pane is still inside that
/// project; otherwise the project most panes are in (ties → the deeper path).
pub fn workspace_projects(
    cat: &CatalogData,
    snap: &SessionSnapshot,
    skip_pane: Option<&str>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for ws in &snap.workspaces {
        let mut votes: HashMap<&str, (usize, usize)> = HashMap::new();
        for pane in snap
            .panes
            .iter()
            .filter(|p| p.workspace_id == ws.workspace_id)
        {
            if Some(pane.pane_id.as_str()) == skip_pane {
                continue;
            }
            let mut seen: Vec<&str> = Vec::new();
            for cwd in pane.cwds() {
                if let Some(p) = project_for_cwd(cat, cwd)
                    && !seen.contains(&p.id.as_str())
                {
                    seen.push(&p.id);
                    let e = votes.entry(&p.id).or_insert((0, norm(&p.path).len()));
                    e.0 += 1;
                }
            }
        }
        let tagged = ws
            .tokens
            .get("pt_id")
            .filter(|id| votes.contains_key(id.as_str()));
        let pick = tagged.cloned().or_else(|| {
            votes
                .iter()
                .max_by_key(|(_, (n, depth))| (*n, *depth))
                .map(|(id, _)| id.to_string())
        });
        if let Some(id) = pick {
            out.insert(ws.workspace_id.clone(), id);
        }
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Presence {
    /// Workspace numbers the project is open in.
    pub workspaces: Vec<(u64, String)>,
    pub working: usize,
    pub blocked: usize,
    pub done: usize,
}

/// Open workspaces and agent counts per project id.
pub fn presence(
    cat: &CatalogData,
    snap: &SessionSnapshot,
    skip_pane: Option<&str>,
) -> HashMap<String, Presence> {
    let mut out: HashMap<String, Presence> = HashMap::new();
    let ws_by_id: HashMap<&str, &WorkspaceInfo> = snap
        .workspaces
        .iter()
        .map(|w| (w.workspace_id.as_str(), w))
        .collect();
    for (ws_id, pid) in workspace_projects(cat, snap, skip_pane) {
        if let Some(ws) = ws_by_id.get(ws_id.as_str()) {
            out.entry(pid)
                .or_default()
                .workspaces
                .push((ws.number, ws_id.clone()));
        }
    }
    for pane in &snap.panes {
        if Some(pane.pane_id.as_str()) == skip_pane {
            continue;
        }
        let Some(p) = pane.cwds().find_map(|c| project_for_cwd(cat, c)) else {
            continue;
        };
        let e = out.entry(p.id.clone()).or_default();
        match pane.agent_status {
            AgentStatus::Working => e.working += 1,
            AgentStatus::Blocked => e.blocked += 1,
            AgentStatus::Done => e.done += 1,
            _ => {}
        }
    }
    for v in out.values_mut() {
        v.workspaces.sort();
        v.workspaces.dedup();
    }
    out
}

pub const TOKEN_KEYS: &[&str] = &["pt_id", "pt_project", "pt_git", "pt_langs"];

/// `↑N ↓N ●N`, `✓` when in sync and clean, `None` for a non-repo.
pub fn git_token(m: &Meta) -> Option<String> {
    let g = m.git.as_ref()?;
    let mut parts = Vec::new();
    if g.ahead > 0 {
        parts.push(format!("↑{}", g.ahead));
    }
    if g.behind > 0 {
        parts.push(format!("↓{}", g.behind));
    }
    if g.dirty > 0 {
        parts.push(format!("●{}", g.dirty));
    }
    Some(if parts.is_empty() {
        "✓".into()
    } else {
        parts.join(" ")
    })
}

pub fn tokens_for(p: &Project, m: Option<&Meta>) -> BTreeMap<String, Option<String>> {
    let mut t = BTreeMap::new();
    t.insert("pt_id".into(), Some(p.id.clone()));
    t.insert(
        "pt_project".into(),
        Some(format!("{} {}", p.badge(), p.name)),
    );
    t.insert("pt_git".into(), m.and_then(git_token));
    t.insert(
        "pt_langs".into(),
        m.and_then(|m| m.languages.first().map(|l| l.0.clone()))
            .or_else(|| p.manual_languages().first().cloned()),
    );
    t
}

/// Tokens to clear for a workspace that is no longer a catalog project.
pub fn clear_tokens() -> BTreeMap<String, Option<String>> {
    TOKEN_KEYS.iter().map(|k| (k.to_string(), None)).collect()
}

/// Only the keys whose value differs from what the workspace already has.
pub fn diff(
    ws: &WorkspaceInfo,
    want: &BTreeMap<String, Option<String>>,
) -> BTreeMap<String, Option<String>> {
    want.iter()
        .filter(|(k, v)| ws.tokens.get(*k).map(|s| s.as_str()) != v.as_deref())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

pub fn report(
    client: &Client,
    workspace_id: &str,
    tokens: &BTreeMap<String, Option<String>>,
) -> Result<(), ApiError> {
    if tokens.is_empty() {
        return Ok(());
    }
    let map: serde_json::Map<String, Value> = tokens
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.clone().map(Value::String).unwrap_or(Value::Null),
            )
        })
        .collect();
    client
        .request(
            "workspace.report_metadata",
            json!({ "workspace_id": workspace_id, "source": SOURCE, "tokens": map }),
        )
        .map(|_| ())
}

/// Brings every workspace's tokens in line with the catalog. Returns the number of reports sent.
pub fn tag_all(
    client: &Client,
    cat: &CatalogData,
    snap: &SessionSnapshot,
    meta: &dyn Fn(&Project) -> Option<Meta>,
    skip_pane: Option<&str>,
) -> usize {
    let map = workspace_projects(cat, snap, skip_pane);
    let mut sent = 0;
    for ws in &snap.workspaces {
        let want = match map.get(&ws.workspace_id).and_then(|id| cat.project(id)) {
            Some(p) => tokens_for(p, meta(p).as_ref()),
            None if ws.tokens.contains_key("pt_id") => clear_tokens(),
            None => continue,
        };
        let d = diff(ws, &want);
        if !d.is_empty() && report(client, &ws.workspace_id, &d).is_ok() {
            sent += 1;
        }
    }
    sent
}

pub fn notify(client: &Client, title: &str, body: &str) -> Option<String> {
    client
        .request(
            "notification.show",
            json!({ "title": crate::text::trunc(title, 80), "body": crate::text::trunc(body, 240) }),
        )
        .ok()
        .and_then(|r| r.get("reason").and_then(Value::as_str).map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PaneInfo;
    use crate::git::GitInfo;

    fn cat() -> CatalogData {
        let mut c = CatalogData::default();
        for (id, path) in [("a", "/w/shop"), ("b", "/w/shop/admin"), ("c", "/w/other")] {
            let mut p = Project::new(path);
            p.id = id.into();
            c.projects.push(p);
        }
        c
    }

    fn pane(id: &str, ws: &str, cwd: &str, st: AgentStatus) -> PaneInfo {
        PaneInfo {
            pane_id: id.into(),
            workspace_id: ws.into(),
            cwd: Some(cwd.into()),
            agent_status: st,
            ..Default::default()
        }
    }

    fn snap() -> SessionSnapshot {
        SessionSnapshot {
            workspaces: vec![
                WorkspaceInfo {
                    workspace_id: "w1".into(),
                    number: 1,
                    ..Default::default()
                },
                WorkspaceInfo {
                    workspace_id: "w2".into(),
                    number: 2,
                    ..Default::default()
                },
                WorkspaceInfo {
                    workspace_id: "w3".into(),
                    number: 3,
                    tokens: [("pt_id".to_string(), "c".to_string())].into(),
                    ..Default::default()
                },
            ],
            panes: vec![
                pane("w1:p1", "w1", "/w/shop/src", AgentStatus::Working),
                pane("w1:p2", "w1", "/w/shop/admin/x", AgentStatus::Blocked),
                pane("w1:p3", "w1", "/w/shop", AgentStatus::Done),
                pane("w2:p1", "w2", "/w/shop/admin", AgentStatus::Idle),
                pane("w3:p1", "w3", "/tmp", AgentStatus::Idle),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn matching_and_presence() {
        let c = cat();
        assert_eq!(project_for_cwd(&c, "/w/shop/admin/q").unwrap().id, "b");
        assert!(project_for_cwd(&c, "/w/shopx").is_none());
        let m = workspace_projects(&c, &snap(), None);
        assert_eq!(m["w1"], "a");
        assert_eq!(m["w2"], "b");
        assert!(!m.contains_key("w3"));
        let pr = presence(&c, &snap(), None);
        assert_eq!(pr["a"].workspaces, vec![(1, "w1".to_string())]);
        assert_eq!((pr["a"].working, pr["a"].done), (1, 1));
        assert_eq!(pr["b"].blocked, 1);
        assert_eq!(pr["b"].workspaces, vec![(2, "w2".to_string())]);
        // Our own pane (w1:p3) does not count; the tie goes to the deeper project.
        let pr = presence(&c, &snap(), Some("w1:p3"));
        assert_eq!(pr["a"].done, 0);
        assert_eq!(pr["b"].workspaces.len(), 2);
    }

    #[test]
    fn tokens() {
        let mut p = Project::new("/w/shop-backend");
        p.id = "x".into();
        let mut m = Meta {
            git: Some(GitInfo {
                ahead: 2,
                behind: 1,
                dirty: 3,
                ..Default::default()
            }),
            languages: vec![("Rust".into(), 90.0)],
            ..Default::default()
        };
        let t = tokens_for(&p, Some(&m));
        assert_eq!(t["pt_project"].as_deref(), Some("SB shop-backend"));
        assert_eq!(t["pt_git"].as_deref(), Some("↑2 ↓1 ●3"));
        assert_eq!(t["pt_langs"].as_deref(), Some("Rust"));
        m.git = Some(GitInfo::default());
        assert_eq!(git_token(&m).as_deref(), Some("✓"));
        m.git = None;
        assert_eq!(tokens_for(&p, Some(&m))["pt_git"], None);
        let ws = WorkspaceInfo {
            tokens: [("pt_id".to_string(), "x".to_string())].into(),
            ..Default::default()
        };
        let d = diff(&ws, &tokens_for(&p, Some(&m)));
        assert!(!d.contains_key("pt_id"));
        assert!(!d.contains_key("pt_git")); // None and missing are equal
        assert!(d.contains_key("pt_project"));
    }
}
