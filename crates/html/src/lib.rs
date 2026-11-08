//! Mail HTML to simple blocks for MailCraft's simplified reader.
//!
//! Only structure and emphasis survive: paragraphs, headings, list items, quotes, preformatted
//! text, rules, images and links. Scripts, styles, forms, hidden text (newsletter "preheaders"),
//! tracking pixels and dangerous link schemes are dropped. Layout tables become plain flow.
//! Colours and fonts are left out on purpose so mail stays readable in light and dark themes.

use scraper::{ElementRef, Html, Node};

/// Upper bound on blocks kept from one message, so a hostile 50 MB mail cannot stall the UI.
pub mod doc;

pub const MAX_BLOCKS: usize = 5_000;
/// Element nesting followed before falling back to plain text (keeps the stack small).
const MAX_DEPTH: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// http(s) or mailto only.
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Para,
    Heading(u8),
    Item {
        ordered: bool,
        number: usize,
        depth: u8,
    },
    Pre,
    Rule,
    /// `src` is http(s) or `cid:` (an attachment of the message).
    Image {
        src: String,
        alt: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub inlines: Vec<Inline>,
    /// Nesting of `<blockquote>` (quoted replies).
    pub quote: u8,
}

#[derive(Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    pre: bool,
    quote: u8,
}

struct Walker {
    drop_trackers: bool,
    /// Addresses of the tracking pixels met (hidden parts included), for the original view.
    trackers: Vec<String>,
    blocks: Vec<Block>,
    current: Vec<Inline>,
    kind: Kind,
    quote: u8,
    /// (ordered, next number) for each open list.
    lists: Vec<(bool, usize)>,
    depth: usize,
}

pub fn parse(html: &str) -> Vec<Block> {
    parse_with(html, true)
}

/// Like [`parse`]; `drop_trackers` false keeps tracking pixels (they are then fetched, so the
/// sender knows the mail was opened).
pub fn parse_with(html: &str, drop_trackers: bool) -> Vec<Block> {
    parse_full(html, drop_trackers).0
}

/// [`parse_with`], and the addresses of the message's tracking pixels found on the way: the
/// original view can drop them without parsing the message again.
pub fn parse_full(html: &str, drop_trackers: bool) -> (Vec<Block>, Vec<String>) {
    let doc = Html::parse_document(html);
    let mut w = Walker {
        drop_trackers,
        trackers: Vec::new(),
        blocks: Vec::new(),
        current: Vec::new(),
        kind: Kind::Para,
        quote: 0,
        lists: Vec::new(),
        depth: 0,
    };
    w.children(doc.root_element(), Style::default(), None);
    w.flush();
    w.blocks.truncate(MAX_BLOCKS);
    w.trackers.sort();
    w.trackers.dedup();
    (w.blocks, w.trackers)
}

/// Plain-text mail: one paragraph per line, blank lines kept as breaks.
pub fn from_text(text: &str) -> Vec<Block> {
    text.lines()
        .take(MAX_BLOCKS)
        .map(|line| Block {
            kind: Kind::Para,
            inlines: vec![Inline {
                text: line.trim_end().to_owned(),
                ..Inline::default()
            }],
            quote: line.chars().take_while(|c| *c == '>').count().min(5) as u8,
        })
        .collect()
}

/// Text for search and plain-text quoting.
pub fn to_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match &b.kind {
            Kind::Image { .. } => continue,
            Kind::Rule => out.push_str("----"),
            Kind::Item {
                ordered,
                number,
                depth,
            } => {
                out.push_str(&"  ".repeat(usize::from(*depth)));
                if *ordered {
                    out.push_str(&format!("{number}. "));
                } else {
                    out.push_str("• ");
                }
            }
            _ => {}
        }
        for i in &b.inlines {
            out.push_str(&i.text);
        }
        out.push('\n');
    }
    out
}

fn safe_link(href: &str) -> Option<String> {
    let h = href.trim();
    let lower = h.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        .then(|| h.to_owned())
}

/// The `style` attribute lowercased and without spaces (`display:none`, `width:1px`…).
fn compact_style(el: &ElementRef<'_>) -> Option<String> {
    el.value().attr("style").map(|s| {
        s.to_ascii_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    })
}

