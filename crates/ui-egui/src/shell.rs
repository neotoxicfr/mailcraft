//! Main window, Outlook classic layout: tab row, ribbon, folder pane, message list, reading pane.

use egui::{
    Align, Align2, CentralPanel, Color32, ComboBox, CursorIcon, FontId, Frame, Layout, Margin,
    Panel, Rect, RichText, Role, Sense, Stroke, Ui, Vec2, WidgetInfo, pos2, vec2,
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
    pub dialog: Option<Dialog>,
    /// Show the shown message in its original look (WebView) rather than simplified.
    pub original: bool,
    /// Where the original view goes this frame (set by the reading pane).
    pub webview: Option<egui::Rect>,
    /// Remote images the reader wants, to send to the worker.
    pub image_requests: Vec<String>,
    /// Messages being written: in the reading pane, or in their own window when detached.
    pub composers: Vec<crate::composer::Composer>,
    /// The clean-up view replaces the message list and reading pane.
    pub cleanup_open: bool,
    /// File › Options replaces the message list and reading pane.
    pub options_open: bool,
    /// Claude key being typed in Options, and whether one is saved (`None`: not checked yet).
    pub claude_key: String,
    pub claude_key_saved: Option<bool>,
    pub cleanup: crate::cleanup::State,
    /// Conversations unfolded in the list (account id + conversation key).
    pub expanded: std::collections::HashSet<(i64, String)>,
}

/// Address completion: (account, typed text) → (name, address) pairs.
pub type Suggest<'a> = dyn Fn(i64, &str) -> Vec<(String, String)> + 'a;

/// A question the window asks before acting.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Dialog {
    /// Permanent deletion of the selected messages.
    ConfirmPermanentDelete {
        count: usize,
    },
    /// New folder under `parent` (top level when `None`), or rename of `rename`.
    FolderName {
        account_id: i64,
        parent: Option<String>,
        rename: Option<String>,
        name: String,
    },
    ConfirmDeleteFolder {
        account_id: i64,
        id: String,
        name: String,
    },
    /// Signature of one account, edited as plain text.
    Signature {
        email: String,
        text: String,
    },
    /// Allow-list of the clean-up view, one address or domain per line.
    AllowList {
        text: String,
    },
    ConfirmRemoveAccount {
        account_id: i64,
        email: String,
    },
    /// First-launch assistant, at step 0..WELCOME_STEPS.
    Welcome {
        step: u8,
    },
    Rules {
        draft: Box<crate::automation_ui::RuleDraft>,
    },
    Away {
        away: Box<mailcraft_sync::Away>,
        until: String,
    },
    /// Microsoft sign-in, or an IMAP + SMTP account.
    AddAccount {
        form: Box<crate::account_form::Form>,
    },
    /// Claude's newsletter digest.
    Digest {
        text: String,
    },
    /// A real (not simulated) clean-up action, confirmed first.
    ConfirmCleanup {
        action: mailcraft_sync::CleanAction,
        addresses: Vec<String>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

/// How a click changes the message selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick {
    /// Plain click: only this one.
    Only,
    /// Ctrl+click: add or remove.
    Toggle,
    /// Shift+click: everything from the anchor.
    Range,
}

