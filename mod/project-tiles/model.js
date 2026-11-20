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

export function norm(p) {
  let s = (p ?? "").replace(/\/+$/, "");
  if (!s) s = "/";
  return s.toLowerCase();
}

/** `child` is `parent` or inside it. */
export function within(child, parent) {
  if (!child || !parent) return false;
  const c = norm(child);
  const p = norm(parent);
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

export function findPath(cat, path) {
  const k = norm(path);
  return cat.projects.find((p) => norm(p.path) === k) ?? null;
}

export function collectionByName(cat, name) {
  const n = (name ?? "").trim().toLowerCase();
  return cat.collections.find((c) => (c.name ?? "").trim().toLowerCase() === n) ?? null;
}

export function isGrouped(cat, p) {
  return Boolean(p.collectionId && cat.collections.some((c) => c.id === p.collectionId));
}

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

/** The catalog project containing `cwd`: in its folder or one of its linked worktrees
 *  (`extra`: {projectId: [paths]}); the deepest match wins. */
export function projectForCwd(cat, cwd, extra = {}) {
  let best = null;
  let depth = -1;
  for (const p of cat.projects) {
    for (const root of [p.path, ...(extra[p.id] ?? [])]) {
      const d = norm(root).length;
      if (within(cwd, root) && d > depth) {
        best = p;
        depth = d;
      }
    }
  }
  return best;
}

/** {projectId: [linked worktree paths]} from the metadata. */
export function worktreeRoots(meta) {
  const out = {};
  for (const [id, m] of Object.entries(meta ?? {})) if (m?.git?.worktrees?.length) out[id] = m.git.worktrees;
  return out;
}

/**
 * Per project id: {spaces: [{id, index, name}], working, blocked, done}.
 * spaces: SpaceInfo[] (local ones), panes: PaneInfo[].
 */
export function presence(cat, spaces, panes, extra = {}) {
  const spaceOfTab = new Map();
  for (const s of spaces) for (const t of s.tabs ?? []) spaceOfTab.set(t.id, s);
  const out = {};
  const get = (id) => (out[id] ??= { spaces: [], working: 0, blocked: 0, done: 0 });
  for (const pane of panes) {
    if (!pane.cwd) continue;
    const s = spaceOfTab.get(pane.tab);
    if (!s || s.machine) continue;
    const p = projectForCwd(cat, pane.cwd, extra);
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

/** Folders of open local terminals for the import (no duplicates). */
export function openFolders(spaces, panes) {
  const local = new Set();
  for (const s of spaces) if (!s.machine) for (const t of s.tabs ?? []) local.add(t.id);
  const seen = new Set();
  const out = [];
  for (const p of panes) {
    if (!p.cwd || !local.has(p.tab) || seen.has(p.cwd)) continue;
    seen.add(p.cwd);
    out.push(p.cwd);
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
