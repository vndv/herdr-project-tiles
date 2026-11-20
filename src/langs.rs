//! Language bar: a bounded breadth-first scan that sums file sizes per language.

use std::collections::{HashMap, HashSet, VecDeque};

/// extension → (language, colour from github/linguist)
const BY_EXT: &[(&str, &str, &str)] = &[
    ("py", "Python", "#3572A5"),
    ("pyi", "Python", "#3572A5"),
    ("ipynb", "Jupyter", "#DA5B0B"),
    ("js", "JavaScript", "#f1e05a"),
    ("mjs", "JavaScript", "#f1e05a"),
    ("cjs", "JavaScript", "#f1e05a"),
    ("jsx", "JavaScript", "#f1e05a"),
    ("ts", "TypeScript", "#3178c6"),
    ("tsx", "TypeScript", "#3178c6"),
    ("go", "Go", "#00ADD8"),
    ("rs", "Rust", "#dea584"),
    ("java", "Java", "#b07219"),
    ("kt", "Kotlin", "#A97BFF"),
    ("kts", "Kotlin", "#A97BFF"),
    ("scala", "Scala", "#c22d40"),
    ("c", "C", "#555555"),
    ("h", "C", "#555555"),
    ("cc", "C++", "#f34b7d"),
    ("cpp", "C++", "#f34b7d"),
    ("hpp", "C++", "#f34b7d"),
    ("cs", "C#", "#178600"),
    ("swift", "Swift", "#F05138"),
    ("m", "Objective-C", "#438eff"),
    ("rb", "Ruby", "#701516"),
    ("php", "PHP", "#4F5D95"),
    ("lua", "Lua", "#000080"),
    ("sh", "Shell", "#89e051"),
    ("bash", "Shell", "#89e051"),
    ("zsh", "Shell", "#89e051"),
    ("fish", "Shell", "#89e051"),
    ("ps1", "PowerShell", "#012456"),
    ("sql", "SQL", "#e38c00"),
    ("html", "HTML", "#e34c26"),
    ("htm", "HTML", "#e34c26"),
    ("css", "CSS", "#663399"),
    ("scss", "SCSS", "#c6538c"),
    ("vue", "Vue", "#41b883"),
    ("svelte", "Svelte", "#ff3e00"),
    ("dart", "Dart", "#00B4AB"),
    ("ex", "Elixir", "#6e4a7e"),
    ("exs", "Elixir", "#6e4a7e"),
    ("erl", "Erlang", "#B83998"),
    ("hs", "Haskell", "#5e5086"),
    ("ml", "OCaml", "#ef7a08"),
    ("clj", "Clojure", "#db5855"),
    ("zig", "Zig", "#ec915c"),
    ("nim", "Nim", "#ffc200"),
    ("r", "R", "#198CE7"),
    ("jl", "Julia", "#a270ba"),
    ("tf", "HCL", "#844FBA"),
    ("hcl", "HCL", "#844FBA"),
    ("nix", "Nix", "#7e7eff"),
    ("vim", "Vim Script", "#199f4b"),
    ("el", "Emacs Lisp", "#c065db"),
    ("md", "Markdown", "#083fa1"),
    ("mdx", "MDX", "#fcb32c"),
    ("toml", "TOML", "#9c4221"),
    ("yml", "YAML", "#cb171e"),
    ("yaml", "YAML", "#cb171e"),
    ("j2", "Jinja", "#a52a22"),
    ("jinja", "Jinja", "#a52a22"),
    ("proto", "Protobuf", "#a0a0a0"),
    ("gd", "GDScript", "#355570"),
    ("shader", "ShaderLab", "#222c37"),
    ("glsl", "GLSL", "#5686a5"),
    ("mk", "Makefile", "#427819"),
];

const BY_NAME: &[(&str, &str, &str)] = &[
    ("Makefile", "Makefile", "#427819"),
    ("Dockerfile", "Dockerfile", "#384d54"),
    ("justfile", "Just", "#384d54"),
    ("Justfile", "Just", "#384d54"),
];

/// Markup and config, not counted as languages (as in Project Tiles).
const MARKUP: &[&str] = &["Markdown", "YAML", "TOML", "MDX"];

