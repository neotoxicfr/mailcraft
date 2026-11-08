//! The three panes under the ribbon (folders, message list, reading pane) and the status bar.

use chrono::{DateTime, Datelike, Local, TimeZone};
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Color32, FontFamily, FontId, Layout, Rect, RichText, Role, ScrollArea, Sense, Stroke,
    TextEdit, TextFormat, Ui, Vec2, WidgetInfo, pos2, vec2,
};
use mailcraft_store::{Folder, MessageSummary};

use crate::i18n::{Lang, T};
use crate::icons::Icon;
use crate::mail::Status;
use crate::shell::{Command, Ctx, Dialog, Pick, View};
use crate::theme;

const FOLDER_ROW: f32 = 24.0;
const MESSAGE_ROW: f32 = 60.0;
const GROUP_ROW: f32 = 26.0;

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

/// One line of text cut with an ellipsis at `width`.
fn one_line(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(
        text.replace(['\r', '\n'], " "),
        TextFormat::simple(font, color),
    );
    job.wrap = TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

// ---- folder pane ----

/// Drag-and-drop payload: the selected messages are being dragged.
pub struct DraggedMessages;

pub fn folder_pane(
    ui: &mut Ui,
    c: &Ctx<'_>,
    dialog: &mut Option<Dialog>,
    command: &mut Option<Command>,
) {
    let mail = c.mail;
    if mail.accounts.is_empty() {
        ui.label(RichText::new(T::Favorites.get(c.lang)).strong());
        ui.add_space(12.0);
        ui.label(RichText::new(T::NoAccount.get(c.lang)).weak());
        ui.add_space(8.0);
        if ui
            .add_enabled(
                !mail.signing_in,
                egui::Button::new(T::AddAccount.get(c.lang).replace('\n', " ")),
            )
            .clicked()
        {
            *command = Some(Command::AddAccount);
        }
        return;
    }
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        section_title(ui, T::Favorites.get(c.lang));
        for f in mail.folders.iter().filter(|f| {
            matches!(
                f.role.as_deref(),
                Some("inbox" | "sentitems" | "deleteditems")
            )
        }) {
            folder_row(ui, c, f, 1, dialog, command);
        }
        for account in &mail.accounts {
            ui.add_space(8.0);
            section_title(ui, &account.email);
            for (f, depth) in mail.folder_tree(account.id) {
                folder_row(ui, c, f, depth, dialog, command);
            }
        }
    });
}

fn section_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(semibold(13.0)));
}

fn folder_row(
    ui: &mut Ui,
    c: &Ctx<'_>,
    f: &Folder,
    depth: usize,
    dialog: &mut Option<Dialog>,
    command: &mut Option<Command>,
) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, FOLDER_ROW), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let selected = c
        .mail
        .folder
        .as_ref()
        .is_some_and(|(a, id)| *a == f.account_id && id == &f.id);
    response.widget_info(|| WidgetInfo::selected(Role::TreeItem, true, selected, &f.name));
    // Messages can only be dropped on a folder of their own account.
    let same_account = c
        .mail
        .folder
        .as_ref()
        .is_some_and(|(a, _)| *a == f.account_id);
    let drop_here = same_account && response.dnd_hover_payload::<DraggedMessages>().is_some();
    paint_selection(ui, c, rect, selected || drop_here, response.hovered());
    if drop_here {
        ui.painter().rect_stroke(
            rect,
            2.0,
            Stroke::new(1.5, c.accent),
            egui::StrokeKind::Inside,
        );
    }
    let x = rect.left() + 6.0 + depth as f32 * 12.0;
    let text = ui.visuals().text_color();
    let (font, count_text) = if f.unread > 0 {
        (semibold(13.0), f.unread.to_string())
    } else {
        (FontId::proportional(13.0), String::new())
    };
    let count = one_line(ui, &count_text, semibold(12.0), c.accent, 60.0);
    let name = one_line(
        ui,
        &f.name,
        font,
        text,
        rect.right() - x - count.size().x - 12.0,
    );
    ui.painter()
        .galley(pos2(x, rect.center().y - name.size().y / 2.0), name, text);
    let w = count.size().x;
    ui.painter().galley(
        pos2(
            rect.right() - w - 6.0,
            rect.center().y - count.size().y / 2.0,
        ),
        count,
        c.accent,
    );

    if same_account && response.dnd_release_payload::<DraggedMessages>().is_some() {
        *command = Some(Command::MoveTo {
            folder_id: f.id.clone(),
        });
    } else if response.clicked() {
        *command = Some(Command::SelectFolder {
            account_id: f.account_id,
            folder_id: f.id.clone(),
        });
    }
    let lang = c.lang;
    response.context_menu(|ui| {
        if ui.button(T::NewSubfolder.get(lang)).clicked() {
            *dialog = Some(crate::shell::folder_name_dialog(
                f.account_id,
                Some(f.id.clone()),
                None,
                "",
            ));
            ui.close();
        }
        let own = f.role.is_none();
        if ui
            .add_enabled(own, egui::Button::new(T::RenameFolder.get(lang)))
            .clicked()
        {
            *dialog = Some(crate::shell::folder_name_dialog(
                f.account_id,
                None,
                Some(f.id.clone()),
                &f.name,
            ));
            ui.close();
        }
        if ui
            .add_enabled(own, egui::Button::new(T::DeleteFolder.get(lang)))
            .clicked()
        {
            *dialog = Some(Dialog::ConfirmDeleteFolder {
                account_id: f.account_id,
                id: f.id.clone(),
                name: f.name.clone(),
            });
            ui.close();
        }
    });
}

