//! MailCraft user interface.

mod account_form;
mod attachments;
mod automation_ui;
mod cleanup;
mod composer;
mod fonts;
pub mod i18n;
mod icons;
pub mod mail;
mod options;
mod panes;
mod print;
mod reader;
pub mod settings;
pub mod shell;
mod spell;
mod startup;
pub mod theme;

use std::time::Instant;

use craft_settings::{Location, SaveStatus, SettingsStore};
use egui::{Context, Ui};
use mailcraft_store::{MasterKey, Store};
use mailcraft_sync::{Config, Event, Request, Worker};

use crate::i18n::Lang;
use crate::mail::{Mail, Status};
use crate::settings::{Appearance, Prefs};
pub use crate::shell::Command;
use crate::shell::{Dialog, Pick};

/// A move Ctrl+Z can put back: (message id, folder it came from).
struct Undo {
    account_id: i64,
    moves: Vec<(String, String)>,
}

struct Backend {
    store: Store,
    /// `None` when only showing a store (tests).
    worker: Option<Worker>,
}

pub struct MailCraftApp {
    settings: SettingsStore<Prefs>,
    applied: Option<Appearance>,
    view: shell::View,
    save_error: Option<String>,
    mail: Mail,
    backend: Option<Backend>,
    dirty: bool,
    /// Moves that Ctrl+Z puts back, newest last.
    undo: Vec<Undo>,
    next_composer: u64,
    /// Real app (not a test): touches Windows (start-up entry).
    integrations: bool,
    applied_startup: Option<settings::Startup>,
    /// Newest inbox message already announced, and what to announce next.
    notify_seen: Option<i64>,
    new_mail: Option<(usize, String, String)>,
}

impl MailCraftApp {
    /// The real app: settings in the usual per-user folder, local mail copy, sync worker.
    pub fn new(ctx: &Context) -> Self {
        let mut app = Self::with_location(ctx, Location::Auto);
        match open_backend(ctx, app.settings.get()) {
            Ok(backend) => {
                app.backend = Some(backend);
                app.dirty = true;
            }
            Err(e) => app.mail.status = Status::Error(e),
        }
        app.integrations = true;
        app.apply_integrations();
        if !app.settings.get().general.onboarded {
            app.start_welcome();
        }
        app
    }

    /// Work that goes on even while the window is hidden in the notification area: worker
    /// events, reloading what changed, spotting new mail.
    pub fn background(&mut self, ctx: &Context) {
        self.handle_events(ctx);
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let prefs = self.settings.get();
        let screener = prefs
            .cleanup
            .screener
            .then_some(prefs.cleanup.screener_since);
        self.mail.blocking = (
            prefs.privacy.block_remote_images,
            prefs.privacy.block_trackers,
        );
        self.mail.account_order = prefs.folders.accounts.clone();
        self.mail.folder_order.clone_from(&prefs.folders.folders);
        if let Some(b) = &self.backend
            && let Err(e) = self.mail.reload(
                &b.store,
                &self.view.search,
                self.view.cleanup_open,
                screener,
            )
        {
            self.mail.status = Status::Error(e.to_string());
        }
        if let Some((account_id, id)) = self.mail.wants_inline() {
            self.send(Request::FetchInline { account_id, id });
        }
        self.check_new_mail();
    }

    /// New unread mail in the first account's inbox since the last look, for a notification.
    /// The first look only records where things stand.
    fn check_new_mail(&mut self) {
        let Some(b) = &self.backend else { return };
        let Some(inbox) = self
            .mail
            .accounts
            .first()
            .and_then(|a| self.mail.folder_with_role(a.id, "inbox"))
        else {
            return;
        };
        let Ok(latest) = b.store.messages(inbox.account_id, &inbox.id, "", 30) else {
            return;
        };
        let newest = latest.iter().map(|m| m.received).max().unwrap_or(0);
        if let Some(seen) = self.notify_seen {
            let new: Vec<&mailcraft_store::MessageSummary> = latest
                .iter()
                .filter(|m| m.received > seen && !m.is_read)
                .collect();
            if let Some(first) = new.first() {
                let sender = if first.from_name.is_empty() {
                    first.from_addr.clone()
                } else {
                    first.from_name.clone()
                };
                self.new_mail = Some((new.len(), sender, first.subject.clone()));
            }
        }
        self.notify_seen = Some(self.notify_seen.unwrap_or(0).max(newest));
    }

    /// New mail to announce: (title, text) in the user's language.
    pub fn take_notification(&mut self) -> Option<(String, String)> {
        let (count, sender, subject) = self.new_mail.take()?;
        let lang = Lang::resolve(self.settings.get().general.language);
        let title = if count == 1 {
            sender
        } else {
            format!("{count} {}", i18n::T::NewMessagesWord.get(lang))
        };
        Some((title, subject))
    }

    /// Puts an account (address) just before or after another one in the folder pane.
    fn place_account(&mut self, email: &str, target: &str, after: bool) {
        let current = self.mail.account_emails();
        self.settings.update(|p| {
            settings::FolderLayout::place(&mut p.folders.accounts, &current, email, target, after);
        });
        self.dirty = true;
    }

    /// Puts a folder just before or after one of its siblings.
    fn place_folder(&mut self, account_id: i64, id: &str, target: &str, after: bool) {
        let Some((email, siblings)) = self.mail.siblings(account_id, id) else {
            return;
        };
        self.settings.update(|p| {
            let list = p.folders.folders.entry(email).or_default();
            settings::FolderLayout::place(list, &siblings, id, target, after);
        });
        self.dirty = true;
    }

