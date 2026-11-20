#!/usr/bin/env python3
"""Project Tiles for herdr: a tile dashboard of projects (port of the idea of
InvisibleF0x/project-tiles for VS Code) rendered as a curses TUI in a herdr
plugin pane.

Data:   $HERDR_PLUGIN_CONFIG_DIR/projects.json   (same shape as Project Tiles)
        $HERDR_PLUGIN_CONFIG_DIR/settings.json
Cache:  $HERDR_PLUGIN_STATE_DIR/metadata-cache.json
"""
from __future__ import annotations

import curses
import json
import os
import re
import subprocess
import sys
import threading
import time
import unicodedata
import uuid
from collections import deque
from pathlib import Path

try:
    import tomllib
except ImportError:  # pragma: no cover
    tomllib = None

HOME = str(Path.home())
CONFIG_DIR = Path(os.environ.get("HERDR_PLUGIN_CONFIG_DIR") or Path(HOME, ".config", "herdr-project-tiles"))
STATE_DIR = Path(os.environ.get("HERDR_PLUGIN_STATE_DIR") or CONFIG_DIR / "state")
CATALOG = CONFIG_DIR / "projects.json"
SETTINGS = CONFIG_DIR / "settings.json"
CACHE = STATE_DIR / "metadata-cache.json"
HERDR = os.environ.get("HERDR_BIN_PATH") or "herdr"
VSCODE_CATALOG = Path(HOME, "Library/Application Support/project-tiles/projects.json") if sys.platform == "darwin" \
    else Path(os.environ.get("XDG_CONFIG_HOME") or Path(HOME, ".config"), "project-tiles/projects.json")

DEFAULT_SETTINGS = {
    "columns": 3,                 # preferred columns, 1..6; fewer when a tile would get too narrow
    "tile_size": "normal",        # "normal" | "compact"
    "show_path": True,
    "show_description": True,
    "show_languages": True,
    "show_git": True,
    "fetch_remote": True,         # git fetch before computing ahead/behind
    "metadata_cache_minutes": 10,
    "language_cache_hours": 12,
    "max_files_scanned": 15000,
    "open_mode": "workspace",     # Enter: "workspace" (focus or create) | "tab" (new tab here)
    "language_ignore_dirs": [
        ".git", ".svn", ".hg", "node_modules", "bower_components", "vendor", "Library", "Temp", "Logs",
        "obj", "bin", "build", "Build", "dist", "out", "target", ".idea", ".vs", ".vscode", "__pycache__",
        ".venv", "venv", "env", ".next", ".nuxt", "coverage", ".gradle", "Pods", "DerivedData",
        ".dart_tool", ".cache", ".mypy_cache", ".pytest_cache", ".ruff_cache", ".terraform", "dbt_packages",
        "logs",
    ],
}

# --------------------------------------------------------------------------- languages
LANGS = {  # ext -> (name, color)  colors from github/linguist
    "py": ("Python", "#3572A5"), "pyi": ("Python", "#3572A5"), "ipynb": ("Jupyter", "#DA5B0B"),
    "js": ("JavaScript", "#f1e05a"), "mjs": ("JavaScript", "#f1e05a"), "cjs": ("JavaScript", "#f1e05a"),
    "jsx": ("JavaScript", "#f1e05a"), "ts": ("TypeScript", "#3178c6"), "tsx": ("TypeScript", "#3178c6"),
    "go": ("Go", "#00ADD8"), "rs": ("Rust", "#dea584"), "java": ("Java", "#b07219"), "kt": ("Kotlin", "#A97BFF"),
    "kts": ("Kotlin", "#A97BFF"), "scala": ("Scala", "#c22d40"), "c": ("C", "#555555"), "h": ("C", "#555555"),
    "cc": ("C++", "#f34b7d"), "cpp": ("C++", "#f34b7d"), "hpp": ("C++", "#f34b7d"), "cs": ("C#", "#178600"),
    "swift": ("Swift", "#F05138"), "m": ("Objective-C", "#438eff"), "rb": ("Ruby", "#701516"),
    "php": ("PHP", "#4F5D95"), "lua": ("Lua", "#000080"), "sh": ("Shell", "#89e051"), "bash": ("Shell", "#89e051"),
    "zsh": ("Shell", "#89e051"), "fish": ("Shell", "#89e051"), "ps1": ("PowerShell", "#012456"),
    "sql": ("SQL", "#e38c00"), "html": ("HTML", "#e34c26"), "htm": ("HTML", "#e34c26"), "css": ("CSS", "#663399"),
    "scss": ("SCSS", "#c6538c"), "vue": ("Vue", "#41b883"), "svelte": ("Svelte", "#ff3e00"),
    "dart": ("Dart", "#00B4AB"), "ex": ("Elixir", "#6e4a7e"), "exs": ("Elixir", "#6e4a7e"),
    "erl": ("Erlang", "#B83998"), "hs": ("Haskell", "#5e5086"), "ml": ("OCaml", "#ef7a08"),
    "clj": ("Clojure", "#db5855"), "zig": ("Zig", "#ec915c"), "nim": ("Nim", "#ffc200"), "r": ("R", "#198CE7"),
    "jl": ("Julia", "#a270ba"), "tf": ("HCL", "#844FBA"), "hcl": ("HCL", "#844FBA"), "nix": ("Nix", "#7e7eff"),
    "vim": ("Vim Script", "#199f4b"), "el": ("Emacs Lisp", "#c065db"), "md": ("Markdown", "#083fa1"),
    "mdx": ("MDX", "#fcb32c"), "toml": ("TOML", "#9c4221"), "yml": ("YAML", "#cb171e"), "yaml": ("YAML", "#cb171e"),
    "j2": ("Jinja", "#a52a22"), "jinja": ("Jinja", "#a52a22"), "proto": ("Protobuf", "#a0a0a0"),
    "gd": ("GDScript", "#355570"), "shader": ("ShaderLab", "#222c37"), "glsl": ("GLSL", "#5686a5"),
    "mk": ("Makefile", "#427819"), "dockerfile": ("Dockerfile", "#384d54"),
}
NAMED_FILES = {"Makefile": ("Makefile", "#427819"), "Dockerfile": ("Dockerfile", "#384d54"),
               "justfile": ("Just", "#384d54"), "Justfile": ("Just", "#384d54")}
LANG_COLOR = {n: c for n, c in list(LANGS.values()) + list(NAMED_FILES.values())}
MARKUP = {"Markdown", "YAML", "TOML", "MDX"}  # counted only when nothing else is found


def scan_languages(root: str, ignore: set[str], max_files: int) -> list[list]:
    sizes: dict[str, int] = {}
    seen = 0
    queue = deque([root])
    while queue and seen < max_files:
        d = queue.popleft()
        try:
            it = os.scandir(d)
        except OSError:
            continue
        with it:
            for e in it:
                try:
                    if e.is_dir(follow_symlinks=False):
                        if e.name not in ignore and not e.name.startswith("."):
                            queue.append(e.path)
                        continue
                    if not e.is_file(follow_symlinks=False):
                        continue
                    seen += 1
                    lang = NAMED_FILES.get(e.name)
                    if not lang:
                        ext = e.name.rsplit(".", 1)[-1].lower() if "." in e.name else ""
                        lang = LANGS.get(ext)
                    if lang:
                        sizes[lang[0]] = sizes.get(lang[0], 0) + e.stat(follow_symlinks=False).st_size
                except OSError:
                    continue
                if seen >= max_files:
                    break
    code = {k: v for k, v in sizes.items() if k not in MARKUP}
    use = code if sum(code.values()) > 0 else sizes
    total = sum(use.values())
    if not total:
        return []
    out = sorted(([k, v * 100.0 / total] for k, v in use.items()), key=lambda x: -x[1])
    return [x for x in out if x[1] >= 0.5][:8]