fn paint_selection(ui: &Ui, c: &Ctx<'_>, rect: Rect, selected: bool, hovered: bool) {
    if selected {
        ui.painter()
            .rect_filled(rect, 2.0, c.accent.gamma_multiply(0.22));
    } else if hovered {
        ui.painter()
            .rect_filled(rect, 2.0, c.accent.gamma_multiply(0.08));
    }
}

// ---- message list ----

pub fn message_list(
    ui: &mut Ui,
    c: &Ctx<'_>,
    view: &mut View,
    dark: bool,
    command: &mut Option<Command>,
) {
    let search = ui.add(
        TextEdit::singleline(&mut view.search)
            .hint_text(T::SearchHint.get(c.lang))
            .desired_width(f32::INFINITY),
    );
    if search.changed() {
        *command = Some(Command::SearchChanged);
    }
    if (search.has_focus() || search.lost_focus())
        && ui.input(|i| i.key_pressed(egui::Key::Escape))
        && !view.search.is_empty()
    {
        view.search.clear();
        *command = Some(Command::SearchChanged);
    }
    if c.mail.searching {
        let enabled = c.mail.status != Status::SearchingServer;
        if ui
            .add_enabled(enabled, egui::Link::new(T::SearchServer.get(c.lang)))
            .clicked()
        {
            *command = Some(Command::SearchServer);
        }
    }
    ui.add_space(4.0);
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        Stroke::new(1.0, theme::separator(dark)),
    );
    ui.add_space(2.0);

    let mail = c.mail;
    if mail.messages.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(T::EmptyList.get(c.lang)).weak()));
        return;
    }

    let now = Local::now();
    let mut rows: Vec<(f32, Row<'_>)> = Vec::with_capacity(mail.messages.len() + 8);
    let mut y = 0.0;
    let mut last = None;
    for m in &mail.messages {
        let group = date_group(m.received, now);
        if last != Some(group) {
            rows.push((y, Row::Group(group)));
            y += GROUP_ROW;
            last = Some(group);
        }
        rows.push((y, Row::Message(m)));
        y += MESSAGE_ROW;
    }
    let total = y;

    ScrollArea::vertical()
        .auto_shrink(false)
        .show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let origin = ui.min_rect().min;
            let width = ui.available_width();
            let first = rows.partition_point(|(top, _)| *top + MESSAGE_ROW < viewport.min.y);
            for (top, row) in &rows[first..] {
                if *top > viewport.max.y {
                    break;
                }
                let h = if matches!(row, Row::Group(_)) {
                    GROUP_ROW
                } else {
                    MESSAGE_ROW
                };
                let rect = Rect::from_min_size(origin + vec2(0.0, *top), vec2(width, h));
                match row {
                    Row::Group(g) => {
                        ui.painter().text(
                            rect.left_center() + vec2(4.0, 2.0),
                            egui::Align2::LEFT_CENTER,
                            g.get(c.lang),
                            semibold(12.0),
                            ui.visuals().weak_text_color(),
                        );
                    }
                    Row::Message(m) => message_row(ui, c, rect, m, now, command),
                }
            }
        });
}