/// Hidden by its style: `display:none` or `visibility:hidden`.
fn style_hides(s: &str) -> bool {
    s.contains("display:none") || s.contains("visibility:hidden")
}

fn hidden(el: &ElementRef<'_>) -> bool {
    let v = el.value();
    if v.attr("hidden").is_some() || v.attr("aria-hidden") == Some("true") {
        return true;
    }
    // Not `font-size:0` alone: MJML and other layouts set it on containers to remove gaps
    // between columns and give the text inside its size back (a whole booking confirmation vanished).
    compact_style(el).is_some_and(|s| {
        style_hides(&s) || (s.contains("max-height:0") && s.contains("overflow:hidden"))
    })
}

/// `background:url(…)` / `background-image:url(…)` or the old `background` attribute: some
/// mails are made of background images only.
fn background_image(el: &ElementRef<'_>) -> Option<String> {
    let v = el.value();
    let from_style = v.attr("style").and_then(|s| {
        let lower = s.to_ascii_lowercase();
        let at = lower.find("url(")? + 4;
        let end = at + lower[at..].find(')')?;
        Some(
            s[at..end]
                .trim()
                .trim_matches(['\'', '"'])
                .trim()
                .to_owned(),
        )
    });
    let url = from_style.or_else(|| v.attr("background").map(|b| b.trim().to_owned()))?;
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("cid:"))
        .then_some(url)
}

/// A 1×1 (or 0×0) image is a tracking pixel, not content.
/// Known open-tracking addresses (mailing services, sales tools). Matched on the lowercased URL.
const TRACKER_MARKS: &[&str] = &[
    "/track/open",
    "/wf/open",
    "/open.php",
    "/open.aspx",
    "/tracking/open",
    "/trk/open",
    "/e/o/",
    "/openpixel",
    "/pixel.gif",
    "/pixel.png",
    "/beacon",
    "mailtrack.io",
    "t.yesware.com",
    "mixmax.com/api/track",
    "/emimp/",
    "/imp?",
];

/// An image whose only purpose is to tell the sender the mail was opened: 1 px wide or high,
/// hidden by its style, or served by a known open-tracking address.
fn is_tracker(el: &ElementRef<'_>) -> bool {
    let styled_hidden = compact_style(el).is_some_and(|s| {
        style_hides(&s)
            || s.split(';').any(|d| {
                matches!(
                    d,
                    "opacity:0" | "width:0" | "width:1px" | "height:0" | "height:1px"
                )
            })
    });
    tracking_pixel(el) || styled_hidden || el.value().attr("src").is_some_and(known_tracker)
}

/// A known open-tracking address.
fn known_tracker(src: &str) -> bool {
    let src = src.to_ascii_lowercase();
    TRACKER_MARKS.iter().any(|m| src.contains(m))
}

/// Addresses of the tracking pixels of a message, as written in its HTML.
pub fn tracker_urls(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let img = scraper::Selector::parse("img").expect("selector");
    let mut out: Vec<String> = doc
        .select(&img)
        .filter(is_tracker)
        .filter_map(|el| el.value().attr("src").map(str::to_owned))
        .filter(|s| web_address(s))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The HTML with its tracking pixels pointing nowhere (`src=""`), for the original view and
/// printing. Both spellings of `&` in the address are handled.
pub fn without_trackers(html: &str) -> String {
    without_tracker_urls(html, &tracker_urls(html))
}

fn web_address(src: &str) -> bool {
    let l = src.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://") || l.starts_with("//")
}

/// [`without_trackers`] with addresses already known (from [`parse_full`]): no parsing.
pub fn without_tracker_urls(html: &str, urls: &[String]) -> String {
    let mut out = html.to_owned();
    for url in urls {
        let escaped = url.replace('&', "&amp;");
        // Only the whole `src` value: the same address elsewhere (a link, a longer image
        // address starting with it) stays as it is.
        for form in [url.as_str(), escaped.as_str()] {
            for quote in ['"', '\''] {
                out = out.replace(&format!("src={quote}{form}{quote}"), "src=\"\"");
            }
        }
    }
    out
}

/// The message's HTML with its tracking pixels removed when they are blocked and remote images
/// are not (blocking every remote image already covers them).
pub fn prepare(html: &str, block_remote: bool, block_trackers: bool) -> std::borrow::Cow<'_, str> {
    if block_trackers && !block_remote {
        std::borrow::Cow::Owned(without_trackers(html))
    } else {
        std::borrow::Cow::Borrowed(html)
    }
}

