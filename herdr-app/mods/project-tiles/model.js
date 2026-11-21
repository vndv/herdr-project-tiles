// Pure logic for the Project Tiles mod: no "herdr" or "gpui-kit" imports, so node can test it
// (mod/test.mjs). The UI is in main.js.

/** Up to `max` user-perceived characters (ZWJ sequences, flags, skin tones count as one). */
export function clampGraphemes(s, max) {
  const out = [];
  const chars = Array.from(s ?? "");
  let i = 0;
  while (i < chars.length && out.length < max) {
    let g = chars[i++];
    // Regional-indicator pairs (flags).
    if (isRegional(g) && i < chars.length && isRegional(chars[i])) g += chars[i++];
    // Joiners, variation selectors, skin tones, combining marks stick to the previous char.
    while (i < chars.length && (isExtender(chars[i]) || chars[i - 1] === "‍")) g += chars[i++];
    out.push(g);
  }
  return out.join("");
}

function isRegional(c) {
  const cp = c.codePointAt(0);
  return cp >= 0x1f1e6 && cp <= 0x1f1ff;
}

function isExtender(c) {
  const cp = c.codePointAt(0);
  return (
    c === "‍" ||
    (cp >= 0xfe00 && cp <= 0xfe0f) ||
    (cp >= 0x1f3fb && cp <= 0x1f3ff) ||
    (cp >= 0x0300 && cp <= 0x036f) ||
    (cp >= 0xe0020 && cp <= 0xe007f)
  );
}

/** "MyGameStudio" → "MGS", "shop-backend" → "SB", "Tools" → "TO". */
export function initials(name) {
  const spaced = (name ?? "").replace(/([\p{Ll}\d])(\p{Lu})/gu, "$1 $2");
  const words = spaced.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return clampGraphemes(words[0], 2).toUpperCase();
  return words.slice(0, 3).map((w) => clampGraphemes(w, 1)).join("").toUpperCase();
}

export function badge(p) {
  const e = emojiOf(p);
  return e ? clampGraphemes(e, 3) : initials(p.name);
}

export function emojiOf(p) {
  const l = p?.logo;
  return l && l.kind === "emoji" && l.value && l.value.trim() ? l.value : null;
}

export function shortPath(p, home) {
  if (!p || !home || home === "/") return p ?? "";
  return p === home || p.startsWith(home + "/") ? "~" + p.slice(home.length) : p;
}

export function expandPath(p, home) {
  const s = (p ?? "").trim();
  if (s === "~") return home;
  if (s.startsWith("~/")) return home + s.slice(1);
  return s;
}

export function norm(p, machine = null) {
  let s = (p ?? "").replace(/\/+$/, "");
  if (!s) s = "/";
  // A remote machine's file system is not known (Linux is case-sensitive): exact paths there.
  if (machine) return s;
  // Herdr.app runs on macOS: case-insensitive by default, and /tmp, /var, /etc are symlinks
  // into /private (git reports one form, a shell cwd the other).
  s = s.replace(/^\/private(?=\/(?:tmp|var|etc)(?:\/|$))/, "");
  return s.toLowerCase();
}

/** `child` is `parent` or inside it (both on `machine`). */
export function within(child, parent, machine = null) {
  if (!child || !parent) return false;
  const c = norm(child, machine);
  const p = norm(parent, machine);
  return c === p || (p === "/" ? c.startsWith("/") : c.startsWith(p + "/"));
}

// ---------------------------------------------------------------- colours

/** Names accepted in colour fields → theme colour names. */
export const PALETTE = {
  red: "ansi1",
  orange: "ansi3",
  yellow: "ansi11",
  green: "ansi2",
  teal: "ansi6",
  blue: "ansi4",
  purple: "ansi5",
  gray: "dim",
};

