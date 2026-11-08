//! File › Options: every setting in one place (the welcome assistant shows the main ones).

use egui::{RichText, Ui};

use crate::i18n::T;
use crate::settings::Prefs;
use crate::shell::{Command, Ctx, Dialog, View};

/// Months offered for the first sync of a folder ("everything" is 120).
pub const MONTHS: [u32; 6] = [1, 3, 6, 12, 24, 120];

fn section(ui: &mut Ui, title: T, lang: crate::i18n::Lang) {
    ui.add_space(14.0);
    ui.label(RichText::new(title.get(lang)).heading());
    ui.separator();
}

pub fn show(ui: &mut Ui, c: &Ctx<'_>, prefs: &mut Prefs, view: &mut View) -> Option<Command> {
    let lang = c.lang;
    let mut command = None;
    ui.horizontal(|ui| {
        ui.heading(T::Options.get(lang));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(T::BackToMail.get(lang)).clicked() {
                command = Some(Command::CloseOptions);
            }
        });
    });
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.set_max_width(720.0);

            section(ui, T::GroupAppearance, lang);
            crate::shell::appearance_controls(ui, lang, prefs);

            section(ui, T::Accounts, lang);
            for a in &c.mail.accounts {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&a.email).strong());
                    ui.label(RichText::new(&a.name).weak());
                    if ui.small_button(T::RemoveAccount.get(lang)).clicked() {
                        view.dialog = Some(Dialog::ConfirmRemoveAccount {
                            account_id: a.id,
                            email: a.email.clone(),
                        });
                    }
                });
            }
            if ui
                .add_enabled(
                    !c.mail.signing_in,
                    egui::Button::new(T::AddAccount.get(lang).replace('\n', " ")),
                )
                .clicked()
            {
                command = Some(Command::AddAccount);
            }

            section(ui, T::SyncSection, lang);
            ui.horizontal(|ui| {
                let l = ui.label(T::SyncMonths.get(lang));
                let label = |m: u32| {
                    if m >= 120 {
                        T::Everything.get(lang).to_owned()
                    } else {
                        format!("{m} {}", T::MonthsWord.get(lang))
                    }
                };
                egui::ComboBox::from_id_salt("months")
                    .selected_text(label(prefs.sync.months))
                    .show_ui(ui, |ui| {
                        for m in MONTHS {
                            ui.selectable_value(&mut prefs.sync.months, m, label(m));
                        }
                    })
                    .response
                    .labelled_by(l.id);
            });
            ui.label(RichText::new(T::SyncMonthsHelp.get(lang)).small().weak());

            section(ui, T::PrivacySection, lang);
            ui.checkbox(
                &mut prefs.privacy.encrypt_local_copy,
                T::EncryptLocalCopy.get(lang),
            );
            if let Some(now) = c.mail.encrypted
                && now != prefs.privacy.encrypt_local_copy
            {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(T::EncryptionChangeHelp.get(lang)).small());
                    if ui.button(T::RebuildNow.get(lang)).clicked() {
                        command = Some(Command::RebuildLocalCopy);
                    }
                });
            }
            ui.checkbox(
                &mut prefs.privacy.block_trackers,
                T::BlockTrackers.get(lang),
            )
            .on_hover_text(T::BlockTrackersHelp.get(lang));
            ui.checkbox(
                &mut prefs.privacy.block_remote_images,
                T::BlockRemoteImages.get(lang),
            )
            .on_hover_text(T::BlockRemoteImagesHelp.get(lang));

            section(ui, T::CleanupTitle, lang);
            ui.checkbox(&mut prefs.cleanup.dry_run, T::DryRun.get(lang))
                .on_hover_text(T::DryRunHelp.get(lang));
            let was = prefs.cleanup.screener;
            ui.checkbox(&mut prefs.cleanup.screener, T::Screener.get(lang))
                .on_hover_text(T::ScreenerHelp.get(lang));
            if prefs.cleanup.screener && !was {
                prefs.cleanup.screener_since = chrono::Utc::now().timestamp();
            }
            if ui.button(T::AllowList.get(lang)).clicked() {
                view.dialog = Some(Dialog::AllowList {
                    text: prefs.cleanup.allow_list.join("\n"),
                });
            }

            section(ui, T::ClaudeSection, lang);
            ui.label(RichText::new(T::ClaudeHelp.get(lang)).small());
            let before = prefs.ai.provider;
            ui.horizontal(|ui| {
                let l = ui.label(T::AiProvider.get(lang));
                egui::ComboBox::from_id_salt("ai_provider")
                    .selected_text(prefs.ai.provider.label())
                    .show_ui(ui, |ui| {
                        for p in mailcraft_ai::Provider::ALL {
                            ui.selectable_value(&mut prefs.ai.provider, p, p.label());
                        }
                    })
                    .response
                    .labelled_by(l.id);
            });
            let provider = prefs.ai.provider;
            if provider != before {
                prefs.ai.model.clear();
                prefs.ai.url.clear();
                view.claude_key.clear();
                command = Some(Command::AiProviderChanged);
            }
            ui.horizontal(|ui| {
                let l = ui.label(T::AiModel.get(lang));
                ui.add(
                    egui::TextEdit::singleline(&mut prefs.ai.model)
                        .hint_text(provider.default_model())
                        .desired_width(220.0),
                )
                .labelled_by(l.id);
            });
            if matches!(
                provider,
                mailcraft_ai::Provider::Ollama | mailcraft_ai::Provider::Custom
            ) {
                ui.horizontal(|ui| {
                    let l = ui.label(T::AiServer.get(lang));
                    ui.add(
                        egui::TextEdit::singleline(&mut prefs.ai.url)
                            .hint_text(provider.default_url())
                            .desired_width(320.0),
                    )
                    .labelled_by(l.id);
                });
            }
            if provider == mailcraft_ai::Provider::Ollama {
                ui.label(RichText::new(T::AiNoKeyNeeded.get(lang)).small());
            } else {
                match view.claude_key_saved {
                    Some(true) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(T::ClaudeKeySaved.get(lang)).strong());
                            if ui.button(T::ClaudeKeyDelete.get(lang)).clicked() {
                                command = Some(Command::DeleteClaudeKey);
                            }
                        });
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            let l = ui.label(T::ClaudeKey.get(lang));
                            ui.add(
                                egui::TextEdit::singleline(&mut view.claude_key)
                                    .password(true)
                                    .hint_text(provider.key_hint())
                                    .desired_width(280.0),
                            )
                            .labelled_by(l.id);
                            let ok = provider.key_looks_valid(&view.claude_key);
                            if ui
                                .add_enabled(ok, egui::Button::new(T::ClaudeKeySave.get(lang)))
                                .clicked()
                            {
                                command = Some(Command::SaveClaudeKey {
                                    key: std::mem::take(&mut view.claude_key),
                                });
                            }
                        });
                    }
                }
            }

            section(ui, T::StartupSection, lang);
            ui.checkbox(
                &mut prefs.startup.with_windows,
                T::StartWithWindows.get(lang),
            );
            ui.add_enabled(
                prefs.startup.with_windows,
                egui::Checkbox::new(&mut prefs.startup.minimized, T::StartMinimized.get(lang)),
            );
            ui.checkbox(&mut prefs.startup.close_to_tray, T::CloseToTray.get(lang));
            ui.add_space(20.0);
        });
    command
}
