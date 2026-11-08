//! What the panes show, re-read from the local store whenever the sync worker changes it.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use mailcraft_store::{Account, Attachment, Folder, MessageBody, MessageSummary, ROLES, Store};

/// Id of the screener's virtual folder « À valider ».
pub const SCREENER: &str = "__screener";
/// Virtual folder: the inboxes of every account, newest first.
pub const UNIFIED: &str = "__all_inboxes";

/// Most recent messages listed per folder; older ones stay in the store.
const LIST_LIMIT: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Ready,
    SigningIn,
    /// Checking the address and password of an IMAP account.
    Connecting,
    Syncing {
        folder: String,
        messages: usize,
    },
    UpToDate,
    SearchingServer,
    ClassifyDone(usize),
    RebuildScheduled,
    CleanupDone {
        done: usize,
        skipped: usize,
        dry_run: bool,
    },
    Sent,
    AttachmentSaved,
    ServerFound(usize),
    Exported(usize),
    /// Changes made locally wait for the network.
    Offline {
        pending: usize,
    },
    Error(String),
}

#[derive(Default)]
pub struct Mail {
    pub accounts: Vec<Account>,
    pub folders: Vec<Folder>,
    pub messages: Vec<MessageSummary>,
    /// (account id, folder id)
    pub folder: Option<(i64, String)>,
    /// (account id, message id) shown in the reading pane.
    pub message: Option<(i64, String)>,
    /// Selected messages (all in the listed folder), and where a Shift+click range starts.
    pub selection: Vec<(i64, String)>,
    pub anchor: Option<(i64, String)>,
    pub body: Option<MessageBody>,
    /// The shown body as blocks for the simplified reader.
    pub blocks: Vec<mailcraft_html::Block>,
    pub attachments: Vec<Attachment>,
    /// Inline images of the shown message: content id → bytes.
    pub inline: HashMap<String, Vec<u8>>,
    inline_requested: bool,
    /// The message's HTML prepared for the original view (inline images embedded, CSP added).
    pub original_html: String,
    /// The message's HTML with its pixels removed as the blocking says (built once per
    /// message): the original view only re-embeds inline images into it.
    prepared_html: String,
    body_key: Option<(i64, String)>,
    /// The user's order: account addresses, and folder ids per account id.
    pub account_order: Vec<String>,
    /// Folder ids in the user's order, per account address (lowercase).
    pub folder_order: std::collections::BTreeMap<String, Vec<String>>,
    /// Image blocking applied to the shown message: (remote images, tracking pixels).
    pub blocking: (bool, bool),
    blocking_applied: (bool, bool),
    /// Remote images: asked for, loaded (bytes given to egui), or failed.
    pub images_requested: HashSet<String>,
    pub images_ready: HashSet<String>,
    /// Clean-up view: senders of the current account, the action log, analysis progress.
    pub senders: Vec<mailcraft_store::SenderRow>,
    pub actions: Vec<mailcraft_store::ActionEntry>,
    pub analyzing: Option<(usize, usize)>,
    pub classifying: Option<(usize, usize)>,
    /// Senders waiting in the screener (when it is on).
    pub screening: usize,
    pub status: Status,
    pub signing_in: bool,
    /// The list shows search results (all folders) rather than one folder.
    pub searching: bool,
    /// `None` until a store is attached.
    pub encrypted: Option<bool>,
}

