// node mod/test.mjs — tests for the mod's pure logic.
import assert from "node:assert/strict";
import * as m from "./project-tiles/model.js";

const t = (name, fn) => {
  try {
    fn();
    console.log("ok  ", name);
  } catch (e) {
    console.log("FAIL", name);
    throw e;
  }
};

t("initials", () => {
  assert.equal(m.initials("MyGameStudio"), "MGS");
  assert.equal(m.initials("shop-backend"), "SB");
  assert.equal(m.initials("Tools"), "TO");
  assert.equal(m.initials("проект"), "ПР");
  assert.equal(m.initials("a b c d"), "ABC");
  assert.equal(m.initials("---"), "?");
  assert.equal(m.initials("api2Server"), "AS");
});

t("graphemes", () => {
  assert.equal(m.clampGraphemes("🇷🇺🦀👍🏽x", 3), "🇷🇺🦀👍🏽");
  assert.equal(m.clampGraphemes("👩‍💻👩‍💻👩‍💻👩‍💻", 3), "👩‍💻👩‍💻👩‍💻");
  assert.equal(m.badge({ name: "x", logo: { kind: "emoji", value: "🦀" } }), "🦀");
  assert.equal(m.badge({ name: "shop-backend", logo: { kind: "auto" } }), "SB");
});

t("paths", () => {
  assert.equal(m.shortPath("/Users/me/x", "/Users/me"), "~/x");
  assert.equal(m.shortPath("/Users/mex", "/Users/me"), "/Users/mex");
  assert.equal(m.expandPath(" ~/a ", "/h"), "/h/a");
  assert(m.within("/a/B/c", "/a/b/"));
  assert(!m.within("/a/bc", "/a/b"));
});

const theme = { ansi1: "#ff0000", ansi2: "#00ff00", ansi3: "#ff8800", ansi4: "#0000ff", ansi5: "#aa00aa", ansi6: "#00aaaa", ansi11: "#ffff00", dim: "#777777" };

t("colours", () => {
  assert.equal(m.parseColor("Orange", theme), "#ff8800");
  assert.equal(m.parseColor("#ABC", theme), "#aabbcc");
  assert.equal(m.parseColor("123456", theme), "#123456");
  assert.equal(m.parseColor("nope", theme), null);
  assert.equal(m.pickColor("x", theme), m.pickColor("x", theme));
});

const catText = JSON.stringify({
  version: 1,
  collections: [
    { id: "w", name: "Work", color: "#83a598" },
    { id: "o", name: "Old", collapsed: true },
  ],
  projects: [
    { id: "a", name: "shop", path: "/srv/shop", collectionId: "w", future: 1 },
    { id: "b", name: "crabs", path: "/srv/shop/crabs", collectionId: "w", languages: ["Rust"] },
    { id: "c", name: "loose", path: "/srv/loose" },
    { id: "d", name: "legacy", path: "/srv/legacy", collectionId: "o" },
    { name: "", path: "/srv/noname" },
    { id: "bad" },
  ],
});

t("catalog parse and round trip", () => {
  const c = m.parseCatalog(catText);
  assert.equal(c.projects.length, 5);
  assert.equal(c.projects[4].name, "noname");
  assert.equal(c.projects[4].id.length, 36);
  assert.equal(c.projects[0].future, 1);
  const back = m.parseCatalog(m.catalogText(c));
  assert.deepEqual(back, c);
  assert.equal(m.findPath(c, "/SRV/shop/").id, "a");
  assert.equal(m.collectionByName(c, " work ").id, "w");
  assert.throws(() => m.parseCatalog("{ nope"));
});

