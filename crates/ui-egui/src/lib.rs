//! MailCraft user interface.

mod fonts;
pub mod i18n;
mod icons;
pub mod settings;
pub mod shell;
pub mod theme;

use std::time::Instant;

use craft_settings::{Location, SaveStatus, SettingsStore};
use egui::{Context, Ui};

use crate::i18n::Lang;
use crate::settings::{Appearance, Prefs};
pub use crate::shell::Command;

pub struct MailCraftApp {
    store: SettingsStore<Prefs>,
    applied: Option<Appearance>,
    view: shell::View,
    save_error: Option<String>,
}

impl MailCraftApp {
    /// Settings in the usual per-user folder.
    pub fn new(ctx: &Context) -> Self {
        Self::with_location(ctx, Location::Auto)
    }

    pub fn with_location(ctx: &Context, location: Location) -> Self {
        fonts::install(ctx);
        let (store, report) = SettingsStore::open(location);
        if report.has_problems() {
            log::warn!("settings: {report:?}");
        }
        Self {
            store,
            applied: None,
            view: shell::View::default(),
            save_error: None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Option<Command> {
        let prefs = self.store.get().clone();
        if self.applied.as_ref() != Some(&prefs.appearance) {
            theme::apply(ui.ctx(), &prefs.appearance);
            self.applied = Some(prefs.appearance.clone());
        }
        let c = shell::Ctx {
            lang: Lang::resolve(prefs.general.language),
            accent: theme::accent(prefs.appearance.accent),
            status_error: self.save_error.as_deref(),
        };
        let mut next = prefs.clone();
        let command = shell::show(ui, &c, &mut self.view, &mut next);
        if next != prefs {
            self.store.update(|p| *p = next);
        }

        let tick = self.store.tick(Instant::now());
        match &tick.status {
            SaveStatus::Failed(err) => self.save_error = Some(err.to_string()),
            SaveStatus::Saved => self.save_error = None,
            _ => {}
        }
        if let Some(deadline) = tick.next_deadline {
            ui.ctx()
                .request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
        command
    }

    pub fn prefs(&self) -> &Prefs {
        self.store.get()
    }

    pub fn update_prefs(&mut self, f: impl FnOnce(&mut Prefs)) {
        self.store.update(f);
    }

    /// Writes pending settings; call on exit.
    pub fn flush(&mut self) {
        if let Err(err) = self.store.flush() {
            log::warn!("settings not saved on exit: {err}");
        }
    }
}
