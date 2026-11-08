//! The clean-up view ("Tri"): one row per sender with its numbers, authentication results, how
//! to unsubscribe and what is suspicious about it; grouped actions on the ticked senders.
//! Simulation is on by default: actions are logged, nothing is changed.

use std::collections::HashSet;

use egui::{Color32, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use mailcraft_store::SenderRow;
use mailcraft_sync::CleanAction;

use crate::i18n::{Lang, T};
use crate::shell::{Command, Ctx, Dialog};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    Newsletters,
    Suspects,
    Others,
    NotAnalyzed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    Count,
    Unread,
    Last,
    Name,
}

#[derive(Default)]
pub struct State {
    pub filter: Filter,
    pub sort: Sort,
    pub search: String,
    pub checked: HashSet<String>,
    pub show_log: bool,
}

const RED: Color32 = Color32::from_rgb(0xD1, 0x34, 0x38);
const GREEN: Color32 = Color32::from_rgb(0x10, 0x7C, 0x10);

/// "suspect" when the local analysis or Claude say so.
pub fn is_suspect(s: &SenderRow) -> bool {
    s.analysis
        .as_ref()
        .is_some_and(|a| a.category == "suspect" || !a.warnings.is_empty())
        || s.ai
            .as_ref()
            .is_some_and(|ai| matches!(ai.category.as_str(), "suspect" | "phishing"))
}

fn matches(s: &SenderRow, f: Filter) -> bool {
    match f {
        Filter::All => true,
        Filter::Suspects => is_suspect(s),
        Filter::Newsletters => {
            !is_suspect(s)
                && s.analysis
                    .as_ref()
                    .is_some_and(|a| a.category == "newsletter")
        }
        Filter::Others => {
            !is_suspect(s) && s.analysis.as_ref().is_some_and(|a| a.category == "other")
        }
        Filter::NotAnalyzed => s.analysis.is_none(),
    }
}

/// Plain words for the warning codes stored by the analysis.
pub fn warning_text(code: &str, lang: Lang) -> String {
    let (kind, brand) = code.split_once(':').unwrap_or((code, ""));
    let t = match kind {
        "impersonation" => T::WarnImpersonation,
        "lookalike" => T::WarnLookalike,
        "punycode" => T::WarnPunycode,
        "dmarc-fail" => T::WarnDmarc,
        "auth-fail" => T::WarnAuth,
        "own-address" => T::WarnOwnAddress,
        "invalid-domain" => T::WarnInvalidDomain,
        "random-domain" => T::WarnRandomDomain,
        "display-domain" => T::WarnDisplayDomain,
        _ => return code.to_owned(),
    };
    if brand.is_empty() {
        t.get(lang).to_owned()
    } else {
        format!("{} ({brand})", t.get(lang))
    }
}

