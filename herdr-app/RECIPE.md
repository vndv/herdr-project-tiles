# Project Tiles

A dashboard of your projects as tiles, opened as a dialog over the window (`cmd-alt-p`). Each
tile shows the project's logo (emoji or initials), name, path, description, a language bar and
its git state: branch, ahead/behind, changed files, last commit. Tiles are grouped into
collapsible coloured collections. It marks which projects are open in a space and where you
are, counts agents working, blocked and done in each project, and opens a project in one keypress.
It is a port of [Project Tiles for VS Code](https://github.com/InvisibleF0x/project-tiles) and shares its catalog format; plain JavaScript, no binary to install.

![Project Tiles](preview.png)

## What you see

- **Header**, 42px, on the dialog background with a bottom border:
  - "Projects" in 13px semibold;
  - a 300px search field (28px high, rounded 6, raised fill);
  - buttons on the right: Add current folder (accent fill), Add open folders…, Add folder…, New collection;
  - a refresh icon and "Esc".
- **Body** with a 14px side padding: collections top to bottom. Each one has:
  - a header row: a colour dot, its name, a chevron and a project count;
  - a grid of tiles under the header, 3 columns by default. Columns drop as the width falls below 320px per tile (250px compact). Gap 10px.
- **Tile**, rounded 8, raised card. From top to bottom:
  - **Head:**
    - a 38px logo square (30px compact, rounded 8): an emoji, or initials on the project colour;
    - the name, 13px semibold, with the path under it in muted text;
    - a pill on the right: `currently open` (the focused terminal is in this project) or `open · space N`.
  - **Description**, one line.
  - **Language bar:**
    - a 5px bar of language shares, in GitHub linguist colours;
    - a legend with the top three languages and their percentages.
  - **Git line:**
    - a branch pill (rounded 9, 18px high);
    - `● N` changed files in the working colour;
    - a worktree count (`⑂ N`) when the repo has linked worktrees;
    - on the right: `↑N` ahead (blue), `↓N` behind (orange), `✓ up to date` (green), or `no fetch` (dim) when the remote was never fetched.
  - **Commit line:** the short hash in the link colour, the subject, and "N minutes ago".
  - **Footer**, on a border line:
    - `Open`, or `Go to space` when it is already open (accent);
    - `New tab`;
    - a worktree button;
    - on the right, icon buttons: gitview (only when herdr-gitview is installed), fetch, pull with the behind count, push with the ahead count.
  - Buttons drop their labels on narrow tiles.
- **Status line** at the bottom, 11px muted:
  - on the left, a key hint;
  - on the right, the last message (git errors in red), or `here: <path>`.
- **Selection:** the selected tile gets a 2px accent border; a selected collection header gets an accent underline.
- **Unavailable projects:**
  - an unmounted volume (`/Volumes/...`) is shown dim and dashed (offline);
  - a deleted folder is shown in red.

## How it behaves

- **Agents:** counts come live from `panes()`:
  - working shows a spinner and the count, in the working colour;
  - blocked shows red `● N blocked`;
  - done shows teal `● N done`;
  - idle is not shown.
- **Keys:**
  - Arrows or `hjkl` move; mouse click selects; a double click opens.
  - **Opening:**
    - `Enter` selects the space showing the project (`space.select`), or creates one (`space.new {folder, name}`), then closes the dialog;
    - `o` always opens a new space;
    - `t` opens a new tab in the current space (`tab.new`).
  - **Worktrees:** `w` opens Herdr's worktree dialog for a new git worktree; `W` opens an existing one.
  - `g` opens gitview for the project, when herdr-gitview is installed.
  - **Search and adding:**
    - `/` searches name, path, description, languages and branch;
    - `a` adds the focused terminal's folder (its git root);
    - `A` adds a folder by path;
    - `I` imports open folders and the VS Code Project Tiles catalog.
  - **Catalog edits:**
    - `e` edits a project (name, path, description, colour, emoji, languages, collection) in a form;
    - `d` removes it from the list (the folder is not touched);
    - `m` moves it to a collection;
    - `n` creates a new collection;
    - `J`/`K` reorder;
    - `Space` collapses a collection.
  - **Git:**
    - `f`, `p`, `P` run git fetch, pull `--ff-only` and push in the background (errors become Herdr toasts);
    - `r`/`R` refresh one or all projects.
  - **Other:**
    - `O` reveals the project in Finder;
    - `c` switches between compact and normal tiles;
    - `E` edits `projects.json` in `$EDITOR` in a new tab;
    - `?` shows help;
    - `Esc` closes.
- **Clicks:**
  - the branch pill opens the branch on GitLab, GitHub or Bitbucket (the repository when the branch is not pushed under its own name);
  - the commit hash opens the commit;
  - the worktree count opens the worktree picker;
  - `● N` opens gitview.
- **Background refresh:**
  - git state refreshes on open, from cache first, then a local scan, then `git fetch` for repos with an upstream;
  - it refreshes again after `metadata_cache_minutes` (10) while the dialog stays open;
  - catalog edits made elsewhere show up within 2s.

## Built from

- `mods/project-tiles/`:
  - `mod.json`: `place: "none"`, `focusable: true`, overlay dialog 0.92 × 0.9.
  - `main.js`: the view, the catalog edits and the calls below.
  - `model.js`: pure logic, kept free of Herdr calls so node tests can import it: rows, navigation, presence,
    colours, git pills; git argv and output parsing, web links, the language table (extension →
    linguist colour) and shares, availability, the refresh TTLs, the cache format, the import
    merge, a 6-slot task pool. Code that runs programs takes a `run(argv, {cwd, timeout})`
    function, so node tests run it against real temp repositories.
  - `icons/*.svg`: its own icons.
- **herdr API:**
  - calls: `spaces()`, `panes()`, `focused()`, `theme()`, `exec()` (always `/usr/bin/env …`, with
    `on` for a project's machine), `readTextFile`/`writeTextFile` (written with the expected-text
    check), `appHome()`, `openOverlay`/`closeOverlay`, `notify`, `pickFolder`, `nextCommand()`;
  - commands: `space.select`, `space.new` (`machine` for a remote project), `tab.new`, `mod.open`
    (`worktree`, `gitview-tab`).
- **Programs**, all through `/usr/bin/env GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 LC_ALL=C`:
  - `git` read-only for the tile, with `--no-optional-locks -c core.fsmonitor=false
    -c core.hooksPath=/dev/null -c core.quotepath=off`: `rev-parse --is-inside-work-tree --show-prefix`
    (the folder is a work tree root, not a bare repo or a subfolder), `symbolic-ref`, `log -1`, `@{upstream}`, `rev-list --left-right --count`,
    `status --porcelain`, `config branch.<b>.remote`, `remote get-url`,
    `worktree list --porcelain -z` (line format on git < 2.36),
    `ls-files -z --format=%(objectmode) %(objectsize)%x09%(path)` for languages (git ≥ 2.38);
  - `git fetch --quiet --no-tags` in the background (20 s), `fetch` / `pull --ff-only` / `push`
    on request (120 s, the user's hooks apply). `GIT_SSH_COMMAND` is never set;
  - `perl -e <group kill>` wraps those network calls: it forks git into its own process group
    (stdin `/dev/null`) and kills the group at the deadline (exit 124) or when it is itself sent
    TERM/INT/HUP (exit 143). `perl -e 'exit 0'` first, once per machine;
  - `sh` (`[ -d ]` checks), `find | head | xargs wc -c` for folders without git;
  - `open` for links and Finder.
- **Refresh:** per project, local git first (published at once), then the fetch, then languages.
  `fetched_at` (local scan) and `remote_at` (fetch) have separate TTLs of
  `metadata_cache_minutes`, so local rescans never postpone the fetch; languages use
  `language_cache_hours`. At most 6 projects at a time.
- **Files outside its folder:**
  - `~/.config/herdr/plugins/config/vndv.project-tiles/projects.json`: the catalog, in the VS Code
    Project Tiles format (unknown fields kept). The folder is created on the first save.
  - `settings.json` next to the catalog.
  - `~/.config/herdr-app/data/project-tiles/metadata-cache.json` (`{version: 3, projects}`), merged
    with what another copy wrote (newer entry wins). Seeded once from
    `~/.local/state/herdr/plugins/vndv.project-tiles/metadata-cache.json` (the old Rust backend's,
    version 2) when present.
  - The import reads `~/Library/Application Support/project-tiles/projects.json` (VS Code
    Project Tiles; `~/.config/project-tiles/` on Linux).
- **Optional:** it can be embedded as a layout tab with `props {embedded: true}`. It then never
  takes focus on start; a tab bar mod sends it `run("project-tiles.focus")`.
- **Remote projects:** a project with `"machine": "<name>"` runs every program there (`exec` `on`),
  matches panes of that machine's spaces only and opens with `space.new {machine}`. `a` records the
  focused terminal's machine; the import keeps the machine of each open folder.

## Watch out

- Herdr's `exec` timeout kills only the process it started: a plain `git fetch` that times out
  leaves `ssh` running. That is what the perl group kill is for. Do not reach for
  `GIT_SSH_COMMAND` or `core.sshCommand=ssh -o BatchMode=yes`: either overrides per-folder ssh
  keys. On a machine without perl (asked once, never guessed from git's output) git runs
  directly and only exec's timeout applies. A git command is never run twice.
- `close(cx)` is Herdr's cleanup hook (pane, overlay gone, app quit): the UI action is `dismiss()`.
  After `close` no refresh starts and nothing is published; git calls already running end on
  their own deadlines.
- Herdr's runtime has no `TextEncoder`, `Buffer` or `setTimeout`; `model.js` must not use them
  (the tests delete them before importing it). Percent-encode with `encodeURIComponent`.
- `exec` rejects when `cwd` does not exist: check availability (`sh -c '[ -d … ]'`) before running
  git there.
- Languages count tracked regular files only (`git ls-files`, sizes from the index; symlinks and
  submodules left out), so gitignored output (`node_modules`, build folders, local `.py` scratch)
  is not counted; hidden folders and `language_ignore_dirs` (exact names) are skipped. Markdown,
  YAML, TOML and MDX are not languages. Folders without git, repositories on git < 2.38 and
  listings over exec's 8 MiB (very large monorepos) use the bounded `find` scan, which skips
  file names containing a newline.
- The cache is checked entry by entry on read: a wrong shape resets that field or drops the
  entry, never the boot. Saving merges per phase (git, languages, availability) with what
  another copy wrote, keeps only entries that fit their project's folder and machine, and shows
  an error when it cannot write.
- `herdr-app check` runs `init()` for real: everything that runs programs or writes files waits
  for `checking()` to be false.
- Never take focus in `render`. Focus comes from `nextCommand()` ("focus") and from handlers.
- Button clicks must `stop_propagation()`, or a double click on Fetch also opens the project.
- Scroll the list to the selected row only when the selection moves, or live updates jump the view.
- Paths on this computer compare case-insensitively, with `/private/tmp` equal to `/tmp` (macOS);
  on a remote machine exactly. Linked worktrees count for their main project. Metadata whose
  `path` or `machine` is not the project's current one is dropped (`metaFits`); a project asked
  for while it is busy is refreshed again when it is free.
- Remote projects are wired through but were not tried against a real remote machine.
- Tested with 8–30 projects. The grid is not virtualised.

## Done when

- `model.js`'s node tests pass and `herdr-app check mods/project-tiles` passes; `cmd-alt-p` opens the
  dialog with your projects grouped by collection.
- Each git tile fills with branch, changed files and the last commit within a second or two; the
  cache file appears in `~/.config/herdr-app/data/project-tiles/`.
- A project open in a space shows `open · space N`; its `Go to space` (or `Enter`) shows that
  space and closes the dialog.
- A repo with local commits shows `↑N`; after `f` on a repo with new upstream commits, it shows `↓N`.
- A fetch that hangs (an unreachable ssh host) is stopped after 20 s with no `git` or `ssh`
  process left behind.
- `/` filters by branch name; `e` changes a name, and the change is in `projects.json`.
- Clicking a GitHub repo's branch pill opens `https://github.com/<owner>/<repo>/tree/<branch>`.