pub fn language_for(file: &str) -> Option<&'static str> {
    if let Some((_, l, _)) = BY_NAME.iter().find(|(n, _, _)| *n == file) {
        return Some(l);
    }
    let lower = file.to_lowercase();
    if lower.starts_with("dockerfile") {
        return Some("Dockerfile");
    }
    let (_, ext) = lower.rsplit_once('.')?;
    BY_EXT
        .iter()
        .find(|(e, _, _)| *e == ext)
        .map(|(_, l, _)| *l)
}

pub fn color_of(lang: &str) -> &'static str {
    BY_EXT
        .iter()
        .chain(BY_NAME.iter())
        .find(|(_, l, _)| l.eq_ignore_ascii_case(lang))
        .map(|(_, _, c)| *c)
        .unwrap_or("#8b8b8b")
}

/// Shares (language, percent) sorted by size; at most 8, each ≥ 0.5 %.
pub fn scan(root: &str, ignore: &HashSet<String>, max_files: usize) -> Vec<(String, f64)> {
    let mut sizes: HashMap<&'static str, u64> = HashMap::new();
    let mut seen = 0usize;
    let mut queue = VecDeque::from([std::path::PathBuf::from(root)]);
    'outer: while let Some(dir) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            if ft.is_dir() {
                if !ignore.contains(&name) && !name.starts_with('.') {
                    queue.push_back(e.path());
                }
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            seen += 1;
            if let Some(lang) = language_for(&name) {
                let sz = e.metadata().map(|m| m.len()).unwrap_or(0).max(1);
                *sizes.entry(lang).or_default() += sz;
            }
            if seen >= max_files {
                break 'outer;
            }
        }
    }
    shares(&sizes)
}

fn shares(sizes: &HashMap<&'static str, u64>) -> Vec<(String, f64)> {
    let use_map: HashMap<_, _> = sizes
        .iter()
        .filter(|(k, _)| !MARKUP.contains(k))
        .map(|(k, v)| (*k, *v))
        .collect();
    let total: u64 = use_map.values().sum();
    if total == 0 {
        return Vec::new();
    }
    let mut out: Vec<(String, f64)> = use_map
        .iter()
        .map(|(k, v)| (k.to_string(), *v as f64 * 100.0 / total as f64))
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out.retain(|x| x.1 >= 0.5);
    out.truncate(8);
    out
}

/// Manual override: equal shares.
pub fn from_names(names: &[String]) -> Vec<(String, f64)> {
    if names.is_empty() {
        return Vec::new();
    }
    let pct = 100.0 / names.len() as f64;
    names.iter().map(|n| (n.clone(), pct)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_ignores() {
        let dir = std::env::temp_dir().join(format!("pt-lang-{}", crate::text::new_id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "x".repeat(300)).unwrap();
        std::fs::write(dir.join("run.sh"), "x".repeat(100)).unwrap();
        std::fs::write(dir.join("README.md"), "x".repeat(5000)).unwrap();
        std::fs::write(dir.join("node_modules/x/a.js"), "x".repeat(9000)).unwrap();
        std::fs::write(dir.join(".hidden/b.go"), "x".repeat(9000)).unwrap();
        std::fs::write(dir.join("Dockerfile"), "x").unwrap();
        let ignore: HashSet<String> = ["node_modules".to_string()].into();
        let s = scan(&dir.to_string_lossy(), &ignore, 1000);
        assert_eq!(s[0].0, "Rust");
        assert!((s[0].1 - 300.0 / 401.0 * 100.0).abs() < 0.01);
        assert_eq!(s[1].0, "Shell");
        assert!(
            s.iter()
                .all(|(l, _)| l != "Markdown" && l != "JavaScript" && l != "Go")
        );
        // Only markup → no languages.
        let md = dir.join("docs");
        std::fs::create_dir_all(&md).unwrap();
        std::fs::write(md.join("a.md"), "x").unwrap();
        assert!(scan(&md.to_string_lossy(), &ignore, 10).is_empty());
        // File limit.
        assert!(scan(&dir.to_string_lossy(), &ignore, 1).len() <= 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn lookup() {
        assert_eq!(language_for("Makefile"), Some("Makefile"));
        assert_eq!(language_for("x.TSX"), Some("TypeScript"));
        assert_eq!(language_for("Dockerfile.dev"), Some("Dockerfile"));
        assert_eq!(language_for("noext"), None);
        assert_eq!(color_of("rust"), "#dea584");
        assert_eq!(from_names(&["A".into(), "B".into()])[1].1, 50.0);
    }
}
