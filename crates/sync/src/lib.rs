//! Background worker: everything that talks to the network runs here, on its own thread with
//! its own database connection. The UI sends [`Request`]s and reads [`Event`]s, then re-reads
//! the store.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use mailcraft_graph::auth::{self, AuthError, Tokens};
use mailcraft_graph::compose::{Base, Outgoing};
use mailcraft_graph::{Graph, GraphError, Op};
use mailcraft_store::{Attachment, Folder, MasterKey, MessageBody, MessageSummary, ROLES, Store};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

pub const PROVIDER_MICROSOFT: &str = "microsoft";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
const SYNC_EVERY: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct Config {
    pub db_path: PathBuf,
    pub key: MasterKey,
    pub encrypt_new: bool,
    pub client_id: String,
    /// How far back the first sync of a folder goes.
    pub months: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    AddMicrosoftAccount,
    SyncAll,
    FetchBody {
        account_id: i64,
        id: String,
    },
    /// Content of the message's inline (`cid:`) images.
    FetchInline {
        account_id: i64,
        id: String,
    },
    /// A remote image shown in a message (http/https only, 5 MB at most).
    FetchImage {
        url: String,
    },
    /// Writes an attachment to `path`, then opens it with its usual program when `open`.
    SaveAttachment {
        account_id: i64,
        message_id: String,
        attachment_id: String,
        path: PathBuf,
        open: bool,
    },
    /// File messages in another folder (`dest` is a folder id).
    Move {
        account_id: i64,
        ids: Vec<String>,
        dest: String,
    },
    /// Permanent deletion.
    Delete {
        account_id: i64,
        ids: Vec<String>,
    },
    SetRead {
        account_id: i64,
        ids: Vec<String>,
        read: bool,
    },
    SetFlag {
        account_id: i64,
        ids: Vec<String>,
        flagged: bool,
    },
    CreateFolder {
        account_id: i64,
        parent: Option<String>,
        name: String,
    },
    RenameFolder {
        account_id: i64,
        id: String,
        name: String,
    },
    DeleteFolder {
        account_id: i64,
        id: String,
    },
    /// Searches the whole mailbox on the server and adds what it finds to the local copy.
    SearchServer {
        account_id: i64,
        query: String,
    },
    /// Creates or updates the server draft of composer `composer`.
    SaveDraft {
        composer: u64,
        account_id: i64,
        base: Compose,
        draft_id: Option<String>,
        out: Outgoing,
    },
    /// Saves the draft, adds the attachments, sends it.
    Send {
        composer: u64,
        account_id: i64,
        base: Compose,
        draft_id: Option<String>,
        out: Outgoing,
        attachments: Vec<PathBuf>,
    },
    DiscardDraft {
        account_id: i64,
        draft_id: String,
    },
}

/// A message change made locally, kept in the store until Microsoft accepts it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
enum PendingOp {
    Move { id: String, dest: String },
    Delete { id: String },
    SetRead { id: String, read: bool },
    SetFlag { id: String, flagged: bool },
}