    /// Opens the shown message, laid out for paper, in the browser's print dialog.
    fn print(&mut self) {
        let (Some(m), Some(body)) = (self.mail.selected().cloned(), self.mail.body.clone()) else {
            return;
        };
        let prefs = self.settings.get();
        let lang = Lang::resolve(prefs.general.language);
        let date = chrono::DateTime::from_timestamp(m.received, 0)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%d/%m/%Y %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let html = print::page(
            &m,
            &body,
            &date,
            lang,
            (
                prefs.privacy.block_remote_images,
                prefs.privacy.block_trackers,
            ),
        );
        let Some(dir) = craft_settings::paths::data_dir("mailcraft") else {
            return;
        };
        let path = dir.join("impression.html");
        if let Err(e) = std::fs::write(&path, html).and_then(|()| open::that_detached(&path)) {
            self.mail.status = Status::Error(format!("{}: {e}", path.display()));
        }
    }

    pub fn french(&self) -> bool {
        Lang::resolve(self.settings.get().general.language) == Lang::Fr
    }

    /// Shows the first-launch assistant.
    pub fn start_welcome(&mut self) {
        self.view.dialog = Some(shell::Dialog::Welcome { step: 0 });
    }

    /// Keeps Windows in step with the start-up settings (real app only, never in tests).
    fn apply_integrations(&mut self) {
        let wanted = self.settings.get().startup.clone();
        if !self.integrations || self.applied_startup.as_ref() == Some(&wanted) {
            return;
        }
        if let Err(e) = startup::apply(wanted.with_windows, wanted.minimized) {
            self.mail.status = Status::Error(format!("Windows start-up: {e}"));
        }
        self.applied_startup = Some(wanted);
    }

    /// Settings only, no mail backend (tests, or when the backend cannot start).
    pub fn with_location(ctx: &Context, location: Location) -> Self {
        fonts::install(ctx);
        spell::warm_up();
        let (settings, report) = SettingsStore::open(location);
        if report.has_problems() {
            log::warn!("settings: {report:?}");
        }
        Self {
            settings,
            applied: None,
            view: shell::View::default(),
            save_error: None,
            mail: Mail::default(),
            backend: None,
            dirty: false,
            undo: Vec::new(),
            next_composer: 1,
            integrations: false,
            applied_startup: None,
            notify_seen: None,
            new_mail: None,
        }
    }

