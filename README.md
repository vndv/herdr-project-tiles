# Project Tiles for Herdr.app

A tile dashboard of projects for [Herdr.app](https://herdr.dev), the desktop app for terminals
and coding agents, modelled on [InvisibleF0x/project-tiles](https://github.com/InvisibleF0x/project-tiles)
for VS Code. It is a mod in `herdr-app/mods/project-tiles/`, listed in the
[Herdr mods gallery](https://herdr.dev/mods). Plain JavaScript: nothing to build or install
besides the mod folder.

![Project Tiles](herdr-app/preview.png)

Each tile shows the name, a logo (emoji or initials), the path, a description, a language bar,
the git branch, ahead/behind (grey "no fetch" until a fresh fetch), changed files, linked
worktrees and the last commit. Live from Herdr (no polling):

- `currently open` / `open · space N` — the project you are in, the spaces it is open in;
- agents in the project: working (spinner), blocked (red), done (teal);
- offline (`/Volumes/...` not mounted: dim, dashed) and deleted folders (red).

Projects can be grouped into collapsible coloured collections, reordered and searched.
`herdr-app/RECIPE.md` describes the mod in full: what it shows, every key, what it runs.

## Install

Tell Herdr's agent *"Read ~/.config/herdr-app/AGENTS.md, then add this mod:
herdr.dev/mods/vndv/herdr-project-tiles"*, or by hand:

    git clone https://github.com/vndv/herdr-project-tiles && cd herdr-project-tiles
    cp -R herdr-app/mods/project-tiles ~/.config/herdr-app/mods/
    herdr-app check ~/.config/herdr-app/mods/project-tiles

Bind a key in `~/.config/herdr-app/keys.json` (see `herdr-app/keys.json`):
`"cmd-alt-p": "mod.open:project-tiles"`.

Needs `git` and `perl` on the machine the projects live on (macOS and most Linux systems have
perl). With git older than 2.38 the language bar comes from a folder scan instead of the tracked
files; without perl, see the timeout note below.

As a tab: put the mod in a layout block with `"props": {"embedded": true}`. Embedded, it never
takes the keyboard on start; a tab bar sends it `project-tiles.focus` when it is shown, and Esc
or opening a project goes back to the stack's other child. Without a block it opens as a dialog
(`mod.open:project-tiles`).

## What it runs and touches

- `git` in your project folders, through `/usr/bin/env GIT_TERMINAL_PROMPT=0 LC_ALL=C`:
  - status, branch, upstream, ahead/behind, last commit, remote URL, worktrees, `ls-files` —
    read-only, with `--no-optional-locks -c core.fsmonitor=false -c core.hooksPath=/dev/null
    -c core.quotepath=off`;
  - `fetch` in the background for repos with an upstream (20 s limit; turn it off with
    `"fetch_remote": false` in `settings.json`), and `fetch` / `pull --ff-only` / `push` when you
    ask (120 s limit). These run with your own config and hooks, so per-folder `includeIf`
    identities and `core.sshCommand` apply; `GIT_SSH_COMMAND` is never set.
- `perl`, as a timeout guard for those network calls: Herdr's `exec` stops only the process it
  started, so git's `ssh` child would outlive a timeout. The guard puts git in its own process
  group and kills the whole group at the deadline. Whether a machine has perl is asked once
  (`perl -e 'exit 0'`); on a machine without it git runs directly, and a hanging `ssh` may outlive
  the timeout there.
- `sh`, `find`, `wc` — folder checks and the language scan of folders without git.
- On this computer only: `open` for web links and Finder, `mkdir -p` for the catalog folder.

No network besides git. Files: the catalog and settings below, and its cache in Herdr's data
folder, all on this computer. A project with a `"machine"` field (a herdr-core machine name,
`hc machine ls`) has git, perl, sh and find run on that machine; its paths compare exactly
(case-sensitive), while this computer's compare the macOS way.

## Files

- `~/.config/herdr/plugins/config/vndv.project-tiles/projects.json` — the catalog, same shape
  as Project Tiles for VS Code (unknown fields are kept). Reloaded when it changes on disk; a
  broken file is left alone (edits are refused until it parses). Edits are written only if the
  file is unchanged since the mod read it.
- `settings.json` next to it — `columns, tile_size, show_path, show_description,
  show_languages, show_git, fetch_remote, metadata_cache_minutes, language_cache_hours,
  max_files_scanned, open_mode (workspace|tab), language_ignore_dirs`.
- `~/.config/herdr-app/data/project-tiles/metadata-cache.json` — git and language cache. On
  first start it is seeded from the cache of the former Rust backend
  (`~/.local/state/herdr/plugins/vndv.project-tiles/metadata-cache.json`) when that exists.

## Development

    node herdr-app/test.mjs                    # model.js: pure logic, plus git against temp repos
    herdr-app check herdr-app/mods/project-tiles

`model.js` has no Herdr imports: everything that runs a program takes a `run(argv, {cwd,
timeout})` function, which `main.js` maps to Herdr's `exec` and the tests to `child_process`.
Tests never touch the network and ignore your own git config (`GIT_CONFIG_GLOBAL=/dev/null`,
only `file://` remotes).

Earlier versions shipped a Rust binary that also served the herdr TUI as a plugin; it is in the
git history (last at `2bdad21`).

## License

MIT, see [LICENSE](LICENSE).