# --------------------------------------------------------------------------- git
GIT_RO = ["git", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null",
          "-c", "core.quotepath=off"]


def git_env() -> dict:
    env = dict(os.environ)
    env["GIT_TERMINAL_PROMPT"] = "0"
    env["LC_ALL"] = "C"
    # Deliberately NOT setting GIT_SSH_COMMAND: it would override per-profile core.sshCommand.
    return env


def run_git(path: str, args: list[str], timeout: float, ro: bool = True) -> tuple[int, str, str]:
    try:
        p = subprocess.run((GIT_RO if ro else ["git"]) + args, cwd=path, env=git_env(), capture_output=True,
                           text=True, timeout=timeout, stdin=subprocess.DEVNULL,
                           start_new_session=True)  # no controlling tty -> ssh cannot prompt
        return p.returncode, p.stdout.strip(), p.stderr.strip()
    except subprocess.TimeoutExpired:
        return 124, "", "timeout"
    except OSError as e:
        return 127, "", str(e)


def is_git(path: str) -> bool:
    return os.path.exists(os.path.join(path, ".git"))


def git_info(path: str, fetch: bool) -> dict | None:
    if not is_git(path):
        return None
    if fetch:
        run_git(path, ["fetch", "--quiet", "--no-tags"], 20)
    info: dict = {}
    rc, out, _ = run_git(path, ["symbolic-ref", "--short", "-q", "HEAD"], 5)
    if rc == 0 and out:
        info["branch"] = out
    else:
        rc, out, _ = run_git(path, ["rev-parse", "--short", "HEAD"], 5)
        info["branch"] = f"({out})" if rc == 0 else "(no commits)"
        info["detached"] = True
    rc, out, _ = run_git(path, ["log", "-1", "--format=%h%x1f%s%x1f%cr%x1f%an"], 5)
    if rc == 0 and out:
        h, s, d, a = (out.split("\x1f") + ["", "", "", ""])[:4]
        info["commit"] = {"hash": h, "subject": s, "date": d.replace(" ago", ""), "author": a}
    rc, out, _ = run_git(path, ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], 5)
    if rc == 0 and out:
        info["upstream"] = out
        rc, out, _ = run_git(path, ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"], 5)
        if rc == 0 and out:
            a, b = out.split()
            info["ahead"], info["behind"] = int(a), int(b)
    rc, out, _ = run_git(path, ["status", "--porcelain", "--untracked-files=normal"], 5)
    if rc == 0:
        lines = [l for l in out.splitlines() if l]
        info["dirty"] = sum(1 for l in lines if not l.startswith("??"))
        info["untracked"] = sum(1 for l in lines if l.startswith("??"))
    info["fetched"] = bool(fetch)
    return info


# --------------------------------------------------------------------------- herdr
def herdr(*args: str, timeout: float = 10) -> dict | None:
    try:
        p = subprocess.run([HERDR, *args], capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.TimeoutExpired):
        return None
    txt = p.stdout.strip() or p.stderr.strip()
    try:
        data = json.loads(txt)
    except ValueError:
        return {"error": {"message": txt or f"exit {p.returncode}"}} if p.returncode else {"raw": txt}
    return data


def walk(obj, want: str):
    """Yield every dict in obj that has key `want`."""
    if isinstance(obj, dict):
        if want in obj:
            yield obj
        for v in obj.values():
            yield from walk(v, want)
    elif isinstance(obj, list):
        for v in obj:
            yield from walk(v, want)


def herdr_context() -> dict:
    try:
        return json.loads(os.environ.get("HERDR_PLUGIN_CONTEXT_JSON") or "{}") or {}
    except ValueError:
        return {}


def norm(p: str) -> str:
    return os.path.normpath(os.path.expanduser(p)).rstrip("/").lower()


def within(child: str, parent: str) -> bool:
    c, p = norm(child), norm(parent)
    return c == p or c.startswith(p + "/")


def herdr_panes() -> list[dict]:
    data = herdr("pane", "list")
    if not data or "error" in data:
        return []
    return [d for d in walk(data, "pane_id") if isinstance(d.get("pane_id"), str)]


# --------------------------------------------------------------------------- storage
def load_json(path: Path, default):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def save_json(path: Path, data) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + f".tmp{os.getpid()}")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp, path)


class Catalog:
    def __init__(self):
        self.collections: list[dict] = []
        self.projects: list[dict] = []
        self.mtime = 0.0
        self.load()

    def load(self) -> bool:
        try:
            mt = CATALOG.stat().st_mtime
        except OSError:
            mt = 0.0
        if mt and mt == self.mtime:
            return False
        data = load_json(CATALOG, None)
        if data is None and mt:  # unparsable hand edit -> keep current state
            self.mtime = mt
            return False
        data = data or {}
        self.collections = [c for c in data.get("collections", []) if isinstance(c, dict) and c.get("id")]
        self.projects = [p for p in data.get("projects", []) if isinstance(p, dict) and p.get("path")]
        for p in self.projects:
            p.setdefault("id", str(uuid.uuid4()))
            p.setdefault("name", os.path.basename(p["path"].rstrip("/")) or p["path"])
        self.mtime = mt
        return True

    def save(self) -> None:
        save_json(CATALOG, {"version": 1, "collections": self.collections, "projects": self.projects})
        try:
            self.mtime = CATALOG.stat().st_mtime
        except OSError:
            pass

    def find_path(self, path: str) -> dict | None:
        return next((p for p in self.projects if norm(p["path"]) == norm(path)), None)

    def collection(self, cid) -> dict | None:
        return next((c for c in self.collections if c["id"] == cid), None) if cid else None

    def collection_by_name(self, name: str, create: bool = True) -> dict | None:
        name = name.strip()
        if not name:
            return None
        c = next((c for c in self.collections if c["name"].lower() == name.lower()), None)
        if not c and create:
            c = {"id": str(uuid.uuid4()), "name": name, "color": pick_color(name), "collapsed": False}
            self.collections.append(c)
        return c

    def add(self, path: str, **fields) -> dict:
        path = os.path.normpath(os.path.expanduser(path))
        p = {"id": str(uuid.uuid4()), "name": os.path.basename(path) or path, "path": path,
             "logo": {"kind": "auto"}, **fields}
        self.projects.append(p)
        return p


def project_root(path: str) -> str:
    rc, out, _ = run_git(path, ["rev-parse", "--show-toplevel"], 3)
    return out if rc == 0 and out else path


# --------------------------------------------------------------------------- text helpers
def cw(ch: str) -> int:
    if unicodedata.combining(ch) or ch in "‍️":
        return 0
    return 2 if unicodedata.east_asian_width(ch) in "WF" else 1


def width(s: str) -> int:
    return sum(cw(c) for c in s)


def trunc(s: str, w: int, left: bool = False) -> str:
    if w <= 0:
        return ""
    if width(s) <= w:
        return s
    if left:
        out, used = [], 1
        for ch in reversed(s):
            if used + cw(ch) > w:
                break
            out.append(ch)
            used += cw(ch)
        return "…" + "".join(reversed(out))
    out, used = [], 1
    for ch in s:
        if used + cw(ch) > w:
            break
        out.append(ch)
        used += cw(ch)
    return "".join(out) + "…"


