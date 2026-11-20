# herdr Project Tiles

A tile dashboard of projects for [herdr](https://herdr.dev), modelled on
[InvisibleF0x/project-tiles](https://github.com/InvisibleF0x/project-tiles) for VS Code.
It is a herdr plugin: one Rust binary (ratatui + crossterm) shown in an overlay pane,
talking to herdr through its socket API.

Each tile shows the name, a logo (emoji or initials), the path, a description, a language bar,
the git branch, ahead/behind (grey "stale" until a fresh fetch), changed/untracked counts and
the last commit. Live from herdr (event subscriptions, no polling):

- `● here` — the project you are in; `◆ 1,3` — workspaces the project is open in;
- agent counts in the project: `●N` working (yellow), blocked (red), done (teal);
- offline (`/Volumes/...` not mounted: dim, dashed) and deleted folders (red).

Projects can be grouped into collapsible coloured collections, reordered and searched.

## Herdr.app (desktop) mod

The desktop Herdr.app does not load TUI plugins; it has its own mods. `mod/project-tiles/` is the
same dashboard as a Herdr.app mod (GPUI), opened as a dialog with `cmd-alt-p`
(`"cmd-alt-p": "mod.open:project-tiles"` in `~/.config/herdr-app/keys.json`). It shares the
catalog, settings and cache with the TUI plugin and uses this binary as its backend
(`project-tiles app paths | meta | git | resolve | import`, JSON on stdout), so build first:

    cargo build --release --locked
    mkdir -p ~/.config/herdr-app/mods/project-tiles
    cp -R mod/project-tiles/{main.js,model.js,mod.json,icons} ~/.config/herdr-app/mods/project-tiles/
    herdr-app check ~/.config/herdr-app/mods/project-tiles
    node mod/test.mjs   # tests for the mod's pure logic

As a tab: put the mod in a layout block with `"props": {"embedded": true}` (here: the
`command-center` layout's `center` stack, after `board`, with a "Projects" segment in the
`cc-tabs` bar and `cmd-alt-p` / `cmd-alt-3` bound to `cc-tabs.projects`). Embedded, it never
takes the keyboard on start; the tab bar sends it `project-tiles.focus` when it is shown, and
Esc or opening a project goes back to Terminals. Without a block it still opens as a dialog
(`mod.open:project-tiles`).

The mod runs programs: this binary (git read-only and fetch/pull/push in project folders) and
`open` for Reveal in Finder. Enter selects the space showing the project (`space.select`) or
creates one (`space.new {folder, name}`); `t` is `tab.new`, `w` opens Herdr's worktree dialog.
Marks are live from `spaces()`/`panes()`: `◆ N` spaces showing the project, `● here`, agents
working/blocked/done. Git errors are Herdr toasts (`notify`).
Clicking a tile's branch opens it on the remote's web page (GitLab `/-/tree/`, GitHub `/tree/`,
Bitbucket `/src/`; the repository when the branch is not pushed under its own name), clicking the
commit hash opens the commit. The address comes from `git remote get-url` (no network); only `https://` addresses with a dotted
host are built (an ssh alias, a local path or an odd remote gives no link).
With [herdr-gitview](https://github.com/ChmaraX/herdr-gitview) installed, a tile's ± button, `g`
or a click on the changed-files count opens gitview for the project (in its space).

## Install (herdr TUI plugin)

    cargo build --release --locked
    herdr plugin link ~/Documents/programming/herdr-project-tiles

`plugin link` does not run `[[build]]`; rebuild after changing the code. The manifest is
re-read by herdr, so a rebuild needs no re-link. Key binding in `~/.config/herdr/config.toml`:

    [[keys.command]]
    key = "prefix+t"
    type = "plugin_action"
    command = "vndv.project-tiles.open"
    description = "project tiles"

## Commands (manifest entrypoints)

| command | used by | does |
|---|---|---|
| `project-tiles tui` | pane `tiles` (overlay) | the dashboard |
| `project-tiles open` | action `open` | `plugin.pane.open` over the socket |
| `project-tiles add-current` | action `add-current` | adds the focused folder (git root), shows a toast |
| `project-tiles tag` | `[[startup]]`, event `workspace.created` | Spaces sidebar tokens |

## Keys

Press `?` in the panel.

- `←↑↓→` / `hjkl` move; Enter focuses the workspace the project is open in, or creates one
  (cwd + label); `o` always opens a new workspace; `t` opens a tab in the current workspace;
  `w` creates a git worktree workspace (`worktree.create`). The panel closes after opening.
- `/` search (name, path, description, languages, branch).
- `a` / `A` / `I` add the current folder, add a folder by path, import (VS Code Project Tiles
  catalog + folders of open workspaces).
- `e` edit (form: name, path, description, colour, emoji, languages, collection),
  `d` remove from the list (the folder is not touched), `m` move to a collection,
  `n` new collection, `J`/`K` reorder, Space/Enter on a header collapses it.
- `f` / `p` / `P` git fetch / pull `--ff-only` / push (in the background; errors also as a herdr toast).
- `r` / `R` refresh one / all, `O` reveal in Finder, `c` compact/normal,
  `E` edit `projects.json` in `$VISUAL`/`$EDITOR`, `q`/Esc close.
- Mouse: click selects, double click opens, wheel scrolls.

## Files

- `$HERDR_PLUGIN_CONFIG_DIR/projects.json` — the catalog, same shape as Project Tiles
  (unknown fields are kept). Written atomically, reloaded when it changes on disk; a broken
  file is left alone (edits are refused until it parses) and the current state kept. Several
  writers share it: the binary saves under a lock (`.projects.json.lock`) and merges what others
  wrote since its last read (by project/collection id, field by field; on a clash its own edit
  wins); the Herdr.app mod writes only if the file is unchanged since it read it.
- `$HERDR_PLUGIN_CONFIG_DIR/settings.json` — `columns, tile_size, show_path, show_description,
  show_languages, show_git, fetch_remote, metadata_cache_minutes, language_cache_hours,
  max_files_scanned, open_mode (workspace|tab), language_ignore_dirs`.
- `$HERDR_PLUGIN_STATE_DIR/metadata-cache.json` — git/language cache (format v2).

`herdr plugin config-dir vndv.project-tiles` prints the config directory.

## Sidebar tokens

Source `plugin:vndv.project-tiles`, per workspace matched to a project by its panes' cwd
(or by the `pt_id` token): `$pt_id`, `$pt_project` (logo + name), `$pt_git` (`↑N ↓N ●N` or `✓`),
`$pt_langs` (top language). Sent by the startup hook (tokens do not survive a server restart),
on `workspace.created`, when opening from the panel and while the panel is open. Example:

    [ui.sidebar.spaces]
    rows = [["state_icon", "workspace"], ["branch", "$pt_git"]]

Toasts need `[ui.toast] delivery = "herdr"` (the default is `off`).

## git

git runs as a subprocess in the project directory, so per-folder `includeIf` identities and
`core.sshCommand` apply; `GIT_SSH_COMMAND` is never set. Calls run in their own process group
with stdin closed and `GIT_TERMINAL_PROMPT=0`, so ssh cannot ask for a password and hang.
Read-only calls add `--no-optional-locks -c core.fsmonitor=false -c core.hooksPath=/dev/null`.

## Development

    cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
    UPDATE_SNAPSHOTS=1 cargo test   # accept new render snapshots (snapshots/*.txt)

Tests never touch the network: render snapshots use `TestBackend`, the herdr side is a fake
NDJSON socket server.

## Rollback

The Python version is in git history (commit `645aedc`). To go back:

    git checkout 645aedc -- tiles.py bin/run.sh herdr-plugin.toml.bak-20261120-python
    cp herdr-plugin.toml.bak-20261120-python herdr-plugin.toml

herdr re-reads the manifest. The config change is undone with
`cp ~/.config/herdr/config.toml.bak-20261120-sidebar-toast ~/.config/herdr/config.toml`.
