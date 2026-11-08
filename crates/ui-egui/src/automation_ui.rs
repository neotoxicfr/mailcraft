//! File › Rules and File › Automatic replies.

use chrono::{Local, NaiveDate, TimeZone};
use egui::{RichText, Ui};
use mailcraft_sync::{Away, Rule, RuleAction};

use crate::i18n::{Lang, T};
use crate::settings::Prefs;
use crate::shell::{Command, Ctx};

/// The rule being written in the dialog.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RuleDraft {
    pub account_id: i64,
    pub from: String,
    pub subject: String,
    /// 0 move, 1 mark read, 2 flag, 3 delete.
    pub action: usize,
    pub folder: Option<String>,
}

const ACTIONS: [T; 4] = [T::RuleMove, T::RuleRead, T::RuleFlag, T::RuleDelete];

fn describe(c: &Ctx<'_>, rule: &Rule) -> String {
    let lang = c.lang;
    let mut parts = Vec::new();
    if !rule.from_contains.trim().is_empty() {
        parts.push(format!(
            "{} « {} »",
            T::RuleFrom.get(lang),
            rule.from_contains.trim()
        ));
    }
    if !rule.subject_contains.trim().is_empty() {
        parts.push(format!(
            "{} « {} »",
            T::RuleSubject.get(lang),
            rule.subject_contains.trim()
        ));
    }
    let action = match &rule.action {
        RuleAction::MoveTo(f) => format!(
            "{} {}",
            T::RuleMove.get(lang),
            c.mail.folder_name(rule.account_id, f)
        ),
        RuleAction::MarkRead => T::RuleRead.get(lang).to_owned(),
        RuleAction::Flag => T::RuleFlag.get(lang).to_owned(),
        RuleAction::Delete => T::RuleDelete.get(lang).to_owned(),
    };
    format!("{} → {action}", parts.join(" + "))
}

fn field(ui: &mut Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        let l = ui.add_sized([120.0, 20.0], egui::Label::new(label));
        ui.add(egui::TextEdit::singleline(value).desired_width(220.0))
            .labelled_by(l.id);
    });
}

/// Rules list and the form for a new one. Changes `prefs.rules` directly. Returns true to close.
pub fn rules(ui: &mut Ui, c: &Ctx<'_>, prefs: &mut Prefs, draft: &mut RuleDraft) -> bool {
    let lang = c.lang;
    let mut close = false;
    ui.heading(T::Rules.get(lang));
    ui.label(RichText::new(T::RulesHelp.get(lang)).small());
    ui.add_space(4.0);
    let mut remove = None;
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            if prefs.rules.is_empty() {
                ui.label(RichText::new(T::RulesNone.get(lang)).weak());
            }
            for (i, rule) in prefs.rules.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let text = describe(c, rule);
                    ui.checkbox(&mut rule.enabled, text);
                    if ui.small_button(T::Delete.get(lang)).clicked() {
                        remove = Some(i);
                    }
                });
            }
        });
    if let Some(i) = remove {
        prefs.rules.remove(i);
    }
    ui.separator();
    ui.label(RichText::new(T::RuleNew.get(lang)).strong());
    if draft.account_id == 0 {
        draft.account_id = c.mail.accounts.first().map_or(0, |a| a.id);
    }
    if c.mail.accounts.len() > 1 {
        let shown = c
            .mail
            .accounts
            .iter()
            .find(|a| a.id == draft.account_id)
            .map_or("", |a| a.email.as_str())
            .to_owned();
        egui::ComboBox::from_label(T::RuleAccount.get(lang))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for a in &c.mail.accounts {
                    if ui
                        .selectable_value(&mut draft.account_id, a.id, &a.email)
                        .changed()
                    {
                        draft.folder = None;
                    }
                }
            });
    }
    field(ui, T::RuleFrom.get(lang), &mut draft.from);
    field(ui, T::RuleSubject.get(lang), &mut draft.subject);
    egui::ComboBox::from_label(T::RuleThen.get(lang))
        .selected_text(ACTIONS[draft.action].get(lang))
        .show_ui(ui, |ui| {
            for (i, a) in ACTIONS.iter().enumerate() {
                ui.selectable_value(&mut draft.action, i, a.get(lang));
            }
        });
    if draft.action == 0 {
        let shown = draft
            .folder
            .as_ref()
            .map(|f| c.mail.folder_name(draft.account_id, f))
            .unwrap_or_default();
        egui::ComboBox::from_label(T::RuleFolder.get(lang))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (f, depth) in c.mail.folder_tree(draft.account_id) {
                    let label = format!("{}{}", "    ".repeat(depth - 1), f.name);
                    ui.selectable_value(&mut draft.folder, Some(f.id.clone()), label);
                }
            });
    }
    let has_condition = !draft.from.trim().is_empty() || !draft.subject.trim().is_empty();
    let ready = has_condition && (draft.action != 0 || draft.folder.is_some());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(ready, egui::Button::new(T::AddButton.get(lang)))
            .clicked()
        {
            let action = match draft.action {
                0 => RuleAction::MoveTo(draft.folder.clone().unwrap_or_default()),
                1 => RuleAction::MarkRead,
                2 => RuleAction::Flag,
                _ => RuleAction::Delete,
            };
            prefs.rules.push(Rule {
                account_id: draft.account_id,
                from_contains: draft.from.trim().to_owned(),
                subject_contains: draft.subject.trim().to_owned(),
                action,
                enabled: true,
            });
            *draft = RuleDraft {
                account_id: draft.account_id,
                ..RuleDraft::default()
            };
        }
        if ui.button(T::Discard.get(lang)).clicked() {
            close = true;
        }
    });
    close
}

