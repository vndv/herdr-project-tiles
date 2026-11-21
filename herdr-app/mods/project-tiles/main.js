// Project Tiles: a dashboard of projects as tiles (name, logo, path, description, languages,
// git), grouped into collapsible collections. Two ways to show it: a dialog
// (mod.open:project-tiles), or embedded in a layout block (props {embedded: true}), e.g. the
// command-center layout's center stack next to Terminals and Board.
//
// Runs programs (through /usr/bin/env): git in project folders (read-only and without hooks for
// the status; fetch, pull, push when asked, plus a background fetch for repos with an upstream),
// perl as a timeout guard for those network calls, sh/find/wc for folder checks and the language
// scan of folders without git, and `open` for links and Finder. A project with a `machine`
// (a herdr-core machine name) runs all of that there (exec's `on`).
//
// Live from Herdr (no polling): which spaces show a project, where you are, and the agents
// working / blocked / done in it.
//
// Files outside its folder: the catalog ~/.config/herdr/plugins/config/vndv.project-tiles/
// projects.json (Project Tiles format) and settings.json next to it, written with
// writeTextFile's stale check. The git/language cache lives in its data folder.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, ScrollHandle } from "gpui-base";
import {
  spaces, panes, focused, theme, run, exec, home, notify, props, closeOverlay, openOverlay,
  readTextFile, writeTextFile, checking, pickFolder, layout, nextCommand, appHome,
} from "herdr";
import * as M from "./model.js";

/** Concurrent per-project refreshes (git + languages). */
const WORKERS = 6;

const HELP = [
  ["←↑↓→ hjkl", "move (mouse: click selects, double-click opens)"],
  ["Enter", "open: go to the project's space, or create one"],
  ["o", "open in a new space"],
  ["t", "open as a new tab in the current space"],
  ["w / W", "new git worktree / open an existing one"],
  ["g", "gitview: review, stage and commit the project's changes"],
  ["/", "search (name, path, description, languages, branch)"],
  ["a / A", "add the current folder / a folder by path"],
  ["I", "import: open terminals + Project Tiles (VS Code)"],
  ["e / d", "edit / remove from the list (project or collection)"],
  ["m / n", "move to a collection / new collection"],
  ["J / K", "move down / up"],
  ["Space", "collapse or expand a collection"],
  ["f p P", "git fetch / pull --ff-only / push"],
  ["r / R", "refresh project / all (R also runs git fetch)"],
  ["O", "reveal in Finder"],
  ["click branch / hash", "open the branch (or repo) / commit on GitLab, GitHub, Bitbucket"],
  ["c", "compact / normal"],
  ["E", "edit projects.json in $EDITOR (new tab)"],
  ["? / Esc", "help / close"],
];

