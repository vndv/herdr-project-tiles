//! Integration-style tests: render snapshots (TestBackend), navigation, import and a fake
//! herdr socket server (NDJSON over a UnixListener).

use crate::api::{AgentStatus, InvocationContext, PaneInfo, SessionSnapshot, WorkspaceInfo};
use crate::catalog::{CatalogData, Collection, Logo, Project};
use crate::git::{Commit, GitInfo};
use crate::meta::Meta;
use crate::settings::Paths;
use crate::socket::Client;
use crate::theme::Theme;
use crate::ui::{self, App, Env, ItemKind, Msg};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pt-{tag}-{}", crate::text::new_id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn project(id: &str, name: &str, path: &str) -> Project {
    let mut p = Project::new(path);
    p.id = id.into();
    p.name = name.into();
    p
}

fn sample_catalog() -> CatalogData {
    let mut c = CatalogData::default();
    c.collections.push(Collection {
        id: "work".into(),
        name: "Work".into(),
        description: Some("contoso".into()),
        color: Some("#83a598".into()),
        ..Default::default()
    });
    c.collections.push(Collection {
        id: "old".into(),
        name: "Archive".into(),
        color: Some("#d3869b".into()),
        collapsed: Some(true),
        ..Default::default()
    });
    let mut a = project("a", "shop-backend", "/srv/work/shop-backend");
    a.description = Some("shop REST API".into());
    a.collection_id = Some("work".into());
    let mut b = project("b", "Crabs", "/srv/work/crabs");
    b.logo = Some(Logo {
        kind: "emoji".into(),
        value: Some("🦀".into()),
    });
    b.collection_id = Some("work".into());
    let mut d = project("d", "MyGameStudio", "/Volumes/External/MyGameStudio");
    d.description = Some("unity".into());
    let e = project("e", "gone", "/srv/gone");
    let mut f = project("f", "legacy", "/srv/legacy");
    f.collection_id = Some("old".into());
    let mut g = project("g", "日本語プロジェクト名がとても長い", "/srv/jp");
    g.languages = Some(vec!["Go".into(), "SQL".into()]);
    c.projects = vec![a, b, d, e, f, g];
    c
}

fn seed_meta(app: &App) {
    app.meta.set(
        "a",
        Meta {
            path: "/srv/work/shop-backend".into(),
            git: Some(GitInfo {
                branch: "main".into(),
                upstream: Some("origin/main".into()),
                ahead: 1,
                behind: 2,
                dirty: 3,
                untracked: 1,
                fetched: true,
                commit: Some(Commit {
                    hash: "a1b2c3d".into(),
                    subject: "fix: retry on 429".into(),
                    date: "2 days".into(),
                    author: "me".into(),
                }),
                ..Default::default()
            }),
            git_checked: true,
            languages: vec![
                ("Python".into(), 71.0),
                ("SQL".into(), 20.0),
                ("Shell".into(), 9.0),
            ],
            languages_scanned: true,
            ..Default::default()
        },
    );
    app.meta.set(
        "b",
        Meta {
            git: Some(GitInfo {
                branch: "feature/x".into(),
                upstream: Some("origin/feature/x".into()),
                fetched: false,
                ..Default::default()
            }),
            git_checked: true,
            languages: vec![("Rust".into(), 100.0)],
            languages_scanned: true,
            ..Default::default()
        },
    );
    app.meta.set(
        "d",
        Meta {
            availability: Some("offline".into()),
            ..Default::default()
        },
    );
    app.meta.set(
        "e",
        Meta {
            availability: Some("deleted".into()),
            ..Default::default()
        },
    );
    app.meta.set(
        "g",
        Meta {
            git_checked: true,
            git: None,
            ..Default::default()
        },
    );
}

