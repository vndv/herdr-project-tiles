//! Project Tiles for herdr: a tile dashboard of projects in a plugin pane.
//!
//! project-tiles tui          panes entrypoint `tiles`
//! project-tiles open         action `open`: opens the pane through the socket
//! project-tiles add-current  action `add-current`: adds the focused folder, shows a toast
//! project-tiles tag          startup / `workspace.created` hook: Spaces sidebar tokens
//! project-tiles app …        JSON backend for the Herdr.app mod

mod api;
mod app;
mod catalog;
mod form;
mod git;
mod langs;
mod live;
mod meta;
mod presence;
mod settings;
mod socket;
mod text;
mod theme;
mod ui;

#[cfg(test)]
mod tests;

use api::{InvocationContext, PLUGIN_ID};
use catalog::{CatalogStore, Project};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use serde_json::json;
use settings::{Paths, Settings};
use socket::Client;
use std::process::ExitCode;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "tui".into());
    let r = match cmd.as_str() {
        "tui" => run_tui(),
        "open" => cmd_open(),
        "add-current" => cmd_add_current(),
        "tag" => cmd_tag(),
        "app" => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            match app::run(&rest) {
                Ok(v) => {
                    println!("{v}");
                    Ok(())
                }
                Err(e) => {
                    println!("{}", serde_json::json!({ "error": e }));
                    Err(e)
                }
            }
        }
        "--version" | "version" => {
            println!("project-tiles {VERSION}");
            Ok(())
        }
        _ => Err(format!(
            "unknown command `{cmd}`; use tui | open | add-current | tag | app"
        )),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("project-tiles: {e}");
            ExitCode::FAILURE
        }
    }
}

fn client() -> Result<Client, String> {
    Client::from_env().ok_or_else(|| "HERDR_SOCKET_PATH is not set (run from herdr)".to_string())
}

fn plugin_id() -> String {
    std::env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| PLUGIN_ID.into())
}

/// Action `open`: `plugin.pane.open` over the socket (no shell, no `herdr` in PATH needed).
fn cmd_open() -> Result<(), String> {
    let c = client()?;
    match c.request(
        "plugin.pane.open",
        json!({ "plugin_id": plugin_id(), "entrypoint": "tiles", "focus": true }),
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            presence::notify(&c, "Project Tiles: could not open", &e.to_string());
            Err(e.to_string())
        }
    }
}

/// Action `add-current`: adds the focused pane's folder (git root) to the catalog.
fn cmd_add_current() -> Result<(), String> {
    let ctx = InvocationContext::from_env();
    let here = ctx
        .here()
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
        .ok_or("no folder")?;
    if !std::path::Path::new(&here).is_dir() {
        return Err(format!("not a directory: {here}"));
    }
    let paths = Paths::from_env();
    let mut store = CatalogStore::open(paths.catalog());
    if let Some(e) = &store.error {
        return Err(format!(
            "projects.json is not valid JSON, not touching it: {e}"
        ));
    }
    let root = git::toplevel(&here);
    let c = Client::from_env();
    let (title, body) = match store.data.find_path(&root) {
        Some(i) => (
            "Project Tiles: already listed",
            store.data.projects[i].name.clone(),
        ),
        None => {
            let p = Project::new(&root);
            let name = p.name.clone();
            store.data.projects.push(p);
            store.save().map_err(|e| e.to_string())?;
            (
                "Project Tiles: added",
                format!("{name} — {}", text::short_path(&root)),
            )
        }
    };
    println!("{title}: {body}");
    if let Some(c) = &c {
        presence::notify(c, title, &body);
        tag_with(c, &paths);
    }
    Ok(())
}

/// Hook `tag`: refreshes local git state of the open projects and reports sidebar tokens.
fn cmd_tag() -> Result<(), String> {
    let c = client()?;
    let r = tag_with(&c, &Paths::from_env());
    println!("project-tiles tag: {} workspace(s) updated", r.sent);
    match r.error {
        Some(e) => Err(format!("token report failed: {e}")),
        None => Ok(()),
    }
}

