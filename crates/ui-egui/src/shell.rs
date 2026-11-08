//! Main window, Outlook classic layout: tab row, ribbon, folder pane, message list, reading pane.

use egui::{
    Align, Align2, CentralPanel, Color32, ComboBox, CursorIcon, FontId, Frame, Layout, Margin,
    Panel, Rect, Role, Sense, Stroke, Ui, Vec2, WidgetInfo, pos2, vec2,
};

use crate::i18n::{Lang, T};
use crate::icons::Icon;
use crate::mail::Mail;
use crate::panes;
use crate::settings::{AccentPref, LangPref, Prefs, ThemePref};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tab {
    File,
    #[default]
    Home,
    SendReceive,
    Folder,
    View,
}

const TABS: [(Tab, T); 5] = [
    (Tab::File, T::TabFile),
    (Tab::Home, T::TabHome),
    (Tab::SendReceive, T::TabSendReceive),
    (Tab::Folder, T::TabFolder),
    (Tab::View, T::TabView),
];

#[derive(Default)]
pub struct View {
    pub tab: Tab,
    pub search: String,
}

/// What the window asks the app to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    Exit,
    AddAccount,
    CancelSignIn,
    SyncAll,
    SelectFolder { account_id: i64, folder_id: String },
    SelectMessage { account_id: i64, id: String },
    SearchChanged,
}

/// Everything a frame needs besides the mutable state.
pub struct Ctx<'a> {
    pub lang: Lang,
    pub accent: Color32,
    pub mail: &'a Mail,
    pub save_error: Option<&'a str>,
}

pub fn show(ui: &mut Ui, c: &Ctx<'_>, view: &mut View, prefs: &mut Prefs) -> Option<Command> {
    let dark = ui.visuals().dark_mode;
    let mut command = None;
    let mut pane_command = None;

    Panel::top("tabs")
        .frame(
            Frame::NONE
                .fill(c.accent)
                .inner_margin(Margin::symmetric(6, 0)),
        )
        .show(ui, |ui| tab_row(ui, c, view, dark));
    Panel::top("ribbon")
        .frame(
            Frame::NONE
                .fill(theme::ribbon_fill(dark))
                .inner_margin(Margin::symmetric(6, 4)),
        )
        .exact_size(100.0)
        .show(ui, |ui| {
            // Full-height row (unlike `horizontal`), so group captions sit at the ribbon's bottom.
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                command = ribbon(ui, c, view.tab, prefs);
            });
        });
    separator_line(ui, dark);
    Panel::bottom("status")
        .frame(
            Frame::NONE
                .fill(theme::ribbon_fill(dark))
                .inner_margin(Margin::symmetric(8, 3)),
        )
        .show(ui, |ui| panes::status_bar(ui, c, &mut pane_command));
    Panel::left("folders")
        .resizable(true)
        .default_size(240.0)
        .size_range(160.0..=420.0)
        .frame(pane_frame(ui))
        .show(ui, |ui| panes::folder_pane(ui, c, &mut pane_command));
    Panel::left("list")
        .resizable(true)
        .default_size(400.0)
        .size_range(260.0..=700.0)
        .frame(pane_frame(ui))
        .show(ui, |ui| {
            panes::message_list(ui, c, view, dark, &mut pane_command)
        });
    CentralPanel::default()
        .frame(pane_frame(ui))
        .show(ui, |ui| panes::reading_pane(ui, c));
    command.or(pane_command)
}

fn pane_frame(ui: &Ui) -> Frame {
    Frame::NONE
        .fill(ui.visuals().panel_fill)
        .inner_margin(Margin::same(8))
}

fn separator_line(ui: &mut Ui, dark: bool) {
    Panel::top("ribbon_sep")
        .frame(Frame::NONE.fill(theme::separator(dark)))
        .exact_size(1.0)
        .show(ui, |_| {});
}

fn tab_row(ui: &mut Ui, c: &Ctx<'_>, view: &mut View, dark: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (tab, label) in TABS {
            let text = label.get(c.lang);
            let font = FontId::proportional(13.0);
            let galley = ui
                .painter()
                .layout_no_wrap(text.to_owned(), font, Color32::WHITE);
            let size = vec2(galley.size().x + 24.0, 28.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let selected = view.tab == tab;
            response.widget_info(|| WidgetInfo::selected(Role::Tab, true, selected, text));
            let (fill, text_color) = if selected {
                (theme::ribbon_fill(dark), c.accent)
            } else if response.hovered() {
                (c.accent.gamma_multiply(0.8), Color32::WHITE)
            } else {
                (Color32::TRANSPARENT, Color32::WHITE)
            };
            ui.painter().rect_filled(rect, 0.0, fill);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(13.0),
                text_color,
            );
            if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                view.tab = tab;
            }
        }
    });
}