    /// Shows the content of `store` without syncing anything (tests).
    pub fn attach_store(&mut self, store: Store) {
        self.backend = Some(Backend {
            store,
            worker: None,
        });
        self.dirty = true;
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Option<Command> {
        let prefs = self.settings.get().clone();
        if self.applied.as_ref() != Some(&prefs.appearance) {
            theme::apply(ui.ctx(), &prefs.appearance);
            self.applied = Some(prefs.appearance.clone());
        }
        self.background(ui.ctx());

        let mut next = prefs.clone();
        // Scoped: the context borrows the store and the mail only while the window is drawn.
        let command = {
            let store = self.backend.as_ref().map(|b| &b.store);
            let suggest = store.map(suggester);
            let accent_fill = theme::accent_fill(prefs.appearance.accent);
            let c = shell::Ctx {
                lang: Lang::resolve(prefs.general.language),
                accent: theme::accent_text(accent_fill, ui.visuals().dark_mode),
                accent_fill,
                mail: &self.mail,
                save_error: self.save_error.as_deref(),
                block_images: prefs.privacy.block_remote_images,
                screener: prefs.cleanup.screener,
                conversations: prefs.appearance.conversations,
                own_words: &prefs.spell_words,
                folded: &prefs.folders.folded,
                suggest: suggest.as_ref().map(|f| f as &shell::Suggest<'_>),
            };
            shell::show(ui, &c, &mut self.view, &mut next)
        };
        if next != prefs {
            if next.rules != prefs.rules || next.away != prefs.away {
                self.send(Request::SetAutomation {
                    rules: next.rules.clone(),
                    away: next.away.clone(),
                });
            }
            self.settings.update(|p| *p = next);
        }
        self.apply_integrations();
        for url in std::mem::take(&mut self.view.image_requests) {
            if self.mail.images_requested.insert(url.clone()) {
                self.send(Request::FetchImage { url });
            }
        }
        let mut command = command.and_then(|cmd| self.run(cmd));
        if let Some(cmd) = self.composer_windows(ui.ctx(), &prefs) {
            command = self.run(cmd).or(command);
        }
        self.autosave_drafts();
        if self.view.composers.iter().any(|k| k.changed_at.is_some()) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(5));
        }

        let tick = self.settings.tick(Instant::now());
        match &tick.status {
            SaveStatus::Failed(err) => self.save_error = Some(err.to_string()),
            SaveStatus::Saved => self.save_error = None,
            _ => {}
        }
        if let Some(deadline) = tick.next_deadline {
            ui.ctx()
                .request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
        if self.dirty {
            ui.ctx().request_repaint();
        }
        command
    }

    fn worker(&self) -> Option<&Worker> {
        self.backend.as_ref()?.worker.as_ref()
    }

    /// Where the original view (WebView) goes this frame and what it shows; `None` hides it
    /// (no message in original mode, or a dialog that the native view would cover).
    pub fn original_view(&self) -> Option<(egui::Rect, &str)> {
        if self.view.dialog.is_some() {
            return None;
        }
        let rect = self.view.webview?;
        Some((rect, self.mail.original_html.as_str()))
    }

    /// The WebView could not start (WebView2 runtime missing): back to the simplified view.
    pub fn original_unavailable(&mut self) {
        if self.view.original {
            self.view.original = false;
            self.mail.status = Status::Error("Edge WebView2 unavailable: original view off".into());
        }
    }

    fn handle_events(&mut self, ctx: &Context) {
        let Some(worker) = self.worker() else { return };
        for event in worker.poll() {
            match event {
                Event::SigningIn => {
                    self.mail.signing_in = true;
                    if self.mail.status != Status::Connecting {
                        self.mail.status = Status::SigningIn;
                    }
                }
                Event::SignInCancelled => {
                    self.mail.signing_in = false;
                    self.mail.status = Status::Ready;
                }
                Event::AccountAdded { .. } => {
                    self.mail.signing_in = false;
                    self.dirty = true;
                }
                Event::Syncing { folder, messages } => {
                    self.mail.status = Status::Syncing { folder, messages }
                }
                Event::SyncDone => {
                    if !self.mail.accounts.is_empty()
                        && !matches!(self.mail.status, Status::Error(_))
                    {
                        self.mail.status = Status::UpToDate;
                    }
                    self.dirty = true;
                }
                Event::Changed | Event::BodyReady { .. } => self.dirty = true,
                Event::Offline { pending, .. } => self.mail.status = Status::Offline { pending },
                Event::Exported { count, dir } => {
                    self.mail.status = Status::Exported(count);
                    let _ = open::that_detached(dir);
                }
                Event::ServerSearchDone { found } => {
                    self.mail.status = Status::ServerFound(found);
                    self.dirty = true;
                }
                Event::InlineReady {
                    account_id,
                    id,
                    images,
                } => {
                    if self.mail.message.as_ref() == Some(&(account_id, id)) {
                        for (cid, bytes) in &images {
                            ctx.include_bytes(format!("bytes://cid/{cid}"), bytes.clone());
                        }
                        self.mail.set_inline(images);
                    }
                }
                Event::ImageReady { url, bytes } => {
                    if let Some(bytes) = bytes {
                        ctx.include_bytes(url.clone(), bytes);
                        self.mail.images_ready.insert(url);
                    }
                }
                Event::AttachmentSaved { .. } => self.mail.status = Status::AttachmentSaved,
                Event::DraftSaved { composer, draft_id } => {
                    if let Some(k) = self.view.composers.iter_mut().find(|k| k.id == composer) {
                        k.draft_id = Some(draft_id);
                    }
                }
                Event::Sent { composer } => {
                    self.view.composers.retain(|k| k.id != composer);
                    self.mail.status = Status::Sent;
                }
                Event::Analyzing { done, total } => self.mail.analyzing = Some((done, total)),
                Event::AnalysisDone => {
                    self.mail.analyzing = None;
                    self.dirty = true;
                }
                Event::CleanupDone {
                    done,
                    skipped,
                    dry_run,
                } => {
                    self.mail.status = Status::CleanupDone {
                        done,
                        skipped,
                        dry_run,
                    };
                    self.dirty = true;
                }
                Event::OpenPage { url } => ctx.open_url(egui::OpenUrl::new_tab(url)),
                Event::Classifying { done, total } => self.mail.classifying = Some((done, total)),
                Event::ClassifyDone { count } => {
                    self.mail.classifying = None;
                    self.mail.status = Status::ClassifyDone(count);
                    self.dirty = true;
                }
                Event::DigestReady { text } => {
                    self.view.dialog = Some(shell::Dialog::Digest { text })
                }
                Event::SendFailed { composer, error } => {
                    if let Some(k) = self.view.composers.iter_mut().find(|k| k.id == composer) {
                        k.sending = false;
                        k.error = Some(error);
                    }
                }
                Event::Error(e) => {
                    self.mail.signing_in = false;
                    self.mail.classifying = None;
                    self.mail.status = Status::Error(e);
                }
            }
        }
    }

    /// Handles what the window can; returns what the host must do (exit).
    fn run(&mut self, command: Command) -> Option<Command> {
        match command {
            Command::Exit => return Some(Command::Exit),
            Command::AddAccount => {
                self.view.dialog = Some(shell::Dialog::AddAccount {
                    form: Box::default(),
                });
            }
            Command::Print => self.print(),
            Command::ToggleFold { key } => {
                self.settings.update(|p| p.folders.toggle(&key));
            }
            Command::MoveAccount { email, up } => {
                let current = self.mail.account_emails();
                let email = email.to_lowercase();
                if let Some((target, after)) =
                    settings::FolderLayout::neighbour(&current, &email, up)
                {
                    self.place_account(&email, &target, after);
                }
            }
            Command::PlaceAccount {
                email,
                target,
                after,
            } => self.place_account(&email.to_lowercase(), &target.to_lowercase(), after),
            Command::MoveFolder { account_id, id, up } => {
                let (_, siblings) = self.mail.siblings(account_id, &id)?;
                if let Some((target, after)) = settings::FolderLayout::neighbour(&siblings, &id, up)
                {
                    self.place_folder(account_id, &id, &target, after);
                }
            }
            Command::PlaceFolder {
                account_id,
                id,
                target,
                after,
            } => self.place_folder(account_id, &id, &target, after),
            Command::Export => {
                let selected = self.mail.selected_messages();
                let account_id = selected.first().map(|m| m.account_id)?;
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let mut used = std::collections::HashSet::new();
                    let items = selected
                        .iter()
                        .filter(|m| m.account_id == account_id)
                        .map(|m| (m.id.clone(), print::eml_name(m, &mut used)))
                        .collect();
                    self.send(Request::Export {
                        account_id,
                        items,
                        dir,
                    });
                }
            }
            Command::SaveAway { away } => {
                let away = *away;
                self.settings.update(|p| p.away = away.clone());
                let rules = self.settings.get().rules.clone();
                self.send(Request::SetAutomation {
                    rules,
                    away: away.clone(),
                });
                self.send(Request::SetAway { away });
            }
            Command::AddImapAccount { server } => {
                self.mail.signing_in = true;
                self.mail.status = Status::Connecting;
                self.send(Request::AddImapAccount { server });
            }
            Command::AddGoogleAccount => {
                self.mail.signing_in = true;
                self.mail.status = Status::SigningIn;
                self.send(Request::AddGoogleAccount);
            }
            Command::AddMicrosoftAccount => {
                self.mail.signing_in = true;
                self.mail.status = Status::SigningIn;
                self.send(Request::AddMicrosoftAccount);
            }
            Command::CancelSignIn => {
                if let Some(w) = self.worker() {
                    w.cancel_sign_in();
                }
            }
            Command::SyncAll => {
                if !matches!(self.mail.status, Status::Syncing { .. }) {
                    self.mail.status = Status::Ready;
                }
                self.send(Request::SyncAll);
            }
            Command::SelectFolder {
                account_id,
                folder_id,
            } => {
                self.mail.folder = Some((account_id, folder_id));
                self.mail.message = None;
                self.mail.selection.clear();
                self.mail.anchor = None;
                self.dirty = true;
            }
            Command::SelectMessage {
                account_id,
                id,
                pick,
            } => {
                // Each message opens in the simplified view first.
                self.view.original = false;
                self.select(account_id, &id, pick);
            }
            Command::SelectAll => {
                self.mail.selection = self
                    .mail
                    .messages
                    .iter()
                    .map(|m| (m.account_id, m.id.clone()))
                    .collect();
                self.mail.message = None;
            }
            Command::Step(delta) => {
                let at = self.mail.message.as_ref().and_then(|(a, id)| {
                    self.mail
                        .messages
                        .iter()
                        .position(|m| m.account_id == *a && &m.id == id)
                });
                let next = match at {
                    Some(i) => i
                        .saturating_add_signed(delta as isize)
                        .min(self.mail.messages.len().saturating_sub(1)),
                    None => 0,
                };
                if let Some(m) = self.mail.messages.get(next) {
                    let (a, id) = (m.account_id, m.id.clone());
                    self.select(a, &id, Pick::Only);
                }
            }
            Command::SearchChanged => self.dirty = true,
            Command::OpenAttachment { index } | Command::SaveAttachment { index } => {
                let open = matches!(command, Command::OpenAttachment { .. });
                let (Some((account_id, message_id)), Some(a)) = (
                    self.mail.message.clone(),
                    self.mail.attachments.get(index).cloned(),
                ) else {
                    return None;
                };
                let name = attachments::safe_file_name(&a.name);
                let path = if open {
                    if attachments::is_risky(&a.name) {
                        return None;
                    }
                    // Its own folder per message, so two "facture.pdf" never collide.
                    let dir = craft_settings::paths::cache_dir("mailcraft").map(|d| {
                        d.join("attachments")
                            .join(format!("{:x}", hash(&message_id)))
                    });
                    dir.and_then(|d| std::fs::create_dir_all(&d).ok().map(|_| d.join(&name)))
                } else {
                    rfd::FileDialog::new().set_file_name(&name).save_file()
                };
                if let Some(path) = path {
                    self.send(Request::SaveAttachment {
                        account_id,
                        message_id,
                        attachment_id: a.id,
                        path,
                        open,
                    });
                }
            }
            Command::NewMessage => {
                let account = self.mail.folder.as_ref().map(|(a, _)| *a).or(self
                    .mail
                    .accounts
                    .first()
                    .map(|a| a.id));
                if let Some(account_id) = account {
                    let mut k = self.new_composer(account_id, composer::Kind::New);
                    k.detached = true;
                    self.view.composers.push(k);
                }
            }
            Command::Reply(kind) => self.start_reply(kind),
            Command::Composer { id, action } => self.composer_action(id, action),
            Command::SaveSignature { email, text } => {
                self.settings.update(|p| {
                    if text.is_empty() {
                        p.accounts.signatures.remove(&email);
                    } else {
                        p.accounts.signatures.insert(email, text);
                    }
                });
            }
            cmd @ (Command::Screen { .. }
            | Command::OpenOptions
            | Command::CloseOptions
            | Command::RebuildLocalCopy
            | Command::RemoveAccount { .. }
            | Command::OpenCleanup
            | Command::CloseCleanup
            | Command::Analyze { .. }
            | Command::Cleanup { .. }
            | Command::AllowSenders { .. }
            | Command::SaveAllowList { .. }
            | Command::SaveClaudeKey { .. }
            | Command::DeleteClaudeKey
            | Command::AiProviderChanged
            | Command::Classify { .. }
            | Command::Digest) => self.run_view(cmd),
            Command::SearchServer => {
                if let Some((account_id, _)) = self.mail.folder.clone() {
                    self.mail.status = Status::SearchingServer;
                    self.send(Request::SearchServer {
                        account_id,
                        query: self.view.search.clone(),
                    });
                }
            }
            Command::Delete { permanent } => {
                let in_trash = self.mail.current_folder().and_then(|f| f.role.as_deref())
                    == Some("deleteditems");
                if permanent {
                    if let Some((account_id, ids)) = self.take_selection() {
                        self.act(Request::Delete { account_id, ids });
                    }
                } else if in_trash {
                    self.view.dialog = Some(Dialog::ConfirmPermanentDelete {
                        count: self.mail.selection.len(),
                    });
                } else {
                    self.move_selection_to_role("deleteditems");
                }
            }
            Command::Archive => self.move_selection_to_role("archive"),
            Command::MoveTo { folder_id } => self.move_selection(&folder_id),
            Command::ToggleRead => {
                let selected = self.mail.selected_messages();
                let read = selected.iter().any(|m| !m.is_read);
                let account_id = selected.first().map(|m| m.account_id)?;
                let ids = selected.iter().map(|m| m.id.clone()).collect();
                self.act(Request::SetRead {
                    account_id,
                    ids,
                    read,
                });
            }
            Command::ToggleFlag => {
                let selected = self.mail.selected_messages();
                let flagged = selected.iter().any(|m| !m.flagged);
                let account_id = selected.first().map(|m| m.account_id)?;
                let ids = selected.iter().map(|m| m.id.clone()).collect();
                self.act(Request::SetFlag {
                    account_id,
                    ids,
                    flagged,
                });
            }
            Command::Undo => {
                if let Some(undo) = self.undo.pop() {
                    // Grouped by the folder each message came from.
                    let mut back: Vec<(String, Vec<String>)> = Vec::new();
                    for (id, from) in undo.moves {
                        match back.iter_mut().find(|(f, _)| *f == from) {
                            Some((_, ids)) => ids.push(id),
                            None => back.push((from, vec![id])),
                        }
                    }
                    for (dest, ids) in back {
                        self.act(Request::Move {
                            account_id: undo.account_id,
                            ids,
                            dest,
                        });
                    }
                }
            }
            Command::CreateFolder {
                account_id,
                parent,
                name,
            } => {
                self.send(Request::CreateFolder {
                    account_id,
                    parent,
                    name,
                });
            }
            Command::RenameFolder {
                account_id,
                id,
                name,
            } => {
                self.send(Request::RenameFolder {
                    account_id,
                    id,
                    name,
                });
            }
            Command::DeleteFolder { account_id, id } => {
                self.send(Request::DeleteFolder { account_id, id });
            }
        }
        None
    }

