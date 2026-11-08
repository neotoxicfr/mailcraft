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
use crate::shell::{Command, Ctx, Dialog, Pick, ReplyKind, View};
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

/// An account heading being dragged to a new place (its address).
pub struct DraggedAccount(String);

/// A folder being dragged to a new place among its siblings.
pub struct DraggedFolder {
    account_id: i64,
    id: String,
    parent: Option<String>,
}

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
        let fav_open = !c.folded.contains("fav");
        fold_title(
            ui,
            c,
            T::Favorites.get(c.lang),
            "fav",
            fav_open,
            None,
            command,
        );
        if fav_open {
            favourites(ui, c, dialog, command);
        }
        let count = mail.accounts.len();
        for (i, account) in mail.accounts.iter().enumerate() {
            ui.add_space(8.0);
            let email = account.email.to_lowercase();
            let key = format!("a:{email}");
            let open = !c.folded.contains(&key);
            let title = fold_title(
                ui,
                c,
                &account.email,
                &key,
                open,
                Some(&account.email),
                command,
            );
            title.context_menu(|ui| {
                if ui.button(T::Refresh.get(c.lang)).clicked() {
                    *command = Some(Command::SyncAll);
                    ui.close();
                }
                if ui.button(T::AccountSettings.get(c.lang)).clicked() {
                    *command = Some(Command::OpenOptions);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(i > 0, egui::Button::new(T::MoveUp.get(c.lang)))
                    .clicked()
                {
                    *command = Some(Command::MoveAccount {
                        email: account.email.clone(),
                        up: true,
                    });
                    ui.close();
                }
                if ui
                    .add_enabled(i + 1 < count, egui::Button::new(T::MoveDown.get(c.lang)))
                    .clicked()
                {
                    *command = Some(Command::MoveAccount {
                        email: account.email.clone(),
                        up: false,
                    });
                    ui.close();
                }
            });
            if !open {
                continue;
            }
            // Folded folders hide everything below them. A folder has sub-folders when the next
            // row of the tree is deeper.
            let tree = mail.folder_tree(account.id);
            let mut hidden_below: Option<usize> = None;
            for (i, &(f, depth)) in tree.iter().enumerate() {
                if let Some(d) = hidden_below {
                    if depth > d {
                        continue;
                    }
                    hidden_below = None;
                }
                let has_children = tree.get(i + 1).is_some_and(|(_, next)| *next > depth);
                let fold_key = has_children.then(|| format!("f:{email}:{}", f.id));
                let open = fold_key.as_ref().is_none_or(|k| !c.folded.contains(k));
                if !open {
                    hidden_below = Some(depth);
                }
                let fold = fold_key.as_deref().map(|k| (k, open));
                folder_row_with(ui, c, f, depth, fold, true, dialog, command);
            }
        }
    });
}

/// Favourites: all inboxes (several accounts), each account's inbox, sent and deleted items,
/// and the screener.
fn favourites(
    ui: &mut Ui,
    c: &Ctx<'_>,
    dialog: &mut Option<Dialog>,
    command: &mut Option<Command>,
) {
    let mail = c.mail;
    if mail.accounts.len() > 1 {
        let inboxes = mail
            .folders
            .iter()
            .filter(|f| f.role.as_deref() == Some("inbox"));
        let all = mailcraft_store::Folder {
            account_id: mail.accounts[0].id,
            id: crate::mail::UNIFIED.into(),
            parent_id: None,
            name: T::AllInboxes.get(c.lang).into(),
            role: None,
            unread: inboxes.map(|f| f.unread).sum(),
            total: 0,
        };
        folder_row(ui, c, &all, 1, &mut None, command);
    }
    for (email, id) in favourite_list(mail, c.favorites) {
        let folder = mail
            .accounts
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(&email))
            .and_then(|a| {
                mail.folders
                    .iter()
                    .find(|f| f.account_id == a.id && f.id == id)
            });
        if let Some(f) = folder {
            folder_row(ui, c, f, 1, dialog, command);
        }
    }
    if c.screener
        && let Some(inbox) = mail
            .folders
            .iter()
            .find(|f| f.role.as_deref() == Some("inbox"))
    {
        // Virtual folder: the count is of senders waiting, not of messages.
        let screen = mailcraft_store::Folder {
            account_id: inbox.account_id,
            id: crate::mail::SCREENER.into(),
            parent_id: None,
            name: T::ToScreen.get(c.lang).into(),
            role: None,
            unread: mail.screening as i64,
            total: 0,
        };
        folder_row(ui, c, &screen, 1, &mut None, command);
    }
}