impl Mail {
    pub fn reload(
        &mut self,
        store: &Store,
        search: &str,
        cleanup: bool,
        screener: Option<i64>,
    ) -> mailcraft_store::Result<()> {
        if cleanup
            && let Some((account, _)) = self
                .folder
                .clone()
                .or_else(|| self.accounts.first().map(|a| (a.id, String::new())))
        {
            self.senders = store.senders(account)?;
            self.actions = store.actions_log(200)?;
        }
        self.encrypted = Some(store.is_encrypted());
        self.accounts = store.accounts()?;
        let rank = |email: &str| {
            self.account_order
                .iter()
                .position(|e| e.eq_ignore_ascii_case(email))
                .unwrap_or(usize::MAX)
        };
        self.accounts.sort_by_key(|a| rank(&a.email));
        self.folders.clear();
        for a in &self.accounts {
            self.folders.extend(store.folders(a.id)?);
        }
        self.screening = 0;
        if let Some(since) = screener {
            let account = self
                .folder
                .as_ref()
                .map(|(a, _)| *a)
                .or(self.accounts.first().map(|a| a.id));
            if let Some(inbox) = account
                .and_then(|a| self.folder_with_role(a, "inbox"))
                .map(|f| (f.account_id, f.id.clone()))
            {
                let waiting = store.screening(inbox.0, &inbox.1, since)?;
                let senders: HashSet<String> = waiting
                    .into_iter()
                    .map(|m| m.from_addr.to_lowercase())
                    .collect();
                self.screening = senders.len();
            }
        }
        let valid = self.folder.as_ref().is_some_and(|(a, f)| {
            (f == SCREENER && screener.is_some())
                || (f == UNIFIED && self.accounts.len() > 1)
                || self
                    .folders
                    .iter()
                    .any(|x| x.account_id == *a && &x.id == f)
        });
        if !valid {
            self.folder = self
                .folders
                .iter()
                .find(|f| f.role.as_deref() == Some("inbox"))
                .or(self.folders.first())
                .map(|f| (f.account_id, f.id.clone()));
        }
        // A search covers every folder of the account; otherwise the folder's messages.
        self.searching = !search.trim().is_empty();
        self.messages = match &self.folder {
            Some((a, _)) if self.searching => store.search(*a, search, LIST_LIMIT)?,
            Some((_, f)) if f == UNIFIED => {
                let mut all = Vec::new();
                for inbox in self
                    .folders
                    .iter()
                    .filter(|f| f.role.as_deref() == Some("inbox"))
                {
                    all.extend(store.messages(inbox.account_id, &inbox.id, "", LIST_LIMIT)?);
                }
                all.sort_by_key(|m| std::cmp::Reverse(m.received));
                all.truncate(LIST_LIMIT);
                all
            }
            Some((a, f)) if f == SCREENER => match (screener, self.folder_with_role(*a, "inbox")) {
                (Some(since), Some(inbox)) => store.screening(*a, &inbox.id.clone(), since)?,
                _ => Vec::new(),
            },
            Some((a, f)) => store.messages(*a, f, "", LIST_LIMIT)?,
            None => Vec::new(),
        };
        let listed = |a: i64, id: &str, list: &[MessageSummary]| {
            list.iter().any(|m| m.account_id == a && m.id == id)
        };
        let messages = std::mem::take(&mut self.messages);
        self.selection.retain(|(a, id)| listed(*a, id, &messages));
        self.messages = messages;
        if let Some((a, id)) = &self.message
            && !self
                .messages
                .iter()
                .any(|m| m.account_id == *a && &m.id == id)
        {
            self.message = None;
        }
        self.body = match &self.message {
            Some((a, id)) => store.body(*a, id)?,
            None => None,
        };
        // Parse once per message (or when its body arrives), not every frame.
        let key = self.message.clone().filter(|_| self.body.is_some());
        if key != self.body_key || self.blocking != self.blocking_applied {
            self.body_key = key;
            self.blocking_applied = self.blocking;
            self.inline.clear();
            self.inline_requested = false;
            match (&self.message, &self.body) {
                (Some((a, id)), Some(b)) => {
                    let (remote, trackers) = self.blocking;
                    (self.blocks, self.prepared_html) = if b.html.is_empty() {
                        (mailcraft_html::from_text(&b.text), String::new())
                    } else {
                        // One parse: the blocks, and the pixels to drop from the original.
                        let (blocks, pixels) = mailcraft_html::parse_full(&b.html, trackers);
                        let prepared = if trackers && !remote {
                            mailcraft_html::without_tracker_urls(&b.html, &pixels)
                        } else {
                            b.html.clone()
                        };
                        (blocks, prepared)
                    };
                    self.attachments = store.attachments(*a, id)?;
                    self.original_html = page(&self.prepared_html, &self.inline, remote);
                }
                _ => {
                    self.blocks.clear();
                    self.attachments.clear();
                    self.original_html.clear();
                    self.prepared_html.clear();
                }
            }
        }
        Ok(())
    }