    fn send(&self, request: Request) {
        if let Some(w) = self.worker() {
            w.send(request);
        }
    }

    /// A mailbox change: through the worker (local first, then the server), or straight into
    /// the store when there is no worker (tests).
    fn act(&mut self, request: Request) {
        if self.worker().is_some() {
            self.send(request);
            return;
        }
        let Some(b) = self.backend.as_mut() else {
            return;
        };
        let store = &mut b.store;
        let result = match &request {
            Request::Move {
                account_id,
                ids,
                dest,
            } => store.move_local(*account_id, ids, dest).map(|_| ()),
            Request::Delete { account_id, ids } => store.delete_local(*account_id, ids),
            Request::SetRead {
                account_id,
                ids,
                read,
            } => ids
                .iter()
                .try_for_each(|id| store.set_read(*account_id, id, *read)),
            Request::SetFlag {
                account_id,
                ids,
                flagged,
            } => ids
                .iter()
                .try_for_each(|id| store.set_flag(*account_id, id, *flagged)),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.mail.status = Status::Error(e.to_string());
        }
        self.dirty = true;
    }

    fn select(&mut self, account_id: i64, id: &str, pick: Pick) {
        let key = (account_id, id.to_owned());
        match pick {
            Pick::Only => {
                self.mail.selection = vec![key.clone()];
                self.mail.anchor = Some(key.clone());
                self.mail.message = Some(key);
                let unread = self
                    .mail
                    .messages
                    .iter()
                    .any(|m| m.account_id == account_id && m.id == id && !m.is_read);
                self.send(Request::FetchBody {
                    account_id,
                    id: id.to_owned(),
                });
                if unread {
                    self.act(Request::SetRead {
                        account_id,
                        ids: vec![id.to_owned()],
                        read: true,
                    });
                }
            }
            Pick::Toggle => {
                if let Some(i) = self.mail.selection.iter().position(|k| *k == key) {
                    self.mail.selection.remove(i);
                } else {
                    self.mail.selection.push(key.clone());
                }
                self.mail.anchor = Some(key);
                self.mail.message = self
                    .mail
                    .selection
                    .first()
                    .cloned()
                    .filter(|_| self.mail.selection.len() == 1);
            }
            Pick::Range => {
                let pos = |k: &(i64, String)| {
                    self.mail
                        .messages
                        .iter()
                        .position(|m| m.account_id == k.0 && m.id == k.1)
                };
                let to = pos(&key);
                let from = self.mail.anchor.as_ref().and_then(pos).or(to);
                if let (Some(a), Some(b)) = (from, to) {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    self.mail.selection = self.mail.messages[lo..=hi]
                        .iter()
                        .map(|m| (m.account_id, m.id.clone()))
                        .collect();
                }
                self.mail.message = None;
            }
        }
        self.dirty = true;
    }