/// What the window asks the app to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    Exit,
    /// Out-of-office reply saved: keep it and set it on the servers.
    SaveAway {
        away: Box<mailcraft_sync::Away>,
    },
    /// The shown message as a page, in the browser's print dialog.
    Print,
    /// The selected messages as .eml files, in a folder the user picks.
    Export,
    /// Fold or unfold an entry of the folder pane (see `FolderLayout::folded`).
    ToggleFold {
        key: String,
    },
    /// One place up or down in the folder pane.
    MoveAccount {
        email: String,
        up: bool,
    },
    MoveFolder {
        account_id: i64,
        id: String,
        up: bool,
    },
    /// Dropped by drag and drop just before (or after) another entry.
    PlaceAccount {
        email: String,
        target: String,
        after: bool,
    },
    PlaceFolder {
        account_id: i64,
        id: String,
        target: String,
        after: bool,
    },
    /// Opens the "add an account" dialog.
    AddAccount,
    AddMicrosoftAccount,
    AddGoogleAccount,
    AddImapAccount {
        server: Box<mailcraft_imap::Server>,
    },
    CancelSignIn,
    SyncAll,
    SelectFolder {
        account_id: i64,
        folder_id: String,
    },
    SelectMessage {
        account_id: i64,
        id: String,
        pick: Pick,
    },
    SelectAll,
    /// Moves the selection up (-1) or down (+1).
    Step(i32),
    SearchChanged,
    /// Search the whole mailbox on the server for the current query.
    SearchServer,
    OpenAttachment {
        index: usize,
    },
    SaveAttachment {
        index: usize,
    },
    NewMessage,
    Reply(ReplyKind),
    /// A composer (by id) asks something.
    Composer {
        id: u64,
        action: crate::composer::Action,
    },
    SaveSignature {
        email: String,
        text: String,
    },
    /// Opens the clean-up view and analyses senders not analysed yet.
    OpenCleanup,
    CloseCleanup,
    Analyze {
        force: bool,
    },
    Cleanup {
        action: mailcraft_sync::CleanAction,
        addresses: Vec<String>,
    },
    AllowSenders {
        addresses: Vec<String>,
    },
    SaveAllowList {
        text: String,
    },
    /// Screener decision on a new sender.
    SaveClaudeKey {
        key: String,
    },
    DeleteClaudeKey,
    /// Another AI provider was picked: its key state must be read again.
    AiProviderChanged,
    /// Claude classifies senders (all of them again with `force`).
    Classify {
        force: bool,
    },
    /// Claude summarises the last 24 hours of newsletters.
    Digest,
    Screen {
        address: String,
        accept: bool,
    },
    OpenOptions,
    CloseOptions,
    /// Rebuild the local copy at next start (encryption switched on or off).
    RebuildLocalCopy,
    RemoveAccount {
        account_id: i64,
    },

    /// Selected messages to Deleted Items, or for good when `permanent` (already confirmed).
    Delete {
        permanent: bool,
    },
    Archive,
    MoveTo {
        folder_id: String,
    },
    ToggleRead,
    ToggleFlag,
    Undo,
    CreateFolder {
        account_id: i64,
        parent: Option<String>,
        name: String,
    },
    RenameFolder {
        account_id: i64,
        id: String,
        name: String,
    },
    DeleteFolder {
        account_id: i64,
        id: String,
    },
}

/// Everything a frame needs besides the mutable state.
pub struct Ctx<'a> {
    pub lang: Lang,
    /// Accent for text and icons, readable on the panes of the current theme.
    pub accent: Color32,
    /// Accent for solid fills under white text.
    pub accent_fill: Color32,
    pub mail: &'a Mail,
    pub save_error: Option<&'a str>,
    /// Remote images stay blocked (tracker blocker on).
    pub block_images: bool,
    /// The screener is on (« À valider » shown).
    pub screener: bool,
    /// The list groups messages by conversation.
    pub conversations: bool,
    /// Folded entries of the folder pane.
    pub folded: &'a std::collections::BTreeSet<String>,
    /// Words the user added to the spell checker.
    pub own_words: &'a [String],
    /// Address completion: (account, typed text) → (name, address).
    pub suggest: Option<&'a Suggest<'a>>,
}