def initials(name: str) -> str:
    parts = [p for p in re.split(r"[\s_\-.]+", name) if p]
    if len(parts) == 1:
        caps = re.findall(r"[A-Z0-9]", parts[0])
        if len(caps) >= 2:
            return "".join(caps[:3])
        return parts[0][:2].upper()
    return "".join(p[0] for p in parts[:3]).upper()


def short_path(p: str) -> str:
    return "~" + p[len(HOME):] if p == HOME or p.startswith(HOME + "/") else p


# --------------------------------------------------------------------------- colors
def load_theme() -> dict:
    theme = {"text": "#ebdbb2", "subtext0": "#a89984", "overlay0": "#7c6f64", "accent": "#fe8019",
             "red": "#fb4934", "green": "#b8bb26", "yellow": "#fabd2f", "blue": "#83a598", "teal": "#8ec07c",
             "mauve": "#d3869b", "peach": "#fe8019", "surface1": "#504945"}
    if tomllib:
        try:
            with open(Path(HOME, ".config/herdr/config.toml"), "rb") as f:
                cfg = tomllib.load(f)
            custom = cfg.get("theme", {}).get("custom", {})
            dark = custom.get("dark", {}) if isinstance(custom.get("dark"), dict) else {}
            flat = {k: v for k, v in custom.items() if isinstance(v, str)}
            for k, v in {**flat, **dark}.items():
                if isinstance(v, str) and v.startswith("#"):
                    theme[k] = v
        except (OSError, ValueError):
            pass
    return theme


THEME = load_theme()
PALETTE = {  # names usable in the colour field
    "red": THEME["red"], "orange": THEME["peach"], "yellow": THEME["yellow"], "green": THEME["green"],
    "teal": THEME["teal"], "blue": THEME["blue"], "purple": THEME["mauve"], "gray": THEME["overlay0"],
}


def pick_color(name: str) -> str:
    vals = list(PALETTE.values())[:-1]
    return vals[sum(map(ord, name)) % len(vals)]


def parse_color(s: str) -> str | None:
    s = (s or "").strip().lower()
    if s in PALETTE:
        return PALETTE[s]
    if re.fullmatch(r"#?[0-9a-f]{6}", s):
        return "#" + s.lstrip("#")
    if re.fullmatch(r"#?[0-9a-f]{3}", s):
        return "#" + "".join(c * 2 for c in s.lstrip("#"))
    return None


def hex_to_256(h: str) -> int:
    h = parse_color(h) or "#808080"
    r, g, b = (int(h[i:i + 2], 16) for i in (1, 3, 5))
    steps = [0, 95, 135, 175, 215, 255]
    q = lambda v: min(range(6), key=lambda i: abs(steps[i] - v))
    ci = 16 + 36 * q(r) + 6 * q(g) + q(b)
    cr = (steps[q(r)], steps[q(g)], steps[q(b)])
    gray = round((r + g + b) / 3)
    gi = max(0, min(23, round((gray - 8) / 10)))
    gv = 8 + gi * 10
    d_c = sum((a - b_) ** 2 for a, b_ in zip(cr, (r, g, b)))
    d_g = sum((gv - x) ** 2 for x in (r, g, b))
    return 232 + gi if d_g < d_c else ci


class Colors:
    def __init__(self):
        self.pairs: dict[tuple, int] = {}
        self.ok = curses.has_colors()
        self.n256 = self.ok and curses.COLORS >= 256
        if self.ok:
            try:
                curses.use_default_colors()
            except curses.error:
                pass

    def __call__(self, fg: str | None, bg: str | None = None) -> int:
        if not self.ok or not fg:
            return 0
        key = (fg, bg)
        if key not in self.pairs:
            n = len(self.pairs) + 1
            if n >= curses.COLOR_PAIRS:
                return 0
            conv = (lambda c: hex_to_256(c)) if self.n256 else (lambda c: curses.COLOR_WHITE)
            try:
                curses.init_pair(n, conv(fg), conv(bg) if bg else -1)
            except curses.error:
                return 0
            self.pairs[key] = n
        return curses.color_pair(self.pairs[key])


# --------------------------------------------------------------------------- metadata
class Metadata:
    """Background git + language info with an on-disk cache."""

    def __init__(self, settings: dict):
        self.s = settings
        self.cache: dict = load_json(CACHE, {}) or {}
        self.lock = threading.Lock()
        self.busy: set[str] = set()
        self.dirty = False
        self.version = 0

    def get(self, p: dict) -> dict:
        return self.cache.get(p["id"], {})

    def refresh(self, p: dict, force: bool = False, fetch: bool | None = None) -> None:
        pid = p["id"]
        with self.lock:
            if pid in self.busy:
                return
            m = self.cache.get(pid, {})
            now = time.time()
            git_stale = force or now - m.get("fetchedAt", 0) > self.s["metadata_cache_minutes"] * 60 \
                or m.get("path") != p["path"]
            lang_stale = force or now - m.get("languagesAt", 0) > self.s["language_cache_hours"] * 3600 \
                or m.get("path") != p["path"]
            if not git_stale and not lang_stale:
                return
            self.busy.add(pid)
        do_fetch = self.s["fetch_remote"] if fetch is None else fetch
        threading.Thread(target=self._work, args=(dict(p), git_stale, lang_stale, do_fetch), daemon=True).start()

    def _work(self, p: dict, git_stale: bool, lang_stale: bool, fetch: bool) -> None:
        path = p["path"]
        try:
            m = dict(self.cache.get(p["id"], {}))
            exists = os.path.isdir(path)
            if not exists:
                # parent missing too -> probably an unmounted volume
                m["availability"] = "offline" if path.startswith("/Volumes/") else "deleted"
            else:
                m.pop("availability", None)
                if git_stale and self.s["show_git"]:
                    m["git"] = git_info(path, fetch)
                    m["fetchedAt"] = time.time()
                if lang_stale and self.s["show_languages"]:
                    m["languages"] = scan_languages(path, set(self.s["language_ignore_dirs"]),
                                                    int(self.s["max_files_scanned"]))
                    m["languagesAt"] = time.time()
            m["path"] = path
            with self.lock:
                self.cache[p["id"]] = m
                self.dirty = True
                self.version += 1
        finally:
            with self.lock:
                self.busy.discard(p["id"])

    def flush(self) -> None:
        with self.lock:
            if not self.dirty:
                return
            data = dict(self.cache)
            self.dirty = False
        try:
            save_json(CACHE, data)
        except OSError:
            pass

    def drop(self, pid: str) -> None:
        with self.lock:
            self.cache.pop(pid, None)
            self.dirty = True


# --------------------------------------------------------------------------- app
HELP = [
    ("←↑↓→ hjkl", "навигация"), ("Enter", "открыть (перейти в workspace или создать)"),
    ("o", "открыть в новом workspace"), ("t", "открыть новой вкладкой в текущем workspace"),
    ("/", "поиск (имя, путь, описание, языки, ветка)"), ("a", "добавить текущую папку"),
    ("A", "добавить папку по пути"), ("I", "импорт: открытые workspace + Project Tiles (VS Code)"),
    ("e", "редактировать проект / коллекцию"), ("d", "удалить проект / коллекцию"),
    ("m", "переместить в коллекцию"), ("n", "новая коллекция"), ("J / K", "сдвинуть вниз / вверх"),
    ("Space", "свернуть/развернуть коллекцию"), ("f p P", "git fetch / pull --ff-only / push"),
    ("r / R", "обновить проект / все"), ("O", "показать в Finder"), ("c", "compact / normal"),
    ("E", "открыть projects.json в $EDITOR"), ("q Esc", "закрыть"),
]