/// Content security policy for a page showing a message (original view, printing): no script
/// but the one with `script_nonce`, no frame or form; remote images, styles, fonts and media
/// only when remote content is allowed.
pub fn page_policy(block_remote: bool, script_nonce: Option<&str>) -> String {
    let remote = if block_remote { "" } else { " https: http:" };
    let script = script_nonce.map_or_else(|| "'none'".to_owned(), |n| format!("'nonce-{n}'"));
    format!(
        "default-src 'none'; style-src 'unsafe-inline'{remote}; img-src data:{remote}; \
         font-src data:{remote}; media-src 'none'{remote}; script-src {script}; \
         frame-src 'none'; form-action 'none'; base-uri 'none'"
    )
}

fn tracking_pixel(el: &ElementRef<'_>) -> bool {
    let tiny = |a: &str| {
        el.value().attr(a).is_some_and(|v| {
            v.trim()
                .trim_end_matches("px")
                .parse::<u32>()
                .is_ok_and(|n| n <= 1)
        })
    };
    tiny("width") || tiny("height")
}

impl Walker {
    fn note_tracker(&mut self, el: &ElementRef<'_>) {
        if let Some(src) = el.value().attr("src")
            && is_tracker(el)
            && web_address(src)
        {
            self.trackers.push(src.to_owned());
        }
    }

    fn flush(&mut self) {
        if self.blocks.len() >= MAX_BLOCKS {
            self.current.clear();
            return;
        }
        let inlines = std::mem::take(&mut self.current);
        let kind = std::mem::replace(&mut self.kind, Kind::Para);
        let mut inlines = trim(inlines, kind == Kind::Pre);
        inlines.retain(|i| !i.text.is_empty());
        if !inlines.is_empty() {
            self.blocks.push(Block {
                kind,
                inlines,
                quote: self.quote,
            });
        }
    }

    fn push_block(&mut self, kind: Kind) {
        self.flush();
        if self.blocks.len() < MAX_BLOCKS {
            self.blocks.push(Block {
                kind,
                inlines: Vec::new(),
                quote: self.quote,
            });
        }
    }

    fn text(&mut self, text: &str, style: Style, link: Option<&str>) {
        let text = if style.pre {
            text.to_owned()
        } else {
            let mut s = String::with_capacity(text.len());
            let mut space = false;
            for c in text.chars() {
                if c.is_whitespace() {
                    space = true;
                } else {
                    if space {
                        s.push(' ');
                    }
                    space = false;
                    s.push(c);
                }
            }
            if space {
                s.push(' ');
            }
            s
        };
        if text.is_empty() {
            return;
        }
        let inline = Inline {
            text,
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
            link: link.map(str::to_owned),
        };
        match self.current.last_mut() {
            Some(last)
                if last.bold == inline.bold
                    && last.italic == inline.italic
                    && last.underline == inline.underline
                    && last.link == inline.link =>
            {
                last.text.push_str(&inline.text);
            }
            _ => self.current.push(inline),
        }
    }

