//! Simplified reader: the blocks of `mailcraft-html` drawn with egui. Nothing in a message can
//! run code here; links open in the browser and show their real address on hover.

use egui::{Color32, FontFamily, FontId, RichText, Stroke, Ui, vec2};
use mailcraft_html::{Block, Inline, Kind};

use crate::i18n::T;
use crate::shell::Ctx;

/// What the reader needs beyond the blocks.
pub struct Images<'a> {
    pub block_remote: bool,
    /// Remote images loaded and handed to egui.
    pub ready: &'a std::collections::HashSet<String>,
    /// Inline images: content id → URI handed to egui.
    pub inline_uri: &'a dyn Fn(&str) -> Option<String>,
}

/// Draws the blocks; returns remote image URLs to fetch.
pub fn show(ui: &mut Ui, c: &Ctx<'_>, blocks: &[Block], images: &Images<'_>) -> Vec<String> {
    let mut wanted = Vec::new();
    ui.spacing_mut().item_spacing.y = 6.0;
    for b in blocks {
        let indent = f32::from(b.quote) * 14.0
            + match b.kind {
                Kind::Item { depth, .. } => f32::from(depth) * 18.0 + 6.0,
                _ => 0.0,
            };
        let response = ui
            .horizontal(|ui| {
                ui.add_space(indent);
                ui.vertical(|ui| block(ui, c, b, images, &mut wanted));
            })
            .response;
        if b.quote > 0 {
            let r = response.rect;
            for q in 0..b.quote {
                let x = r.left() + f32::from(q) * 14.0 + 3.0;
                ui.painter().vline(
                    x,
                    r.y_range(),
                    Stroke::new(2.0, c.accent.gamma_multiply(0.5)),
                );
            }
        }
    }
    wanted
}

fn block(ui: &mut Ui, c: &Ctx<'_>, b: &Block, images: &Images<'_>, wanted: &mut Vec<String>) {
    match &b.kind {
        Kind::Rule => {
            ui.separator();
        }
        Kind::Image { src, alt } => image(ui, c, src, alt, images, wanted),
        Kind::Pre => {
            let text: String = b.inlines.iter().map(|i| i.text.as_str()).collect();
            ui.add(egui::Label::new(RichText::new(text).monospace()).selectable(true));
        }
        kind => {
            let size = match kind {
                Kind::Heading(1) => 21.0,
                Kind::Heading(2) => 18.0,
                Kind::Heading(_) => 16.0,
                _ => 14.0,
            };
            let heading = matches!(kind, Kind::Heading(_));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if let Kind::Item {
                    ordered, number, ..
                } = kind
                {
                    let marker = if *ordered {
                        format!("{number}. ")
                    } else {
                        "•  ".to_owned()
                    };
                    ui.label(RichText::new(marker).size(size));
                }
                for i in &b.inlines {
                    inline(ui, c, i, size, heading);
                }
            });
        }
    }
}

fn inline(ui: &mut Ui, c: &Ctx<'_>, i: &Inline, size: f32, heading: bool) {
    let family = if i.bold || heading {
        FontFamily::Name("semibold".into())
    } else {
        FontFamily::Proportional
    };
    let mut text = RichText::new(&i.text).font(FontId::new(size, family));
    if i.italic {
        text = text.italics();
    }
    if i.underline {
        text = text.underline();
    }
    match &i.link {
        Some(url) => {
            let lang = c.lang;
            let response = ui
                .add(egui::Label::new(text.color(c.accent).underline()).sense(egui::Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{} {url}", T::OpensInBrowser.get(lang)));
            if response.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        None => {
            ui.add(egui::Label::new(text).selectable(true));
        }
    }
}

fn image(
    ui: &mut Ui,
    c: &Ctx<'_>,
    src: &str,
    alt: &str,
    images: &Images<'_>,
    wanted: &mut Vec<String>,
) {
    let max_w = ui.available_width().min(900.0);
    let uri = if let Some(cid) = src.strip_prefix("cid:") {
        (images.inline_uri)(cid)
    } else if images.block_remote {
        None
    } else if images.ready.contains(src) {
        Some(src.to_owned())
    } else {
        wanted.push(src.to_owned());
        None
    };
    match uri {
        Some(uri) => {
            ui.add(
                egui::Image::new(uri)
                    .max_width(max_w)
                    .fit_to_original_size(1.0)
                    .max_width(max_w),
            );
        }
        None if !alt.is_empty() => {
            ui.label(RichText::new(format!("[{alt}]")).weak().italics());
        }
        None => {
            let (r, _) = ui.allocate_exact_size(vec2(max_w.min(120.0), 16.0), egui::Sense::hover());
            ui.painter().rect_stroke(
                r,
                2.0,
                Stroke::new(1.0, Color32::GRAY.gamma_multiply(0.4)),
                egui::StrokeKind::Inside,
            );
            let _ = c;
        }
    }
}
