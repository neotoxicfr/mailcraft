//! MailCraft user interface.

mod attachments;
mod fonts;
pub mod i18n;
mod icons;
pub mod mail;
mod panes;
mod reader;
pub mod settings;
pub mod shell;
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
        app
    }

    /// Settings only, no mail backend (tests, or when the backend cannot start).
    pub fn with_location(ctx: &Context, location: Location) -> Self {
        fonts::install(ctx);
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
        self.handle_events(ui.ctx());
        if self.dirty {
            self.dirty = false;
            if let Some(b) = &self.backend
                && let Err(e) = self.mail.reload(&b.store, &self.view.search)
            {
                self.mail.status = Status::Error(e.to_string());
            }
            if let Some((account_id, id)) = self.mail.wants_inline() {
                self.send(Request::FetchInline { account_id, id });
            }
        }

        let c = shell::Ctx {
            lang: Lang::resolve(prefs.general.language),
            accent: theme::accent(prefs.appearance.accent),
            mail: &self.mail,
            save_error: self.save_error.as_deref(),
            block_images: prefs.privacy.block_remote_images,
        };
        let mut next = prefs.clone();
        let command = shell::show(ui, &c, &mut self.view, &mut next);
        if next != prefs {
            self.settings.update(|p| *p = next);
        }
        for url in std::mem::take(&mut self.view.image_requests) {
            if self.mail.images_requested.insert(url.clone()) {
                self.send(Request::FetchImage { url });
            }
        }
        let command = command.and_then(|cmd| self.run(cmd));

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
                    self.mail.status = Status::SigningIn;
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
                Event::Error(e) => {
                    self.mail.signing_in = false;
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
        let account_id = selected.first()?.account_id;
        let ids: Vec<String> = selected.iter().map(|m| m.id.clone()).collect();
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
        let Some((account_id, _)) = self.mail.folder.clone() else {
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
        // Where each message comes from (search results mix folders); those already in `dest` stay.
        let from: Vec<(String, String)> = self
            .mail
            .selected_messages()
            .iter()
            .filter(|m| m.folder_id != dest)
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
    let store = Store::open(&db_path, key.clone(), encrypt).map_err(|e| e.to_string())?;
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