fn ribbon(ui: &mut Ui, c: &Ctx<'_>, tab: Tab, prefs: &mut Prefs) -> Option<Command> {
    let lang = c.lang;
    let later = T::AvailableLater.get(lang);
    let mut command = None;
    match tab {
        Tab::File => {
            group(ui, c, T::TabFile, |ui| {
                if big_button(
                    ui,
                    c,
                    Icon::AddAccount,
                    T::AddAccount,
                    !c.mail.signing_in,
                    "",
                )
                .clicked()
                {
                    command = Some(Command::AddAccount);
                }
                if big_button(ui, c, Icon::Exit, T::Exit, true, "").clicked() {
                    command = Some(Command::Exit);
                }
            });
        }
        Tab::Home => {
            group(ui, c, T::GroupNew, |ui| {
                big_button(ui, c, Icon::Envelope, T::NewEmail, false, later);
            });
            group(ui, c, T::GroupDelete, |ui| {
                big_button(ui, c, Icon::Trash, T::Delete, false, later);
                big_button(ui, c, Icon::Archive, T::Archive, false, later);
            });
            group(ui, c, T::GroupRespond, |ui| {
                ui.vertical(|ui| {
                    small_button(ui, c, Icon::Reply, T::Reply, false, later);
                    small_button(ui, c, Icon::ReplyAll, T::ReplyAll, false, later);
                    small_button(ui, c, Icon::Forward, T::Forward, false, later);
                });
            });
            group(ui, c, T::GroupMove, |ui| {
                big_button(ui, c, Icon::Folder, T::Move, false, later);
            });
            group(ui, c, T::GroupTags, |ui| {
                ui.vertical(|ui| {
                    small_button(ui, c, Icon::Envelope, T::UnreadRead, false, later);
                    small_button(ui, c, Icon::Flag, T::Flag, false, later);
                });
            });
            group(ui, c, T::GroupSort, |ui| {
                big_button(ui, c, Icon::Unsubscribe, T::Unsubscribe, false, later);
                big_button(ui, c, Icon::Analyze, T::Analyze, false, later);
            });
        }
        Tab::SendReceive => {
            group(ui, c, T::TabSendReceive, |ui| {
                let can = !c.mail.accounts.is_empty();
                if big_button(ui, c, Icon::Sync, T::SendReceiveAll, can, later).clicked() {
                    command = Some(Command::SyncAll);
                }
            });
        }
        Tab::Folder => {
            group(ui, c, T::TabFolder, |ui| {
                big_button(ui, c, Icon::FolderPlus, T::NewFolder, false, later);
                ui.vertical(|ui| {
                    small_button(ui, c, Icon::Rename, T::RenameFolder, false, later);
                    small_button(ui, c, Icon::Trash, T::DeleteFolder, false, later);
                });
            });
        }
        Tab::View => {
            group(ui, c, T::GroupAppearance, |ui| {
                appearance_controls(ui, lang, prefs)
            });
        }
    }
    command
}

fn appearance_controls(ui: &mut Ui, lang: Lang, prefs: &mut Prefs) {
    egui::Grid::new("appearance")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            let theme = &mut prefs.appearance.theme;
            let label = ui.label(T::Theme.get(lang)).id;
            ComboBox::from_id_salt("theme")
                .width(170.0)
                .selected_text(theme_label(*theme).get(lang))
                .show_ui(ui, |ui| {
                    for v in [ThemePref::System, ThemePref::Light, ThemePref::Dark] {
                        ui.selectable_value(theme, v, theme_label(v).get(lang));
                    }
                })
                .response
                .labelled_by(label);
            ui.end_row();

            let accent = &mut prefs.appearance.accent;
            let label = ui.label(T::Accent.get(lang)).id;
            ComboBox::from_id_salt("accent")
                .width(170.0)
                .selected_text(accent_label(*accent).get(lang))
                .show_ui(ui, |ui| {
                    for v in [
                        AccentPref::Outlook,
                        AccentPref::Windows,
                        AccentPref::MailCraft,
                    ] {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                            ui.painter().rect_filled(r, 2.0, theme::accent(v));
                            ui.selectable_value(accent, v, accent_label(v).get(lang));
                        });
                    }
                })
                .response
                .labelled_by(label);
            ui.end_row();

            let language = &mut prefs.general.language;
            let label = ui.label(T::Language.get(lang)).id;
            ComboBox::from_id_salt("language")
                .width(170.0)
                .selected_text(lang_label(*language, lang))
                .show_ui(ui, |ui| {
                    for v in [LangPref::System, LangPref::Fr, LangPref::En] {
                        ui.selectable_value(language, v, lang_label(v, lang));
                    }
                })
                .response
                .labelled_by(label);
            ui.end_row();
        });
}