export function normalizeHex(s) {
  const h = (s ?? "").trim().replace(/^#/, "").toLowerCase();
  if (!/^[0-9a-f]+$/.test(h)) return null;
  if (h.length === 6) return "#" + h;
  if (h.length === 3) return "#" + [...h].map((c) => c + c).join("");
  return null;
}

/** "#abc", "aabbcc" or a palette name → "#rrggbb" (palette names through the theme). */
export function parseColor(s, theme) {
  const k = (s ?? "").trim().toLowerCase();
  if (!k) return null;
  if (PALETTE[k]) return normalizeHex(theme?.[PALETTE[k]] ?? "") ?? null;
  return normalizeHex(k);
}

/** A stable palette colour for a name. */
export function pickColor(name, theme) {
  const keys = Object.keys(PALETTE).filter((k) => k !== "gray");
  let sum = 0;
  for (const c of name ?? "") sum += c.codePointAt(0);
  return parseColor(keys[sum % keys.length], theme) ?? "#888888";
}

export function tileColor(p, cat, theme) {
  const own = normalizeHex(p.color ?? "");
  if (own) return own;
  const c = cat.collections.find((c) => c.id === p.collectionId);
  return normalizeHex(c?.color ?? "") ?? pickColor(p.name, theme);
}

// ---------------------------------------------------------------- catalog

export function emptyCatalog() {
  return { version: 1, collections: [], projects: [] };
}

export function newId() {
  const h = [];
  for (let i = 0; i < 32; i++) h.push(Math.floor(Math.random() * 16).toString(16));
  h[12] = "4";
  h[16] = ((parseInt(h[16], 16) & 0x3) | 0x8).toString(16);
  const s = h.join("");
  return `${s.slice(0, 8)}-${s.slice(8, 12)}-${s.slice(12, 16)}-${s.slice(16, 20)}-${s.slice(20)}`;
}

export function baseName(path) {
  const parts = (path ?? "").replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}

/** Parses and repairs (drops entries without path/id, fills ids and names). Throws on bad JSON. */
export function parseCatalog(text) {
  const d = JSON.parse(text);
  if (!d || typeof d !== "object" || Array.isArray(d)) throw new Error("not an object");
  const out = { ...d, version: d.version ?? 1 };
  out.collections = (Array.isArray(d.collections) ? d.collections : []).filter((c) => c && c.id);
  out.projects = (Array.isArray(d.projects) ? d.projects : []).filter((p) => p && p.path);
  for (const p of out.projects) {
    if (!p.id) p.id = newId();
    if (!p.name || !String(p.name).trim()) p.name = baseName(p.path);
  }
  return out;
}

export function catalogText(cat) {
  return JSON.stringify(cat, null, 2) + "\n";
}

/** The machine a project lives on (`machine` in the catalog, a space's machine); null: here. */
export function machineOf(p) {
  return p?.machine || null;
}

export function findPath(cat, path, machine = null) {
  const k = norm(path, machine);
  return cat.projects.find((p) => machineOf(p) === (machine || null) && norm(p.path, machine) === k) ?? null;
}

export function collectionByName(cat, name) {
  const n = (name ?? "").trim().toLowerCase();
  return cat.collections.find((c) => (c.name ?? "").trim().toLowerCase() === n) ?? null;
}

export function isGrouped(cat, p) {
  return Boolean(p.collectionId && cat.collections.some((c) => c.id === p.collectionId));
}

/** Folders the language scan skips (besides hidden ones). */
export const IGNORE_DIRS = [
  ".git", ".svn", ".hg", "node_modules", "bower_components", "vendor", "Library", "Temp", "Logs", "obj", "bin",
  "build", "Build", "dist", "out", "target", ".idea", ".vs", ".vscode", "__pycache__", ".venv", "venv", "env",
  ".next", ".nuxt", "coverage", ".gradle", "Pods", "DerivedData", ".dart_tool", ".cache", ".mypy_cache",
  ".pytest_cache", ".ruff_cache", ".terraform", "dbt_packages", "logs",
];

export const DEFAULT_SETTINGS = {
  columns: 3,
  tile_size: "normal",
  show_path: true,
  show_description: true,
  show_languages: true,
  show_git: true,
  fetch_remote: true,
  metadata_cache_minutes: 10,
  language_cache_hours: 12,
  max_files_scanned: 15000,
  open_mode: "workspace",
  language_ignore_dirs: IGNORE_DIRS,
};

export function shown(p, key, settings) {
  const own = p.show?.[key];
  return own === undefined || own === null ? Boolean(settings[`show_${key}`]) : Boolean(own);
}

// ---------------------------------------------------------------- search, sections, layout

export function languagesOf(p, m) {
  const manual = Array.isArray(p.languages) && p.languages.length ? p.languages : null;
  if (manual) return manual.map((name) => ({ name, pct: 100 / manual.length, color: null }));
  return m?.languages ?? [];
}

export function matches(p, m, query) {
  const q = (query ?? "").trim().toLowerCase();
  if (!q) return true;
  const hay = [p.name, p.path, p.description ?? "", languagesOf(p, m).map((l) => l.name).join(" "), m?.git?.branch ?? ""]
    .join(" ")
    .toLowerCase();
  return q.split(/\s+/).every((w) => hay.includes(w));
}

/** [{collection: id|null, projects: [project]}] in display order. */
export function sections(cat, meta, query) {
  const out = [];
  const ungrouped = cat.projects.filter((p) => !isGrouped(cat, p) && matches(p, meta[p.id], query));
  if (ungrouped.length || !cat.collections.length) out.push({ collection: null, projects: ungrouped });
  for (const c of cat.collections) {
    const ps = cat.projects.filter((p) => p.collectionId === c.id && matches(p, meta[p.id], query));
    if (query && !ps.length) continue;
    out.push({ collection: c.id, projects: ps });
  }
  return out;
}

export function columnsFor(width, settings) {
  const minW = settings.tile_size === "compact" ? 250 : 320;
  const fit = Math.max(1, Math.floor(width / minW));
  return Math.max(1, Math.min(Math.max(1, Math.min(6, settings.columns || 3)), fit));
}

/**
 * Visual rows: {kind: "header", collection, count} | {kind: "tiles", ids: [...]} | {kind: "empty"}.
 * Selectable items: headers and tiles, each with its row index and column.
 */
export function buildRows(cat, meta, query, cols) {
  const secs = sections(cat, meta, query);
  const onlyUngrouped = secs.length === 1 && secs[0].collection === null;
  const rows = [];
  const items = [];
  for (const s of secs) {
    if (!onlyUngrouped) {
      items.push({ kind: "header", collection: s.collection, row: rows.length, col: 0 });
      rows.push({ kind: "header", collection: s.collection, count: s.projects.length });
    }
    const coll = cat.collections.find((c) => c.id === s.collection);
    const collapsed = !query && coll?.collapsed;
    if (collapsed) continue;
    if (!s.projects.length && !onlyUngrouped) {
      rows.push({ kind: "empty" });
      continue;
    }
    for (let i = 0; i < s.projects.length; i += cols) {
      const ids = s.projects.slice(i, i + cols).map((p) => p.id);
      ids.forEach((id, col) => items.push({ kind: "tile", id, row: rows.length, col }));
      rows.push({ kind: "tiles", ids });
    }
  }
  return { rows, items };
}

/** Arrow navigation over items; returns the new index. */
export function nav(items, at, dx, dy) {
  if (!items.length) return 0;
  const cur = items[Math.min(at, items.length - 1)];
  if (dx) return Math.max(0, Math.min(items.length - 1, at + dx));
  const rowsWith = [...new Set(items.map((i) => i.row))].sort((a, b) => a - b);
  const ri = rowsWith.indexOf(cur.row) + dy;
  if (ri < 0 || ri >= rowsWith.length) return at;
  const target = items.map((it, i) => [it, i]).filter(([it]) => it.row === rowsWith[ri]);
  const best = target.reduce((b, x) => (Math.abs(x[0].col - cur.col) < Math.abs(b[0].col - cur.col) ? x : b));
  return best[1];
}

// ---------------------------------------------------------------- herdr presence

/** The catalog project containing `cwd` (on `machine`): in its folder or one of its linked
 *  worktrees (`extra`: {projectId: [paths]}); the deepest match wins. */
export function projectForCwd(cat, cwd, extra = {}, machine = null) {
  let best = null;
  let depth = -1;
  for (const p of cat.projects) {
    if (machineOf(p) !== (machine || null)) continue;
    for (const root of [p.path, ...(extra[p.id] ?? [])]) {
      const d = norm(root, machine).length;
      if (within(cwd, root, machine) && d > depth) {
        best = p;
        depth = d;
      }
    }
  }
  return best;
}

/** Metadata belongs to the project's current folder on its machine (an old cache entry has no
 *  path; entries without a machine were made on this computer). */
export function metaFits(m, p) {
  if (!m || !p || (m.machine ?? null) !== machineOf(p)) return false;
  return !m.path || norm(m.path, machineOf(p)) === norm(p.path, machineOf(p));
}

/** {id: meta} for the catalog, only entries that still fit their project. */
export function fittingMeta(cat, meta) {
  const out = {};
  for (const p of cat.projects) if (metaFits(meta[p.id], p)) out[p.id] = meta[p.id];
  return out;
}

/** {projectId: [linked worktree paths]} from the metadata. */
export function worktreeRoots(meta) {
  const out = {};
  for (const [id, m] of Object.entries(meta ?? {})) if (m?.git?.worktrees?.length) out[id] = m.git.worktrees;
  return out;
}

/**
 * Per project id: {spaces: [{id, index, name}], working, blocked, done}.
 * spaces: SpaceInfo[], panes: PaneInfo[]. A pane counts for projects on its space's machine.
 */
export function presence(cat, spaces, panes, extra = {}) {
  const spaceOfTab = new Map();
  for (const s of spaces) for (const t of s.tabs ?? []) spaceOfTab.set(t.id, s);
  const out = {};
  const get = (id) => (out[id] ??= { spaces: [], working: 0, blocked: 0, done: 0 });
  for (const pane of panes) {
    if (!pane.cwd) continue;
    const s = spaceOfTab.get(pane.tab);
    if (!s) continue;
    const p = projectForCwd(cat, pane.cwd, extra, s.machine);
    if (!p) continue;
    const e = get(p.id);
    if (!e.spaces.some((x) => x.id === s.id)) e.spaces.push({ id: s.id, index: s.index, name: s.name, active: s.active });
    if (pane.status === "working") e.working++;
    else if (pane.status === "blocked") e.blocked++;
    else if (pane.status === "done") e.done++;
  }
  for (const e of Object.values(out)) e.spaces.sort((a, b) => a.index - b.index);
  return out;
}

/** The machine of the space a tab is in (null: this one). */
export function machineOfTab(spaces, tab) {
  for (const s of spaces) if ((s.tabs ?? []).some((t) => t.id === tab)) return s.machine || null;
  return null;
}

/** Folders of open terminals for the import: [{path, machine}], no duplicates. */
export function openFolders(spaces, panes) {
  const byTab = new Map();
  for (const s of spaces) for (const t of s.tabs ?? []) byTab.set(t.id, s.machine || null);
  const seen = new Set();
  const out = [];
  for (const p of panes) {
    if (!p.cwd || !byTab.has(p.tab)) continue;
    const machine = byTab.get(p.tab);
    const k = `${machine ?? ""}\0${p.cwd}`;
    if (seen.has(k)) continue;
    seen.add(k);
    out.push({ path: p.cwd, machine });
  }
  return out;
}

/** "1 project", "5 projects". */
export function projectsLabel(n) {
  return `${n} project${n === 1 ? "" : "s"}`;
}

/** Git status pills, as Project Tiles shows them: behind / ahead counts, else up to date
 *  (after a fetch) or "no fetch". tone: orange | blue | green | dim. */
export function gitPills(g) {
  if (!g) return [];
  if (!g.upstream) return [{ icon: null, text: "no upstream", tone: "dim" }];
  const out = [];
  if (g.behind) out.push({ icon: "down", text: String(g.behind), tone: "orange" });
  if (g.ahead) out.push({ icon: "up", text: String(g.ahead), tone: "blue" });
  if (!out.length) out.push(g.fetched ? { icon: "check", text: "up to date", tone: "green" } : { icon: "check", text: "no fetch", tone: "dim" });
  return out;
}

/** 91.94 → "91.9%". */
export function pct(p) {
  return `${(Math.round((p ?? 0) * 10) / 10).toFixed(1)}%`;
}

/** git's relative date without "ago" ("2 days") → "2 days ago". */
export function ago(d) {
  return d ? `${d} ago` : "";
}

// ================================================================ backend (was the Rust binary)
//
// Everything below runs programs only through an injected `run(argv, {cwd, timeout})`, which
// main.js maps to herdr's exec("/usr/bin/env", argv, {cwd, on, timeout}) and node tests map to
// child_process. It resolves {code, stdout, stderr}; a failed start or a timeout is a code, not
// a throw.

/** Hardening for read-only calls: no index lock, no fsmonitor or hooks from the repository. */
export const RO_ARGS = ["--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null", "-c", "core.quotepath=off"];
/** fetch / pull / push the user asked for run like their own git (their hooks apply). */
export const RW_ARGS = ["-c", "core.fsmonitor=false", "-c", "core.quotepath=off"];
/** Set through /usr/bin/env. GIT_SSH_COMMAND is never set: it would override a per-folder
 *  core.sshCommand (profile keys). */
export const GIT_ENV = ["GIT_TERMINAL_PROMPT=0", "GIT_OPTIONAL_LOCKS=0", "LC_ALL=C"];
export const QUICK = 5000;
export const FETCH = 20000;
export const REMOTE = 120000;

// herdr's exec stops only the process it started when its timeout hits: git's ssh child would
// keep running. For network calls git runs under this perl helper instead: the child gets its own
// process group (set by both sides, so there is no race) and stdin from /dev/null; at the deadline,
// or when the helper itself gets TERM/INT/HUP, the whole group (git, ssh, credential helpers) is
// killed. Exit 124 on timeout, 125 when it cannot fork, 127 when the program cannot start, else
// git's code (128 + signal when it died of one).
export const GROUP_KILL = 'my $t=shift; my $p=fork; defined $p or do { print STDERR "fork: $!\\n"; exit 125 }; ' +
  'if(!$p){ setpgrp(0,0); open STDIN, "<", "/dev/null"; exec {$ARGV[0]} @ARGV or do { print STDERR "$ARGV[0]: $!\\n"; exit 127 } } ' +
  'setpgrp($p,$p); my $stop=sub{ kill "KILL", -$p; kill "KILL", $p; waitpid($p,0); print STDERR "$_[0]\\n"; exit $_[1] }; ' +
  '$SIG{ALRM}=sub{ $stop->("timeout after ${t}s", 124) }; $SIG{$_}=sub{ $stop->("stopped", 143) } for qw(TERM INT HUP); ' +
  'alarm $t; my $w; do { $w=waitpid($p,0) } while ($w == -1 && $!{EINTR}); alarm 0; ' +
  'exit($? & 127 ? 128 + ($? & 127) : $? >> 8)';

/** argv for /usr/bin/env. `deadline` (ms): run under GROUP_KILL with that deadline. */
export function gitArgv(args, { ro = true, deadline = 0 } = {}) {
  const git = ["git", ...(ro ? RO_ARGS : RW_ARGS), ...args];
  if (!deadline) return [...GIT_ENV, ...git];
  return [...GIT_ENV, "perl", "-e", GROUP_KILL, String(Math.max(1, Math.ceil(deadline / 1000))), ...git];
}

/** Whether the machine has perl, asked once per machine (`state.perl`) with a harmless call. Only
 *  a clear answer is remembered: env's 127 (not found) or perl's own exit 0. */
export async function hasPerl(run, state = {}) {
  if (typeof state.perl === "boolean") return state.perl;
  const r = await run(["perl", "-e", "exit 0"], { timeout: QUICK });
  if (r.code === 0 || r.code === 127) state.perl = r.code === 0;
  // Unclear (timeout, machine unreachable): assume perl, the git call fails the same way anyway.
  return r.code !== 127;
}

/**
 * Runs git, once. Network calls (`deadline`) go through GROUP_KILL; exec's own timeout is a few
 * seconds later as a backstop. On a machine without perl git runs directly and only exec's timeout
 * applies (ssh may outlive it).
 */
export async function runGit(run, cwd, args, { ro = true, deadline = 0, timeout = QUICK, state = {} } = {}) {
  if (deadline && (await hasPerl(run, state))) return run(gitArgv(args, { ro, deadline }), { cwd, timeout: deadline + 5000 });
  return run(gitArgv(args, { ro }), { cwd, timeout: deadline || timeout });
}

/** The last meaningful line of git's message. */
export function gitMessage(r) {
  const src = (r.stderr ?? "").trim() ? r.stderr : r.stdout ?? "";
  const lines = src.split("\n").map((l) => l.trim()).filter(Boolean);
  return lines.length ? lines[lines.length - 1] : `exit ${r.code}`;
}

export const LOG_FORMAT = "--format=%h%x1f%s%x1f%cr%x1f%an";

/** `git log -1 LOG_FORMAT` → {hash, subject, date ("2 days"), author}. */
export function parseCommit(line) {
  const parts = (line ?? "").trim().split("\x1f");
  const hash = (parts[0] ?? "").trim();
  if (!hash) return null;
  return { hash, subject: parts[1] ?? "", date: (parts[2] ?? "").trim().replace(/ ago$/, ""), author: parts[3] ?? "" };
}

/** `git rev-list --left-right --count HEAD...@{upstream}` → [ahead, behind]. */
export function parseAheadBehind(s) {
  const w = (s ?? "").trim().split(/\s+/);
  if (w.length !== 2 || !w.every((x) => /^\d+$/.test(x))) return null;
  return [Number(w[0]), Number(w[1])];
}

/** `git worktree list --porcelain [-z]` → linked checkouts (the first entry is the main one).
 *  With -z, fields end in NUL and paths are verbatim (spaces, newlines kept). */
export function parseWorktrees(s, z = false) {
  return (s ?? "").split(z ? "\0" : "\n").filter((l) => l.startsWith("worktree ")).slice(1)
    .map((l) => l.slice("worktree ".length)).filter(Boolean);
}

/** `git status --porcelain` → [changed, untracked]. */
export function parsePorcelain(s) {
  let dirty = 0;
  let untracked = 0;
  for (const l of (s ?? "").split("\n")) {
    if (!l) continue;
    if (l.startsWith("??")) untracked++;
    else dirty++;
  }
  return [dirty, untracked];
}

/** Prints the arguments that are folders, each followed by NUL. */
const DIRS_SH = 'for d do [ -d "$d" ] && printf "%s\\0" "$d"; done; exit 0';

/** The project folder is the top level of a repository's working tree (not a subfolder of one,
 *  not a bare repository). */
export async function isRepo(run, path) {
  const r = await runGit(run, path, ["rev-parse", "--is-inside-work-tree", "--show-prefix"]);
  const lines = r.stdout.split("\n");
  return r.code === 0 && lines[0] === "true" && (lines[1] ?? "") === "";
}

/**
 * Local git state, after a quiet `git fetch` when `fetch`. null when not a repository.
 * `state`: per-machine flags for runGit.
 */
export async function gitInfo(run, path, fetch = false, state = {}) {
  if (!(await isRepo(run, path))) return null;
  const git = (args) => runGit(run, path, args, { state });
  let fetched = false;
  if (fetch) fetched = (await runGit(run, path, ["fetch", "--quiet", "--no-tags"], { deadline: FETCH, state })).code === 0;
  const g = { branch: "", detached: false, upstream: null, ahead: 0, behind: 0, dirty: 0, untracked: 0, fetched, commit: null, remote_url: null, worktrees: [] };
  let o = await git(["symbolic-ref", "--short", "-q", "HEAD"]);
  if (o.code === 0 && o.stdout.trim()) g.branch = o.stdout.trim();
  else {
    o = await git(["rev-parse", "--short", "HEAD"]);
    g.branch = o.code === 0 ? `(${o.stdout.trim()})` : "(no commits)";
    g.detached = true;
  }
  o = await git(["log", "-1", LOG_FORMAT]);
  if (o.code === 0) g.commit = parseCommit(o.stdout);
  o = await git(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]);
  if (o.code === 0 && o.stdout.trim()) {
    g.upstream = o.stdout.trim();
    const ab = parseAheadBehind((await git(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])).stdout);
    if (ab) [g.ahead, g.behind] = ab;
  }
  o = await git(["status", "--porcelain", "--untracked-files=normal"]);
  if (o.code === 0) [g.dirty, g.untracked] = parsePorcelain(o.stdout);
  // The web links need the remote's address: the branch's remote (its name may contain `/`),
  // else origin. Local only.
  let remote = "origin";
  if (g.upstream && !g.detached) {
    o = await git(["config", "--get", `branch.${g.branch}.remote`]);
    if (o.code === 0 && o.stdout.trim()) remote = o.stdout.trim();
    g.upstream_remote = remote;
  }
  o = await git(["remote", "get-url", "--", remote]);
  if (o.code === 0 && o.stdout.trim()) g.remote_url = o.stdout.trim();
  // -z (git 2.36+) keeps unusual paths verbatim; older git gets the line format.
  let z = true;
  o = await git(["worktree", "list", "--porcelain", "-z"]);
  if (o.code !== 0) {
    z = false;
    o = await git(["worktree", "list", "--porcelain"]);
  }
  const wts = o.code === 0 ? parseWorktrees(o.stdout, z) : [];
  if (wts.length) {
    // Stale entries (folder gone, not yet pruned) are left out.
    const d = await run(["sh", "-c", DIRS_SH, "sh", ...wts], { timeout: QUICK });
    const ok = new Set(d.code === 0 ? d.stdout.split("\0").filter(Boolean) : []);
    g.worktrees = wts.filter((w) => ok.has(w));
  }
  return g;
}