    /// Inline images arrived for the message shown.
    pub fn set_inline(&mut self, images: Vec<(String, Vec<u8>)>) {
        self.inline = images.into_iter().collect();
        if self.body.is_some() {
            self.original_html = page(&self.prepared_html, &self.inline, self.blocking.0);
        }
    }

    /// The shown message has inline images not requested yet.
    pub fn wants_inline(&mut self) -> Option<(i64, String)> {
        if self.inline_requested || !self.attachments.iter().any(|a| a.is_inline) {
            return None;
        }
        self.inline_requested = true;
        self.body_key.clone()
    }

    pub fn is_selected(&self, account_id: i64, id: &str) -> bool {
        self.selection
            .iter()
            .any(|(a, i)| *a == account_id && i == id)
    }

    /// Selected messages, in list order.
    pub fn selected_messages(&self) -> Vec<&MessageSummary> {
        self.messages
            .iter()
            .filter(|m| self.is_selected(m.account_id, &m.id))
            .collect()
    }

    pub fn folder_name(&self, account_id: i64, id: &str) -> &str {
        self.folders
            .iter()
            .find(|f| f.account_id == account_id && f.id == id)
            .map_or("", |f| f.name.as_str())
    }

    pub fn current_folder(&self) -> Option<&Folder> {
        let (a, id) = self.folder.as_ref()?;
        self.folders
            .iter()
            .find(|f| f.account_id == *a && &f.id == id)
    }

    pub fn folder_with_role(&self, account_id: i64, role: &str) -> Option<&Folder> {
        self.folders
            .iter()
            .find(|f| f.account_id == account_id && f.role.as_deref() == Some(role))
    }

    /// The account's folders, tree order, with their depth (1 for top level).
    /// Account addresses (lowercase) in the order shown.
    pub fn account_emails(&self) -> Vec<String> {
        self.accounts
            .iter()
            .map(|a| a.email.to_lowercase())
            .collect()
    }

    /// A folder's account address and the ids of its siblings (itself included), in the
    /// order shown.
    pub fn siblings(&self, account_id: i64, id: &str) -> Option<(String, Vec<String>)> {
        let email = self
            .accounts
            .iter()
            .find(|a| a.id == account_id)?
            .email
            .to_lowercase();
        let parent = &self
            .folders
            .iter()
            .find(|f| f.account_id == account_id && f.id == id)?
            .parent_id;
        let ids = self
            .folder_tree(account_id)
            .into_iter()
            .filter(|(f, _)| &f.parent_id == parent)
            .map(|(f, _)| f.id.clone())
            .collect();
        Some((email, ids))
    }

    pub fn folder_tree(&self, account_id: i64) -> Vec<(&Folder, usize)> {
        let mut out = Vec::new();
        let order = self
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .and_then(|a| self.folder_order.get(&a.email.to_lowercase()))
            .map_or(&[][..], Vec::as_slice);
        tree(&self.folders, account_id, None, 1, order, &mut out);
        out
    }

    pub fn selected(&self) -> Option<&MessageSummary> {
        let (a, id) = self.message.as_ref()?;
        self.messages
            .iter()
            .find(|m| m.account_id == *a && &m.id == id)
    }
}