/// The favourites as (account address, folder id): the user's list, or by default each
/// account's Inbox, Sent Items and Deleted Items (in that order, account by account).
pub fn favourite_list(
    mail: &crate::mail::Mail,
    chosen: Option<&[(String, String)]>,
) -> Vec<(String, String)> {
    if let Some(list) = chosen {
        return list.to_vec();
    }
    let mut out = Vec::new();
    for role in ["inbox", "sentitems", "deleteditems"] {
        for f in mail
            .folders
            .iter()
            .filter(|f| f.role.as_deref() == Some(role))
        {
            if let Some(a) = mail.accounts.iter().find(|a| a.id == f.account_id) {
                out.push((a.email.to_lowercase(), f.id.clone()));
            }
        }
    }
    out
}

/// A small triangle: pointing down when open, right when folded.
fn paint_fold(ui: &Ui, center: egui::Pos2, open: bool, color: Color32) {
    let points = if open {
        vec![
            center + vec2(-4.0, -2.0),
            center + vec2(4.0, -2.0),
            center + vec2(0.0, 3.0),
        ]
    } else {
        vec![
            center + vec2(-2.0, -4.0),
            center + vec2(3.0, 0.0),
            center + vec2(-2.0, 4.0),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Where a dragged entry would land on `rect`: just after it when the pointer is in its lower
/// half. Draws the insertion line.
fn drop_line(ui: &Ui, c: &Ctx<'_>, rect: Rect) -> bool {
    let after = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|p| p.y > rect.center().y);
    let y = if after { rect.bottom() } else { rect.top() };
    ui.painter()
        .hline(rect.x_range(), y, Stroke::new(2.0, c.accent));
    after
}

/// Section heading that folds what is under it (Favorites, an account). An account heading
/// (`drag`: its address) can be dragged onto another one to change their order.
fn fold_title(
    ui: &mut Ui,
    c: &Ctx<'_>,
    text: &str,
    key: &str,
    open: bool,
    drag: Option<&str>,
    command: &mut Option<Command>,
) -> egui::Response {
    let width = ui.available_width();
    let sense = if drag.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), sense);
    if let Some(email) = drag {
        if response.drag_started() {
            response.dnd_set_drag_payload(DraggedAccount(email.to_owned()));
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if let Some(p) = response.dnd_hover_payload::<DraggedAccount>()
            && p.0 != email
        {
            let after = drop_line(ui, c, rect);
            if let Some(p) = response.dnd_release_payload::<DraggedAccount>() {
                *command = Some(Command::PlaceAccount {
                    email: p.0.clone(),
                    target: email.to_owned(),
                    after,
                });
            }
        }
    }
    response.widget_info(|| WidgetInfo::selected(Role::TreeItem, true, open, text));
    let color = ui.visuals().text_color();
    paint_fold(
        ui,
        rect.left_center() + vec2(6.0, 0.0),
        open,
        ui.visuals().weak_text_color(),
    );
    let galley = one_line(ui, text, semibold(13.0), color, width - 20.0);
    ui.painter().galley(
        pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if response.clicked() {
        *command = Some(Command::ToggleFold {
            key: key.to_owned(),
        });
    }
    response
}

fn folder_row(
    ui: &mut Ui,
    c: &Ctx<'_>,
    f: &Folder,
    depth: usize,
    dialog: &mut Option<Dialog>,
    command: &mut Option<Command>,
) {
    folder_row_with(ui, c, f, depth, None, false, dialog, command);
}

/// A folder row; `fold` (key, open) draws the fold triangle of a folder with sub-folders,
/// `movable` offers Move up / Move down in its menu.
#[allow(clippy::too_many_arguments)]
fn folder_row_with(
    ui: &mut Ui,
    c: &Ctx<'_>,
    f: &Folder,
    depth: usize,
    fold: Option<(&str, bool)>,
    movable: bool,
    dialog: &mut Option<Dialog>,
    command: &mut Option<Command>,
) {
    let width = ui.available_width();
    let sense = if movable {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, FOLDER_ROW), sense);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if movable {
        if response.drag_started() {
            response.dnd_set_drag_payload(DraggedFolder {
                account_id: f.account_id,
                id: f.id.clone(),
                parent: f.parent_id.clone(),
            });
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }
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
    // A folder dragged among its siblings: show where it would go, place it on release.
    if movable
        && let Some(p) = response.dnd_hover_payload::<DraggedFolder>()
        && p.account_id == f.account_id
        && p.parent == f.parent_id
        && p.id != f.id
    {
        let after = drop_line(ui, c, rect);
        if let Some(p) = response.dnd_release_payload::<DraggedFolder>() {
            *command = Some(Command::PlaceFolder {
                account_id: p.account_id,
                id: p.id.clone(),
                target: f.id.clone(),
                after,
            });
        }
    }
    let x = rect.left() + 10.0 + depth as f32 * 12.0;
    let mut toggled = false;
    if let Some((key, open)) = fold {
        let arrow = Rect::from_center_size(pos2(x - 9.0, rect.center().y), vec2(14.0, FOLDER_ROW));
        let r = ui.interact(arrow, ui.scope_id().with(("fold", key)), Sense::click());
        r.widget_info(|| WidgetInfo::selected(Role::Button, true, open, T::FoldFolder.get(c.lang)));
        paint_fold(ui, arrow.center(), open, ui.visuals().weak_text_color());
        if r.clicked() {
            toggled = true;
            *command = Some(Command::ToggleFold {
                key: key.to_owned(),
            });
        }
    }
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
    } else if response.clicked() && !toggled {
        *command = Some(Command::SelectFolder {
            account_id: f.account_id,
            folder_id: f.id.clone(),
        });
    }
    let lang = c.lang;
    let virtual_folder = f.id == crate::mail::UNIFIED || f.id == crate::mail::SCREENER;
    response.context_menu(|ui| {
        if virtual_folder {
            if ui.button(T::Refresh.get(lang)).clicked() {
                *command = Some(Command::SyncAll);
                ui.close();
            }
            return;
        }
        let mut pick = |cmd: Command, ui: &mut Ui| {
            *command = Some(cmd);
            ui.close();
        };
        if ui.button(T::MarkAllRead.get(lang)).clicked() {
            pick(
                Command::MarkFolderRead {
                    account_id: f.account_id,
                    folder_id: f.id.clone(),
                },
                ui,
            );
        }
        let permanent = matches!(f.role.as_deref(), Some("deleteditems" | "junkemail"));
        if ui.button(T::EmptyFolder.get(lang)).clicked() {
            *dialog = Some(Dialog::ConfirmEmpty {
                account_id: f.account_id,
                folder_id: f.id.clone(),
                name: f.name.clone(),
                permanent,
            });
            ui.close();
        }
        if ui.button(T::Refresh.get(lang)).clicked() {
            pick(Command::SyncAll, ui);
        }
        ui.separator();
        let favourite = c
            .mail
            .accounts
            .iter()
            .find(|a| a.id == f.account_id)
            .is_some_and(|a| {
                favourite_list(c.mail, c.favorites)
                    .iter()
                    .any(|(e, id)| e.eq_ignore_ascii_case(&a.email) && *id == f.id)
            });
        let label = if favourite {
            T::RemoveFavorite
        } else {
            T::AddFavorite
        };
        if ui.button(label.get(lang)).clicked() {
            pick(
                Command::ToggleFavorite {
                    account_id: f.account_id,
                    folder_id: f.id.clone(),
                },
                ui,
            );
        }
        let has_children = c
            .mail
            .folders
            .iter()
            .any(|x| x.account_id == f.account_id && x.parent_id.as_deref() == Some(&f.id));
        if has_children && ui.button(T::SortSubfolders.get(lang)).clicked() {
            pick(
                Command::SortSubfolders {
                    account_id: f.account_id,
                    parent: f.id.clone(),
                },
                ui,
            );
        }
        ui.separator();
        if movable {
            for (up, label) in [(true, T::MoveUp), (false, T::MoveDown)] {
                if ui.button(label.get(lang)).clicked() {
                    *command = Some(Command::MoveFolder {
                        account_id: f.account_id,
                        id: f.id.clone(),
                        up,
                    });
                    ui.close();
                }
            }
            ui.separator();
        }
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

/// Fluent persona colours, darkened where needed so white initials stay readable.
const AVATAR_COLOURS: [Color32; 10] = [
    Color32::from_rgb(0xD1, 0x34, 0x38),
    Color32::from_rgb(0xCA, 0x50, 0x10),
    Color32::from_rgb(0x49, 0x82, 0x05),
    Color32::from_rgb(0x03, 0x83, 0x87),
    Color32::from_rgb(0x00, 0x78, 0xD4),
    Color32::from_rgb(0x87, 0x64, 0xB8),
    Color32::from_rgb(0xC2, 0x39, 0xB3),
    Color32::from_rgb(0x69, 0x79, 0x7E),
    Color32::from_rgb(0x98, 0x6F, 0x0B),
    Color32::from_rgb(0x4F, 0x6B, 0xED),
];

/// "Alex Dupont" → "AD"; an address alone → its first letter.
pub fn initials(name: &str, address: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .collect();
    let letters = if letters.is_empty() {
        address
            .chars()
            .find(|c| c.is_alphanumeric())
            .into_iter()
            .collect()
    } else {
        letters
    };
    letters.to_uppercase()
}

/// Decorative circle with the sender's initials; the colour follows the address.
fn avatar(ui: &mut Ui, name: &str, address: &str) {
    let size = 40.0;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    let hash = address
        .to_lowercase()
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    let fill = crate::theme::readable(
        AVATAR_COLOURS[hash as usize % AVATAR_COLOURS.len()],
        &[Color32::WHITE],
        crate::theme::MIN_CONTRAST,
    );
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials(name, address),
        semibold(15.0),
        Color32::WHITE,
    );
}

fn paint_selection(ui: &Ui, c: &Ctx<'_>, rect: Rect, selected: bool, hovered: bool) {
    if selected {
        ui.painter().rect_filled(
            rect,
            2.0,
            c.accent.gamma_multiply(crate::theme::SELECTION_TINT),
        );
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
    let search = ui
        .add(
            TextEdit::singleline(&mut view.search)
                .hint_text(T::SearchHint.get(c.lang))
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(T::SearchFilters.get(c.lang));
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
    for (m, thread) in list_entries(&mail.messages, c.conversations, &view.expanded) {
        if thread.depth == 0 {
            let group = date_group(m.received, now);
            if last != Some(group) {
                rows.push((y, Row::Group(group)));
                y += GROUP_ROW;
                last = Some(group);
            }
        }
        rows.push((y, Row::Message(m, thread)));
        y += MESSAGE_ROW;
    }
    let total = y;
    let mut toggle = None;

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
                    Row::Message(m, thread) => {
                        if message_row(ui, c, rect, m, thread, now, command) {
                            toggle = Some(thread.key.clone());
                        }
                    }
                }
            }
        });
    if let Some(key) = toggle
        && !view.expanded.remove(&key)
    {
        view.expanded.insert(key);
    }
}

enum Row<'a> {
    Group(T),
    Message(&'a MessageSummary, Thread),
}

/// Where a message sits in the list when it is grouped by conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    /// (account, conversation key).
    pub key: (i64, String),
    /// 0: the newest message of its conversation (or any message when not grouped); 1: an
    /// older message shown under it.
    pub depth: u8,
    /// Messages in the conversation (on the head row).
    pub count: usize,
    pub expanded: bool,
    /// Some message of the conversation is unread.
    pub unread: bool,
}

