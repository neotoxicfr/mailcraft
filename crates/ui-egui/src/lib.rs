//! MailCraft user interface.

mod fonts;
pub mod i18n;
mod icons;
pub mod mail;
mod panes;
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
        self.handle_events();
        if self.dirty {
            self.dirty = false;
            if let Some(b) = &self.backend
                && let Err(e) = self.mail.reload(&b.store, &self.view.search)
            {
                self.mail.status = Status::Error(e.to_string());
            }
        }

        let c = shell::Ctx {
            lang: Lang::resolve(prefs.general.language),
            accent: theme::accent(prefs.appearance.accent),
            mail: &self.mail,
            save_error: self.save_error.as_deref(),
        };
        let mut next = prefs.clone();
        let command = shell::show(ui, &c, &mut self.view, &mut next);
        if next != prefs {
            self.settings.update(|p| *p = next);
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

    fn handle_events(&mut self) {
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
                Event::Error(e) => {
                    self.mail.signing_in = false;
                    self.mail.status = Status::Error(e);
                }
            }
        }
    }

    /// Handles what the window can; returns what the host must do (exit).
    fn run(&mut self, command: Command) -> Option<Command> {
        let send = |app: &Self, r: Request| {
            if let Some(w) = app.worker() {
                w.send(r);
            }
        };
        match command {
            Command::Exit => return Some(Command::Exit),
            Command::AddAccount => {
                self.mail.signing_in = true;
                self.mail.status = Status::SigningIn;
                send(self, Request::AddMicrosoftAccount);
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
                send(self, Request::SyncAll);
            }
            Command::SelectFolder {
                account_id,
                folder_id,
            } => {
                self.mail.folder = Some((account_id, folder_id));
                self.mail.message = None;
                self.dirty = true;
            }
            Command::SelectMessage { account_id, id } => {
                let unread = self
                    .mail
                    .messages
                    .iter()
                    .any(|m| m.account_id == account_id && m.id == id && !m.is_read);
                self.mail.message = Some((account_id, id.clone()));
                self.dirty = true;
                send(
                    self,
                    Request::FetchBody {
                        account_id,
                        id: id.clone(),
                    },
                );
                if unread {
                    send(self, Request::MarkRead { account_id, id });
                }
            }
            Command::SearchChanged => self.dirty = true,
        }
        None
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
        Box::new(move || repaint.request_repaint()),
    );
    Ok(Backend {
        store,
        worker: Some(worker),
    })
}