fn sample_snapshot() -> SessionSnapshot {
    SessionSnapshot {
        workspaces: vec![
            WorkspaceInfo {
                workspace_id: "w1".into(),
                number: 1,
                label: "shop".into(),
                ..Default::default()
            },
            WorkspaceInfo {
                workspace_id: "w2".into(),
                number: 3,
                label: "crabs".into(),
                ..Default::default()
            },
        ],
        panes: vec![
            PaneInfo {
                pane_id: "w1:p1".into(),
                workspace_id: "w1".into(),
                cwd: Some("/srv/work/shop-backend".into()),
                agent_status: AgentStatus::Working,
                ..Default::default()
            },
            PaneInfo {
                pane_id: "w1:p2".into(),
                workspace_id: "w1".into(),
                cwd: Some("/srv/work/shop-backend/api".into()),
                agent_status: AgentStatus::Blocked,
                ..Default::default()
            },
            PaneInfo {
                pane_id: "w2:p1".into(),
                workspace_id: "w2".into(),
                cwd: Some("/srv/work/crabs".into()),
                agent_status: AgentStatus::Done,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

struct Fixture {
    app: App,
    _rx: Receiver<Msg>,
    dir: PathBuf,
}

fn fixture(
    cat: CatalogData,
    ctx: InvocationContext,
    client: Option<Client>,
    settings: Option<Value>,
) -> Fixture {
    let dir = tmp("ui");
    let paths = Paths {
        config_dir: dir.join("config"),
        state_dir: dir.join("state"),
    };
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(paths.catalog(), cat.to_json()).unwrap();
    if let Some(s) = settings {
        std::fs::write(paths.settings(), s.to_string()).unwrap();
    }
    let (tx, rx) = channel();
    let env = Env {
        paths,
        ctx,
        own_pane: None,
        client,
        background: false,
    };
    let app = App::new(env, tx, Theme::default());
    seed_meta(&app);
    Fixture { app, _rx: rx, dir }
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| app.render(f)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        let mut line = String::new();
        let mut skip = 0;
        for x in 0..w {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            line.push_str(sym);
            skip = crate::text::width(sym).saturating_sub(1);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Compares with `snapshots/<name>.txt`; writes it when missing or with UPDATE_SNAPSHOTS=1.
fn snapshot(name: &str, got: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        got, want,
        "snapshot {name} differs; rerun with UPDATE_SNAPSHOTS=1 to accept"
    );
}

fn select(app: &mut App, id: &str) {
    app.sel = app
        .items
        .iter()
        .position(|i| i.kind == ItemKind::Tile(id.into()))
        .unwrap();
}

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn here(p: &str) -> InvocationContext {
    InvocationContext {
        focused_pane_cwd: Some(p.into()),
        workspace_id: Some("w1".into()),
        ..Default::default()
    }
}

#[test]
fn render_normal() {
    let mut fx = fixture(sample_catalog(), here("/srv/work/crabs/src"), None, None);
    fx.app.on_msg(Msg::Snapshot(Box::new(sample_snapshot())));
    let s = render(&mut fx.app, 120, 40);
    assert!(s.contains("▦ Project Tiles"));
    assert!(s.contains("● here"));
    assert!(s.contains("◆ 1"));
    assert!(s.contains("⊘ offline"));
    assert!(s.contains("✗ missing"));
    assert!(s.contains("a1b2c3d fix: retry on 429 · 2 days"));
    assert!(s.contains("stale"));
    assert!(s.contains("▸ ● Archive (1)"));
    snapshot("normal", &s);
    std::fs::remove_dir_all(fx.dir).ok();
}

#[test]
fn render_compact_and_narrow() {
    let mut fx = fixture(
        sample_catalog(),
        InvocationContext::default(),
        None,
        Some(json!({"tile_size": "compact"})),
    );
    let s = render(&mut fx.app, 120, 24);
    assert!(s.contains("main"));
    assert!(!s.contains("a1b2c3d"));
    snapshot("compact", &s);
    let s = render(&mut fx.app, 40, 30);
    assert!(s.lines().all(|l| crate::text::width(l) <= 40));
    snapshot("narrow", &s);
    assert_eq!(fx.app.columns_for(40), 1);
    assert_eq!(fx.app.columns_for(120), 3);
    std::fs::remove_dir_all(fx.dir).ok();
}

#[test]
fn modals_survive_tiny_terminals() {
    let sizes = [
        (1, 1),
        (5, 2),
        (19, 24),
        (80, 2),
        (80, 6),
        (30, 8),
        (100, 32),
    ];
    for k in ['?', 'e', 'd', 'n'] {
        let mut fx = fixture(sample_catalog(), InvocationContext::default(), None, None);
        render(&mut fx.app, 100, 32);
        select(&mut fx.app, "a");
        fx.app.on_key(key(KeyCode::Char(k)));
        for (w, h) in sizes {
            render(&mut fx.app, w, h);
        }
        std::fs::remove_dir_all(fx.dir).ok();
    }
}

#[test]
fn render_empty_help_and_form() {
    let mut fx = fixture(
        CatalogData::default(),
        InvocationContext::default(),
        None,
        None,
    );
    let s = render(&mut fx.app, 90, 14);
    assert!(s.contains("No projects yet."));
    snapshot("empty", &s);
    std::fs::remove_dir_all(fx.dir).ok();

    let mut fx = fixture(sample_catalog(), InvocationContext::default(), None, None);
    render(&mut fx.app, 100, 32);
    fx.app.on_key(key(KeyCode::Char('?')));
    let s = render(&mut fx.app, 100, 32);
    assert!(s.contains("Keys"));
    snapshot("help", &s);
    fx.app.on_key(key(KeyCode::Char('x'))); // any key closes help
    select(&mut fx.app, "a");
    fx.app.on_key(key(KeyCode::Char('e')));
    let s = render(&mut fx.app, 100, 32);
    assert!(s.contains("Project: shop-backend"));
    assert!(s.contains("◂ Work ▸"));
    snapshot("form", &s);
    std::fs::remove_dir_all(fx.dir).ok();
}

#[test]
fn form_saves_and_validates() {
    let mut fx = fixture(sample_catalog(), InvocationContext::default(), None, None);
    render(&mut fx.app, 120, 40);
    select(&mut fx.app, "a");
    fx.app.on_key(key(KeyCode::Char('e')));
    // Name field: replace with "api".
    fx.app
        .on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    for c in "api".chars() {
        fx.app.on_key(key(KeyCode::Char(c)));
    }
    // Colour: invalid first.
    for _ in 0..3 {
        fx.app.on_key(key(KeyCode::Tab));
    }
    for c in "zzz".chars() {
        fx.app.on_key(key(KeyCode::Char(c)));
    }
    fx.app.on_key(key(KeyCode::Enter));
    let s = render(&mut fx.app, 120, 40);
    assert!(s.contains("unknown colour \"zzz\""), "{s}");
    fx.app
        .on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    for c in "teal".chars() {
        fx.app.on_key(key(KeyCode::Char(c)));
    }
    // Collection → none.
    for _ in 0..3 {
        fx.app.on_key(key(KeyCode::Tab));
    }
    fx.app.on_key(key(KeyCode::Left));
    fx.app.on_key(key(KeyCode::Enter));
    let p = fx.app.cat().project("a").unwrap();
    assert_eq!(p.name, "api");
    assert_eq!(p.color.as_deref(), Some("#8ec07c"));
    assert_eq!(p.collection_id, None);
    let on_disk =
        CatalogData::parse(&std::fs::read_to_string(fx.app.store.path.clone()).unwrap()).unwrap();
    assert_eq!(on_disk.project("a").unwrap().name, "api");
    std::fs::remove_dir_all(fx.dir).ok();
}

#[test]
fn navigation_and_reorder() {
    let mut fx = fixture(sample_catalog(), InvocationContext::default(), None, None);
    render(&mut fx.app, 120, 40);
    let kinds: Vec<_> = fx.app.items.iter().map(|i| i.kind.clone()).collect();
    // Sections: ungrouped (d, e, g), Work (a, b), Archive (collapsed).
    assert!(matches!(
        &kinds[0],
        ItemKind::Header {
            collection: None,
            count: 3
        }
    ));
    // Starts on the first catalog project.
    assert_eq!(fx.app.items[fx.app.sel].kind, ItemKind::Tile("a".into()));
    fx.app.sel = 0;
    fx.app.nav((0, 1));
    assert_eq!(fx.app.items[fx.app.sel].kind, ItemKind::Tile("d".into()));
    fx.app.nav((1, 0));
    assert_eq!(fx.app.items[fx.app.sel].kind, ItemKind::Tile("e".into()));
    fx.app.nav((0, 1)); // down → Work header
    assert!(matches!(
        fx.app.items[fx.app.sel].kind,
        ItemKind::Header {
            collection: Some(_),
            ..
        }
    ));
    fx.app.nav((0, 1));
    assert_eq!(fx.app.items[fx.app.sel].kind, ItemKind::Tile("a".into()));
    fx.app.on_key(key(KeyCode::Char('J')));
    render(&mut fx.app, 120, 40);
    let order: Vec<_> = fx
        .app
        .cat()
        .projects
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(order, vec!["b", "a", "d", "e", "f", "g"]);
    assert_eq!(fx.app.items[fx.app.sel].kind, ItemKind::Tile("a".into()));
    // Search narrows and hides empty collections.
    fx.app.on_key(key(KeyCode::Char('/')));
    for c in "crabs".chars() {
        fx.app.on_key(key(KeyCode::Char(c)));
    }
    fx.app.on_key(key(KeyCode::Enter));
    render(&mut fx.app, 120, 40);
    let tiles: Vec<_> = fx
        .app
        .items
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Tile(_)))
        .collect();
    assert_eq!(tiles.len(), 1);
    std::fs::remove_dir_all(fx.dir).ok();
}

#[test]
fn import_merges_vscode_and_panes() {
    let home = tmp("home");
    let repo = home.join("repo");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    let mut cat = CatalogData::default();
    cat.collections.push(Collection {
        id: "mine".into(),
        name: "work".into(),
        ..Default::default()
    });
    let src = CatalogData::parse(
        r##"{"version":1,"collections":[{"id":"c1","name":"Work","color":"#ff0000","description":"d"},{"id":"c2","name":"Fun"}],
            "projects":[{"id":"x","name":"X","path":"/a/x","logo":{"kind":"emoji","value":"🎮"},"collectionId":"c1"},
                        {"id":"y","name":"Y","path":"/a/y.code-workspace"},
                        {"id":"z","name":"Z","path":"/a/z","logo":{"kind":"image","value":"z.png"},"collectionId":"c2"}]}"##,
    )
    .unwrap();
    let h = home.to_string_lossy().to_string();
    let cwds = vec![
        repo.join("sub").to_string_lossy().to_string(),
        repo.to_string_lossy().to_string(),
        h.clone(),
        "/no/such".into(),
    ];
    let top = |p: &str| {
        if p.ends_with("/sub") {
            p.trim_end_matches("/sub").to_string()
        } else {
            p.to_string()
        }
    };
    let n = ui::import_into(&mut cat, Some(&src), &cwds, &h, &|_| "#000000".into(), &top);
    assert_eq!(n, 3); // x, z, repo
    assert_eq!(cat.collections.len(), 2);
    assert_eq!(cat.collections[0].color.as_deref(), Some("#ff0000"));
    // A new collection without a colour in the source stays without one.
    assert_eq!(cat.collections[1].name, "Fun");
    assert_eq!(cat.collections[1].color, None);
    let x = &cat.projects[cat.find_path("/a/x").unwrap()];
    assert_eq!(x.emoji(), Some("🎮"));
    assert_eq!(x.collection_id.as_deref(), Some("mine"));
    let z = &cat.projects[cat.find_path("/a/z").unwrap()];
    assert_eq!(z.logo.as_ref().unwrap().kind, "auto");
    assert!(cat.find_path(&repo.to_string_lossy()).is_some());
    // A project already listed without a collection adopts its VS Code collection.
    let i = cat.find_path("/a/x").unwrap();
    cat.projects[i].collection_id = None;
    assert_eq!(
        ui::import_into(&mut cat, Some(&src), &cwds, &h, &|_| "#000000".into(), &top),
        1
    );
    assert_eq!(cat.projects[i].collection_id.as_deref(), Some("mine"));
    // Again: nothing new.
    assert_eq!(
        ui::import_into(&mut cat, Some(&src), &cwds, &h, &|_| "#000000".into(), &top),
        0
    );
    std::fs::remove_dir_all(home).ok();
}

// ---------------------------------------------------------------------- fake herdr server

type Handler = dyn Fn(&Value, &mut dyn FnMut(Value), usize) -> bool + Send + Sync;

struct FakeHerdr {
    path: PathBuf,
    log: Arc<Mutex<Vec<Value>>>,
    dir: PathBuf,
}

impl FakeHerdr {
    /// `handler(request, send, nth_subscribe)` answers; returning false closes the connection.
    fn start(handler: Arc<Handler>) -> FakeHerdr {
        // Unix socket paths are short (SUN_LEN); macOS temp dirs are long.
        let dir = PathBuf::from(format!("/tmp/pt-{}", &crate::text::new_id()[..8]));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let l2 = log.clone();
        let subs = Arc::new(Mutex::new(0usize));
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let log = l2.clone();
                let handler = handler.clone();
                let subs = subs.clone();
                std::thread::spawn(move || {
                    let mut w = conn.try_clone().unwrap();
                    let mut r = BufReader::new(conn);
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let req: Value = serde_json::from_str(line.trim()).unwrap();
                        log.lock().unwrap().push(req.clone());
                        let nth = if req["method"] == "events.subscribe" {
                            let mut s = subs.lock().unwrap();
                            *s += 1;
                            *s
                        } else {
                            0
                        };
                        let mut send = |v: Value| {
                            let _ = w.write_all(format!("{v}\n").as_bytes());
                        };
                        if !handler(&req, &mut send, nth) {
                            return;
                        }
                    }
                });
            }
        });
        FakeHerdr { path, log, dir }
    }
    fn client(&self) -> Client {
        let mut c = Client::new(&self.path);
        c.timeout = Duration::from_secs(3);
        c
    }
    fn methods(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|v| v["method"].as_str().unwrap_or("").to_string())
            .collect()
    }
    fn find(&self, method: &str) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["method"] == method)
            .cloned()
            .collect()
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn ok(req: &Value, result: Value) -> Value {
    json!({ "id": req["id"], "result": result })
}