pub fn show(ui: &mut Ui, c: &Ctx<'_>, view: &mut View, prefs: &mut Prefs) -> Option<Command> {
    let dark = ui.visuals().dark_mode;
    let mut command = None;
    let mut pane_command = None;
    view.webview = None;

    Panel::top("tabs")
        .frame(
            Frame::NONE
                .fill(c.accent_fill)
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
                command = ribbon(ui, c, view, prefs);
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
        .show(ui, |ui| {
            panes::folder_pane(ui, c, &mut view.dialog, &mut pane_command)
        });
    if view.options_open {
        CentralPanel::default()
            .frame(pane_frame(ui))
            .show(ui, |ui| {
                if let Some(cmd) = crate::options::show(ui, c, prefs, view) {
                    pane_command = Some(cmd);
                }
            });
    } else if view.cleanup_open {
        let allow = prefs.cleanup.allow_list.clone();
        CentralPanel::default()
            .frame(pane_frame(ui))
            .show(ui, |ui| {
                if let Some(cmd) = crate::cleanup::show(
                    ui,
                    c,
                    &mut view.cleanup,
                    &mut prefs.cleanup.dry_run,
                    &allow,
                    &mut view.dialog,
                ) {
                    pane_command = Some(cmd);
                }
            });
    } else {
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
            .show(ui, |ui| panes::reading_pane(ui, c, view, &mut pane_command));
    }
    let dialog_command = dialog(ui.ctx(), c, &mut view.dialog, prefs);
    let key_command = if view.dialog.is_none() {
        keys(ui.ctx(), c, &mut view.dialog)
    } else {
        None
    };
    command.or(pane_command).or(dialog_command).or(key_command)
}

/// Whether Delete on the selection should be permanent (asked first): Shift held, or the
/// messages already sit in Deleted Items, as in Outlook.
pub fn delete_is_permanent(c: &Ctx<'_>, shift: bool) -> bool {
    shift || c.mail.current_folder().and_then(|f| f.role.as_deref()) == Some("deleteditems")
}

/// Delete pressed or clicked: a command, or a confirmation to show first.
pub fn delete_request(c: &Ctx<'_>, shift: bool, dialog: &mut Option<Dialog>) -> Option<Command> {
    let count = c.mail.selection.len();
    if count == 0 {
        return None;
    }
    if delete_is_permanent(c, shift) {
        *dialog = Some(Dialog::ConfirmPermanentDelete { count });
        None
    } else {
        Some(Command::Delete { permanent: false })
    }
}

/// Outlook's shortcuts, when no text field has the keyboard.
fn keys(ctx: &egui::Context, c: &Ctx<'_>, dialog: &mut Option<Dialog>) -> Option<Command> {
    use egui::{Key, Modifiers};
    if ctx.egui_wants_keyboard_input() {
        return None;
    }
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::SHIFT, Key::Delete) {
            delete_request(c, true, dialog)
        } else if i.consume_key(Modifiers::NONE, Key::Delete) {
            delete_request(c, false, dialog)
        } else if i.consume_key(Modifiers::COMMAND, Key::Z) {
            Some(Command::Undo)
        } else if i.consume_key(Modifiers::COMMAND, Key::A) {
            Some(Command::SelectAll)
        } else if i.consume_key(Modifiers::COMMAND, Key::Q)
            || i.consume_key(Modifiers::COMMAND, Key::U)
        {
            Some(Command::ToggleRead)
        } else if i.consume_key(Modifiers::NONE, Key::Insert) {
            Some(Command::ToggleFlag)
        } else if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            Some(Command::Step(1))
        } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            Some(Command::Step(-1))
        } else {
            None
        }
    })
}