/// `31/12/2026` → end of that day, local time.
pub fn parse_day(text: &str) -> Option<i64> {
    let d = NaiveDate::parse_from_str(text.trim(), "%d/%m/%Y").ok()?;
    let end = d.and_hms_opt(23, 59, 59)?;
    Local
        .from_local_datetime(&end)
        .earliest()
        .map(|t| t.timestamp())
}

pub fn format_day(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%d/%m/%Y").to_string())
        .unwrap_or_default()
}

/// Out-of-office dialog. Returns the command to run and whether it closes.
pub fn away(
    ui: &mut Ui,
    lang: Lang,
    away: &mut Away,
    until: &mut String,
) -> (Option<Command>, bool) {
    let (mut command, mut close) = (None, false);
    ui.heading(T::AwayTitle.get(lang));
    ui.checkbox(&mut away.enabled, T::AwaySend.get(lang));
    let l = ui.label(T::AwayMessage.get(lang));
    ui.add(
        egui::TextEdit::multiline(&mut away.message)
            .desired_rows(5)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(l.id);
    field(ui, T::AwayUntil.get(lang), until);
    let date_ok = until.trim().is_empty() || parse_day(until).is_some();
    if !date_ok {
        ui.label(RichText::new(T::AwayBadDate.get(lang)).small());
    }
    ui.label(RichText::new(T::AwayHelp.get(lang)).small());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let ready = date_ok && (!away.enabled || !away.message.trim().is_empty());
        if ui
            .add_enabled(ready, egui::Button::new(T::Ok.get(lang)))
            .clicked()
        {
            let mut saved = away.clone();
            saved.until = parse_day(until);
            command = Some(Command::SaveAway {
                away: Box::new(saved),
            });
            close = true;
        }
        if ui.button(T::Cancel.get(lang)).clicked() {
            close = true;
        }
    });
    (command, close)
}

#[cfg(test)]
mod tests {
    use super::{format_day, parse_day};

    #[test]
    fn days_round_trip() {
        let t = parse_day(" 31/12/2026 ").unwrap();
        assert_eq!(format_day(t), "31/12/2026");
        assert!(parse_day("2026-12-31").is_none());
        assert!(parse_day("31/02/2026").is_none());
    }
}