    /// Takes the selection for an action that removes it from the list, and selects the next
    /// message below it, as Outlook does.
    fn take_selection(&mut self) -> Option<(i64, Vec<String>)> {
        let selected = self.mail.selected_messages();
        // One account per action (the shared inbox can mix them): the first one's messages.
        let account_id = selected.first()?.account_id;
        let ids: Vec<String> = selected
            .iter()
            .filter(|m| m.account_id == account_id)
            .map(|m| m.id.clone())
            .collect();
        let last = self.mail.messages.iter().rposition(|m| ids.contains(&m.id));
        let next = last.and_then(|i| {
            self.mail.messages[i + 1..]
                .iter()
                .find(|m| !ids.contains(&m.id))
        });
        let next = next.map(|m| (m.account_id, m.id.clone()));
        self.mail.selection = next.iter().cloned().collect();
        self.mail.anchor = next.clone();
        self.mail.message = next;
        Some((account_id, ids))
    }

    fn move_selection_to_role(&mut self, role: &str) {
        let Some(account_id) = self.mail.selected_messages().first().map(|m| m.account_id) else {
            return;
        };
        if let Some(dest) = self
            .mail
            .folder_with_role(account_id, role)
            .map(|f| f.id.clone())
        {
            self.move_selection(&dest);
        }
    }

    fn move_selection(&mut self, dest: &str) {
        if dest == mail::SCREENER || dest == mail::UNIFIED {
            return;
        }
        let account = self.mail.selected_messages().first().map(|m| m.account_id);
        // Where each message comes from (search results mix folders); those already in `dest` stay.
        let from: Vec<(String, String)> = self
            .mail
            .selected_messages()
            .iter()
            .filter(|m| m.folder_id != dest && Some(m.account_id) == account)
            .map(|m| (m.id.clone(), m.folder_id.clone()))
            .collect();
        if from.is_empty() {
            return;
        }
        let Some((account_id, _)) = self.take_selection() else {
            return;
        };
        let ids = from.iter().map(|(id, _)| id.clone()).collect();
        self.undo.push(Undo {
            account_id,
            moves: from,
        });
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
        self.act(Request::Move {
            account_id,
            ids,
            dest: dest.to_owned(),
        });
    }