/// Depth-first: the user's order first, then well-known folders in Outlook's order, then the
/// others by name.
fn tree<'a>(
    all: &'a [Folder],
    account: i64,
    parent: Option<&str>,
    depth: usize,
    order: &[String],
    out: &mut Vec<(&'a Folder, usize)>,
) {
    let mut level: Vec<&Folder> = all
        .iter()
        .filter(|f| f.account_id == account && f.parent_id.as_deref() == parent)
        .collect();
    let rank = |f: &Folder| {
        f.role
            .as_deref()
            .and_then(|r| ROLES.iter().position(|x| *x == r))
            .unwrap_or(ROLES.len())
    };
    let mine = |f: &Folder| order.iter().position(|x| *x == f.id).unwrap_or(usize::MAX);
    level.sort_by_cached_key(|f| (mine(f), rank(f), f.name.to_lowercase()));
    for f in level {
        out.push((f, depth));
        tree(all, account, Some(&f.id), depth + 1, order, out);
    }
}

/// The original HTML for the WebView: `cid:` images replaced by their bytes, scripts and
/// plug-ins forbidden by a Content-Security-Policy (JavaScript is also off in the view).
/// The message's HTML for the original view: scripts, frames and forms off; tracking pixels
/// removed and remote images refused as `blocking` (remote, trackers) says; inline images
/// embedded.
pub fn original_html(
    html: &str,
    inline: &HashMap<String, Vec<u8>>,
    (remote, trackers): (bool, bool),
) -> String {
    page(
        &mailcraft_html::prepare(html, remote, trackers),
        inline,
        remote,
    )
}

/// The original-view page from HTML already rid of its pixels: the page's security policy,
/// then the HTML with inline (`cid:`) images embedded.
fn page(html: &str, inline: &HashMap<String, Vec<u8>>, remote: bool) -> String {
    if html.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(html.len() + 512);
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    out.push_str(&mailcraft_html::page_policy(remote, None));
    out.push_str("\">");
    let mut rest: &str = html;
    while let Some(at) = rest.find("cid:") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 4..];
        let end = after
            .find(['"', '\'', ' ', '>', ')'])
            .unwrap_or(after.len());
        let cid = &after[..end];
        match inline.get(cid) {
            Some(bytes) => {
                out.push_str("data:");
                out.push_str(image_mime(bytes));
                out.push_str(";base64,");
                out.push_str(&base64::engine::general_purpose::STANDARD.encode(bytes));
            }
            None => {
                out.push_str("cid:");
                out.push_str(cid);
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn image_mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, ..] => "image/jpeg",
        [b'G', b'I', b'F', ..] => "image/gif",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => "image/webp",
        [b'B', b'M', ..] => "image/bmp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::original_html;
    use std::collections::HashMap;

    #[test]
    fn cid_images_are_embedded_and_scripts_forbidden() {
        let inline = HashMap::from([("logo@x".to_string(), vec![0x89, b'P', b'N', b'G', 1])]);
        let out = original_html(
            "<img src=\"cid:logo@x\"><img src='cid:missing'>",
            &inline,
            (false, true),
        );
        assert!(out.starts_with(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'"
        ));
        assert!(out.contains("script-src 'none'"));
        assert!(out.contains("src=\"data:image/png;base64,iVBORwE=\""));
        assert!(out.contains("src='cid:missing'"));
        assert!(original_html("", &inline, (false, true)).is_empty());
        // Remote images refused through the page's policy; pixels removed when only they are.
        let page =
            "<img src=\"https://x.example/a.jpg\"><img src=\"https://x.example/t\" width=\"1\">";
        assert!(original_html(page, &inline, (true, true)).contains("img-src data:"));
        let only_pixels = original_html(page, &inline, (false, true));
        assert!(only_pixels.contains("img-src data: https:") && only_pixels.contains("a.jpg"));
        assert!(!only_pixels.contains("x.example/t"));
        assert!(original_html(page, &inline, (false, false)).contains("x.example/t"));
    }
}
