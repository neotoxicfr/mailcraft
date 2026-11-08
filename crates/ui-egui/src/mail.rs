//! What the panes show, re-read from the local store whenever the sync worker changes it.

use mailcraft_store::{Account, Folder, MessageBody, MessageSummary, ROLES, Store};

/// Most recent messages listed per folder; older ones stay in the store.
const LIST_LIMIT: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Ready,
    SigningIn,
    Syncing {
        folder: String,
        messages: usize,
    },
    UpToDate,
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
    pub status: Status,
    pub signing_in: bool,
    /// `None` until a store is attached.
    pub encrypted: Option<bool>,
}

impl Mail {
    pub fn reload(&mut self, store: &Store, search: &str) -> mailcraft_store::Result<()> {
        self.encrypted = Some(store.is_encrypted());
        self.accounts = store.accounts()?;
        self.folders.clear();
        for a in &self.accounts {
            self.folders.extend(store.folders(a.id)?);
        }
        let valid = self.folder.as_ref().is_some_and(|(a, f)| {
            self.folders
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
        self.messages = match &self.folder {
            Some((a, f)) => store.messages(*a, f, search, LIST_LIMIT)?,
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
        Ok(())
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
    pub fn folder_tree(&self, account_id: i64) -> Vec<(&Folder, usize)> {
        let mut out = Vec::new();
        tree(&self.folders, account_id, None, 1, &mut out);
        out
    }

    pub fn selected(&self) -> Option<&MessageSummary> {
        let (a, id) = self.message.as_ref()?;
        self.messages
            .iter()
            .find(|m| m.account_id == *a && &m.id == id)
    }
}

/// Depth-first: well-known folders first in Outlook's order, then the others by name.
fn tree<'a>(
    all: &'a [Folder],
    account: i64,
    parent: Option<&str>,
    depth: usize,
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
    level.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for f in level {
        out.push((f, depth));
        tree(all, account, Some(&f.id), depth + 1, out);
    }
}