    pub fn prefs(&self) -> &Prefs {
        self.settings.get()
    }

    pub fn update_prefs(&mut self, f: impl FnOnce(&mut Prefs)) {
        self.settings.update(f);
    }

    /// Writes pending settings; call on exit.
    fn new_composer(&mut self, account_id: i64, kind: composer::Kind) -> composer::Composer {
        let id = self.next_composer;
        self.next_composer += 1;
        let mut k = composer::Composer::new(id, account_id, kind);
        let email = self
            .mail
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.email.clone());
        if let Some(sig) =
            email.and_then(|e| self.settings.get().accounts.signatures.get(&e).cloned())
        {
            k.doc = mailcraft_html::doc::Doc::plain(&format!("\n\n-- \n{sig}"));
        }
        k
    }

    /// Reply, reply all or forward the shown message, in the reading pane.
    fn start_reply(&mut self, kind: shell::ReplyKind) {
        use shell::ReplyKind;
        let Some(m) = self.mail.selected().cloned() else {
            return;
        };
        // One reply at a time in the pane; another one goes to its own window.
        let inline_taken = self.view.composers.iter().any(|k| !k.detached);
        let lang = Lang::resolve(self.settings.get().general.language);
        let own = self
            .mail
            .accounts
            .iter()
            .find(|a| a.id == m.account_id)
            .map(|a| a.email.to_lowercase());
        let body = self.mail.body.clone().unwrap_or_default();
        let sender = if m.from_name.is_empty() {
            m.from_addr.clone()
        } else {
            format!("{} <{}>", m.from_name, m.from_addr)
        };
        let (ck, prefix) = match kind {
            ReplyKind::Reply => (
                composer::Kind::Reply {
                    original: m.id.clone(),
                },
                i18n::T::ReplyPrefix,
            ),
            ReplyKind::ReplyAll => (
                composer::Kind::ReplyAll {
                    original: m.id.clone(),
                },
                i18n::T::ReplyPrefix,
            ),
            ReplyKind::Forward => (
                composer::Kind::Forward {
                    original: m.id.clone(),
                },
                i18n::T::ForwardPrefix,
            ),
        };
        let mut k = self.new_composer(m.account_id, ck);
        k.detached = inline_taken;
        let prefix = prefix.get(lang);
        let bare = |s: &str| {
            let t = s.trim_start();
            ["RE:", "RE :", "TR:", "TR :", "FW:", "FWD:"]
                .iter()
                .any(|p| t.len() >= p.len() && t[..p.len()].eq_ignore_ascii_case(p))
        };
        k.subject = if bare(&m.subject) {
            m.subject.clone()
        } else {
            format!("{prefix}{}", m.subject)
        };
        let not_me = |a: &&str| {
            own.as_deref()
                .is_none_or(|me| !a.to_lowercase().contains(me))
        };
        match kind {
            ReplyKind::Reply => k.to = sender.clone(),
            ReplyKind::ReplyAll => {
                let mut to = vec![sender.as_str()];
                to.extend(
                    body.to
                        .split(';')
                        .map(str::trim)
                        .filter(|a| !a.is_empty())
                        .filter(not_me),
                );
                k.to = to.join("; ");
                k.cc = body
                    .cc
                    .split(';')
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .filter(not_me)
                    .collect::<Vec<_>>()
                    .join("; ");
            }
            ReplyKind::Forward => {}
        }
        k.quoted = self.mail.blocks.clone();
        k.quoted_header = format!("{} — {sender}", i18n::T::OriginalMessage.get(lang));
        self.view.composers.push(k);
    }

    fn outgoing(k: &composer::Composer) -> mailcraft_graph::compose::Outgoing {
        use mailcraft_graph::compose::split_recipients;
        mailcraft_graph::compose::Outgoing {
            subject: k.subject.trim().to_owned(),
            html: k.doc.to_html(),
            to: split_recipients(&k.to),
            cc: split_recipients(&k.cc),
            bcc: split_recipients(&k.bcc),
        }
    }

    fn compose_base(kind: &composer::Kind) -> mailcraft_sync::Compose {
        use mailcraft_sync::Compose;
        match kind {
            composer::Kind::New => Compose::New,
            composer::Kind::Reply { original } => Compose::Reply(original.clone()),
            composer::Kind::ReplyAll { original } => Compose::ReplyAll(original.clone()),
            composer::Kind::Forward { original } => Compose::Forward(original.clone()),
        }
    }

    fn composer_action(&mut self, id: u64, action: composer::Action) {
        let Some(pos) = self.view.composers.iter().position(|k| k.id == id) else {
            return;
        };
        match action {
            composer::Action::Send => {
                let k = &mut self.view.composers[pos];
                let out = Self::outgoing(k);
                if out.to.is_empty() && out.cc.is_empty() && out.bcc.is_empty() {
                    k.error = Some(
                        i18n::T::NoRecipient
                            .get(Lang::resolve(self.settings.get().general.language))
                            .into(),
                    );
                    return;
                }
                if let Some(bad) = [&k.to, &k.cc, &k.bcc]
                    .iter()
                    .flat_map(|f| mailcraft_graph::compose::split_recipients(f))
                    .find(|r| mailcraft_graph::compose::recipient(r).is_none())
                {
                    k.error = Some(format!(
                        "{} {bad}",
                        i18n::T::BadAddress
                            .get(Lang::resolve(self.settings.get().general.language))
                    ));
                    return;
                }
                k.sending = true;
                k.error = None;
                let request = Request::Send {
                    composer: k.id,
                    account_id: k.account_id,
                    base: Self::compose_base(&k.kind),
                    draft_id: k.draft_id.clone(),
                    out,
                    attachments: k.attachments.clone(),
                };
                self.send(request);
            }
            composer::Action::PickAttachments => {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    let k = &mut self.view.composers[pos];
                    k.attachments.extend(files);
                    k.changed_at = Some(Instant::now());
                }
            }
            composer::Action::Detach => self.view.composers[pos].detached = true,
            composer::Action::LearnWord(word) => {
                let word = word.trim().to_owned();
                self.settings.update(|p| {
                    if !p.spell_words.iter().any(|w| w.eq_ignore_ascii_case(&word)) {
                        p.spell_words.push(word);
                    }
                });
            }
            composer::Action::Close(choice) => {
                let k = &self.view.composers[pos];
                match choice {
                    None if k.has_content() && !k.sending => {
                        self.view.composers[pos].confirm_close = true
                    }
                    Some(true) => {
                        let request = Request::SaveDraft {
                            composer: k.id,
                            account_id: k.account_id,
                            base: Self::compose_base(&k.kind),
                            draft_id: k.draft_id.clone(),
                            out: Self::outgoing(k),
                        };
                        self.send(request);
                        self.view.composers.remove(pos);
                    }
                    _ => {
                        if let Some(draft_id) = k.draft_id.clone() {
                            self.send(Request::DiscardDraft {
                                account_id: k.account_id,
                                draft_id,
                            });
                        }
                        self.view.composers.remove(pos);
                    }
                }
            }
        }
    }

    /// Detached composers, each in its own window (viewport).
    fn composer_windows(&mut self, ctx: &Context, prefs: &Prefs) -> Option<Command> {
        let lang = Lang::resolve(prefs.general.language);
        let accent_fill = theme::accent_fill(prefs.appearance.accent);
        let accent = theme::accent_text(accent_fill, ctx.theme() == egui::Theme::Dark);
        let mut out = None;
        let mail = &self.mail;
        let store = self.backend.as_ref().map(|b| &b.store);
        let finder = store.map(suggester);
        let suggest = finder.as_ref();
        for k in self.view.composers.iter_mut().filter(|k| k.detached) {
            let title = if k.subject.trim().is_empty() {
                i18n::T::NewMessage.get(lang).to_owned()
            } else {
                k.subject.clone()
            };
            let id = k.id;
            let builder = egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([820.0, 680.0]);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("composer", id)),
                builder,
                |ui, _| {
                    let c = shell::Ctx {
                        lang,
                        accent,
                        accent_fill,
                        mail,
                        save_error: None,
                        block_images: prefs.privacy.block_remote_images,
                        screener: prefs.cleanup.screener,
                        conversations: prefs.appearance.conversations,
                        own_words: &prefs.spell_words,
                        folded: &prefs.folders.folded,
                        suggest: suggest.map(|f| f as &shell::Suggest<'_>),
                    };
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink(false)
                            .show(ui, |ui| {
                                if let Some(action) = composer::show(ui, &c, k) {
                                    out = Some(Command::Composer { id, action });
                                }
                            });
                    });
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        // Closing the window goes through the same question as the Close button.
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                        out = Some(Command::Composer {
                            id,
                            action: composer::Action::Close(None),
                        });
                    }
                },
            );
        }
        out
    }

    /// Every 30 s of quiet after a change, the server draft is updated (text and recipients;
    /// attachments go up when sending).
    fn autosave_drafts(&mut self) {
        let due: Vec<usize> = self
            .view
            .composers
            .iter()
            .enumerate()
            .filter(|(_, k)| {
                !k.sending
                    && k.has_content()
                    && k.changed_at.is_some_and(|t| t.elapsed().as_secs() >= 30)
            })
            .map(|(i, _)| i)
            .collect();
        for i in due {
            let k = &mut self.view.composers[i];
            k.changed_at = None;
            let request = Request::SaveDraft {
                composer: k.id,
                account_id: k.account_id,
                base: Self::compose_base(&k.kind),
                draft_id: k.draft_id.clone(),
                out: Self::outgoing(k),
            };
            self.send(request);
        }
    }

    pub fn flush(&mut self) {
        if let Err(err) = self.settings.flush() {
            log::warn!("settings not saved on exit: {err}");
        }
    }
}

