//! The same requests as Microsoft accounts, for IMAP + SMTP accounts.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use mailcraft_graph::compose::Outgoing;
use mailcraft_imap::{Error as ImapError, Imap, Server, mime, smtp};
use mailcraft_store::{Attachment, Folder, MessageBody, MessageSummary};

use crate::{Compose, Event, MAX_IMAGE, PendingOp, Runner, unix_now};

/// The largest number of messages a server search adds to the local copy.
const SEARCH_LIMIT: usize = 100;

fn summary(account_id: i64, folder_id: &str, s: mailcraft_imap::Summary) -> MessageSummary {
    MessageSummary {
        account_id,
        id: s.id,
        folder_id: folder_id.to_owned(),
        from_name: s.from_name,
        from_addr: s.from_addr,
        subject: s.subject,
        preview: s.preview,
        received: s.received,
        is_read: s.is_read,
        has_attachments: s.has_attachments,
        flagged: s.flagged,
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl Runner {
    pub(crate) fn is_imap(&self, account_id: i64) -> bool {
        if let Some(known) = self.imap_accounts.borrow().get(&account_id) {
            return *known;
        }
        let imap = self.store.accounts().is_ok_and(|list| {
            list.iter()
                .any(|a| a.id == account_id && a.provider == mailcraft_imap::PROVIDER_IMAP)
        });
        self.imap_accounts.borrow_mut().insert(account_id, imap);
        imap
    }

    /// The account's inbox, if it has one.
    pub(crate) fn inbox_of(&self, account_id: i64) -> Result<Option<Folder>, String> {
        Ok(self
            .store
            .folders(account_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|f| f.role.as_deref() == Some("inbox")))
    }

    /// The account's server entry, ready to connect: for a Google sign-in, `password` holds a
    /// fresh access token.
    fn server(&mut self, account_id: i64) -> Result<Server, ImapError> {
        let mut server = self.stored_server(account_id)?;
        if server.oauth {
            server.password = self
                .google_access(account_id, &server)
                .map_err(ImapError::Auth)?;
        }
        Ok(server)
    }

    fn stored_server(&mut self, account_id: i64) -> Result<Server, ImapError> {
        if let Some(s) = self.servers.get(&account_id) {
            return Ok(s.clone());
        }
        let text = self
            .store
            .refresh_token(account_id)
            .map_err(|e| ImapError::Refused(e.to_string()))?
            .ok_or_else(|| ImapError::Auth("no saved password: add the account again".into()))?;
        let server: Server =
            serde_json::from_str(&text).map_err(|e| ImapError::Refused(e.to_string()))?;
        self.servers.insert(account_id, server.clone());
        Ok(server)
    }

    /// Runs `f` on the account's open connection; reconnects once if it was dropped.
    pub(crate) fn try_imap<T>(
        &mut self,
        account_id: i64,
        mut f: impl FnMut(&mut Imap) -> Result<T, ImapError>,
    ) -> Result<T, ImapError> {
        for attempt in 0..2 {
            if !self.imap.contains_key(&account_id) {
                let server = self.server(account_id)?;
                self.imap.insert(account_id, Imap::connect(&server)?);
            }
            let session = self.imap.get_mut(&account_id).expect("just inserted");
            match f(session) {
                Err(ImapError::Network(e)) => {
                    self.imap.remove(&account_id);
                    if attempt == 1 {
                        return Err(ImapError::Network(e));
                    }
                }
                r => return r,
            }
        }
        unreachable!()
    }

    pub(crate) fn with_imap<T>(
        &mut self,
        account_id: i64,
        f: impl FnMut(&mut Imap) -> Result<T, ImapError>,
    ) -> Result<T, String> {
        self.try_imap(account_id, f).map_err(|e| e.to_string())
    }

    pub(crate) fn add_imap_account(
        &mut self,
        server: &Server,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        emit(Event::SigningIn);
        let checked = Imap::connect(server).and_then(|s| {
            s.logout();
            smtp::check(server)
        });
        if let Err(e) = checked {
            emit(Event::SignInCancelled);
            return Err(e.to_string());
        }
        let secret = serde_json::to_string(server).map_err(|e| e.to_string())?;
        // (App-password accounts: the password itself is what is sealed.)
        let name = if server.name.trim().is_empty() {
            server.email.clone()
        } else {
            server.name.clone()
        };
        let id = self
            .store
            .upsert_account(mailcraft_imap::PROVIDER_IMAP, &server.email, &name, &secret)
            .map_err(|e| e.to_string())?;
        self.servers.insert(id, server.clone());
        self.imap.remove(&id);
        emit(Event::AccountAdded {
            email: server.email.clone(),
        });
        self.imap_sync_account(id, emit)?;
        emit(Event::SyncDone);
        Ok(())
    }

    pub(crate) fn imap_refresh_folders(
        &mut self,
        account_id: i64,
        emit: &dyn Fn(Event),
    ) -> Result<Vec<Folder>, String> {
        let remote = self.with_imap(account_id, Imap::folders)?;
        let mut roles = HashMap::new();
        let mut views = HashSet::new();
        for f in &remote {
            match &f.state {
                Some(s) => {
                    self.folder_states
                        .insert((account_id, f.id.clone()), s.clone());
                }
                None => {
                    self.folder_states.remove(&(account_id, f.id.clone()));
                }
            }
        }
        let folders: Vec<Folder> = remote
            .into_iter()
            .map(|f| {
                if let Some(r) = f.role {
                    roles.insert(f.id.clone(), r.to_owned());
                }
                if f.is_view {
                    views.insert(f.id.clone());
                }
                Folder {
                    account_id,
                    id: f.id,
                    parent_id: f.parent_id,
                    name: f.name,
                    role: f.role.map(str::to_owned),
                    unread: f.unread,
                    total: f.total,
                }
            })
            .collect();
        self.roles.insert(account_id, roles);
        self.views.insert(account_id, views);
        self.store
            .replace_folders(account_id, &folders)
            .map_err(|e| e.to_string())?;
        emit(Event::Changed);
        Ok(folders)
    }

    /// Inbox only. `force`: the server announced a change (IDLE); otherwise one STATUS tells
    /// whether anything changed since the last sync.
    pub(crate) fn imap_sync_inbox(
        &mut self,
        account_id: i64,
        force: bool,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        let Some(inbox) = self.inbox_of(account_id)? else {
            return Ok(());
        };
        if !force {
            let id = inbox.id.clone();
            match self.with_imap(account_id, |s| s.state(&id))? {
                Some(state) => {
                    self.folder_states
                        .insert((account_id, inbox.id.clone()), state);
                }
                None => {
                    self.folder_states.remove(&(account_id, inbox.id.clone()));
                }
            }
        }
        self.imap_sync_folder(account_id, &inbox, force, emit)
    }

    /// Starts, once per account, a thread that waits on the inbox with IDLE and asks this
    /// worker to sync it as soon as the server announces a change. Mail then arrives within
    /// seconds instead of at the next 2-minute sync.
    fn watch_inbox(&mut self, account_id: i64) {
        if !self.watched.insert(account_id) {
            return;
        }
        let Ok(server) = self.server(account_id) else {
            return;
        };
        let again = self.again.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("mailcraft-idle-{account_id}"))
            .spawn(move || {
                loop {
                    let result = Imap::connect(&server).and_then(|mut s| {
                        s.watch_inbox(|| {
                            again.send(crate::Request::SyncInbox { account_id }).is_ok()
                        })
                    });
                    match result {
                        // The worker is gone (MailCraft closing).
                        Ok(()) => return,
                        // Wrong password or no IDLE on this server: the 2-minute sync remains.
                        Err(ImapError::Auth(e) | ImapError::Refused(e)) => {
                            log::info!("no instant mail for account {account_id}: {e}");
                            let _ = again.send(crate::Request::IdleStopped { account_id });
                            return;
                        }
                        Err(e) => {
                            log::info!("instant mail for account {account_id} paused: {e}");
                            std::thread::sleep(std::time::Duration::from_secs(60));
                        }
                    }
                }
            });
        if spawned.is_err() {
            self.watched.remove(&account_id);
        }
    }

    pub(crate) fn imap_sync_account(
        &mut self,
        account_id: i64,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        self.watch_inbox(account_id);
        let folders = self.imap_refresh_folders(account_id, emit)?;
        let views = self.views.get(&account_id).cloned().unwrap_or_default();
        let mut order: Vec<&Folder> = folders.iter().filter(|f| !views.contains(&f.id)).collect();
        order.sort_by_key(|f| {
            f.role
                .as_deref()
                .and_then(|r| mailcraft_store::ROLES.iter().position(|x| *x == r))
                .unwrap_or(mailcraft_store::ROLES.len())
        });
        for folder in order {
            self.imap_sync_folder(account_id, folder, false, emit)?;
            self.serve_urgent(emit);
        }
        Ok(())
    }

    /// Syncs one folder. Unless `force`, a folder whose server state (uidvalidity, next uid,
    /// message count, change counter) is the one seen at its last sync is skipped: nothing in
    /// it changed, not even a read mark.
    fn imap_sync_folder(
        &mut self,
        account_id: i64,
        folder: &Folder,
        force: bool,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        let state = self
            .folder_states
            .get(&(account_id, folder.id.clone()))
            .map(|s| format!("{s}:{}", self.config.months));
        if !force
            && let Some(state) = &state
            && self
                .store
                .delta_link(account_id, &folder.id)
                .map_err(|e| e.to_string())?
                .as_deref()
                == Some(state.as_str())
        {
            return Ok(());
        }
        let since = unix_now() - i64::from(self.config.months) * 30 * 24 * 3600;
        let scan = match self.try_imap(account_id, |s| s.scan(&folder.id, since)) {
            Ok(s) => s,
            Err(ImapError::NotFound) => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let local = self
            .store
            .messages(account_id, &folder.id, "", usize::MAX >> 1)
            .map_err(|e| e.to_string())?;
        let mut known: HashMap<u32, MessageSummary> = HashMap::new();
        let mut removed = Vec::new();
        for m in local {
            match mailcraft_imap::parse_id(&m.id) {
                Some((v, uid, mb))
                    if v == scan.uid_validity
                        && mb == folder.id
                        && scan.flags.contains_key(&uid) =>
                {
                    known.insert(uid, m);
                }
                _ => removed.push(m.id),
            }
        }
        let mut upserts: Vec<MessageSummary> = Vec::new();
        for (uid, m) in &known {
            let (read, flagged) = scan.flags[uid];
            if m.is_read != read || m.flagged != flagged {
                let mut m = m.clone();
                m.is_read = read;
                m.flagged = flagged;
                upserts.push(m);
            }
        }
        let mut new: Vec<u32> = scan
            .flags
            .keys()
            .filter(|u| !known.contains_key(u))
            .copied()
            .collect();
        // Newest first, so the top of the list fills in first.
        new.sort_unstable_by(|a, b| b.cmp(a));
        if !upserts.is_empty() || !removed.is_empty() {
            self.store
                .apply_changes(account_id, &folder.id, &upserts, &removed)
                .map_err(|e| e.to_string())?;
            emit(Event::Changed);
        }
        let blocked: HashSet<String> = if folder.role.as_deref() == Some("inbox") {
            self.store
                .senders(account_id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|s| s.blocked)
                .map(|s| s.address.to_lowercase())
                .collect()
        } else {
            HashSet::new()
        };
        let mut count = 0;
        for chunk in new.chunks(200) {
            let fetched = self.with_imap(account_id, |s| {
                s.summaries(&folder.id, scan.uid_validity, chunk)
            })?;
            let rows: Vec<MessageSummary> = fetched
                .into_iter()
                .map(|s| summary(account_id, &folder.id, s))
                .collect();
            count += rows.len();
            self.store
                .apply_changes(account_id, &folder.id, &rows, &[])
                .map_err(|e| e.to_string())?;
            emit(Event::Syncing {
                folder: folder.name.clone(),
                messages: count,
            });
            emit(Event::Changed);
            // Senders blocked in the clean-up view: IMAP has no server rule, MailCraft files
            // their new mail itself.
            let to_junk: Vec<String> = rows
                .iter()
                .filter(|r| blocked.contains(&r.from_addr.to_lowercase()))
                .map(|r| r.id.clone())
                .collect();
            self.serve_urgent(emit);
            if !to_junk.is_empty()
                && let Some(junk) = self.role_of(account_id, "junkemail")
            {
                for id in &to_junk {
                    match self.try_imap(account_id, |s| s.move_to(id, &junk)) {
                        Ok(()) | Err(ImapError::NotFound) => {}
                        Err(e) => return Err(e.to_string()),
                    }
                }
                self.store
                    .delete_local(account_id, &to_junk)
                    .map_err(|e| e.to_string())?;
            }
        }
        if let Some(state) = state {
            self.store
                .set_delta_link(account_id, &folder.id, Some(&state))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn role_of(&self, account_id: i64, role: &str) -> Option<String> {
        self.roles
            .get(&account_id)?
            .iter()
            .find(|(_, r)| *r == role)
            .map(|(id, _)| id.clone())
    }

    /// The whole message; the last one read is kept for its images and attachments.
    fn raw(&mut self, account_id: i64, id: &str) -> Result<Vec<u8>, String> {
        if let Some((key, bytes)) = &self.raw
            && key == id
        {
            return Ok(bytes.clone());
        }
        let bytes = self.with_imap(account_id, |s| s.raw(id))?;
        self.raw = Some((id.to_owned(), bytes.clone()));
        Ok(bytes)
    }

    pub(crate) fn imap_fetch_body(&mut self, account_id: i64, id: &str) -> Result<(), String> {
        let raw = self.raw(account_id, id)?;
        let p = mime::parse(&raw);
        let body = MessageBody {
            text: p.text,
            html: p.html,
            to: p.to,
            cc: p.cc,
        };
        let attachments: Vec<Attachment> = p
            .parts
            .into_iter()
            .map(|a| Attachment {
                id: a.id,
                name: a.name,
                content_type: a.content_type,
                size: a.size,
                is_inline: a.is_inline,
            })
            .collect();
        self.store
            .put_body(account_id, id, &body, &attachments)
            .map_err(|e| e.to_string())
    }

    pub(crate) fn imap_inline(
        &mut self,
        account_id: i64,
        id: &str,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let raw = self.raw(account_id, id)?;
        Ok(mime::parse(&raw)
            .parts
            .into_iter()
            .filter(|p| p.is_inline && p.size <= MAX_IMAGE as i64)
            .filter_map(|p| Some((p.content_id, mime::part_bytes(&raw, &p.id)?)))
            .collect())
    }

    pub(crate) fn imap_attachment(
        &mut self,
        account_id: i64,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<u8>, String> {
        let raw = self.raw(account_id, message_id)?;
        mime::part_bytes(&raw, attachment_id).ok_or_else(|| "attachment not found".to_owned())
    }

    pub(crate) fn imap_apply(&mut self, account_id: i64, op: &PendingOp) -> Result<(), ImapError> {
        self.try_imap(account_id, |s| match op {
            PendingOp::Move { id, dest } => s.move_to(id, dest),
            PendingOp::Delete { id } => s.delete(id),
            PendingOp::SetRead { id, read } => s.set_flag(id, "\\Seen", *read),
            PendingOp::SetFlag { id, flagged } => s.set_flag(id, "\\Flagged", *flagged),
        })
    }

    /// Full name for a folder named `name` under `parent`.
    fn child_name(
        &mut self,
        account_id: i64,
        parent: Option<&str>,
        name: &str,
    ) -> Result<String, String> {
        let encoded = mailcraft_imap::utf7::encode(name.trim());
        Ok(match parent {
            Some(p) => {
                let d = self.with_imap(account_id, |s| Ok(s.delimiter()))?;
                format!("{p}{d}{encoded}")
            }
            None => encoded,
        })
    }

    pub(crate) fn imap_create_folder(
        &mut self,
        account_id: i64,
        parent: Option<&str>,
        name: &str,
    ) -> Result<(), String> {
        let full = self.child_name(account_id, parent, name)?;
        self.with_imap(account_id, |s| s.create_folder(&full))
    }

    pub(crate) fn imap_rename_folder(
        &mut self,
        account_id: i64,
        id: &str,
        name: &str,
    ) -> Result<(), String> {
        let parent = self
            .store
            .folders(account_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|f| f.id == id)
            .and_then(|f| f.parent_id);
        let full = self.child_name(account_id, parent.as_deref(), name)?;
        self.with_imap(account_id, |s| s.rename_folder(id, &full))
    }

    pub(crate) fn imap_delete_folder(&mut self, account_id: i64, id: &str) -> Result<(), String> {
        self.with_imap(account_id, |s| s.delete_folder(id))
    }

    /// The message as it will be sent: our HTML, then the quoted original for a reply/forward.
    fn imap_mail(
        &mut self,
        account_id: i64,
        base: &Compose,
        out: &Outgoing,
        files: &[(String, Vec<u8>)],
    ) -> Result<lettre::Message, String> {
        let server = self.server(account_id).map_err(|e| e.to_string())?;
        let original = match base {
            Compose::New => None,
            Compose::Reply(id) | Compose::ReplyAll(id) | Compose::Forward(id) => Some(id.clone()),
        };
        let mut html = out.html.clone();
        let (mut in_reply_to, mut references) = (String::new(), String::new());
        if let Some(id) = original
            && let Ok(raw) = self.raw(account_id, &id)
        {
            let p = mime::parse(&raw);
            let quoted = if p.html.is_empty() {
                format!("<pre>{}</pre>", html_escape(&p.text))
            } else {
                p.html.clone()
            };
            let from = if p.from_name.is_empty() {
                p.from_addr.clone()
            } else {
                format!("{} &lt;{}&gt;", html_escape(&p.from_name), p.from_addr)
            };
            html.push_str(&format!(
                    "<hr><p><b>De :</b> {from}<br><b>Objet :</b> {}</p><blockquote>{quoted}</blockquote>",
                    html_escape(&p.subject)
                ));
            if !matches!(base, Compose::Forward(_)) && !p.message_id.is_empty() {
                in_reply_to = format!("<{}>", p.message_id);
                references = format!("{} {in_reply_to}", p.references).trim().to_owned();
            }
        }
        let text = mailcraft_html::to_text(&mailcraft_html::parse(&html));
        smtp::build(
            &server,
            &smtp::Mail {
                to: &out.to,
                cc: &out.cc,
                bcc: &out.bcc,
                subject: &out.subject,
                html: &html,
                text: &text,
                in_reply_to: Some(&in_reply_to),
                references: Some(&references),
                files,
            },
        )
        .map_err(|e| e.to_string())
    }

    /// Saves the draft in the Drafts folder (replacing the previous version). Returns its id.
    pub(crate) fn imap_save_draft(
        &mut self,
        account_id: i64,
        base: &Compose,
        draft_id: Option<String>,
        out: &Outgoing,
    ) -> Result<String, String> {
        let message = self.imap_mail(account_id, base, out, &[])?;
        let Some(drafts) = self.role_of(account_id, "drafts") else {
            return Ok(draft_id.unwrap_or_else(|| "local-draft".into()));
        };
        let bytes = message.formatted();
        let new_id = self.with_imap(account_id, |s| {
            s.append(
                &drafts,
                &bytes,
                &[imap::types::Flag::Draft, imap::types::Flag::Seen],
            )
        })?;
        if let Some(old) = draft_id.filter(|d| mailcraft_imap::parse_id(d).is_some()) {
            let _ = self.try_imap(account_id, |s| s.delete(&old));
        }
        Ok(new_id.unwrap_or_else(|| format!("local-draft-{}", unix_now())))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn imap_send(
        &mut self,
        account_id: i64,
        base: &Compose,
        draft_id: Option<String>,
        out: &Outgoing,
        attachments: &[PathBuf],
    ) -> Result<(), String> {
        let mut files = Vec::new();
        for path in attachments {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path.file_name().map_or_else(
                || "piece-jointe".into(),
                |n| n.to_string_lossy().into_owned(),
            );
            files.push((name, bytes));
        }
        let server = self.server(account_id).map_err(|e| e.to_string())?;
        let message = self.imap_mail(account_id, base, out, &files)?;
        smtp::send(&server, &message).map_err(|e| e.to_string())?;
        // Gmail files sent mail itself; elsewhere MailCraft puts the copy in Sent.
        if !server.is_gmail()
            && let Some(sent) = self.role_of(account_id, "sentitems")
        {
            let bytes = message.formatted();
            if let Err(e) = self.try_imap(account_id, |s| {
                s.append(&sent, &bytes, &[imap::types::Flag::Seen])
            }) {
                log::warn!("sent copy not stored: {e}");
            }
        }
        if let Some(old) = draft_id.filter(|d| mailcraft_imap::parse_id(d).is_some()) {
            let _ = self.try_imap(account_id, |s| s.delete(&old));
        }
        Ok(())
    }

    pub(crate) fn imap_discard_draft(
        &mut self,
        account_id: i64,
        draft_id: &str,
    ) -> Result<(), String> {
        if mailcraft_imap::parse_id(draft_id).is_none() {
            return Ok(());
        }
        match self.try_imap(account_id, |s| s.delete(draft_id)) {
            Ok(()) | Err(ImapError::NotFound) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Searches every synced folder on the server; adds what it finds. Returns how many.
    pub(crate) fn imap_search(&mut self, account_id: i64, query: &str) -> Result<usize, String> {
        let views = self.views.get(&account_id).cloned().unwrap_or_default();
        let folders: Vec<String> = self
            .store
            .folders(account_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|f| f.id)
            .filter(|f| !views.contains(f))
            .collect();
        let mut count = 0;
        for folder in folders {
            if count >= SEARCH_LIMIT {
                break;
            }
            let (validity, mut uids) = match self.try_imap(account_id, |s| s.search(&folder, query))
            {
                Ok(r) => r,
                Err(ImapError::NotFound | ImapError::Refused(_)) => continue,
                Err(e) => return Err(e.to_string()),
            };
            uids.sort_unstable_by(|a, b| b.cmp(a));
            uids.truncate(SEARCH_LIMIT - count);
            if uids.is_empty() {
                continue;
            }
            let rows: Vec<MessageSummary> = self
                .with_imap(account_id, |s| s.summaries(&folder, validity, &uids))?
                .into_iter()
                .map(|s| summary(account_id, &folder, s))
                .collect();
            count += rows.len();
            self.store
                .apply_changes(account_id, &folder, &rows, &[])
                .map_err(|e| e.to_string())?;
        }
        Ok(count)
    }

    pub(crate) fn imap_headers(
        &mut self,
        account_id: i64,
        id: &str,
    ) -> Result<Option<Vec<(String, String)>>, String> {
        match self.try_imap(account_id, |s| s.headers(id)) {
            Ok(h) => Ok(Some(h)),
            Err(ImapError::NotFound) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Ids of every message from `address`, in every synced folder.
    pub(crate) fn imap_messages_from(
        &mut self,
        account_id: i64,
        address: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let views = self.views.get(&account_id).cloned().unwrap_or_default();
        let folders: Vec<String> = self
            .store
            .folders(account_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|f| f.id)
            .filter(|f| !views.contains(f))
            .collect();
        let mut out = Vec::new();
        for folder in folders {
            match self.try_imap(account_id, |s| s.from_sender(&folder, address)) {
                Ok(ids) => out.extend(ids.into_iter().map(|i| (i, folder.clone()))),
                Err(ImapError::NotFound | ImapError::Refused(_)) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(out)
    }

    pub(crate) fn imap_send_plain(
        &mut self,
        account_id: i64,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), String> {
        let server = self.server(account_id).map_err(|e| e.to_string())?;
        let to = [to.to_owned()];
        let message = smtp::build(
            &server,
            &smtp::Mail {
                to: &to,
                cc: &[],
                bcc: &[],
                subject,
                html: &format!("<p>{}</p>", html_escape(body)),
                text: body,
                in_reply_to: None,
                references: None,
                files: &[],
            },
        )
        .map_err(|e| e.to_string())?;
        smtp::send(&server, &message).map_err(|e| e.to_string())
    }
}