/// Conversation key: the subject without its reply/forward prefixes, case and spaces ignored.
pub fn conversation_key(subject: &str) -> String {
    let mut s = subject.trim().to_lowercase();
    loop {
        let before = s.len();
        for prefix in ["re", "tr", "fw", "fwd", "aw", "wg", "réf", "ref"] {
            for sep in [":", " :"] {
                let p = format!("{prefix}{sep}");
                if let Some(rest) = s.strip_prefix(&p) {
                    s = rest.trim_start().to_owned();
                }
            }
        }
        if s.len() == before {
            break;
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The list rows, newest first. Grouped: one row per conversation (its newest message), and its
/// older messages under it when the conversation is unfolded.
pub fn list_entries<'a>(
    messages: &'a [MessageSummary],
    grouped: bool,
    expanded: &std::collections::HashSet<(i64, String)>,
) -> Vec<(&'a MessageSummary, Thread)> {
    let single = |m: &MessageSummary| Thread {
        key: (m.account_id, m.id.clone()),
        depth: 0,
        count: 1,
        expanded: false,
        unread: !m.is_read,
    };
    if !grouped {
        return messages.iter().map(|m| (m, single(m))).collect();
    }
    let mut order: Vec<(i64, String)> = Vec::new();
    let mut members: std::collections::HashMap<(i64, String), Vec<&MessageSummary>> =
        std::collections::HashMap::new();
    for m in messages {
        let subject = conversation_key(&m.subject);
        // No subject: no conversation to guess, each message stands alone.
        let key = if subject.is_empty() {
            (m.account_id, format!("\u{1}{}", m.id))
        } else {
            (m.account_id, subject)
        };
        let list = members.entry(key.clone()).or_default();
        if list.is_empty() {
            order.push(key);
        }
        list.push(m);
    }
    let mut out = Vec::with_capacity(messages.len());
    for key in order {
        let list = &members[&key];
        let open = expanded.contains(&key);
        let head = Thread {
            key: key.clone(),
            depth: 0,
            count: list.len(),
            expanded: open,
            unread: list.iter().any(|m| !m.is_read),
        };
        out.push((list[0], head));
        if open {
            for m in &list[1..] {
                out.push((
                    *m,
                    Thread {
                        key: key.clone(),
                        depth: 1,
                        count: 0,
                        expanded: true,
                        unread: !m.is_read,
                    },
                ));
            }
        }
    }
    out
}

/// Draws one message row. Returns true when its conversation must be unfolded or folded.
fn message_row(
    ui: &mut Ui,
    c: &Ctx<'_>,
    rect: Rect,
    m: &MessageSummary,
    thread: &Thread,
    now: DateTime<Local>,
    command: &mut Option<Command>,
) -> bool {
    let mut toggle = false;
    let id = ui.scope_id().with(("msg", m.account_id, &m.id));
    // Older messages of an unfolded conversation sit a little to the right.
    let rect = rect.with_min_x(rect.left() + f32::from(thread.depth) * 14.0);
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if thread.count > 1 {
        // Unfold/fold triangle at the left, its own target (keyboard and screen readers too).
        let arrow = Rect::from_min_size(rect.min + vec2(0.0, 2.0), vec2(14.0, 18.0));
        let r = ui.interact(arrow, id.with("thread"), Sense::click());
        r.widget_info(|| {
            WidgetInfo::selected(
                Role::Button,
                true,
                thread.expanded,
                T::ConversationToggle.get(c.lang),
            )
        });
        paint_fold(
            ui,
            arrow.center(),
            thread.expanded,
            ui.visuals().weak_text_color(),
        );
        toggle = r.clicked();
    }
    let sender_name = if m.from_name.is_empty() {
        &m.from_addr
    } else {
        &m.from_name
    };
    let counted;
    let sender = if thread.count > 1 {
        counted = format!("{sender_name} ({})", thread.count);
        &counted
    } else {
        sender_name
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
    let unread = thread.unread;
    if unread {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 4.0), vec2(3.0, rect.height() - 8.0)),
            0.0,
            c.accent,
        );
    }
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let left = rect.left() + if thread.count > 1 { 16.0 } else { 10.0 };
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
        painter.rect_filled(r, 3.0, c.accent_fill);
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
        for (kind, label) in [
            (ReplyKind::Reply, T::Reply),
            (ReplyKind::ReplyAll, T::ReplyAll),
            (ReplyKind::Forward, T::Forward),
        ] {
            if ui.button(label.get(lang).replace('\n', " ")).clicked() {
                pick(Some(Command::Reply(kind)), ui);
            }
        }
        ui.separator();
        let (read_label, read) = if m.is_read {
            (T::MarkUnread, false)
        } else {
            (T::MarkRead, true)
        };
        if ui.button(read_label.get(lang)).clicked() {
            pick(Some(Command::MarkRead(read)), ui);
        }
        ui.menu_button(T::FollowUp.get(lang), |ui| {
            if ui.button(T::FlagMessage.get(lang)).clicked() {
                pick(Some(Command::MarkFlag(true)), ui);
            }
            if ui.button(T::ClearFlag.get(lang)).clicked() {
                pick(Some(Command::MarkFlag(false)), ui);
            }
        });
        ui.separator();
        ui.menu_button(T::FindRelated.get(lang), |ui| {
            if ui.button(T::FromThisSender.get(lang)).clicked() {
                pick(Some(Command::FindRelated { sender: true }), ui);
            }
            if ui.button(T::InThisConversation.get(lang)).clicked() {
                pick(Some(Command::FindRelated { sender: false }), ui);
            }
        });
        ui.menu_button(T::Rules.get(lang), |ui| {
            let text = format!("{} {}…", T::AlwaysMoveFrom.get(lang), m.from_addr);
            if ui.button(text).clicked() {
                pick(Some(Command::RuleFromSender), ui);
            }
        });
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
        let in_junk = c.mail.folders.iter().any(|f| {
            f.account_id == m.account_id
                && f.id == m.folder_id
                && f.role.as_deref() == Some("junkemail")
        });
        ui.menu_button(T::JunkMenu.get(lang), |ui| {
            if ui.button(T::BlockSender.get(lang)).clicked() {
                pick(Some(Command::BlockSender), ui);
            }
            if ui.button(T::NeverBlockSender.get(lang)).clicked() {
                pick(Some(Command::NeverBlock { domain: false }), ui);
            }
            if ui.button(T::NeverBlockDomain.get(lang)).clicked() {
                pick(Some(Command::NeverBlock { domain: true }), ui);
            }
            if in_junk && ui.button(T::NotJunk.get(lang)).clicked() {
                pick(Some(Command::NotJunk), ui);
            }
        });
        ui.separator();
        if ui.button(T::Print.get(lang)).clicked() {
            pick(Some(Command::Print), ui);
        }
        if ui.button(T::Export.get(lang)).clicked() {
            pick(Some(Command::Export), ui);
        }
        ui.separator();
        if ui.button(T::Delete.get(lang)).clicked() {
            pick(Some(Command::Delete { permanent: false }), ui);
        }
    });
    toggle
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
    if ts <= 0 {
        return String::new();
    }
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