fn open_backend(ctx: &Context, prefs: &Prefs) -> Result<Backend, String> {
    let dir =
        craft_settings::paths::data_dir("mailcraft").ok_or("no folder for MailCraft's data")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let db_path = dir.join("mailcraft.db");
    let key = MasterKey::load_or_create().map_err(|e| e.to_string())?;
    let encrypt = prefs.privacy.encrypt_local_copy;
    // Encryption switched in Options: rebuild the local copy (accounts and sessions kept).
    let marker = dir.join("rebuild-local-copy");
    let store = if marker.exists() {
        let s = Store::recreate(&db_path, key.clone(), encrypt).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&marker);
        s
    } else {
        Store::open(&db_path, key.clone(), encrypt).map_err(|e| e.to_string())?
    };
    let client_id = match prefs.accounts.microsoft_client_id.trim() {
        "" => mailcraft_graph::auth::DEFAULT_CLIENT_ID.to_owned(),
        id => id.to_owned(),
    };
    let repaint = ctx.clone();
    let worker = Worker::start(
        Config {
            db_path,
            key,
            encrypt_new: encrypt,
            client_id,
            months: prefs.sync.months.max(1),
        },
        std::sync::Arc::new(move || repaint.request_repaint()),
    );
    worker.send(Request::SetAutomation {
        rules: prefs.rules.clone(),
        away: prefs.away.clone(),
    });
    Ok(Backend {
        store,
        worker: Some(worker),
    })
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Address completion from the local store (contacts, then frequent senders).
fn suggester(store: &Store) -> impl Fn(i64, &str) -> Vec<(String, String)> + '_ {
    move |account, typed| {
        store
            .address_suggestions(account, typed, 8)
            .unwrap_or_default()
    }
}