pub fn show(
    ui: &mut Ui,
    c: &Ctx<'_>,
    state: &mut State,
    dry_run: &mut bool,
    allow_list: &[String],
    dialog: &mut Option<Dialog>,
) -> Option<Command> {
    let lang = c.lang;
    let mut command = None;
    let mail = c.mail;

    ui.horizontal(|ui| {
        ui.heading(T::CleanupTitle.get(lang));
        ui.separator();
        ui.checkbox(dry_run, T::DryRun.get(lang))
            .on_hover_text(T::DryRunHelp.get(lang));
        ui.separator();
        if ui.button(T::AllowList.get(lang)).clicked() {
            *dialog = Some(Dialog::AllowList {
                text: allow_list.join("\n"),
            });
        }
        if ui.button(T::Reanalyze.get(lang)).clicked() {
            command = Some(Command::Analyze { force: true });
        }
        ui.toggle_value(&mut state.show_log, T::ActionsLog.get(lang));
        if ui.button(T::BackToMail.get(lang)).clicked() {
            command = Some(Command::CloseCleanup);
        }
    });
    if let Some((done, total)) = mail.analyzing {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("{} {done}/{total}", T::Analyzing.get(lang)));
        });
    }
    if *dry_run {
        ui.colored_label(c.accent, T::DryRunBanner.get(lang));
    }

    ui.horizontal(|ui| {
        let label = |f: Filter| match f {
            Filter::All => T::FilterAll,
            Filter::Newsletters => T::FilterNewsletters,
            Filter::Suspects => T::FilterSuspects,
            Filter::Others => T::FilterOthers,
            Filter::NotAnalyzed => T::FilterNotAnalyzed,
        };
        for f in [
            Filter::All,
            Filter::Newsletters,
            Filter::Suspects,
            Filter::Others,
            Filter::NotAnalyzed,
        ] {
            let n = mail.senders.iter().filter(|s| matches(s, f)).count();
            if ui
                .selectable_label(state.filter == f, format!("{} ({n})", label(f).get(lang)))
                .clicked()
            {
                state.filter = f;
            }
        }
        ui.separator();
        let l = ui.label(T::SearchSender.get(lang));
        ui.add(egui::TextEdit::singleline(&mut state.search).desired_width(180.0))
            .labelled_by(l.id);
    });

    let q = state.search.trim().to_lowercase();
    let mut rows: Vec<&SenderRow> = mail
        .senders
        .iter()
        .filter(|s| matches(s, state.filter))
        .filter(|s| {
            q.is_empty()
                || s.address.to_lowercase().contains(&q)
                || s.name.to_lowercase().contains(&q)
        })
        .collect();
    match state.sort {
        Sort::Count => rows.sort_by_key(|s| std::cmp::Reverse(s.count)),
        Sort::Unread => rows.sort_by_key(|s| std::cmp::Reverse(s.unread)),
        Sort::Last => rows.sort_by_key(|s| std::cmp::Reverse(s.last)),
        Sort::Name => rows.sort_by_key(|s| s.name.to_lowercase()),
    }

    // Grouped actions on the ticked senders.
    let ticked: Vec<String> = rows
        .iter()
        .filter(|s| state.checked.contains(&s.address))
        .map(|s| s.address.clone())
        .collect();
    ui.horizontal(|ui| {
        let any = !ticked.is_empty();
        ui.label(format!("{} {}", ticked.len(), T::SendersTicked.get(lang)));
        for (action, label) in [
            (CleanAction::Unsubscribe, T::Unsubscribe),
            (CleanAction::DeleteAll, T::DeleteAll),
            (CleanAction::ArchiveAll, T::ArchiveAll),
            (CleanAction::Block, T::Block),
        ] {
            if ui
                .add_enabled(any, egui::Button::new(label.get(lang).replace('\n', " ")))
                .clicked()
            {
                command = Some(if *dry_run {
                    Command::Cleanup {
                        action,
                        addresses: ticked.clone(),
                    }
                } else {
                    *dialog = Some(Dialog::ConfirmCleanup {
                        action,
                        addresses: ticked.clone(),
                    });
                    return;
                });
            }
        }
        if ui
            .add_enabled(any, egui::Button::new(T::AddToAllowList.get(lang)))
            .clicked()
        {
            command = Some(Command::AllowSenders {
                addresses: ticked.clone(),
            });
            state.checked.clear();
        }
    });
    ui.separator();

    if state.show_log {
        egui::ScrollArea::vertical()
            .id_salt("log")
            .max_height(160.0)
            .show(ui, |ui| {
                for e in &mail.actions {
                    let when = chrono::DateTime::from_timestamp(e.at, 0)
                        .map(|d| {
                            d.with_timezone(&chrono::Local)
                                .format("%d/%m %H:%M")
                                .to_string()
                        })
                        .unwrap_or_default();
                    let tag = if e.dry_run {
                        T::Simulated.get(lang)
                    } else {
                        ""
                    };
                    let color = if e.ok { ui.visuals().text_color() } else { RED };
                    ui.label(
                        RichText::new(format!(
                            "{when}  {tag}{}  {}  {}",
                            e.action, e.address, e.detail
                        ))
                        .small()
                        .color(color),
                    );
                }
            });
        ui.separator();
    }

    let header = |ui: &mut Ui, text: T, sort: Option<Sort>, state: &mut State| match sort {
        Some(s) => {
            if ui
                .selectable_label(state.sort == s, RichText::new(text.get(lang)).strong())
                .clicked()
            {
                state.sort = s;
            }
        }
        None => {
            ui.label(RichText::new(text.get(lang)).strong());
        }
    };
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(24.0))
        .column(Column::initial(260.0).at_least(140.0).clip(true))
        .column(Column::exact(60.0))
        .column(Column::exact(72.0))
        .column(Column::exact(90.0))
        .column(Column::exact(130.0))
        .column(Column::initial(120.0))
        .column(Column::remainder().at_least(160.0).clip(true))
        .header(22.0, |mut h| {
            h.col(|ui| {
                let all =
                    !rows.is_empty() && rows.iter().all(|s| state.checked.contains(&s.address));
                let mut on = all;
                if ui
                    .checkbox(&mut on, "")
                    .on_hover_text(T::TickAll.get(lang))
                    .changed()
                {
                    for s in &rows {
                        if on {
                            state.checked.insert(s.address.clone());
                        } else {
                            state.checked.remove(&s.address);
                        }
                    }
                }
            });
            h.col(|ui| header(ui, T::ColSender, Some(Sort::Name), state));
            h.col(|ui| header(ui, T::ColMails, Some(Sort::Count), state));
            h.col(|ui| header(ui, T::ColUnread, Some(Sort::Unread), state));
            h.col(|ui| header(ui, T::ColLast, Some(Sort::Last), state));
            h.col(|ui| header(ui, T::ColAuth, None, state));
            h.col(|ui| header(ui, T::ColUnsubscribe, None, state));
            h.col(|ui| header(ui, T::ColVerdict, None, state));
        })
        .body(|body| {
            body.rows(34.0, rows.len(), |mut row| {
                let s = rows[row.index()];
                let protected = mailcraft_sync::is_protected(&s.address, allow_list);
                row.col(|ui| {
                    let mut on = state.checked.contains(&s.address);
                    if ui.checkbox(&mut on, "").on_hover_text(&s.address).changed() {
                        if on {
                            state.checked.insert(s.address.clone());
                        } else {
                            state.checked.remove(&s.address);
                        }
                    }
                });
                row.col(|ui| {
                    ui.vertical(|ui| {
                        let name = if s.name.is_empty() {
                            &s.address
                        } else {
                            &s.name
                        };
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(&s.address).small().weak());
                    });
                });
                row.col(|ui| {
                    ui.label(s.count.to_string());
                });
                row.col(|ui| {
                    let pct = if s.count > 0 {
                        s.unread * 100 / s.count
                    } else {
                        0
                    };
                    ui.label(format!("{pct} %"));
                });
                row.col(|ui| {
                    let d = chrono::DateTime::from_timestamp(s.last, 0)
                        .map(|d| {
                            d.with_timezone(&chrono::Local)
                                .format("%d/%m/%Y")
                                .to_string()
                        })
                        .unwrap_or_default();
                    ui.label(d);
                });
                row.col(|ui| match &s.analysis {
                    Some(a) => {
                        for (name, v) in [("SPF", a.spf), ("DKIM", a.dkim), ("DMARC", a.dmarc)] {
                            let (color, tip) = match v {
                                1 => (GREEN, T::CheckPass),
                                2 => (RED, T::CheckFail),
                                _ => (ui.visuals().weak_text_color(), T::CheckUnknown),
                            };
                            ui.label(RichText::new(name).small().color(color))
                                .on_hover_text(tip.get(lang));
                        }
                    }
                    None => {
                        ui.label(RichText::new("—").weak());
                    }
                });
                row.col(|ui| {
                    let how = s.analysis.as_ref().map(|a| {
                        if a.one_click.is_some() {
                            T::OneClick
                        } else if a.mailto.is_some() {
                            T::ByEmail
                        } else if a.web.is_some() {
                            T::ByWebPage
                        } else {
                            T::NoUnsubscribe
                        }
                    });
                    match (s.unsubscribed_at, how) {
                        (Some(_), _) if s.still_writing => {
                            ui.label(RichText::new(T::StillWriting.get(lang)).color(RED))
                                .on_hover_text(T::StillWritingHelp.get(lang));
                        }
                        (Some(_), _) => {
                            ui.label(RichText::new(T::Unsubscribed.get(lang)).color(GREEN));
                        }
                        (None, Some(t)) => {
                            ui.label(t.get(lang));
                        }
                        (None, None) => {
                            ui.label(RichText::new("—").weak());
                        }
                    }
                });
                row.col(|ui| {
                    if protected {
                        ui.label(RichText::new(T::Protected.get(lang)).color(GREEN));
                    } else if s.blocked {
                        ui.label(RichText::new(T::Blocked.get(lang)).weak());
                    } else if is_suspect(s) {
                        let reasons: Vec<String> = s
                            .analysis
                            .iter()
                            .flat_map(|a| a.warnings.split(','))
                            .filter(|w| !w.is_empty())
                            .map(|w| warning_text(w, lang))
                            .chain(s.ai.iter().map(|ai| ai.reason.clone()))
                            .collect();
                        let badge = egui::Button::new(
                            RichText::new(T::SuspectBadge.get(lang))
                                .color(Color32::WHITE)
                                .small(),
                        )
                        .fill(RED)
                        .sense(egui::Sense::hover());
                        ui.add(badge).on_hover_text(reasons.join("\n"));
                    } else if let Some(ai) = &s.ai {
                        ui.label(format!("{} ({:.0} %)", ai.category, ai.confidence * 100.0))
                            .on_hover_text(&ai.reason);
                    } else if let Some(a) = &s.analysis {
                        let t = if a.category == "newsletter" {
                            T::FilterNewsletters
                        } else {
                            T::FilterOthers
                        };
                        ui.label(RichText::new(t.get(lang)).weak());
                    }
                });
            });
        });
    command
}

#[cfg(test)]
mod tests {
    use super::warning_text;
    use crate::i18n::Lang;

    #[test]
    fn warnings_in_words() {
        assert_eq!(
            warning_text("impersonation:paypal", Lang::Fr),
            "Se fait passer pour une marque (paypal)"
        );
        assert_eq!(
            warning_text("dmarc-fail", Lang::En),
            "DMARC failed: the domain did not authorise this mail"
        );
        assert_eq!(warning_text("whatever", Lang::Fr), "whatever");
    }
}
