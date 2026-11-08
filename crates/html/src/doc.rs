//! The rich-text document behind MailCraft's composer: plain text plus one style per character,
//! so egui's text editor can edit the text while MailCraft keeps the formatting in step.
//!
//! Lists are lines starting with `• ` (bullets) or `1. ` (numbers): they stay readable as text
//! and become `<ul>`/`<ol>` in the HTML that is sent.

use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Size {
    Small,
    #[default]
    Normal,
    Large,
    Huge,
}

impl Size {
    pub fn points(self) -> f32 {
        match self {
            Self::Small => 11.0,
            Self::Normal => 14.0,
            Self::Large => 18.0,
            Self::Huge => 24.0,
        }
    }

    fn css(self) -> Option<&'static str> {
        match self {
            Self::Small => Some("small"),
            Self::Normal => None,
            Self::Large => Some("large"),
            Self::Huge => Some("x-large"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub size: Size,
    /// RGB; `None` is the default text colour.
    pub color: Option<[u8; 3]>,
    /// http(s) or mailto, checked when set.
    pub link: Option<Arc<str>>,
}

pub const BULLET: &str = "• ";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    pub text: String,
    /// One style per `char` of `text`.
    pub styles: Vec<Style>,
}

/// Where `old` and `new` differ: (start, end in old, end in new), in chars.
fn changed_range(old: &[char], new: &[char]) -> (usize, usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    (prefix, old.len() - suffix, new.len() - suffix)
}

impl Doc {
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            styles: vec![Style::default(); text.chars().count()],
        }
    }

    /// Call after the editor changed `text` (from `old`): keeps one style per character.
    /// Inserted characters take `typing`, the style the user has chosen for new text.
    pub fn sync_after_edit(&mut self, old: &str, typing: &Style) {
        if old == self.text {
            return;
        }
        let old: Vec<char> = old.chars().collect();
        let new: Vec<char> = self.text.chars().collect();
        // Styles may be out of step if the caller lost track; fix the length first.
        self.styles.resize(old.len(), Style::default());
        let (start, old_end, new_end) = changed_range(&old, &new);
        let inserted = std::iter::repeat_n(typing.clone(), new_end - start);
        self.styles.splice(start..old_end, inserted);
    }

    /// Style of the character before `at` (what typing at `at` continues).
    pub fn style_before(&self, at: usize) -> Style {
        at.checked_sub(1)
            .and_then(|i| self.styles.get(i))
            .cloned()
            .unwrap_or_default()
    }

    /// Applies `f` to the styles of chars `range`.
    pub fn restyle(&mut self, range: std::ops::Range<usize>, f: impl Fn(&mut Style)) {
        let end = range.end.min(self.styles.len());
        for s in &mut self.styles[range.start.min(end)..end] {
            f(s);
        }
    }

    /// True when every char of `range` has `pred` (false for an empty range).
    pub fn all(&self, range: std::ops::Range<usize>, pred: impl Fn(&Style) -> bool) -> bool {
        let end = range.end.min(self.styles.len());
        range.start < end && self.styles[range.start..end].iter().all(pred)
    }

    /// Toggles a list marker (`BULLET` or numbers) on the lines touched by `range`: if they all
    /// have this kind already it is removed, otherwise every touched line gets it.
    pub fn toggle_list(&mut self, range: std::ops::Range<usize>, numbered: bool) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                lines.push((start, i));
                start = i + 1;
            }
        }
        lines.push((start, chars.len()));
        let touched = |&(s, e): &(usize, usize)| s <= range.end && e >= range.start;
        let all_marked = lines
            .iter()
            .filter(|l| touched(l))
            .all(|&(s, e)| marker_len(&chars[s..e]).is_some_and(|(_, num)| num == numbered));
        let style = |i: usize| self.styles.get(i).cloned().unwrap_or_default();
        let mut text = String::with_capacity(self.text.len() + 16);
        let mut styles = Vec::with_capacity(self.styles.len() + 16);
        let mut number = 1;
        for (idx, line) in lines.iter().enumerate() {
            let (s, e) = *line;
            if idx > 0 {
                text.push('\n');
                styles.push(style(s - 1));
            }
            let mut from = s;
            if touched(line) {
                from += marker_len(&chars[s..e]).map_or(0, |(n, _)| n);
                if !all_marked {
                    let marker = if numbered {
                        format!("{number}. ")
                    } else {
                        BULLET.to_owned()
                    };
                    number += 1;
                    styles.extend(std::iter::repeat_n(
                        Style::default(),
                        marker.chars().count(),
                    ));
                    text.push_str(&marker);
                }
            }
            for (i, c) in chars.iter().enumerate().take(e).skip(from) {
                text.push(*c);
                styles.push(style(i));
            }
        }
        self.text = text;
        self.styles = styles;
    }

    /// HTML body for sending: escaped text, spans for styles, lists from line markers.
    pub fn to_html(&self) -> String {
        let chars: Vec<char> = self.text.chars().collect();
        let mut html =
            String::from("<div style=\"font-family:Calibri,Arial,sans-serif;font-size:11pt\">");
        let mut list: Option<bool> = None;
        let mut line_start = 0;
        loop {
            let end = chars[line_start..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(chars.len(), |p| line_start + p);
            let line = &chars[line_start..end];
            let marker = marker_len(line);
            let kind = marker.filter(|(n, _)| *n > 0).map(|(_, numbered)| numbered);
            if list != kind {
                if let Some(numbered) = list {
                    html.push_str(if numbered { "</ol>" } else { "</ul>" });
                }
                if let Some(numbered) = kind {
                    html.push_str(if numbered { "<ol>" } else { "<ul>" });
                }
                list = kind;
            }
            let skip = marker.map_or(0, |(n, _)| n);
            let content = self.spans_html(line_start + skip, end);
            match kind {
                Some(_) => html.push_str(&format!("<li>{content}</li>")),
                None if content.is_empty() => html.push_str("<div><br></div>"),
                None => html.push_str(&format!("<div>{content}</div>")),
            }
            if end >= chars.len() {
                break;
            }
            line_start = end + 1;
        }
        if let Some(numbered) = list {
            html.push_str(if numbered { "</ol>" } else { "</ul>" });
        }
        html.push_str("</div>");
        html
    }

    fn spans_html(&self, from: usize, to: usize) -> String {
        let chars: Vec<char> = self.text.chars().collect();
        let mut out = String::new();
        let mut i = from;
        while i < to {
            let style = self.styles.get(i).cloned().unwrap_or_default();
            let mut j = i;
            let mut run = String::new();
            while j < to && self.styles.get(j).cloned().unwrap_or_default() == style {
                run.push(chars[j]);
                j += 1;
            }
            out.push_str(&styled(&escape(&run), &style));
            i = j;
        }
        out
    }

    /// Plain-text alternative and quoting.
    pub fn to_text(&self) -> String {
        self.text.clone()
    }
}