fn snapshot_json(dirs: &[(&str, u64, &str)]) -> Value {
    let workspaces: Vec<Value> = dirs
        .iter()
        .map(|(id, n, _)| json!({"workspace_id": id, "label": id, "number": n, "focused": false,
            "active_tab_id": format!("{id}:t1"), "agent_status": "idle", "pane_count": 1, "tab_count": 1, "tokens": {}}))
        .collect();
    let panes: Vec<Value> = dirs
        .iter()
        .map(|(id, _, cwd)| json!({"pane_id": format!("{id}:p1"), "workspace_id": id, "tab_id": format!("{id}:t1"),
            "terminal_id": "t", "focused": false, "agent_status": "working", "revision": 1, "cwd": cwd}))
        .collect();
    json!({"type": "session_snapshot", "snapshot": {"version": "0.9.5", "protocol": 22, "workspaces": workspaces,
        "tabs": [], "panes": panes, "agents": [], "layouts": []}})
}

fn standard_handler(snapshot: Value) -> Arc<Handler> {
    Arc::new(
        move |req: &Value, send: &mut dyn FnMut(Value), _nth: usize| {
            let m = req["method"].as_str().unwrap_or("");
            let r = match m {
                "session.snapshot" => snapshot.clone(),
                "workspace.create" => json!({"type": "workspace_created",
                "workspace": {"workspace_id": "w9", "label": "x", "number": 9},
                "tab": {"tab_id": "w9:t1"}, "root_pane": {"pane_id": "w9:p1"}}),
                "workspace.focus" => {
                    json!({"type": "workspace_info", "workspace": {"workspace_id": req["params"]["workspace_id"]}})
                }
                "tab.create" => {
                    json!({"type": "tab_created", "tab": {"tab_id": "w1:t2"}, "root_pane": {"pane_id": "w1:p9"}})
                }
                "events.subscribe" => json!({"type": "subscription_started"}),
                "notification.show" => {
                    json!({"type": "notification_show", "shown": false, "reason": "disabled"})
                }
                _ => json!({"type": "ok"}),
            };
            send(ok(req, r));
            true
        },
    )
}

