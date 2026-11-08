//! Reading messages: list entry from the header (+ start of the text), full body, parts.

use mail_parser::MessageParser;

use crate::Summary;

/// One part a user can open or that the body shows (`cid:` image).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Index of the part in the message (stable for the same bytes).
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub is_inline: bool,
    pub content_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub text: String,
    pub html: String,
    pub to: String,
    pub cc: String,
    pub message_id: String,
    pub references: String,
    pub parts: Vec<Part>,
}

fn addresses(list: Option<mail_parser::AddressList<'_>>) -> String {
    list.map(|l| {
        l.mailboxes()
            .filter_map(|m| {
                let a = m.address()?;
                Some(match m.name().filter(|n| !n.trim().is_empty()) {
                    Some(n) => format!("{n} <{a}>"),
                    None => a.to_owned(),
                })
            })
            .collect::<Vec<_>>()
            .join("; ")
    })
    .unwrap_or_default()
}

/// One line of text for the list.
fn preview_of(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = words.join(" ");
    if let Some((i, _)) = out.char_indices().nth(200) {
        out.truncate(i);
    }
    out
}

/// List entry from the header block and the first bytes of the text.
pub fn summary(header: &[u8], text_start: &[u8]) -> Summary {
    let mut raw = header.to_vec();
    raw.extend_from_slice(text_start);
    let Some(m) = MessageParser::default().parse(&raw) else {
        return Summary::default();
    };
    let from = m.from().and_then(|l| l.first());
    let has_attachments = m.content_type().is_some_and(|c| {
        c.ctype().eq_ignore_ascii_case("multipart")
            && c.subtype().is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
    });
    let text = m
        .body_text(0)
        .map(|t| t.into_owned())
        .or_else(|| m.body_html(0).map(|h| strip_tags(&h)))
        .unwrap_or_default();
    Summary {
        from_name: from.and_then(|f| f.name()).unwrap_or_default().to_owned(),
        from_addr: from
            .and_then(|f| f.address())
            .unwrap_or_default()
            .to_lowercase(),
        subject: m.subject().unwrap_or_default().to_owned(),
        preview: preview_of(&text),
        received: m.date().map_or(0, |d| d.to_timestamp()),
        has_attachments,
        ..Summary::default()
    }
}

/// Rough text of an HTML start (it may be cut anywhere), for the preview only.
fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let lower = html.to_ascii_lowercase();
    // Skip <head>/<style> content when the cut start includes it.
    let start = lower.find("<body").unwrap_or(0);
    for c in html[start..].chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&")
}

pub fn parse(raw: &[u8]) -> Parsed {
    let Some(m) = MessageParser::default().parse(raw) else {
        return Parsed::default();
    };
    let from = m.from().and_then(|l| l.first());
    let html = m.body_html(0).map(|h| h.into_owned()).unwrap_or_default();
    let text = m.body_text(0).map(|t| t.into_owned()).unwrap_or_default();
    // A text-only mail gives back its text as "HTML" too: keep the text then.
    let is_html =
        m.html_bodies().next().is_some() && m.parts().any(|p| p.is_content_type("text", "html"));
    let mut parts = Vec::new();
    for p in m.parts() {
        let body_text = !is_media(&p) && (p.in_text_body() || p.in_html_body());
        if p.is_multipart() || body_text {
            continue;
        }
        let ct = p
            .content_type()
            .map(|c| match c.subtype() {
                Some(s) => format!("{}/{}", c.ctype(), s),
                None => c.ctype().to_owned(),
            })
            .unwrap_or_else(|| "application/octet-stream".into())
            .to_ascii_lowercase();
        let content_id = p
            .content_id()
            .unwrap_or_default()
            .trim_matches(['<', '>'])
            .to_owned();
        let is_inline = !content_id.is_empty() && ct.starts_with("image/") && !p.is_attachment();
        let name = p.attachment_name().map(str::to_owned).unwrap_or_else(|| {
            if is_inline {
                content_id.clone()
            } else {
                "piece-jointe".into()
            }
        });
        parts.push(Part {
            id: p.id().to_string(),
            name,
            content_type: ct,
            size: p.decoded_len() as i64,
            is_inline,
            content_id,
        });
    }
    Parsed {
        subject: m.subject().unwrap_or_default().to_owned(),
        from_name: from.and_then(|f| f.name()).unwrap_or_default().to_owned(),
        from_addr: from
            .and_then(|f| f.address())
            .unwrap_or_default()
            .to_owned(),
        text: if text.is_empty() && is_html {
            crate::mime::strip_tags(&html)
        } else {
            text
        },
        html: if is_html { html } else { String::new() },
        to: addresses(m.to()),
        cc: addresses(m.cc()),
        message_id: m.message_id().unwrap_or_default().to_owned(),
        references: m
            .references()
            .map(|r| {
                r.iter()
                    .map(|x| format!("<{x}>"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default(),
        parts,
    }
}

fn is_media(p: &mail_parser::MessagePart<'_>) -> bool {
    !p.is_text()
}

/// Decoded bytes of a part, by the id [`parse`] gave it.
pub fn part_bytes(raw: &[u8], id: &str) -> Option<Vec<u8>> {
    let m = MessageParser::default().parse(raw)?;
    let id: u32 = id.parse().ok()?;
    Some(m.part(id)?.decoded().into_owned())
}

/// Header block → (name, value) pairs, folded lines joined, values left encoded.
pub fn header_fields(raw: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw);
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.split("\r\n").flat_map(|l| l.split('\n')) {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    out
}
