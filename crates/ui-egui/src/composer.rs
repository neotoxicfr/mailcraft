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
    /// Unknown words of the text (byte ranges), for the text they were computed on.
    spell: Option<(u64, Vec<std::ops::Range<usize>>)>,
}

/// What the composer asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Send,
    PickAttachments,
    Detach,
    /// Close; `Some(true)` keeps the draft, `Some(false)` discards it, `None` asks first.
    Close(Option<bool>),
    /// Add a word to the user's dictionary.
    LearnWord(String),
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
            spell: None,
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
    if comp.detached {
        // A window of its own: Outlook's "Message" ribbon, closed with the window's own button.
        if let Some(a) = message_ribbon(ui, c, comp) {
            action = Some(a);
        }
    } else {
        // Outlook layout: a small bar (attach, detach, close, state), then the big Send button
        // on the left of the address and subject fields.
        ui.horizontal(|ui| {
            if ui.button(T::Attach.get(lang)).clicked() {
                action = Some(Action::PickAttachments);
            }
            if comp.sending {
                ui.spinner();
                ui.label(RichText::new(T::Sending.get(lang)).weak());
            } else if comp.draft_id.is_some() && comp.changed_at.is_none() {
                ui.label(RichText::new(T::DraftSaved.get(lang)).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(T::Discard.get(lang)).clicked() {
                    action = Some(Action::Close(None));
                }
                if !comp.detached && ui.button(T::Detach.get(lang)).clicked() {
                    action = Some(Action::Detach);
                }
            });
        });
    }
    if let Some(e) = &comp.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    ui.add_space(4.0);

    let mut changed = false;
    let rows = if comp.show_bcc { 5.0 } else { 4.0 };
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
        if send_button(ui, c, !comp.sending, rows * 30.0 - 6.0) {
            action = Some(Action::Send);
        }
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            from_row(ui, c, comp);
            let account = comp.account_id;
            let mut field = |ui: &mut Ui, label: T, value: &mut String, button: bool| {
                ui.horizontal(|ui| {
                    // Outlook's "To…" / "Cc…" buttons in front of the address fields.
                    let l = if button {
                        let b = ui.add_sized(
                            [64.0, 24.0],
                            egui::Button::new(format!("{}…", label.get(lang))),
                        );
                        b.on_hover_text(T::FocusField.get(lang)).id
                    } else {
                        ui.add_sized([64.0, 24.0], egui::Label::new(label.get(lang)))
                            .id
                    };
                    let r = underlined(ui, value).labelled_by(l);
                    if ui.ctx().memory(|m| m.focused()) == Some(l) {
                        r.request_focus();
                    }
                    changed |= r.changed();
                    // Completion of the address being typed (after the last ';' or ',').
                    if button
                        && r.has_focus()
                        && let Some(suggest) = c.suggest
                    {
                        let cut = value.rfind([';', ',']).map_or(0, |i| i + 1);
                        let typed = value[cut..].trim().to_owned();
                        let found = suggest(account, &typed);
                        if !found.is_empty()
                            && !found.iter().any(|(_, a)| a.eq_ignore_ascii_case(&typed))
                        {
                            egui::Area::new(r.id.with("complete"))
                                .fixed_pos(r.rect.left_bottom())
                                .order(egui::Order::Foreground)
                                .show(ui.ctx(), |ui| {
                                    egui::Frame::popup(ui.style()).show(ui, |ui| {
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
                                });
                        }
                    }
                });
            };
            field(ui, T::To, &mut comp.to, true);
            field(ui, T::Cc, &mut comp.cc, true);
            if comp.show_bcc {
                field(ui, T::Bcc, &mut comp.bcc, true);
            }
            field(ui, T::Subject, &mut comp.subject, false);
        });
    });
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

    if !comp.detached {
        ui.add_space(4.0);
        format_bar(ui, c, comp, false);
    }
    ui.add_space(4.0);
    ui.separator();
    if let Some(a) = editor(ui, c, comp) {
        action = Some(a);
    }

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

/// A frameless single-line field with a line under it (Outlook's address and subject fields).
fn underlined(ui: &mut Ui, value: &mut String) -> egui::Response {
    let r = ui.add(
        TextEdit::singleline(value)
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(4, 4)),
    );
    let color = if r.has_focus() {
        ui.visuals().selection.bg_fill
    } else {
        crate::theme::CONTROL_BORDER
    };
    ui.painter()
        .hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, color));
    r
}