enum Row<'a> {
    Group(T),
    Message(&'a MessageSummary),
}

fn message_row(
    ui: &mut Ui,
    c: &Ctx<'_>,
    rect: Rect,
    m: &MessageSummary,
    now: DateTime<Local>,
    command: &mut Option<Command>,
) {
    let id = ui.scope_id().with(("msg", m.account_id, &m.id));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let sender = if m.from_name.is_empty() {
        &m.from_addr
    } else {
        &m.from_name
    };
    let subject = if m.subject.is_empty() {
        T::NoSubject.get(c.lang)
    } else {
        &m.subject
    };
    response
        .widget_info(|| WidgetInfo::labeled(Role::ListItem, true, format!("{sender} — {subject}")));
    let selected = c.mail.is_selected(m.account_id, &m.id);
    paint_selection(
        ui,
        c,
        rect.shrink2(vec2(0.0, 1.0)),
        selected,
        response.hovered(),
    );
    let unread = !m.is_read;
    if unread {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 4.0), vec2(3.0, rect.height() - 8.0)),
            0.0,
            c.accent,
        );
    }
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let left = rect.left() + 10.0;
    let right = rect.right() - 6.0;

    let date = one_line(
        ui,
        &short_date(m.received, now, c.lang),
        FontId::proportional(12.0),
        if unread { c.accent } else { weak },
        120.0,
    );
    let date_w = date.size().x;
    ui.painter()
        .galley(pos2(right - date_w, rect.top() + 6.0), date, weak);
    let sender_font = if unread {
        semibold(13.5)
    } else {
        FontId::proportional(13.5)
    };
    let s = one_line(ui, sender, sender_font, text, right - date_w - left - 8.0);
    ui.painter().galley(pos2(left, rect.top() + 5.0), s, text);

    let mut icons_w = 0.0;
    for (on, icon, color) in [
        (m.flagged, Icon::Flag, Color32::from_rgb(0xD1, 0x34, 0x38)),
        (m.has_attachments, Icon::Paperclip, weak),
    ] {
        if on {
            icons_w += 16.0;
            icon.paint(
                ui.painter(),
                Rect::from_min_size(pos2(right - icons_w, rect.top() + 23.0), Vec2::splat(13.0)),
                color,
            );
        }
    }
    let subject_color = if unread { c.accent } else { text };
    let sub = one_line(
        ui,
        subject,
        FontId::proportional(12.5),
        subject_color,
        right - left - icons_w - 4.0,
    );
    ui.painter()
        .galley(pos2(left, rect.top() + 23.0), sub, subject_color);
    // Search results come from every folder: say which one.
    let third = if c.mail.searching {
        format!(
            "{} · {}",
            c.mail.folder_name(m.account_id, &m.folder_id),
            m.preview
        )
    } else {
        m.preview.clone()
    };
    let pre = one_line(ui, &third, FontId::proportional(12.0), weak, right - left);
    ui.painter()
        .galley(pos2(left, rect.top() + 40.0), pre, weak);

    let select = |pick| Command::SelectMessage {
        account_id: m.account_id,
        id: m.id.clone(),
        pick,
    };
    if response.drag_started() {
        // Dragging a message outside the selection drags that message alone.
        if !selected {
            *command = Some(select(Pick::Only));
        }
        response.dnd_set_drag_payload(DraggedMessages);
    } else if response.clicked() {
        let mods = ui.input(|i| i.modifiers);
        let pick = if mods.shift {
            Pick::Range
        } else if mods.command {
            Pick::Toggle
        } else {
            Pick::Only
        };
        *command = Some(select(pick));
    } else if response.secondary_clicked() && !selected {
        *command = Some(select(Pick::Only));
    }
    if response.dragged()
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        let n = c.mail.selection.len().max(1);
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("drag")));
        let text = format!("{n} {}", T::ItemsWord.get(c.lang));
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
        let r = Rect::from_min_size(pointer + vec2(14.0, 10.0), galley.size() + vec2(12.0, 6.0));
        painter.rect_filled(r, 3.0, c.accent);
        painter.galley(r.min + vec2(6.0, 3.0), galley, Color32::WHITE);
    }
    let lang = c.lang;
    response.context_menu(|ui| {
        let mut pick = |cmd: Option<Command>, ui: &mut Ui| {
            if cmd.is_some() {
                *command = cmd;
            }
            ui.close();
        };
        if ui.button(T::MarkReadUnread.get(lang)).clicked() {
            pick(Some(Command::ToggleRead), ui);
        }
        if ui.button(T::Flag.get(lang)).clicked() {
            pick(Some(Command::ToggleFlag), ui);
        }
        ui.menu_button(T::MoveTo.get(lang), |ui| {
            if let Some(cmd) = crate::shell::folder_menu(ui, c, m.account_id) {
                pick(Some(cmd), ui);
            }
        });
        if c.mail.folder_with_role(m.account_id, "archive").is_some()
            && ui.button(T::Archive.get(lang)).clicked()
        {
            pick(Some(Command::Archive), ui);
        }
        ui.separator();
        if ui.button(T::Delete.get(lang)).clicked() {
            pick(Some(Command::Delete { permanent: false }), ui);
        }
    });
}