fn dialog(
    ctx: &egui::Context,
    c: &Ctx<'_>,
    state: &mut Option<Dialog>,
    prefs: &mut Prefs,
) -> Option<Command> {
    let lang = c.lang;
    let current = state.clone()?;
    let mut result = None;
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("mailcraft_dialog")).show(ctx, |ui| {
        ui.set_width(380.0);
        match &current {
            Dialog::ConfirmPermanentDelete { count } => {
                ui.heading(T::ConfirmPermanentTitle.get(lang));
                ui.label(format!("{count} — {}", T::ConfirmPermanentBody.get(lang)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(T::DeleteForGood.get(lang)).clicked() {
                        result = Some(Command::Delete { permanent: true });
                        close = true;
                    }
                    if ui.button(T::Cancel.get(lang)).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::FolderName {
                account_id,
                parent,
                rename,
                name,
            } => {
                let title = if rename.is_some() {
                    T::RenameFolder
                } else {
                    T::NewFolder
                };
                ui.heading(title.get(lang).replace('\n', " "));
                let mut text = name.clone();
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut text).hint_text(T::FolderName.get(lang)));
                edit.request_focus();
                let typed = text.trim().to_owned();
                if let Some(Dialog::FolderName { name, .. }) = state.as_mut() {
                    *name = text;
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = ui
                        .add_enabled(!typed.is_empty(), egui::Button::new(T::Ok.get(lang)))
                        .clicked();
                    if (ok || enter) && !typed.is_empty() {
                        result = Some(match rename {
                            Some(id) => Command::RenameFolder {
                                account_id: *account_id,
                                id: id.clone(),
                                name: typed.clone(),
                            },
                            None => Command::CreateFolder {
                                account_id: *account_id,
                                parent: parent.clone(),
                                name: typed.clone(),
                            },
                        });
                        close = true;
                    }
                    if ui.button(T::Cancel.get(lang)).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ConfirmRemoveAccount { account_id, email } => {
                ui.heading(T::RemoveAccount.get(lang));
                ui.label(RichText::new(email).strong());
                ui.label(T::ConfirmRemoveAccountBody.get(lang));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(T::RemoveAccount.get(lang)).clicked() {
                        result = Some(Command::RemoveAccount {
                            account_id: *account_id,
                        });
                        close = true;
                    }
                    if ui.button(T::Cancel.get(lang)).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Welcome { step } => {
                let step = *step;
                let titles = [
                    T::WelcomeTitle,
                    T::WelcomePrivacy,
                    T::WelcomeCleanup,
                    T::WelcomeStartup,
                    T::WelcomeAccount,
                ];
                ui.label(
                    RichText::new(format!(
                        "{} {} / {}",
                        T::StepWord.get(lang),
                        step + 1,
                        titles.len()
                    ))
                    .weak(),
                );
                ui.heading(titles[usize::from(step)].get(lang));
                ui.add_space(6.0);
                match step {
                    0 => {
                        ui.label(T::WelcomeIntro.get(lang));
                        ui.add_space(6.0);
                        appearance_controls(ui, lang, prefs);
                    }
                    1 => {
                        ui.checkbox(
                            &mut prefs.privacy.encrypt_local_copy,
                            T::EncryptLocalCopy.get(lang),
                        );
                        ui.checkbox(
                            &mut prefs.privacy.block_trackers,
                            T::BlockTrackers.get(lang),
                        );
                        ui.label(RichText::new(T::BlockTrackersHelp.get(lang)).small().weak());
                        ui.checkbox(
                            &mut prefs.privacy.block_remote_images,
                            T::BlockRemoteImages.get(lang),
                        );
                        ui.label(
                            RichText::new(T::BlockRemoteImagesHelp.get(lang))
                                .small()
                                .weak(),
                        );
                    }
                    2 => {
                        ui.checkbox(&mut prefs.cleanup.dry_run, T::DryRun.get(lang));
                        ui.label(RichText::new(T::DryRunHelp.get(lang)).small().weak());
                        let was = prefs.cleanup.screener;
                        ui.checkbox(&mut prefs.cleanup.screener, T::Screener.get(lang));
                        if prefs.cleanup.screener && !was {
                            prefs.cleanup.screener_since = chrono::Utc::now().timestamp();
                        }
                        ui.label(RichText::new(T::ScreenerHelp.get(lang)).small().weak());
                    }
                    3 => {
                        ui.checkbox(
                            &mut prefs.startup.with_windows,
                            T::StartWithWindows.get(lang),
                        );
                        ui.add_enabled(
                            prefs.startup.with_windows,
                            egui::Checkbox::new(
                                &mut prefs.startup.minimized,
                                T::StartMinimized.get(lang),
                            ),
                        );
                        ui.checkbox(&mut prefs.startup.close_to_tray, T::CloseToTray.get(lang));
                    }
                    _ => {
                        if c.mail.accounts.is_empty() {
                            if ui
                                .button(T::AddAccount.get(lang).replace('\n', " "))
                                .clicked()
                            {
                                result = Some(Command::AddAccount);
                                prefs.general.onboarded = true;
                                close = true;
                            }
                        } else {
                            for a in &c.mail.accounts {
                                ui.label(RichText::new(&a.email).strong());
                            }
                        }
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if step > 0 && ui.button(T::Previous.get(lang)).clicked() {
                        *state = Some(Dialog::Welcome { step: step - 1 });
                    }
                    let last = usize::from(step) + 1 == titles.len();
                    let next = if last { T::Finish } else { T::Next };
                    if ui.button(next.get(lang)).clicked() {
                        if last {
                            prefs.general.onboarded = true;
                            close = true;
                        } else {
                            *state = Some(Dialog::Welcome { step: step + 1 });
                        }
                    }
                    if !last && ui.button(T::Skip.get(lang)).clicked() {
                        prefs.general.onboarded = true;
                        close = true;
                    }
                });
            }
            Dialog::AllowList { text } => {
                ui.heading(T::AllowList.get(lang).trim_end_matches('…'));
                ui.label(T::AllowListHelp.get(lang));
                let mut edited = text.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut edited)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .hint_text("impots.gouv.fr\nmabanque.fr\npatron@example.com"),
                );
                if let Some(Dialog::AllowList { text }) = state.as_mut() {
                    *text = edited.clone();
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(T::Ok.get(lang)).clicked() {
                        result = Some(Command::SaveAllowList {
                            text: edited.clone(),
                        });
                        close = true;
                    }
                    if ui.button(T::Cancel.get(lang)).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ConfirmCleanup { action, addresses } => {
                ui.heading(T::ConfirmCleanupTitle.get(lang));
                ui.label(format!(
                    "{} — {}",
                    cleanup_label(*action).get(lang).replace('\n', " "),
                    addresses.len()
                ));
                ui.label(T::ConfirmCleanupBody.get(lang));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(cleanup_label(*action).get(lang).replace('\n', " "))
                        .clicked()
                    {
                        result = Some(Command::Cleanup {
                            action: *action,
                            addresses: addresses.clone(),
                        });
                        close = true;
                    }
                    if ui.button(T::Cancel.get(lang)).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Rules { draft } => {
                let mut d = (**draft).clone();
                close = crate::automation_ui::rules(ui, c, prefs, &mut d);
                if let Some(Dialog::Rules { draft }) = state.as_mut() {
                    **draft = d;
                }
            }
            Dialog::Away { away, until } => {
                let (mut a, mut u) = ((**away).clone(), until.clone());
                let (command, done) = crate::automation_ui::away(ui, lang, &mut a, &mut u);
                if let Some(Dialog::Away { away, until }) = state.as_mut() {
                    **away = a;
                    *until = u;
                }
                result = command;
                close = done;
            }
            Dialog::AddAccount { form } => {
                let mut form = form.clone();
                let (command, done) = crate::account_form::show(ui, lang, &mut form);
                if let Some(Dialog::AddAccount { form: f }) = state.as_mut() {
                    *f = form;
                }
                result = command;
                close = done;
            }
            Dialog::Digest { text } => {
                ui.heading(T::DigestTitle.get(lang));
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        let shown = if text.trim().is_empty() {
                            T::DigestEmpty.get(lang)
                        } else {
                            text.as_str()
                        };
                        ui.add(egui::Label::new(shown).selectable(true).wrap());
                    });
                ui.add_space(8.0);
                if ui.button(T::Ok.get(lang)).clicked() {
                    close = true;
                }
            }
            Dialog::Signature { email, text } => {
                ui.heading(T::Signature.get(lang));
                ui.label(RichText::new(email).weak());
                let mut edited = text.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut edited)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                );
                if let Some(Dialog::Signature { text, .. }) = state.as_mut() {
                    *text = edited.clone();
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(T::Ok.get(lang)).clicked() {
                        result = Some(Command::SaveSignature {
                            email: email.clone(),
                            text: edited.trim_end().to_owned(),
                        });
                        close = true;
                    }
                    if ui.button(T::Cancel.get(lang)).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ConfirmDeleteFolder {
                account_id,
                id,
                name,
            } => {
                ui.heading(T::DeleteFolder.get(lang));
                ui.label(format!(
                    "« {name} » — {}",
                    T::ConfirmDeleteFolderBody.get(lang)
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(T::Delete.get(lang)).clicked() {
                        result = Some(Command::DeleteFolder {
                            account_id: *account_id,
                            id: id.clone(),
                        });
                        close = true;
                    }
                    if ui.button(T::Cancel.get(lang)).clicked() {
                        close = true;
                    }
                });
            }
        }
    });
    if modal.should_close() && matches!(current, Dialog::Welcome { .. }) {
        // Escape or a click outside counts as « Skip ».
        prefs.general.onboarded = true;
    }
    if close || modal.should_close() {
        *state = None;
    }
    result
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
                (c.accent_fill.gamma_multiply(0.8), Color32::WHITE)
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

fn ribbon(ui: &mut Ui, c: &Ctx<'_>, view: &mut View, prefs: &mut Prefs) -> Option<Command> {
    let lang = c.lang;
    let later = T::AvailableLater.get(lang);
    let pick_first = T::SelectFirst.get(lang);
    let has_selection = !c.mail.selection.is_empty();
    let account = c.mail.folder.as_ref().map(|(a, _)| *a);
    let mut command = None;
    match view.tab {
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
                if let Some(account) = c.mail.accounts.first()
                    && big_button(ui, c, Icon::Signature, T::Signature, true, "").clicked()
                {
                    let text = prefs
                        .accounts
                        .signatures
                        .get(&account.email)
                        .cloned()
                        .unwrap_or_default();
                    view.dialog = Some(Dialog::Signature {
                        email: account.email.clone(),
                        text,
                    });
                }
                let has_account = !c.mail.accounts.is_empty();
                if big_button(ui, c, Icon::Rules, T::Rules, has_account, "").clicked() {
                    view.dialog = Some(Dialog::Rules {
                        draft: Box::default(),
                    });
                }
                if big_button(ui, c, Icon::Away, T::AwayButton, has_account, "").clicked() {
                    view.dialog = Some(Dialog::Away {
                        until: prefs
                            .away
                            .until
                            .map(crate::automation_ui::format_day)
                            .unwrap_or_default(),
                        away: Box::new(prefs.away.clone()),
                    });
                }
                if big_button(ui, c, Icon::Gear, T::Options, true, "").clicked() {
                    command = Some(Command::OpenOptions);
                }
                if big_button(ui, c, Icon::Exit, T::Exit, true, "").clicked() {
                    command = Some(Command::Exit);
                }
            });
        }
        Tab::Home => {
            group(ui, c, T::GroupNew, |ui| {
                let can = !c.mail.accounts.is_empty();
                if big_button(
                    ui,
                    c,
                    Icon::Envelope,
                    T::NewEmail,
                    can,
                    T::StatusNoAccount.get(lang),
                )
                .clicked()
                {
                    command = Some(Command::NewMessage);
                }
            });
            group(ui, c, T::GroupDelete, |ui| {
                if big_button(ui, c, Icon::Trash, T::Delete, has_selection, pick_first).clicked() {
                    let shift = ui.input(|i| i.modifiers.shift);
                    command = delete_request(c, shift, &mut view.dialog);
                }
                let archive = account
                    .and_then(|a| c.mail.folder_with_role(a, "archive"))
                    .is_some();
                if big_button(
                    ui,
                    c,
                    Icon::Archive,
                    T::Archive,
                    has_selection && archive,
                    pick_first,
                )
                .clicked()
                {
                    command = Some(Command::Archive);
                }
            });
            group(ui, c, T::GroupRespond, |ui| {
                ui.vertical(|ui| {
                    let one = c.mail.selection.len() == 1 && c.mail.selected().is_some();
                    if small_button(ui, c, Icon::Reply, T::Reply, one, pick_first).clicked() {
                        command = Some(Command::Reply(ReplyKind::Reply));
                    }
                    if small_button(ui, c, Icon::ReplyAll, T::ReplyAll, one, pick_first).clicked() {
                        command = Some(Command::Reply(ReplyKind::ReplyAll));
                    }
                    if small_button(ui, c, Icon::Forward, T::Forward, one, pick_first).clicked() {
                        command = Some(Command::Reply(ReplyKind::Forward));
                    }
                });
            });
            group(ui, c, T::GroupMove, |ui| {
                let button = big_button(ui, c, Icon::Move, T::Move, has_selection, pick_first);
                egui::Popup::menu(&button).show(|ui| {
                    if let Some(a) = account
                        && let Some(cmd) = folder_menu(ui, c, a)
                    {
                        command = Some(cmd);
                    }
                });
            });
            group(ui, c, T::GroupTags, |ui| {
                ui.vertical(|ui| {
                    if small_button(
                        ui,
                        c,
                        Icon::Envelope,
                        T::UnreadRead,
                        has_selection,
                        pick_first,
                    )
                    .clicked()
                    {
                        command = Some(Command::ToggleRead);
                    }
                    if small_button(ui, c, Icon::Flag, T::Flag, has_selection, pick_first).clicked()
                    {
                        command = Some(Command::ToggleFlag);
                    }
                });
            });
            group(ui, c, T::GroupSort, |ui| {
                // From the mail view: the sender of the selected message.
                let sender = c
                    .mail
                    .selected()
                    .map(|m| m.from_addr.clone())
                    .filter(|a| !a.is_empty());
                if big_button(
                    ui,
                    c,
                    Icon::Unsubscribe,
                    T::Unsubscribe,
                    sender.is_some(),
                    pick_first,
                )
                .clicked()
                    && let Some(address) = sender
                {
                    let action = mailcraft_sync::CleanAction::Unsubscribe;
                    if prefs.cleanup.dry_run {
                        command = Some(Command::Cleanup {
                            action,
                            addresses: vec![address],
                        });
                    } else {
                        view.dialog = Some(Dialog::ConfirmCleanup {
                            action,
                            addresses: vec![address],
                        });
                    }
                }
                let can = !c.mail.accounts.is_empty();
                if big_button(
                    ui,
                    c,
                    Icon::Analyze,
                    T::Analyze,
                    can,
                    T::StatusNoAccount.get(lang),
                )
                .clicked()
                {
                    command = Some(Command::OpenCleanup);
                }
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
            let current = c.mail.current_folder();
            let own = current.is_some_and(|f| f.role.is_none());
            group(ui, c, T::TabFolder, |ui| {
                if big_button(
                    ui,
                    c,
                    Icon::FolderPlus,
                    T::NewFolder,
                    current.is_some(),
                    pick_first,
                )
                .clicked()
                    && let Some(f) = current
                {
                    view.dialog = Some(folder_name_dialog(
                        f.account_id,
                        Some(f.id.clone()),
                        None,
                        "",
                    ));
                }
                ui.vertical(|ui| {
                    if small_button(
                        ui,
                        c,
                        Icon::Rename,
                        T::RenameFolder,
                        own,
                        T::OwnFoldersOnly.get(lang),
                    )
                    .clicked()
                        && let Some(f) = current
                    {
                        view.dialog = Some(folder_name_dialog(
                            f.account_id,
                            None,
                            Some(f.id.clone()),
                            &f.name,
                        ));
                    }
                    if small_button(
                        ui,
                        c,
                        Icon::Trash,
                        T::DeleteFolder,
                        own,
                        T::OwnFoldersOnly.get(lang),
                    )
                    .clicked()
                        && let Some(f) = current
                    {
                        view.dialog = Some(Dialog::ConfirmDeleteFolder {
                            account_id: f.account_id,
                            id: f.id.clone(),
                            name: f.name.clone(),
                        });
                    }
                });
            });
        }
        Tab::View => {
            group(ui, c, T::GroupAppearance, |ui| {
                appearance_controls(ui, lang, prefs)
            });
            group(ui, c, T::GroupLayout, |ui| {
                ui.checkbox(
                    &mut prefs.appearance.conversations,
                    T::Conversations.get(lang),
                );
            });
        }
    }
    command
}

pub fn folder_name_dialog(
    account_id: i64,
    parent: Option<String>,
    rename: Option<String>,
    name: &str,
) -> Dialog {
    Dialog::FolderName {
        account_id,
        parent,
        rename,
        name: name.to_owned(),
    }
}

/// "Move to" menu: the account's folders as an indented list, except the current one.
pub fn folder_menu(ui: &mut Ui, c: &Ctx<'_>, account_id: i64) -> Option<Command> {
    let current = c.mail.folder.as_ref().map(|(_, f)| f.as_str());
    let mut command = None;
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for (f, depth) in c.mail.folder_tree(account_id) {
                if Some(f.id.as_str()) == current {
                    continue;
                }
                let label = format!("{}{}", "    ".repeat(depth - 1), f.name);
                if ui.button(label).clicked() {
                    command = Some(Command::MoveTo {
                        folder_id: f.id.clone(),
                    });
                }
            }
        });
    command
}