/// "From": the sending account; a choice when there are several, until the draft exists on a
/// server (it belongs to that account then).
fn from_row(ui: &mut Ui, c: &Ctx<'_>, comp: &mut Composer) {
    let lang = c.lang;
    let accounts = &c.mail.accounts;
    let current = accounts
        .iter()
        .find(|a| a.id == comp.account_id)
        .map_or_else(String::new, |a| a.email.clone());
    ui.horizontal(|ui| {
        let l = ui
            .add_sized([64.0, 24.0], egui::Label::new(T::From.get(lang)))
            .id;
        if accounts.len() > 1 && comp.draft_id.is_none() {
            egui::ComboBox::from_id_salt(("from", comp.id))
                .width(ui.available_width().min(360.0))
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for a in accounts {
                        ui.selectable_value(&mut comp.account_id, a.id, &a.email);
                    }
                })
                .response
                .labelled_by(l);
        } else {
            ui.label(current).labelled_by(l);
        }
        if !comp.show_bcc
            && ui
                .add(egui::Button::new(T::ShowBcc.get(lang)).frame(false))
                .clicked()
        {
            comp.show_bcc = true;
        }
    });
}

/// A small ribbon tool: an icon, highlighted when on or hovered, its name as tooltip and for
/// screen readers.
fn tool(ui: &mut Ui, icon: crate::icons::Icon, name: &str, on: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(28.0, 26.0), egui::Sense::click());
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
    paint_tool_bg(ui, rect, &r, on);
    icon.paint(ui.painter(), rect.shrink(6.0), ui.visuals().text_color());
    r.on_hover_text(name)
}

fn paint_tool_bg(ui: &Ui, rect: egui::Rect, r: &egui::Response, on: bool) {
    let v = ui.visuals();
    if on {
        ui.painter()
            .rect_filled(rect, 4.0, v.selection.bg_fill.gamma_multiply(0.25));
    } else if r.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
    }
}

/// Bold / italic / underline: the letter itself, styled.
fn letter_tool(ui: &mut Ui, letter: &str, name: &str, on: bool, style: u8) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(28.0, 26.0), egui::Sense::click());
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
    paint_tool_bg(ui, rect, &r, on);
    let color = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    let mut format = TextFormat::simple(
        FontId::new(
            15.0,
            if style == 0 {
                FontFamily::Name("semibold".into())
            } else {
                FontFamily::Proportional
            },
        ),
        color,
    );
    format.italics = style == 1;
    if style == 2 {
        format.underline = Stroke::new(1.0, color);
    }
    job.append(letter, 0.0, format);
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    r.on_hover_text(name)
}

/// The ribbon of a message window, like Outlook's "Message" tab: the tab strip in the accent
/// colour, then Basic Text (the format bar) and Include (attach a file).
fn message_ribbon(ui: &mut Ui, c: &Ctx<'_>, comp: &mut Composer) -> Option<Action> {
    let lang = c.lang;
    let dark = ui.visuals().dark_mode;
    let mut action = None;
    // Tab strip.
    let (strip, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
    ui.painter().rect_filled(strip, 0.0, c.accent_fill);
    let tab = T::TabMessage.get(lang);
    let galley = ui
        .painter()
        .layout_no_wrap(tab.to_owned(), FontId::proportional(13.0), c.accent);
    let tab_rect = egui::Rect::from_min_size(
        strip.min + egui::vec2(8.0, 4.0),
        egui::vec2(galley.size().x + 24.0, 24.0),
    );
    ui.painter()
        .rect_filled(tab_rect, 0.0, crate::theme::ribbon_fill(dark));
    ui.painter()
        .galley(tab_rect.center() - galley.size() / 2.0, galley, c.accent);
    // Groups.
    egui::Frame::NONE
        .fill(crate::theme::ribbon_fill(dark))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            // Full width and the groups' height, captions included.
            ui.set_min_size(egui::vec2(ui.available_width(), 86.0));
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 86.0),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    crate::shell::group(ui, c, T::GroupBasicText, |ui| {
                        ui.vertical(|ui| format_bar(ui, c, comp, true));
                    });
                    crate::shell::group(ui, c, T::GroupInclude, |ui| {
                        if crate::shell::big_button(
                            ui,
                            c,
                            crate::icons::Icon::Paperclip,
                            T::AttachBig,
                            true,
                            "",
                        )
                        .clicked()
                        {
                            action = Some(Action::PickAttachments);
                        }
                        if crate::shell::big_button(
                            ui,
                            c,
                            crate::icons::Icon::Link,
                            T::Link,
                            true,
                            "",
                        )
                        .on_hover_text("Ctrl+K")
                        .clicked()
                        {
                            comp.link = Some(String::new());
                        }
                    });
                    if comp.sending {
                        ui.spinner();
                        ui.label(RichText::new(T::Sending.get(lang)).weak());
                    } else if comp.draft_id.is_some() && comp.changed_at.is_none() {
                        ui.label(RichText::new(T::DraftSaved.get(lang)).weak());
                    }
                },
            );
        });
    ui.add_space(6.0);
    action
}

