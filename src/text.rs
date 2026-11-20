//! Text helpers: display width, truncation, initials, paths.

use std::path::{Component, Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Display width of a string in terminal cells.
pub fn width(s: &str) -> usize {
    s.graphemes(true).map(grapheme_width).sum()
}

fn grapheme_width(g: &str) -> usize {
    let w = UnicodeWidthStr::width(g);
    // Emoji sequences (ZWJ, variation selector 16) are rendered as one wide cell pair.
    if g.chars().count() > 1 && (g.contains('\u{200d}') || g.contains('\u{fe0f}')) {
        return 2;
    }
    w.min(2)
}

/// Cuts `s` to at most `w` cells, ending with `…` when something was cut.
pub fn trunc(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    if width(s) <= w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 1; // room for the ellipsis
    for g in s.graphemes(true) {
        let gw = grapheme_width(g);
        if used + gw > w {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out.push('…');
    out
}

/// Cuts `s` from the left to at most `w` cells, starting with `…` (for paths).
pub fn trunc_left(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    if width(s) <= w {
        return s.to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    let mut used = 1;
    for g in s.graphemes(true).rev() {
        let gw = grapheme_width(g);
        if used + gw > w {
            break;
        }
        parts.push(g);
        used += gw;
    }
    parts.reverse();
    format!("…{}", parts.concat())
}

/// At most `max` grapheme clusters, so a flag or a skin-toned emoji counts as one.
pub fn clamp_graphemes(s: &str, max: usize) -> String {
    s.graphemes(true).take(max).collect()
}

/// Up to three initials: "MyGameStudio" → "MGS", "shop-backend" → "SB", "Tools" → "TO".
pub fn initials(name: &str) -> String {
    // Split camelCase: a lowercase letter or digit followed by an uppercase letter.
    let mut spaced = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if let Some(p) = prev
            && (p.is_lowercase() || p.is_ascii_digit())
            && c.is_uppercase()
        {
            spaced.push(' ');
        }
        spaced.push(c);
        prev = Some(c);
    }
    let words: Vec<&str> = spaced
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    match words.len() {
        0 => "?".to_string(),
        1 => clamp_graphemes(words[0], 2).to_uppercase(),
        _ => words
            .iter()
            .take(3)
            .map(|w| clamp_graphemes(w, 1))
            .collect::<String>()
            .to_uppercase(),
    }
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

/// `~/x` for paths under $HOME.
pub fn short_path(p: &str) -> String {
    short_path_in(p, &home())
}

pub fn short_path_in(p: &str, home: &str) -> String {
    if home.len() > 1 && (p == home || p.starts_with(&format!("{home}/"))) {
        format!("~{}", &p[home.len()..])
    } else {
        p.to_string()
    }
}

/// Expands `~`, makes absolute against `base` and removes `.`/`..` without touching the disk.
pub fn expand_path(p: &str, base: &str) -> PathBuf {
    let p = p.trim();
    let raw = if p == "~" {
        PathBuf::from(home())
    } else if let Some(rest) = p.strip_prefix("~/") {
        Path::new(&home()).join(rest)
    } else if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        Path::new(base).join(p)
    };
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Comparison key for paths. On macOS the file system is case-insensitive by default and
/// `/tmp`, `/var`, `/etc` are symlinks into `/private` (git reports one, a shell cwd the other);
/// elsewhere paths compare as they are.
pub fn norm(p: &str) -> String {
    let s = p.trim_end_matches('/');
    let s = if s.is_empty() { "/" } else { s };
    if cfg!(target_os = "macos") {
        let s = ["/private/tmp", "/private/var", "/private/etc"]
            .iter()
            .find(|a| s == **a || s.starts_with(&format!("{a}/")))
            .map_or(s, |_| &s["/private".len()..]);
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

/// `child` is `parent` or inside it.
pub fn within(child: &str, parent: &str) -> bool {
    let c = norm(child);
    let p = norm(parent);
    c == p || (p == "/" && c.starts_with('/')) || c.starts_with(&format!("{p}/"))
}

/// Random RFC 4122 v4 id (catalog ids). Uses /dev/urandom, falls back to time+pid.
pub fn new_id() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    if !ok {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ ((std::process::id() as u128) << 64);
        b = t.to_le_bytes();
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_rule() {
        assert_eq!(initials("MyGameStudio"), "MGS");
        assert_eq!(initials("shop-backend"), "SB");
        assert_eq!(initials("Tools"), "TO");
        assert_eq!(initials("dbt_contoso"), "DC");
        assert_eq!(initials("a b c d"), "ABC");
        assert_eq!(initials("---"), "?");
        assert_eq!(initials("проект"), "ПР");
        assert_eq!(initials("api2Server"), "AS");
    }

    #[test]
    fn widths() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("日本"), 4);
        assert_eq!(width("🦀"), 2);
        assert_eq!(width("👩‍💻"), 2);
        assert_eq!(width("🇷🇺"), 2);
    }

    #[test]
    fn truncation() {
        assert_eq!(trunc("hello world", 6), "hello…");
        assert_eq!(trunc("hello", 5), "hello");
        assert_eq!(trunc("日本語テキスト", 5), "日本…");
        assert_eq!(trunc("🦀🦀🦀", 4), "🦀…");
        assert_eq!(trunc("abc", 0), "");
        assert_eq!(trunc_left("/a/b/c/project", 8), "…project");
        assert_eq!(trunc_left("~/日本/x", 4), "…/x");
        assert!(width(&trunc("👩‍💻👩‍💻👩‍💻", 5)) <= 5);
    }

    #[test]
    fn graphemes() {
        assert_eq!(clamp_graphemes("🇷🇺🦀👍🏽x", 3), "🇷🇺🦀👍🏽");
    }

    #[test]
    fn paths() {
        assert_eq!(short_path_in("/Users/me/x", "/Users/me"), "~/x");
        assert_eq!(short_path_in("/Users/mex", "/Users/me"), "/Users/mex");
        assert!(within("/a/b/c", "/a/b"));
        // Case and /private aliases fold on macOS only.
        assert_eq!(within("/a/B/", "/a/b"), cfg!(target_os = "macos"));
        assert_eq!(
            within("/tmp/wt/src", "/private/tmp/wt"),
            cfg!(target_os = "macos")
        );
        assert_eq!(within("/private/var/x", "/var"), cfg!(target_os = "macos"));
        assert!(!within("/privatex/tmp", "/tmp"));
        assert_eq!(norm("/private"), norm("/private"));
        assert!(!within("/a/bc", "/a/b"));
        assert_eq!(expand_path("../x/./y", "/a/b"), PathBuf::from("/a/x/y"));
        let id = new_id();
        assert_eq!(id.len(), 36);
        assert_ne!(id, new_id());
    }
}