export const REMOTE_OPS = {
  fetch: ["fetch", "--quiet"],
  pull: ["pull", "--ff-only", "--quiet"],
  push: ["push", "--quiet"],
};

/** A fetch / pull / push the user asked for, with the 120s deadline. */
export function remoteOp(run, path, op, state = {}) {
  return runGit(run, path, REMOTE_OPS[op], { ro: false, deadline: REMOTE, state });
}

// ---------------------------------------------------------------- web links

/** Percent-encodes all but A-Z a-z 0-9 - . _ ~ / (no TextEncoder in Herdr's runtime). */
function encodePath(s) {
  let e;
  try {
    e = encodeURIComponent(s);
  } catch {
    return "";
  }
  return e.replace(/%2F/g, "/").replace(/[!*'()]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
}

/**
 * `git@host:group/repo.git`, `ssh://git@host:22/group/repo.git` or
 * `https://user@host/group/repo.git` → `https://host/group/repo`. null for local paths, ssh
 * aliases (a host without a dot) and anything odd.
 */
export function webUrl(remote) {
  const r = (remote ?? "").trim();
  // eslint-disable-next-line no-control-regex
  if (/[\s\x00-\x1f\x7f]/.test(r)) return null;
  const splitOnce = (s, ch) => {
    const i = s.indexOf(ch);
    return i < 0 ? null : [s.slice(0, i), s.slice(i + 1)];
  };
  const afterAt = (s) => {
    const i = s.lastIndexOf("@");
    return i < 0 ? s : s.slice(i + 1);
  };
  let host;
  let port = "";
  let path;
  const http = r.startsWith("https://") ? r.slice(8) : r.startsWith("http://") ? r.slice(7) : null;
  const ssh = r.startsWith("ssh://") ? r.slice(6) : r.startsWith("git+ssh://") ? r.slice(10) : null;
  if (http !== null) {
    // Userinfo is only stripped from the authority (before the first `/`), never from the path.
    const ap = splitOnce(http, "/");
    if (!ap) return null;
    const hp = afterAt(ap[0]);
    const hpp = splitOnce(hp, ":");
    [host, port] = hpp ?? [hp, ""];
    path = ap[1];
  } else if (ssh !== null) {
    const ap = splitOnce(ssh, "/");
    if (!ap) return null;
    // The ssh port is not the web port.
    host = afterAt(ap[0]).split(":")[0];
    path = ap[1];
  } else if (!r.includes("://")) {
    // scp-like `[user@]host:path`; git treats it as a local path when a `/` comes first.
    const hp = splitOnce(r, ":");
    if (!hp || hp[0].includes("/")) return null;
    host = afterAt(hp[0]);
    path = hp[1];
  } else return null;
  const hostOk = Boolean(host) && /^[A-Za-z0-9.-]+$/.test(host) && !/^[.-]/.test(host) && host.includes(".");
  if (!hostOk || !/^\d*$/.test(port)) return null;
  path = path.replace(/^\/+|\/+$/g, "").replace(/(\.git)+$/, "").replace(/^\/+|\/+$/g, "");
  const segs = path.split("/");
  if (!path || segs.some((s) => !s || s === "." || s === "..")) return null;
  return `https://${host}${port ? ":" + port : ""}/${segs.map(encodePath).join("/")}`;
}

/** {repo, branch, commit} web pages: GitHub and Bitbucket path styles, GitLab's for the rest. */
export function links(g) {
  const repo = g?.remote_url ? webUrl(g.remote_url) : null;
  if (!repo) return { repo: null, branch: null, commit: null };
  const host = repo.slice(8).split("/")[0].toLowerCase();
  const [tree, commit] = host === "github.com" ? ["/tree/", "/commit/"] : host === "bitbucket.org" ? ["/src/", "/commits/"] : ["/-/tree/", "/-/commit/"];
  // The branch as the remote knows it: the upstream's name without its remote (whose name may
  // contain `/`). A local branch tracking another name (refactor/x → origin/main) is not on the
  // remote as itself.
  const rn = g.upstream_remote;
  const i = g.upstream ? (rn && g.upstream.startsWith(rn + "/") ? rn.length : g.upstream.indexOf("/")) : -1;
  const remoteBranch = i >= 0 ? g.upstream.slice(i + 1) : null;
  const branch = remoteBranch !== null && !g.detached && remoteBranch === g.branch ? `${repo}${tree}${encodePath(remoteBranch)}` : null;
  return { repo, branch, commit: g.commit ? `${repo}${commit}${g.commit.hash}` : null };
}

// ---------------------------------------------------------------- languages

/** extension → [language, colour from github/linguist]. */
export const BY_EXT = {
  py: ["Python", "#3572A5"], pyi: ["Python", "#3572A5"], ipynb: ["Jupyter", "#DA5B0B"],
  js: ["JavaScript", "#f1e05a"], mjs: ["JavaScript", "#f1e05a"], cjs: ["JavaScript", "#f1e05a"], jsx: ["JavaScript", "#f1e05a"],
  ts: ["TypeScript", "#3178c6"], tsx: ["TypeScript", "#3178c6"],
  go: ["Go", "#00ADD8"], rs: ["Rust", "#dea584"], java: ["Java", "#b07219"],
  kt: ["Kotlin", "#A97BFF"], kts: ["Kotlin", "#A97BFF"], scala: ["Scala", "#c22d40"],
  c: ["C", "#555555"], h: ["C", "#555555"], cc: ["C++", "#f34b7d"], cpp: ["C++", "#f34b7d"], hpp: ["C++", "#f34b7d"],
  cs: ["C#", "#178600"], swift: ["Swift", "#F05138"], m: ["Objective-C", "#438eff"],
  rb: ["Ruby", "#701516"], php: ["PHP", "#4F5D95"], lua: ["Lua", "#000080"],
  sh: ["Shell", "#89e051"], bash: ["Shell", "#89e051"], zsh: ["Shell", "#89e051"], fish: ["Shell", "#89e051"],
  ps1: ["PowerShell", "#012456"], sql: ["SQL", "#e38c00"],
  html: ["HTML", "#e34c26"], htm: ["HTML", "#e34c26"], css: ["CSS", "#663399"], scss: ["SCSS", "#c6538c"],
  vue: ["Vue", "#41b883"], svelte: ["Svelte", "#ff3e00"], dart: ["Dart", "#00B4AB"],
  ex: ["Elixir", "#6e4a7e"], exs: ["Elixir", "#6e4a7e"], erl: ["Erlang", "#B83998"], hs: ["Haskell", "#5e5086"],
  ml: ["OCaml", "#ef7a08"], clj: ["Clojure", "#db5855"], zig: ["Zig", "#ec915c"], nim: ["Nim", "#ffc200"],
  r: ["R", "#198CE7"], jl: ["Julia", "#a270ba"], tf: ["HCL", "#844FBA"], hcl: ["HCL", "#844FBA"],
  nix: ["Nix", "#7e7eff"], vim: ["Vim Script", "#199f4b"], el: ["Emacs Lisp", "#c065db"],
  md: ["Markdown", "#083fa1"], mdx: ["MDX", "#fcb32c"], toml: ["TOML", "#9c4221"],
  yml: ["YAML", "#cb171e"], yaml: ["YAML", "#cb171e"], j2: ["Jinja", "#a52a22"], jinja: ["Jinja", "#a52a22"],
  proto: ["Protobuf", "#a0a0a0"], gd: ["GDScript", "#355570"], shader: ["ShaderLab", "#222c37"],
  glsl: ["GLSL", "#5686a5"], mk: ["Makefile", "#427819"],
};
export const BY_NAME = {
  Makefile: ["Makefile", "#427819"],
  Dockerfile: ["Dockerfile", "#384d54"],
  justfile: ["Just", "#384d54"],
  Justfile: ["Just", "#384d54"],
};
/** Markup and config, not counted as languages (as in Project Tiles). */
export const MARKUP = ["Markdown", "YAML", "TOML", "MDX"];

export function languageFor(file) {
  if (BY_NAME[file]) return BY_NAME[file][0];
  const lower = file.toLowerCase();
  if (lower.startsWith("dockerfile")) return "Dockerfile";
  const i = lower.lastIndexOf(".");
  if (i < 0) return null;
  return BY_EXT[lower.slice(i + 1)]?.[0] ?? null;
}

export function colorOf(lang) {
  const l = (lang ?? "").toLowerCase();
  for (const [name, color] of [...Object.values(BY_EXT), ...Object.values(BY_NAME)]) if (name.toLowerCase() === l) return color;
  return "#8b8b8b";
}

/** {language: bytes} → [{name, pct, color}] sorted by size; at most 8, each ≥ 0.5 %. */
export function shares(sizes) {
  const use = Object.entries(sizes).filter(([k]) => !MARKUP.includes(k));
  const total = use.reduce((s, [, v]) => s + v, 0);
  if (!total) return [];
  return use
    .map(([name, v]) => ({ name, pct: (v * 100) / total }))
    .sort((a, b) => b.pct - a.pct || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .filter((x) => x.pct >= 0.5)
    .slice(0, 8)
    .map((x) => ({ ...x, color: colorOf(x.name) }));
}

/** Manual languages: equal shares. */
export function fromNames(names) {
  if (!names?.length) return [];
  return names.map((name) => ({ name, pct: 100 / names.length, color: colorOf(name) }));
}

/** Sums sizes per language over [size, relative path] entries. Paths under a hidden or ignored
 *  folder are skipped; at most `max` files are looked at. */
export function languageSizes(entries, ignore, max) {
  const skip = new Set(ignore ?? []);
  const sizes = {};
  let seen = 0;
  for (const [size, rel] of entries) {
    const parts = rel.split("/");
    const name = parts.pop();
    if (parts.some((d) => d.startsWith(".") || skip.has(d))) continue;
    if (++seen > max) break;
    const lang = languageFor(name);
    if (lang) sizes[lang] = (sizes[lang] ?? 0) + Math.max(1, size);
  }
  return sizes;
}

export const LS_FORMAT = "--format=%(objectmode) %(objectsize)%x09%(path)";

/** `git ls-files -z LS_FORMAT` → [[size, path]] of regular files (symlinks and submodules left
 *  out). A record cut off without its NUL (output over exec's limit) is dropped. */
export function parseLsFiles(s) {
  const out = [];
  const recs = (s ?? "").split("\0");
  recs.pop();
  for (const rec of recs) {
    const m = /^(\d+) (\d+)\t(.+)$/s.exec(rec);
    if (!m || m[1] === "120000" || m[1] === "160000") continue;
    out.push([Number(m[2]), m[3]]);
  }
  return out;
}

/** `wc -c` lines for files under `root` → [[size, path relative to root]] (totals dropped). */
export function parseWc(s, root) {
  const base = root.replace(/\/+$/, "") + "/";
  const out = [];
  for (const line of (s ?? "").split("\n")) {
    const m = /^\s*(\d+)\s+(.+)$/.exec(line);
    if (!m || !m[2].startsWith(base)) continue;
    out.push([Number(m[1]), m[2].slice(base.length)]);
  }
  return out;
}

/** find over the folder (hidden and ignored folders pruned), first `max` files, their sizes.
 *  Ignored names match exactly (glob characters escaped); files whose name has a newline are left
 *  out (the list between find and wc is line-based). */
export function findArgv(root, ignore, max) {
  const exact = (n) => n.replace(/[[\]*?\\]/g, "\\$&");
  const names = [".*", ...(ignore ?? []).map(exact)].flatMap((n, i) => (i ? ["-o", "-name", n] : ["-name", n]));
  const script = 'm=$1; shift; find "$@" 2>/dev/null | head -n "$m" | tr "\\n" "\\0" | xargs -0 wc -c 2>/dev/null; exit 0';
  return ["sh", "-c", script, "sh", String(max), root, "-mindepth", "1", "(", "-type", "d", "(", ...names, ")", "-prune", ")", "-o", "-type", "f", "!", "-name", "*\n*", "-print"];
}

/** Language shares: tracked files of a repository (`git ls-files`, sizes from the index), or a
 *  bounded `find` for a folder without git. */
export async function scanLanguages(run, path, repo, settings, state = {}) {
  const ignore = settings.language_ignore_dirs ?? DEFAULT_SETTINGS.language_ignore_dirs;
  const max = settings.max_files_scanned || DEFAULT_SETTINGS.max_files_scanned;
  if (repo) {
    const r = await runGit(run, path, ["ls-files", "-z", LS_FORMAT], { timeout: 15000, state });
    if (r.code === 0) return shares(languageSizes(parseLsFiles(r.stdout), ignore, max));
    // git older than 2.38 has no --format, and a listing over exec's 8 MiB fails: fall back to
    // the bounded folder scan.
  }
  const r = await run(findArgv(path, ignore, max), { timeout: 30000 });
  return shares(languageSizes(parseWc(r.stdout, path), ignore, max));
}

// ---------------------------------------------------------------- availability, resolve

/** The volume a path lives on when it is a mount point that may be missing. */
export function volumeRoot(path) {
  const p = (path ?? "").split("/");
  if (p[0] !== "") return null;
  if (p[1] === "Volumes" && p[2]) return `/Volumes/${p[2]}`;
  if (p[1] === "media" && p[2] && p[3]) return `/media/${p[2]}/${p[3]}`;
  if (p[1] === "run" && p[2] === "media" && p[3] && p[4]) return `/run/media/${p[3]}/${p[4]}`;
  if (p[1] === "mnt" && p[2]) return `/mnt/${p[2]}`;
  return null;
}

const AVAIL_SH = 'if [ -d "$1" ]; then echo ok; elif [ -n "$2" ] && [ ! -e "$2" ]; then echo offline; else echo deleted; fi';

/** null when the folder is there; "offline" (its volume is not mounted, or the machine cannot be
 *  reached) or "deleted". */
export async function availability(run, path) {
  const r = await run(["sh", "-c", AVAIL_SH, "sh", path, volumeRoot(path) ?? ""], { timeout: QUICK });
  const w = r.stdout.trim();
  if (w === "ok") return null;
  if (w === "offline" || w === "deleted") return w;
  return "offline";
}

/** `~`, `~/x`, relative to `base`, with `.` and `..` resolved. */
export function resolvePath(raw, base, home) {
  const s = (raw ?? "").trim();
  let full;
  if (s === "~") full = home;
  else if (s.startsWith("~/")) full = home + s.slice(1);
  else if (s.startsWith("/")) full = s;
  else full = (base || home).replace(/\/+$/, "") + "/" + s;
  const out = [];
  for (const part of full.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") out.pop();
    else out.push(part);
  }
  return "/" + out.join("/");
}

/** $HOME on the machine `run` runs on (null when it cannot be asked). */
export async function machineHome(run) {
  const r = await run(["sh", "-c", 'printf %s "$HOME"'], { timeout: QUICK });
  const h = r.code === 0 ? r.stdout.trim() : "";
  return h.startsWith("/") ? h : null;
}

/** {ok, path, name} — the folder normalized to its git top level — or {ok: false, error}. */
export async function resolveFolder(run, raw, base, home) {
  const path = resolvePath(raw, base, home);
  if ((await availability(run, path)) !== null) return { ok: false, path, error: `no such folder: ${path}` };
  const r = await runGit(run, path, ["rev-parse", "--show-toplevel"], { timeout: 3000 });
  const root = r.code === 0 && r.stdout.trim() ? r.stdout.trim() : path;
  return { ok: true, path: root, name: baseName(root) };
}

// ---------------------------------------------------------------- metadata and its cache


/** Seconds since the epoch (the cache keeps the old binary's units). */
export function nowSecs() {
  return Date.now() / 1000;
}

/**
 * What a refresh has to do for a project: {git, lang, remote}. The local scan (`fetched_at`) and
 * the fetch (`remote_at`) have separate TTLs, so local rescans do not postpone the network
 * refresh; only repositories with an upstream (or not looked at yet) have anything to fetch.
 */
export function due(m, p, s, now, { force = false, fetch = false } = {}) {
  const moved = !m || m.path !== p.path || (m.machine ?? null) !== machineOf(p);
  const ttl = (s.metadata_cache_minutes ?? 10) * 60;
  const hasUpstream = !m?.git || Boolean(m.git.upstream);
  const remote = Boolean(fetch) && hasUpstream && (force || moved || now - (m?.remote_at ?? 0) > ttl);
  const git = force || moved || remote || now - (m?.fetched_at ?? 0) > ttl;
  const lang = force || moved || now - (m?.languages_at ?? 0) > (s.language_cache_hours ?? 12) * 3600;
  return { git, lang, remote };
}

/** The meta object the view reads, with links derived from git. */
export function withLinks(m) {
  return { ...m, links: m.git ? links(m.git) : null };
}

/**
 * Refreshes one project's metadata. `publish(meta)` is called after each phase (local git first
 * so the tile fills at once, then the fetch, then languages). Returns the final meta.
 */
export async function refreshProject(run, p, prev, s, d, publish = () => {}, state = {}) {
  const fresh = { git: null, git_checked: false, languages: [], languages_scanned: false, fetched_at: 0, remote_at: 0, languages_at: 0 };
  const m = { ...fresh, ...(metaFits(prev, p) ? prev : {}), path: p.path, machine: machineOf(p) };
  if (!metaFits(prev, p)) prev = null;
  const out = () => withLinks(m);
  const avail = await availability(run, p.path);
  m.checked_at = nowSecs();
  if (avail) {
    m.availability = avail;
    if (avail === "deleted") {
      m.git = null;
      m.languages = [];
    }
    publish(out());
    return out();
  }
  m.availability = null;
  const manual = Array.isArray(p.languages) ? p.languages.filter(Boolean) : [];
  if (manual.length) {
    m.languages = fromNames(manual);
    m.languages_scanned = true;
    m.languages_at = nowSecs();
  }
  const wantGit = d.git && s.show_git !== false && p.show?.git !== false;
  if (wantGit) {
    const g = await gitInfo(run, p.path, false, state);
    const before = prev?.git;
    if (g && g.upstream && before && g.upstream === before.upstream) g.fetched = Boolean(before.fetched);
    m.git = g;
    m.git_checked = true;
    m.fetched_at = nowSecs();
    publish(out());
    if (d.remote && g?.upstream) {
      const g2 = await gitInfo(run, p.path, true, state);
      if (g2) m.git = g2;
      m.fetched_at = nowSecs();
      m.remote_at = m.fetched_at;
      publish(out());
    }
  }
  if (d.lang && s.show_languages !== false && !manual.length) {
    const repo = m.git ? true : await isRepo(run, p.path);
    m.languages = await scanLanguages(run, p.path, repo, s, state);
    m.languages_scanned = true;
    m.languages_at = nowSecs();
  }
  publish(out());
  return out();
}

/** After fetch / pull / push: the new local state; `fetched` when the op worked (or it already
 *  was, for the same upstream). */
export async function afterRemoteOp(run, p, prev, ok, state = {}) {
  if (!metaFits(prev, p)) prev = null;
  const m = { ...(prev ?? {}), path: p.path, machine: machineOf(p), availability: null, checked_at: nowSecs() };
  const g = await gitInfo(run, p.path, false, state);
  if (g) g.fetched = ok || Boolean(prev?.git?.fetched && prev.git.upstream === g.upstream);
  m.git = g;
  m.git_checked = true;
  m.fetched_at = nowSecs();
  if (ok) m.remote_at = m.fetched_at;
  return withLinks(m);
}

export const CACHE_VERSION = 3;

const str = (v) => (typeof v === "string" ? v : null);
const num = (v) => (typeof v === "number" && Number.isFinite(v) ? v : 0);
const strs = (v) => (Array.isArray(v) ? v.filter((x) => typeof x === "string") : []);

/** A git record from the cache with every field of the expected type, or null. */
function cleanGit(g) {
  if (!g || typeof g !== "object" || Array.isArray(g)) return null;
  const c = g.commit && typeof g.commit === "object" && typeof g.commit.hash === "string"
    ? { hash: g.commit.hash, subject: str(g.commit.subject) ?? "", date: str(g.commit.date) ?? "", author: str(g.commit.author) ?? "" }
    : null;
  const out = {
    branch: str(g.branch) ?? "", detached: g.detached === true, upstream: str(g.upstream),
    ahead: num(g.ahead), behind: num(g.behind), dirty: num(g.dirty), untracked: num(g.untracked),
    fetched: g.fetched === true, commit: c, remote_url: str(g.remote_url), worktrees: strs(g.worktrees),
  };
  if (str(g.upstream_remote)) out.upstream_remote = g.upstream_remote;
  return out;
}

/** A cache entry with the expected shape (bad fields dropped or reset), or null. */
export function cleanMeta(m) {
  if (!m || typeof m !== "object" || Array.isArray(m)) return null;
  const languages = (Array.isArray(m.languages) ? m.languages : []).flatMap((l) => {
    const [name, pct] = Array.isArray(l) ? l : [l?.name, l?.pct];
    if (typeof name !== "string" || typeof pct !== "number" || !Number.isFinite(pct)) return [];
    return [{ name, pct, color: !Array.isArray(l) && /^#[0-9a-fA-F]{6}$/.test(l.color ?? "") ? l.color : colorOf(name) }];
  });
  const out = {
    git: cleanGit(m.git), git_checked: m.git_checked === true, languages, languages_scanned: m.languages_scanned === true,
    fetched_at: num(m.fetched_at), remote_at: num(m.remote_at), languages_at: num(m.languages_at), checked_at: num(m.checked_at),
    availability: m.availability === "offline" || m.availability === "deleted" ? m.availability : null,
  };
  if (str(m.path)) out.path = m.path;
  if (str(m.machine)) out.machine = m.machine;
  return out;
}

/** Cache text → {id: meta}. Reads this format and the old binary's v2 (languages as pairs). An
 *  entry that does not have the expected shape is left out (it is scanned again). */
export function parseCache(text) {
  let d;
  try {
    d = JSON.parse(text ?? "");
  } catch {
    return {};
  }
  if (!d || typeof d !== "object" || (d.version !== CACHE_VERSION && d.version !== 2)) return {};
  if (!d.projects || typeof d.projects !== "object") return {};
  const out = {};
  for (const [id, m] of Object.entries(d.projects)) {
    const c = cleanMeta(m);
    if (c) out[id] = withLinks(c);
  }
  return out;
}

/** {id: meta} → cache text (links are derived again on read). */
export function cacheText(projects) {
  const slim = {};
  for (const [id, m] of Object.entries(projects)) {
    const { links: _l, ...rest } = m;
    slim[id] = rest;
  }
  return JSON.stringify({ version: CACHE_VERSION, projects: slim }) + "\n";
}

/**
 * Another copy's cache and ours, for the projects of `cat`: entries that do not fit their project
 * (moved, other machine, removed) are dropped, and each phase comes from whichever side did it
 * last — git (`fetched_at`, with `remote_at`), languages (`languages_at`), availability
 * (`checked_at`) — so two copies that refreshed different phases both keep their work.
 */
export function mergeCache(theirs, ours, cat) {
  const out = {};
  for (const p of cat.projects) {
    const a = metaFits(theirs[p.id], p) ? theirs[p.id] : null;
    const b = metaFits(ours[p.id], p) ? ours[p.id] : null;
    if (!a || !b) {
      if (a || b) out[p.id] = a || b;
      continue;
    }
    const newer = (k) => ((b[k] ?? 0) >= (a[k] ?? 0) ? b : a);
    const g = newer("fetched_at");
    const l = newer("languages_at");
    const v = newer("checked_at");
    const m = {
      ...g,
      remote_at: Math.max(a.remote_at ?? 0, b.remote_at ?? 0),
      languages: l.languages, languages_scanned: l.languages_scanned, languages_at: l.languages_at,
      availability: v.availability ?? null, checked_at: v.checked_at,
    };
    out[p.id] = withLinks(m);
  }
  return out;
}

/** Runs async tasks at most `n` at a time. */
export function makePool(n) {
  let active = 0;
  const queue = [];
  const next = () => {
    if (active >= n || !queue.length) return;
    active++;
    const { task, resolve, reject } = queue.shift();
    Promise.resolve().then(task).then(resolve, reject).finally(() => {
      active--;
      next();
    });
  };
  return (task) => new Promise((resolve, reject) => {
    queue.push({ task, resolve, reject });
    next();
  });
}

// ---------------------------------------------------------------- import

/** Where the VS Code Project Tiles extension keeps its catalog. */
export function vscodeCatalogPaths(home) {
  return [`${home}/Library/Application Support/project-tiles/projects.json`, `${home}/.config/project-tiles/projects.json`];
}

/**
 * Merges the VS Code Project Tiles catalog and open folders into `cat` (in place); returns how
 * many projects were added or regrouped. `roots`: [{path, machine}] already normalized to git top
 * levels.
 */
export function importInto(cat, vscode, roots, home) {
  let added = 0;
  if (vscode) {
    const idmap = {};
    for (const c of vscode.collections ?? []) {
      const name = (c.name ?? "").trim();
      if (!name) continue;
      let mine = collectionByName(cat, name);
      if (!mine) {
        // A new collection keeps the source's colour, or none (like Project Tiles).
        mine = { id: newId(), name, collapsed: false };
        if (c.color) mine.color = c.color;
        cat.collections.push(mine);
      }
      if (!mine.description && c.description) mine.description = c.description;
      if (!mine.color && c.color) mine.color = c.color;
      idmap[c.id] = mine.id;
    }
    for (const sp of vscode.projects ?? []) {
      if (!sp.path || sp.path.toLowerCase().endsWith(".code-workspace")) continue;
      const target = sp.collectionId ? idmap[sp.collectionId] ?? null : null;
      const have = findPath(cat, sp.path);
      if (have) {
        // Already listed: only adopt its VS Code collection when it has none here.
        if (target && !isGrouped(cat, have)) {
          have.collectionId = target;
          added++;
        }
        continue;
      }
      const p = { id: newId(), name: sp.name || baseName(sp.path), path: sp.path };
      for (const k of ["description", "color", "languages", "show"]) if (sp[k] !== undefined && sp[k] !== null) p[k] = sp[k];
      const e = emojiOf(sp);
      p.logo = e ? { kind: "emoji", value: e } : { kind: "auto" };
      if (target) p.collectionId = target;
      cat.projects.push(p);
      added++;
    }
  }
  const seen = new Set();
  for (const { path, machine = null } of roots) {
    const k = `${machine ?? ""}\0${norm(path, machine)}`;
    if ((!machine && norm(path) === norm(home)) || norm(path, machine) === "/" || seen.has(k) || findPath(cat, path, machine)) continue;
    seen.add(k);
    const p = { id: newId(), name: baseName(path), path, logo: { kind: "auto" } };
    if (machine) p.machine = machine;
    cat.projects.push(p);
    added++;
  }
  return added;
}
