//! Writing a message: headers, a rich-text editor (egui's text editor + one style per character,
//! see `mailcraft_html::doc`), formatting bar, attachments, and the quoted original below.

use std::path::PathBuf;
use std::time::Instant;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Key, Modifiers, RichText, Stroke, TextEdit, Ui};
use mailcraft_html::doc::{Doc, Size, Style};

use crate::i18n::T;
use crate::shell::Ctx;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    New,
    Reply { original: String },
    ReplyAll { original: String },
    Forward { original: String },
}

pub struct Composer {
    pub id: u64,
    pub account_id: i64,
    pub kind: Kind,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    show_bcc: bool,
    pub subject: String,
    pub doc: Doc,
    /// Style chosen with the bar for the next characters typed (cleared when the cursor moves).
    pending: Option<Style>,
    last_cursor: Option<(usize, usize)>,
    selection: (usize, usize),
    /// The message replied to or forwarded, shown read-only under the editor.
    pub quoted: Vec<mailcraft_html::Block>,
    pub quoted_header: String,
    pub attachments: Vec<PathBuf>,
    /// Server draft, once saved.
    pub draft_id: Option<String>,
    pub detached: bool,
    pub sending: bool,
    pub error: Option<String>,
    /// Last change not yet saved as a draft.
    pub changed_at: Option<Instant>,
    link: Option<String>,
    pub confirm_close: bool,
}

/// What the composer asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Send,
    PickAttachments,
    Detach,
    /// Close; `Some(true)` keeps the draft, `Some(false)` discards it, `None` asks first.
    Close(Option<bool>),
}

const COLORS: [(u8, u8, u8); 8] = [
    (0x00, 0x00, 0x00),
    (0xC5, 0x0F, 0x1F),
    (0xCA, 0x50, 0x10),
    (0x10, 0x7C, 0x10),
    (0x0F, 0x6C, 0xBD),
    (0x5C, 0x2D, 0x91),
    (0x60, 0x5E, 0x5C),
    (0xE3, 0x00, 0x8C),
];

impl Composer {
    pub fn new(id: u64, account_id: i64, kind: Kind) -> Self {
        Self {
            id,
            account_id,
            kind,
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            show_bcc: false,
            subject: String::new(),
            doc: Doc::default(),
            pending: None,
            last_cursor: None,
            selection: (0, 0),
            quoted: Vec::new(),
            quoted_header: String::new(),
            attachments: Vec::new(),
            draft_id: None,
            detached: false,
            sending: false,
            error: None,
            changed_at: None,
            link: None,
            confirm_close: false,
        }
    }

    /// The user wrote or changed something (prefilled reply fields and the signature do not
    /// count), so closing should offer to keep a draft.
    pub fn has_content(&self) -> bool {
        self.changed_at.is_some() || self.draft_id.is_some()
    }

    fn touch(&mut self) {
        self.changed_at = Some(Instant::now());
    }

    fn typing_style(&self) -> Style {
        self.pending
            .clone()
            .unwrap_or_else(|| self.doc.style_before(self.selection.0))
    }

    /// Bold/italic/underline/size/colour/link from the bar: on the selection, or on what is typed next.
    fn apply(&mut self, set: impl Fn(&mut Style, bool), test: impl Fn(&Style) -> bool) {
        let (a, b) = self.selection;
        if a < b {
            let on = !self.doc.all(a..b, &test);
            self.doc.restyle(a..b, |s| set(s, on));
            self.touch();
        } else {
            let mut s = self.typing_style();
            let on = !test(&s);
            set(&mut s, on);
            self.pending = Some(s);
        }
    }
}

