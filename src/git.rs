//! git through the `git` CLI, so per-folder includeIf identities and `core.sshCommand` apply.
//!
//! Every call runs in the project directory, in its own process group with stdin closed and
//! `GIT_TERMINAL_PROMPT=0`: ssh cannot reach a terminal to ask for a password, so nothing hangs.
//! `GIT_SSH_COMMAND` is never set (it would override the art profile's `core.sshCommand`).

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hardening flags for read-only calls (no index lock, no fsmonitor/hooks from the repo).
pub const RO_ARGS: &[&str] = &[
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "core.quotepath=off",
];

pub const QUICK: Duration = Duration::from_secs(5);
pub const FETCH: Duration = Duration::from_secs(20);
pub const REMOTE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
    /// The last meaningful line of git's message.
    pub fn message(&self) -> String {
        let src = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        src.lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("exit {}", self.code))
    }
}

/// Runs git in `cwd`. `ro` adds the read-only hardening flags.
pub fn run(cwd: &str, args: &[&str], timeout: Duration, ro: bool) -> Output {
    let mut cmd = Command::new("git");
    if ro {
        cmd.args(RO_ARGS);
    } else {
        cmd.args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=off"]);
    }
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    run_command(cmd, timeout)
}

pub fn run_command(mut cmd: Command, timeout: Duration) -> Output {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Output {
                code: 127,
                stdout: String::new(),
                stderr: e.to_string(),
            };
        }
    };
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let start = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(1),
            Ok(None) if start.elapsed() > timeout => {
                // Kill the whole process group (git + ssh helpers).
                let _ = Command::new("/bin/kill")
                    .args(["-KILL", &format!("-{}", child.id())])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                return Output {
                    code: 124,
                    stdout: String::new(),
                    stderr: "timeout".into(),
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => {
                return Output {
                    code: 1,
                    stdout: String::new(),
                    stderr: e.to_string(),
                };
            }
        }
    };
    Output {
        code,
        stdout: t_out.join().unwrap_or_default(),
        stderr: t_err.join().unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub date: String,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitInfo {
    pub branch: String,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub dirty: u32,
    pub untracked: u32,
    /// The counts reflect a fresh `git fetch` (otherwise they are "stale").
    pub fetched: bool,
    pub commit: Option<Commit>,
}

pub fn is_repo(path: &str) -> bool {
    std::path::Path::new(path).join(".git").exists()
}

/// `git rev-parse --show-toplevel`, or the path itself.
pub fn toplevel(path: &str) -> String {
    let o = run(
        path,
        &["rev-parse", "--show-toplevel"],
        Duration::from_secs(3),
        true,
    );
    let t = o.stdout.trim();
    if o.ok() && !t.is_empty() {
        t.to_string()
    } else {
        path.to_string()
    }
}

pub const LOG_FORMAT: &str = "--format=%h%x1f%s%x1f%cr%x1f%an";

pub fn parse_commit(line: &str) -> Option<Commit> {
    let mut it = line.trim().split('\u{1f}');
    let hash = it.next()?.trim();
    if hash.is_empty() {
        return None;
    }
    let subject = it.next().unwrap_or("").to_string();
    let date = it.next().unwrap_or("").trim_end_matches(" ago").to_string();
    let author = it.next().unwrap_or("").to_string();
    Some(Commit {
        hash: hash.to_string(),
        subject,
        date,
        author,
    })
}

/// `git rev-list --left-right --count HEAD...@{upstream}` → (ahead, behind).
pub fn parse_ahead_behind(s: &str) -> Option<(u32, u32)> {
    let mut it = s.split_whitespace();
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b))
}

/// `git status --porcelain` → (changed, untracked).
pub fn parse_porcelain(s: &str) -> (u32, u32) {
    let mut dirty = 0;
    let mut untracked = 0;
    for l in s.lines().filter(|l| !l.is_empty()) {
        if l.starts_with("??") {
            untracked += 1;
        } else {
            dirty += 1;
        }
    }
    (dirty, untracked)
}