fn local(ts: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(Local::now)
}

/// Outlook's date groups, newest first.
fn date_group(ts: i64, now: DateTime<Local>) -> T {
    let d = local(ts).date_naive();
    let today = now.date_naive();
    let days = (today - d).num_days();
    let week_start = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    if days <= 0 {
        T::Today
    } else if days == 1 {
        T::Yesterday
    } else if d >= week_start {
        T::ThisWeek
    } else if d >= week_start - chrono::Days::new(7) {
        T::LastWeek
    } else if d.year() == today.year() && d.month() == today.month() {
        T::ThisMonth
    } else {
        T::Older
    }
}

const WEEKDAYS_FR: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
const WEEKDAYS_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn weekday(d: &DateTime<Local>, lang: Lang) -> &'static str {
    let i = d.weekday().num_days_from_monday() as usize;
    match lang {
        Lang::Fr => WEEKDAYS_FR[i],
        Lang::En => WEEKDAYS_EN[i],
    }
}

/// Time today, weekday and time within the week, date before that.
fn short_date(ts: i64, now: DateTime<Local>, lang: Lang) -> String {
    let d = local(ts);
    let days = (now.date_naive() - d.date_naive()).num_days();
    if days <= 0 {
        d.format("%H:%M").to_string()
    } else if days < 7 {
        format!("{} {}", weekday(&d, lang), d.format("%H:%M"))
    } else {
        d.format("%d/%m/%Y").to_string()
    }
}

fn long_date(ts: i64, lang: Lang) -> String {
    let d = local(ts);
    format!("{} {}", weekday(&d, lang), d.format("%d/%m/%Y %H:%M"))
}

// ---- reading pane ----