pub fn show(ui: &mut Ui, c: &Ctx<'_>, comp: &mut Composer) -> Option<Action> {
    let lang = c.lang;
    let mut action = None;
    if comp.confirm_close {
        ui.group(|ui| {
            ui.label(T::SaveDraftQuestion.get(lang));
            ui.horizontal(|ui| {
                if ui.button(T::KeepDraft.get(lang)).clicked() {
                    action = Some(Action::Close(Some(true)));
                }
                if ui.button(T::DiscardDraft.get(lang)).clicked() {
                    action = Some(Action::Close(Some(false)));
                }
                if ui.button(T::Cancel.get(lang)).clicked() {
                    comp.confirm_close = false;
                }
            });
        });
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        let send = egui::Button::new(RichText::new(T::Send.get(lang)).color(Color32::WHITE))
            .fill(c.accent);
        if ui.add_enabled(!comp.sending, send).clicked() {
            action = Some(Action::Send);
        }
        if ui.button(T::Attach.get(lang)).clicked() {
            action = Some(Action::PickAttachments);
        }
        if !comp.detached && ui.button(T::Detach.get(lang)).clicked() {
            action = Some(Action::Detach);
        }
        if ui.button(T::Discard.get(lang)).clicked() {
            action = Some(Action::Close(None));
        }
        if comp.sending {
            ui.spinner();
            ui.label(RichText::new(T::Sending.get(lang)).weak());
        } else if comp.draft_id.is_some() && comp.changed_at.is_none() {
            ui.label(RichText::new(T::DraftSaved.get(lang)).weak());
        }
    });
    if let Some(e) = &comp.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    ui.add_space(4.0);

    let mut changed = false;
    egui::Grid::new(("composer-headers", comp.id))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            let account = comp.account_id;
            let mut field = |ui: &mut Ui, label: T, value: &mut String| {
                let l = ui.label(label.get(lang));
                let r = ui
                    .add(TextEdit::singleline(value).desired_width(f32::INFINITY))
                    .labelled_by(l.id);
                changed |= r.changed();
                ui.end_row();
                // Completion of the address being typed (after the last ';' or ',').
                if label != T::Subject
                    && r.has_focus()
                    && let Some(suggest) = c.suggest
                {
                    let cut = value.rfind([';', ',']).map_or(0, |i| i + 1);
                    let typed = value[cut..].trim().to_owned();
                    let found = suggest(account, &typed);
                    if !found.is_empty()
                        && !found.iter().any(|(_, a)| a.eq_ignore_ascii_case(&typed))
                    {
                        ui.label("");
                        ui.vertical(|ui| {
                            for (name, addr) in found {
                                let text = if name.is_empty() {
                                    addr.clone()
                                } else {
                                    format!("{name} <{addr}>")
                                };
                                if ui.selectable_label(false, &text).clicked() {
                                    value.truncate(cut);
                                    if cut > 0 {
                                        value.push(' ');
                                    }
                                    value.push_str(&text);
                                    value.push_str("; ");
                                    changed = true;
                                    r.request_focus();
                                }
                            }
                        });
                        ui.end_row();
                    }
                }
            };
            field(ui, T::To, &mut comp.to);
            field(ui, T::Cc, &mut comp.cc);
            if comp.show_bcc {
                field(ui, T::Bcc, &mut comp.bcc);
            }
            field(ui, T::Subject, &mut comp.subject);
        });
    if !comp.show_bcc && ui.small_button(T::ShowBcc.get(lang)).clicked() {
        comp.show_bcc = true;
    }
    if changed {
        comp.touch();
    }

    // Attachments.
    let mut remove = None;
    if !comp.attachments.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (i, path) in comp.attachments.iter().enumerate() {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                ui.label(RichText::new(name).small());
                if ui
                    .small_button("×")
                    .on_hover_text(T::Remove.get(lang))
                    .clicked()
                {
                    remove = Some(i);
                }
            }
        });
    }
    if let Some(i) = remove {
        comp.attachments.remove(i);
        comp.touch();
    }
    // Files dropped on the window join the message.
    let dropped: Vec<PathBuf> = ui.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| p.is_file())
            .collect()
    });
    if !dropped.is_empty() {
        comp.attachments.extend(dropped);
        comp.touch();
    }

    format_bar(ui, c, comp);
    ui.separator();
    editor(ui, c, comp);

    if !comp.quoted.is_empty() {
        ui.add_space(8.0);
        egui::CollapsingHeader::new(&comp.quoted_header)
            .id_salt(("quoted", comp.id))
            .show(ui, |ui| {
                let none = |_: &str| None;
                let ready = std::collections::HashSet::new();
                let images = crate::reader::Images {
                    block_remote: true,
                    ready: &ready,
                    inline_uri: &none,
                };
                crate::reader::show(ui, c, &comp.quoted, &images);
            });
    }
    action
}

