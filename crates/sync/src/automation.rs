//! What MailCraft does by itself with new mail: the user's rules, and the out-of-office reply
//! for accounts whose server cannot send it (IMAP). Microsoft accounts get the reply on the
//! server instead, so it works while the computer is off.

use serde::{Deserialize, Serialize};

use crate::{Compose, Event, Request, Runner, unix_now};

/// What a rule does to a matching message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum RuleAction {
    /// To a folder of the same account (its id).
    MoveTo(String),
    #[default]
    MarkRead,
    Flag,
    /// To Deleted Items.
    Delete,
}

/// "When a new message arrives in the inbox and … then …". Empty conditions are ignored; a
/// rule with no condition at all never matches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Rule {
    pub account_id: i64,
    /// Part of the sender's name or address.
    pub from_contains: String,
    pub subject_contains: String,
    pub action: RuleAction,
    pub enabled: bool,
}

impl Rule {
    pub fn matches(&self, from_name: &str, from_addr: &str, subject: &str) -> bool {
        let has =
            |hay: &str, needle: &str| hay.to_lowercase().contains(&needle.trim().to_lowercase());
        let from = self.from_contains.trim();
        let subject_part = self.subject_contains.trim();
        self.enabled
            && (!from.is_empty() || !subject_part.is_empty())
            && (from.is_empty() || has(from_name, from) || has(from_addr, from))
            && (subject_part.is_empty() || has(subject, subject_part))
    }
}

/// Out-of-office reply.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Away {
    pub enabled: bool,
    pub message: String,
    /// Last day (unix seconds, end of day) it is sent; `None`: until turned off.
    pub until: Option<i64>,
}

impl Away {
    pub fn active(&self, now: i64) -> bool {
        self.enabled && !self.message.trim().is_empty() && self.until.is_none_or(|u| now <= u)
    }
}

/// Senders that must never get an automatic reply (robots, lists, the user).
pub fn never_reply_to(address: &str, own: &[String]) -> bool {
    let a = address.to_lowercase();
    a.is_empty()
        || own.iter().any(|o| o.eq_ignore_ascii_case(&a))
        || [
            "noreply",
            "no-reply",
            "no_reply",
            "donotreply",
            "do-not-reply",
            "mailer-daemon",
            "postmaster",
            "notification",
            "newsletter",
            "bounce",
        ]
        .iter()
        .any(|p| a.contains(p))
}

/// One reply per sender per this many seconds (4 days, like Exchange).
const REPLY_EVERY: i64 = 4 * 24 * 3600;
/// Newest inbox message already looked at, per account (store mark).
pub(crate) const MARK: &str = "automation";

