//! Colours from the user's herdr theme (`[theme.custom]`, `[theme.custom.dark]`), read only.

use ratatui::style::Color;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Theme {
    pub text: Color,
    pub subtext0: Color,
    pub overlay0: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub accent: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub teal: Color,
    pub peach: Color,
    pub panel_bg: Color,
    hex: HashMap<String, String>,
}

/// gruvbox (dark), the user's base theme.
const DEFAULTS: &[(&str, &str)] = &[
    ("text", "#ebdbb2"),
    ("subtext0", "#a89984"),
    ("overlay0", "#7c6f64"),
    ("overlay1", "#928374"),
    ("surface0", "#3c3836"),
    ("surface1", "#504945"),
    ("accent", "#fe8019"),
    ("red", "#fb4934"),
    ("green", "#b8bb26"),
    ("yellow", "#fabd2f"),
    ("blue", "#83a598"),
    ("teal", "#8ec07c"),
    ("mauve", "#d3869b"),
    ("peach", "#fe8019"),
    ("panel_bg", "#1d2021"),
];

/// Names accepted in colour fields, mapped to theme tokens.
pub const PALETTE: &[(&str, &str)] = &[
    ("red", "red"),
    ("orange", "peach"),
    ("yellow", "yellow"),
    ("green", "green"),
    ("teal", "teal"),
    ("blue", "blue"),
    ("purple", "mauve"),
    ("gray", "overlay0"),
];

impl Default for Theme {
    fn default() -> Self {
        Theme::from_map(HashMap::new())
    }
}

impl Theme {
    fn from_map(over: HashMap<String, String>) -> Theme {
        let mut hex: HashMap<String, String> = DEFAULTS
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        for (k, v) in over {
            if parse_hex(&v).is_some() {
                hex.insert(k, v);
            }
        }
        let c = |k: &str| {
            hex.get(k)
                .and_then(|v| parse_hex(v))
                .unwrap_or(Color::Reset)
        };
        Theme {
            text: c("text"),
            subtext0: c("subtext0"),
            overlay0: c("overlay0"),
            surface0: c("surface0"),
            surface1: c("surface1"),
            accent: c("accent"),
            red: c("red"),
            green: c("green"),
            yellow: c("yellow"),
            blue: c("blue"),
            teal: c("teal"),
            peach: c("peach"),
            panel_bg: c("panel_bg"),
            hex,
        }
    }

    /// Reads `$HERDR_CONFIG_PATH` or `~/.config/herdr/config.toml`. Never writes.
    pub fn load() -> Theme {
        let path = std::env::var_os("HERDR_CONFIG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(crate::text::home()).join(".config/herdr/config.toml")
            });
        match std::fs::read_to_string(path) {
            Ok(s) => Theme::from_toml(&s),
            Err(_) => Theme::default(),
        }
    }

    pub fn from_toml(s: &str) -> Theme {
        let mut over = HashMap::new();
        if let Ok(v) = s.parse::<toml::Table>()
            && let Some(custom) = v
                .get("theme")
                .and_then(|t| t.get("custom"))
                .and_then(|c| c.as_table())
        {
            for (k, val) in custom {
                if let Some(s) = val.as_str() {
                    over.insert(k.clone(), s.to_string());
                }
            }
            if let Some(dark) = custom.get("dark").and_then(|d| d.as_table()) {
                for (k, val) in dark {
                    if let Some(s) = val.as_str() {
                        over.insert(k.clone(), s.to_string());
                    }
                }
            }
        }
        Theme::from_map(over)
    }

    pub fn hex(&self, token: &str) -> &str {
        self.hex.get(token).map(|s| s.as_str()).unwrap_or("#808080")
    }

    /// `#hex`, `#rgb`, `rrggbb` or a palette name → normalized `#rrggbb`.
    pub fn parse_color(&self, s: &str) -> Option<String> {
        let s = s.trim().to_lowercase();
        if let Some((_, tok)) = PALETTE.iter().find(|(n, _)| *n == s) {
            return Some(self.hex(tok).to_lowercase());
        }
        normalize_hex(&s)
    }

    /// A stable palette colour for a name (new collections, tiles without colour).
    pub fn pick_color(&self, name: &str) -> String {
        let vals: Vec<&str> = PALETTE[..PALETTE.len() - 1]
            .iter()
            .map(|(_, t)| *t)
            .collect();
        let sum: u32 = name.chars().map(|c| c as u32).sum();
        self.hex(vals[sum as usize % vals.len()]).to_lowercase()
    }

    pub fn color(&self, hex: &str) -> Color {
        parse_hex(hex).unwrap_or(self.overlay0)
    }
}

pub fn normalize_hex(s: &str) -> Option<String> {
    let h = s.trim().trim_start_matches('#').to_lowercase();
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        6 => Some(format!("#{h}")),
        3 => Some(format!(
            "#{}",
            h.chars().flat_map(|c| [c, c]).collect::<String>()
        )),
        _ => None,
    }
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let h = normalize_hex(s)?;
    let n = u32::from_str_radix(&h[1..], 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_custom_dark() {
        let t = Theme::from_toml(
            "[theme]\nname='gruvbox'\n[theme.custom]\ntext='#111111'\n[theme.custom.dark]\ntext='#222222'\nred='nope'\n",
        );
        assert_eq!(t.text, Color::Rgb(0x22, 0x22, 0x22));
        assert_eq!(t.red, Color::Rgb(0xfb, 0x49, 0x34));
        assert_eq!(t.parse_color("Orange").as_deref(), Some("#fe8019"));
        assert_eq!(t.parse_color("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(t.parse_color("123456").as_deref(), Some("#123456"));
        assert_eq!(t.parse_color("nope"), None);
        assert_eq!(t.pick_color("x"), t.pick_color("x"));
    }
}