/// Local git state, optionally after a quiet `git fetch`. `None` when not a repository.
pub fn info(path: &str, fetch: bool) -> Option<GitInfo> {
    if !is_repo(path) {
        return None;
    }
    let mut fetched = false;
    if fetch {
        fetched = run(path, &["fetch", "--quiet", "--no-tags"], FETCH, true).ok();
    }
    let mut g = GitInfo::default();
    let o = run(
        path,
        &["symbolic-ref", "--short", "-q", "HEAD"],
        QUICK,
        true,
    );
    if o.ok() && !o.stdout.trim().is_empty() {
        g.branch = o.stdout.trim().to_string();
    } else {
        let o = run(path, &["rev-parse", "--short", "HEAD"], QUICK, true);
        g.branch = if o.ok() {
            format!("({})", o.stdout.trim())
        } else {
            "(no commits)".into()
        };
        g.detached = true;
    }
    let o = run(path, &["log", "-1", LOG_FORMAT], QUICK, true);
    if o.ok() {
        g.commit = parse_commit(&o.stdout);
    }
    let o = run(
        path,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        QUICK,
        true,
    );
    if o.ok() && !o.stdout.trim().is_empty() {
        g.upstream = Some(o.stdout.trim().to_string());
        let o = run(
            path,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            QUICK,
            true,
        );
        if let Some((a, b)) = parse_ahead_behind(&o.stdout) {
            g.ahead = a;
            g.behind = b;
        }
    }
    let o = run(
        path,
        &["status", "--porcelain", "--untracked-files=normal"],
        QUICK,
        true,
    );
    if o.ok() {
        (g.dirty, g.untracked) = parse_porcelain(&o.stdout);
    }
    g.fetched = fetched;
    Some(g)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOp {
    Fetch,
    Pull,
    Push,
}

impl RemoteOp {
    pub fn name(self) -> &'static str {
        match self {
            RemoteOp::Fetch => "fetch",
            RemoteOp::Pull => "pull",
            RemoteOp::Push => "push",
        }
    }
    pub fn args(self) -> &'static [&'static str] {
        match self {
            RemoteOp::Fetch => &["fetch", "--quiet"],
            RemoteOp::Pull => &["pull", "--ff-only", "--quiet"],
            RemoteOp::Push => &["push", "--quiet"],
        }
    }
}

/// A fetch/pull/push the user asked for. Runs like the user's own git (their hooks apply).
pub fn remote(path: &str, op: RemoteOp) -> Output {
    run(path, op.args(), REMOTE, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        let c = parse_commit("a1b2c3d\u{1f}fix: x\u{1f}2 days ago\u{1f}Me\n").unwrap();
        assert_eq!(c.hash, "a1b2c3d");
        assert_eq!(c.subject, "fix: x");
        assert_eq!(c.date, "2 days");
        assert_eq!(c.author, "Me");
        assert!(parse_commit("").is_none());
        assert_eq!(parse_ahead_behind("3\t1\n"), Some((3, 1)));
        assert_eq!(parse_ahead_behind("x 1"), None);
        assert_eq!(parse_ahead_behind("1"), None);
        assert_eq!(parse_porcelain(" M a\n?? b\nA  c\n?? d\n"), (2, 2));
        assert_eq!(parse_porcelain(""), (0, 0));
    }

    #[test]
    fn local_repo_without_network() {
        let dir = std::env::temp_dir().join(format!("pt-git-{}", crate::text::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().to_string();
        assert!(info(&d, false).is_none());
        let mut init = Command::new("git");
        init.args(["init", "-q", "-b", "main"]).current_dir(&dir);
        assert!(init.status().unwrap().success());
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let g = info(&d, false).unwrap();
        assert_eq!(g.branch, "main");
        assert_eq!(g.untracked, 1);
        assert!(g.upstream.is_none());
        assert!(!g.fetched);
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let top = toplevel(&sub.to_string_lossy());
        assert!(top.ends_with(&dir.file_name().unwrap().to_string_lossy().to_string()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn timeout_kills() {
        let mut c = Command::new("/bin/sleep");
        c.arg("5")
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let t = Instant::now();
        let o = run_command(c, Duration::from_millis(200));
        assert_eq!(o.code, 124);
        assert!(t.elapsed() < Duration::from_secs(3));
    }
}
