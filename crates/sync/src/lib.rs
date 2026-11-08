//! Background worker: everything that talks to the network runs here, on its own thread with
//! its own database connection. The UI sends [`Request`]s and reads [`Event`]s, then re-reads
//! the store.

mod automation;
mod google;
mod imap_side;

pub use automation::{Away, Rule, RuleAction};
pub use google::configured as google_configured;

use std::collections::{HashMap, HashSet};
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
/// Full sync of every folder of every account.
const SYNC_EVERY: Duration = Duration::from_secs(600);
/// Inboxes that the server does not push (Microsoft, IMAP without IDLE): checked this often.
const QUICK_EVERY: Duration = Duration::from_secs(20);

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
    /// Inboxes only, for new mail within seconds (the worker sends it to itself).
    QuickSync,
    /// The IDLE watch of this IMAP account stopped for good: its inbox is polled instead.
    IdleStopped {
        account_id: i64,
    },
    /// The server said the inbox of this IMAP account changed (IDLE): sync it now.
    SyncInbox {
        account_id: i64,
    },
    /// The user's rules and out-of-office reply, as set in the app (no server change).
    SetAutomation {
        rules: Vec<Rule>,
        away: Away,
    },
    /// Writes each message as an .eml file in `dir`: (message id, file name).
    Export {
        account_id: i64,
        items: Vec<(String, String)>,
        dir: PathBuf,
    },
    /// The out-of-office reply was changed: set it on Microsoft servers too.
    SetAway {
        away: Away,
    },
    /// Gmail through the browser sign-in (only when [`google_configured`]).
    AddGoogleAccount,
    /// Checks the IMAP and SMTP sign-in, then adds the account and syncs it.
    AddImapAccount {
        server: Box<mailcraft_imap::Server>,
    },
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
    /// The chosen AI classifies senders not classified yet (all with `force`).
    ClassifySenders {
        account_id: i64,
        force: bool,
        ai: mailcraft_ai::Choice,
    },
    /// The chosen AI summarises the newsletters of the last 24 hours.
    Digest {
        account_id: i64,
        ai: mailcraft_ai::Choice,
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
    /// Reads the headers of each sender's latest message (only senders not analysed yet,
    /// or all with `force`) and stores what the local analysis finds.
    AnalyzeSenders {
        account_id: i64,
        force: bool,
    },
    /// A clean-up action on senders. `dry_run` only logs what would happen. `protected` lists
    /// addresses and domains never to touch (the allow-list), checked again here.
    Cleanup {
        account_id: i64,
        addresses: Vec<String>,
        action: CleanAction,
        dry_run: bool,
        protected: Vec<String>,
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
    Analyzing {
        done: usize,
        total: usize,
    },
    AnalysisDone,
    /// Clean-up finished: actions done (or simulated) and senders skipped by a safeguard.
    CleanupDone {
        done: usize,
        skipped: usize,
        dry_run: bool,
    },
    /// An unsubscribe that only has a web page: the UI opens it for the user to confirm.
    OpenPage {
        url: String,
    },
    Classifying {
        done: usize,
        total: usize,
    },
    ClassifyDone {
        count: usize,
    },
    /// The newsletter digest, ready to show.
    DigestReady {
        text: String,
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
    Exported {
        count: usize,
        dir: PathBuf,
    },
    Error(String),
}

pub struct Worker {
    tx: Sender<Request>,
    /// The reader: a second worker with its own connections, for what the user opens.
    reads: Sender<Request>,
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    /// Starts the threads: the worker (sync, changes, sending) and the reader (message bodies,
    /// images, attachments, export), each with its own store connection and server sessions,
    /// so opening a message never waits behind a sync. `wake` is called after each event so the
    /// UI repaints.
    pub fn start(config: Config, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (reads, read_requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let spawn =
            |name: &str, requests: Receiver<Request>, cancel: Arc<AtomicBool>, reader: bool| {
                let (config, events, wake, again) =
                    (config.clone(), events.clone(), wake.clone(), tx.clone());
                std::thread::Builder::new()
                    .name(name.into())
                    .spawn(move || {
                        let side = (events.clone(), wake.clone());
                        let emit = |e: Event| {
                            let _ = events.send(e);
                            wake();
                        };
                        match Runner::new(config, cancel, side, again) {
                            Ok(mut runner) if reader => runner.serve(&requests, &emit),
                            Ok(mut runner) => runner.run(requests, &emit),
                            Err(e) => emit(Event::Error(e)),
                        }
                    })
                    .ok();
            };
        spawn("mailcraft-sync", requests, cancel.clone(), false);
        spawn(
            "mailcraft-read",
            read_requests,
            Arc::new(AtomicBool::new(false)),
            true,
        );
        Self {
            tx,
            reads,
            rx,
            cancel,
        }
    }

    /// What the reader handles: reading only, nothing that changes the mailbox.
    fn is_read(request: &Request) -> bool {
        matches!(
            request,
            Request::FetchBody { .. }
                | Request::FetchInline { .. }
                | Request::FetchImage { .. }
                | Request::SaveAttachment { .. }
                | Request::Export { .. }
        )
    }

    pub fn send(&self, request: Request) {
        if Self::is_read(&request) {
            let _ = self.reads.send(request);
            return;
        }
        if matches!(
            request,
            Request::AddMicrosoftAccount | Request::AddGoogleAccount
        ) {
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
    /// Open IMAP connections, one per account.
    imap: HashMap<i64, mailcraft_imap::Imap>,
    servers: HashMap<i64, mailcraft_imap::Server>,
    /// Gmail views (All Mail, Starred…): listed, never synced.
    views: HashMap<i64, HashSet<String>>,
    /// Last IMAP message read in full: (id, bytes).
    raw: Option<(String, Vec<u8>)>,
    /// Sends requests to this same worker (from the IDLE threads).
    again: Sender<Request>,
    /// IMAP accounts whose inbox is watched with IDLE.
    watched: HashSet<i64>,
    rules: Vec<Rule>,
    away: Option<Away>,
    /// The request queue, kept here so a long sync can serve what the user waits for.
    requests: Option<Receiver<Request>>,
    /// Requests that arrived during a sync and can wait for its end.
    deferred: std::collections::VecDeque<Request>,
    /// Account id → IMAP or not (asked once per account).
    imap_accounts: std::cell::RefCell<HashMap<i64, bool>>,
    serving: bool,
    /// IMAP folder state from the last folder list ((account, folder) → signature).
    folder_states: HashMap<(i64, String), String>,
}

impl Runner {
    fn new(
        config: Config,
        cancel: Arc<AtomicBool>,
        side: Side,
        again: Sender<Request>,
    ) -> Result<Self, String> {
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
            imap: HashMap::new(),
            servers: HashMap::new(),
            views: HashMap::new(),
            raw: None,
            again,
            watched: HashSet::new(),
            rules: Vec::new(),
            away: None,
            requests: None,
            deferred: std::collections::VecDeque::new(),
            imap_accounts: std::cell::RefCell::new(HashMap::new()),
            serving: false,
            folder_states: HashMap::new(),
        })
    }

    fn run(&mut self, requests: Receiver<Request>, emit: &dyn Fn(Event)) {
        self.requests = Some(requests);
        let mut next_sync = Instant::now();
        let mut next_quick = Instant::now() + QUICK_EVERY;
        loop {
            let request = match self.deferred.pop_front() {
                Some(r) => r,
                None => {
                    let due = next_sync.min(next_quick);
                    let wait = due.saturating_duration_since(Instant::now());
                    let Some(rx) = self.requests.as_ref() else {
                        return;
                    };
                    match rx.recv_timeout(wait) {
                        Ok(r) => r,
                        Err(RecvTimeoutError::Timeout) if next_sync <= next_quick => {
                            Request::SyncAll
                        }
                        Err(RecvTimeoutError::Timeout) => Request::QuickSync,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            };
            match request {
                Request::SyncAll => {
                    next_sync = Instant::now() + SYNC_EVERY;
                    next_quick = Instant::now() + QUICK_EVERY;
                }
                Request::QuickSync => next_quick = Instant::now() + QUICK_EVERY,
                _ => {}
            }
            if let Err(e) = self.handle(request, emit) {
                emit(Event::Error(e));
            }
        }
    }

    /// The reader's loop: one request after the other, no timers, no sync.
    fn serve(&mut self, requests: &Receiver<Request>, emit: &dyn Fn(Event)) {
        for request in requests {
            if let Err(e) = self.handle(request, emit) {
                emit(Event::Error(e));
            }
        }
    }

    /// Changes the user is waiting for: read and flag marks, moves, sending. Served even in the
    /// middle of a sync (reading goes to the reader thread).
    fn urgent(request: &Request) -> bool {
        matches!(
            request,
            Request::SetRead { .. }
                | Request::SetFlag { .. }
                | Request::Move { .. }
                | Request::Delete { .. }
                | Request::SaveDraft { .. }
                | Request::Send { .. }
                | Request::DiscardDraft { .. }
        )
    }

    /// Called between folders and pages of a sync: handles the urgent requests waiting, keeps
    /// the others for after the sync.
    pub(crate) fn serve_urgent(&mut self, emit: &dyn Fn(Event)) {
        if self.serving {
            return;
        }
        self.serving = true;
        let waiting: Vec<Request> = self
            .requests
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for request in waiting {
            if Self::urgent(&request) {
                if let Err(e) = self.handle(request, emit) {
                    emit(Event::Error(e));
                }
            } else {
                self.defer(request);
            }
        }
        self.serving = false;
    }

    /// Keeps a request for after the current work; the same request waiting twice runs once
    /// (IDLE pings during a long sync, a resync asked by several refused changes).
    fn defer(&mut self, request: Request) {
        if !self.deferred.contains(&request) {
            self.deferred.push_back(request);
        }
    }

    fn handle(&mut self, request: Request, emit: &dyn Fn(Event)) -> Result<(), String> {
        let Some(request) = self.handle_imap(request, emit)? else {
            return Ok(());
        };
        match request {
            Request::AddImapAccount { .. } => Ok(()),
            Request::AddGoogleAccount => self.add_google_account(emit),
            Request::SetAutomation { rules, away } => {
                // New rules start from mail arriving now, not from the backlog since the last run.
                if rules != self.rules || Some(&away) != self.away.as_ref() {
                    self.store
                        .clear_marks(automation::MARK)
                        .map_err(|e| e.to_string())?;
                }
                self.rules = rules;
                self.away = Some(away);
                Ok(())
            }
            Request::SetAway { away } => self.set_away(Some(away)),
            Request::Export {
                account_id,
                items,
                dir,
            } => {
                let imap = self.is_imap(account_id);
                let mut count = 0;
                for (id, name) in &items {
                    let bytes = if imap {
                        self.with_imap(account_id, |s| s.raw(id))?
                    } else {
                        self.with_graph(account_id, |g| g.mime(id))?
                    };
                    let path = dir.join(name);
                    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                    count += 1;
                }
                emit(Event::Exported { count, dir });
                Ok(())
            }
            Request::SyncInbox { account_id } => {
                // The server announced a change: the inbox is synced even if its state looks known.
                self.imap_sync_inbox(account_id, true, emit)?;
                self.run_automation(account_id, emit)?;
                emit(Event::SyncDone);
                Ok(())
            }
            // Polled from now on; the next full sync tries IDLE again.
            Request::IdleStopped { account_id } => {
                self.watched.remove(&account_id);
                Ok(())
            }
            Request::QuickSync => {
                for account in self.store.accounts().map_err(|e| e.to_string())? {
                    self.quick_sync(account.id, emit)?;
                    self.run_automation(account.id, emit)?;
                    self.serve_urgent(emit);
                }
                emit(Event::SyncDone);
                Ok(())
            }
            Request::AddMicrosoftAccount => self.add_account(emit),
            Request::SyncAll => {
                self.flush_pending(emit)?;
                for account in self.store.accounts().map_err(|e| e.to_string())? {
                    let t = Instant::now();
                    self.sync_account(account.id, emit)?;
                    log::debug!("sync: account {} in {:?}", account.id, t.elapsed());
                    self.run_automation(account.id, emit)?;
                    self.serve_urgent(emit);
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
            Request::AnalyzeSenders { account_id, force } => {
                self.analyze_senders(account_id, force, emit)
            }
            Request::ClassifySenders {
                account_id,
                force,
                ai,
            } => self.classify_senders(account_id, force, &ai, emit),
            Request::Digest { account_id, ai } => {
                let text = self.digest(account_id, &ai)?;
                emit(Event::DigestReady { text });
                Ok(())
            }
            Request::Cleanup {
                account_id,
                addresses,
                action,
                dry_run,
                protected,
            } => {
                let (done, skipped) =
                    self.cleanup(account_id, &addresses, action, dry_run, &protected, emit)?;
                emit(Event::Changed);
                emit(Event::CleanupDone {
                    done,
                    skipped,
                    dry_run,
                });
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
        // Sent at once, even during a sync: a sync reading the server after the change sees it.
        self.flush_pending(emit)
    }

    /// Sends pending changes in order. A network failure keeps them for the next sync; a change
    /// the server refuses is dropped and the folders are synced again so the local copy matches.
    /// Requests about an IMAP account go to its own code. Gives back the others.
    fn handle_imap(
        &mut self,
        request: Request,
        emit: &dyn Fn(Event),
    ) -> Result<Option<Request>, String> {
        let account_id = match &request {
            Request::AddImapAccount { server } => {
                self.add_imap_account(server, emit)?;
                return Ok(None);
            }
            Request::FetchBody { account_id, .. }
            | Request::FetchInline { account_id, .. }
            | Request::SaveAttachment { account_id, .. }
            | Request::CreateFolder { account_id, .. }
            | Request::RenameFolder { account_id, .. }
            | Request::DeleteFolder { account_id, .. }
            | Request::SaveDraft { account_id, .. }
            | Request::Send { account_id, .. }
            | Request::DiscardDraft { account_id, .. }
            | Request::SearchServer { account_id, .. } => *account_id,
            _ => return Ok(Some(request)),
        };
        if !self.is_imap(account_id) {
            return Ok(Some(request));
        }
        match request {
            Request::FetchBody { id, .. } => {
                if self
                    .store
                    .body(account_id, &id)
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    self.imap_fetch_body(account_id, &id)?;
                }
                emit(Event::BodyReady { account_id, id });
            }
            Request::FetchInline { id, .. } => {
                let images = self.imap_inline(account_id, &id)?;
                emit(Event::InlineReady {
                    account_id,
                    id,
                    images,
                });
            }
            Request::SaveAttachment {
                message_id,
                attachment_id,
                path,
                open,
                ..
            } => {
                let bytes = self.imap_attachment(account_id, &message_id, &attachment_id)?;
                std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                if open {
                    open::that_detached(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                }
                emit(Event::AttachmentSaved { path });
            }
            Request::CreateFolder { parent, name, .. } => {
                self.imap_create_folder(account_id, parent.as_deref(), &name)?;
                self.imap_refresh_folders(account_id, emit)?;
            }
            Request::RenameFolder { id, name, .. } => {
                self.imap_rename_folder(account_id, &id, &name)?;
                self.imap_sync_account(account_id, emit)?;
            }
            Request::DeleteFolder { id, .. } => {
                self.imap_delete_folder(account_id, &id)?;
                self.imap_sync_account(account_id, emit)?;
            }
            Request::SaveDraft {
                composer,
                base,
                draft_id,
                out,
                ..
            } => {
                let draft_id = self.imap_save_draft(account_id, &base, draft_id, &out)?;
                emit(Event::DraftSaved { composer, draft_id });
            }
            Request::Send {
                composer,
                base,
                draft_id,
                out,
                attachments,
                ..
            } => match self.imap_send(account_id, &base, draft_id, &out, &attachments) {
                Ok(()) => emit(Event::Sent { composer }),
                Err(error) => emit(Event::SendFailed { composer, error }),
            },
            Request::DiscardDraft { draft_id, .. } => {
                self.imap_discard_draft(account_id, &draft_id)?;
            }
            Request::SearchServer { query, .. } => {
                let found = self.imap_search(account_id, &query)?;
                emit(Event::Changed);
                emit(Event::ServerSearchDone { found });
            }
            other => return Ok(Some(other)),
        }
        Ok(None)
    }

    fn flush_pending(&mut self, emit: &dyn Fn(Event)) -> Result<(), String> {
        let mut refused = Vec::new();
        for (pid, account_id, json) in self.store.pending().map_err(|e| e.to_string())? {
            let Ok(op) = serde_json::from_str::<PendingOp>(&json) else {
                self.store.drop_pending(pid).map_err(|e| e.to_string())?;
                continue;
            };
            let result = if self.is_imap(account_id) {
                match self.imap_apply(account_id, &op) {
                    Ok(()) | Err(mailcraft_imap::Error::NotFound) => Ok(()),
                    Err(mailcraft_imap::Error::Network(e)) => Err((true, e)),
                    Err(e) => Err((false, e.to_string())),
                }
            } else {
                match self.try_graph(account_id, |g| g.apply(&op.op())) {
                    Ok(_) | Err(GraphError::NotFound) => Ok(()),
                    Err(GraphError::Network(e)) => Err((true, e)),
                    Err(e) => Err((false, e.to_string())),
                }
            };
            match result {
                Ok(()) => {}
                Err((true, e)) => {
                    emit(Event::Offline {
                        pending: self.store.pending().map_or(0, |p| p.len()),
                        reason: e,
                    });
                    return Ok(());
                }
                Err((false, e)) => {
                    refused.push(account_id);
                    emit(Event::Error(e));
                }
            }
            self.store.drop_pending(pid).map_err(|e| e.to_string())?;
        }
        // A refused change: the local copy is wrong somewhere, a full sync puts it right. Asked
        // for after the current work, never as a sync inside a sync.
        if !refused.is_empty() {
            self.defer(Request::SyncAll);
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
        if self.is_imap(account_id) {
            return self.imap_refresh_folders(account_id, emit);
        }
        if !self.roles.contains_key(&account_id) {
            // The six well-known folders asked at once (one request each); any failure: asked
            // again one by one, with the token refresh and error handling of with_graph.
            let token = self.access_token(account_id, false)?;
            let http = &self.http;
            let found: Vec<Option<(String, &str)>> = std::thread::scope(|scope| {
                let asks: Vec<_> = ROLES
                    .iter()
                    .map(|role| {
                        let token = token.as_str();
                        scope.spawn(move || {
                            Graph::new(http, token)
                                .well_known_id(role)
                                .ok()
                                .map(|id| id.map(|id| (id, *role)))
                        })
                    })
                    .collect();
                asks.into_iter()
                    .map(|h| h.join().ok().flatten().unwrap_or(None))
                    .collect()
            });
            let mut roles = HashMap::new();
            for (role, got) in ROLES.iter().zip(found) {
                let id = match got {
                    Some((id, _)) => Some(id),
                    None => self.with_graph(account_id, |g| g.well_known_id(role))?,
                };
                if let Some(id) = id {
                    roles.insert(id, (*role).to_owned());
                }
            }
            self.serve_urgent(emit);
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

    /// The inbox only: Microsoft (one delta request when nothing changed) and IMAP accounts
    /// whose server does not push. IMAP inboxes watched with IDLE need nothing.
    fn quick_sync(&mut self, account_id: i64, emit: &dyn Fn(Event)) -> Result<(), String> {
        if self.is_imap(account_id) {
            if self.watched.contains(&account_id) {
                return Ok(());
            }
            // One STATUS when nothing changed (most polls).
            return self.imap_sync_inbox(account_id, false, emit);
        }
        match self.inbox_of(account_id)? {
            Some(f) => self.sync_folder(account_id, &f, None, emit),
            None => Ok(()),
        }
    }

    fn sync_account(&mut self, account_id: i64, emit: &dyn Fn(Event)) -> Result<(), String> {
        if self.is_imap(account_id) {
            return self.imap_sync_account(account_id, emit);
        }
        let folders = self.refresh_folders(account_id, emit)?;
        self.serve_urgent(emit);
        self.sync_folders(account_id, &folders, emit)?;
        if self.contacts_done.insert(account_id) {
            self.sync_contacts(account_id);
        }
        Ok(())
    }

    fn analyze_senders(
        &mut self,
        account_id: i64,
        force: bool,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        // The user's own addresses: mail "from yourself" that fails DMARC is a scam.
        let own: Vec<String> = self
            .store
            .accounts()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| a.email)
            .collect();
        let todo: Vec<(String, String, Option<String>)> = self
            .store
            .senders(account_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|s| force || s.analysis.is_none())
            .map(|s| {
                let kept = s.analysis.map(|a| a.headers).filter(|h| !h.is_empty());
                (s.address, s.name, kept)
            })
            .collect();
        let total = todo.len();
        for (done, (address, name, kept)) in todo.into_iter().enumerate() {
            if done % 10 == 0 {
                emit(Event::Analyzing { done, total });
            }
            // Analysed before: the kept headers are enough, no network needed.
            let headers = match kept {
                Some(text) => parse_kept_headers(&text),
                None => {
                    let Some(id) = self
                        .store
                        .latest_from(account_id, &address)
                        .map_err(|e| e.to_string())?
                    else {
                        continue;
                    };
                    if self.is_imap(account_id) {
                        match self.imap_headers(account_id, &id)? {
                            Some(h) => h,
                            None => continue,
                        }
                    } else {
                        match self.try_graph(account_id, |g| g.headers(&id)) {
                            Ok(h) => h,
                            Err(GraphError::NotFound) => continue,
                            Err(e) => return Err(e.to_string()),
                        }
                    }
                }
            };
            let analysis = analyze(&name, &address, &headers, &own);
            self.store
                .put_sender_analysis(account_id, &address, &analysis)
                .map_err(|e| e.to_string())?;
        }
        emit(Event::Analyzing { done: total, total });
        emit(Event::Changed);
        emit(Event::AnalysisDone);
        Ok(())
    }

    /// Key of the chosen provider; empty for one that needs none (Ollama, a local server).
    fn ai_key(ai: &mailcraft_ai::Choice) -> Result<String, String> {
        let key = mailcraft_store::vault::secret(ai.provider.vault_name())
            .map_err(|e| e.to_string())?
            .filter(|k| !k.trim().is_empty());
        match key {
            Some(k) => Ok(k),
            None if !ai.provider.needs_key() => Ok(String::new()),
            None => Err(format!(
                "no {} API key (File › Options › AI)",
                ai.provider.label()
            )),
        }
    }

    /// Asks the AI about senders, 20 per request; the answer is kept, never asked twice.
    fn classify_senders(
        &mut self,
        account_id: i64,
        force: bool,
        ai: &mailcraft_ai::Choice,
        emit: &dyn Fn(Event),
    ) -> Result<(), String> {
        let key = Self::ai_key(ai)?;
        let todo: Vec<mailcraft_store::SenderRow> = self
            .store
            .senders(account_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|s| force || s.ai.is_none())
            .collect();
        let total = todo.len();
        let mut count = 0;
        for (i, batch) in todo.chunks(mailcraft_ai::BATCH).enumerate() {
            emit(Event::Classifying {
                done: i * mailcraft_ai::BATCH,
                total,
            });
            let facts: Vec<mailcraft_ai::SenderFacts> =
                batch.iter().map(|s| self.facts(account_id, s)).collect();
            let verdicts = mailcraft_ai::Assistant::new(&self.http, &key, ai)
                .classify(&facts)
                .map_err(|e| e.to_string())?;
            for v in verdicts {
                // Only answers about senders that were asked about.
                if !batch
                    .iter()
                    .any(|s| s.address.eq_ignore_ascii_case(&v.adresse))
                {
                    continue;
                }
                let verdict = mailcraft_store::AiVerdict {
                    category: v.categorie,
                    confidence: v.confiance,
                    reason: v.raison,
                    action: v.action_suggeree,
                };
                self.store
                    .put_ai_verdict(account_id, &v.adresse, &verdict)
                    .map_err(|e| e.to_string())?;
                count += 1;
            }
            emit(Event::Changed);
        }
        emit(Event::ClassifyDone { count });
        Ok(())
    }

    /// What Claude is told about a sender: nothing more than needed to judge it.
    fn facts(&self, account_id: i64, s: &mailcraft_store::SenderRow) -> mailcraft_ai::SenderFacts {
        let check = |v: i64| match v {
            1 => "pass",
            2 => "fail",
            _ => "inconnu",
        };
        let a = s.analysis.clone().unwrap_or_default();
        let samples = self
            .store
            .sender_samples(account_id, &s.address, 3)
            .unwrap_or_default();
        mailcraft_ai::SenderFacts {
            adresse: s.address.clone(),
            nom: mailcraft_ai::short(&s.name, 80),
            domaine: mailcraft_analysis::domain_of(&s.address),
            spf: check(a.spf).into(),
            dkim: check(a.dkim).into(),
            dmarc: check(a.dmarc).into(),
            alertes: a
                .warnings
                .split(',')
                .filter(|w| !w.is_empty())
                .map(str::to_owned)
                .collect(),
            desabonnement_possible: a.one_click.is_some() || a.mailto.is_some() || a.web.is_some(),
            nombre_de_mails: s.count,
            objets: samples
                .iter()
                .map(|(sub, _)| mailcraft_ai::short(sub, 150))
                .collect(),
            extraits: samples
                .iter()
                .map(|(_, pre)| mailcraft_ai::short(pre, 150))
                .collect(),
        }
    }

    /// Summary of the last 24 hours of newsletters (by the local analysis or Claude).
    fn digest(&mut self, account_id: i64, ai: &mailcraft_ai::Choice) -> Result<String, String> {
        let key = Self::ai_key(ai)?;
        let newsletters: std::collections::HashSet<String> = self
            .store
            .senders(account_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|s| {
                let ai = s.ai.as_ref().map(|v| v.category.as_str());
                matches!(ai, Some("newsletter" | "promo"))
                    || (ai.is_none()
                        && s.analysis
                            .as_ref()
                            .is_some_and(|a| a.category == "newsletter"))
            })
            .map(|s| s.address.to_lowercase())
            .collect();
        let items: Vec<(String, String, String)> = self
            .store
            .received_since(account_id, unix_now() - 24 * 3600)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|m| newsletters.contains(&m.from_addr.to_lowercase()))
            .take(40)
            .map(|m| {
                let from = if m.from_name.is_empty() {
                    m.from_addr
                } else {
                    m.from_name
                };
                (
                    mailcraft_ai::short(&from, 60),
                    mailcraft_ai::short(&m.subject, 150),
                    mailcraft_ai::short(&m.preview, 200),
                )
            })
            .collect();
        if items.is_empty() {
            return Ok(String::new());
        }
        mailcraft_ai::Assistant::new(&self.http, &key, ai)
            .digest(&items)
            .map_err(|e| e.to_string())
    }

    /// Folder id of a well-known folder (`deleteditems`, `archive`, `junkemail`).
    fn role_folder(
        &mut self,
        account_id: i64,
        role: &str,
        emit: &dyn Fn(Event),
    ) -> Result<String, String> {
        if !self.roles.contains_key(&account_id) {
            self.refresh_folders(account_id, emit)?;
        }
        self.roles
            .get(&account_id)
            .and_then(|r| r.iter().find(|(_, v)| *v == role).map(|(id, _)| id.clone()))
            .ok_or_else(|| format!("no {role} folder in this mailbox"))
    }

    /// Runs (or simulates) a clean-up action on each sender. Returns (done, skipped).
    fn cleanup(
        &mut self,
        account_id: i64,
        addresses: &[String],
        action: CleanAction,
        dry_run: bool,
        protected: &[String],
        emit: &dyn Fn(Event),
    ) -> Result<(usize, usize), String> {
        let senders = self.store.senders(account_id).map_err(|e| e.to_string())?;
        let (mut done, mut skipped) = (0, 0);
        for address in addresses {
            let log = |store: &Store, kind: &str, detail: String, ok: bool| {
                let _ = store.log_action(&mailcraft_store::ActionEntry {
                    at: unix_now(),
                    account_id,
                    address: address.clone(),
                    action: kind.to_owned(),
                    detail,
                    dry_run,
                    ok,
                });
            };
            if is_protected(address, protected) {
                log(
                    &self.store,
                    action.code(),
                    "allow-list: left untouched".into(),
                    false,
                );
                skipped += 1;
                continue;
            }
            let row = senders
                .iter()
                .find(|s| s.address.eq_ignore_ascii_case(address));
            match action {
                CleanAction::Unsubscribe => {
                    let Some(row) = row else { continue };
                    if let Some(reason) = unsubscribe_refusal(row) {
                        log(&self.store, "unsubscribe", reason.into(), false);
                        skipped += 1;
                        continue;
                    }
                    let a = row.analysis.clone().unwrap_or_default();
                    let (detail, result) = if let Some(url) = &a.one_click {
                        let r = if dry_run {
                            Ok(())
                        } else {
                            one_click(&self.images, url)
                        };
                        (format!("one-click POST {url}"), r)
                    } else if let Some((to, subject)) = a
                        .mailto
                        .as_deref()
                        .and_then(mailcraft_analysis::mailto_parts)
                    {
                        let r = if dry_run {
                            Ok(())
                        } else if self.is_imap(account_id) {
                            self.imap_send_plain(account_id, &to, &subject, "unsubscribe")
                        } else {
                            self.with_graph(account_id, |g| {
                                g.send_plain(&to, &subject, "unsubscribe")
                            })
                        };
                        (format!("e-mail to {to} « {subject} »"), r)
                    } else if let Some(url) = &a.web {
                        if !dry_run {
                            emit(Event::OpenPage { url: url.clone() });
                        }
                        (format!("page opened for you to confirm: {url}"), Ok(()))
                    } else {
                        log(
                            &self.store,
                            "unsubscribe",
                            "no unsubscribe link in its mail".into(),
                            false,
                        );
                        skipped += 1;
                        continue;
                    };
                    let ok = result.is_ok();
                    log(
                        &self.store,
                        "unsubscribe",
                        result
                            .err()
                            .map_or(detail.clone(), |e| format!("{detail}: {e}")),
                        ok,
                    );
                    if ok {
                        done += 1;
                        if !dry_run {
                            self.store
                                .mark_unsubscribed(account_id, address, unix_now())
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
                CleanAction::DeleteAll | CleanAction::ArchiveAll | CleanAction::Block => {
                    let role = match action {
                        CleanAction::DeleteAll => "deleteditems",
                        CleanAction::ArchiveAll => "archive",
                        _ => "junkemail",
                    };
                    let dest = self.role_folder(account_id, role, emit)?;
                    let imap = self.is_imap(account_id);
                    let found = if imap {
                        self.imap_messages_from(account_id, address)?
                    } else {
                        self.with_graph(account_id, |g| g.messages_from(address))?
                    };
                    let ids: Vec<String> = found
                        .into_iter()
                        .filter(|(_, folder)| *folder != dest)
                        .map(|(id, _)| id)
                        .collect();
                    if !dry_run {
                        for id in &ids {
                            if imap {
                                let op = PendingOp::Move {
                                    id: id.clone(),
                                    dest: dest.clone(),
                                };
                                match self.imap_apply(account_id, &op) {
                                    Ok(()) | Err(mailcraft_imap::Error::NotFound) => {}
                                    Err(e) => return Err(e.to_string()),
                                }
                                continue;
                            }
                            match self
                                .try_graph(account_id, |g| g.apply(&Op::Move { id, dest: &dest }))
                            {
                                Ok(_) | Err(GraphError::NotFound) => {}
                                Err(e) => return Err(e.to_string()),
                            }
                        }
                        self.store
                            .move_local(account_id, &ids, &dest)
                            .map_err(|e| e.to_string())?;
                        if action == CleanAction::Block {
                            // IMAP has no server rules: MailCraft files their new mail at sync.
                            if !imap {
                                self.with_graph(account_id, |g| g.block_rule(address, &dest))?;
                            }
                            self.store
                                .mark_blocked(account_id, address)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    let rule = if action == CleanAction::Block {
                        " + rule for new mail"
                    } else {
                        ""
                    };
                    log(
                        &self.store,
                        action.code(),
                        format!("{} message(s) → {role}{rule}", ids.len()),
                        true,
                    );
                    done += 1;
                }
            }
        }
        Ok((done, skipped))
    }

    /// Reads the address book for completion. Needs Contacts.Read, which accounts added before
    /// it was asked for do not have: then completion uses known senders only, silently.
    fn sync_contacts(&mut self, account_id: i64) {
        if self.is_imap(account_id) {
            return;
        }
        let Ok(Some(refresh)) = self.store.refresh_token(account_id) else {
            return;
        };
        // Read-write first; accounts signed in before it was asked for still have read-only.
        let tokens = match auth::refresh(
            &self.http,
            &self.config.client_id,
            &refresh,
            auth::CONTACTS_SCOPES,
        )
        .or_else(|_| {
            auth::refresh(
                &self.http,
                &self.config.client_id,
                &refresh,
                auth::CONTACTS_READ_SCOPES,
            )
        }) {
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
        let mut first = self.first_pages(account_id, &order);
        for folder in order {
            let page = first.remove(&folder.id);
            self.sync_folder(account_id, folder, page, emit)?;
            self.serve_urgent(emit);
        }
        Ok(())
    }

    /// The first delta page of every folder, asked four at a time: when nothing changed it is
    /// the only request a folder needs (about 0.5 s each, one after the other before). A folder
    /// whose request fails is left out and synced the usual way.
    fn first_pages(
        &mut self,
        account_id: i64,
        folders: &[&Folder],
    ) -> HashMap<String, mailcraft_graph::DeltaPage> {
        const LANES: usize = 4;
        let since = unix_now() - i64::from(self.config.months) * 30 * 24 * 3600;
        let urls: Vec<(String, String)> = folders
            .iter()
            .map(|f| {
                let url = self
                    .store
                    .delta_link(account_id, &f.id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| Graph::delta_start(&f.id, since));
                (f.id.clone(), url)
            })
            .collect();
        let Ok(token) = self.access_token(account_id, false) else {
            return HashMap::new();
        };
        let (http, token, urls) = (&self.http, token.as_str(), &urls);
        std::thread::scope(|scope| {
            let lanes: Vec<_> = (0..LANES)
                .map(|lane| {
                    scope.spawn(move || {
                        let graph = Graph::new(http, token);
                        urls.iter()
                            .skip(lane)
                            .step_by(LANES)
                            .filter_map(|(id, url)| {
                                graph.delta_page(url).ok().map(|p| (id.clone(), p))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            lanes
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        })
    }

    /// Syncs one folder through its delta pages; `first`: its first page, already fetched.
    fn sync_folder(
        &mut self,
        account_id: i64,
        folder: &Folder,
        mut first: Option<mailcraft_graph::DeltaPage>,
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
            let fetched = match first.take() {
                Some(page) => Some(page),
                None => self.with_graph(account_id, |g| match g.delta_page(&url) {
                    Err(GraphError::SyncReset) => Ok(None),
                    r => r.map(Some),
                })?,
            };
            let page = match fetched {
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
                (Some(next), _) => {
                    url = next;
                    self.serve_urgent(emit);
                }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanAction {
    Unsubscribe,
    DeleteAll,
    ArchiveAll,
    /// Junk folder now and a rule for new mail.
    Block,
}

impl CleanAction {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsubscribe => "unsubscribe",
            Self::DeleteAll => "delete-all",
            Self::ArchiveAll => "archive-all",
            Self::Block => "block",
        }
    }
}

/// Allow-list entries are addresses (`a@b.fr`) or domains (`b.fr`, which also covers
/// `news.b.fr`).
pub fn is_protected(address: &str, protected: &[String]) -> bool {
    let address = address.to_ascii_lowercase();
    let domain = mailcraft_analysis::domain_of(&address);
    protected
        .iter()
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .any(|p| {
            if p.contains('@') {
                p == address
            } else {
                domain == p || domain.ends_with(&format!(".{p}"))
            }
        })
}

/// Why MailCraft will not unsubscribe from this sender automatically, if it will not.
pub fn unsubscribe_refusal(row: &mailcraft_store::SenderRow) -> Option<&'static str> {
    let analysis = row.analysis.as_ref()?;
    if !analysis.warnings.is_empty() || analysis.category == "suspect" {
        return Some("suspect sender: clicking its links could confirm your address to a scammer");
    }
    if row.blocked {
        return Some("sender already blocked");
    }
    match &row.ai {
        Some(ai) if matches!(ai.category.as_str(), "suspect" | "phishing") => {
            Some("the AI rates it suspect or phishing")
        }
        Some(ai) if ai.confidence < 0.8 => Some("the AI is not confident enough (below 0.8)"),
        _ => None,
    }
}

/// Headers of a sender's latest message → what MailCraft stores about it.
/// Headers the analysis reads, kept to analyse again without the network.
const LINE_BREAKS: [char; 2] = ['\r', '\n'];
const KEPT_HEADERS: [&str; 3] = [
    "List-Unsubscribe",
    "List-Unsubscribe-Post",
    "Authentication-Results",
];

/// Back from the stored "Name: value" lines.
pub fn parse_kept_headers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            l.split_once(": ")
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
        })
        .collect()
}

pub fn analyze(
    name: &str,
    address: &str,
    headers: &[(String, String)],
    own: &[String],
) -> mailcraft_store::SenderAnalysis {
    use mailcraft_analysis::{Check, authentication, category, unsubscribe, warnings};
    let get = |n: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_str())
    };
    let auth_headers: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("Authentication-Results"))
        .map(|(_, v)| v.as_str())
        .collect();
    let auth = authentication(&auth_headers);
    let unsub = unsubscribe(get("List-Unsubscribe"), get("List-Unsubscribe-Post"));
    let warns = warnings(name, address, auth, own);
    let code = |c: Check| match c {
        Check::Pass => 1,
        Check::Fail => 2,
        Check::Unknown => 0,
    };
    let cat = match category(&unsub, &warns) {
        mailcraft_analysis::Category::Suspect => "suspect",
        mailcraft_analysis::Category::Newsletter => "newsletter",
        mailcraft_analysis::Category::Other => "other",
    };
    mailcraft_store::SenderAnalysis {
        one_click: unsub.one_click,
        mailto: unsub.mailto,
        web: unsub.web,
        spf: code(auth.spf),
        dkim: code(auth.dkim),
        dmarc: code(auth.dmarc),
        warnings: warns.iter().map(warning_code).collect::<Vec<_>>().join(","),
        category: cat.to_owned(),
        analyzed_at: unix_now(),
        headers: headers
            .iter()
            .filter(|(k, _)| KEPT_HEADERS.iter().any(|h| k.eq_ignore_ascii_case(h)))
            .map(|(k, v)| format!("{k}: {}", v.replace(LINE_BREAKS, " ")))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn warning_code(w: &mailcraft_analysis::Warning) -> String {
    use mailcraft_analysis::Warning;
    match w {
        Warning::Impersonation { brand } => format!("impersonation:{brand}"),
        Warning::Lookalike { brand } => format!("lookalike:{brand}"),
        Warning::Punycode => "punycode".into(),
        Warning::DmarcFail => "dmarc-fail".into(),
        Warning::AuthFail => "auth-fail".into(),
        Warning::OwnAddress => "own-address".into(),
        Warning::InvalidDomain => "invalid-domain".into(),
        Warning::RandomDomain => "random-domain".into(),
        Warning::DisplayDomain { shown } => format!("display-domain:{shown}"),
    }
}

/// RFC 8058 one-click unsubscribe: POST `List-Unsubscribe=One-Click` to the https link, never to
/// this machine or the local network.
fn one_click(http: &Client, url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https" || parsed.host().is_none_or(is_local_host) {
        return Err("refused: not a public https address".into());
    }
    let resp = http
        .post(parsed)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .timeout(Duration::from_secs(20))
        .send()
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() || resp.status().is_redirection() {
        Ok(())
    } else {
        Err(format!("server answered {}", resp.status()))
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::{analyze, is_protected, unsubscribe_refusal};
    use mailcraft_store::{AiVerdict, SenderRow};

    fn h(k: &str, v: &str) -> (String, String) {
        (k.into(), v.into())
    }

    #[test]
    fn analysis_from_headers() {
        let a = analyze(
            "Shop News",
            "news@shop.example",
            &[
                h(
                    "List-Unsubscribe",
                    "<https://shop.example/u?x=1>, <mailto:u@shop.example>",
                ),
                h("list-unsubscribe-post", "List-Unsubscribe=One-Click"),
                h(
                    "Authentication-Results",
                    "spf=pass smtp.mailfrom=shop.example; dkim=pass; dmarc=pass",
                ),
                h("Received", "from somewhere"),
            ],
            &[],
        );
        assert_eq!(a.one_click.as_deref(), Some("https://shop.example/u?x=1"));
        // Only the headers the analysis reads are kept, and they parse back to the same result.
        assert!(!a.headers.contains("Received"));
        let again = analyze(
            "Shop News",
            "news@shop.example",
            &super::parse_kept_headers(&a.headers),
            &[],
        );
        assert_eq!(
            (again.one_click, again.spf, again.dmarc),
            (a.one_click.clone(), a.spf, a.dmarc)
        );
        assert_eq!(
            (a.spf, a.dkim, a.dmarc, a.category.as_str()),
            (1, 1, 1, "newsletter")
        );
        assert!(a.warnings.is_empty());

        let p = analyze(
            "PayPal",
            "x@pay-secure.example",
            &[h("Authentication-Results", "dmarc=fail")],
            &[],
        );
        assert_eq!(
            (p.category.as_str(), p.warnings.as_str()),
            ("suspect", "impersonation:paypal,dmarc-fail")
        );
    }

    #[test]
    fn allow_list_and_refusals() {
        let list = vec![
            "impots.gouv.fr".to_string(),
            "boss@work.example".to_string(),
        ];
        assert!(is_protected("no-reply@dgfip.impots.gouv.fr", &list));
        assert!(is_protected("BOSS@work.example", &list));
        assert!(!is_protected("other@work.example", &list));
        assert!(!is_protected("x@notimpots.gouv.fr", &list));

        let mut row = SenderRow {
            address: "news@shop.example".into(),
            name: "Shop".into(),
            count: 3,
            unread: 1,
            last: 0,
            analysis: Some(analyze(
                "Shop",
                "news@shop.example",
                &[h("List-Unsubscribe", "<mailto:u@shop.example>")],
                &[],
            )),
            unsubscribed_at: None,
            blocked: false,
            ai: None,
            still_writing: false,
        };
        assert_eq!(unsubscribe_refusal(&row), None);
        row.ai = Some(AiVerdict {
            category: "promo".into(),
            confidence: 0.6,
            ..Default::default()
        });
        assert!(unsubscribe_refusal(&row).is_some());
        row.ai = None;
        row.analysis = Some(analyze("PayPal", "x@pay-secure.example", &[], &[]));
        assert!(unsubscribe_refusal(&row).unwrap().starts_with("suspect"));
    }
}