impl Runner {
    /// Rules and the local out-of-office reply, on the inbox messages that arrived since the
    /// last run. The very first run only records where things stand: old mail is left alone.
    pub(crate) fn run_automation(
        &mut self,
        account_id: i64,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        let rules: Vec<Rule> = self
            .rules
            .iter()
            .filter(|r| r.account_id == account_id && r.enabled)
            .cloned()
            .collect();
        let imap = self.is_imap(account_id);
        let away = self.away.clone().filter(|a| imap && a.active(unix_now()));
        // Nothing to do: no store reads (this runs every 20 s). New rules reset the mark.
        if rules.is_empty() && away.is_none() {
            return Ok(());
        }
        let Some(inbox) = self.inbox_of(account_id)? else {
            return Ok(());
        };
        let recent = self
            .store
            .messages(account_id, &inbox.id, "", 200)
            .map_err(|e| e.to_string())?;
        let newest = recent.iter().map(|m| m.received).max().unwrap_or(0);
        let Some(seen) = self
            .store
            .mark(account_id, MARK)
            .map_err(|e| e.to_string())?
        else {
            return self
                .store
                .set_mark(account_id, MARK, newest)
                .map_err(|e| e.to_string());
        };
        if rules.is_empty() && away.is_none() {
            return self
                .store
                .set_mark(account_id, MARK, newest.max(seen))
                .map_err(|e| e.to_string());
        }
        let own: Vec<String> = self
            .store
            .accounts()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| a.email)
            .collect();
        let mut new: Vec<_> = recent.into_iter().filter(|m| m.received > seen).collect();
        new.sort_by_key(|m| m.received);
        for m in &new {
            if let Some(rule) = rules
                .iter()
                .find(|r| r.matches(&m.from_name, &m.from_addr, &m.subject))
            {
                let ids = vec![m.id.clone()];
                let request = match &rule.action {
                    RuleAction::MoveTo(dest) => Request::Move {
                        account_id,
                        ids,
                        dest: dest.clone(),
                    },
                    RuleAction::MarkRead => Request::SetRead {
                        account_id,
                        ids,
                        read: true,
                    },
                    RuleAction::Flag => Request::SetFlag {
                        account_id,
                        ids,
                        flagged: true,
                    },
                    RuleAction::Delete => match self.role_id(account_id, "deleteditems") {
                        Some(dest) => Request::Move {
                            account_id,
                            ids,
                            dest,
                        },
                        None => continue,
                    },
                };
                self.handle(request, emit)?;
            }
            if let Some(away) = &away
                && !never_reply_to(&m.from_addr, &own)
            {
                let last = self
                    .store
                    .auto_replied(account_id, &m.from_addr)
                    .map_err(|e| e.to_string())?;
                if last.is_none_or(|t| unix_now() - t > REPLY_EVERY) {
                    self.auto_reply(account_id, m, &away.message)?;
                }
            }
        }
        self.store
            .set_mark(account_id, MARK, newest.max(seen))
            .map_err(|e| e.to_string())
    }

    fn role_id(&self, account_id: i64, role: &str) -> Option<String> {
        self.roles
            .get(&account_id)?
            .iter()
            .find(|(_, r)| *r == role)
            .map(|(id, _)| id.clone())
    }

    fn auto_reply(
        &mut self,
        account_id: i64,
        m: &mailcraft_store::MessageSummary,
        message: &str,
    ) -> Result<(), String> {
        let html = format!(
            "<p>{}</p>",
            message
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\n', "<br>")
        );
        let out = mailcraft_graph::compose::Outgoing {
            subject: format!("Réponse automatique : {}", m.subject),
            html,
            to: vec![m.from_addr.clone()],
            cc: Vec::new(),
            bcc: Vec::new(),
        };
        match self.imap_send(account_id, &Compose::Reply(m.id.clone()), None, &out, &[]) {
            Ok(()) => self
                .store
                .set_auto_replied(account_id, &m.from_addr, unix_now())
                .map_err(|e| e.to_string()),
            // A failed reply is not worth stopping the sync: it is tried again next time.
            Err(e) => {
                log::warn!("automatic reply not sent: {e}");
                Ok(())
            }
        }
    }

    /// Sets (or clears) the out-of-office reply on every Microsoft account's server.
    pub(crate) fn set_away(&mut self, away: Option<Away>) -> Result<(), String> {
        self.away = away.clone();
        let accounts = self.store.accounts().map_err(|e| e.to_string())?;
        for a in accounts
            .iter()
            .filter(|a| a.provider == crate::PROVIDER_MICROSOFT)
        {
            let setting = away.clone().unwrap_or_default();
            self.with_graph(a.id, |g| {
                g.set_auto_reply(setting.active(unix_now()), &setting.message, setting.until)
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Away, Rule, RuleAction, never_reply_to};

    #[test]
    fn rules_match_on_sender_and_subject() {
        let rule = Rule {
            account_id: 1,
            from_contains: "Shop".into(),
            subject_contains: "facture".into(),
            action: RuleAction::MarkRead,
            enabled: true,
        };
        assert!(rule.matches("Example SHOP", "news@x.example", "Votre FACTURE"));
        assert!(!rule.matches("Example Shop", "news@x.example", "Promo"));
        let empty = Rule {
            enabled: true,
            ..Rule::default()
        };
        assert!(!empty.matches("a", "b", "c"));
        let off = Rule {
            enabled: false,
            ..rule
        };
        assert!(!off.matches("Example Shop", "", "facture"));
    }

    #[test]
    fn away_dates_and_robots() {
        let away = Away {
            enabled: true,
            message: "Absent".into(),
            until: Some(100),
        };
        assert!(away.active(100) && !away.active(101));
        assert!(!Away::default().active(0));
        let own = vec!["me@example.com".to_owned()];
        assert!(never_reply_to("No-Reply@shop.example", &own));
        assert!(never_reply_to("ME@example.com", &own));
        assert!(!never_reply_to("alex@example.org", &own));
    }
}