pub fn reading_pane(ui: &mut Ui, c: &Ctx<'_>) {
    let several = c.mail.selection.len();
    let Some(m) = c.mail.selected().filter(|_| several <= 1) else {
        let text = if several > 1 {
            format!("{several} {}", T::ItemsSelected.get(c.lang))
        } else {
            T::ReadingEmpty.get(c.lang).to_owned()
        };
        ui.centered_and_justified(|ui| ui.label(RichText::new(text).size(15.0).weak()));
        return;
    };
    let subject = if m.subject.is_empty() {
        T::NoSubject.get(c.lang)
    } else {
        &m.subject
    };
    ui.add(egui::Label::new(RichText::new(subject).font(semibold(18.0))).wrap());
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let sender = if m.from_name.is_empty() {
            m.from_addr.clone()
        } else {
            format!("{} <{}>", m.from_name, m.from_addr)
        };
        ui.add(egui::Label::new(RichText::new(sender).font(semibold(13.5))).selectable(true));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(long_date(m.received, c.lang)).weak());
        });
    });
    if let Some(body) = &c.mail.body {
        for (label, value) in [(T::To, &body.to), (T::Cc, &body.cc)] {
            if !value.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{} : {value}", label.get(c.lang))).weak(),
                    )
                    .wrap(),
                );
            }
        }
    }
    ui.add_space(6.0);
    ui.separator();
    ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| match &c.mail.body {
            Some(body) => {
                ui.add(
                    egui::Label::new(RichText::new(&body.text).size(14.0))
                        .selectable(true)
                        .wrap(),
                );
            }
            None => {
                ui.label(RichText::new(T::Loading.get(c.lang)).weak());
            }
        });
}

// ---- status bar ----

pub fn status_bar(ui: &mut Ui, c: &Ctx<'_>, command: &mut Option<Command>) {
    let small = |t: String| RichText::new(t).size(11.0);
    ui.horizontal(|ui| {
        let lang = c.lang;
        match &c.mail.status {
            Status::Ready if c.mail.accounts.is_empty() => {
                ui.label(small(T::StatusNoAccount.get(lang).into()));
            }
            Status::Ready => {}
            Status::SigningIn => {
                ui.label(small(T::SigningIn.get(lang).into()));
                if ui.small_button(T::Cancel.get(lang)).clicked() {
                    *command = Some(Command::CancelSignIn);
                }
            }
            Status::Syncing { folder, messages } => {
                ui.label(small(format!(
                    "{} : {folder} — {messages} {}",
                    T::Syncing.get(lang),
                    T::MessagesWord.get(lang)
                )));
            }
            Status::SearchingServer => {
                ui.label(small(T::SearchingServer.get(lang).into()));
            }
            Status::ServerFound(n) => {
                ui.label(small(format!("{n} {}", T::ServerFound.get(lang))));
            }
            Status::UpToDate => {
                ui.label(small(T::UpToDate.get(lang).into()));
            }
            Status::Offline { pending } => {
                ui.label(small(format!(
                    "{} : {pending} {}",
                    T::Offline.get(lang),
                    T::PendingChanges.get(lang)
                )));
            }
            Status::Error(e) => {
                ui.label(
                    small(format!("{} : {e}", T::Error.get(lang)))
                        .color(ui.visuals().error_fg_color),
                );
            }
        }
        if let Some(err) = c.save_error {
            ui.separator();
            ui.label(
                small(format!("{} : {err}", T::SaveFailed.get(lang)))
                    .color(ui.visuals().error_fg_color),
            );
        }
        if let Some(encrypted) = c.mail.encrypted {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let t = if encrypted {
                    T::CacheEncrypted
                } else {
                    T::CacheClear
                };
                ui.label(small(t.get(lang).into()).weak());
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{date_group, short_date};
    use crate::i18n::{Lang, T};
    use chrono::{Local, TimeZone};

    #[test]
    fn groups_and_short_dates() {
        // Sunday 8 November 2026, 12:00 local (the week starts on Monday 2).
        let now = Local.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
        let at = |d, h| {
            Local
                .with_ymd_and_hms(2026, 11, d, h, 30, 0)
                .unwrap()
                .timestamp()
        };
        assert_eq!(date_group(at(8, 9), now), T::Today);
        assert_eq!(date_group(at(7, 9), now), T::Yesterday);
        assert_eq!(date_group(at(3, 9), now), T::ThisWeek);
        assert_eq!(date_group(at(1, 9), now), T::LastWeek);
        let oct = Local
            .with_ymd_and_hms(2026, 10, 20, 9, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(date_group(oct, now), T::Older);
        assert_eq!(short_date(at(8, 9), now, Lang::Fr), "09:30");
        assert_eq!(short_date(at(6, 9), now, Lang::Fr), "ven. 09:30");
        assert_eq!(short_date(oct, now, Lang::En), "20/10/2026");
    }
}