#[test]
fn open_focuses_existing_or_creates() {
    let repo = tmp("proj");
    let other = tmp("proj2");
    let r = repo.to_string_lossy().to_string();
    let o = other.to_string_lossy().to_string();
    let fake = FakeHerdr::start(standard_handler(snapshot_json(&[("w1", 1, &r)])));
    let mut cat = CatalogData::default();
    cat.projects.push(project("p", "proj", &r));
    cat.projects.push(project("q", "other", &o));
    let mut fx = fixture(
        cat,
        InvocationContext {
            workspace_id: Some("w1".into()),
            ..Default::default()
        },
        Some(fake.client()),
        None,
    );
    fx.app
        .on_msg(Msg::Snapshot(Box::new(fake.client().snapshot().unwrap())));

    let p = fx.app.cat().project("p").unwrap().clone();
    fx.app.open(&p, ui::OpenMode::Workspace);
    assert!(fx.app.quit);
    assert_eq!(
        fake.find("workspace.focus")[0]["params"]["workspace_id"],
        "w1"
    );
    assert!(fake.find("workspace.create").is_empty());
    let rep = fake.find("workspace.report_metadata");
    assert_eq!(rep[0]["params"]["workspace_id"], "w1");
    assert_eq!(rep[0]["params"]["source"], "plugin:vndv.project-tiles");
    assert_eq!(rep[0]["params"]["tokens"]["pt_id"], "p");

    fx.app.quit = false;
    let q = fx.app.cat().project("q").unwrap().clone();
    fx.app.open(&q, ui::OpenMode::Workspace);
    let c = fake.find("workspace.create");
    assert_eq!(c[0]["params"]["cwd"], o);
    assert_eq!(c[0]["params"]["label"], "other");
    assert_eq!(c[0]["params"]["focus"], true);
    assert_eq!(
        fake.find("workspace.report_metadata")[1]["params"]["workspace_id"],
        "w9"
    );

    fx.app.open(&p, ui::OpenMode::New);
    assert_eq!(fake.find("workspace.create").len(), 2);

    fx.app.open(&p, ui::OpenMode::Tab);
    let t = fake.find("tab.create");
    assert_eq!(t[0]["params"]["workspace_id"], "w1");
    assert_eq!(t[0]["params"]["cwd"], r);

    std::fs::remove_dir_all(repo).ok();
    std::fs::remove_dir_all(other).ok();
    std::fs::remove_dir_all(fx.dir).ok();
}