class App:
    def __init__(self, scr):
        self.scr = scr
        self.settings = {**DEFAULT_SETTINGS, **(load_json(SETTINGS, {}) or {})}
        if not SETTINGS.exists():
            try:
                save_json(SETTINGS, DEFAULT_SETTINGS)
            except OSError:
                pass
        self.cat = Catalog()
        self.meta = Metadata(self.settings)
        self.ctx = herdr_context()
        self.here = self.ctx.get("focused_pane_cwd") or self.ctx.get("workspace_cwd") \
            or os.environ.get("HERDR_PANE_CWD") or ""
        self.color = Colors()
        self.sel = 0
        self.scroll = 0
        self.query = ""
        self.searching = False
        self.status = ""
        self.status_err = False
        self.show_help = False
        self.items: list[dict] = []
        self.open_ws: dict[str, str] = {}  # project id -> workspace id
        self.quit = False
        first = next((p for p in self.cat.projects), None)
        if first:
            self.focus_project(first)
        threading.Thread(target=self._scan_workspaces, daemon=True).start()
        for p in self.cat.projects:
            self.meta.refresh(p)

    # ----------------------------------------------------------------- helpers
    def _scan_workspaces(self) -> None:
        panes = herdr_panes()
        found = {}
        for p in self.cat.projects:
            for pane in panes:
                for key in ("cwd", "foreground_cwd"):
                    c = pane.get(key)
                    if c and within(c, p["path"]) and pane.get("workspace_id"):
                        found[p["id"]] = pane["workspace_id"]
        self.open_ws = found

    def msg(self, text: str, err: bool = False) -> None:
        self.status, self.status_err = text, err

    def attr(self, name: str) -> int:
        return self.color(THEME.get(name, name))

    def tile_color(self, p: dict) -> str:
        c = parse_color(p.get("color", ""))
        if c:
            return c
        col = self.cat.collection(p.get("collectionId"))
        return parse_color(col.get("color", "")) if col and parse_color(col.get("color", "")) else pick_color(p["name"])

    def show(self, p: dict, key: str) -> bool:
        v = (p.get("show") or {}).get(key)
        return self.settings[f"show_{key}"] if v is None else bool(v)

    def matches(self, p: dict) -> bool:
        if not self.query:
            return True
        m = self.meta.get(p)
        hay = " ".join([p["name"], p["path"], p.get("description", ""),
                        " ".join(p.get("languages") or [l[0] for l in m.get("languages") or []]),
                        ((m.get("git") or {}).get("branch") or "")]).lower()
        return all(w in hay for w in self.query.lower().split())

    def sections(self) -> list[tuple[dict | None, list[dict]]]:
        out = []
        known = {c["id"] for c in self.cat.collections}
        ungrouped = [p for p in self.cat.projects if p.get("collectionId") not in known and self.matches(p)]
        if ungrouped or not self.cat.collections:
            out.append((None, ungrouped))
        for c in self.cat.collections:
            ps = [p for p in self.cat.projects if p.get("collectionId") == c["id"] and self.matches(p)]
            if self.query and not ps:
                continue
            out.append((c, ps))
        return out

    def current(self) -> dict | None:
        return self.items[self.sel] if 0 <= self.sel < len(self.items) else None

    def current_project(self) -> dict | None:
        it = self.current()
        return it["project"] if it and it["kind"] == "tile" else None

    def current_collection(self) -> dict | None:
        it = self.current()
        if not it:
            return None
        if it["kind"] == "header":
            return it["collection"]
        return self.cat.collection(it["project"].get("collectionId"))

    # ----------------------------------------------------------------- layout
    def layout(self, W: int) -> tuple[list[dict], int, int, int]:
        compact = self.settings["tile_size"] == "compact"
        min_w = 30 if compact else 36
        cols = max(1, min(int(self.settings["columns"]), 6, (W - 2) // min_w or 1))
        gap = 1
        tw = max(20, (W - 2 - gap * (cols - 1)) // cols)
        th = self.tile_height()
        items, y = [], 0
        secs = self.sections()
        only_ungrouped = len(secs) == 1 and secs[0][0] is None
        for col, ps in secs:
            if not only_ungrouped:
                items.append({"kind": "header", "collection": col, "count": len(ps), "y": y, "x": 0, "w": W - 2,
                              "h": 1, "row": y})
                y += 1
            collapsed = col and col.get("collapsed") and not self.query
            if not collapsed:
                for i, p in enumerate(ps):
                    r, c = divmod(i, cols)
                    items.append({"kind": "tile", "project": p, "y": y + r * th, "x": 1 + c * (tw + gap),
                                  "w": tw, "h": th, "row": y + r * th})
                if ps:
                    y += ((len(ps) + cols - 1) // cols) * th
                elif not only_ungrouped:
                    items.append({"kind": "empty", "y": y, "x": 2, "w": W - 4, "h": 1, "row": y})
                    y += 1
            y += 1
        return items, y, tw, th

    def tile_height(self) -> int:
        if self.settings["tile_size"] == "compact":
            return 5
        h = 4  # borders + name + git line
        h += 1 if self.settings["show_path"] else 0
        h += 1 if self.settings["show_description"] else 0
        h += 2 if self.settings["show_languages"] else 0
        h += 1 if self.settings["show_git"] else 0
        return h

    # ----------------------------------------------------------------- drawing
    def put(self, y: int, x: int, s: str, a: int = 0, maxw: int | None = None) -> int:
        H, W = self.scr.getmaxyx()
        if y < 0 or y >= H or x >= W:
            return x
        if maxw is not None:
            s = trunc(s, maxw)
        s = trunc(s, W - x - (1 if y == H - 1 else 0))
        try:
            self.scr.addstr(y, x, s, a)
        except curses.error:
            pass
        return x + width(s)

    def draw(self) -> None:
        scr = self.scr
        scr.erase()
        H, W = scr.getmaxyx()
        top = 3
        body_h = H - top - 1
        items, total_h, tw, th = self.layout(W)
        selectable = [it for it in items if it["kind"] != "empty"]
        self.items = selectable
        if self.sel >= len(selectable):
            self.sel = max(0, len(selectable) - 1)
        cur = self.current()
        if cur:
            if cur["y"] < self.scroll:
                self.scroll = cur["y"] - (1 if cur["kind"] == "tile" else 0)
            elif cur["y"] + cur["h"] > self.scroll + body_h:
                self.scroll = cur["y"] + cur["h"] - body_h
        self.scroll = max(0, min(self.scroll, max(0, total_h - body_h)))

        # header
        acc = self.attr("accent") | curses.A_BOLD
        x = self.put(0, 1, "▦ Project Tiles", acc)
        x = self.put(0, x + 2, f"{len(self.cat.projects)} проектов", self.attr("subtext0"))
        busy = len(self.meta.busy)
        if busy:
            self.put(0, x + 2, f"⟳ {busy}", self.attr("yellow"))
        if self.here:
            hp = "здесь: " + short_path(self.here)
            self.put(0, max(x + 8, W - width(hp) - 2), hp, self.attr("overlay0"), maxw=W // 2)
        sq = ("/ " + self.query + ("▏" if self.searching else "")) if (self.query or self.searching) \
            else "/ поиск…   ? помощь"
        self.put(1, 1, sq, self.attr("text") if self.searching else self.attr("overlay0"))
        self.put(2, 0, "─" * W, self.attr("surface1"))

        for it in items:
            y = it["y"] - self.scroll + top
            if y + it["h"] <= top or y >= top + body_h:
                continue
            is_sel = cur is not None and it is cur
            if it["kind"] == "header":
                self.draw_header(y, it, is_sel, W)
            elif it["kind"] == "empty":
                self.put(y, it["x"], "пусто — m на проекте, чтобы перенести сюда", self.attr("overlay0"))
            else:
                self.draw_tile(y, it, is_sel, top, top + body_h)
        if not self.cat.projects:
            self.draw_empty(top + 1, W)

        # status line
        if self.status:
            self.put(H - 1, 1, self.status, self.attr("red" if self.status_err else "green"))
        else:
            hint = "Enter открыть · o new ws · t вкладка · a добавить · e правка · / поиск · ? помощь · q выход"
            self.put(H - 1, 1, hint, self.attr("overlay0"))
        scr.noutrefresh()
        if self.show_help:
            self.draw_help(H, W)
        curses.doupdate()

    def draw_empty(self, y: int, W: int) -> None:
        lines = ["Пока нет проектов.", "",
                 "a — добавить папку текущей панели herdr",
                 "A — добавить папку по пути",
                 "I — импортировать открытые workspace и каталог Project Tiles из VS Code"]
        for i, l in enumerate(lines):
            self.put(y + 1 + i, max(1, (W - 60) // 2), l, self.attr("subtext0"))

    def draw_header(self, y: int, it: dict, sel: bool, W: int) -> None:
        col = it["collection"]
        if col is None:
            name, color, desc, collapsed = "Без коллекции", THEME["overlay0"], "", False
        else:
            name, color = col["name"], parse_color(col.get("color", "")) or THEME["overlay0"]
            desc, collapsed = col.get("description", ""), col.get("collapsed") and not self.query
        a = curses.A_REVERSE if sel else 0
        x = self.put(y, 1, ("▸ " if collapsed else "▾ "), self.attr("overlay0") | a)
        x = self.put(y, x, "● ", self.color(color) | a)
        x = self.put(y, x, name, self.attr("text") | curses.A_BOLD | a)
        x = self.put(y, x + 1, f"({it['count']})", self.attr("subtext0") | a)
        if desc:
            x = self.put(y, x + 2, desc, self.attr("overlay0"), maxw=W - x - 4)
        self.put(y, x + 1, "─" * max(0, W - x - 2), self.attr("surface1"))

    def draw_tile(self, y0: int, it: dict, sel: bool, ytop: int, ybot: int) -> None:
        p, x0, w, h = it["project"], it["x"], it["w"], it["h"]
        m = self.meta.get(p)
        avail = m.get("availability")
        color = self.tile_color(p)
        ca = self.color(color)
        border = ca | curses.A_BOLD if sel else self.attr("surface1")
        dim = curses.A_DIM if avail else 0
        tl, tr, bl, br, hz, vt = ("┏", "┓", "┗", "┛", "━", "┃") if sel else ("╭", "╮", "╰", "╯", "─", "│")
        if avail == "offline":
            hz = "╌"
        if avail == "deleted":
            border = self.attr("red")
        inner = w - 4

        def row(i: int) -> int:
            return y0 + i

        def line(i: int, draw) -> None:
            y = row(i)
            if y < ytop or y >= ybot:
                return
            self.put(y, x0, vt, border)
            self.put(y, x0 + w - 1, vt, border)
            self.put(y, x0 + 1, "▌", ca | dim)
            draw(y, x0 + 2)

        if ytop <= row(0) < ybot:
            self.put(row(0), x0, tl + hz * (w - 2) + tr, border)
        if ytop <= row(h - 1) < ybot:
            self.put(row(h - 1), x0, bl + hz * (w - 2) + br, border)

        # title line: logo + name + markers
        logo = p.get("logo") or {}
        mark = ""
        mark_attr = self.attr("teal")
        if self.here and within(self.here, p["path"]):
            mark = "● здесь"
        elif p["id"] in self.open_ws:
            mark = "◆ открыт"
            mark_attr = self.attr("blue")
        if avail:
            mark, mark_attr = ("⊘ offline", self.attr("overlay0")) if avail == "offline" else ("✗ нет папки", self.attr("red"))

        def title(y, x):
            if logo.get("kind") == "emoji" and logo.get("value"):
                badge = trunc(logo["value"], 3)
                x = self.put(y, x, badge + " ", 0)
            else:
                ini = f" {initials(p['name'])} "
                x = self.put(y, x, ini, self.color("#1d2021", color) | curses.A_BOLD | dim)
                x += 1
            room = inner - (x - x0 - 2) - (width(mark) + 1 if mark else 0)
            self.put(y, x, p["name"], self.attr("text") | curses.A_BOLD | dim, maxw=room)
            if mark:
                self.put(y, x0 + w - 2 - width(mark), mark, mark_attr)

        lines = [title]
        compact = self.settings["tile_size"] == "compact"
        if compact or self.settings["show_path"]:
            lines.append(lambda y, x: self.show(p, "path") and
                         self.put(y, x, trunc(short_path(p["path"]), inner, left=True), self.attr("overlay0")))
        if not compact and self.settings["show_description"]:
            lines.append(lambda y, x: self.show(p, "description") and
                         self.put(y, x, p.get("description", ""), self.attr("subtext0") | dim, maxw=inner))
        if not compact and self.settings["show_languages"]:
            langs = [[l, None] for l in p["languages"]] if p.get("languages") else (m.get("languages") or [])
            if p.get("languages"):
                langs = [[l, 100.0 / len(langs)] for l, _ in langs]
            lines.append(lambda y, x: self.show(p, "languages") and self.draw_langbar(y, x, inner, langs, dim))
            lines.append(lambda y, x: self.show(p, "languages") and
                         self.draw_legend(y, x, inner, langs, "languagesAt" in m))
        if self.settings["show_git"]:
            lines.append(lambda y, x: self.show(p, "git") and self.draw_git1(y, x, inner, m, p))
            if not compact:
                lines.append(lambda y, x: self.show(p, "git") and self.draw_git2(y, x, inner, m))
        for i, fn in enumerate(lines[: h - 2]):
            line(i + 1, fn)
        for i in range(len(lines) + 1, h - 1):
            line(i, lambda y, x: None)

    def draw_langbar(self, y, x, w, langs, dim) -> None:
        if dim:  # offline / deleted
            return
        if not langs:
            self.put(y, x, "·" * w, self.attr("surface1"))
            return
        used = 0
        for i, (name, pct) in enumerate(langs):
            n = w - used if i == len(langs) - 1 else max(1, round(w * pct / 100))
            n = min(n, w - used)
            if n <= 0:
                break
            self.put(y, x + used, "▀" * n, self.color(LANG_COLOR.get(name, "#8b8b8b")) | dim)
            used += n

    def draw_legend(self, y, x, w, langs, scanned: bool) -> None:
        cx = x
        for name, pct in langs[:4]:
            label = f"{name} {pct:.0f}%" if pct is not None else name
            if cx - x + width(label) + 2 > w:
                break
            cx = self.put(y, cx, "● ", self.color(LANG_COLOR.get(name, "#8b8b8b")))
            cx = self.put(y, cx, label + "  ", self.attr("subtext0"))
        if not langs:
            self.put(y, x, "языки не найдены" if scanned else "", self.attr("overlay0"))

    def draw_git1(self, y, x, w, m, p) -> None:
        g = m.get("git")
        if m.get("availability"):
            return
        if "git" not in m:
            self.put(y, x, "…" if p["id"] in self.meta.busy else "", self.attr("overlay0"))
            return
        if not g:
            self.put(y, x, "не git-репозиторий", self.attr("overlay0"))
            return
        cx = self.put(y, x, " " + g["branch"] + " ", self.color(THEME["text"], THEME["surface1"]), maxw=w - 14)
        cx += 1
        if "upstream" in g:
            ahead, behind = g.get("ahead", 0), g.get("behind", 0)
            stale = not g.get("fetched")
            if not ahead and not behind:
                cx = self.put(y, cx, "✓ ", self.attr("overlay0" if stale else "green"))
            if behind:
                cx = self.put(y, cx, f"↓{behind} ", self.attr("overlay0" if stale else "peach"))
            if ahead:
                cx = self.put(y, cx, f"↑{ahead} ", self.attr("overlay0" if stale else "blue"))
        else:
            cx = self.put(y, cx, "∅ upstream ", self.attr("overlay0"))
        if g.get("dirty"):
            cx = self.put(y, cx, f"●{g['dirty']} ", self.attr("yellow"))
        if g.get("untracked"):
            self.put(y, cx, f"+{g['untracked']}", self.attr("overlay0"))

    def draw_git2(self, y, x, w, m) -> None:
        c = (m.get("git") or {}).get("commit")
        if not c:
            return
        cx = self.put(y, x, c["hash"] + " ", self.attr("blue"))
        date = " · " + c["date"]
        room = w - (cx - x) - width(date)
        cx = self.put(y, cx, c["subject"], self.attr("subtext0"), maxw=max(4, room))
        self.put(y, cx, date, self.attr("overlay0"))

    def draw_help(self, H: int, W: int) -> None:
        bw = min(W - 4, 66)
        bh = min(H - 2, len(HELP) + 4)
        y0, x0 = max(0, (H - bh) // 2), max(0, (W - bw) // 2)
        win = curses.newwin(bh, bw, y0, x0)
        win.bkgd(" ", self.attr("text"))
        win.erase()
        try:
            ba = self.attr("accent")
            win.addstr(0, 0, "╭" + "─" * (bw - 2) + "╮", ba)
            for yy in range(1, bh - 1):
                win.addstr(yy, 0, "│", ba)
                win.addstr(yy, bw - 1, "│", ba)
            win.insstr(bh - 1, 0, "╰" + "─" * (bw - 2) + "╯", ba)
            win.addstr(0, 2, " Клавиши ", self.attr("accent") | curses.A_BOLD)
            for i, (k, d) in enumerate(HELP[: bh - 4]):
                win.addstr(2 + i, 2, k.ljust(12), self.attr("yellow"))
                win.addstr(2 + i, 15, trunc(d, bw - 17), self.attr("text"))
        except curses.error:
            pass
        win.noutrefresh()

    # ----------------------------------------------------------------- input helpers
    def prompt(self, label: str, value: str = "") -> str | None:
        """One-line editor on the bottom row. Returns None on Esc."""
        H, W = self.scr.getmaxyx()
        buf = list(value)
        pos = len(buf)
        curses.curs_set(1)
        try:
            while True:
                self.scr.move(H - 1, 0)
                self.scr.clrtoeol()
                x = self.put(H - 1, 1, label + ": ", self.attr("accent") | curses.A_BOLD)
                txt = "".join(buf)
                room = W - x - 2
                start = max(0, width(txt[:pos]) - room + 1)
                vis = txt
                while start > 0 and vis:
                    start -= cw(vis[0])
                    vis = vis[1:]
                self.put(H - 1, x, vis, self.attr("text"))
                try:
                    self.scr.move(H - 1, min(W - 1, x + width("".join(buf[:pos])) - (width(txt) - width(vis))))
                except curses.error:
                    pass
                self.scr.refresh()
                try:
                    ch = self.scr.get_wch()
                except curses.error:  # input timeout
                    continue
                if ch in ("\x1b",):
                    return None
                if ch in ("\n", "\r", curses.KEY_ENTER):
                    return "".join(buf)
                if ch in (curses.KEY_BACKSPACE, "\x7f", "\b"):
                    if pos:
                        del buf[pos - 1]
                        pos -= 1
                elif ch == curses.KEY_DC:
                    if pos < len(buf):
                        del buf[pos]
                elif ch == curses.KEY_LEFT:
                    pos = max(0, pos - 1)
                elif ch == curses.KEY_RIGHT:
                    pos = min(len(buf), pos + 1)
                elif ch in (curses.KEY_HOME, "\x01"):
                    pos = 0
                elif ch in (curses.KEY_END, "\x05"):
                    pos = len(buf)
                elif ch == "\x15":  # ctrl+u
                    buf, pos = buf[pos:], 0
                elif isinstance(ch, str) and ch.isprintable():
                    buf.insert(pos, ch)
                    pos += 1
        finally:
            curses.curs_set(0)

    def confirm(self, question: str) -> bool:
        ans = self.prompt(question + " [y/N]")
        return bool(ans) and ans.strip().lower() in ("y", "yes", "д", "да")

    # ----------------------------------------------------------------- actions
    def edit_project(self, p: dict, new: bool = False) -> bool:
        fields = [
            ("Имя", "name", p.get("name", "")),
            ("Путь", "path", short_path(p.get("path", ""))),
            ("Описание", "description", p.get("description", "")),
            ("Цвет (#hex | red orange yellow green teal blue purple gray | пусто = цвет коллекции)", "color",
             p.get("color", "")),
            ("Логотип (emoji | пусто = инициалы)", "logo",
             (p.get("logo") or {}).get("value", "") if (p.get("logo") or {}).get("kind") == "emoji" else ""),
            ("Языки вручную (через запятую | пусто = авто)", "languages", ", ".join(p.get("languages") or [])),
            ("Коллекция (имя, новая создастся | пусто = без коллекции)", "collection",
             (self.cat.collection(p.get("collectionId")) or {}).get("name", "")),
        ]
        vals = {}
        for label, key, cur in fields:
            v = self.prompt(label, cur)
            if v is None:
                self.msg("отменено")
                return False
            vals[key] = v.strip()
        path = os.path.normpath(os.path.expanduser(vals["path"] or p.get("path", "")))
        other = self.cat.find_path(path)
        if other and other is not p:
            self.msg(f"этот путь уже в проекте «{other['name']}»", True)
            return False
        p["name"] = vals["name"] or os.path.basename(path) or path
        p["path"] = path
        for k in ("description",):
            if vals[k]:
                p[k] = vals[k]
            else:
                p.pop(k, None)
        if vals["color"]:
            c = parse_color(vals["color"])
            if c:
                p["color"] = c
            else:
                self.msg(f"не понял цвет «{vals['color']}», оставил прежний", True)
        else:
            p.pop("color", None)
        p["logo"] = {"kind": "emoji", "value": vals["logo"]} if vals["logo"] else {"kind": "auto"}
        langs = [l.strip() for l in vals["languages"].split(",") if l.strip()]
        if langs:
            p["languages"] = langs
        else:
            p.pop("languages", None)
        col = self.cat.collection_by_name(vals["collection"])
        if col:
            p["collectionId"] = col["id"]
        else:
            p.pop("collectionId", None)
        if new and p not in self.cat.projects:
            self.cat.projects.append(p)
        self.cat.save()
        self.meta.refresh(p, force=True, fetch=False)
        self.focus_project(p)
        if not self.status_err:
            self.msg(f"сохранено: {p['name']}")
        return True

    def edit_collection(self, c: dict) -> None:
        name = self.prompt("Имя коллекции", c["name"])
        if name is None:
            return
        desc = self.prompt("Описание", c.get("description", ""))
        if desc is None:
            return
        color = self.prompt("Цвет (#hex | red orange yellow green teal blue purple gray)", c.get("color", ""))
        if color is None:
            return
        c["name"] = name.strip() or c["name"]
        if desc.strip():
            c["description"] = desc.strip()
        else:
            c.pop("description", None)
        if parse_color(color):
            c["color"] = parse_color(color)
        self.cat.save()
        self.msg(f"коллекция сохранена: {c['name']}")

    def add_path(self, path: str) -> None:
        path = os.path.normpath(os.path.expanduser(path.strip()))
        if not os.path.isdir(path):
            self.msg(f"нет такой папки: {path}", True)
            return
        path = project_root(path)
        existing = self.cat.find_path(path)
        if existing:
            self.msg(f"уже есть: {existing['name']} — редактирование")
            self.focus_project(existing)
            self.edit_project(existing)
            return
        p = {"id": str(uuid.uuid4()), "name": os.path.basename(path) or path, "path": path, "logo": {"kind": "auto"}}
        col = self.current_collection()
        if col:
            p["collectionId"] = col["id"]
        self.edit_project(p, new=True)

    def import_projects(self) -> None:
        added = 0
        src = load_json(VSCODE_CATALOG, None)
        if src:
            idmap = {}
            for c in src.get("collections", []):
                mine = self.cat.collection_by_name(c.get("name", ""))
                if mine:
                    idmap[c.get("id")] = mine["id"]
                    if c.get("color") and not mine.get("color"):
                        mine["color"] = c["color"]
                    if c.get("description") and not mine.get("description"):
                        mine["description"] = c["description"]
            for sp in src.get("projects", []):
                path = sp.get("path")
                if not path or path.endswith(".code-workspace") or self.cat.find_path(path):
                    continue
                p = {k: v for k, v in sp.items() if k in ("name", "description", "color", "languages", "show")}
                logo = sp.get("logo") or {}
                p["logo"] = logo if logo.get("kind") == "emoji" else {"kind": "auto"}
                if sp.get("collectionId") in idmap:
                    p["collectionId"] = idmap[sp["collectionId"]]
                self.cat.add(path, **p)
                added += 1
        seen = set()
        for pane in herdr_panes():
            c = pane.get("cwd")
            if not c or not os.path.isdir(c):
                continue
            root = project_root(c)
            if norm(root) in (norm(HOME), "/") or norm(root) in seen or self.cat.find_path(root):
                continue
            seen.add(norm(root))
            self.cat.add(root)
            added += 1
        self.cat.save()
        for p in self.cat.projects:
            self.meta.refresh(p)
        threading.Thread(target=self._scan_workspaces, daemon=True).start()
        self.msg(f"импортировано: {added}" + ("" if src else " (каталог Project Tiles из VS Code не найден)"))

    def open_project(self, p: dict, mode: str) -> None:
        path = p["path"]
        if not os.path.isdir(path):
            self.msg(f"папка недоступна: {path}", True)
            return
        if mode == "tab":
            ws = self.ctx.get("workspace_id") or os.environ.get("HERDR_WORKSPACE_ID")
            args = ["tab", "create", "--cwd", path, "--label", p["name"], "--focus"]
            if ws:
                args[2:2] = ["--workspace", ws]
            res = herdr(*args)
        else:
            res = None
            if mode == "workspace":
                self._scan_workspaces()
                ws = self.open_ws.get(p["id"])
                if ws:
                    res = herdr("workspace", "focus", ws)
            if res is None or "error" in (res or {}):
                res = herdr("workspace", "create", "--cwd", path, "--label", p["name"], "--focus")
        if res is None or "error" in res:
            err = (res or {}).get("error", {}).get("message", "herdr не ответил")
            self.msg(f"не удалось открыть: {err}", True)
            return
        self.quit = True

    def git_action(self, p: dict, op: str) -> None:
        if not is_git(p["path"]):
            self.msg("не git-репозиторий", True)
            return
        args = {"fetch": ["fetch", "--quiet"], "pull": ["pull", "--ff-only", "--quiet"],
                "push": ["push", "--quiet"]}[op]
        self.msg(f"git {op}: {p['name']}…")
        self.draw()
        rc, out, err = run_git(p["path"], args, 120, ro=False)
        if rc == 0:
            self.msg(f"git {op}: готово — {p['name']}")
        else:
            self.msg(f"git {op} ошибка: {(err or out).splitlines()[-1] if (err or out) else rc}", True)
        self.meta.refresh(p, force=True, fetch=False)

    def move(self, delta: int) -> None:
        it = self.current()
        if not it:
            return
        if it["kind"] == "header":
            c = it["collection"]
            if c is None:
                return
            arr = self.cat.collections
            i = arr.index(c)
            j = i + delta
            if 0 <= j < len(arr):
                arr[i], arr[j] = arr[j], arr[i]
                self.cat.save()
                self._pending_focus = ("header", c["id"])
            return
        p = it["project"]
        same = [q for q in self.cat.projects if q.get("collectionId") == p.get("collectionId")]
        i = same.index(p)
        j = i + delta
        if not 0 <= j < len(same):
            return
        a, b = self.cat.projects.index(same[i]), self.cat.projects.index(same[j])
        self.cat.projects[a], self.cat.projects[b] = self.cat.projects[b], self.cat.projects[a]
        self.cat.save()
        self.focus_project(p)

    _pending_focus: tuple | None = None

    def focus_project(self, p: dict) -> None:
        self._pending_focus = ("tile", p["id"])

    def apply_focus(self) -> None:
        if not self._pending_focus:
            return
        kind, key = self._pending_focus
        H, W = self.scr.getmaxyx()
        items, *_ = self.layout(W)
        sel = [it for it in items if it["kind"] != "empty"]
        for i, it in enumerate(sel):
            if kind == "tile" and it["kind"] == "tile" and it["project"]["id"] == key:
                self.sel = i
            if kind == "header" and it["kind"] == "header" and it["collection"] and it["collection"]["id"] == key:
                self.sel = i
        self._pending_focus = None

    def nav(self, key: str) -> None:
        if not self.items:
            return
        cur = self.items[self.sel]
        if key in ("left", "right"):
            self.sel = max(0, min(len(self.items) - 1, self.sel + (1 if key == "right" else -1)))
            return
        rows = sorted({it["row"] for it in self.items})
        ri = rows.index(cur["row"])
        ri += 1 if key == "down" else -1
        if not 0 <= ri < len(rows):
            return
        target = [i for i, it in enumerate(self.items) if it["row"] == rows[ri]]
        cx = cur["x"] + (cur["w"] // 2 if cur["kind"] == "tile" else 0)
        self.sel = min(target, key=lambda i: abs(self.items[i]["x"] + self.items[i]["w"] // 2 - cx)
                       if self.items[i]["kind"] == "tile" else 0)

    # ----------------------------------------------------------------- main loop
    def handle(self, ch) -> None:
        if self.show_help:
            self.show_help = False
            return
        if self.searching:
            if ch in ("\x1b",):
                self.searching, self.query = False, ""
            elif ch in ("\n", "\r", curses.KEY_ENTER):
                self.searching = False
                tiles = [i for i, it in enumerate(self.items) if it["kind"] == "tile"]
                if tiles:
                    self.sel = tiles[0]
            elif ch in (curses.KEY_BACKSPACE, "\x7f", "\b"):
                self.query = self.query[:-1]
            elif ch in (curses.KEY_DOWN, curses.KEY_UP):
                self.searching = False
                self.nav("down" if ch == curses.KEY_DOWN else "up")
            elif isinstance(ch, str) and ch.isprintable():
                self.query += ch
                self.sel = 0
            return
        self.status = ""
        p = self.current_project()
        it = self.current()
        keymap = {curses.KEY_LEFT: "left", "h": "left", curses.KEY_RIGHT: "right", "l": "right",
                  curses.KEY_UP: "up", "k": "up", curses.KEY_DOWN: "down", "j": "down"}
        if ch in keymap:
            self.nav(keymap[ch])
        elif ch in ("q", "\x1b"):
            if self.query:
                self.query = ""
            else:
                self.quit = True
        elif ch == "?":
            self.show_help = True
        elif ch == "/":
            self.searching = True
        elif ch in ("\n", "\r", curses.KEY_ENTER):
            if p:
                self.open_project(p, self.settings["open_mode"])
            elif it and it["kind"] == "header" and it["collection"]:
                it["collection"]["collapsed"] = not it["collection"].get("collapsed")
                self.cat.save()
        elif ch == " ":
            c = self.current_collection()
            if c:
                c["collapsed"] = not c.get("collapsed")
                self.cat.save()
                self._pending_focus = ("header", c["id"])
        elif ch == "o" and p:
            self.open_project(p, "new")
        elif ch == "t" and p:
            self.open_project(p, "tab")
        elif ch == "a":
            if self.here:
                self.add_path(self.here)
            else:
                self.msg("не знаю текущую папку — используй A", True)
        elif ch == "A":
            v = self.prompt("Путь к папке", short_path(self.here) if self.here else "~/")
            if v:
                self.add_path(v)
        elif ch == "I":
            self.import_projects()
        elif ch == "n":
            name = self.prompt("Новая коллекция")
            if name and name.strip():
                c = self.cat.collection_by_name(name)
                self.cat.save()
                self._pending_focus = ("header", c["id"])
        elif ch == "e":
            if p:
                self.edit_project(p)
            elif it and it["kind"] == "header" and it["collection"]:
                self.edit_collection(it["collection"])
        elif ch == "d":
            if p and self.confirm(f"Убрать «{p['name']}» из списка (папка не трогается)?"):
                self.cat.projects.remove(p)
                self.meta.drop(p["id"])
                self.cat.save()
                self.msg(f"убран: {p['name']}")
            elif it and it["kind"] == "header" and it["collection"]:
                c = it["collection"]
                if self.confirm(f"Удалить коллекцию «{c['name']}»? Проекты станут без коллекции"):
                    self.cat.collections.remove(c)
                    for q in self.cat.projects:
                        if q.get("collectionId") == c["id"]:
                            q.pop("collectionId", None)
                    self.cat.save()
        elif ch == "m" and p:
            names = ", ".join(c["name"] for c in self.cat.collections)
            v = self.prompt(f"В коллекцию ({names or 'новая'}; пусто = без коллекции)",
                            (self.cat.collection(p.get("collectionId")) or {}).get("name", ""))
            if v is not None:
                c = self.cat.collection_by_name(v)
                if c:
                    p["collectionId"] = c["id"]
                    c["collapsed"] = False
                else:
                    p.pop("collectionId", None)
                self.cat.save()
                self.focus_project(p)
        elif ch == "J":
            self.move(1)
        elif ch == "K":
            self.move(-1)
        elif ch in ("f", "p", "P") and p:
            self.git_action(p, {"f": "fetch", "p": "pull", "P": "push"}[ch])
        elif ch == "r" and p:
            self.meta.refresh(p, force=True)
            self.msg(f"обновляю {p['name']}…")
        elif ch == "R":
            for q in self.cat.projects:
                self.meta.refresh(q, force=True)
            threading.Thread(target=self._scan_workspaces, daemon=True).start()
            self.msg("обновляю все проекты…")
        elif ch == "O" and p:
            subprocess.Popen(["open" if sys.platform == "darwin" else "xdg-open", p["path"]],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        elif ch == "c":
            self.settings["tile_size"] = "normal" if self.settings["tile_size"] == "compact" else "compact"
            self.save_setting("tile_size")
            self.scr.clear()  # full repaint: the whole layout changes
        elif ch == "E":
            self.edit_file()

    def save_setting(self, key: str) -> None:
        cur = {**DEFAULT_SETTINGS, **(load_json(SETTINGS, {}) or {})}
        cur[key] = self.settings[key]
        save_json(SETTINGS, cur)

    def edit_file(self) -> None:
        if not CATALOG.exists():
            self.cat.save()
        editor = os.environ.get("VISUAL") or os.environ.get("EDITOR") or "vi"
        curses.def_prog_mode()
        curses.endwin()
        try:
            subprocess.call([*editor.split(), str(CATALOG)])
        finally:
            curses.reset_prog_mode()
            self.scr.refresh()
        self.cat.load()

    def run(self) -> None:
        curses.curs_set(0)
        self.scr.keypad(True)
        self.scr.timeout(250)
        last_v = -1
        last_flush = time.time()
        while not self.quit:
            if self.cat.load():  # catalog changed on disk (another client, hand edit)
                for p in self.cat.projects:
                    self.meta.refresh(p)
            self.apply_focus()
            self.draw()
            try:
                ch = self.scr.get_wch()
            except curses.error:
                ch = None
            except KeyboardInterrupt:
                break
            if ch == curses.KEY_RESIZE:
                self.scr.clear()
                continue
            if ch is not None:
                self.handle(ch)
            if time.time() - last_flush > 2 or self.quit:
                self.meta.flush()
                last_flush = time.time()
        self.meta.flush()


# --------------------------------------------------------------------------- CLI
def add_current_cli() -> int:
    ctx = herdr_context()
    here = ctx.get("focused_pane_cwd") or ctx.get("workspace_cwd") or os.getcwd()
    if not os.path.isdir(here):
        print(f"not a directory: {here}", file=sys.stderr)
        return 1
    cat = Catalog()
    root = project_root(here)
    if cat.find_path(root):
        print(f"already in Project Tiles: {root}")
        return 0
    cat.add(root)
    cat.save()
    print(f"added to Project Tiles: {root}")
    return 0


def main() -> int:
    if "--add-current" in sys.argv:
        return add_current_cli()
    if "--version" in sys.argv:
        print("project-tiles 0.1.0")
        return 0
    os.environ.setdefault("ESCDELAY", "25")
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    curses.wrapper(lambda scr: App(scr).run())
    return 0


if __name__ == "__main__":
    sys.exit(main())
