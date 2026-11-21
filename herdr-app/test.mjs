// node herdr-app/test.mjs — tests for the mod's pure logic.
import assert from "node:assert/strict";
// Herdr's runtime has none of these: model.js must not need them.
for (const g of ["TextEncoder", "TextDecoder", "Buffer", "setTimeout", "setInterval", "structuredClone"]) {
  globalThis[`__${g}`] = globalThis[g];
  delete globalThis[g];
}
const m = await import("./mods/project-tiles/model.js");

const t = async (name, fn) => {
  try {
    await fn();
    console.log("ok  ", name);
  } catch (e) {
    console.log("FAIL", name);
    throw e;
  }
};

await t("initials", () => {
  assert.equal(m.initials("MyGameStudio"), "MGS");
  assert.equal(m.initials("shop-backend"), "SB");
  assert.equal(m.initials("Tools"), "TO");
  assert.equal(m.initials("проект"), "ПР");
  assert.equal(m.initials("a b c d"), "ABC");
  assert.equal(m.initials("---"), "?");
  assert.equal(m.initials("api2Server"), "AS");
});

await t("graphemes", () => {
  assert.equal(m.clampGraphemes("🇷🇺🦀👍🏽x", 3), "🇷🇺🦀👍🏽");
  assert.equal(m.clampGraphemes("👩‍💻👩‍💻👩‍💻👩‍💻", 3), "👩‍💻👩‍💻👩‍💻");
  assert.equal(m.badge({ name: "x", logo: { kind: "emoji", value: "🦀" } }), "🦀");
  assert.equal(m.badge({ name: "shop-backend", logo: { kind: "auto" } }), "SB");
});

await t("paths", () => {
  assert.equal(m.shortPath("/Users/me/x", "/Users/me"), "~/x");
  assert.equal(m.shortPath("/Users/mex", "/Users/me"), "/Users/mex");
  assert.equal(m.expandPath(" ~/a ", "/h"), "/h/a");
  assert(m.within("/a/B/c", "/a/b/"));
  assert(!m.within("/a/bc", "/a/b"));
  assert(m.within("/tmp/wt/src", "/private/tmp/wt"));
  assert(m.within("/private/var/x", "/var"));
  assert(!m.within("/privatex/tmp", "/tmp"));
  assert.equal(m.norm("/private/tmpx"), "/private/tmpx");
});

const theme = { ansi1: "#ff0000", ansi2: "#00ff00", ansi3: "#ff8800", ansi4: "#0000ff", ansi5: "#aa00aa", ansi6: "#00aaaa", ansi11: "#ffff00", dim: "#777777" };