    fn children(&mut self, el: ElementRef<'_>, style: Style, link: Option<&str>) {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            // Absurd nesting (hostile mail): keep the words, drop the structure, no recursion.
            for t in el.text() {
                self.text(t, style, link);
            }
        } else {
            self.children_inner(el, style, link);
        }
        self.depth -= 1;
    }

    fn children_inner(&mut self, el: ElementRef<'_>, style: Style, link: Option<&str>) {
        for child in el.children() {
            if self.blocks.len() >= MAX_BLOCKS {
                return;
            }
            match child.value() {
                Node::Text(t) => self.text(t, style, link),
                Node::Element(_) => {
                    if let Some(e) = ElementRef::wrap(child) {
                        self.element(e, style, link);
                    }
                }
                _ => {}
            }
        }
    }

    fn element(&mut self, el: ElementRef<'_>, mut style: Style, link: Option<&str>) {
        let name = el.value().name();
        if matches!(
            name,
            "script"
                | "style"
                | "head"
                | "title"
                | "noscript"
                | "template"
                | "svg"
                | "iframe"
                | "object"
                | "embed"
                | "form"
                | "button"
                | "input"
                | "select"
                | "textarea"
                | "meta"
                | "link"
        ) || hidden(&el)
        {
            // Not shown, but the original view would still load its pixels.
            for d in el.descendants().filter_map(ElementRef::wrap) {
                if d.value().name() == "img" {
                    self.note_tracker(&d);
                }
            }
            return;
        }
        if name != "img"
            && let Some(src) = background_image(&el)
            && !(self.drop_trackers && known_tracker(&src))
        {
            self.push_block(Kind::Image {
                src,
                alt: String::new(),
            });
        }
        match name {
            "br" => {
                if style.pre {
                    self.text("\n", style, link);
                } else {
                    let kind = self.kind.clone();
                    self.flush();
                    // A line break inside a list item continues the item.
                    if matches!(kind, Kind::Item { .. }) {
                        self.kind = Kind::Para;
                    }
                }
            }
            "hr" => self.push_block(Kind::Rule),
            "img" => {
                self.note_tracker(&el);
                let src = el.value().attr("src").unwrap_or("").trim();
                let lower = src.to_ascii_lowercase();
                if (lower.starts_with("https://")
                    || lower.starts_with("http://")
                    || lower.starts_with("cid:"))
                    && !(self.drop_trackers && is_tracker(&el))
                {
                    let alt = el.value().attr("alt").unwrap_or("").trim().to_owned();
                    self.push_block(Kind::Image {
                        src: src.to_owned(),
                        alt,
                    });
                }
            }
            "b" | "strong" => {
                style.bold = true;
                self.children(el, style, link);
            }
            "i" | "em" | "cite" => {
                style.italic = true;
                self.children(el, style, link);
            }
            "u" | "ins" => {
                style.underline = true;
                self.children(el, style, link);
            }
            "a" => {
                let href = el.value().attr("href").and_then(safe_link);
                self.children(el, style, href.as_deref().or(link));
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                self.kind = Kind::Heading(name.as_bytes()[1] - b'0');
                self.children(el, style, link);
                self.flush();
            }
            "pre" => {
                self.flush();
                self.kind = Kind::Pre;
                style.pre = true;
                self.children(el, style, link);
                self.flush();
            }
            "ul" | "ol" => {
                self.flush();
                self.lists.push((name == "ol", 1));
                self.children(el, style, link);
                self.flush();
                self.lists.pop();
            }
            "li" => {
                self.flush();
                let depth = self.lists.len().saturating_sub(1).min(5) as u8;
                let (ordered, number) = match self.lists.last_mut() {
                    Some((ordered, next)) => {
                        let n = *next;
                        *next += 1;
                        (*ordered, n)
                    }
                    None => (false, 1),
                };
                self.kind = Kind::Item {
                    ordered,
                    number,
                    depth,
                };
                self.children(el, style, link);
                self.flush();
            }
            "blockquote" => {
                self.flush();
                self.quote = self.quote.saturating_add(1).min(5);
                style.quote = self.quote;
                self.children(el, style, link);
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            "p" | "div" | "tr" | "table" | "tbody" | "thead" | "tfoot" | "section" | "article"
            | "header" | "footer" | "main" | "nav" | "center" | "address" | "dl" | "dt" | "dd"
            | "figure" | "figcaption" | "body" | "html" => {
                self.flush();
                self.children(el, style, link);
                self.flush();
            }
            "td" | "th" => {
                // Cells of a layout table read as separate paragraphs; a data row as one line.
                self.children(el, style, link);
                self.text(" ", style, link);
            }
            _ => self.children(el, style, link),
        }
    }
}

/// Removes leading and trailing spaces of a paragraph (kept in preformatted text).
fn trim(mut inlines: Vec<Inline>, pre: bool) -> Vec<Inline> {
    if pre {
        if let Some(last) = inlines.last_mut() {
            let t = last.text.trim_end_matches('\n').to_owned();
            last.text = t;
        }
        return inlines;
    }
    if let Some(first) = inlines.first_mut() {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(last) = inlines.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }
    inlines
}

#[cfg(test)]
mod tests;