#[test]
fn open_without_folder_or_socket_reports_error() {
    let mut cat = CatalogData::default();
    cat.projects.push(project("p", "proj", "/no/such/folder"));
    let mut fx = fixture(cat, InvocationContext::default(), None, None);
    let p = fx.app.cat().project("p").unwrap().clone();
    fx.app.open(&p, ui::OpenMode::Workspace);
    assert!(!fx.app.quit);
    assert!(
        fx.app
            .status
            .as_ref()
            .unwrap()
            .0
            .contains("folder unavailable")
    );
    std::fs::remove_dir_all(fx.dir).ok();
}

#[test]
fn tag_reports_only_changes_and_clears_stale() {
    let fake = FakeHerdr::start(standard_handler(json!({})));
    let mut cat = CatalogData::default();
    cat.projects.push(project("p", "shop-backend", "/srv/shop"));
    let snap: SessionSnapshot = serde_json::from_value(
        snapshot_json(&[
            ("w1", 1, "/srv/shop/src"),
            ("w2", 2, "/tmp/elsewhere"),
            ("w3", 3, "/srv/shop"),
        ])["snapshot"]
            .clone(),
    )
    .unwrap();
    let mut snap = snap;
    snap.workspaces[1]
        .tokens
        .insert("pt_id".into(), "old".into());
    snap.workspaces[2].tokens = [
        ("pt_id".to_string(), "p".to_string()),
        ("pt_project".to_string(), "SB shop-backend".to_string()),
    ]
    .into();
    let meta = Meta {
        git: Some(GitInfo {
            ahead: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    let n = crate::presence::tag_all(&fake.client(), &cat, &snap, &|_| Some(meta.clone()), None);
    assert_eq!((n.sent, n.error), (3, None));
    let reps = fake.find("workspace.report_metadata");
    let by_ws = |w: &str| {
        reps.iter()
            .find(|r| r["params"]["workspace_id"] == w)
            .unwrap()["params"]["tokens"]
            .clone()
    };
    assert_eq!(by_ws("w1")["pt_git"], "↑1");
    assert_eq!(by_ws("w1")["pt_project"], "SB shop-backend");
    assert_eq!(by_ws("w2")["pt_id"], Value::Null);
    // Only keys the workspace actually has are cleared.
    assert_eq!(by_ws("w2").as_object().unwrap().len(), 1);
    let w3 = by_ws("w3");
    assert!(w3.get("pt_id").is_none() && w3.get("pt_project").is_none());
    assert_eq!(w3["pt_git"], "↑1");
}

#[test]
fn events_lost_resubscribes_and_rereads_snapshot() {
    // The first lifecycle subscription is acked, then reports events_lost and closes.
    let snap = snapshot_json(&[("w1", 1, "/srv/a")]);
    let handler: Arc<Handler> = Arc::new(
        move |req: &Value, send: &mut dyn FnMut(Value), nth: usize| match req["method"]
            .as_str()
            .unwrap_or("")
        {
            "session.snapshot" => {
                send(ok(req, snap.clone()));
                true
            }
            "events.subscribe" => {
                send(ok(req, json!({"type": "subscription_started"})));
                let lifecycle = req["params"]["subscriptions"][0]["type"] == "workspace.created";
                if lifecycle && nth == 1 {
                    std::thread::sleep(Duration::from_millis(150));
                    send(
                        json!({"event": "pane_created", "data": {"type": "pane_created", "pane": {"pane_id": "w1:p2"}}}),
                    );
                    send(
                        json!({"id": req["id"], "error": {"code": "events_lost", "message": "subscriber fell behind"}}),
                    );
                    return false;
                }
                true
            }
            _ => {
                send(ok(req, json!({"type": "ok"})));
                true
            }
        },
    );
    let fake = FakeHerdr::start(handler);
    let (tx, rx) = channel();
    let tx = Mutex::new(tx);
    crate::live::start(
        fake.client(),
        Box::new(move |e| {
            if let crate::live::LiveEvent::Snapshot(s) = e {
                let _ = tx.lock().unwrap().send(s.workspaces.len());
            }
        }),
    );
    // Index of the nth subscribe of a kind in the request log.
    let nth_sub = |kind: &str, n: usize| {
        let log = fake.log.lock().unwrap();
        log.iter()
            .enumerate()
            .filter(|(_, r)| {
                r["method"] == "events.subscribe" && r["params"]["subscriptions"][0]["type"] == kind
            })
            .nth(n)
            .map(|(i, _)| i)
    };
    let snapshot_after = |i: Option<usize>| {
        i.is_some_and(|i| fake.methods()[i..].iter().any(|x| x == "session.snapshot"))
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut snapshots = 0;
    while std::time::Instant::now() < deadline {
        if rx.recv_timeout(Duration::from_millis(100)).is_ok() {
            snapshots += 1;
        }
        if snapshot_after(nth_sub("workspace.created", 1))
            && snapshot_after(nth_sub("pane.agent_status_changed", 0))
        {
            break;
        }
    }
    // The agent subscription's ack is followed by a reread.
    assert!(
        snapshot_after(nth_sub("pane.agent_status_changed", 0)),
        "{:?}",
        fake.methods()
    );
    let subs = fake.find("events.subscribe");
    let lifecycle: Vec<_> = subs
        .iter()
        .filter(|r| r["params"]["subscriptions"][0]["type"] == "workspace.created")
        .collect();
    assert!(lifecycle.len() >= 2, "resubscribed: {:?}", fake.methods());
    assert!(snapshots >= 2, "snapshot reread after events_lost");
    // The agent-status subscription names each pane.
    let agent = subs
        .iter()
        .find(|r| r["params"]["subscriptions"][0]["type"] == "pane.agent_status_changed")
        .unwrap();
    assert_eq!(agent["params"]["subscriptions"][0]["pane_id"], "w1:p1");
    // Order: the resubscribe comes before the reread snapshot.
    let m = fake.methods();
    let second_sub = m
        .iter()
        .enumerate()
        .filter(|(i, x)| {
            *x == "events.subscribe"
                && fake.log.lock().unwrap()[*i]["params"]["subscriptions"][0]["type"]
                    == "workspace.created"
        })
        .nth(1)
        .unwrap()
        .0;
    assert!(m[second_sub..].iter().any(|x| x == "session.snapshot"));
}

#[test]
fn socket_errors_are_reported() {
    let handler: Arc<Handler> = Arc::new(|req: &Value, send: &mut dyn FnMut(Value), _| {
        send(
            json!({"id": req["id"], "error": {"code": "not_found", "message": "workspace not found"}}),
        );
        true
    });
    let fake = FakeHerdr::start(handler);
    let e = fake
        .client()
        .request("workspace.focus", json!({"workspace_id": "nope"}))
        .unwrap_err();
    assert_eq!(e.code, "not_found");
    assert_eq!(e.to_string(), "workspace not found");
    let missing = Client::new("/no/such/herdr.sock")
        .request("ping", json!({}))
        .unwrap_err();
    assert_eq!(missing.code, "io");
}
