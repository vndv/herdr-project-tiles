//! The dashboard: state, layout, drawing and keys.

use crate::api::{InvocationContext, SessionSnapshot};
use crate::catalog::{CatalogData, CatalogStore, Logo, Project};
use crate::form::{Field, Form, Outcome};
use crate::git::{self, RemoteOp};
use crate::langs;
use crate::meta::{Meta, Metadata};
use crate::presence::{self, Presence};
use crate::settings::{Paths, Settings};
use crate::socket::Client;
use crate::text::{self, norm, short_path, trunc, trunc_left, width};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Messages from background threads.
pub enum Msg {
    Meta,
    Snapshot(Box<SessionSnapshot>),
    LiveDown(String),
    GitDone {
        id: String,
        op: RemoteOp,
        ok: bool,
        message: String,
    },
}

/// What the dashboard knows about where it runs.
pub struct Env {
    pub paths: Paths,
    pub ctx: InvocationContext,
    /// Our own pane (overlay), left out of "open" and agent counts.
    pub own_pane: Option<String>,
    pub client: Option<Client>,
    /// Start git/language workers (off in tests).
    pub background: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Header {
        collection: Option<String>,
        count: usize,
    },
    Tile(String),
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    pub x: u16,
    pub y: usize,
    pub w: u16,
    pub h: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenMode {
    Workspace,
    New,
    Tab,
}

enum Purpose {
    /// `None` id: a new project (the draft is kept here).
    EditProject {
        id: Option<String>,
        draft: Box<Project>,
    },
    AddPath,
    EditCollection {
        id: Option<String>,
    },
    Move {
        id: String,
    },
    Worktree {
        id: String,
    },
}

enum Modal {
    Form(Box<Form>, Purpose),
    Confirm(String, ConfirmAction),
}

enum ConfirmAction {
    RemoveProject(String),
    RemoveCollection(String),
}

enum Focus {
    Tile(String),
    Header(String),
}

pub const HELP: &[(&str, &str)] = &[
    ("←↑↓→ hjkl", "навигация"),
    ("Enter", "открыть (перейти в workspace или создать)"),
    ("o", "открыть в новом workspace"),
    ("t", "открыть новой вкладкой в текущем workspace"),
    ("w", "новый git worktree (workspace herdr)"),
    ("/", "поиск (имя, путь, описание, языки, ветка)"),
    ("a", "добавить текущую папку"),
    ("A", "добавить папку по пути"),
    ("I", "импорт: открытые workspace + Project Tiles (VS Code)"),
    ("e", "редактировать проект / коллекцию"),
    ("d", "убрать проект / коллекцию из списка"),
    ("m", "переместить в коллекцию"),
    ("n", "новая коллекция"),
    ("J / K", "сдвинуть вниз / вверх"),
    ("Space", "свернуть/развернуть коллекцию"),
    ("f p P", "git fetch / pull --ff-only / push"),
    ("r / R", "обновить проект / все"),
    ("O", "показать в Finder"),
    ("c", "compact / normal"),
    ("E", "открыть projects.json в $EDITOR"),
    (
        "мышь",
        "клик — выбрать, двойной — открыть, колесо — листать",
    ),
    ("q Esc", "закрыть"),
];

const NO_COLLECTION: &str = "— без коллекции —";

pub struct App {
    pub env: Env,
    pub settings: Settings,
    pub store: CatalogStore,
    pub meta: Metadata,
    pub theme: Theme,
    pub snap: Option<SessionSnapshot>,
    pub presence: HashMap<String, Presence>,
    pub live_error: Option<String>,
    pub sel: usize,
    pub scroll: usize,
    pub query: String,
    pub searching: bool,
    pub status: Option<(String, bool)>,
    pub help: bool,
    modal: Option<Modal>,
    pub items: Vec<Item>,
    pending: Option<Focus>,
    pub quit: bool,
    pub tx: Sender<Msg>,
    /// Layout changed (compact/normal, resize): repaint everything.
    pub need_clear: bool,
    pub want_editor: bool,
    pub need_tag: bool,
    git_busy: HashSet<String>,
    last_click: Option<(Instant, usize)>,
    body_top: u16,
    last_width: u16,
}

impl App {
    pub fn new(env: Env, tx: Sender<Msg>, theme: Theme) -> App {
        let settings = Settings::load(&env.paths.settings());
        let store = CatalogStore::open(env.paths.catalog());
        let mtx = std::sync::Mutex::new(tx.clone());
        let cache = if env.background {
            Some(env.paths.cache())
        } else {
            None
        };
        let meta = Metadata::new(
            cache,
            std::sync::Arc::new(move |_: &str| {
                let _ = mtx.lock().unwrap().send(Msg::Meta);
            }),
        );
        let mut app = App {
            env,
            settings,
            store,
            meta,
            theme,
            snap: None,
            presence: HashMap::new(),
            live_error: None,
            sel: 0,
            scroll: 0,
            query: String::new(),
            searching: false,
            status: None,
            help: false,
            modal: None,
            items: Vec::new(),
            pending: None,
            quit: false,
            tx,
            need_clear: false,
            want_editor: false,
            need_tag: false,
            git_busy: HashSet::new(),
            last_click: None,
            body_top: 3,
            last_width: 0,
        };
        if let Some(e) = &app.store.error {
            app.msg(&format!("projects.json не читается: {e}"), true);
        }
        // Start on the project we are in, else on the first one.
        let start = app
            .env
            .ctx
            .here()
            .and_then(|h| presence::project_for_cwd(&app.store.data, &h).map(|p| p.id.clone()))
            .or_else(|| app.store.data.projects.first().map(|p| p.id.clone()));
        if let Some(id) = start {
            app.pending = Some(Focus::Tile(id));
        }
        app.refresh_all(false, None);
        app
    }

    pub fn cat(&self) -> &CatalogData {
        &self.store.data
    }

    fn msg(&mut self, s: &str, err: bool) {
        self.status = Some((s.to_string(), err));
    }

    fn refresh_all(&mut self, force: bool, fetch: Option<bool>) {
        if !self.env.background {
            return;
        }
        for p in self.store.data.projects.clone() {
            self.meta.refresh(&p, &self.settings, force, fetch);
        }
    }

    fn refresh_one(&self, p: &Project, force: bool, fetch: Option<bool>) {
        if self.env.background {
            self.meta.refresh(p, &self.settings, force, fetch);
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.store.save() {
            self.msg(&format!("не удалось сохранить projects.json: {e}"), true);
        }
        self.recompute_presence();
        self.need_tag = true;
    }

    // ------------------------------------------------------------------ background messages

    pub fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Meta => self.need_tag = true,
            Msg::Snapshot(s) => {
                self.snap = Some(*s);
                self.live_error = None;
                self.recompute_presence();
                self.need_tag = true;
            }
            Msg::LiveDown(e) => self.live_error = Some(e),
            Msg::GitDone {
                id,
                op,
                ok,
                message,
            } => {
                self.git_busy.remove(&id);
                let name = self
                    .cat()
                    .project(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                if ok {
                    self.msg(&format!("git {}: готово — {name}", op.name()), false);
                } else {
                    self.msg(
                        &format!("git {} ошибка — {name}: {message}", op.name()),
                        true,
                    );
                    if let Some(c) = &self.env.client {
                        presence::notify(
                            c,
                            &format!("git {}: ошибка — {name}", op.name()),
                            &message,
                        );
                    }
                }
                if let Some(p) = self.cat().project(&id).cloned() {
                    self.refresh_one(&p, true, Some(false));
                }
            }
        }
    }

    pub fn recompute_presence(&mut self) {
        self.presence = match &self.snap {
            Some(s) => presence::presence(&self.store.data, s, self.env.own_pane.as_deref()),
            None => HashMap::new(),
        };
    }