fn format_bar(ui: &mut Ui, c: &Ctx<'_>, comp: &mut Composer) {
    let lang = c.lang;
    // Shortcuts work while typing in the editor.
    let (b, i, u, k) = ui.input_mut(|inp| {
        (
            inp.consume_key(Modifiers::COMMAND, Key::B),
            inp.consume_key(Modifiers::COMMAND, Key::I),
            inp.consume_key(Modifiers::COMMAND, Key::U),
            inp.consume_key(Modifiers::COMMAND, Key::K),
        )
    });
    let current = comp.typing_style();
    ui.horizontal(|ui| {
        let toggle = |ui: &mut Ui, text: RichText, on: bool, tip: T| {
            ui.add(
                egui::Button::new(text)
                    .selected(on)
                    .min_size(egui::vec2(26.0, 22.0)),
            )
            .on_hover_text(tip.get(lang))
            .clicked()
        };
        if toggle(
            ui,
            RichText::new(T::BoldLetter.get(lang)).strong(),
            current.bold,
            T::Bold,
        ) || b
        {
            comp.apply(|s, on| s.bold = on, |s| s.bold);
        }
        if toggle(
            ui,
            RichText::new(T::ItalicLetter.get(lang)).italics(),
            current.italic,
            T::Italic,
        ) || i
        {
            comp.apply(|s, on| s.italic = on, |s| s.italic);
        }
        if toggle(
            ui,
            RichText::new(T::UnderlineLetter.get(lang)).underline(),
            current.underline,
            T::Underline,
        ) || u
        {
            comp.apply(|s, on| s.underline = on, |s| s.underline);
        }
        ui.separator();
        let size_label = |s: Size| match s {
            Size::Small => T::SizeSmall,
            Size::Normal => T::SizeNormal,
            Size::Large => T::SizeLarge,
            Size::Huge => T::SizeHuge,
        };
        egui::ComboBox::from_id_salt(("size", comp.id))
            .width(90.0)
            .selected_text(size_label(current.size).get(lang))
            .show_ui(ui, |ui| {
                for s in [Size::Small, Size::Normal, Size::Large, Size::Huge] {
                    if ui
                        .selectable_label(current.size == s, size_label(s).get(lang))
                        .clicked()
                    {
                        comp.apply(move |st, _| st.size = s, move |st| st.size == s);
                    }
                }
            })
            .response
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::ComboBox, true, T::TextSize.get(lang))
            });
        let color_button = ui.button(T::Color.get(lang));
        egui::Popup::menu(&color_button).show(|ui| {
            ui.horizontal(|ui| {
                if ui.button(T::DefaultColor.get(lang)).clicked() {
                    comp.apply(|s, _| s.color = None, |s| s.color.is_none());
                }
                for (r, g, b) in COLORS {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                    ui.painter()
                        .rect_filled(rect, 3.0, Color32::from_rgb(r, g, b));
                    let resp = resp.on_hover_text(format!("#{r:02x}{g:02x}{b:02x}"));
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            true,
                            format!("#{r:02x}{g:02x}{b:02x}"),
                        )
                    });
                    if resp.clicked() {
                        comp.apply(
                            move |s, _| s.color = Some([r, g, b]),
                            move |s| s.color == Some([r, g, b]),
                        );
                    }
                }
            });
        });
        ui.separator();
        let (a, z) = comp.selection;
        if ui.button(T::BulletList.get(lang)).clicked() {
            comp.doc.toggle_list(a..z, false);
            comp.touch();
        }
        if ui.button(T::NumberedList.get(lang)).clicked() {
            comp.doc.toggle_list(a..z, true);
            comp.touch();
        }
        if ui
            .button(T::Link.get(lang))
            .on_hover_text("Ctrl+K")
            .clicked()
            || k
        {
            comp.link = Some(String::new());
        }
    });
    if let Some(url) = comp.link.as_mut() {
        let mut done = None;
        ui.horizontal(|ui| {
            let l = ui.label(T::LinkAddress.get(lang));
            let r = ui
                .add(
                    TextEdit::singleline(url)
                        .hint_text("https://")
                        .desired_width(320.0),
                )
                .labelled_by(l.id);
            r.request_focus();
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui.button(T::Ok.get(lang)).clicked() || enter {
                done = Some(true);
            }
            if ui.button(T::Cancel.get(lang)).clicked() {
                done = Some(false);
            }
        });
        if let Some(ok) = done {
            let url = comp.link.take().unwrap_or_default();
            if ok && let Some(link) = clean_link(&url) {
                let link: std::sync::Arc<str> = link.into();
                comp.apply(move |s, _| s.link = Some(link.clone()), |_| false);
            }
        }
    }
}