pub fn reading_pane(ui: &mut Ui, c: &Ctx<'_>, view: &mut View, command: &mut Option<Command>) {
    // A reply being written takes the reading pane, as in Outlook.
    if let Some(comp) = view.composers.iter_mut().find(|k| !k.detached) {
        let id = comp.id;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if let Some(action) = crate::composer::show(ui, c, comp) {
                *command = Some(Command::Composer { id, action });
            }
        });
        return;
    }
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
    // Outlook layout: the sender's initials in a circle, name, recipients and date beside it.
    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
        avatar(ui, &m.from_name, &m.from_addr);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let sender = if m.from_name.is_empty() {
                    m.from_addr.clone()
                } else {
                    format!("{} <{}>", m.from_name, m.from_addr)
                };
                ui.add(
                    egui::Label::new(RichText::new(sender).font(semibold(13.5))).selectable(true),
                );
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
        });
    });
    if c.mail
        .folder
        .as_ref()
        .is_some_and(|(_, f)| f == crate::mail::SCREENER)
    {
        egui::Frame::group(ui.style())
            .fill(c.accent.gamma_multiply(0.12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(T::NewSender.get(c.lang));
                    if ui.button(T::Accept.get(c.lang)).clicked() {
                        *command = Some(Command::Screen {
                            address: m.from_addr.clone(),
                            accept: true,
                        });
                    }
                    if ui
                        .button(T::Refuse.get(c.lang))
                        .on_hover_text(T::RefuseHelp.get(c.lang))
                        .clicked()
                    {
                        *command = Some(Command::Screen {
                            address: m.from_addr.clone(),
                            accept: false,
                        });
                    }
                });
            });
    }
    attachments(ui, c, command);
    let has_html = !c.mail.original_html.is_empty();
    if has_html {
        ui.horizontal(|ui| {
            let label = if view.original {
                T::SimplifiedView
            } else {
                T::OriginalView
            };
            if ui.small_button(label.get(c.lang)).clicked() {
                view.original = !view.original;
            }
        });
    }
    ui.add_space(4.0);
    ui.separator();
    if c.mail.body.is_none() {
        ui.label(RichText::new(T::Loading.get(c.lang)).weak());
        return;
    }
    if view.original && has_html {
        // The WebView is a native window drawn over this rectangle by the app.
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        view.webview = Some(rect);
        return;
    }
    if c.mail.blocks.is_empty() && has_html {
        ui.label(
            RichText::new(T::NothingSimplified.get(c.lang))
                .weak()
                .italics(),
        );
        if ui.button(T::OriginalView.get(c.lang)).clicked() {
            view.original = true;
        }
        return;
    }
    let inline_uri = |cid: &str| {
        c.mail
            .inline
            .contains_key(cid)
            .then(|| format!("bytes://cid/{cid}"))
    };
    let images = crate::reader::Images {
        block_remote: c.block_images,
        ready: &c.mail.images_ready,
        inline_uri: &inline_uri,
    };
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let wanted = crate::reader::show(ui, c, &c.mail.blocks, &images);
        for url in wanted {
            if !c.mail.images_requested.contains(&url) && !view.image_requests.contains(&url) {
                view.image_requests.push(url);
            }
        }
    });
}