impl PendingOp {
    fn op(&self) -> Op<'_> {
        match self {
            Self::Move { id, dest } => Op::Move { id, dest },
            Self::Delete { id } => Op::Delete { id },
            Self::SetRead { id, read } => Op::SetRead { id, read: *read },
            Self::SetFlag { id, flagged } => Op::SetFlag {
                id,
                flagged: *flagged,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    SigningIn,
    SignInCancelled,
    AccountAdded {
        email: String,
    },
    Syncing {
        folder: String,
        messages: usize,
    },
    SyncDone,
    /// Data changed in the store: reload what is shown.
    Changed,
    BodyReady {
        account_id: i64,
        id: String,
    },
    /// Inline images of a message: (content id, bytes).
    InlineReady {
        account_id: i64,
        id: String,
        images: Vec<(String, Vec<u8>)>,
    },
    /// A remote image, or `None` when it could not be loaded.
    ImageReady {
        url: String,
        bytes: Option<Vec<u8>>,
    },
    AttachmentSaved {
        path: PathBuf,
    },
    DraftSaved {
        composer: u64,
        draft_id: String,
    },
    Sent {
        composer: u64,
    },
    SendFailed {
        composer: u64,
        error: String,
    },
    /// Changes could not be sent; they wait for the next sync.
    Offline {
        pending: usize,
        reason: String,
    },
    /// A server search finished; `found` messages are now in the local copy.
    ServerSearchDone {
        found: usize,
    },
    Error(String),
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    /// Starts the thread. `wake` is called after each event so the UI repaints.
    pub fn start(config: Config, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("mailcraft-sync".into())
            .spawn(move || {
                let side = (events.clone(), wake.clone());
                let emit = |e: Event| {
                    let _ = events.send(e);
                    wake();
                };
                match Runner::new(config, flag, side) {
                    Ok(mut runner) => runner.run(&requests, &emit),
                    Err(e) => emit(Event::Error(e)),
                }
            })
            .ok();
        Self { tx, rx, cancel }
    }

    pub fn send(&self, request: Request) {
        if request == Request::AddMicrosoftAccount {
            self.cancel.store(false, Ordering::Relaxed);
        }
        let _ = self.tx.send(request);
    }

    pub fn cancel_sign_in(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

/// Event sender and repaint callback for work done off the worker thread.
type Side = (Sender<Event>, Arc<dyn Fn() + Send + Sync>);

struct Runner {
    store: Store,
    http: Client,
    config: Config,
    cancel: Arc<AtomicBool>,
    tokens: HashMap<i64, Tokens>,
    /// Remote images are fetched on their own threads so they never hold up sync.
    images: Client,
    side: Side,
    roles: HashMap<i64, HashMap<String, String>>,
    /// Quoted part Microsoft put in each reply/forward draft, kept under the user's text.
    quoted: HashMap<String, String>,
    /// Accounts whose address book was read this run.
    contacts_done: std::collections::HashSet<i64>,
}

impl Runner {
    fn new(config: Config, cancel: Arc<AtomicBool>, side: Side) -> Result<Self, String> {
        let store = Store::open(&config.db_path, config.key.clone(), config.encrypt_new)
            .map_err(|e| e.to_string())?;
        let http = mailcraft_graph::http_client()?;
        Ok(Self {
            store,
            http,
            config,
            cancel,
            tokens: HashMap::new(),
            images: image_client()?,
            side,
            roles: HashMap::new(),
            quoted: HashMap::new(),
            contacts_done: std::collections::HashSet::new(),
        })
    }

    fn run(&mut self, requests: &Receiver<Request>, emit: &dyn Fn(Event)) {
        let mut next_sync = Instant::now();
        loop {
            let wait = next_sync.saturating_duration_since(Instant::now());
            let request = match requests.recv_timeout(wait) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => Request::SyncAll,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if request == Request::SyncAll {
                next_sync = Instant::now() + SYNC_EVERY;
            }
            if let Err(e) = self.handle(request, emit) {
                emit(Event::Error(e));
            }
        }
    }

    fn handle(&mut self, request: Request, emit: &dyn Fn(Event)) -> Result<(), String> {
        match request {
            Request::AddMicrosoftAccount => self.add_account(emit),
            Request::SyncAll => {
                self.flush_pending(emit)?;
                for account in self.store.accounts().map_err(|e| e.to_string())? {
                    self.sync_account(account.id, emit)?;
                }
                emit(Event::SyncDone);
                Ok(())
            }
            Request::FetchBody { account_id, id } => {
                if self
                    .store
                    .body(account_id, &id)
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    let b = self.with_graph(account_id, |g| g.body(&id))?;
                    let (text, html) = if b.is_html {
                        (
                            mailcraft_html::to_text(&mailcraft_html::parse(&b.content)),
                            b.content,
                        )
                    } else {
                        (b.content, String::new())
                    };
                    let body = MessageBody {
                        text,
                        html,
                        to: b.to,
                        cc: b.cc,
                    };
                    let attachments: Vec<Attachment> = b
                        .attachments
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
                        .put_body(account_id, &id, &body, &attachments)
                        .map_err(|e| e.to_string())?;
                }
                emit(Event::BodyReady { account_id, id });
                Ok(())
            }
            Request::FetchInline { account_id, id } => {
                let inline: Vec<String> = self
                    .store
                    .attachments(account_id, &id)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .filter(|a| a.is_inline && a.size <= MAX_IMAGE as i64)
                    .map(|a| a.id)
                    .collect();
                let mut images = Vec::new();
                for aid in inline {
                    let c = self.with_graph(account_id, |g| g.attachment(&id, &aid))?;
                    if !c.content_id.is_empty() {
                        images.push((c.content_id, c.bytes));
                    }
                }
                emit(Event::InlineReady {
                    account_id,
                    id,
                    images,
                });
                Ok(())
            }
            Request::FetchImage { url } => {
                let (http, (events, wake)) = (self.images.clone(), self.side.clone());
                std::thread::spawn(move || {
                    let bytes = fetch_image(&http, &url);
                    let _ = events.send(Event::ImageReady { url, bytes });
                    wake();
                });
                Ok(())
            }
            Request::SaveAttachment {
                account_id,
                message_id,
                attachment_id,
                path,
                open,
            } => {
                let bytes = self.with_graph(account_id, |g| {
                    g.attachment_bytes(&message_id, &attachment_id)
                })?;
                std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                if open {
                    open::that_detached(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                }
                emit(Event::AttachmentSaved { path });
                Ok(())
            }
            Request::Move {
                account_id,
                ids,
                dest,
            } => {
                let moved = self
                    .store
                    .move_local(account_id, &ids, &dest)
                    .map_err(|e| e.to_string())?;
                let ops = moved.into_iter().map(|(id, _)| PendingOp::Move {
                    id,
                    dest: dest.clone(),
                });
                self.queue(account_id, ops, emit)
            }
            Request::Delete { account_id, ids } => {
                self.store
                    .delete_local(account_id, &ids)
                    .map_err(|e| e.to_string())?;
                self.queue(
                    account_id,
                    ids.into_iter().map(|id| PendingOp::Delete { id }),
                    emit,
                )
            }
            Request::SetRead {
                account_id,
                ids,
                read,
            } => {
                for id in &ids {
                    self.store
                        .set_read(account_id, id, read)
                        .map_err(|e| e.to_string())?;
                }
                self.queue(
                    account_id,
                    ids.into_iter().map(|id| PendingOp::SetRead { id, read }),
                    emit,
                )
            }
            Request::SetFlag {
                account_id,
                ids,
                flagged,
            } => {
                for id in &ids {
                    self.store
                        .set_flag(account_id, id, flagged)
                        .map_err(|e| e.to_string())?;
                }
                self.queue(
                    account_id,
                    ids.into_iter().map(|id| PendingOp::SetFlag { id, flagged }),
                    emit,
                )
            }
            // Folder changes need the server's answer (new ids), so they are made online only.
            Request::CreateFolder {
                account_id,
                parent,
                name,
            } => {
                self.with_graph(account_id, |g| g.create_folder(parent.as_deref(), &name))?;
                self.refresh_folders(account_id, emit).map(|_| ())
            }
            Request::RenameFolder {
                account_id,
                id,
                name,
            } => {
                self.with_graph(account_id, |g| {
                    g.apply(&Op::RenameFolder {
                        id: &id,
                        name: &name,
                    })
                })?;
                self.refresh_folders(account_id, emit).map(|_| ())
            }
            Request::DeleteFolder { account_id, id } => {
                self.with_graph(account_id, |g| g.apply(&Op::DeleteFolder { id: &id }))?;
                self.refresh_folders(account_id, emit)?;
                // Its messages now sit in Deleted Items on the server.
                self.sync_account(account_id, emit)
            }
            Request::SaveDraft {
                composer,
                account_id,
                base,
                draft_id,
                out,
            } => {
                let draft_id = self.save_draft(account_id, &base, draft_id, &out)?;
                emit(Event::DraftSaved { composer, draft_id });
                Ok(())
            }
            Request::Send {
                composer,
                account_id,
                base,
                draft_id,
                out,
                attachments,
            } => {
                let result = self.send_message(
                    account_id,
                    &base,
                    draft_id,
                    &out,
                    &attachments,
                    composer,
                    emit,
                );
                match result {
                    Ok(()) => emit(Event::Sent { composer }),
                    Err(error) => emit(Event::SendFailed { composer, error }),
                }
                Ok(())
            }
            Request::DiscardDraft {
                account_id,
                draft_id,
            } => {
                self.quoted.remove(&draft_id);
                match self.try_graph(account_id, |g| g.apply(&Op::Delete { id: &draft_id })) {
                    Ok(_) | Err(GraphError::NotFound) => Ok(()),
                    Err(e) => Err(e.to_string()),
                }
            }
            Request::SearchServer { account_id, query } => {
                let found = self.with_graph(account_id, |g| g.search(&query, 100))?;
                let known: Vec<String> = self
                    .store
                    .folders(account_id)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|f| f.id)
                    .collect();
                let mut count = 0;
                for (folder_id, m) in found {
                    // Messages in folders MailCraft does not list (hidden ones) are left out.
                    if !known.contains(&folder_id) {
                        continue;
                    }
                    let summary = summary(account_id, &folder_id, m);
                    self.store
                        .apply_changes(account_id, &folder_id, &[summary], &[])
                        .map_err(|e| e.to_string())?;
                    count += 1;
                }
                emit(Event::Changed);
                emit(Event::ServerSearchDone { found: count });
                Ok(())
            }
        }
    }

    /// Records local changes, shows them, then tries to send them all.
    fn queue(
        &mut self,
        account_id: i64,
        ops: impl Iterator<Item = PendingOp>,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        for op in ops {
            let json = serde_json::to_string(&op).map_err(|e| e.to_string())?;
            self.store
                .push_pending(account_id, &json)
                .map_err(|e| e.to_string())?;
        }
        emit(Event::Changed);
        self.flush_pending(emit)
    }

    /// Sends pending changes in order. A network failure keeps them for the next sync; a change
    /// the server refuses is dropped and the folders are synced again so the local copy matches.
    fn flush_pending(&mut self, emit: &dyn Fn(Event)) -> Result<(), String> {
        let mut refused = Vec::new();
        for (pid, account_id, json) in self.store.pending().map_err(|e| e.to_string())? {
            let Ok(op) = serde_json::from_str::<PendingOp>(&json) else {
                self.store.drop_pending(pid).map_err(|e| e.to_string())?;
                continue;
            };
            match self.try_graph(account_id, |g| g.apply(&op.op())) {
                Ok(_) | Err(GraphError::NotFound) => {}
                Err(GraphError::Network(e)) => {
                    emit(Event::Offline {
                        pending: self.store.pending().map_or(0, |p| p.len()),
                        reason: e,
                    });
                    return Ok(());
                }
                Err(e) => {
                    refused.push(account_id);
                    emit(Event::Error(e.to_string()));
                }
            }
            self.store.drop_pending(pid).map_err(|e| e.to_string())?;
        }
        refused.dedup();
        for account_id in refused {
            self.sync_account(account_id, emit)?;
        }
        Ok(())
    }

    fn add_account(&mut self, emit: &dyn Fn(Event)) -> Result<(), String> {
        emit(Event::SigningIn);
        let cancel = self.cancel.clone();
        let tokens = match auth::sign_in(
            &self.http,
            &self.config.client_id,
            SIGN_IN_TIMEOUT,
            &move || cancel.load(Ordering::Relaxed),
        ) {
            Ok(t) => t,
            Err(AuthError::Cancelled) => {
                emit(Event::SignInCancelled);
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        let (email, name) = Graph::new(&self.http, &tokens.access)
            .me()
            .map_err(|e| e.to_string())?;
        let id = self
            .store
            .upsert_account(PROVIDER_MICROSOFT, &email, &name, &tokens.refresh)
            .map_err(|e| e.to_string())?;
        self.tokens.insert(id, tokens);
        emit(Event::AccountAdded { email });
        self.sync_account(id, emit)?;
        emit(Event::SyncDone);
        Ok(())
    }

    /// A valid access token for the account, refreshed (and the new refresh token kept) as needed.
    fn access_token(&mut self, account_id: i64, force: bool) -> Result<String, String> {
        if let Some(t) = self.tokens.get(&account_id)
            && !force
            && Instant::now() < t.expires_at
        {
            return Ok(t.access.clone());
        }
        let refresh = self
            .store
            .refresh_token(account_id)
            .map_err(|e| e.to_string())?
            .ok_or("no saved session for this account: add it again")?;
        let tokens = auth::refresh(
            &self.http,
            &self.config.client_id,
            &refresh,
            auth::MAIL_SCOPES,
        )
        .map_err(|e| e.to_string())?;
        if tokens.refresh != refresh {
            self.store
                .set_refresh_token(account_id, &tokens.refresh)
                .map_err(|e| e.to_string())?;
        }
        let access = tokens.access.clone();
        self.tokens.insert(account_id, tokens);
        Ok(access)
    }

    /// Runs `f`, refreshing the token once if Microsoft rejects it.
    fn with_graph<T>(
        &mut self,
        account_id: i64,
        f: impl Fn(&Graph<'_>) -> mailcraft_graph::Result<T>,
    ) -> Result<T, String> {
        self.try_graph(account_id, f).map_err(|e| e.to_string())
    }

    /// Like [`Self::with_graph`] but keeps the error kind. A session that cannot be refreshed
    /// counts as a network problem: the change waits rather than being dropped.
    fn try_graph<T>(
        &mut self,
        account_id: i64,
        f: impl Fn(&Graph<'_>) -> mailcraft_graph::Result<T>,
    ) -> mailcraft_graph::Result<T> {
        let token = self
            .access_token(account_id, false)
            .map_err(GraphError::Network)?;
        match f(&Graph::new(&self.http, &token)) {
            Err(GraphError::Unauthorized) => {
                let token = self
                    .access_token(account_id, true)
                    .map_err(GraphError::Network)?;
                f(&Graph::new(&self.http, &token))
            }
            r => r,
        }
    }

    /// Lists the account's folders again and stores them. Returns them.
    fn refresh_folders(
        &mut self,
        account_id: i64,
        emit: &dyn Fn(Event),
    ) -> Result<Vec<Folder>, String> {
        if !self.roles.contains_key(&account_id) {
            let mut roles = HashMap::new();
            for role in ROLES {
                if let Some(id) = self.with_graph(account_id, |g| g.well_known_id(role))? {
                    roles.insert(id, role.to_owned());
                }
            }
            self.roles.insert(account_id, roles);
        }
        let roles = self.roles[&account_id].clone();
        let remote = self.with_graph(account_id, |g| g.folders())?;
        let folders: Vec<Folder> = remote
            .into_iter()
            .map(|f| Folder {
                account_id,
                role: roles.get(&f.id).cloned(),
                id: f.id,
                parent_id: f.parent_id,
                name: f.name,
                unread: f.unread,
                total: f.total,
            })
            .collect();
        self.store
            .replace_folders(account_id, &folders)
            .map_err(|e| e.to_string())?;
        emit(Event::Changed);
        Ok(folders)
    }

    fn sync_account(&mut self, account_id: i64, emit: &dyn Fn(Event)) -> Result<(), String> {
        let folders = self.refresh_folders(account_id, emit)?;
        self.sync_folders(account_id, &folders, emit)?;
        if self.contacts_done.insert(account_id) {
            self.sync_contacts(account_id);
        }
        Ok(())
    }

    /// Reads the address book for completion. Needs Contacts.Read, which accounts added before
    /// it was asked for do not have: then completion uses known senders only, silently.
    fn sync_contacts(&mut self, account_id: i64) {
        let Ok(Some(refresh)) = self.store.refresh_token(account_id) else {
            return;
        };
        let tokens = match auth::refresh(
            &self.http,
            &self.config.client_id,
            &refresh,
            auth::CONTACTS_SCOPES,
        ) {
            Ok(t) => t,
            Err(e) => {
                log::info!("address book not available: {e}");
                return;
            }
        };
        match Graph::new(&self.http, &tokens.access).contacts() {
            Ok(list) => {
                if let Err(e) = self.store.replace_contacts(account_id, &list) {
                    log::warn!("address book not stored: {e}");
                }
            }
            Err(e) => log::info!("address book not read: {e}"),
        }
    }

    fn save_draft(
        &mut self,
        account_id: i64,
        base: &Compose,
        draft_id: Option<String>,
        out: &Outgoing,
    ) -> Result<String, String> {
        let quoted = draft_id.as_ref().and_then(|d| self.quoted.get(d).cloned());
        let (id, quoted) = self.with_graph(account_id, |g| {
            g.save_draft(draft_id.as_deref(), base.base(), out, quoted.as_deref())
        })?;
        self.quoted.insert(id.clone(), quoted);
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    fn send_message(
        &mut self,
        account_id: i64,
        base: &Compose,
        draft_id: Option<String>,
        out: &Outgoing,
        attachments: &[PathBuf],
        composer: u64,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        // Read the files first: a missing one stops the send before anything reaches Microsoft.
        let mut files = Vec::new();
        for path in attachments {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path.file_name().map_or_else(
                || "piece-jointe".into(),
                |n| n.to_string_lossy().into_owned(),
            );
            files.push((name, bytes));
        }
        let id = self.save_draft(account_id, base, draft_id, out)?;
        // The draft exists from here on: if a later step fails, the composer keeps its id.
        emit(Event::DraftSaved {
            composer,
            draft_id: id.clone(),
        });
        for (name, bytes) in &files {
            self.with_graph(account_id, |g| g.attach(&id, name, bytes))?;
        }
        self.with_graph(account_id, |g| g.send_draft(&id))?;
        self.quoted.remove(&id);
        Ok(())
    }

    fn sync_folders(
        &mut self,
        account_id: i64,
        folders: &[Folder],
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        // Well-known folders first (inbox before everything), then the others.
        let mut order: Vec<&Folder> = folders.iter().collect();
        order.sort_by_key(|f| {
            f.role
                .as_deref()
                .and_then(|r| ROLES.iter().position(|x| *x == r))
                .unwrap_or(ROLES.len())
        });
        for folder in order {
            self.sync_folder(account_id, folder, emit)?;
        }
        Ok(())
    }

    fn sync_folder(
        &mut self,
        account_id: i64,
        folder: &Folder,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        let since = unix_now() - i64::from(self.config.months) * 30 * 24 * 3600;
        let start = Graph::delta_start(&folder.id, since);
        let mut url = self
            .store
            .delta_link(account_id, &folder.id)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| start.clone());
        let mut count = 0;
        let mut reset = false;
        loop {
            let page = match self.with_graph(account_id, |g| match g.delta_page(&url) {
                Err(GraphError::SyncReset) => Ok(None),
                r => r.map(Some),
            })? {
                Some(p) => p,
                None if !reset => {
                    reset = true;
                    url = start.clone();
                    continue;
                }
                None => return Err(format!("{}: sync state keeps expiring", folder.name)),
            };
            let upserts: Vec<MessageSummary> = page
                .messages
                .into_iter()
                .map(|m| summary(account_id, &folder.id, m))
                .collect();
            count += upserts.len();
            self.store
                .apply_changes(account_id, &folder.id, &upserts, &page.removed)
                .map_err(|e| e.to_string())?;
            if !upserts.is_empty() || !page.removed.is_empty() {
                emit(Event::Syncing {
                    folder: folder.name.clone(),
                    messages: count,
                });
                emit(Event::Changed);
            }
            match (page.next, page.delta_link) {
                (Some(next), _) => url = next,
                (None, link) => {
                    self.store
                        .set_delta_link(account_id, &folder.id, link.as_deref())
                        .map_err(|e| e.to_string())?;
                    return Ok(());
                }
            }
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn summary(account_id: i64, folder_id: &str, m: mailcraft_graph::RemoteMessage) -> MessageSummary {
    MessageSummary {
        account_id,
        id: m.id,
        folder_id: folder_id.to_owned(),
        from_name: m.from_name,
        from_addr: m.from_addr,
        subject: m.subject,
        preview: m.preview,
        received: m.received,
        is_read: m.is_read,
        has_attachments: m.has_attachments,
        flagged: m.flagged,
    }
}

/// Largest image MailCraft loads into a message (remote or inline).
pub const MAX_IMAGE: usize = 5 * 1024 * 1024;

/// Client for remote images: follows redirects only to public hosts.
fn image_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("MailCraft/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let local = attempt.url().host().is_none_or(is_local_host);
            if local || attempt.previous().len() >= 5 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// Downloads a remote image of a message: http(s) only, never to this machine or the local
/// network (a mail must not make MailCraft probe the LAN), 5 MB at most.
fn fetch_image(http: &Client, url: &str) -> Option<Vec<u8>> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || is_local_host(parsed.host()?) {
        return None;
    }
    let mut resp = http
        .get(parsed)
        .timeout(Duration::from_secs(20))
        .send()
        .ok()?;
    if !resp.status().is_success() || resp.content_length().is_some_and(|n| n > MAX_IMAGE as u64) {
        return None;
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(&mut resp, MAX_IMAGE as u64 + 1),
        &mut bytes,
    )
    .ok()?;
    (bytes.len() <= MAX_IMAGE).then_some(bytes)
}

fn is_local_host(host: url::Host<&str>) -> bool {
    use std::net::IpAddr;
    let ip = match host {
        url::Host::Domain(d) => {
            let d = d.to_ascii_lowercase();
            return d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || !d.contains('.');
        }
        url::Host::Ipv4(v4) => IpAddr::V4(v4),
        url::Host::Ipv6(v6) => IpAddr::V6(v6),
    };
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_local_host;

    #[test]
    fn local_hosts_are_refused() {
        let local = |s: &str| is_local_host(url::Url::parse(s).unwrap().host().unwrap());
        for url in [
            "http://localhost/x",
            "http://127.0.0.1/x",
            "http://192.168.1.10/x",
            "http://10.0.0.1/x",
            "http://[::1]/x",
            "http://nas/x",
            "http://printer.local/x",
        ] {
            assert!(local(url), "{url}");
        }
        assert!(!local("https://cdn.example.com/logo.png"));
        assert!(!local("https://93.184.215.14/logo.png"));
    }
}

/// What a draft starts from (owned, to cross the channel).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compose {
    New,
    Reply(String),
    ReplyAll(String),
    Forward(String),
}

impl Compose {
    fn base(&self) -> Base<'_> {
        match self {
            Self::New => Base::New,
            Self::Reply(m) => Base::Reply(m),
            Self::ReplyAll(m) => Base::ReplyAll(m),
            Self::Forward(m) => Base::Forward(m),
        }
    }
}