    /// Catalog changed on disk (hand edit, another client).
    pub fn reload_catalog(&mut self) {
        if self.store.reload() {
            self.recompute_presence();
            self.refresh_all(false, None);
            self.need_tag = true;
        }
        if let Some(e) = self.store.error.clone() {
            self.msg(
                &format!("projects.json не читается, оставлено прежнее: {e}"),
                true,
            );
        }
    }

    // ------------------------------------------------------------------ queries

    fn here_project(&self) -> Option<String> {
        let h = self.env.ctx.here()?;
        presence::project_for_cwd(self.cat(), &h).map(|p| p.id.clone())
    }

    fn matches(&self, p: &Project) -> bool {
        if self.query.is_empty() {
            return true;
        }
        let m = self.meta.get(&p.id).unwrap_or_default();
        let langs: Vec<String> = if p.manual_languages().is_empty() {
            m.languages.iter().map(|l| l.0.clone()).collect()
        } else {
            p.manual_languages().to_vec()
        };
        let hay = format!(
            "{} {} {} {} {}",
            p.name,
            p.path,
            p.description.as_deref().unwrap_or(""),
            langs.join(" "),
            m.git.as_ref().map(|g| g.branch.as_str()).unwrap_or("")
        )
        .to_lowercase();
        self.query
            .to_lowercase()
            .split_whitespace()
            .all(|w| hay.contains(w))
    }

    /// (collection id or None for "Без коллекции", project ids) in display order.
    fn sections(&self) -> Vec<(Option<String>, Vec<String>)> {
        let cat = self.cat();
        let mut out = Vec::new();
        let ungrouped: Vec<String> = cat
            .projects
            .iter()
            .filter(|p| !cat.is_grouped(p) && self.matches(p))
            .map(|p| p.id.clone())
            .collect();
        if !ungrouped.is_empty() || cat.collections.is_empty() {
            out.push((None, ungrouped));
        }
        for c in &cat.collections {
            let ps: Vec<String> = cat
                .projects
                .iter()
                .filter(|p| p.collection_id.as_deref() == Some(c.id.as_str()) && self.matches(p))
                .map(|p| p.id.clone())
                .collect();
            if !self.query.is_empty() && ps.is_empty() {
                continue;
            }
            out.push((Some(c.id.clone()), ps));
        }
        out
    }

    pub fn tile_height(&self) -> usize {
        let s = &self.settings;
        if s.compact() {
            return 5;
        }
        3 + s.show_path as usize
            + s.show_description as usize
            + 2 * s.show_languages as usize
            + 2 * s.show_git as usize
    }

    pub fn columns_for(&self, w: u16) -> usize {
        let min_w = if self.settings.compact() { 30 } else { 36 };
        let fit = (w.saturating_sub(2) as usize / min_w).max(1);
        self.settings.columns().min(fit)
    }

    /// Items with content coordinates; returns (items, total height).
    pub fn layout(&self, w: u16) -> (Vec<Item>, usize) {
        let cols = self.columns_for(w);
        let gap = 1u16;
        let tw = ((w.saturating_sub(2) - gap * (cols as u16 - 1)) / cols as u16).max(20);
        let th = self.tile_height();
        let mut items = Vec::new();
        let mut y = 0usize;
        let secs = self.sections();
        let only_ungrouped = secs.len() == 1 && secs[0].0.is_none();
        for (cid, ps) in secs {
            if !only_ungrouped {
                items.push(Item {
                    kind: ItemKind::Header {
                        collection: cid.clone(),
                        count: ps.len(),
                    },
                    x: 0,
                    y,
                    w: w.saturating_sub(1),
                    h: 1,
                });
                y += 1;
            }
            let collapsed = self.query.is_empty()
                && self
                    .cat()
                    .collection(cid.as_deref())
                    .is_some_and(|c| c.is_collapsed());
            if !collapsed {
                for (i, id) in ps.iter().enumerate() {
                    let (r, c) = (i / cols, i % cols);
                    items.push(Item {
                        kind: ItemKind::Tile(id.clone()),
                        x: 1 + c as u16 * (tw + gap),
                        y: y + r * th,
                        w: tw,
                        h: th,
                    });
                }
                if !ps.is_empty() {
                    y += ps.len().div_ceil(cols) * th;
                } else if !only_ungrouped {
                    items.push(Item {
                        kind: ItemKind::Empty,
                        x: 2,
                        y,
                        w: w.saturating_sub(4),
                        h: 1,
                    });
                    y += 1;
                }
            }
            y += 1;
        }
        (items, y)
    }

    fn current(&self) -> Option<&Item> {
        self.items.get(self.sel)
    }
    fn current_project(&self) -> Option<Project> {
        match &self.current()?.kind {
            ItemKind::Tile(id) => self.cat().project(id).cloned(),
            _ => None,
        }
    }
    fn current_header(&self) -> Option<Option<String>> {
        match &self.current()?.kind {
            ItemKind::Header { collection, .. } => Some(collection.clone()),
            _ => None,
        }
    }
    /// The collection of the selection (header or tile).
    fn current_collection(&self) -> Option<String> {
        match &self.current()?.kind {
            ItemKind::Header { collection, .. } => collection.clone(),
            ItemKind::Tile(id) => {
                let p = self.cat().project(id)?;
                self.cat()
                    .collection(p.collection_id.as_deref())
                    .map(|c| c.id.clone())
            }
            ItemKind::Empty => None,
        }
    }

    fn relayout(&mut self, w: u16) {
        let (items, _) = self.layout(w);
        self.items = items
            .into_iter()
            .filter(|i| i.kind != ItemKind::Empty)
            .collect();
        if let Some(f) = self.pending.take() {
            let pos = self.items.iter().position(|it| match (&f, &it.kind) {
                (Focus::Tile(id), ItemKind::Tile(t)) => id == t,
                (
                    Focus::Header(id),
                    ItemKind::Header {
                        collection: Some(c),
                        ..
                    },
                ) => id == c,
                _ => false,
            });
            if let Some(p) = pos {
                self.sel = p;
            }
        }
        if self.sel >= self.items.len() {
            self.sel = self.items.len().saturating_sub(1);
        }
    }

    pub fn nav(&mut self, dir: (i32, i32)) {
        if self.items.is_empty() {
            return;
        }
        let cur = self.items[self.sel].clone();
        if dir.0 != 0 {
            let n = self.sel as i32 + dir.0;
            self.sel = n.clamp(0, self.items.len() as i32 - 1) as usize;
            return;
        }
        let mut rows: Vec<usize> = self.items.iter().map(|i| i.y).collect();
        rows.sort();
        rows.dedup();
        let ri = rows.iter().position(|r| *r == cur.y).unwrap_or(0) as i32 + dir.1;
        if ri < 0 || ri >= rows.len() as i32 {
            return;
        }
        let row = rows[ri as usize];
        let cx = cur.x as i32
            + if matches!(cur.kind, ItemKind::Tile(_)) {
                cur.w as i32 / 2
            } else {
                0
            };
        if let Some((i, _)) = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.y == row)
            .min_by_key(|(_, it)| {
                if matches!(it.kind, ItemKind::Tile(_)) {
                    (it.x as i32 + it.w as i32 / 2 - cx).abs()
                } else {
                    0
                }
            })
        {
            self.sel = i;
        }
    }

    // ------------------------------------------------------------------ drawing