fn attachments(ui: &mut Ui, c: &Ctx<'_>, command: &mut Option<Command>) {
    let files: Vec<(usize, &mailcraft_store::Attachment)> = c
        .mail
        .attachments
        .iter()
        .enumerate()
        .filter(|(_, a)| !a.is_inline)
        .collect();
    if files.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (index, a) in files {
            let text = format!("{}  ({})", a.name, human_size(a.size));
            let button = ui.add(egui::Button::new(text).small());
            egui::Popup::menu(&button).show(|ui| {
                let risky = crate::attachments::is_risky(&a.name);
                let open = ui.add_enabled(!risky, egui::Button::new(T::Open.get(c.lang)));
                let open = if risky {
                    open.on_disabled_hover_text(T::RiskyFile.get(c.lang))
                } else {
                    open
                };
                if open.clicked() {
                    *command = Some(Command::OpenAttachment { index });
                }
                if ui.button(T::SaveAs.get(c.lang)).clicked() {
                    *command = Some(Command::SaveAttachment { index });
                }
            });
        }
    });
}

fn human_size(bytes: i64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} Mo", b as f64 / f64::from(1 << 20)),
        b if b >= 1 << 10 => format!("{} Ko", b >> 10),
        b => format!("{b} o"),
    }
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
            Status::Connecting => {
                ui.label(small(T::Connecting.get(lang).into()));
            }
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
            Status::CleanupDone {
                done,
                skipped,
                dry_run,
            } => {
                let what = if *dry_run {
                    T::CleanupSimulated
                } else {
                    T::CleanupDone
                };
                ui.label(small(format!(
                    "{} : {done}, {skipped} {}",
                    what.get(lang),
                    T::SkippedWord.get(lang)
                )));
            }
            Status::RebuildScheduled => {
                ui.label(small(T::RebuildScheduled.get(lang).into()));
            }
            Status::ClassifyDone(n) => {
                ui.label(small(format!("{n} {}", T::ClassifyDone.get(lang))));
            }
            Status::Sent => {
                ui.label(small(T::Sent.get(lang).into()));
            }
            Status::AttachmentSaved => {
                ui.label(small(T::AttachmentSaved.get(lang).into()));
            }
            Status::SearchingServer => {
                ui.label(small(T::SearchingServer.get(lang).into()));
            }
            Status::ServerFound(n) => {
                ui.label(small(format!("{n} {}", T::ServerFound.get(lang))));
            }
            Status::Exported(n) => {
                ui.label(small(format!("{n} {}", T::ExportedWord.get(lang))));
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

#[cfg(test)]
mod conversation_tests {
    use super::{conversation_key, list_entries};
    use mailcraft_store::MessageSummary;

    fn msg(id: &str, subject: &str, received: i64, read: bool) -> MessageSummary {
        MessageSummary {
            account_id: 1,
            id: id.into(),
            folder_id: "in".into(),
            from_name: "A".into(),
            from_addr: "a@example.com".into(),
            subject: subject.into(),
            preview: String::new(),
            received,
            is_read: read,
            has_attachments: false,
            flagged: false,
        }
    }

    #[test]
    fn conversation_keys_ignore_reply_prefixes() {
        assert_eq!(
            conversation_key("RE : TR: Re:  Réunion   jeudi"),
            "réunion jeudi"
        );
        assert_eq!(conversation_key("Fwd: Photos"), conversation_key("photos"));
        assert_eq!(conversation_key("Return policy"), "return policy");
    }

    #[test]
    fn grouped_list_folds_conversations() {
        let list = [
            msg("3", "RE: Réunion", 30, true),
            msg("2", "Promo", 20, true),
            msg("1", "Réunion", 10, false),
            msg("0", "", 5, true),
            msg("9", "", 4, true),
        ];
        let mut open = std::collections::HashSet::new();
        let rows = list_entries(&list, true, &open);
        let ids: Vec<&str> = rows.iter().map(|(m, _)| m.id.as_str()).collect();
        assert_eq!(ids, ["3", "2", "0", "9"]); // empty subjects are not merged
        assert_eq!((rows[0].1.count, rows[0].1.unread), (2, true));
        open.insert(rows[0].1.key.clone());
        let rows = list_entries(&list, true, &open);
        let ids: Vec<&str> = rows.iter().map(|(m, _)| m.id.as_str()).collect();
        assert_eq!(ids, ["3", "1", "2", "0", "9"]);
        assert_eq!(rows[1].1.depth, 1);
        assert_eq!(list_entries(&list, false, &open).len(), 5);
    }
}
