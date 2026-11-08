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
use mailcraft_graph::{Graph, GraphError};
use mailcraft_store::{Folder, MasterKey, MessageBody, MessageSummary, ROLES, Store};
use reqwest::blocking::Client;

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
    FetchBody { account_id: i64, id: String },
    MarkRead { account_id: i64, id: String },
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
    Error(String),
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    /// Starts the thread. `wake` is called after each event so the UI repaints.
    pub fn start(config: Config, wake: Box<dyn Fn() + Send>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("mailcraft-sync".into())
            .spawn(move || {
                let emit = |e: Event| {
                    let _ = events.send(e);
                    wake();
                };
                match Runner::new(config, flag) {
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

struct Runner {
    store: Store,
    http: Client,
    config: Config,
    cancel: Arc<AtomicBool>,
    tokens: HashMap<i64, Tokens>,
    roles: HashMap<i64, HashMap<String, String>>,
}

impl Runner {
    fn new(config: Config, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let store = Store::open(&config.db_path, config.key.clone(), config.encrypt_new)
            .map_err(|e| e.to_string())?;
        let http = mailcraft_graph::http_client()?;
        Ok(Self {
            store,
            http,
            config,
            cancel,
            tokens: HashMap::new(),
            roles: HashMap::new(),
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
                    let body = MessageBody {
                        text: b.text,
                        to: b.to,
                        cc: b.cc,
                    };
                    self.store
                        .put_body(account_id, &id, &body)
                        .map_err(|e| e.to_string())?;
                }
                emit(Event::BodyReady { account_id, id });
                Ok(())
            }
            Request::MarkRead { account_id, id } => {
                self.store
                    .set_read(account_id, &id, true)
                    .map_err(|e| e.to_string())?;
                emit(Event::Changed);
                self.with_graph(account_id, |g| g.set_read(&id, true))
            }
        }
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
        let tokens = auth::refresh(&self.http, &self.config.client_id, &refresh)
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
        let token = self.access_token(account_id, false)?;
        match f(&Graph::new(&self.http, &token)) {
            Err(GraphError::Unauthorized) => {
                let token = self.access_token(account_id, true)?;
                f(&Graph::new(&self.http, &token)).map_err(|e| e.to_string())
            }
            r => r.map_err(|e| e.to_string()),
        }
    }

    fn sync_account(&mut self, account_id: i64, emit: &dyn Fn(Event)) -> Result<(), String> {
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
                .map(|m| MessageSummary {
                    account_id,
                    id: m.id,
                    folder_id: folder.id.clone(),
                    from_name: m.from_name,
                    from_addr: m.from_addr,
                    subject: m.subject,
                    preview: m.preview,
                    received: m.received,
                    is_read: m.is_read,
                    has_attachments: m.has_attachments,
                    flagged: m.flagged,
                })
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