/// Length of a list marker at the start of `line` and whether it is numbered.
fn marker_len(line: &[char]) -> Option<(usize, bool)> {
    let bullet: Vec<char> = BULLET.chars().collect();
    if line.starts_with(&bullet) {
        return Some((bullet.len(), false));
    }
    let digits = line.iter().take_while(|c| c.is_ascii_digit()).count();
    if (1..=3).contains(&digits)
        && line.get(digits) == Some(&'.')
        && line.get(digits + 1) == Some(&' ')
    {
        return Some((digits + 2, true));
    }
    None
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn styled(text: &str, s: &Style) -> String {
    let mut css = String::new();
    if let Some(size) = s.size.css() {
        css.push_str(&format!("font-size:{size};"));
    }
    if let Some([r, g, b]) = s.color {
        css.push_str(&format!("color:#{r:02x}{g:02x}{b:02x};"));
    }
    let mut out = text.to_owned();
    if !css.is_empty() {
        out = format!("<span style=\"{css}\">{out}</span>");
    }
    if s.underline {
        out = format!("<u>{out}</u>");
    }
    if s.italic {
        out = format!("<i>{out}</i>");
    }
    if s.bold {
        out = format!("<b>{out}</b>");
    }
    if let Some(link) = &s.link {
        out = format!("<a href=\"{}\">{out}</a>", escape(link));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{BULLET, Doc, Size, Style};

    fn bold() -> Style {
        Style {
            bold: true,
            ..Style::default()
        }
    }

    #[test]
    fn styles_follow_edits_including_multibyte_text() {
        let mut d = Doc::plain("Bonjour monde");
        d.restyle(0..7, |s| s.bold = true);
        // Insert in the middle, with accents and an emoji.
        let old = d.text.clone();
        d.text = "Bonjour à tous 😀 le monde".into();
        d.sync_after_edit(&old, &Style::default());
        assert_eq!(d.styles.len(), d.text.chars().count());
        assert!(d.all(0..7, |s| s.bold));
        assert!(!d.all(7..9, |s| s.bold));
        // Delete everything but "le monde".
        let old = d.text.clone();
        d.text = "le monde".into();
        d.sync_after_edit(&old, &Style::default());
        assert_eq!(d.styles.len(), 8);
    }

    #[test]
    fn random_edits_keep_one_style_per_char() {
        // Deterministic pseudo-random edits (no extra dependency).
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };
        let pieces = ["a", "é", "😀", "\n", "• ", "bonjour ", ""];
        let mut d = Doc::plain("");
        for _ in 0..2_000 {
            let old = d.text.clone();
            let chars: Vec<char> = old.chars().collect();
            let at = next(chars.len() + 1);
            let del = next(4).min(chars.len() - at);
            let ins = pieces[next(pieces.len())];
            let mut s: String = chars[..at].iter().collect();
            s.push_str(ins);
            s.extend(&chars[at + del..]);
            d.text = s;
            d.sync_after_edit(&old, &bold());
            assert_eq!(d.styles.len(), d.text.chars().count());
        }
    }

    #[test]
    fn html_output() {
        let mut d = Doc::plain("Salut <toi> & co\n• un\n• deux\nfin");
        d.restyle(0..5, |s| s.bold = true);
        d.restyle(6..11, |s| {
            s.link = Some("https://example.com/?a=1&b=2".into());
            s.size = Size::Large;
            s.color = Some([255, 0, 0]);
        });
        let html = d.to_html();
        assert!(html.contains("<b>Salut</b>"), "{html}");
        assert!(html.contains(
            "<a href=\"https://example.com/?a=1&amp;b=2\"><span style=\"font-size:large;color:#ff0000;\">&lt;toi&gt;</span></a>"
        ));
        assert!(
            html.contains("&amp; co</div><ul><li>un</li><li>deux</li></ul><div>fin</div>"),
            "{html}"
        );
    }

    #[test]
    fn list_toggle_adds_and_removes_markers() {
        let mut d = Doc::plain("un\ndeux\ntrois");
        d.toggle_list(0..5, false);
        assert_eq!(d.text, format!("{BULLET}un\n{BULLET}deux\ntrois"));
        assert_eq!(d.styles.len(), d.text.chars().count());
        d.toggle_list(0..3, false);
        assert_eq!(d.text, format!("un\n{BULLET}deux\ntrois"));
        d.toggle_list(0..20, true);
        assert_eq!(d.text, "1. un\n2. deux\n3. trois");
        assert!(
            d.to_html()
                .contains("<ol><li>un</li><li>deux</li><li>trois</li></ol>")
        );
    }
}