pub fn appearance_controls(ui: &mut Ui, lang: Lang, prefs: &mut Prefs) {
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

            let original = &mut prefs.appearance.original_view;
            let label = ui.label(T::MessageView.get(lang)).id;
            let shown = |o: bool| {
                if o {
                    T::ViewOriginal.get(lang)
                } else {
                    T::ViewSimplified.get(lang)
                }
            };
            ComboBox::from_id_salt("message_view")
                .width(170.0)
                .selected_text(shown(*original))
                .show_ui(ui, |ui| {
                    for v in [false, true] {
                        ui.selectable_value(original, v, shown(v));
                    }
                })
                .response
                .labelled_by(label)
                .on_hover_text(T::MessageViewHelp.get(lang));
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
pub(crate) fn group(ui: &mut Ui, c: &Ctx<'_>, caption: T, add: impl FnOnce(&mut Ui)) {
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

pub(crate) fn big_button(
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

fn cleanup_label(action: mailcraft_sync::CleanAction) -> T {
    use mailcraft_sync::CleanAction;
    match action {
        CleanAction::Unsubscribe => T::Unsubscribe,
        CleanAction::DeleteAll => T::DeleteAll,
        CleanAction::ArchiveAll => T::ArchiveAll,
        CleanAction::Block => T::Block,
    }
}