/// Accepts http(s) and mailto links; a bare domain gets `https://`.
pub fn clean_link(url: &str) -> Option<String> {
    let u = url.trim();
    if u.is_empty() || u.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
    {
        Some(u.to_owned())
    } else if u.contains('.') && !u.contains(':') {
        Some(format!("https://{u}"))
    } else {
        None
    }
}

fn editor(ui: &mut Ui, c: &Ctx<'_>, comp: &mut Composer) {
    let typing = comp.typing_style();
    let old = comp.doc.text.clone();
    let text_color = ui.visuals().text_color();
    let accent = c.accent;
    let Doc { text, styles } = &mut comp.doc;
    let styles_now: &Vec<Style> = styles;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = layout_job(buf.as_str(), styles_now, text_color, accent);
        job.wrap.max_width = wrap;
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    let out = TextEdit::multiline(text)
        .id_salt(("editor", comp.id))
        .desired_width(f32::INFINITY)
        .desired_rows(14)
        .lock_focus(true)
        .layouter(&mut layouter)
        .show(ui);
    out.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, "message"));
    let edited = out.response.changed();
    if edited {
        comp.doc.sync_after_edit(&old, &typing);
        comp.touch();
    }
    if let Some(range) = out.cursor_range {
        let r = range.as_sorted_char_range();
        let sel = (r.start.0, r.end.0);
        if comp.last_cursor != Some(sel) && !edited {
            comp.pending = None;
        }
        comp.last_cursor = Some(sel);
        comp.selection = sel;
    }
}

fn layout_job(text: &str, styles: &[Style], color: Color32, accent: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut run_start = 0usize; // byte index
    let mut run_style: Option<&Style> = None;
    let default = Style::default();
    for (i, (byte, _)) in text.char_indices().enumerate() {
        let s = styles.get(i).unwrap_or(&default);
        if run_style != Some(s) {
            if let Some(prev) = run_style {
                job.append(&text[run_start..byte], 0.0, format(prev, color, accent));
            }
            run_start = byte;
            run_style = Some(s);
        }
    }
    let last = run_style.unwrap_or(&default);
    job.append(&text[run_start..], 0.0, format(last, color, accent));
    job
}

fn format(s: &Style, color: Color32, accent: Color32) -> TextFormat {
    let family = if s.bold {
        FontFamily::Name("semibold".into())
    } else {
        FontFamily::Proportional
    };
    let mut f = TextFormat::simple(FontId::new(s.size.points(), family), color);
    if let Some([r, g, b]) = s.color {
        f.color = Color32::from_rgb(r, g, b);
    }
    if s.link.is_some() {
        f.color = accent;
    }
    f.italics = s.italic;
    if s.underline || s.link.is_some() {
        f.underline = Stroke::new(1.0, f.color);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::clean_link;

    #[test]
    fn links() {
        assert_eq!(
            clean_link(" example.com ").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            clean_link("mailto:a@example.com").as_deref(),
            Some("mailto:a@example.com")
        );
        assert_eq!(clean_link("javascript:alert(1)"), None);
        assert_eq!(clean_link("file:///C:/x"), None);
        assert_eq!(clean_link("a b.com"), None);
    }
}