    pub fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        if area.width != self.last_width {
            self.last_width = area.width;
        }
        self.relayout(area.width);
        let cursor = self.draw(f.buffer_mut(), area);
        if let Some((x, y)) = cursor {
            f.set_cursor_position((x, y));
        }
    }

    /// Draws into `buf`; returns a cursor position when a text input is active.
    pub fn draw(&mut self, buf: &mut Buffer, area: Rect) -> Option<(u16, u16)> {
        let t = self.theme.clone();
        let (w, h) = (area.width, area.height);
        let top = 3u16;
        self.body_top = top;
        let body_h = h.saturating_sub(top + 1) as usize;
        let (all, total_h) = self.layout(w);
        // Keep the selection visible.
        if let Some(cur) = self.current().cloned() {
            if cur.y < self.scroll {
                self.scroll = cur
                    .y
                    .saturating_sub(matches!(cur.kind, ItemKind::Tile(_)) as usize);
            } else if cur.y + cur.h > self.scroll + body_h {
                self.scroll = cur.y + cur.h - body_h;
            }
        }
        self.scroll = self.scroll.min(total_h.saturating_sub(body_h));

        let mut p = Painter { buf, area };
        let st = |c: Color| Style::default().fg(c);
        // Header.
        let mut x = p.put(
            1,
            0,
            "▦ Project Tiles",
            st(t.accent).add_modifier(Modifier::BOLD),
            None,
        );
        x = p.put(
            x + 2,
            0,
            &projects_label(self.cat().projects.len()),
            st(t.subtext0),
            None,
        );
        let busy = self.meta.busy_count() + self.git_busy.len();
        if busy > 0 {
            x = p.put(x + 2, 0, &format!("⟳ {busy}"), st(t.yellow), None);
        }
        if self.env.client.is_none() {
            x = p.put(x + 2, 0, "без herdr", st(t.overlay0), None);
        } else if self.live_error.is_some() {
            x = p.put(x + 2, 0, "herdr: нет связи", st(t.red), None);
        }
        if let Some(here) = self.env.ctx.here() {
            let s = format!("здесь: {}", short_path(&here));
            let room = (w as usize / 2).max(10);
            let s = trunc_left(&s, room);
            let sx = w.saturating_sub(width(&s) as u16 + 2).max(x + 2);
            p.put(sx, 0, &s, st(t.overlay0), None);
        }
        let search = if self.searching || !self.query.is_empty() {
            format!("/ {}{}", self.query, if self.searching { "▏" } else { "" })
        } else {
            "/ поиск…   ? помощь".to_string()
        };
        p.put(
            1,
            1,
            &search,
            st(if self.searching { t.text } else { t.overlay0 }),
            None,
        );
        p.put(0, 2, &"─".repeat(w as usize), st(t.surface1), None);

        let here = self.here_project();
        let cur = self.current().cloned();
        for it in &all {
            let y = it.y as i64 - self.scroll as i64 + top as i64;
            if y + it.h as i64 <= top as i64 || y >= top as i64 + body_h as i64 {
                continue;
            }
            let sel = cur.as_ref() == Some(it);
            match &it.kind {
                ItemKind::Header { collection, count } => {
                    self.draw_header(&mut p, y as u16, collection.as_deref(), *count, sel)
                }
                ItemKind::Empty => {
                    p.put(
                        it.x,
                        y as u16,
                        "пусто — m на проекте, чтобы перенести сюда",
                        st(t.overlay0),
                        None,
                    );
                }
                ItemKind::Tile(id) => {
                    if let Some(pr) = self.cat().project(id) {
                        let ytop = top as i64;
                        let ybot = top as i64 + body_h as i64;
                        self.draw_tile(
                            &mut p,
                            pr,
                            it,
                            y,
                            sel,
                            here.as_deref() == Some(id.as_str()),
                            (ytop, ybot),
                        );
                    }
                }
            }
        }
        if self.cat().projects.is_empty() {
            let lines = [
                "Пока нет проектов.",
                "",
                "a — добавить папку текущей панели herdr",
                "A — добавить папку по пути",
                "I — импортировать открытые workspace и каталог Project Tiles из VS Code",
            ];
            for (i, l) in lines.iter().enumerate() {
                p.put(
                    (w.saturating_sub(70) / 2).max(1),
                    top + 1 + i as u16,
                    l,
                    st(t.subtext0),
                    None,
                );
            }
        }
        // Status line.
        let sy = h.saturating_sub(1);
        match &self.status {
            Some((s, err)) => {
                p.put(1, sy, s, st(if *err { t.red } else { t.green }), None);
            }
            None => {
                p.put(
                    1,
                    sy,
                    "Enter открыть · o new ws · t вкладка · a добавить · e правка · / поиск · ? помощь · q выход",
                    st(t.overlay0),
                    None,
                );
            }
        }
        if self.help {
            draw_help(p.buf, area, &t);
        }
        match &self.modal {
            Some(Modal::Form(form, _)) => return form.render(p.buf, area, &t),
            Some(Modal::Confirm(q, _)) => draw_confirm(p.buf, area, &t, q),
            None => {}
        }
        None
    }

    fn draw_header(&self, p: &mut Painter, y: u16, cid: Option<&str>, count: usize, sel: bool) {
        let t = &self.theme;
        let w = p.area.width;
        let (name, color, desc, collapsed) = match self.cat().collection(cid) {
            None => (
                "Без коллекции".to_string(),
                t.overlay0,
                String::new(),
                false,
            ),
            Some(c) => (
                c.name.clone(),
                c.color
                    .as_deref()
                    .and_then(crate::theme::parse_hex)
                    .unwrap_or(t.overlay0),
                c.description.clone().unwrap_or_default(),
                c.is_collapsed() && self.query.is_empty(),
            ),
        };
        let rev = if sel {
            Modifier::REVERSED
        } else {
            Modifier::empty()
        };
        let mut x = p.put(
            1,
            y,
            if collapsed { "▸ " } else { "▾ " },
            Style::default().fg(t.overlay0).add_modifier(rev),
            None,
        );
        x = p.put(
            x,
            y,
            "● ",
            Style::default().fg(color).add_modifier(rev),
            None,
        );
        x = p.put(
            x,
            y,
            &name,
            Style::default()
                .fg(t.text)
                .add_modifier(Modifier::BOLD | rev),
            None,
        );
        x = p.put(
            x + 1,
            y,
            &format!("({count})"),
            Style::default().fg(t.subtext0).add_modifier(rev),
            None,
        );
        if !desc.is_empty() {
            let room = (w as usize).saturating_sub(x as usize + 4);
            x = p.put(x + 2, y, &desc, Style::default().fg(t.overlay0), Some(room));
        }
        let rest = (w as usize).saturating_sub(x as usize + 2);
        p.put(
            x + 1,
            y,
            &"─".repeat(rest),
            Style::default().fg(t.surface1),
            None,
        );
    }

    fn tile_color(&self, pr: &Project) -> Color {
        let t = &self.theme;
        if let Some(c) = pr.color.as_deref().and_then(crate::theme::parse_hex) {
            return c;
        }
        if let Some(c) = self
            .cat()
            .collection(pr.collection_id.as_deref())
            .and_then(|c| c.color.as_deref())
            .and_then(crate::theme::parse_hex)
        {
            return c;
        }
        t.color(&t.pick_color(&pr.name))
    }

    fn shown(&self, pr: &Project, key: &str) -> bool {
        let s = pr.show.as_ref();
        let (own, global) = match key {
            "path" => (s.and_then(|s| s.path), self.settings.show_path),
            "description" => (
                s.and_then(|s| s.description),
                self.settings.show_description,
            ),
            "languages" => (s.and_then(|s| s.languages), self.settings.show_languages),
            _ => (s.and_then(|s| s.git), self.settings.show_git),
        };
        own.unwrap_or(global)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tile(
        &self,
        p: &mut Painter,
        pr: &Project,
        it: &Item,
        y0: i64,
        sel: bool,
        here: bool,
        clip: (i64, i64),
    ) {
        let t = &self.theme;
        let m = self.meta.get(&pr.id).unwrap_or_default();
        let avail = m.availability.as_deref();
        let color = self.tile_color(pr);
        let dim = if avail.is_some() {
            Modifier::DIM
        } else {
            Modifier::empty()
        };
        let mut border = if sel {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.surface1)
        };
        let (tl, tr, bl, br, mut hz, vt) = if sel {
            ("┏", "┓", "┗", "┛", "━", "┃")
        } else {
            ("╭", "╮", "╰", "╯", "─", "│")
        };
        if avail == Some("offline") {
            hz = if sel { "╍" } else { "╌" };
        }
        if avail == Some("deleted") {
            border = Style::default().fg(t.red);
        }
        let (x0, w, h) = (it.x, it.w, it.h);
        let inner = w.saturating_sub(4) as usize;
        let visible = |y: i64| y >= clip.0 && y < clip.1;
        if visible(y0) {
            p.put(
                x0,
                y0 as u16,
                &format!("{tl}{}{tr}", hz.repeat(w as usize - 2)),
                border,
                None,
            );
        }
        let yb = y0 + h as i64 - 1;
        if visible(yb) {
            p.put(
                x0,
                yb as u16,
                &format!("{bl}{}{br}", hz.repeat(w as usize - 2)),
                border,
                None,
            );
        }
        for i in 1..h as i64 - 1 {
            let y = y0 + i;
            if visible(y) {
                p.put(x0, y as u16, vt, border, None);
                p.put(x0 + w - 1, y as u16, vt, border, None);
                p.put(
                    x0 + 1,
                    y as u16,
                    "▌",
                    Style::default().fg(color).add_modifier(dim),
                    None,
                );
            }
        }
        let cx = x0 + 2;
        let mut row = y0 + 1;
        let mut line = |p: &mut Painter, f: &mut dyn FnMut(&mut Painter, u16)| {
            if visible(row) && row < yb {
                f(p, row as u16);
            }
            row += 1;
        };
        // Title: badge, name, marks on the right.
        let pres = self.presence.get(&pr.id).cloned().unwrap_or_default();
        line(p, &mut |p, y| {
            let mut marks: Vec<(String, Style)> = Vec::new();
            match avail {
                Some("offline") => {
                    marks.push(("⊘ offline".into(), Style::default().fg(t.overlay0)))
                }
                Some(_) => marks.push(("✗ нет папки".into(), Style::default().fg(t.red))),
                None => {
                    if pres.working > 0 {
                        marks.push((format!("●{}", pres.working), Style::default().fg(t.yellow)));
                    }
                    if pres.blocked > 0 {
                        marks.push((format!("●{}", pres.blocked), Style::default().fg(t.red)));
                    }
                    if pres.done > 0 {
                        marks.push((format!("●{}", pres.done), Style::default().fg(t.teal)));
                    }
                    if here {
                        marks.push(("● здесь".into(), Style::default().fg(t.teal)));
                    }
                    if !pres.workspaces.is_empty() {
                        let nums: Vec<String> =
                            pres.workspaces.iter().map(|(n, _)| n.to_string()).collect();
                        marks.push((format!("◆ {}", nums.join(",")), Style::default().fg(t.blue)));
                    }
                }
            }
            let mw: usize = marks.iter().map(|(s, _)| width(s) + 1).sum();
            let mut x = cx;
            if let Some(e) = pr.emoji() {
                x = p.put(
                    x,
                    y,
                    &format!("{} ", text::clamp_graphemes(e, 3)),
                    Style::default(),
                    None,
                );
            } else {
                let ini = format!(" {} ", text::initials(&pr.name));
                x = p.put(
                    x,
                    y,
                    &ini,
                    Style::default()
                        .fg(t.panel_bg)
                        .bg(color)
                        .add_modifier(Modifier::BOLD | dim),
                    None,
                );
                x += 1;
            }
            let room = (inner + cx as usize).saturating_sub(x as usize + mw);
            p.put(
                x,
                y,
                &pr.name,
                Style::default()
                    .fg(t.text)
                    .add_modifier(Modifier::BOLD | dim),
                Some(room),
            );
            let mut mx = (x0 + w - 1).saturating_sub(mw as u16);
            for (s, stl) in &marks {
                mx = p.put(mx, y, s, *stl, None) + 1;
            }
        });
        let compact = self.settings.compact();
        if compact || self.settings.show_path {
            line(p, &mut |p, y| {
                if self.shown(pr, "path") {
                    p.put(
                        cx,
                        y,
                        &trunc_left(&short_path(&pr.path), inner),
                        Style::default().fg(t.overlay0),
                        None,
                    );
                }
            });
        }
        if !compact && self.settings.show_description {
            line(p, &mut |p, y| {
                if self.shown(pr, "description") {
                    let d = pr.description.as_deref().unwrap_or("");
                    p.put(
                        cx,
                        y,
                        d,
                        Style::default().fg(t.subtext0).add_modifier(dim),
                        Some(inner),
                    );
                }
            });
        }
        if !compact && self.settings.show_languages {
            let langs = if pr.manual_languages().is_empty() {
                m.languages.clone()
            } else {
                langs::from_names(pr.manual_languages())
            };
            let show = self.shown(pr, "languages");
            line(p, &mut |p, y| {
                if show && avail.is_none() {
                    draw_langbar(p, cx, y, inner, &langs, t);
                }
            });
            line(p, &mut |p, y| {
                if show {
                    draw_legend(p, cx, y, inner, &langs, m.languages_scanned, t);
                }
            });
        }
        if self.settings.show_git {
            let show = self.shown(pr, "git");
            let busy = self.meta.is_busy(&pr.id) || self.git_busy.contains(&pr.id);
            line(p, &mut |p, y| {
                if show && avail.is_none() {
                    draw_git1(p, cx, y, inner, &m, busy, t);
                }
            });
            if !compact {
                line(p, &mut |p, y| {
                    if show && avail.is_none() {
                        draw_git2(p, cx, y, inner, &m, t);
                    }
                });
            }
        }
    }

    // ------------------------------------------------------------------ input

    pub fn on_key(&mut self, k: KeyEvent) {
        if let Some(modal) = self.modal.take() {
            self.on_modal_key(modal, k);
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        if self.searching {
            match k.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                }
                KeyCode::Enter => {
                    self.searching = false;
                    self.select_first_tile();
                }
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Down | KeyCode::Up => {
                    self.searching = false;
                    self.nav((0, if k.code == KeyCode::Down { 1 } else { -1 }));
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.push(c);
                    self.sel = 0;
                }
                _ => {}
            }
            return;
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        self.status = None;
        let p = self.current_project();
        match k.code {
            KeyCode::Left | KeyCode::Char('h') => self.nav((-1, 0)),
            KeyCode::Right | KeyCode::Char('l') => self.nav((1, 0)),
            KeyCode::Up | KeyCode::Char('k') => self.nav((0, -1)),
            KeyCode::Down | KeyCode::Char('j') => self.nav((0, 1)),
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.query.is_empty() {
                    self.quit = true;
                } else {
                    self.query.clear();
                }
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Enter => self.activate(),
            KeyCode::Char(' ') => {
                if let Some(cid) = self.current_collection() {
                    self.toggle_collapse(&cid);
                }
            }
            KeyCode::Char('o') => {
                if let Some(p) = p {
                    self.open(&p, OpenMode::New)
                }
            }
            KeyCode::Char('t') => {
                if let Some(p) = p {
                    self.open(&p, OpenMode::Tab)
                }
            }
            KeyCode::Char('w') => {
                if let Some(p) = p {
                    self.worktree_form(&p)
                }
            }
            KeyCode::Char('a') => match self.env.ctx.here() {
                Some(h) => self.add_path(&h),
                None => self.msg("не знаю текущую папку — используй A", true),
            },
            KeyCode::Char('A') => {
                let start = self
                    .env
                    .ctx
                    .here()
                    .map(|h| short_path(&h))
                    .unwrap_or_else(|| "~/".into());
                self.modal = Some(Modal::Form(
                    Box::new(Form::new(
                        "Добавить папку",
                        vec![Field::text(
                            "Путь к папке",
                            &start,
                            "~ раскрывается; путь нормализуется до корня git",
                        )],
                    )),
                    Purpose::AddPath,
                ));
            }
            KeyCode::Char('I') => self.import(),
            KeyCode::Char('n') => self.collection_form(None),
            KeyCode::Char('e') => {
                if let Some(p) = p {
                    self.project_form(Some(p.id.clone()), p);
                } else if let Some(Some(cid)) = self.current_header() {
                    self.collection_form(Some(cid));
                }
            }
            KeyCode::Char('d') => {
                if let Some(p) = p {
                    self.modal = Some(Modal::Confirm(
                        format!("Убрать «{}» из списка? Папка не трогается.", p.name),
                        ConfirmAction::RemoveProject(p.id),
                    ));
                } else if let Some(Some(cid)) = self.current_header() {
                    let name = self
                        .cat()
                        .collection(Some(&cid))
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    self.modal = Some(Modal::Confirm(
                        format!("Удалить коллекцию «{name}»? Проекты станут без коллекции."),
                        ConfirmAction::RemoveCollection(cid),
                    ));
                }
            }
            KeyCode::Char('m') => {
                if let Some(p) = p {
                    self.move_form(&p)
                }
            }
            KeyCode::Char('J') => self.shift(1),
            KeyCode::Char('K') => self.shift(-1),
            KeyCode::Char('f') => self.git_op(p, RemoteOp::Fetch),
            KeyCode::Char('p') => self.git_op(p, RemoteOp::Pull),
            KeyCode::Char('P') => self.git_op(p, RemoteOp::Push),
            KeyCode::Char('r') => {
                if let Some(p) = p {
                    self.refresh_one(&p, true, None);
                    self.msg(&format!("обновляю {}…", p.name), false);
                }
            }
            KeyCode::Char('R') => {
                self.refresh_all(true, None);
                self.msg("обновляю все проекты…", false);
            }
            KeyCode::Char('O') => {
                if let Some(p) = p {
                    reveal(&p.path);
                }
            }
            KeyCode::Char('c') => {
                self.settings.tile_size = if self.settings.compact() {
                    "normal".into()
                } else {
                    "compact".into()
                };
                let _ = self
                    .settings
                    .save_key(&self.env.paths.settings(), "tile_size");
                self.need_clear = true;
            }
            KeyCode::Char('E') => self.want_editor = true,
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.modal.is_some() || self.help {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollDown => self.nav((0, 1)),
            MouseEventKind::ScrollUp => self.nav((0, -1)),
            MouseEventKind::Down(MouseButton::Left) => {
                if m.row < self.body_top {
                    return;
                }
                let cy = m.row as usize - self.body_top as usize + self.scroll;
                let hit = self.items.iter().position(|it| {
                    cy >= it.y && cy < it.y + it.h && m.column >= it.x && m.column < it.x + it.w
                });
                if let Some(i) = hit {
                    let double = self
                        .last_click
                        .is_some_and(|(t, j)| j == i && t.elapsed() < Duration::from_millis(400));
                    self.sel = i;
                    self.last_click = Some((Instant::now(), i));
                    if double {
                        self.last_click = None;
                        self.activate();
                    }
                }
            }
            _ => {}
        }
    }

    fn select_first_tile(&mut self) {
        if let Some(i) = self
            .items
            .iter()
            .position(|it| matches!(it.kind, ItemKind::Tile(_)))
        {
            self.sel = i;
        }
    }

    fn activate(&mut self) {
        if let Some(p) = self.current_project() {
            let mode = if self.settings.open_mode == "tab" {
                OpenMode::Tab
            } else {
                OpenMode::Workspace
            };
            self.open(&p, mode);
        } else if let Some(Some(cid)) = self.current_header() {
            self.toggle_collapse(&cid);
        }
    }

    fn toggle_collapse(&mut self, cid: &str) {
        if let Some(c) = self.store.data.collection_mut(cid) {
            c.collapsed = Some(!c.is_collapsed());
        }
        self.pending = Some(Focus::Header(cid.to_string()));
        self.save();
    }

    fn shift(&mut self, delta: i32) {
        let Some(it) = self.current().cloned() else {
            return;
        };
        match it.kind {
            ItemKind::Header {
                collection: Some(cid),
                ..
            } => {
                let arr = &mut self.store.data.collections;
                let Some(i) = arr.iter().position(|c| c.id == cid) else {
                    return;
                };
                let j = i as i32 + delta;
                if j >= 0 && (j as usize) < arr.len() {
                    arr.swap(i, j as usize);
                    self.pending = Some(Focus::Header(cid));
                    self.save();
                }
            }
            ItemKind::Tile(id) => {
                let cat = &self.store.data;
                let Some(p) = cat.project(&id) else { return };
                let group = if cat.is_grouped(p) {
                    p.collection_id.clone()
                } else {
                    None
                };
                let same: Vec<usize> = cat
                    .projects
                    .iter()
                    .enumerate()
                    .filter(|(_, q)| {
                        (if cat.is_grouped(q) {
                            q.collection_id.clone()
                        } else {
                            None
                        }) == group
                    })
                    .map(|(i, _)| i)
                    .collect();
                let Some(i) = same.iter().position(|&k| cat.projects[k].id == id) else {
                    return;
                };
                let j = i as i32 + delta;
                if j < 0 || j as usize >= same.len() {
                    return;
                }
                self.store.data.projects.swap(same[i], same[j as usize]);
                self.pending = Some(Focus::Tile(id));
                self.save();
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ herdr actions

    /// Opens a project: focus its workspace, create one, or add a tab. Exits on success.
    pub fn open(&mut self, p: &Project, mode: OpenMode) {
        if !std::path::Path::new(&p.path).is_dir() {
            self.msg(&format!("папка недоступна: {}", p.path), true);
            return;
        }
        let Some(client) = self.env.client.clone() else {
            self.msg(
                "нет сокета herdr (HERDR_SOCKET_PATH) — панель запущена вне herdr",
                true,
            );
            return;
        };
        let label = p.name.clone();
        let res = match mode {
            OpenMode::Tab => {
                let mut params = json!({ "cwd": p.path, "label": label, "focus": true });
                if let Some(ws) = &self.env.ctx.workspace_id {
                    params["workspace_id"] = json!(ws);
                }
                client.request("tab.create", params).map(|_| None)
            }
            OpenMode::Workspace | OpenMode::New => {
                let existing = (mode == OpenMode::Workspace)
                    .then(|| {
                        self.presence
                            .get(&p.id)
                            .and_then(|pr| pr.workspaces.first().cloned())
                    })
                    .flatten();
                let focused = existing.and_then(|(_, ws)| {
                    client
                        .request("workspace.focus", json!({ "workspace_id": ws }))
                        .ok()
                        .map(|_| ws)
                });
                match focused {
                    Some(ws) => Ok(Some(ws)),
                    None => client
                        .request(
                            "workspace.create",
                            json!({ "cwd": p.path, "label": label, "focus": true }),
                        )
                        .map(|r| workspace_id_of(&r)),
                }
            }
        };
        match res {
            Ok(ws) => {
                if let Some(ws) = ws {
                    let m = self.meta.get(&p.id);
                    let want = presence::tokens_for(p, m.as_ref());
                    let _ = presence::report(&client, &ws, &want);
                }
                self.quit = true;
            }
            Err(e) => self.msg(&format!("не удалось открыть: {e}"), true),
        }
    }

    fn git_op(&mut self, p: Option<Project>, op: RemoteOp) {
        let Some(p) = p else { return };
        if !git::is_repo(&p.path) {
            self.msg("не git-репозиторий", true);
            return;
        }
        if !self.git_busy.insert(p.id.clone()) {
            return;
        }
        self.msg(&format!("git {}: {}…", op.name(), p.name), false);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let o = git::remote(&p.path, op);
            let _ = tx.send(Msg::GitDone {
                id: p.id,
                op,
                ok: o.ok(),
                message: o.message(),
            });
        });
    }

    // ------------------------------------------------------------------ catalog editing

    fn collection_names(&self) -> (Vec<String>, Vec<Option<String>>) {
        let mut names = vec![NO_COLLECTION.to_string()];
        let mut ids = vec![None];
        for c in &self.cat().collections {
            names.push(c.name.clone());
            ids.push(Some(c.id.clone()));
        }
        (names, ids)
    }

    fn project_form(&mut self, id: Option<String>, p: Project) {
        let (names, ids) = self.collection_names();
        let cur = ids
            .iter()
            .position(|i| i.is_some() && *i == p.collection_id)
            .unwrap_or(0);
        let fields = vec![
            Field::text("Имя", &p.name, ""),
            Field::text(
                "Путь",
                &short_path(&p.path),
                "папка проекта; ~ раскрывается",
            ),
            Field::text("Описание", p.description.as_deref().unwrap_or(""), ""),
            Field::text(
                "Цвет",
                p.color.as_deref().unwrap_or(""),
                "#hex или red orange yellow green teal blue purple gray; пусто — цвет коллекции",
            ),
            Field::text(
                "Логотип",
                p.emoji().unwrap_or(""),
                "до 3 emoji; пусто — инициалы",
            ),
            Field::text(
                "Языки",
                &p.manual_languages().join(", "),
                "через запятую; пусто — определять автоматически",
            ),
            Field::choice(
                "Коллекция",
                names,
                cur,
                "←→ выбрать; новая коллекция — клавиша n",
            ),
        ];
        let title = if id.is_some() {
            format!("Проект: {}", p.name)
        } else {
            "Новый проект".to_string()
        };
        self.modal = Some(Modal::Form(
            Box::new(Form::new(&title, fields)),
            Purpose::EditProject {
                id,
                draft: Box::new(p),
            },
        ));
    }

    fn collection_form(&mut self, id: Option<String>) {
        let c = id
            .as_deref()
            .and_then(|i| self.cat().collection(Some(i)))
            .cloned()
            .unwrap_or_default();
        let fields = vec![
            Field::text("Имя коллекции", &c.name, ""),
            Field::text("Описание", c.description.as_deref().unwrap_or(""), ""),
            Field::text(
                "Цвет",
                c.color.as_deref().unwrap_or(""),
                "#hex или red orange yellow green teal blue purple gray",
            ),
        ];
        let title = if id.is_some() {
            "Коллекция"
        } else {
            "Новая коллекция"
        };
        self.modal = Some(Modal::Form(
            Box::new(Form::new(title, fields)),
            Purpose::EditCollection { id },
        ));
    }

    fn move_form(&mut self, p: &Project) {
        let (names, ids) = self.collection_names();
        let cur = ids
            .iter()
            .position(|i| i.is_some() && *i == p.collection_id)
            .unwrap_or(0);
        self.modal = Some(Modal::Form(
            Box::new(Form::new(
                &format!("Переместить «{}»", p.name),
                vec![Field::choice("В коллекцию", names, cur, "←→ выбрать")],
            )),
            Purpose::Move { id: p.id.clone() },
        ));
    }

    fn worktree_form(&mut self, p: &Project) {
        if !git::is_repo(&p.path) {
            self.msg("не git-репозиторий", true);
            return;
        }
        self.modal = Some(Modal::Form(
            Box::new(Form::new(
                &format!("Worktree: {}", p.name),
                vec![Field::text(
                    "Ветка",
                    "",
                    "существующая ветка или новая (от HEAD)",
                )],
            )),
            Purpose::Worktree { id: p.id.clone() },
        ));
    }

    fn on_modal_key(&mut self, modal: Modal, k: KeyEvent) {
        match modal {
            Modal::Confirm(q, action) => match k.code {
                KeyCode::Char('y')
                | KeyCode::Char('Y')
                | KeyCode::Char('д')
                | KeyCode::Char('Д') => self.confirm(action),
                KeyCode::Char('n')
                | KeyCode::Char('N')
                | KeyCode::Char('н')
                | KeyCode::Esc
                | KeyCode::Enter => self.msg("отменено", false),
                _ => self.modal = Some(Modal::Confirm(q, action)),
            },
            Modal::Form(mut form, purpose) => match form.handle(k) {
                Outcome::Continue => self.modal = Some(Modal::Form(form, purpose)),
                Outcome::Cancel => self.msg("отменено", false),
                Outcome::Submit => {
                    if let Err(e) = self.submit(&form, &purpose) {
                        form.error = Some(e);
                        self.modal = Some(Modal::Form(form, purpose));
                    }
                }
            },
        }
    }

    fn confirm(&mut self, a: ConfirmAction) {
        match a {
            ConfirmAction::RemoveProject(id) => {
                let name = self
                    .cat()
                    .project(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.store.data.projects.retain(|p| p.id != id);
                self.meta.drop_project(&id);
                self.save();
                self.msg(&format!("убран: {name}"), false);
            }
            ConfirmAction::RemoveCollection(cid) => {
                self.store.data.collections.retain(|c| c.id != cid);
                for p in &mut self.store.data.projects {
                    if p.collection_id.as_deref() == Some(cid.as_str()) {
                        p.collection_id = None;
                    }
                }
                self.save();
                self.msg("коллекция удалена", false);
            }
        }
    }

    fn submit(&mut self, f: &Form, purpose: &Purpose) -> Result<(), String> {
        match purpose {
            Purpose::AddPath => {
                let v = f.value(0);
                if v.trim().is_empty() {
                    return Err("укажи путь".into());
                }
                self.add_path(&v);
                Ok(())
            }
            Purpose::Move { id } => {
                let (_, ids) = self.collection_names();
                let target = ids.get(f.choice(0)).cloned().flatten();
                if let Some(c) = target
                    .as_deref()
                    .and_then(|c| self.store.data.collection_mut(c))
                {
                    c.collapsed = Some(false);
                }
                if let Some(p) = self.store.data.project_mut(id) {
                    p.collection_id = target;
                }
                self.pending = Some(Focus::Tile(id.clone()));
                self.save();
                Ok(())
            }
            Purpose::EditCollection { id } => {
                let name = f.value(0).trim().to_string();
                if name.is_empty() {
                    return Err("имя коллекции не может быть пустым".into());
                }
                let color = f.value(2);
                let color = if color.trim().is_empty() {
                    None
                } else {
                    Some(
                        self.theme
                            .parse_color(&color)
                            .ok_or_else(|| format!("не понял цвет «{color}»"))?,
                    )
                };
                let desc = Some(f.value(1).trim().to_string()).filter(|s| !s.is_empty());
                if let Some(other) = self.cat().collection_by_name(&name)
                    && Some(&other.id) != id.as_ref()
                {
                    return Err(format!("коллекция «{}» уже есть", other.name));
                }
                let cid = match id {
                    Some(id) => id.clone(),
                    None => {
                        let picked = self.theme.pick_color(&name);
                        self.store
                            .data
                            .ensure_collection(&name, |_| picked.clone())
                            .unwrap_or_default()
                    }
                };
                if let Some(c) = self.store.data.collection_mut(&cid) {
                    c.name = name.clone();
                    c.description = desc;
                    if color.is_some() || id.is_some() {
                        c.color = color.or_else(|| c.color.clone());
                    }
                }
                self.pending = Some(Focus::Header(cid));
                self.save();
                self.msg(&format!("коллекция сохранена: {name}"), false);
                Ok(())
            }
            Purpose::Worktree { id } => {
                let branch = f.value(0).trim().to_string();
                if branch.is_empty() {
                    return Err("укажи ветку".into());
                }
                let p = self.cat().project(id).cloned().ok_or("проект пропал")?;
                let client = self.env.client.clone().ok_or("нет сокета herdr")?;
                let r = client
                    .request(
                        "worktree.create",
                        json!({ "cwd": p.path, "branch": branch, "focus": true }),
                    )
                    .map_err(|e| format!("worktree: {e}"))?;
                if let Some(ws) = workspace_id_of(&r) {
                    let _ = presence::report(
                        &client,
                        &ws,
                        &presence::tokens_for(&p, self.meta.get(&p.id).as_ref()),
                    );
                }
                self.quit = true;
                Ok(())
            }
            Purpose::EditProject { id, draft } => {
                let mut p = (**draft).clone();
                if let Some(id) = id
                    && let Some(cur) = self.cat().project(id)
                {
                    p = cur.clone();
                }
                let base = self.env.ctx.here().unwrap_or_else(text::home);
                let raw_path = f.value(1);
                let path = if raw_path.trim().is_empty() {
                    p.path.clone()
                } else {
                    text::expand_path(&raw_path, &base)
                        .to_string_lossy()
                        .to_string()
                };
                if (norm(&path) != norm(&p.path) || id.is_none())
                    && !std::path::Path::new(&path).is_dir()
                {
                    return Err(format!("нет такой папки: {path}"));
                }
                if let Some(i) = self.cat().find_path(&path)
                    && Some(&self.cat().projects[i].id) != id.as_ref()
                {
                    return Err(format!(
                        "этот путь уже в проекте «{}»",
                        self.cat().projects[i].name
                    ));
                }
                let color = f.value(3);
                let color = if color.trim().is_empty() {
                    None
                } else {
                    Some(
                        self.theme
                            .parse_color(&color)
                            .ok_or_else(|| format!("не понял цвет «{}»", color.trim()))?,
                    )
                };
                let name = f.value(0).trim().to_string();
                p.name = if name.is_empty() {
                    crate::catalog::default_name(&path)
                } else {
                    name
                };
                p.path = path;
                p.description = Some(f.value(2).trim().to_string()).filter(|s| !s.is_empty());
                p.color = color;
                let emoji = text::clamp_graphemes(f.value(4).trim(), 3);
                p.logo = Some(if emoji.is_empty() {
                    Logo::auto()
                } else {
                    Logo {
                        kind: "emoji".into(),
                        value: Some(emoji),
                    }
                });
                let langs: Vec<String> = f
                    .value(5)
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                p.languages = if langs.is_empty() { None } else { Some(langs) };
                let (_, ids) = self.collection_names();
                p.collection_id = ids.get(f.choice(6)).cloned().flatten();
                match id {
                    Some(id) => {
                        if let Some(slot) = self.store.data.project_mut(id) {
                            *slot = p.clone();
                        }
                    }
                    None => self.store.data.projects.push(p.clone()),
                }
                self.pending = Some(Focus::Tile(p.id.clone()));
                self.save();
                self.refresh_one(&p, true, Some(false));
                self.msg(&format!("сохранено: {}", p.name), false);
                Ok(())
            }
        }
    }

    /// Adds a folder (normalized to its git root); an existing project opens for editing.
    pub fn add_path(&mut self, raw: &str) {
        let base = self.env.ctx.here().unwrap_or_else(text::home);
        let path = text::expand_path(raw, &base).to_string_lossy().to_string();
        if !std::path::Path::new(&path).is_dir() {
            self.msg(&format!("нет такой папки: {path}"), true);
            return;
        }
        let path = git::toplevel(&path);
        if let Some(i) = self.cat().find_path(&path) {
            let p = self.cat().projects[i].clone();
            self.pending = Some(Focus::Tile(p.id.clone()));
            self.msg(&format!("уже есть: {} — редактирование", p.name), false);
            let id = p.id.clone();
            self.project_form(Some(id), p);
            return;
        }
        let mut p = Project::new(&path);
        p.collection_id = self.current_collection();
        self.project_form(None, p);
    }

    /// `I`: the VS Code Project Tiles catalog + folders of open herdr workspaces.
    pub fn import(&mut self) {
        let vscode = std::fs::read_to_string(Paths::vscode_catalog())
            .ok()
            .and_then(|s| CatalogData::parse(&s).ok());
        let cwds: Vec<String> = self
            .snap
            .as_ref()
            .map(|s| {
                s.panes
                    .iter()
                    .filter(|p| Some(&p.pane_id) != self.env.own_pane.as_ref())
                    .filter_map(|p| p.cwd.clone())
                    .collect()
            })
            .unwrap_or_default();
        let theme = self.theme.clone();
        let added = import_into(
            &mut self.store.data,
            vscode.as_ref(),
            &cwds,
            &text::home(),
            &|n| theme.pick_color(n),
            &git::toplevel,
        );
        self.save();
        self.refresh_all(false, None);
        let note = if vscode.is_some() {
            ""
        } else {
            " (каталог Project Tiles из VS Code не найден)"
        };
        self.msg(&format!("импортировано: {added}{note}"), false);
    }

    /// Restores the screen after `$EDITOR` and reloads the catalog.
    pub fn after_editor(&mut self) {
        self.need_clear = true;
        self.reload_catalog();
    }
}

/// "1 проект", "3 проекта", "5 проектов".
pub fn projects_label(n: usize) -> String {
    let word = match (n % 10, n % 100) {
        (1, x) if x != 11 => "проект",
        (2..=4, x) if !(12..=14).contains(&x) => "проекта",
        _ => "проектов",
    };
    format!("{n} {word}")
}

fn workspace_id_of(r: &Value) -> Option<String> {
    r.get("workspace")
        .and_then(|w| w.get("workspace_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Merges projects from the VS Code catalog (collections by name, emoji logos, no `.code-workspace`)
/// and from open herdr panes (git toplevel, not `$HOME` or `/`, no duplicates). Returns the count added.
pub fn import_into(
    cat: &mut CatalogData,
    vscode: Option<&CatalogData>,
    cwds: &[String],
    home: &str,
    pick: &dyn Fn(&str) -> String,
    toplevel: &dyn Fn(&str) -> String,
) -> usize {
    let mut added = 0;
    if let Some(src) = vscode {
        let mut idmap: HashMap<String, String> = HashMap::new();
        for c in &src.collections {
            if let Some(mine) =
                cat.ensure_collection(&c.name, |n| c.color.clone().unwrap_or_else(|| pick(n)))
            {
                if let Some(m) = cat.collection_mut(&mine) {
                    if m.description.is_none() {
                        m.description = c.description.clone();
                    }
                    if m.color.is_none() {
                        m.color = c.color.clone();
                    }
                }
                idmap.insert(c.id.clone(), mine);
            }
        }
        for sp in &src.projects {
            if sp.path.to_lowercase().ends_with(".code-workspace")
                || cat.find_path(&sp.path).is_some()
            {
                continue;
            }
            let mut p = Project::new(&sp.path);
            p.name = sp.name.clone();
            p.description = sp.description.clone();
            p.color = sp.color.clone();
            p.languages = sp.languages.clone();
            p.show = sp.show.clone();
            p.logo = Some(match sp.emoji() {
                Some(e) => Logo {
                    kind: "emoji".into(),
                    value: Some(e.to_string()),
                },
                None => Logo::auto(),
            });
            p.collection_id = sp
                .collection_id
                .as_ref()
                .and_then(|c| idmap.get(c).cloned());
            cat.projects.push(p);
            added += 1;
        }
    }
    let mut seen = HashSet::new();
    for c in cwds {
        if !std::path::Path::new(c).is_dir() {
            continue;
        }
        let root = toplevel(c);
        let k = norm(&root);
        if k == norm(home) || k == "/" || !seen.insert(k) || cat.find_path(&root).is_some() {
            continue;
        }
        cat.projects.push(Project::new(&root));
        added += 1;
    }
    added
}

fn reveal(path: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ---------------------------------------------------------------------- painting helpers

pub struct Painter<'a> {
    pub buf: &'a mut Buffer,
    pub area: Rect,
}

impl Painter<'_> {
    /// Writes `s` clipped to the area (and to `maxw` cells); returns the x after it.
    pub fn put(&mut self, x: u16, y: u16, s: &str, st: Style, maxw: Option<usize>) -> u16 {
        if y >= self.area.bottom() || x >= self.area.right() || s.is_empty() {
            return x;
        }
        let room = (self.area.right() - x) as usize;
        let w = maxw.map_or(room, |m| m.min(room));
        if w == 0 {
            return x;
        }
        let s = trunc(s, w);
        let (nx, _) = self.buf.set_stringn(x, y, &s, w, st);
        nx
    }
}

fn draw_langbar(p: &mut Painter, x: u16, y: u16, w: usize, langs: &[(String, f64)], t: &Theme) {
    if langs.is_empty() {
        p.put(x, y, &"·".repeat(w), Style::default().fg(t.surface1), None);
        return;
    }
    let mut used = 0usize;
    for (i, (name, pct)) in langs.iter().enumerate() {
        let n = if i == langs.len() - 1 {
            w - used
        } else {
            ((w as f64 * pct / 100.0).round() as usize).max(1)
        };
        let n = n.min(w - used);
        if n == 0 {
            break;
        }
        p.put(
            x + used as u16,
            y,
            &"▀".repeat(n),
            Style::default().fg(t.color(langs::color_of(name))),
            None,
        );
        used += n;
    }
}

fn draw_legend(
    p: &mut Painter,
    x: u16,
    y: u16,
    w: usize,
    langs: &[(String, f64)],
    scanned: bool,
    t: &Theme,
) {
    if langs.is_empty() {
        if scanned {
            p.put(
                x,
                y,
                "языки не найдены",
                Style::default().fg(t.overlay0),
                None,
            );
        }
        return;
    }
    let mut cx = x;
    for (name, pct) in langs.iter().take(4) {
        let label = format!("{name} {pct:.0}%");
        if (cx - x) as usize + width(&label) + 2 > w {
            break;
        }
        cx = p.put(
            cx,
            y,
            "● ",
            Style::default().fg(t.color(langs::color_of(name))),
            None,
        );
        cx = p.put(cx, y, &label, Style::default().fg(t.subtext0), None) + 2;
    }
}

fn draw_git1(p: &mut Painter, x: u16, y: u16, w: usize, m: &Meta, busy: bool, t: &Theme) {
    if !m.git_checked {
        if busy {
            p.put(x, y, "…", Style::default().fg(t.overlay0), None);
        }
        return;
    }
    let Some(g) = &m.git else {
        p.put(
            x,
            y,
            "не git-репозиторий",
            Style::default().fg(t.overlay0),
            None,
        );
        return;
    };
    let mut cx = p.put(
        x,
        y,
        &format!(" {} ", g.branch),
        Style::default().fg(t.text).bg(t.surface1),
        Some(w.saturating_sub(14).max(6)),
    ) + 1;
    let gray = Style::default().fg(t.overlay0);
    if g.upstream.is_some() {
        let stale = !g.fetched;
        let pick = |c: Color| if stale { gray } else { Style::default().fg(c) };
        if g.ahead == 0 && g.behind == 0 {
            cx = p.put(cx, y, "✓", pick(t.green), None) + 1;
        }
        if g.behind > 0 {
            cx = p.put(cx, y, &format!("↓{}", g.behind), pick(t.peach), None) + 1;
        }
        if g.ahead > 0 {
            cx = p.put(cx, y, &format!("↑{}", g.ahead), pick(t.blue), None) + 1;
        }
        if stale {
            cx = p.put(cx, y, "stale", gray, None) + 1;
        }
    } else {
        cx = p.put(cx, y, "∅ upstream", gray, None) + 1;
    }
    if g.dirty > 0 {
        cx = p.put(
            cx,
            y,
            &format!("●{}", g.dirty),
            Style::default().fg(t.yellow),
            None,
        ) + 1;
    }
    if g.untracked > 0 {
        p.put(cx, y, &format!("+{}", g.untracked), gray, None);
    }
    if busy {
        p.put(
            x + w as u16 - 1,
            y,
            "⟳",
            Style::default().fg(t.yellow),
            None,
        );
    }
}

fn draw_git2(p: &mut Painter, x: u16, y: u16, w: usize, m: &Meta, t: &Theme) {
    let Some(c) = m.git.as_ref().and_then(|g| g.commit.as_ref()) else {
        return;
    };
    let cx = p.put(
        x,
        y,
        &format!("{} ", c.hash),
        Style::default().fg(t.blue),
        None,
    );
    let date = format!(" · {}", c.date);
    let room = w.saturating_sub((cx - x) as usize + width(&date)).max(4);
    let cx = p.put(
        cx,
        y,
        &c.subject,
        Style::default().fg(t.subtext0),
        Some(room),
    );
    p.put(
        cx,
        y,
        &date,
        Style::default().fg(t.overlay0),
        Some(w.saturating_sub((cx - x) as usize)),
    );
}

/// Blanks `r` with `style`. A wide character that starts just left of `r` (and would spill
/// into it) is blanked too, as is a dangling continuation cell right of it.
pub fn clear_area(buf: &mut Buffer, r: Rect, style: Style) {
    let full = buf.area;
    for y in r.top()..r.bottom() {
        if r.left() > full.left() && width(buf[(r.left() - 1, y)].symbol()) > 1 {
            buf[(r.left() - 1, y)].set_symbol(" ");
        }
        for x in r.left()..r.right() {
            buf[(x, y)].reset();
            buf[(x, y)].set_style(style);
        }
        if r.right() < full.right() && buf[(r.right(), y)].symbol().is_empty() {
            buf[(r.right(), y)].set_symbol(" ");
        }
    }
}

fn boxed(buf: &mut Buffer, area: Rect, bw: u16, bh: u16, t: &Theme, title: &str) -> Rect {
    let bw = bw.min(area.width);
    let bh = bh.min(area.height);
    let x0 = area.x + (area.width - bw) / 2;
    let y0 = area.y + (area.height - bh) / 2;
    let base = Style::default().fg(t.text).bg(t.panel_bg);
    clear_area(buf, Rect::new(x0, y0, bw, bh), base);
    let b = Style::default().fg(t.accent).bg(t.panel_bg);
    let hz = "─".repeat(bw.saturating_sub(2) as usize);
    buf.set_string(x0, y0, format!("╭{hz}╮"), b);
    buf.set_string(x0, y0 + bh - 1, format!("╰{hz}╯"), b);
    for y in y0 + 1..y0 + bh - 1 {
        buf.set_string(x0, y, "│", b);
        buf.set_string(x0 + bw - 1, y, "│", b);
    }
    buf.set_string(
        x0 + 2,
        y0,
        format!(" {title} "),
        b.add_modifier(Modifier::BOLD),
    );
    Rect::new(x0, y0, bw, bh)
}

fn draw_help(buf: &mut Buffer, area: Rect, t: &Theme) {
    let r = boxed(buf, area, 70, HELP.len() as u16 + 4, t, "Клавиши");
    let inner = r.width.saturating_sub(17) as usize;
    for (i, (k, d)) in HELP.iter().enumerate() {
        let y = r.y + 2 + i as u16;
        if y >= r.bottom() - 1 {
            break;
        }
        buf.set_stringn(
            r.x + 2,
            y,
            k,
            12,
            Style::default().fg(t.yellow).bg(t.panel_bg),
        );
        buf.set_stringn(
            r.x + 15,
            y,
            trunc(d, inner),
            inner,
            Style::default().fg(t.text).bg(t.panel_bg),
        );
    }
}

fn draw_confirm(buf: &mut Buffer, area: Rect, t: &Theme, q: &str) {
    let w = (width(q) as u16 + 6).clamp(30, area.width.saturating_sub(2).max(30));
    let r = boxed(buf, area, w, 5, t, "Подтверждение");
    let inner = r.width.saturating_sub(4) as usize;
    buf.set_stringn(
        r.x + 2,
        r.y + 1,
        trunc(q, inner),
        inner,
        Style::default().fg(t.text).bg(t.panel_bg),
    );
    buf.set_stringn(
        r.x + 2,
        r.y + 3,
        "y — да · n / Esc — нет",
        inner,
        Style::default().fg(t.overlay0).bg(t.panel_bg),
    );
}

#[cfg(test)]
mod plural_tests {
    #[test]
    fn plural() {
        let l = super::projects_label;
        assert_eq!(l(1), "1 проект");
        assert_eq!(l(3), "3 проекта");
        assert_eq!(l(5), "5 проектов");
        assert_eq!(l(11), "11 проектов");
        assert_eq!(l(12), "12 проектов");
        assert_eq!(l(21), "21 проект");
        assert_eq!(l(0), "0 проектов");
    }
}