await t("colours", () => {
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

await t("catalog parse and round trip", () => {
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

await t("sections, rows, nav", () => {
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

await t("presence", () => {
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
  assert.deepEqual(m.openFolders(spaces, panes), [
    { path: "/srv/shop/src", machine: null },
    { path: "/srv/shop", machine: null },
    { path: "/srv/shop/crabs/x", machine: null },
    { path: "/srv/shop", machine: "vps" },
  ]);
  assert.equal(m.machineOfTab(spaces, "t3"), "vps");
  assert.equal(m.machineOfTab(spaces, "t1"), null);
  // A project on another machine counts only for that machine's panes.
  const rc = m.parseCatalog(JSON.stringify({ projects: [{ id: "r", name: "shop", path: "/srv/shop", machine: "vps" }] }));
  assert.deepEqual(m.presence(rc, spaces, panes).r.spaces.map((s) => s.id), ["s3"]);
  assert.equal(m.findPath(rc, "/srv/shop"), null);
  assert.equal(m.findPath(rc, "/srv/shop", "vps").id, "r");
  assert.equal(m.projectForCwd(c, "/srv/shop/crabs/q").id, "b");
  // A linked worktree outside the project folder counts for its project.
  const extra = m.worktreeRoots({ c: { git: { worktrees: ["/wt/loose/fix-x"] } }, a: { git: {} } });
  assert.deepEqual(extra, { c: ["/wt/loose/fix-x"] });
  assert.equal(m.projectForCwd(c, "/wt/loose/fix-x/src", extra).id, "c");
  const pr2 = m.presence(c, [{ id: "s9", index: 9, name: "fix-x", machine: null, tabs: [{ id: "t9" }] }],
    [{ tab: "t9", cwd: "/wt/loose/fix-x", status: "working" }], extra);
  assert.deepEqual(pr2.c.spaces.map((s) => s.index), [9]);
  assert.equal(pr2.c.working, 1);
});

await t("labels and git badges", () => {
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

await t("metadata follows the folder", () => {
  const p = { id: "a", path: "/new/repo" };
  assert.equal(m.metaFits(undefined, p), false);
  assert.equal(m.metaFits({ path: "/old/repo", git: { branch: "x" } }, p), false);
  assert.equal(m.metaFits({ path: "/new/repo/" }, p), true);
  assert.equal(m.metaFits({ git: {} }, p), true); // old cache without a path
  // Another machine's metadata never fits, even for the same path.
  assert.equal(m.metaFits({ path: "/new/repo" }, { ...p, machine: "vps" }), false);
  assert.equal(m.metaFits({ path: "/new/repo", machine: "vps" }, { ...p, machine: "vps" }), true);
  assert.equal(m.metaFits({ path: "/new/repo", machine: "vps" }, p), false);
  // macOS rules (case, /private) only on this computer.
  assert.equal(m.metaFits({ path: "/New/Repo" }, p), true);
  assert.equal(m.metaFits({ path: "/New/Repo", machine: "vps" }, { path: "/new/repo", machine: "vps" }), false);
  const cat = { collections: [], projects: [{ id: "u", path: "/srv/App", machine: "vps" }, { id: "l", path: "/srv/app", machine: "vps" }] };
  assert.equal(m.findPath(cat, "/srv/app", "vps").id, "l");
  assert.equal(m.projectForCwd(cat, "/srv/App/src", {}, "vps").id, "u");
  assert.deepEqual(Object.keys(m.fittingMeta(cat, { u: { path: "/srv/App", machine: "vps" }, l: { path: "/srv/App", machine: "vps" } })), ["u"]);
});

await t("shown", () => {
  assert.equal(m.shown({ show: { git: false } }, "git", m.DEFAULT_SETTINGS), false);
  assert.equal(m.shown({}, "git", m.DEFAULT_SETTINGS), true);
});


// ---------------------------------------------------------------- backend (ported from the Rust tests)

await t("git parsing", () => {
  const c = m.parseCommit("a1b2c3d\x1ffix: x\x1f2 days ago\x1fMe\n");
  assert.deepEqual(c, { hash: "a1b2c3d", subject: "fix: x", date: "2 days", author: "Me" });
  assert.equal(m.parseCommit(""), null);
  assert.deepEqual(m.parseAheadBehind("3\t1\n"), [3, 1]);
  assert.equal(m.parseAheadBehind("x 1"), null);
  assert.equal(m.parseAheadBehind("1"), null);
  assert.deepEqual(m.parsePorcelain(" M a\n?? b\nA  c\n?? d\n"), [2, 2]);
  assert.deepEqual(m.parsePorcelain(""), [0, 0]);
  const wt = "worktree /r/main\nHEAD abc\nbranch refs/heads/main\n\nworktree /w/r/fix-x\nHEAD def\nbranch refs/heads/fix/x\n\nworktree /w/r/det\nHEAD 123\ndetached\n";
  assert.deepEqual(m.parseWorktrees(wt), ["/w/r/fix-x", "/w/r/det"]);
  assert.deepEqual(m.parseWorktrees("worktree /only/main\nHEAD a\n"), []);
  assert.deepEqual(m.parseWorktrees("worktree /r\0HEAD a\0\0worktree /w/odd \nname \0HEAD b\0\0", true), ["/w/odd \nname "]);
  assert.equal(m.gitMessage({ code: 1, stdout: "", stderr: "hint: x\nfatal: no upstream\n\n" }), "fatal: no upstream");
  assert.equal(m.gitMessage({ code: 2, stdout: "", stderr: "" }), "exit 2");
});

await t("git argv: env, hardening, group kill", () => {
  const ro = m.gitArgv(["status"]);
  assert.deepEqual(ro.slice(0, 4), ["GIT_TERMINAL_PROMPT=0", "GIT_OPTIONAL_LOCKS=0", "LC_ALL=C", "git"]);
  for (const f of ["--no-optional-locks", "core.fsmonitor=false", "core.hooksPath=/dev/null", "core.quotepath=off"]) assert(ro.includes(f), f);
  assert(!ro.join(" ").includes("GIT_SSH_COMMAND"));
  const rw = m.gitArgv(["push", "--quiet"], { ro: false, deadline: 120000 });
  assert.equal(rw[3], "perl");
  assert.equal(rw[6], "120");
  assert(!rw.includes("core.hooksPath=/dev/null"), "the user's hooks apply to push");
  assert(!rw.join(" ").includes("sshCommand"));
});

await t("runGit: perl asked once per machine, git never run twice", async () => {
  const calls = [];
  const fake = (perl, gitCode = 0) => async (argv) => {
    calls.push(argv);
    if (argv[0] === "perl") return perl;
    return { code: gitCode, stdout: "ok", stderr: gitCode === 127 ? "perl: not found" : "" };
  };
  const missing = { code: 127, stdout: "", stderr: "env: perl: No such file or directory" };
  let state = {};
  const r = await m.runGit(fake(missing), "/x", ["fetch"], { deadline: 20000, state });
  assert.equal(r.stdout, "ok");
  assert.equal(state.perl, false);
  await m.runGit(fake(missing), "/x", ["fetch"], { deadline: 20000, state });
  assert.equal(calls.filter((a) => a[0] === "perl").length, 1, "the probe is not repeated");
  assert(!calls.some((a) => a.includes("-e") && a.includes(m.GROUP_KILL)), "no group kill without perl");
  // With perl, a git (or hook) failure that looks like "127 … perl … not found" is returned as is.
  calls.length = 0;
  state = {};
  const r2 = await m.runGit(fake({ code: 0, stdout: "", stderr: "" }, 127), "/x", ["push"], { ro: false, deadline: 120000, state });
  assert.equal(r2.code, 127);
  assert.equal(calls.filter((a) => a.includes(m.GROUP_KILL)).length, 1, "git ran once, under the group kill");
  // An unclear probe (machine unreachable) is not remembered.
  state = {};
  await m.runGit(fake({ code: 124, stdout: "", stderr: "" }), "/x", ["fetch"], { deadline: 20000, state });
  assert.equal(state.perl, undefined);
});

await t("web links", () => {
  const eq = (a, b) => assert.equal(m.webUrl(a), b, a);
  eq("git@gitlab.contoso.io:data/warehouse.git", "https://gitlab.contoso.io/data/warehouse");
  eq("ssh://git@gitlab.example.com:2222/a/b/c.git", "https://gitlab.example.com/a/b/c");
  eq("https://user:tok@github.com/o/r.git", "https://github.com/o/r");
  eq("git@bitbucket.org:contoso/x.git", "https://bitbucket.org/contoso/x");
  eq("/srv/git/repo.git", null);
  eq("file:///srv/repo", null);
  // `@` in the path does not move the host.
  eq("https://github.com/owner/repo@evil.example/x.git", "https://github.com/owner/repo%40evil.example/x");
  eq("https://git.example.com:8443/a/b.git", "https://git.example.com:8443/a/b");
  for (const bad of ["javascript:alert(1)", "https://h/a/../b", "https://h/a b", "https:///a", "https://-h/a", "https://h:x/a",
    "https://h\x07/a", "./rel/path:x", "git@h:", "github-personal:o/r.git", "https://h.example/a/../b", "git@h.example:a/./b"]) eq(bad, null);
  eq("https://10.0.0.5/a/b", "https://10.0.0.5/a/b");
  eq("git@h.example:grp/проект.git", "https://h.example/grp/%D0%BF%D1%80%D0%BE%D0%B5%D0%BA%D1%82");
  const g = {
    branch: "feat/TASK-1 x",
    upstream: "origin/feat/TASK-1 x",
    remote_url: "git@gitlab.contoso.io:data/dbt.git",
    commit: { hash: "abc1234" },
  };
  let l = m.links(g);
  assert.equal(l.branch, "https://gitlab.contoso.io/data/dbt/-/tree/feat/TASK-1%20x");
  assert.equal(l.commit, "https://gitlab.contoso.io/data/dbt/-/commit/abc1234");
  g.remote_url = "git@github.com:o/r.git";
  assert.equal(m.links(g).branch, "https://github.com/o/r/tree/feat/TASK-1%20x");
  g.remote_url = "git@bitbucket.org:o/r.git";
  assert.equal(m.links(g).commit, "https://bitbucket.org/o/r/commits/abc1234");
  g.upstream = "origin/main"; // tracks another branch: no branch page
  assert.equal(m.links(g).branch, null);
  g.upstream = null; // not pushed: no branch page, the repo still opens
  l = m.links(g);
  assert.equal(l.branch, null);
  assert.equal(l.repo, "https://bitbucket.org/o/r");
  g.upstream = "origin/feat/TASK-1 x";
  g.detached = true;
  assert.equal(m.links(g).branch, null);
  // A remote whose name has a slash.
  g.detached = false;
  g.branch = "main";
  g.upstream = "team/up/main";
  g.upstream_remote = "team/up";
  g.remote_url = "git@github.com:o/r.git";
  assert.equal(m.links(g).branch, "https://github.com/o/r/tree/main");
  g.remote_url = null;
  assert.deepEqual(m.links(g), { repo: null, branch: null, commit: null });
});

await t("languages: lookup and shares", () => {
  assert.equal(m.languageFor("Makefile"), "Makefile");
  assert.equal(m.languageFor("x.TSX"), "TypeScript");
  assert.equal(m.languageFor("Dockerfile.dev"), "Dockerfile");
  assert.equal(m.languageFor("noext"), null);
  assert.equal(m.colorOf("rust"), "#dea584");
  assert.equal(m.fromNames(["A", "B"])[1].pct, 50);
  const entries = [[300, "src/main.rs"], [100, "run.sh"], [5000, "README.md"], [9000, "node_modules/x/a.js"], [9000, ".hidden/b.go"], [0, "Dockerfile"]];
  const s = m.shares(m.languageSizes(entries, ["node_modules"], 1000));
  assert.equal(s[0].name, "Rust");
  assert(Math.abs(s[0].pct - (300 / 401) * 100) < 0.01);
  assert.equal(s[0].color, "#dea584");
  assert.equal(s[1].name, "Shell");
  assert(s.every((l) => !["Markdown", "JavaScript", "Go"].includes(l.name)));
  assert.deepEqual(m.shares(m.languageSizes([[1, "docs/a.md"]], [], 10)), []);
  assert(m.shares(m.languageSizes(entries, ["node_modules"], 1)).length <= 1);
  // At most 8, each ≥ 0.5 %.
  const many = Object.fromEntries(["Go", "Rust", "C", "Lua", "Ruby", "PHP", "Zig", "Nim", "Dart"].map((l, i) => [l, 100 + i]));
  assert.equal(m.shares({ ...many, Shell: 1 }).length, 8);
  assert.deepEqual(m.parseLsFiles("100644 12\ta b.rs\x00100755 34\tsrc/x.py\x00"), [[12, "a b.rs"], [34, "src/x.py"]]);
  // Symlinks and submodules are not files; a record cut off without its NUL is dropped.
  assert.deepEqual(m.parseLsFiles("120000 9\tlink.rs\x00160000 -\tsub.go\x00100644 5\tnl\nname.py\x00100644 7\tcut.r"), [[5, "nl\nname.py"]]);
  assert.deepEqual(m.parseWc("     12 /r/a.rs\n      5 /r/s/b.py\n     17 total\n", "/r"), [[12, "a.rs"], [5, "s/b.py"]]);
});

await t("availability volumes and paths", () => {
  assert.equal(m.volumeRoot("/Volumes/Disk/x"), "/Volumes/Disk");
  assert.equal(m.volumeRoot("/run/media/me/USB/a"), "/run/media/me/USB");
  assert.equal(m.volumeRoot("/mnt/data"), "/mnt/data");
  assert.equal(m.volumeRoot("/Users/me/x"), null);
  assert.equal(m.resolvePath("~/a/../b/./c/", "/base", "/h"), "/h/b/c");
  assert.equal(m.resolvePath("x/y", "/base", "/h"), "/base/x/y");
  assert.equal(m.resolvePath("/a/../../b", "/base", "/h"), "/b");
  assert.equal(m.resolvePath("~", "/base", "/h"), "/h");
});

await t("due: local rescan does not postpone fetch", () => {
  const s = { ...m.DEFAULT_SETTINGS };
  const p = { id: "a", path: "/r" };
  const now = 1_000_000;
  const fresh = { path: "/r", git: { upstream: "origin/main", fetched: true }, fetched_at: now, remote_at: now - 86400, languages_at: now };
  assert.deepEqual(m.due(fresh, p, s, now, { fetch: false }), { git: false, lang: false, remote: false });
  assert.deepEqual(m.due(fresh, p, s, now, { fetch: true }), { git: true, lang: false, remote: true });
  // No upstream: nothing to fetch.
  assert.equal(m.due({ ...fresh, git: { upstream: null } }, p, s, now, { fetch: true }).remote, false);
  // Moved folder or nothing known: everything.
  assert.deepEqual(m.due({ ...fresh, path: "/old" }, p, s, now), { git: true, lang: true, remote: false });
  assert.deepEqual(m.due(undefined, p, s, now, { fetch: true }), { git: true, lang: true, remote: true });
  assert.deepEqual(m.due(fresh, p, s, now, { force: true }), { git: true, lang: true, remote: false });
  assert.equal(m.due({ ...fresh, languages_at: now - 13 * 3600 }, p, s, now).lang, true);
});

await t("cache: old format, round trip, merge", () => {
  const v2 = JSON.stringify({ version: 2, projects: { a: { path: "/r", languages: [["Rust", 90], ["Shell", 10]], git: { branch: "main", upstream: "origin/main", remote_url: "git@github.com:o/r.git", commit: { hash: "abc" } }, fetched_at: 5 } } });
  const c = m.parseCache(v2);
  assert.deepEqual(c.a.languages[0], { name: "Rust", pct: 90, color: "#dea584" });
  assert.equal(c.a.links.branch, "https://github.com/o/r/tree/main");
  const back = m.parseCache(m.cacheText(c));
  assert.deepEqual(back, c);
  assert(!m.cacheText(c).includes("links"));
  assert.deepEqual(m.parseCache("{bad"), {});
  assert.deepEqual(m.parseCache(JSON.stringify({ version: 1, projects: { a: {} } })), {});
  // Wrong shapes never throw: bad fields are reset, bad entries dropped.
  const bad = m.parseCache(JSON.stringify({ version: 3, projects: {
    a: { path: "/r", languages: {}, git: { branch: 5, worktrees: "x", commit: "y" }, fetched_at: "z" },
    b: [1], c: null, d: { languages: [["Go"], { name: "Rust", pct: 2, color: "javascript:" }, 7] },
  } }));
  assert.deepEqual(Object.keys(bad), ["a", "d"]);
  assert.deepEqual(bad.a.languages, []);
  assert.equal(bad.a.git.branch, "");
  assert.deepEqual(bad.a.git.worktrees, []);
  assert.equal(bad.a.git.commit, null);
  assert.equal(bad.a.fetched_at, 0);
  assert.deepEqual(bad.d.languages, [{ name: "Rust", pct: 2, color: "#dea584" }]);
  assert.deepEqual(m.parseCache(JSON.stringify({ version: 3, projects: 5 })), {});

  // Merge: per phase, newest wins; entries that do not fit (moved, other machine, removed) go.
  const cat = { collections: [], projects: [{ id: "a", path: "/a" }, { id: "b", path: "/b" }, { id: "c", path: "/c", machine: "vps" }] };
  const gitA = { branch: "new" };
  const theirs = {
    a: { path: "/a", git: gitA, fetched_at: 20, languages: [], languages_at: 1, checked_at: 20 },
    b: { path: "/old-b", fetched_at: 99 },
    c: { path: "/c", fetched_at: 99 },
    gone: { path: "/x", fetched_at: 99 },
  };
  const ours = {
    a: { path: "/a", git: { branch: "old" }, fetched_at: 10, languages: [{ name: "Go", pct: 100 }], languages_at: 21, checked_at: 21, availability: null },
    c: { path: "/c", machine: "vps", fetched_at: 1 },
  };
  const merged = m.mergeCache(theirs, ours, cat);
  assert.deepEqual(Object.keys(merged).sort(), ["a", "c"]);
  assert.equal(merged.a.git.branch, "new", "their newer git");
  assert.equal(merged.a.languages[0].name, "Go", "our newer languages");
  assert.equal(merged.c.machine, "vps");
});

await t("pool runs at most n at once", async () => {
  const pool = m.makePool(2);
  let active = 0;
  let peak = 0;
  const done = await Promise.all([1, 2, 3, 4, 5].map((i) => pool(async () => {
    active++;
    peak = Math.max(peak, active);
    await new Promise((r) => setImmediate(r));
    active--;
    return i;
  })));
  assert.deepEqual(done, [1, 2, 3, 4, 5]);
  assert.equal(peak, 2);
  await assert.rejects(pool(async () => { throw new Error("x"); }));
});

await t("import merges VS Code and open folders", () => {
  const cat = m.emptyCatalog();
  cat.collections.push({ id: "mine", name: "work" });
  const src = m.parseCatalog(JSON.stringify({
    version: 1,
    collections: [{ id: "c1", name: "Work", color: "#ff0000", description: "d" }, { id: "c2", name: "Fun" }],
    projects: [
      { id: "x", name: "X", path: "/a/x", logo: { kind: "emoji", value: "🎮" }, collectionId: "c1" },
      { id: "y", name: "Y", path: "/a/y.code-workspace" },
      { id: "z", name: "Z", path: "/a/z", logo: { kind: "image", value: "z.png" }, collectionId: "c2" },
    ],
  }));
  const roots = [{ path: "/h/repo" }, { path: "/h/repo" }, { path: "/h" }, { path: "/" }, { path: "/h/repo", machine: "vps" }];
  assert.equal(m.importInto(cat, src, roots, "/h"), 4); // x, z, repo, repo on vps
  assert.equal(cat.collections.length, 2);
  assert.equal(cat.collections[0].color, "#ff0000");
  assert.equal(cat.collections[0].description, "d");
  // A new collection without a colour in the source stays without one.
  assert.equal(cat.collections[1].name, "Fun");
  assert.equal(cat.collections[1].color, undefined);
  const x = m.findPath(cat, "/a/x");
  assert.equal(m.emojiOf(x), "🎮");
  assert.equal(x.collectionId, "mine");
  assert.equal(m.findPath(cat, "/a/z").logo.kind, "auto");
  assert(m.findPath(cat, "/h/repo"));
  assert.equal(m.findPath(cat, "/h/repo", "vps").machine, "vps");
  // A project already listed without a collection adopts its VS Code collection.
  delete x.collectionId;
  assert.equal(m.importInto(cat, src, roots, "/h"), 1);
  assert.equal(x.collectionId, "mine");
  assert.equal(m.importInto(cat, src, roots, "/h"), 0);
  assert.equal(m.vscodeCatalogPaths("/h")[0], "/h/Library/Application Support/project-tiles/projects.json");
});

// ---------------------------------------------------------------- against real git (local, no network)

import { execFile } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, realpathSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";

// The user's own git config (signing, hooks, templates, URL rewrites) must not change the result,
// and nothing may reach the network: only file:// remotes.
Object.assign(process.env, { GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1", GIT_ALLOW_PROTOCOL: "file",
  GIT_AUTHOR_NAME: "T", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "T", GIT_COMMITTER_EMAIL: "t@t" });

/** herdr's exec, as main.js calls it: /usr/bin/env argv, cwd, timeout. */
const run = (argv, { cwd, timeout = 30000 } = {}) => new Promise((resolve) => {
  execFile("/usr/bin/env", argv, { cwd, timeout, maxBuffer: 8 << 20, encoding: "utf8" }, (err, stdout, stderr) => {
    resolve({ code: err ? (typeof err.code === "number" ? err.code : 124) : 0, stdout, stderr });
  });
});
/** Test setup: git with argv (no shell), failing loudly. */
const git = async (cwd, ...args) => {
  const r = await run(["git", ...args], { cwd });
  assert.equal(r.code, 0, `git ${args.join(" ")}: ${r.stderr}`);
  return r;
};

// A space in the folder name: nothing may depend on shell word splitting.
const tmp = realpathSync(mkdtempSync(`${tmpdir()}/pt test-`));
try {
  await t("git state of a local repository", async () => {
    const r = `${tmp}/repo`;
    mkdirSync(`${r}/sub`, { recursive: true });
    assert.equal(await m.gitInfo(run, r), null);
    await git(r, "init", "-q", "-b", "main");
    writeFileSync(`${r}/a.txt`, "x");
    let g = await m.gitInfo(run, r);
    assert.equal(g.branch, "main"); // unborn: symbolic-ref still names it
    assert.equal(g.commit, null);
    assert.equal(g.untracked, 1);
    writeFileSync(`${r}/main.rs`, "fn main() {}\n".repeat(20));
    writeFileSync(`${r}/sub/x.py`, "print(1)\n");
    await git(r, "add", "-A");
    await git(r, "commit", "-q", "-m", "first commit");
    writeFileSync(`${r}/main.rs`, "changed");
    writeFileSync(`${r}/new.txt`, "n");
    g = await m.gitInfo(run, r);
    assert.equal(g.branch, "main");
    assert.equal(g.dirty, 1);
    assert.equal(g.untracked, 1);
    assert.equal(g.upstream, null);
    assert.equal(g.commit.subject, "first commit");
    assert.equal(g.fetched, false);
    // A subfolder of a repository is not a project root.
    assert.equal(await m.gitInfo(run, `${r}/sub`), null);
    // Upstream, ahead/behind, worktrees, remote URL — all from a local "remote".
    await git(tmp, "clone", "-q", r, "clone");
    const c = `${tmp}/clone`;
    await git(c, "commit", "-q", "--allow-empty", "-m", "local");
    await git(r, "commit", "-q", "-am", "upstream");
    await git(c, "fetch", "-q");
    await git(c, "worktree", "add", "-q", `${tmp}/wt fix`, "-b", "fix");
    await git(c, "worktree", "add", "-q", `${tmp}/wt-gone`, "-b", "gone");
    rmSync(`${tmp}/wt-gone`, { recursive: true });
    g = await m.gitInfo(run, c, true);
    assert.equal(g.upstream, "origin/main");
    assert.equal(g.ahead, 1);
    assert.equal(g.behind, 1);
    assert.equal(g.fetched, true);
    assert.equal(g.remote_url, r);
    assert.deepEqual(g.worktrees, [`${tmp}/wt fix`], "a space kept, the stale one left out");
    const op = await m.remoteOp(run, c, "pull");
    assert.notEqual(op.code, 0, "diverged: --ff-only refuses");
    assert(m.gitMessage(op).length > 0);
    assert.equal((await m.resolveFolder(run, "repo/sub", tmp, "/h")).path, r);
    assert.equal((await m.resolveFolder(run, "nope", tmp, "/h")).ok, false);
    // A bare repository is not a working tree.
    await git(tmp, "init", "-q", "--bare", "bare.git");
    assert.equal(await m.isRepo(run, `${tmp}/bare.git`), false);
    assert.equal(await m.gitInfo(run, `${tmp}/bare.git`), null);
    // A remote whose name has a slash: its URL, not origin's.
    await git(c, "remote", "add", "team/up", r);
    await git(c, "fetch", "-q", "team/up");
    await git(c, "remote", "set-url", "team/up", "git@github.com:o/r.git");
    await git(c, "branch", "-q", "-u", "team/up/main");
    g = await m.gitInfo(run, c);
    assert.equal(g.upstream, "team/up/main");
    assert.equal(g.remote_url, "git@github.com:o/r.git");
    assert.equal(m.links(g).branch, "https://github.com/o/r/tree/main");
    await git(c, "branch", "-q", "-u", "origin/main");
  });

  await t("languages: git ls-files and the folder scan", async () => {
    const s = { ...m.DEFAULT_SETTINGS };
    const c = `${tmp}/clone`;
    // Symlinks and submodules are not counted.
    symlinkSync("main.rs", `${c}/link.go`);
    await git(c, "add", "link.go");
    await git(c, "-c", "protocol.file.allow=always", "submodule", "add", "-q", `${tmp}/repo`, "vendored.go");
    await git(c, "commit", "-q", "-m", "link and submodule");
    const g = await m.scanLanguages(run, c, true, s);
    assert.deepEqual(g.map((l) => l.name), ["Rust", "Python"]);
    const f = `${tmp}/plain`;
    mkdirSync(`${f}/node_modules/x`, { recursive: true });
    mkdirSync(`${f}/.hidden`, { recursive: true });
    mkdirSync(`${f}/src dir`, { recursive: true });
    mkdirSync(`${f}/b[ui]ld`, { recursive: true });
    mkdirSync(`${f}/bild`, { recursive: true });
    writeFileSync(`${f}/src dir/main.go`, "x".repeat(300));
    writeFileSync(`${f}/run.sh`, "x".repeat(100));
    writeFileSync(`${f}/README.md`, "x".repeat(5000));
    writeFileSync(`${f}/node_modules/x/a.js`, "x".repeat(9000));
    writeFileSync(`${f}/.hidden/b.rs`, "x".repeat(9000));
    writeFileSync(`${f}/b[ui]ld/c.rb`, "x".repeat(9000));
    writeFileSync(`${f}/bild/d.go`, "x".repeat(100));
    writeFileSync(`${f}/odd\nname.lua`, "x".repeat(9000));
    // "b[ui]ld" is ignored by name, not as a pattern that would also hide "bild".
    const p = await m.scanLanguages(run, f, false, { ...s, language_ignore_dirs: ["node_modules", "b[ui]ld"] });
    assert.deepEqual(p.map((l) => l.name), ["Go", "Shell"]);
    assert(Math.abs(p[0].pct - 80) < 0.01);
    assert((await m.scanLanguages(run, f, false, { ...s, max_files_scanned: 1 })).length <= 1);
  });

  await t("availability and refresh", async () => {
    assert.equal(await m.availability(run, tmp), null);
    assert.equal(await m.availability(run, `${tmp}/gone`), "deleted");
    assert.equal(await m.availability(run, "/Volumes/NoSuchDisk-pt/x"), "offline");
    const s = { ...m.DEFAULT_SETTINGS, fetch_remote: false };
    const p = { id: "c", path: `${tmp}/clone` };
    const phases = [];
    const meta = await m.refreshProject(run, p, null, s, m.due(null, p, s, m.nowSecs()), (x) => phases.push(x));
    assert(phases.length >= 2, "local git is published before languages");
    assert.equal(meta.git.branch, "main");
    assert(meta.git_checked && meta.languages_scanned && meta.checked_at > 0);
    assert.equal(meta.languages[0].name, "Rust");
    assert.equal(meta.links.repo, null, "a local remote has no web page");
    const gone = await m.refreshProject(run, { id: "g", path: `${tmp}/gone` }, meta, s, { git: true, lang: true });
    assert.equal(gone.availability, "deleted");
    assert.equal(gone.git, null);
    const after = await m.afterRemoteOp(run, p, meta, true);
    assert.equal(after.git.fetched, true);
    assert(after.remote_at > 0);
    // Metadata of the same path on another machine is not carried over.
    const other = await m.refreshProject(run, { ...p, machine: "vps" }, meta, s, { git: false, lang: false });
    assert.equal(other.machine, "vps");
    assert.equal(other.git, null);
    assert.equal(await m.machineHome(run), process.env.HOME);
    assert.equal(await m.hasPerl(run, {}), true);
  });

  await t("group kill stops git's children at the deadline", async () => {
    const started = Date.now();
    const pidFile = `${tmp}/child.pid`;
    const r = await run(["perl", "-e", m.GROUP_KILL, "1", "sh", "-c", 'sleep 30 & echo $! > "$1"; wait', "sh", pidFile]);
    assert.equal(r.code, 124);
    assert(Date.now() - started < 5000);
    const alive = await run(["sh", "-c", 'kill -0 "$(cat "$1")" 2>/dev/null && echo alive || echo dead', "sh", pidFile]);
    assert.equal(alive.stdout.trim(), "dead");
    // The child's stdin is /dev/null, its exit code comes through, a missing program is 127.
    assert.equal((await run(["perl", "-e", m.GROUP_KILL, "5", "sh", "-c", "cat; exit 3"])).code, 3);
    assert.equal((await run(["perl", "-e", m.GROUP_KILL, "5", "no-such-program-pt"])).code, 127);
    // TERM to the helper (exec stopping it) takes the group down too.
    const pid2 = `${tmp}/child2.pid`;
    const term = await run(["sh", "-c", 'perl -e "$1" 30 sh -c \'sleep 30 & echo $! > "$1"; wait\' sh "$2" & h=$!; sleep 1; kill -TERM $h; wait $h; echo $?',
      "sh", m.GROUP_KILL, pid2]);
    assert.equal(term.stdout.trim(), "143");
    const alive2 = await run(["sh", "-c", 'kill -0 "$(cat "$1")" 2>/dev/null && echo alive || echo dead', "sh", pid2]);
    assert.equal(alive2.stdout.trim(), "dead");
  });
} finally {
  rmSync(tmp, { recursive: true, force: true });
}

console.log("all passed");
