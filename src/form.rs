//! Modal forms: text and choice fields, Tab/Shift+Tab, Enter saves, Esc cancels.

use crate::text::{trunc, width};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

#[derive(Debug, Clone)]
pub enum Kind {
    Text,
    Choice(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub label: String,
    pub hint: String,
    pub kind: Kind,
    chars: Vec<char>,
    cursor: usize,
    pub choice: usize,
}

impl Field {
    pub fn text(label: &str, value: &str, hint: &str) -> Field {
        let chars: Vec<char> = value.chars().collect();
        Field {
            label: label.into(),
            hint: hint.into(),
            kind: Kind::Text,
            cursor: chars.len(),
            chars,
            choice: 0,
        }
    }
    pub fn choice(label: &str, options: Vec<String>, selected: usize, hint: &str) -> Field {
        Field {
            label: label.into(),
            hint: hint.into(),
            choice: selected.min(options.len().saturating_sub(1)),
            kind: Kind::Choice(options),
            chars: Vec::new(),
            cursor: 0,
        }
    }
    pub fn value(&self) -> String {
        match &self.kind {
            Kind::Text => self.chars.iter().collect(),
            Kind::Choice(o) => o.get(self.choice).cloned().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    pub focus: usize,
    pub error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Submit,
    Cancel,
}

impl Form {
    pub fn new(title: &str, fields: Vec<Field>) -> Form {
        Form {
            title: title.into(),
            fields,
            focus: 0,
            error: None,
        }
    }

    pub fn value(&self, i: usize) -> String {
        self.fields.get(i).map(|f| f.value()).unwrap_or_default()
    }
    pub fn choice(&self, i: usize) -> usize {
        self.fields.get(i).map(|f| f.choice).unwrap_or(0)
    }

    pub fn handle(&mut self, k: KeyEvent) -> Outcome {
        let n = self.fields.len();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter => return Outcome::Submit,
            KeyCode::Tab | KeyCode::Down => {
                self.focus = (self.focus + 1) % n.max(1);
                return Outcome::Continue;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = (self.focus + n.max(1) - 1) % n.max(1);
                return Outcome::Continue;
            }
            _ => {}
        }
        let Some(f) = self.fields.get_mut(self.focus) else {
            return Outcome::Continue;
        };
        self.error = None;
        match &f.kind {
            Kind::Choice(opts) => {
                let m = opts.len().max(1);
                match k.code {
                    KeyCode::Left => f.choice = (f.choice + m - 1) % m,
                    KeyCode::Right | KeyCode::Char(' ') => f.choice = (f.choice + 1) % m,
                    KeyCode::Home => f.choice = 0,
                    KeyCode::End => f.choice = m - 1,
                    _ => {}
                }
            }
            Kind::Text => match k.code {
                KeyCode::Char('u') if ctrl => {
                    f.chars.drain(..f.cursor);
                    f.cursor = 0;
                }
                KeyCode::Char('a') if ctrl => f.cursor = 0,
                KeyCode::Char('e') if ctrl => f.cursor = f.chars.len(),
                KeyCode::Char(c) if !ctrl => {
                    f.chars.insert(f.cursor, c);
                    f.cursor += 1;
                }
                KeyCode::Backspace if f.cursor > 0 => {
                    f.cursor -= 1;
                    f.chars.remove(f.cursor);
                }
                KeyCode::Delete if f.cursor < f.chars.len() => {
                    f.chars.remove(f.cursor);
                }
                KeyCode::Left => f.cursor = f.cursor.saturating_sub(1),
                KeyCode::Right => f.cursor = (f.cursor + 1).min(f.chars.len()),
                KeyCode::Home => f.cursor = 0,
                KeyCode::End => f.cursor = f.chars.len(),
                _ => {}
            },
        }
        Outcome::Continue
    }

    /// Height the form wants: title, 2 rows per field, hint, error, footer.
    pub fn height(&self) -> u16 {
        (self.fields.len() as u16) * 2 + 6
    }

    /// Draws the form centred in `area`; returns the terminal cursor position for a text field.
    pub fn render(&self, buf: &mut Buffer, area: Rect, t: &Theme) -> Option<(u16, u16)> {
        let w = area.width.saturating_sub(4).clamp(20, 76);
        let h = self.height().min(area.height);
        // Frame + one field + hint/error/footer must fit; otherwise just say so.
        if w > area.width || h < 7 {
            if area.width > 0 && area.height > 0 {
                let y = area.y + area.height / 2;
                let s = Style::default().fg(t.yellow).bg(t.panel_bg);
                buf.set_stringn(
                    area.x,
                    y,
                    "window too small for the form",
                    area.width as usize,
                    s,
                );
            }
            return None;
        }
        let x0 = area.x + area.width.saturating_sub(w) / 2;
        let y0 = area.y + area.height.saturating_sub(h) / 2;
        let rect = Rect::new(x0, y0, w, h);
        let border = Style::default().fg(t.accent);
        let base = Style::default().fg(t.text).bg(t.panel_bg);
        crate::ui::clear_area(buf, rect, base);
        let inner = w.saturating_sub(4) as usize;
        let hz = "─".repeat(w.saturating_sub(2) as usize);
        buf.set_string(x0, y0, format!("╭{hz}╮"), border);
        buf.set_string(x0, y0 + h - 1, format!("╰{hz}╯"), border);
        for y in y0 + 1..y0 + h - 1 {
            buf.set_string(x0, y, "│", border);
            buf.set_string(x0 + w - 1, y, "│", border);
        }
        buf.set_string(
            x0 + 2,
            y0,
            format!(" {} ", trunc(&self.title, inner.saturating_sub(2))),
            border.add_modifier(Modifier::BOLD),
        );
        let mut cursor = None;
        let mut y = y0 + 1;
        for (i, f) in self.fields.iter().enumerate() {
            if y + 1 >= y0 + h - 3 {
                break;
            }
            let focused = i == self.focus;
            let ls = if focused {
                Style::default()
                    .fg(t.yellow)
                    .bg(t.panel_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.subtext0).bg(t.panel_bg)
            };
            buf.set_stringn(x0 + 2, y, &f.label, inner, ls);
            y += 1;
            let marker = if focused { "› " } else { "  " };
            buf.set_string(
                x0 + 2,
                y,
                marker,
                Style::default().fg(t.accent).bg(t.panel_bg),
            );
            let vx = x0 + 4;
            let room = inner.saturating_sub(2);
            match &f.kind {
                Kind::Text => {
                    let before: String = f.chars[..f.cursor].iter().collect();
                    // Scroll so the cursor stays visible.
                    let mut start = 0;
                    while width(&before) - width(&f.chars[..start].iter().collect::<String>())
                        >= room
                        && start < f.cursor
                    {
                        start += 1;
                    }
                    let shown: String = f.chars[start..].iter().collect();
                    let vs = Style::default().fg(t.text).bg(t.surface0);
                    let pad = " ".repeat(room);
                    buf.set_stringn(vx, y, &pad, room, vs);
                    buf.set_stringn(vx, y, trunc(&shown, room), room, vs);
                    if focused {
                        let cx = width(&f.chars[start..f.cursor].iter().collect::<String>());
                        cursor = Some((vx + cx as u16, y));
                    }
                }
                Kind::Choice(opts) => {
                    let v = opts.get(f.choice).cloned().unwrap_or_default();
                    let s = format!("◂ {v} ▸");
                    let vs = if focused {
                        Style::default()
                            .fg(t.text)
                            .bg(t.surface1)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.text).bg(t.surface0)
                    };
                    buf.set_stringn(vx, y, trunc(&s, room), room, vs);
                }
            }
            y += 1;
        }
        if let Some(f) = self.fields.get(self.focus)
            && !f.hint.is_empty()
        {
            buf.set_stringn(
                x0 + 2,
                y0 + h - 4,
                trunc(&f.hint, inner),
                inner,
                Style::default().fg(t.overlay0).bg(t.panel_bg),
            );
        }
        if let Some(e) = &self.error {
            buf.set_stringn(
                x0 + 2,
                y0 + h - 3,
                trunc(e, inner),
                inner,
                Style::default().fg(t.red).bg(t.panel_bg),
            );
        }
        buf.set_stringn(
            x0 + 2,
            y0 + h - 2,
            trunc(
                "Tab/Shift+Tab field · ←→ choose · Enter save · Esc cancel",
                inner,
            ),
            inner,
            Style::default().fg(t.overlay0).bg(t.panel_bg),
        );
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn editing_and_navigation() {
        let mut f = Form::new(
            "t",
            vec![
                Field::text("a", "ab", ""),
                Field::choice("c", vec!["x".into(), "y".into()], 0, ""),
            ],
        );
        f.handle(key(KeyCode::Left));
        f.handle(key(KeyCode::Char('я')));
        assert_eq!(f.value(0), "aяb");
        f.handle(key(KeyCode::Backspace));
        f.handle(key(KeyCode::Delete));
        assert_eq!(f.value(0), "a");
        f.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(f.value(0), "");
        f.handle(key(KeyCode::Tab));
        assert_eq!(f.focus, 1);
        f.handle(key(KeyCode::Right));
        assert_eq!(f.value(1), "y");
        f.handle(key(KeyCode::Right));
        assert_eq!(f.value(1), "x");
        f.handle(key(KeyCode::BackTab));
        assert_eq!(f.focus, 0);
        assert_eq!(f.handle(key(KeyCode::Enter)), Outcome::Submit);
        assert_eq!(f.handle(key(KeyCode::Esc)), Outcome::Cancel);
    }

    #[test]
    fn render_in_tiny_areas_does_not_panic() {
        let f = Form::new(
            "t",
            vec![Field::text("a", "ab", "hint"), Field::text("b", "", "")],
        );
        let t = Theme::default();
        for (w, h) in [
            (80, 24),
            (19, 24),
            (20, 24),
            (24, 7),
            (80, 6),
            (80, 2),
            (80, 1),
            (80, 0),
            (0, 0),
            (1, 1),
        ] {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            f.render(&mut buf, area, &t);
        }
        // Offset areas too (the form is drawn inside the panel).
        let area = Rect::new(3, 2, 19, 5);
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
        f.render(&mut buf, area, &t);
    }
}