t("sections, rows, nav", () => {
  const c = m.parseCatalog(catText);
  const meta = { a: { git: { branch: "main" } } };
  const { rows, items } = m.buildRows(c, meta, "", 2);
  assert.deepEqual(rows.map((r) => r.kind), ["header", "tiles", "header", "tiles", "header"]);
  assert.equal(rows[0].count, 2); // ungrouped: loose, noname
  assert.equal(items[0].kind, "header");
  assert.equal(items.filter((i) => i.kind === "tile").length, 4); // legacy is collapsed
  // From the first tile down: the Work header, then shop.
  let at = 1;
  at = m.nav(items, at, 0, 1);
  assert.equal(items[at].kind, "header");
  at = m.nav(items, at, 0, 1);
  assert.equal(items[at].id, "a");
  at = m.nav(items, at, 1, 0);
  assert.equal(items[at].id, "b");
  // Search: by branch and by language; empty collections hidden.
  const s = m.buildRows(c, meta, "main", 3);
  assert.deepEqual(s.items.filter((i) => i.kind === "tile").map((i) => i.id), ["a"]);
  assert.equal(m.buildRows(c, meta, "rust", 3).items.filter((i) => i.kind === "tile")[0].id, "b");
  assert.equal(m.columnsFor(1000, { columns: 3, tile_size: "normal" }), 3);
  assert.equal(m.columnsFor(500, { columns: 3, tile_size: "normal" }), 1);
  assert.equal(m.columnsFor(600, { columns: 6, tile_size: "compact" }), 2);
});

t("presence", () => {
  const c = m.parseCatalog(catText);
  const spaces = [
    { id: "s1", index: 1, name: "shop", machine: null, tabs: [{ id: "t1" }] },
    { id: "s2", index: 2, name: "crabs", machine: null, tabs: [{ id: "t2" }] },
    { id: "s3", index: 3, name: "vps", machine: "vps", tabs: [{ id: "t3" }] },
  ];
  const panes = [
    { tab: "t1", cwd: "/srv/shop/src", status: "working" },
    { tab: "t1", cwd: "/srv/shop", status: "blocked" },
    { tab: "t2", cwd: "/srv/shop/crabs/x", status: "done" },
    { tab: "t3", cwd: "/srv/shop", status: "working" },
    { tab: "t1", cwd: null, mod: "files" },
  ];
  const pr = m.presence(c, spaces, panes);
  assert.deepEqual(pr.a.spaces.map((s) => s.index), [1]);
  assert.equal(pr.a.working, 1);
  assert.equal(pr.a.blocked, 1);
  assert.equal(pr.b.done, 1);
  assert.deepEqual(m.openFolders(spaces, panes), ["/srv/shop/src", "/srv/shop", "/srv/shop/crabs/x"]);
  assert.equal(m.projectForCwd(c, "/srv/shop/crabs/q").id, "b");
});

t("labels and git badges", () => {
  assert.equal(m.projectsLabel(1), "1 project");
  assert.equal(m.projectsLabel(0), "0 projects");
  assert.equal(m.projectsLabel(22), "22 projects");
  const pills = (g) => m.gitPills(g).map((x) => `${x.icon ?? ""}:${x.text}:${x.tone}`);
  assert.deepEqual(pills({ upstream: "o/m", ahead: 1, behind: 7, fetched: false }), ["down:7:orange", "up:1:blue"]);
  assert.deepEqual(pills({ upstream: "o/m", fetched: true }), ["check:up to date:green"]);
  assert.deepEqual(pills({ upstream: "o/m", fetched: false }), ["check:no fetch:dim"]);
  assert.deepEqual(pills({}), [":no upstream:dim"]);
  assert.deepEqual(m.gitPills(null), []);
  assert.equal(m.pct(91.94), "91.9%");
  assert.equal(m.pct(0.5), "0.5%");
  assert.equal(m.ago("2 days"), "2 days ago");
});

t("shown", () => {
  assert.equal(m.shown({ show: { git: false } }, "git", m.DEFAULT_SETTINGS), false);
  assert.equal(m.shown({}, "git", m.DEFAULT_SETTINGS), true);
});

console.log("all passed");