export default class ProjectTiles extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.scroll = ScrollHandle.new();
    this.home = home();
    const config = `${this.home}/.config/herdr/plugins/config/vndv.project-tiles`;
    this.paths = {
      config,
      catalog: `${config}/projects.json`,
      settings: `${config}/settings.json`,
      cache: `${appHome()}/data/project-tiles/metadata-cache.json`,
      // The cache of the former Rust backend: read once when there is no cache of our own.
      legacyCache: `${this.home}/.local/state/herdr/plugins/vndv.project-tiles/metadata-cache.json`,
    };
    this.pool = M.makePool(WORKERS);
    this.machines = {};
    this.cat = M.emptyCatalog();
    this.catText = null;
    this.catError = null;
    this.settings = { ...M.DEFAULT_SETTINGS };
    this.settingsText = null;
    this.meta = {};
    this.busy = new Set();
    // Asked for while busy (e.g. its path changed mid-refresh): refreshed again when it is free.
    this.pending = new Set();
    this.closed = false;
    this.loading = 0;
    this.at = null;
    this.query = "";
    this.search = null;
    this.status = null;
    this.help = false;
    this.modal = null;
    this.width = 1100;
    this.lastClick = null;
    // Embedded in a layout block: never take the keyboard from a terminal on start.
    this.embedded = Boolean(props()?.embedded) || !props()?.overlay;
    if (!this.embedded) {
      this.focusSelf();
      cx.timer.after(400, () => this.focusSelf());
    }
    if (checking()) return;
    cx.spawn(async (cx) => {
      try {
        await this.boot(cx);
      } catch (e) {
        this.say(`could not load: ${e.message}`, true);
        cx.notify();
      }
    });
    // "focus": sent by pane.go (focusable) or by the tab bar when it shows this tab.
    cx.spawn(async (cx) => {
      for (;;) {
        const c = await nextCommand();
        if (c === "focus") this.focusSelf();
        cx.notify();
      }
    });
    // The catalog can change on disk (hand edit, the TUI plugin, `E`).
    cx.timer.every(2000, (cx) => cx.spawn(async (cx) => {
      if (!this.booted || this.closed) return;
      try {
        const fetch = this.settings.fetch_remote;
        if (await this.reloadCatalog()) {
          // New projects, or a path / machine changed elsewhere (editor, another copy): the old
          // folder's metadata (branch, links, availability) does not apply any more.
          const stale = this.cat.projects.filter((p) => !M.metaFits(this.meta[p.id], p)).map((p) => p.id);
          stale.forEach((id) => delete this.meta[id]);
          cx.notify();
          if (stale.length) await this.refresh(cx, stale, { fetch });
        } else if (!this.loading && this.metaAt && Date.now() - this.metaAt > this.settings.metadata_cache_minutes * 60000) {
          // The dashboard stays open: refresh what went past its TTL (`due` decides what).
          await this.refresh(cx, null, { fetch });
        }
      } catch (e) {
        this.say(`refresh: ${e.message}`, true);
      }
      cx.notify();
    }));
  }

  focusSelf() {
    if (this.modal?.inputs?.length) this.modal.inputs[this.modal.focus]?.input?.focus();
    else if (this.search) this.search.focus();
    else this.handle.focus();
  }

  // ------------------------------------------------------------ data

  /** exec for a project's machine, as model.js expects it: /usr/bin/env argv, never throws. */
  runner(p) {
    const on = M.machineOf(p);
    return (argv, { cwd, timeout = M.QUICK } = {}) =>
      exec("/usr/bin/env", argv, { cwd, on, timeout }).catch((e) => ({
        code: /longer than|timed? ?out/i.test(e.message) ? 124 : 127,
        stdout: "",
        stderr: e.message,
      }));
  }

  /** Per-machine git flags (a machine without perl). */
  gitState(p) {
    return (this.machines[M.machineOf(p) ?? ""] ??= {});
  }

  async boot(cx) {
    // gitview (github.com/ChmaraX/herdr-gitview) is optional: its button shows when it is installed.
    this.hasGitview = (await readTextFile(`${appHome()}/mods/gitview-tab/mod.json`).catch(() => null)) !== null;
    await this.reloadSettings();
    await this.reloadCatalog();
    await this.loadCache();
    this.startAt();
    this.booted = true;
    cx.notify();
    // Local state first so every tile fills quickly, then the fetches.
    await this.refresh(cx, null, {});
    if (this.settings.fetch_remote) {
      const tried = await this.refresh(cx, null, { fetch: true });
      // Counts not yet checked against the remote ("no fetch"), even if the fetch TTL is fresh.
      const unfetched = this.cat.projects
        .filter((p) => this.metaOf(p)?.git?.upstream && !this.metaOf(p).git.fetched && !tried.includes(p.id))
        .map((p) => p.id);
      if (unfetched.length) await this.refresh(cx, unfetched, { force: true, fetch: true });
    }
  }

  /**
   * Refreshes what is past its TTL (everything with `force`), at most WORKERS projects at once;
   * each tile updates as its phases finish. Returns the ids that were fetched.
   */
  async refresh(cx, ids = null, { force = false, fetch = false } = {}) {
    const now = M.nowSecs();
    const list = ids ? ids.map((id) => this.project(id)).filter(Boolean) : [...this.cat.projects];
    const jobs = [];
    for (const p of list) {
      if (this.closed) break;
      if (this.busy.has(p.id)) {
        if (ids) this.pending.add(p.id);
        continue;
      }
      const d = M.due(this.meta[p.id], p, this.settings, now, { force, fetch: fetch && this.settings.fetch_remote });
      if (!d.git && !d.lang) continue;
      this.busy.add(p.id);
      jobs.push(this.pool(async () => {
        if (this.closed) {
          this.busy.delete(p.id);
          return null;
        }
        // A late answer for a project whose path or machine changed meanwhile is dropped.
        const publish = (m) => {
          const q = this.project(p.id);
          if (!this.closed && q && M.metaFits(m, q)) this.meta[p.id] = m;
          cx.notify();
        };
        try {
          await M.refreshProject(this.runner(p), p, this.meta[p.id], this.settings, d, publish, this.gitState(p));
        } catch (e) {
          this.say(`git/languages — ${p.name}: ${e.message}`, true);
        }
        this.busy.delete(p.id);
        cx.notify();
        return d.remote ? p.id : null;
      }));
    }
    if (!ids) this.metaAt = Date.now();
    if (!jobs.length) return [];
    this.loading++;
    cx.notify();
    const done = await Promise.all(jobs);
    this.loading--;
    await this.saveCache();
    cx.notify();
    this.runPending(cx);
    return done.filter(Boolean);
  }

  /** Projects asked for while they were busy: refreshed now (their metadata was dropped if it no
   *  longer fits, so `due` rescans them). */
  runPending(cx) {
    const ids = [...this.pending].filter((id) => !this.busy.has(id) && this.project(id));
    ids.forEach((id) => this.pending.delete(id));
    if (ids.length && !this.closed) cx.spawn(async (cx) => this.refresh(cx, ids, { fetch: this.settings.fetch_remote }));
  }

  /** The project's metadata, when it still belongs to its folder and machine. */
  metaOf(p) {
    const m = p ? this.meta[p.id] : null;
    return M.metaFits(m, p) ? m : null;
  }

  /** The cache, or (only when there is none yet) the old Rust backend's, which then seeds ours.
   *  A cache that cannot be read is left alone: everything is scanned again. */
  async loadCache() {
    let text;
    try {
      text = await readTextFile(this.paths.cache);
    } catch (e) {
      this.say(`metadata cache not readable, rescanning: ${e.message}`, true);
      return;
    }
    const legacy = text === null;
    if (legacy) text = await readTextFile(this.paths.legacyCache).catch(() => null);
    Object.assign(this.meta, M.fittingMeta(this.cat, M.parseCache(text)));
    if (legacy && Object.keys(this.meta).length) await this.saveCache();
  }

  /** Writes the cache, merged per phase with what another copy wrote meanwhile. The write only
   *  lands if the file is still what was read (else read, merge and try again). */
  async saveCache() {
    if (checking() || this.closed) return;
    let error = null;
    for (let i = 0; i < 3; i++) {
      let text;
      try {
        text = await readTextFile(this.paths.cache);
      } catch (e) {
        // Not "absent": writing with expected = null would be refused anyway; don't guess.
        error = e;
        break;
      }
      const merged = M.mergeCache(M.parseCache(text), this.meta, this.cat);
      for (const [id, m] of Object.entries(merged)) if (m !== this.meta[id] && !this.busy.has(id)) this.meta[id] = m;
      try {
        await writeTextFile(this.paths.cache, M.cacheText(merged), text);
        if (this.cacheError) this.cacheError = null;
        return;
      } catch (e) {
        error = e;
      }
    }
    if (this.cacheError !== error.message) this.say(`metadata cache not saved: ${error.message}`, true);
    this.cacheError = error.message;
  }

  async reloadSettings() {
    const text = await readTextFile(this.paths.settings).catch(() => null);
    this.settingsText = text;
    try {
      this.settings = { ...M.DEFAULT_SETTINGS, ...(text ? JSON.parse(text) : {}) };
    } catch {
      this.settings = { ...M.DEFAULT_SETTINGS };
    }
  }

  async saveSetting(key, value) {
    this.settings[key] = value;
    let cur = {};
    try {
      cur = this.settingsText ? JSON.parse(this.settingsText) : { ...M.DEFAULT_SETTINGS };
    } catch {}
    cur[key] = value;
    const text = JSON.stringify(cur, null, 2) + "\n";
    try {
      if (this.settingsText === null) await this.ensureConfigDir();
      await writeTextFile(this.paths.settings, text, this.settingsText);
      this.settingsText = text;
    } catch {
      await this.reloadSettings();
    }
  }

  /** True when the catalog changed. A file that doesn't parse keeps the current state. */
  async reloadCatalog() {
    let text;
    try {
      text = await readTextFile(this.paths.catalog);
    } catch (e) {
      // Not readable (too big, permissions): keep the list, don't treat it as deleted.
      if (this.catError !== e.message) this.say(`projects.json not readable, keeping the previous state: ${e.message}`, true);
      this.catError = e.message;
      return false;
    }
    if (text === this.catText) return false;
    if (text === null) {
      this.catText = null;
      if (!this.cat.projects.length) return false;
      this.cat = M.emptyCatalog();
      return true;
    }
    try {
      this.cat = M.parseCatalog(text);
      this.catText = text;
      this.catError = null;
      return true;
    } catch (e) {
      if (this.catError !== e.message) this.say(`projects.json does not parse, keeping the previous state: ${e.message}`, true);
      this.catError = e.message;
      return false;
    }
  }

  async save(cx) {
    const text = M.catalogText(this.cat);
    try {
      if (this.catText === null) await this.ensureConfigDir();
      await writeTextFile(this.paths.catalog, text, this.catText);
      this.catText = text;
      return true;
    } catch (e) {
      this.say(`projects.json changed on disk, edit not saved: ${e.message}`, true);
      await this.reloadCatalog();
      cx?.notify();
      return false;
    }
  }

  /** The catalog folder, for a first save on a fresh machine. */
  async ensureConfigDir() {
    await exec("mkdir", ["-p", this.paths.config]).catch(() => null);
  }

  say(text, err = false) {
    this.status = { text, err };
  }

  // ------------------------------------------------------------ derived state

  derived() {
    const cols = M.columnsFor(Math.max(200, this.width - 32), this.settings);
    const meta = M.fittingMeta(this.cat, this.meta);
    const { rows, items } = M.buildRows(this.cat, meta, this.query, cols);
    const all = spaces();
    // Linked worktrees (~/.herdr/worktrees/...) count for their project.
    const extra = M.worktreeRoots(meta);
    const pres = M.presence(this.cat, all, panes(), extra);
    const f = focused();
    const here = f?.cwd ? M.projectForCwd(this.cat, f.cwd, extra, M.machineOfTab(all, f.tab))?.id ?? null : null;
    return { cols, rows, items, pres, here };
  }

  startAt() {
    const { items, here } = this.derived();
    const target = here ?? this.cat.projects[0]?.id;
    const i = items.findIndex((it) => it.kind === "tile" && it.id === target);
    this.at = Math.max(0, i);
  }

  current(items) {
    return items[Math.min(this.at ?? 0, Math.max(0, items.length - 1))] ?? null;
  }

  project(id) {
    return this.cat.projects.find((p) => p.id === id) ?? null;
  }

  selectId(items, kind, id) {
    const i = items.findIndex((it) => it.kind === kind && (kind === "tile" ? it.id === id : it.collection === id));
    if (i >= 0) this.at = i;
  }

  // ------------------------------------------------------------ actions

  /** Embedded: the layout stack this block sits in, and whether it is the shown child. */
  stackInfo() {
    const find = (n) => {
      for (const c of n?.children ?? []) {
        if (c?.content?.mod === "project-tiles") return { stack: n, child: c };
        const f = find(c);
        if (f) return f;
      }
      return null;
    };
    const f = find(layout());
    if (!f || f.stack.type !== "stack") return null;
    const first = f.stack.children.find((c) => c.id !== f.child.id);
    return { stack: f.stack.id, child: f.child.id, back: first?.id ?? null, shown: f.stack.active === f.child.id };
  }

  /** Herdr's cleanup hook (pane, tab, overlay gone, or app quit): stop starting work and stop
   *  publishing. git calls already running finish on their own deadlines. */
  async close() {
    this.closed = true;
  }

  /** The dialog closes; the embedded tab goes back to the stack's other child (Terminals). */
  dismiss() {
    if (!this.embedded) return closeOverlay(props()?.overlay);
    const s = this.stackInfo();
    if (s?.back) run("layout.select", { stack: s.stack, child: s.back });
  }

  open(p, mode, pres) {
    if (this.metaOf(p)?.availability) return this.say(`folder unavailable: ${p.path}`, true);
    const machine = M.machineOf(p);
    // A tab opens in the current space, so only on that space's machine.
    if (mode === "tab" && M.machineOfTab(spaces(), focused()?.tab) !== machine) mode = "space";
    this.dismiss();
    if (mode === "tab") {
      run("tab.new", { folder: p.path, name: p.name });
      return;
    }
    const open = pres[p.id]?.spaces ?? [];
    if (mode === "space" && open.length) {
      // The shown one if it is the project's, else the first.
      const s = open.find((x) => x.active) ?? open[0];
      run("space.select", { id: s.id });
      return;
    }
    run("space.new", { folder: p.path, name: p.name, ...(machine ? { machine } : {}) });
  }

  openUrl(cx, url) {
    if (!url) return;
    cx.spawn(async (cx) => {
      const r = await exec("open", [url]).catch((e) => ({ code: 1, stderr: e.message }));
      if (r.code !== 0) {
        this.say(`could not open ${url}: ${(r.stderr ?? "").trim() || `exit ${r.code}`}`, true);
        cx.notify();
      }
    });
  }

  /** gitview for this project: show its space (the open one, else a new one), then gitview's own
   *  launcher (cmd-g), which opens the gitview tab for the shown space's repo. */
  gitview(p, pres) {
    if (!this.hasGitview) return this.say("gitview is not installed (github.com/ChmaraX/herdr-gitview)", true);
    if (!this.metaOf(p)?.git) return this.say("not a git repository", true);
    if (this.metaOf(p)?.availability) return this.say(`folder unavailable: ${p.path}`, true);
    this.dismiss();
    const open = pres[p.id]?.spaces ?? [];
    const s = open.find((x) => x.active) ?? open[0];
    if (s) run("space.select", { id: s.id });
    else run("space.new", { folder: p.path, name: p.name, ...(M.machineOf(p) ? { machine: M.machineOf(p) } : {}) });
    run("mod.open", { name: "gitview-tab" });
  }

  /** Herdr's worktree dialog for this repository: "new" makes a checkout and opens it as a space
   *  nested under the repo; "open" lists the existing checkouts. */
  worktree(p, action = "new") {
    if (!this.metaOf(p)?.git) return this.say("not a git repository", true);
    this.dismiss();
    const size = action === "open" ? { width: 560, height: 480 } : { width: 460, height: 280 };
    openOverlay({ mod: "worktree", kind: "dialog", ...size, props: { folder: p.path, machine: M.machineOf(p), action } });
  }

  gitOp(cx, p, op) {
    if (!p || this.busy.has(p.id)) return;
    this.busy.add(p.id);
    this.say(`git ${op}: ${p.name}…`);
    cx.spawn(async (cx) => {
      try {
        const runp = this.runner(p);
        const state = this.gitState(p);
        const avail = await M.availability(runp, p.path);
        if (avail) throw new Error(`folder ${avail}: ${p.path}`);
        if (!(await M.isRepo(runp, p.path))) throw new Error("not a git repository");
        const o = await M.remoteOp(runp, p.path, op, state);
        const m = await M.afterRemoteOp(runp, p, this.meta[p.id], o.code === 0, state);
        const q = this.project(p.id);
        if (!this.closed && q && M.metaFits(m, q)) this.meta[p.id] = m;
        await this.saveCache();
        if (o.code === 0) this.say(`git ${op}: done — ${p.name}`);
        else {
          const msg = M.gitMessage(o);
          this.say(`git ${op} failed — ${p.name}: ${msg}`, true);
          notify({ title: `git ${op} failed — ${p.name}`, body: msg, level: "error" });
        }
      } catch (e) {
        this.say(`git ${op}: ${e.message}`, true);
        notify({ title: `git ${op} failed — ${p.name}`, body: e.message, level: "error" });
      }
      this.busy.delete(p.id);
      cx.notify();
      this.runPending(cx);
    });
  }

  toggleCollapse(cx, cid) {
    const c = this.cat.collections.find((c) => c.id === cid);
    if (!c) return;
    c.collapsed = !c.collapsed;
    cx.spawn(async (cx) => {
      await this.save(cx);
      const { items } = this.derived();
      this.selectId(items, "header", cid);
      cx.notify();
    });
  }

  shift(cx, items, delta) {
    const it = this.current(items);
    if (!it) return;
    if (it.kind === "header" && it.collection) {
      const arr = this.cat.collections;
      const i = arr.findIndex((c) => c.id === it.collection);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= arr.length) return;
      [arr[i], arr[j]] = [arr[j], arr[i]];
    } else if (it.kind === "tile") {
      const p = this.project(it.id);
      const group = M.isGrouped(this.cat, p) ? p.collectionId : null;
      const same = this.cat.projects
        .map((q, i) => [q, i])
        .filter(([q]) => (M.isGrouped(this.cat, q) ? q.collectionId : null) === group)
        .map(([, i]) => i);
      const i = same.findIndex((k) => this.cat.projects[k].id === p.id);
      const j = i + delta;
      if (j < 0 || j >= same.length) return;
      const ps = this.cat.projects;
      [ps[same[i]], ps[same[j]]] = [ps[same[j]], ps[same[i]]];
    } else return;
    cx.spawn(async (cx) => {
      await this.save(cx);
      const { items } = this.derived();
      if (it.kind === "tile") this.selectId(items, "tile", it.id);
      else this.selectId(items, "header", it.collection);
      cx.notify();
    });
  }

  /** `machine`: the folder is on that machine (the focused terminal's), null for this one. */
  addPath(cx, raw, collection, machine = null) {
    cx.spawn(async (cx) => {
      try {
        const runm = this.runner({ machine });
        const home = await this.homeOn(runm, machine);
        if (!home) throw new Error(`cannot reach ${machine}`);
        const base = (machine === this.focusedMachine() && focused()?.cwd) || home;
        const r = await M.resolveFolder(runm, raw, base, home);
        if (!r.ok) {
          this.say(r.error, true);
          return cx.notify();
        }
        const existing = M.findPath(this.cat, r.path, machine);
        if (existing) {
          this.say(`already listed: ${existing.name} — editing`);
          this.projectForm(existing, false);
        } else {
          const p = { id: M.newId(), name: r.name, path: r.path, logo: { kind: "auto" } };
          if (machine) p.machine = machine;
          if (collection) p.collectionId = collection;
          this.projectForm(p, true);
        }
      } catch (e) {
        this.say(e.message, true);
      }
      cx.notify();
    });
  }

  importAll(cx) {
    const folders = M.openFolders(spaces(), panes());
    this.say("importing…");
    cx.spawn(async (cx) => {
      try {
        // Open folders, each normalized to its git top level on its own machine.
        const roots = [];
        for (const f of folders) {
          const r = await M.resolveFolder(this.runner(f), f.path, f.path, this.home);
          if (r.ok) roots.push({ path: r.path, machine: f.machine });
        }
        let vscode = null;
        for (const path of M.vscodeCatalogPaths(this.home)) {
          const text = await readTextFile(path).catch(() => null);
          if (text === null) continue;
          try {
            vscode = M.parseCatalog(text);
            break;
          } catch {}
        }
        await this.reloadCatalog();
        if (this.catError) throw new Error("projects.json does not parse, not touching it");
        const before = new Set(this.cat.projects.map((p) => p.id));
        const was = M.catalogText(this.cat);
        const added = M.importInto(this.cat, vscode, roots, this.home);
        // Collections alone (a new one, a description, a colour) are a change too.
        if (M.catalogText(this.cat) !== was && !(await this.save(cx))) return cx.notify();
        this.say(`imported or updated: ${added}${vscode ? "" : " (VS Code Project Tiles catalog not found)"}`);
        cx.notify();
        const fresh = this.cat.projects.filter((p) => !before.has(p.id)).map((p) => p.id);
        if (fresh.length) await this.refresh(cx, fresh, { fetch: this.settings.fetch_remote });
      } catch (e) {
        this.say(`import: ${e.message}`, true);
      }
      cx.notify();
    });
  }

  editFile() {
    this.dismiss();
    run("tab.new", {
      name: "projects.json",
      argv: ["sh", "-c", 'exec ${VISUAL:-${EDITOR:-vi}} "$1"', "sh", this.paths.catalog],
    });
  }

  // ------------------------------------------------------------ forms

  makeForm(title, fields, onSubmit) {
    const inputs = fields.map((f) => {
      if (f.options) return { ...f, choice: f.choice ?? 0 };
      const input = InputState.new({ value: f.value ?? "", placeholder: f.placeholder ?? "" });
      input.on("submit", (_e, cx) => this.submitForm(cx));
      return { ...f, input };
    });
    this.modal = { kind: "form", title, inputs, focus: 0, error: null, onSubmit };
    this.focusSelf();
  }

  formValue(i) {
    const f = this.modal.inputs[i];
    return f.options ? f.options[f.choice]?.value ?? null : f.input.value();
  }

  submitForm(cx) {
    const m = this.modal;
    if (!m || m.kind !== "form") return;
    cx.spawn(async (cx) => {
      const err = await m.onSubmit(cx);
      if (err) {
        m.error = err;
      } else if (this.modal === m) {
        this.modal = null;
        this.focusSelf();
      }
      cx.notify();
    });
  }

  collectionOptions() {
    return [{ label: "— no collection —", value: null }, ...this.cat.collections.map((c) => ({ label: c.name, value: c.id }))];
  }

  projectForm(p, isNew) {
    const opts = this.collectionOptions();
    const cur = Math.max(0, opts.findIndex((o) => o.value && o.value === p.collectionId));
    this.makeForm(isNew ? "New project" : `Project: ${p.name}`, [
      { label: "Name", value: p.name },
      { label: "Path", value: M.machineOf(p) ? p.path : M.shortPath(p.path, this.home), hint: M.machineOf(p) ? `folder on ${M.machineOf(p)}; ~ is that machine's home` : "project folder; ~ is expanded" },
      { label: "Description", value: p.description ?? "" },
      { label: "Colour", value: p.color ?? "", hint: "#hex or red orange yellow green teal blue purple gray; empty: the collection's colour" },
      { label: "Logo", value: M.emojiOf(p) ?? "", hint: "up to 3 emoji; empty: initials" },
      { label: "Languages", value: (p.languages ?? []).join(", "), hint: "comma-separated; empty: detect automatically" },
      { label: "Collection", options: opts, choice: cur, hint: "←→ to choose; n creates a new collection" },
    ], async (cx) => {
      const raw = this.formValue(1).trim();
      const machine = M.machineOf(p);
      const runm = this.runner(p);
      let path = p.path;
      if (raw && raw !== M.shortPath(p.path, this.home)) {
        const home = await this.homeOn(runm, machine);
        if (!home) return `cannot reach ${machine}`;
        if (M.norm(M.resolvePath(raw, home, home), machine) !== M.norm(p.path, machine)) {
          const r = await M.resolveFolder(runm, raw, home, home);
          if (!r.ok) return r.error;
          path = r.path;
        }
      }
      const other = M.findPath(this.cat, path, M.machineOf(p));
      if (other && other.id !== p.id) return `this path is already the project "${other.name}"`;
      const colorRaw = this.formValue(3).trim();
      const color = colorRaw ? M.parseColor(colorRaw, theme()) : null;
      if (colorRaw && !color) return `unknown colour "${colorRaw}"`;
      const q = isNew ? { ...p } : this.project(p.id);
      if (!q) return "the project is no longer in the catalog";
      q.name = this.formValue(0).trim() || M.baseName(path);
      // Another folder: the old one's branch, links and languages no longer apply.
      if (q.path !== path) delete this.meta[q.id];
      q.path = path;
      const desc = this.formValue(2).trim();
      if (desc) q.description = desc;
      else delete q.description;
      if (color) q.color = color;
      else delete q.color;
      const emoji = M.clampGraphemes(this.formValue(4).trim(), 3);
      q.logo = emoji ? { kind: "emoji", value: emoji } : { kind: "auto" };
      const langs = this.formValue(5).split(",").map((s) => s.trim()).filter(Boolean);
      if (langs.length) q.languages = langs;
      else delete q.languages;
      const cid = this.formValue(6);
      if (cid) q.collectionId = cid;
      else delete q.collectionId;
      if (isNew) this.cat.projects.push(q);
      if (!(await this.save(cx))) return "not saved (the file changed), open the form again";
      const { items } = this.derived();
      this.selectId(items, "tile", q.id);
      this.say(`saved: ${q.name}`);
      cx.spawn(async (cx) => this.refresh(cx, [q.id], { force: true }));
      return null;
    });
  }

  collectionForm(c) {
    this.makeForm(c ? "Collection" : "New collection", [
      { label: "Collection name", value: c?.name ?? "" },
      { label: "Description", value: c?.description ?? "" },
      { label: "Colour", value: c?.color ?? "", hint: "#hex or red orange yellow green teal blue purple gray" },
    ], async (cx) => {
      const name = this.formValue(0).trim();
      if (!name) return "the collection name cannot be empty";
      const dup = M.collectionByName(this.cat, name);
      if (dup && dup.id !== c?.id) return `collection "${dup.name}" already exists`;
      const colorRaw = this.formValue(2).trim();
      const color = colorRaw ? M.parseColor(colorRaw, theme()) : null;
      if (colorRaw && !color) return `unknown colour "${colorRaw}"`;
      let target = c ? this.cat.collections.find((x) => x.id === c.id) : null;
      if (!target) {
        target = { id: M.newId(), name, color: color ?? M.pickColor(name, theme()), collapsed: false };
        this.cat.collections.push(target);
      }
      target.name = name;
      const desc = this.formValue(1).trim();
      if (desc) target.description = desc;
      else delete target.description;
      if (color) target.color = color;
      if (!(await this.save(cx))) return "not saved (the file changed), open the form again";
      const { items } = this.derived();
      this.selectId(items, "header", target.id);
      this.say(`collection saved: ${name}`);
      return null;
    });
  }

  moveForm(p) {
    const opts = this.collectionOptions();
    const cur = Math.max(0, opts.findIndex((o) => o.value && o.value === p.collectionId));
    this.makeForm(`Move "${p.name}"`, [{ label: "To collection", options: opts, choice: cur, hint: "←→ to choose" }], async (cx) => {
      const q = this.project(p.id);
      const cid = this.formValue(0);
      if (cid) {
        q.collectionId = cid;
        const c = this.cat.collections.find((x) => x.id === cid);
        if (c) c.collapsed = false;
      } else delete q.collectionId;
      if (!(await this.save(cx))) return "not saved";
      const { items } = this.derived();
      this.selectId(items, "tile", p.id);
      return null;
    });
  }

  confirm(text, onYes) {
    this.modal = { kind: "confirm", text, onYes };
    this.focusSelf();
  }

  // ------------------------------------------------------------ keys

  onKey(e, cx, d) {
    const k = e.key;
    const ch = e.key_char ?? "";
    const m = this.modal;
    if (m?.kind === "form") {
      const f = m.inputs[m.focus];
      if (k === "escape") this.modal = null;
      else if (k === "tab" || (k === "down" && !f.input)) m.focus = (m.focus + (e.modifiers.shift ? m.inputs.length - 1 : 1)) % m.inputs.length;
      else if (k === "up" && !f.input) m.focus = (m.focus + m.inputs.length - 1) % m.inputs.length;
      else if (f.options && (k === "left" || k === "right" || k === "space")) {
        const n = f.options.length;
        f.choice = (f.choice + (k === "left" ? n - 1 : 1)) % n;
      } else if (k === "enter" && f.options) this.submitForm(cx);
      else return;
      this.focusSelf();
      cx.stop_propagation();
      cx.notify();
      return;
    }
    if (m?.kind === "confirm") {
      if (["y"].includes(ch.toLowerCase())) {
        this.modal = null;
        m.onYes(cx);
      } else if (["n"].includes(ch.toLowerCase()) || k === "escape" || k === "enter") {
        this.modal = null;
        this.say("cancelled");
      } else return;
      cx.stop_propagation();
      cx.notify();
      return;
    }
    if (this.help) {
      this.help = false;
      cx.stop_propagation();
      cx.notify();
      return;
    }
    if (this.search) {
      if (k === "escape") {
        this.search = null;
        this.query = "";
      } else if (k === "enter" || k === "down" || k === "up") {
        this.search = null;
        const first = d.items.findIndex((it) => it.kind === "tile");
        if (first >= 0) this.at = first;
      } else return;
      this.focusSelf();
      cx.stop_propagation();
      cx.notify();
      return;
    }
    if (e.modifiers.platform || e.modifiers.control) return;
    const cur = this.current(d.items);
    const p = cur?.kind === "tile" ? this.project(cur.id) : null;
    const head = cur?.kind === "header" ? cur.collection : undefined;
    this.status = null;
    const navs = { left: [-1, 0], h: [-1, 0], right: [1, 0], l: [1, 0], up: [0, -1], k: [0, -1], down: [0, 1], j: [0, 1] };
    const key = navs[k] && !e.modifiers.shift ? k : ch;
    if (navs[key]) this.at = M.nav(d.items, this.at ?? 0, ...navs[key]);
    else if (k === "escape" || key === "q") {
      if (this.query) this.query = "";
      else return this.dismiss();
    } else if (k === "enter") {
      if (p) this.open(p, this.settings.open_mode === "tab" ? "tab" : "space", d.pres);
      else if (head) this.toggleCollapse(cx, head);
    } else if (k === "space") {
      const cid = head ?? (p && M.isGrouped(this.cat, p) ? p.collectionId : null);
      if (cid) this.toggleCollapse(cx, cid);
    } else if (key === "?") this.help = true;
    else if (key === "/") this.startSearch();
    else if (key === "o" && p) this.open(p, "new", d.pres);
    else if (key === "t" && p) this.open(p, "tab", d.pres);
    else if (key === "w" && p) this.worktree(p, "new");
    else if (key === "W" && p) this.worktree(p, "open");
    else if (key === "g" && p) this.gitview(p, d.pres);
    else if (key === "a") {
      const cwd = focused()?.cwd;
      if (cwd) this.addPath(cx, cwd, this.collectionOf(cur), this.focusedMachine());
      else this.say("no current folder — use A", true);
    } else if (key === "A") {
      // On the focused terminal's machine, starting from its folder.
      const machine = this.focusedMachine();
      const cwd = focused()?.cwd ?? "";
      const start = (machine ? cwd : M.shortPath(cwd, this.home)) || "~/";
      const coll = this.collectionOf(cur);
      this.makeForm("Add folder", [{ label: "Folder path", value: start, hint: "~ is expanded; the path is normalized to the git root" }], async (cx) => {
        const v = this.formValue(0).trim();
        if (!v) return "enter a path";
        this.modal = null;
        this.addPath(cx, v, coll, machine);
        return null;
      });
    } else if (key === "I") this.importAll(cx);
    else if (key === "n") this.collectionForm(null);
    else if (key === "e") {
      if (p) this.projectForm(p, false);
      else if (head) this.collectionForm(this.cat.collections.find((c) => c.id === head));
    } else if (key === "d") {
      if (p) {
        this.confirm(`Remove "${p.name}" from the list? The folder is not touched.`, (cx) => {
          this.cat.projects = this.cat.projects.filter((q) => q.id !== p.id);
          delete this.meta[p.id];
          cx.spawn(async (cx) => {
            if (await this.save(cx)) this.say(`removed: ${p.name}`);
            cx.notify();
          });
        });
      } else if (head) {
        const c = this.cat.collections.find((c) => c.id === head);
        this.confirm(`Delete collection "${c?.name}"? Its projects move to no collection.`, (cx) => {
          this.cat.collections = this.cat.collections.filter((x) => x.id !== head);
          for (const q of this.cat.projects) if (q.collectionId === head) delete q.collectionId;
          cx.spawn(async (cx) => {
            if (await this.save(cx)) this.say("collection deleted");
            cx.notify();
          });
        });
      }
    } else if (key === "m" && p) this.moveForm(p);
    else if (key === "J") this.shift(cx, d.items, 1);
    else if (key === "K") this.shift(cx, d.items, -1);
    else if (key === "f" && p) this.gitOp(cx, p, "fetch");
    else if (key === "p" && p) this.gitOp(cx, p, "pull");
    else if (key === "P" && p) this.gitOp(cx, p, "push");
    else if (key === "r" && p) {
      this.say(`refreshing ${p.name}…`);
      cx.spawn(async (cx) => this.refresh(cx, [p.id], { force: true }));
    } else if (key === "R") this.refreshAll(cx);
    else if (key === "O" && p) {
      if (M.machineOf(p)) this.say(`on ${M.machineOf(p)}: Finder shows this computer only`, true);
      else {
        cx.spawn(async (cx) => {
          const r = await exec("open", ["--", p.path]).catch((e) => ({ code: 1, stderr: e.message }));
          if (r.code !== 0) {
            this.say(`Finder: ${(r.stderr ?? "").trim() || `exit ${r.code}`}`, true);
            cx.notify();
          }
        });
      }
    }
    else if (key === "c") {
      cx.spawn(async (cx) => {
        await this.saveSetting("tile_size", this.settings.tile_size === "compact" ? "normal" : "compact");
        cx.notify();
      });
    } else if (key === "E") this.editFile();
    else return;
    this.focusSelf();
    cx.stop_propagation();
    cx.notify();
  }

  /** $HOME on `machine` (this computer's for null), asked once per machine. */
  async homeOn(run, machine) {
    if (!machine) return this.home;
    const st = (this.machines[machine] ??= {});
    st.home ??= await M.machineHome(run);
    return st.home;
  }

  /** The machine of the focused terminal's space (null: this computer). */
  focusedMachine() {
    return M.machineOfTab(spaces(), focused()?.tab);
  }

  collectionOf(it) {
    if (!it) return null;
    if (it.kind === "header") return it.collection;
    const p = this.project(it.id);
    return p && M.isGrouped(this.cat, p) ? p.collectionId : null;
  }

  // ------------------------------------------------------------ render

  icon(name, size, color) {
    return svg(`mods/project-tiles/icons/${name}.svg`).size(size).flex_none().text_color(color);
  }

  /** A footer/header button: primary (accent fill) or secondary (raised). Its clicks stop here,
   *  so a double click does not also reach the tile (which opens the project). */
  button(t, id, icon, label, primary, onClick) {
    return h_flex().id(id).flex_none().h(26).px(label ? 9 : 7).gap(5).items_center().rounded(6).cursor_pointer()
      .bg(primary ? t.accent : t.raised)
      .text_color(primary ? t.accent_foreground : t.foreground)
      .hover((s) => s.opacity(0.85))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click((_e, cx) => { cx.stop_propagation(); onClick(cx); })
      .when(Boolean(icon), (el) => el.child(this.icon(icon, 13, primary ? t.accent_foreground : t.muted)))
      .when(Boolean(label), (el) => el.child(div().text_size(12).font_medium().whitespace_nowrap().child(label)));
  }

  iconButton(t, id, icon, label, color, onClick) {
    return h_flex().id(id).flex_none().h(24).px(5).gap(3).items_center().rounded(6).cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click((_e, cx) => { cx.stop_propagation(); onClick(cx); })
      .child(this.icon(icon, 13, color))
      .when(Boolean(label), (el) => el.child(div().text_size(11.5).font_medium().text_color(color).child(label)));
  }

  pickFolder(cx) {
    cx.spawn(async (cx) => {
      const dir = await pickFolder({ prompt: "Add" });
      if (dir) this.addPath(cx, dir, null);
      cx.notify();
    });
  }

  render() {
    const t = theme();
    const d = this.derived();
    if (this.at === null) this.at = 0;
    this.at = Math.min(this.at, Math.max(0, d.items.length - 1));
    const cur = this.current(d.items);
    // Bring the selection into view only when it moves; live updates keep a manual scroll.
    const key = `${this.at}:${cur?.row ?? 0}:${d.items.length}`;
    if (key !== this.scrolledFor) {
      this.scrolledFor = key;
      this.scroll.scroll_to_item(cur ? cur.row : 0);
    }

    const search = h_flex().id("pt-search").w(300).flex_shrink(1).min_w(140).h(28).px(9).gap(7).items_center().rounded(6)
      .bg(t.raised).border_1().border_color(this.search ? t.accent : t.raised).cursor_text()
      .on_click((_e, cx) => { if (!this.search) this.startSearch(); cx.notify(); })
      .child(this.icon("search", 13, t.dim))
      .child(this.search
        ? Input.new(this.search).flex_1().h_full().text_size(12).text_color(t.foreground)
        : div().flex_1().truncate().text_size(12).text_color(this.query ? t.foreground : t.dim).child(this.query || "Search projects…"));

    const busy = this.loading > 0 || this.busy.size > 0;
    const header = h_flex().h(42).flex_none().px(14).gap(8).items_center().border_b(1).border_color(t.border).bg(t.background)
      .child(div().flex_none().font_semibold().pr(6).text_color(t.foreground).child("Projects"))
      .child(search)
      .child(div().flex_1())
      .child(this.button(t, "pt-add", "plus", "Add current folder", true, (cx) => {
        const cwd = focused()?.cwd;
        if (cwd) this.addPath(cx, cwd, null, this.focusedMachine());
        else this.say("no current folder — use Add folder…", true);
        cx.notify();
      }))
      .child(this.button(t, "pt-import", "clock", "Add open folders…", false, (cx) => this.importAll(cx)))
      .child(this.button(t, "pt-folder", "folder", "Add folder…", false, (cx) => this.pickFolder(cx)))
      .child(this.button(t, "pt-coll", "plus", "New collection", false, (cx) => { this.collectionForm(null); cx.notify(); }))
      .child(h_flex().id("pt-refresh").flex_none().size(26).items_center().justify_center().rounded(6).cursor_pointer()
        .hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => this.refreshAll(cx))
        .child(busy
          ? this.icon("spinner", 14, t.working).spin(900)
          : this.icon("refresh", 14, t.muted)))
      .when(!this.embedded, (el) => el.child(div().id("pt-esc").flex_none().px(6).py(2).rounded(5).text_size(11).text_color(t.dim).cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_click(() => this.dismiss()).child("Esc")));

    const body = this.cat.projects.length ? this.renderRows(t, d, cur) : this.renderEmpty(t);

    const statusLine = h_flex().flex_none().h(16).gap(12).items_center().text_size(11)
      .child(div().flex_1().truncate()
        .text_color(this.status ? (this.status.err ? t.blocked : t.done) : t.dim)
        .child(this.status?.text ?? `←↑↓→ move · Enter open · o new space · t tab · e edit · / search · ? keys · Esc ${this.embedded ? "back to terminals" : "close"}`))
      .when(Boolean(focused()?.cwd), (el) =>
        el.child(div().flex_none().max_w(420).truncate().text_color(t.dim).child(`here: ${M.shortPath(focused().cwd, this.home)}`)));

    const root = v_flex()
      .id("pt-root")
      .track_focus(this.handle)
      .on_key_down((e, cx) => this.onKey(e, cx, d))
      .on_size_change((size, cx) => {
        if (Math.abs((this.width ?? 0) - size.width) > 1) {
          this.width = size.width;
          cx.notify();
        }
      })
      .on_mouse_down("left", () => this.focusSelf())
      .size_full()
      .min_h(0)
      .when(this.embedded, (el) => el.rounded(10).overflow_hidden().border_1().border_color(t.border).bg(t.canvas))
      .when(!this.embedded, (el) => el.bg(t.popover))
      .text_size(13)
      .child(header)
      .child(v_flex().flex_1().min_h(0).px(14).pt(12).pb(8).gap(10).child(body).child(statusLine));
    if (this.help) return root.child(this.renderHelp(t));
    if (this.modal?.kind === "form") return root.child(this.renderForm(t));
    if (this.modal?.kind === "confirm") return root.child(this.renderConfirm(t));
    return root;
  }

  startSearch() {
    this.search = InputState.new({ value: this.query, placeholder: "Search projects…" });
    this.search.on("change", (_e, cx) => {
      this.query = this.search?.value() ?? "";
      this.at = 0;
      cx.notify();
    });
    this.search.on("submit", (_e, cx) => {
      this.search = null;
      const { items } = this.derived();
      this.at = Math.max(0, items.findIndex((it) => it.kind === "tile"));
      this.focusSelf();
      cx.notify();
    });
    this.search.focus();
  }

  refreshAll(cx) {
    this.say("refreshing all projects…");
    cx.spawn(async (cx) => {
      await this.reloadCatalog();
      await this.refresh(cx, null, { force: true, fetch: this.settings.fetch_remote });
      // An error said meanwhile (a failed write, an unreadable file) stays.
      if (!this.status?.err) this.say("refreshed");
      cx.notify();
    });
  }

  renderEmpty(t) {
    return v_flex().flex_1().items_center().justify_center().gap(8).text_color(t.muted)
      .child(div().text_size(13).font_semibold().text_color(t.foreground).child("No projects yet"))
      .child("Add current folder, a folder from the picker, or the folders of open terminals.")
      .child(div().text_size(11).text_color(t.dim).child("Keys: a — current folder · A — by path · I — import (open terminals + VS Code Project Tiles)"));
  }

  renderRows(t, d, cur) {
    const gap = 10;
    const inner = Math.max(240, this.width - 28 - 8);
    const tileW = Math.floor((inner - gap * (d.cols - 1)) / d.cols);
    const children = d.rows.map((r, ri) => {
      if (r.kind === "header") {
        const c = this.cat.collections.find((x) => x.id === r.collection);
        const sel = cur?.kind === "header" && cur.row === ri;
        const collapsed = c?.collapsed && !this.query;
        const color = c ? M.normalizeHex(c.color ?? "") ?? t.muted : t.dim;
        return h_flex().id(`pt-h-${r.collection ?? "none"}`).flex_none().gap(8).items_center()
          .pt(ri ? 10 : 0).pb(6).px(2).border_b(1).border_color(sel ? t.accent : t.border)
          .cursor_pointer()
          .on_click((_e, cx) => {
            const i = d.items.findIndex((it) => it.kind === "header" && it.row === ri);
            if (i >= 0) this.at = i;
            if (c) this.toggleCollapse(cx, c.id);
            cx.notify();
          })
          .child(div().size(8).rounded(4).bg(color))
          .child(div().text_size(12.5).font_semibold().text_color(sel ? t.accent : t.foreground).child(c?.name ?? "No collection"))
          .when(Boolean(c), (el) => el.child(this.icon(collapsed ? "chevron-right" : "chevron-down", 12, t.dim)))
          .child(div().text_size(11).text_color(t.dim).child(String(r.count)))
          .when(Boolean(c?.description), (el) => el.child(div().truncate().text_size(11).text_color(t.dim).child(c.description)));
      }
      if (r.kind === "empty") {
        return div().p(10).text_size(11.5).text_color(t.dim).child("Empty — press m on a project to move it here.");
      }
      return h_flex().flex_none().gap(gap).items_stretch().children(r.ids.map((id) =>
        this.renderTile(t, this.project(id), tileW, cur?.kind === "tile" && cur.id === id, d)));
    });
    return v_flex().id("pt-list").flex_1().min_h(0).overflow_y_scroll().track_scroll(this.scroll).gap(10).pr(4).pb(4).children(children);
  }

  renderTile(t, p, w, sel, d) {
    const m = this.metaOf(p) ?? {};
    const pres = d.pres[p.id] ?? { spaces: [], working: 0, blocked: 0, done: 0 };
    const compact = this.settings.tile_size === "compact";
    const s = this.settings;
    const avail = m.availability;
    const busy = this.busy.has(p.id);
    const here = d.here === p.id;
    const own = M.normalizeHex(p.color ?? "") ??
      M.normalizeHex(this.cat.collections.find((c) => c.id === p.collectionId)?.color ?? "");
    const color = own ?? t.accent;
    const mode = this.settings.open_mode === "tab" ? "tab" : "space";
        const logoSize = compact ? 30 : 38;

    // Top-right pills: where it is open, agents, availability.
    const pills = [];
    const pill = (text, fg, bg) => div().flex_none().px(7).h(18).rounded(9).bg(bg).items_center().flex()
      .text_size(11).font_medium().text_color(fg).child(text);
    if (avail === "offline") pills.push(pill("offline", t.muted, t.raised));
    else if (avail) pills.push(pill("missing", t.blocked, t.raised));
    else {
      if (here) pills.push(pill("currently open", t.muted, t.raised));
      else if (pres.spaces.length) pills.push(pill(`open · space ${pres.spaces.map((x) => x.index).join(", ")}`, t.muted, t.raised));
      if (pres.working) pills.push(h_flex().flex_none().gap(4).px(7).h(18).rounded(9).bg(t.raised).items_center()
        .child(this.icon("spinner", 11, t.working).spin(900)).child(div().text_size(11).text_color(t.working).child(String(pres.working))));
      if (pres.blocked) pills.push(pill(`● ${pres.blocked} blocked`, t.blocked, t.raised));
      if (pres.done) pills.push(pill(`● ${pres.done} done`, t.done, t.raised));
    }

    const emoji = M.emojiOf(p);
    const logo = h_flex().flex_none().size(logoSize).rounded(compact ? 6 : 8).items_center().justify_center()
      .bg(emoji ? t.raised : color)
      .child(emoji
        ? div().text_size(compact ? 16 : 19).child(M.clampGraphemes(emoji, 3))
        : div().text_size(compact ? 11.5 : 13).font_semibold().text_color("#ffffff").child(M.initials(p.name)));

    const head = h_flex().gap(10).items_center()
      .child(logo)
      .child(v_flex().flex_1().min_w(0).gap(1)
        .child(div().truncate().text_size(13).font_semibold().text_color(t.foreground).child(p.name))
        .when((compact || s.show_path) && M.shown(p, "path", s), (el) => el.child(
          div().truncate().text_ellipsis_start().text_size(11).text_color(t.dim).child(M.machineOf(p) ? `${M.machineOf(p)}: ${p.path}` : p.path))));

    const lines = [];
    if (!compact && s.show_description && M.shown(p, "description", s) && p.description) {
      lines.push(div().truncate().text_size(12).text_color(t.muted).child(p.description));
    }
    if (!compact && s.show_languages && M.shown(p, "languages", s) && !avail) {
      const langs = M.languagesOf(p, m).map((l) => ({ ...l, color: l.color ?? t.dim }));
      if (langs.length) {
        const barW = w - 2 * 14;
        lines.push(h_flex().h(5).w(barW).rounded(3).overflow_hidden().bg(t.raised).children(langs.map((l, i) =>
          i === langs.length - 1
            ? div().h_full().flex_1().bg(l.color)
            : div().h_full().flex_none().w(Math.max(2, Math.round((barW * l.pct) / 100))).bg(l.color))));
        lines.push(h_flex().flex_wrap().gap_x(12).gap_y(3).items_center().children(langs.slice(0, 6).map((l) =>
          h_flex().flex_none().gap(5).items_center()
            .child(div().size(7).rounded_full().bg(l.color))
            .child(div().text_size(11.5).font_medium().text_color(t.foreground).child(l.name))
            .child(div().text_size(11.5).text_color(t.dim).child(M.pct(l.pct))))));
      } else if (m.languages_scanned) {
        lines.push(div().text_size(11.5).text_color(t.dim).child("no languages detected"));
      }
    }
    if (s.show_git && M.shown(p, "git", s) && !avail) {
      const g = m.git;
      if (!m.git_checked) {
        lines.push(div().h(20).text_size(11.5).text_color(t.dim).child(busy ? "reading git…" : ""));
      } else if (!g) {
        lines.push(div().h(20).text_size(11.5).text_color(t.dim).child("not a git repository"));
      } else {
        const tone = { orange: t.ansi3, blue: t.ansi4, green: t.done, dim: t.muted };
        // Web pages on the remote (GitLab / GitHub / Bitbucket): the branch, else the repository.
        const links = m.links ?? {};
        const branchUrl = links.branch ?? links.repo ?? null;
        lines.push(h_flex().h(20).gap(7).items_center()
          .child(this.icon("branch", 13, t.dim))
          .child(h_flex().id(`pt-br-${p.id}`).min_w(0).flex_shrink(1).px(7).h(18).rounded(9).bg(t.raised).items_center()
            .when(Boolean(branchUrl), (el) => el.cursor_pointer().hover((st) => st.bg(t.hover))
              .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
              .on_click((_e, cx) => { cx.stop_propagation(); this.openUrl(cx, branchUrl); }))
            .child(div().truncate().text_size(11.5).font_medium().text_color(t.foreground).child(g.branch)))
          .when(g.dirty > 0, (el) => el.child(div().id(`pt-dirty-${p.id}`).flex_none().px(4).rounded(4).text_size(11).text_color(t.working)
            .when(this.hasGitview, (d2) => d2.cursor_pointer().hover((st) => st.bg(t.hover))
              .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
              .on_click((_e, cx) => { cx.stop_propagation(); this.gitview(p, d.pres); }))
            .child(`● ${g.dirty}`)))
          .when((g.worktrees?.length ?? 0) > 0, (el) => el.child(h_flex().id(`pt-wt-${p.id}`).flex_none().gap(3).px(6).h(18).rounded(9)
            .items_center().cursor_pointer().hover((st) => st.bg(t.hover))
            .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
            .on_click((_e, cx) => { cx.stop_propagation(); this.worktree(p, "open"); })
            .child(this.icon("worktree", 11, t.muted))
            .child(div().text_size(11).text_color(t.muted).child(String(g.worktrees.length)))))
          .child(div().flex_1())
          .children(M.gitPills(g).map((x) => h_flex().flex_none().gap(4).px(7).h(18).rounded(9).items_center()
            .bg(x.tone === "dim" ? t.raised : t.raised)
            .when(Boolean(x.icon), (el) => el.child(this.icon(x.icon, 11, tone[x.tone])))
            .child(div().text_size(11).font_medium().text_color(tone[x.tone]).child(x.text)))));
        if (!compact && g.commit) {
          const c = g.commit;
          lines.push(h_flex().h(16).gap(7).items_center()
            .child(this.icon("commit", 13, t.dim))
            .child(div().id(`pt-c-${p.id}`).flex_none().text_size(11.5).text_color(t.ansi4)
              .when(Boolean(links.commit), (el) => el.cursor_pointer().hover((st) => st.underline())
                .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                .on_click((_e, cx) => { cx.stop_propagation(); this.openUrl(cx, links.commit); }))
              .child(c.hash))
            .child(div().flex_1().min_w(0).truncate().text_size(11.5).text_color(t.muted).child(c.subject))
            .child(div().flex_none().text_size(11).text_color(t.dim).child(M.ago(c.date))));
        }
      }
    }

    const g = m.git;
    const footer = h_flex().flex_none().px(14).py(8).gap(6).items_center().border_t(1).border_color(t.border)
      // Open = the project's space (or a new one); New tab = here, in the current space.
      // A second space for an open project stays on the "o" key.
      .child(this.button(t, `pt-open-${p.id}`, "window", pres.spaces.length && mode === "space" ? "Go to space" : "Open", true,
        () => this.open(p, mode, d.pres)))
      .child(this.button(t, `pt-tab-${p.id}`, "terminal", w >= 300 ? "New tab" : "", false, () => this.open(p, "tab", d.pres)))
      .when(Boolean(g), (el) => el.child(this.button(t, `pt-wt-new-${p.id}`, "worktree", w >= 400 ? "Worktree" : "", false, () => this.worktree(p, "new"))))
      .child(div().flex_1().min_w(4))
      .when(Boolean(g) && this.hasGitview, (el) => el.child(this.iconButton(t, `pt-gv-${p.id}`, "gitview", "", t.muted, () => this.gitview(p, d.pres))))
      .when(Boolean(g), (el) => el
        .child(this.iconButton(t, `pt-r-${p.id}`, busy ? "spinner" : "refresh", "", t.muted, (cx) => this.gitOp(cx, p, "fetch")))
        .child(this.iconButton(t, `pt-pull-${p.id}`, "down", g?.behind ? String(g.behind) : "", g?.behind ? t.ansi4 : t.muted, (cx) => this.gitOp(cx, p, "pull")))
        .child(this.iconButton(t, `pt-push-${p.id}`, "up", g?.ahead ? String(g.ahead) : "", g?.ahead ? t.ansi4 : t.muted, (cx) => this.gitOp(cx, p, "push"))));

    return v_flex().id(`pt-t-${p.id}`).w(w).flex_none().rounded(8).overflow_hidden()
      .border_1().border_color(sel ? t.accent : avail === "deleted" ? t.blocked : t.border)
      .when(sel, (el) => el.border_2())
      .when(avail === "offline", (el) => el.border_dashed().opacity(0.6))
      .bg(t.background)
      .on_click((e, cx) => {
        const i = d.items.findIndex((it) => it.kind === "tile" && it.id === p.id);
        if (i >= 0) this.at = i;
        if (e.click_count >= 2) this.open(p, mode, d.pres);
        this.focusSelf();
        cx.notify();
      })
      .child(v_flex().flex_1().px(14).pt(12).pb(10).gap(8)
        .child(h_flex().items_start().gap(8)
          .child(div().flex_1().min_w(0).child(head))
          .child(h_flex().flex_none().gap(6).items_center().children(pills)))
        .child(div().flex_1())
        .children(lines))
      .child(footer);
  }

  overlayBox(t, w, children) {
    return div().absolute().top(0).left(0).right(0).bottom(0).bg("#00000066").items_center().justify_center().flex()
      .child(v_flex().id("pt-modal").w(w).max_h_full().p(16).gap(8).rounded(10).border_1().border_color(t.accent).bg(t.popover)
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .children(children));
  }

  renderHelp(t) {
    return this.overlayBox(t, 560, [
      div().text_size(14).font_semibold().text_color(t.foreground).child("Keys"),
      ...HELP.map(([k, v]) => h_flex().gap(12)
        .child(div().w(110).flex_none().text_size(12).text_color(t.working).child(k))
        .child(div().flex_1().text_size(12).text_color(t.foreground).child(v))),
      div().pt(4).text_size(11).text_color(t.dim).child("any key closes"),
    ]);
  }

  renderConfirm(t) {
    return this.overlayBox(t, 460, [
      div().text_size(14).font_semibold().text_color(t.foreground).child("Confirm"),
      div().text_size(12.5).text_color(t.foreground).child(this.modal.text),
      div().text_size(11).text_color(t.dim).child("y — yes · n / Esc — no"),
    ]);
  }

  renderForm(t) {
    const m = this.modal;
    const fields = m.inputs.map((f, i) => {
      const on = i === m.focus;
      const control = f.options
        ? h_flex().id(`pt-f-${i}`).h(30).px(10).gap(8).items_center().rounded(6).border_1()
          .border_color(on ? t.accent : t.border).bg(t.background).cursor_pointer()
          .on_click((_e, cx) => { m.focus = i; f.choice = (f.choice + 1) % f.options.length; this.focusSelf(); cx.notify(); })
          .child(div().text_color(t.dim).child("◂"))
          .child(div().flex_1().text_color(t.foreground).child(f.options[f.choice]?.label ?? ""))
          .child(div().text_color(t.dim).child("▸"))
        : h_flex().id(`pt-f-${i}`).h(30).px(10).items_center().rounded(6).border_1()
          .border_color(on ? t.accent : t.border).bg(t.background)
          .on_mouse_down("left", (_e, cx) => { m.focus = i; cx.notify(); })
          .child(Input.new(f.input).flex_1().h_full().text_size(12.8).text_color(t.foreground));
      return v_flex().gap(3)
        .child(div().text_size(11.5).text_color(on ? t.foreground : t.muted).when(on, (el) => el.font_semibold()).child(f.label))
        .child(control);
    });
    const hint = m.inputs[m.focus]?.hint;
    return this.overlayBox(t, 600, [
      div().text_size(14).font_semibold().text_color(t.foreground).child(m.title),
      ...fields,
      div().h(16).text_size(11).text_color(t.dim).child(hint ?? ""),
      div().h(16).text_size(11.5).text_color(t.blocked).child(m.error ?? ""),
      h_flex().gap(8).items_center()
        .child(div().flex_1().text_size(11).text_color(t.dim).child("Tab / Shift+Tab — field · ←→ — choose · Enter — save · Esc — cancel"))
        .child(div().id("pt-cancel").px(10).py(4).rounded(6).text_size(12).text_color(t.muted).cursor_pointer()
          .hover((s) => s.bg(t.hover)).on_click((_e, cx) => { this.modal = null; this.focusSelf(); cx.notify(); }).child("Cancel"))
        .child(div().id("pt-save").px(12).py(4).rounded(6).text_size(12).bg(t.solid).text_color(t.solid_foreground).cursor_pointer()
          .on_click((_e, cx) => this.submitForm(cx)).child("Save")),
    ]);
  }
}