impl MailCraftApp {
    /// The account the user is looking at (its folder), else the first one.
    fn current_account(&self) -> Option<i64> {
        self.mail
            .folder
            .as_ref()
            .map(|(a, _)| *a)
            .or(self.mail.accounts.first().map(|a| a.id))
    }
}

impl MailCraftApp {
    /// Whether the chosen AI provider has a key in the vault (real app only: tests never touch
    /// the vault).
    fn read_key_state(&mut self) {
        if self.integrations {
            let name = self.settings.get().ai.provider.vault_name();
            self.view.claude_key_saved = Some(
                mailcraft_store::vault::secret(name)
                    .ok()
                    .flatten()
                    .is_some(),
            );
        } else {
            self.view.claude_key_saved = None;
        }
    }

    /// Commands of the Options, clean-up and screener views.
    fn run_view(&mut self, command: Command) {
        match command {
            Command::SaveClaudeKey { key } => {
                let name = self.settings.get().ai.provider.vault_name();
                match mailcraft_store::vault::set_secret(name, key.trim()) {
                    Ok(()) => self.view.claude_key_saved = Some(true),
                    Err(e) => self.mail.status = Status::Error(e.to_string()),
                }
            }
            Command::DeleteClaudeKey => {
                let name = self.settings.get().ai.provider.vault_name();
                match mailcraft_store::vault::delete_secret(name) {
                    Ok(()) => self.view.claude_key_saved = Some(false),
                    Err(e) => self.mail.status = Status::Error(e.to_string()),
                }
            }
            Command::AiProviderChanged => self.read_key_state(),
            Command::Classify { force } => {
                if let Some(account_id) = self.current_account() {
                    self.mail.classifying = Some((0, 0));
                    let ai = self.settings.get().ai.clone();
                    self.send(Request::ClassifySenders {
                        account_id,
                        force,
                        ai,
                    });
                }
            }
            Command::Digest => {
                if let Some(account_id) = self.current_account() {
                    let ai = self.settings.get().ai.clone();
                    self.send(Request::Digest { account_id, ai });
                }
            }
            Command::Screen { address, accept } => {
                let Some(account_id) = self.current_account() else {
                    return;
                };
                if let Some(b) = &self.backend
                    && let Err(e) = b.store.set_screened(account_id, &address, accept)
                {
                    self.mail.status = Status::Error(e.to_string());
                }
                if !accept {
                    let prefs = self.settings.get();
                    let request = Request::Cleanup {
                        account_id,
                        addresses: vec![address],
                        action: mailcraft_sync::CleanAction::Block,
                        dry_run: prefs.cleanup.dry_run,
                        protected: prefs.cleanup.allow_list.clone(),
                    };
                    self.send(request);
                }
                self.mail.message = None;
                self.mail.selection.clear();
                self.dirty = true;
            }
            Command::OpenOptions => {
                self.view.options_open = true;
                self.view.cleanup_open = false;
                self.read_key_state();
            }
            Command::CloseOptions => self.view.options_open = false,
            Command::RebuildLocalCopy => {
                let marker = craft_settings::paths::data_dir("mailcraft")
                    .map(|d| d.join("rebuild-local-copy"));
                match marker.map(|m| std::fs::write(m, b"")) {
                    Some(Ok(())) => self.mail.status = Status::RebuildScheduled,
                    Some(Err(e)) => self.mail.status = Status::Error(e.to_string()),
                    None => {}
                }
            }
            Command::RemoveAccount { account_id } => {
                if let Some(b) = &self.backend
                    && let Err(e) = b.store.delete_account(account_id)
                {
                    self.mail.status = Status::Error(e.to_string());
                }
                self.mail.folder = None;
                self.mail.message = None;
                self.dirty = true;
            }
            Command::OpenCleanup => {
                self.view.cleanup_open = true;
                self.view.options_open = false;
                self.dirty = true;
                if let Some(account_id) = self.current_account() {
                    self.send(Request::AnalyzeSenders {
                        account_id,
                        force: false,
                    });
                }
            }
            Command::CloseCleanup => {
                self.view.cleanup_open = false;
                self.dirty = true;
            }
            Command::Analyze { force } => {
                if let Some(account_id) = self.current_account() {
                    self.send(Request::AnalyzeSenders { account_id, force });
                }
            }
            Command::Cleanup { action, addresses } => {
                if let Some(account_id) = self.current_account() {
                    let prefs = self.settings.get();
                    let request = Request::Cleanup {
                        account_id,
                        addresses,
                        action,
                        dry_run: prefs.cleanup.dry_run,
                        protected: prefs.cleanup.allow_list.clone(),
                    };
                    self.send(request);
                    self.view.cleanup.checked.clear();
                }
            }
            Command::AllowSenders { addresses } => {
                self.settings.update(|p| {
                    for a in addresses {
                        if !p
                            .cleanup
                            .allow_list
                            .iter()
                            .any(|x| x.eq_ignore_ascii_case(&a))
                        {
                            p.cleanup.allow_list.push(a);
                        }
                    }
                });
            }
            Command::SaveAllowList { text } => {
                let list: Vec<String> = text
                    .lines()
                    .map(|l| l.trim().trim_start_matches('@').to_lowercase())
                    .filter(|l| !l.is_empty())
                    .collect();
                self.settings.update(|p| p.cleanup.allow_list = list);
            }
            _ => {}
        }
    }
}