fn theme_label(v: ThemePref) -> T {
    match v {
        ThemePref::System => T::ThemeSystem,
        ThemePref::Light => T::ThemeLight,
        ThemePref::Dark => T::ThemeDark,
    }
}

fn accent_label(v: AccentPref) -> T {
    match v {
        AccentPref::Outlook => T::AccentOutlook,
        AccentPref::Windows => T::AccentWindows,
        AccentPref::MailCraft => T::AccentMailCraft,
    }
}

/// Language names are written in their own language, as everywhere.
fn lang_label(v: LangPref, lang: Lang) -> &'static str {
    match v {
        LangPref::System => T::LangSystem.get(lang),
        LangPref::Fr => "Français",
        LangPref::En => "English",
    }
}

/// A ribbon group: its buttons, its caption at the bottom, a separator on the right.
fn group(ui: &mut Ui, c: &Ctx<'_>, caption: T, add: impl FnOnce(&mut Ui)) {
    let dark = ui.visuals().dark_mode;
    let color = ui.visuals().weak_text_color();
    let top = ui.max_rect().top();
    let height = ui.max_rect().height();
    let start = ui.cursor().left();
    let galley = ui.painter().layout_no_wrap(
        caption.get(c.lang).to_owned(),
        FontId::proportional(11.0),
        color,
    );
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height - 16.0),
        Layout::left_to_right(Align::Min),
        add,
    );
    let content = ui.cursor().left() - start;
    let needed = galley.size().x + 8.0;
    if content < needed {
        ui.add_space(needed - content);
    }
    let end = ui.cursor().left();
    let pos = pos2(
        (start + end - galley.size().x) / 2.0,
        top + height - galley.size().y,
    );
    ui.painter().galley(pos, galley, color);
    ui.painter().vline(
        end + 4.0,
        top + 2.0..=top + height - 2.0,
        Stroke::new(1.0, theme::separator(dark)),
    );
    ui.add_space(9.0);
}

fn big_button(
    ui: &mut Ui,
    c: &Ctx<'_>,
    icon: Icon,
    label: T,
    enabled: bool,
    disabled_tip: &str,
) -> egui::Response {
    let text = label.get(c.lang);
    let font = FontId::proportional(12.0);
    let lines: Vec<_> = text
        .lines()
        .map(|l| {
            ui.painter()
                .layout_no_wrap(l.to_owned(), font.clone(), Color32::WHITE)
        })
        .collect();
    let text_width = lines.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let size = vec2((text_width + 12.0).max(56.0), 70.0);
    let (rect, response) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let color = paint_button_bg(ui, c, rect, &response, enabled);
    let icon_rect = Rect::from_center_size(rect.center_top() + vec2(0.0, 20.0), Vec2::splat(30.0));
    icon.paint(
        ui.painter(),
        icon_rect,
        if enabled { c.accent } else { color },
    );
    let mut y = rect.top() + 38.0;
    for line in text.lines() {
        ui.painter().text(
            pos2(rect.center().x, y),
            Align2::CENTER_TOP,
            line,
            font.clone(),
            color,
        );
        y += 14.0;
    }
    with_tip(response, text, enabled, disabled_tip)
}

fn small_button(
    ui: &mut Ui,
    c: &Ctx<'_>,
    icon: Icon,
    label: T,
    enabled: bool,
    disabled_tip: &str,
) -> egui::Response {
    let text = label.get(c.lang);
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(12.0), Color32::WHITE);
    let size = vec2(galley.size().x + 32.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let color = paint_button_bg(ui, c, rect, &response, enabled);
    let icon_rect = Rect::from_min_size(rect.min + vec2(4.0, 3.0), Vec2::splat(16.0));
    icon.paint(
        ui.painter(),
        icon_rect,
        if enabled { c.accent } else { color },
    );
    ui.painter().text(
        rect.left_center() + vec2(26.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.0),
        color,
    );
    with_tip(response, text, enabled, disabled_tip)
}

/// Hover/press background; returns the text colour to use.
fn paint_button_bg(
    ui: &Ui,
    c: &Ctx<'_>,
    rect: Rect,
    response: &egui::Response,
    enabled: bool,
) -> Color32 {
    let visuals = ui.visuals();
    if !enabled {
        return visuals.weak_text_color();
    }
    if response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, 2.0, c.accent.gamma_multiply(0.3));
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, c.accent.gamma_multiply(0.15));
        ui.painter().rect_stroke(
            rect,
            2.0,
            Stroke::new(1.0, c.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    visuals.text_color()
}

fn with_tip(response: egui::Response, label: &str, enabled: bool, tip: &str) -> egui::Response {
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label.replace('\n', " ")));
    if enabled || tip.is_empty() {
        response
    } else {
        response.on_hover_text(tip)
    }
}