fn tag_with(c: &Client, paths: &Paths) -> presence::Tagged {
    let snap = match c.snapshot() {
        Ok(s) => s,
        Err(e) => {
            return presence::Tagged {
                sent: 0,
                error: Some(e.to_string()),
            };
        }
    };
    let store = CatalogStore::open(paths.catalog());
    let settings = Settings::load(&paths.settings());
    let md = meta::Metadata::new(Some(paths.cache()), std::sync::Arc::new(|_: &str| {}));
    let roots = presence::worktree_roots(&store.data, &|p| md.get(&p.id));
    let open = presence::workspace_projects(&store.data, &snap, None, &roots);
    // Fresh local git info (no network) for the projects that are open.
    for id in open.values() {
        if let Some(p) = store.data.project(id)
            && settings.show_git
        {
            let mut m = md.get(&p.id).unwrap_or_default();
            m.path = p.path.clone();
            m.git = git::info(&p.path, false);
            m.git_checked = true;
            md.set(&p.id, m);
        }
    }
    md.flush();
    presence::tag_all(c, &store.data, &snap, &|p| md.get(&p.id), None)
}

fn run_tui() -> Result<(), String> {
    let paths = Paths::from_env();
    let _ = std::fs::create_dir_all(&paths.config_dir);
    let _ = std::fs::create_dir_all(&paths.state_dir);
    let client = Client::from_env();
    let env = ui::Env {
        paths: paths.clone(),
        ctx: InvocationContext::from_env(),
        own_pane: std::env::var("HERDR_PANE_ID")
            .ok()
            .filter(|s| !s.is_empty()),
        client: client.clone(),
        background: true,
    };
    let (tx, rx) = channel::<ui::Msg>();
    if let Some(c) = client.clone() {
        let ltx = std::sync::Mutex::new(tx.clone());
        live::start(
            c,
            Box::new(move |e| {
                let m = match e {
                    live::LiveEvent::Snapshot(s) => ui::Msg::Snapshot(s),
                    live::LiveEvent::Down(e) => ui::Msg::LiveDown(e),
                };
                let _ = ltx.lock().unwrap().send(m);
            }),
        );
    }
    let mut app = ui::App::new(env, tx, theme::Theme::load());

    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let res = event_loop(&mut terminal, &mut app, &rx);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    app.meta.flush();
    res
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut ui::App,
    rx: &std::sync::mpsc::Receiver<ui::Msg>,
) -> Result<(), String> {
    let mut last_reload = Instant::now();
    let mut last_flush = Instant::now();
    let mut last_tag = Instant::now() - Duration::from_secs(10);
    let tagging = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    while !app.quit {
        if app.need_clear {
            app.need_clear = false;
            let _ = terminal.clear();
        }
        terminal
            .draw(|f| app.render(f))
            .map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(100)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
                Event::Mouse(m) => app.on_mouse(m),
                Event::Resize(_, _) => app.need_clear = true,
                _ => {}
            }
        }
        while let Ok(m) = rx.try_recv() {
            app.on_msg(m);
        }
        if app.want_editor {
            app.want_editor = false;
            run_editor(terminal, app);
        }
        if last_reload.elapsed() > Duration::from_secs(1) {
            last_reload = Instant::now();
            app.reload_catalog();
        }
        if last_flush.elapsed() > Duration::from_secs(2) {
            last_flush = Instant::now();
            app.meta.flush();
        }
        // Sidebar tokens, at most once a second, off the UI thread.
        if app.need_tag
            && last_tag.elapsed() > Duration::from_secs(1)
            && !tagging.load(std::sync::atomic::Ordering::SeqCst)
            && let (Some(c), Some(snap)) = (app.env.client.clone(), app.snap.clone())
        {
            app.need_tag = false;
            last_tag = Instant::now();
            let cat = app.store.data.clone();
            let md = app.meta.clone();
            let own = app.env.own_pane.clone();
            let flag = tagging.clone();
            let tx = app.tx.clone();
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::spawn(move || {
                let r = presence::tag_all(&c, &cat, &snap, &|p| md.get(&p.id), own.as_deref());
                if let Some(e) = r.error {
                    let _ = tx.send(ui::Msg::TagError(e));
                }
                flag.store(false, std::sync::atomic::Ordering::SeqCst);
            });
        }
    }
    Ok(())
}

/// `E`: leaves the alternate screen, runs `$VISUAL`/`$EDITOR` on projects.json, comes back.
fn run_editor(terminal: &mut ratatui::DefaultTerminal, app: &mut ui::App) {
    if !app.store.path.exists() {
        let _ = app.store.save();
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let mut parts = editor.split_whitespace();
    let Some(bin) = parts.next() else { return };
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    let status = std::process::Command::new(bin)
        .args(parts)
        .arg(&app.store.path)
        .status();
    *terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    if let Err(e) = status {
        app.status = Some((format!("could not start {editor}: {e}"), true));
    }
    app.after_editor();
}