/// The formatting tools; `ribbon`: on two rows (size and colour above, styles and lists
/// below) with the link in the Include group.
fn format_bar(ui: &mut Ui, c: &Ctx<'_>, comp: &mut Composer, ribbon: bool) {
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
    let size_label = |s: Size| match s {
        Size::Small => T::SizeSmall,
        Size::Normal => T::SizeNormal,
        Size::Large => T::SizeLarge,
        Size::Huge => T::SizeHuge,
    };
    let size_and_colour = |ui: &mut Ui, comp: &mut Composer| {
        egui::ComboBox::from_id_salt(("size", comp.id))
            .width(110.0)
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
        // Font colour: an "A" over a bar of the current colour, the palette below.
        let (rect, colour_button) =
            ui.allocate_exact_size(egui::vec2(34.0, 26.0), egui::Sense::click());
        colour_button.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, T::Color.get(lang))
        });
        paint_tool_bg(ui, rect, &colour_button, false);
        let text = ui.visuals().text_color();
        let bar = current
            .color
            .map_or(Color32::from_rgb(0xC5, 0x0F, 0x1F), |[r, g, b]| {
                Color32::from_rgb(r, g, b)
            });
        ui.painter().text(
            rect.center() - egui::vec2(5.0, 3.0),
            egui::Align2::CENTER_CENTER,
            "A",
            FontId::new(15.0, FontFamily::Name("semibold".into())),
            text,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                rect.left_bottom() + egui::vec2(5.0, -7.0),
                egui::vec2(16.0, 4.0),
            ),
            0.0,
            bar,
        );
        // Small arrow: this palette opens below.
        let tip = rect.right_center() - egui::vec2(6.0, 0.0);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                tip + egui::vec2(-3.0, -1.5),
                tip + egui::vec2(3.0, -1.5),
                tip + egui::vec2(0.0, 2.0),
            ],
            text,
            Stroke::NONE,
        ));
        let colour_button = colour_button.on_hover_text(T::Color.get(lang));
        egui::Popup::menu(&colour_button).show(|ui| {
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
    };
    let styles_and_lists = |ui: &mut Ui, comp: &mut Composer| {
        if letter_tool(
            ui,
            T::BoldLetter.get(lang),
            T::Bold.get(lang),
            current.bold,
            0,
        )
        .clicked()
            || b
        {
            comp.apply(|s, on| s.bold = on, |s| s.bold);
        }
        if letter_tool(
            ui,
            T::ItalicLetter.get(lang),
            T::Italic.get(lang),
            current.italic,
            1,
        )
        .clicked()
            || i
        {
            comp.apply(|s, on| s.italic = on, |s| s.italic);
        }
        if letter_tool(
            ui,
            T::UnderlineLetter.get(lang),
            T::Underline.get(lang),
            current.underline,
            2,
        )
        .clicked()
            || u
        {
            comp.apply(|s, on| s.underline = on, |s| s.underline);
        }
        ui.separator();
        let (a, z) = comp.selection;
        if tool(
            ui,
            crate::icons::Icon::Bullets,
            T::BulletList.get(lang),
            false,
        )
        .clicked()
        {
            comp.doc.toggle_list(a..z, false);
            comp.touch();
        }
        if tool(
            ui,
            crate::icons::Icon::Numbering,
            T::NumberedList.get(lang),
            false,
        )
        .clicked()
        {
            comp.doc.toggle_list(a..z, true);
            comp.touch();
        }
    };
    if ribbon {
        ui.horizontal(|ui| size_and_colour(ui, comp));
        ui.horizontal(|ui| styles_and_lists(ui, comp));
        if k {
            comp.link = Some(String::new());
        }
    } else {
        ui.horizontal(|ui| {
            styles_and_lists(ui, comp);
            ui.separator();
            size_and_colour(ui, comp);
            ui.separator();
            if tool(ui, crate::icons::Icon::Link, T::Link.get(lang), false)
                .on_hover_text("Ctrl+K")
                .clicked()
                || k
            {
                comp.link = Some(String::new());
            }
        });
    }
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

/// Outlook's tall Send button: envelope and label in white on the accent. True when clicked.
fn send_button(ui: &mut Ui, c: &Ctx<'_>, enabled: bool, height: f32) -> bool {
    let label = T::Send.get(c.lang);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(72.0, height.max(64.0)), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let fill = if !enabled {
        ui.visuals().widgets.noninteractive.bg_fill
    } else if response.is_pointer_button_down_on() {
        c.accent_fill.gamma_multiply(0.8)
    } else if response.hovered() {
        c.accent_fill.gamma_multiply(0.9)
    } else {
        c.accent_fill
    };
    let ink = if enabled {
        Color32::WHITE
    } else {
        ui.visuals().weak_text_color()
    };
    ui.painter().rect_filled(rect, 4.0, fill);
    let icon = egui::Rect::from_center_size(
        rect.center() - egui::vec2(0.0, 10.0),
        egui::Vec2::splat(24.0),
    );
    crate::icons::Icon::Envelope.paint(ui.painter(), icon, ink);
    ui.painter().text(
        rect.center() + egui::vec2(0.0, 16.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        ink,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            5.0,
            Stroke::new(2.0, c.accent),
            egui::StrokeKind::Outside,
        );
    }
    response.clicked()
}

fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn editor(ui: &mut Ui, c: &Ctx<'_>, comp: &mut Composer) -> Option<Action> {
    let mut action = None;
    let typing = comp.typing_style();
    let old = comp.doc.text.clone();
    let text_color = ui.visuals().text_color();
    let accent = c.accent;
    // Unknown words, computed again only when the text changed.
    let hash = text_hash(&comp.doc.text);
    if comp.spell.as_ref().is_none_or(|(h, _)| *h != hash) {
        let own: std::collections::HashSet<String> =
            c.own_words.iter().map(|w| w.to_lowercase()).collect();
        let bad = crate::spell::misspelled(&comp.doc.text, &own);
        // Dictionaries still loading: try again next frame.
        if crate::spell::ready() {
            comp.spell = Some((hash, bad));
        }
    }
    let spell = comp.spell.clone();
    let Doc { text, styles } = &mut comp.doc;
    let styles_now: &Vec<Style> = styles;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = layout_job(buf.as_str(), styles_now, text_color, accent);
        if let Some((h, bad)) = &spell
            && *h == text_hash(buf.as_str())
        {
            underline(&mut job, bad);
        }
        job.wrap.max_width = wrap;
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    let out = TextEdit::multiline(text)
        .frame(egui::Frame::NONE)
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
    // Right-click on an underlined word (caret in it): corrections, or learn the word.
    let caret = comp
        .doc
        .text
        .char_indices()
        .nth(comp.selection.0)
        .map_or(comp.doc.text.len(), |(b, _)| b);
    let word = crate::spell::word_at(&comp.doc.text, caret).filter(|w| {
        comp.spell
            .as_ref()
            .is_some_and(|(_, bad)| bad.iter().any(|b| b.start <= w.start && w.end <= b.end))
    });
    if let Some(range) = word {
        let lang = c.lang;
        let shown = comp.doc.text[range.clone()].to_owned();
        out.response.context_menu(|ui| {
            let fixes = crate::spell::suggestions(&shown);
            if fixes.is_empty() {
                ui.label(egui::RichText::new(T::SpellNoSuggestion.get(lang)).weak());
            }
            for fix in fixes {
                if ui.button(&fix).clicked() {
                    let before = comp.doc.text.clone();
                    comp.doc.text.replace_range(range.clone(), &fix);
                    comp.doc.sync_after_edit(&before, &typing);
                    comp.touch();
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(T::SpellLearn.get(lang)).clicked() {
                action = Some(Action::LearnWord(shown.clone()));
                comp.spell = None;
                ui.close();
            }
        });
    }
    action
}

/// Red underline under the unknown words (byte ranges), splitting the styled runs as needed.
fn underline(job: &mut LayoutJob, bad: &[std::ops::Range<usize>]) {
    if bad.is_empty() {
        return;
    }
    let red = Stroke::new(1.5, Color32::from_rgb(0xD1, 0x34, 0x38));
    let mut sections = Vec::with_capacity(job.sections.len() + bad.len() * 2);
    for s in std::mem::take(&mut job.sections) {
        let (start, end): (usize, usize) = (s.byte_range.start.into(), s.byte_range.end.into());
        let mut cuts = vec![start, end];
        for b in bad {
            for x in [b.start, b.end] {
                if start < x && x < end {
                    cuts.push(x);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for (i, w) in cuts.windows(2).enumerate() {
            let mut part = s.clone();
            part.byte_range = egui::text::ByteIndex(w[0])..egui::text::ByteIndex(w[1]);
            if i > 0 {
                part.leading_space = 0.0;
            }
            if bad.iter().any(|b| b.start <= w[0] && w[1] <= b.end) {
                part.format.underline = red;
            }
            sections.push(part);
        }
    }
    job.sections = sections;
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
