//! What the panes show, re-read from the local store whenever the sync worker changes it.

use mailcraft_store::{Account, Folder, MessageBody, MessageSummary, Store};

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
    Error(String),
}

#[derive(Default)]
pub struct Mail {
    pub accounts: Vec<Account>,
    pub folders: Vec<Folder>,
    pub messages: Vec<MessageSummary>,
    /// (account id, folder id)
    pub folder: Option<(i64, String)>,
    /// (account id, message id)
    pub message: Option<(i64, String)>,
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

    pub fn selected(&self) -> Option<&MessageSummary> {
        let (a, id) = self.message.as_ref()?;
        self.messages
            .iter()
            .find(|m| m.account_id == *a && &m.id == id)
    }
}
