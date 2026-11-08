//! IMAP + SMTP accounts: Gmail, Yahoo, iCloud, Orange, Free, La Poste… or any provider, signed
//! in with an app password. TLS goes through rustls on ring (no OpenSSL, no native-tls).
//!
//! A message id is `<uidvalidity>:<uid>:<mailbox>`: it names the message on the server without a
//! lookup, and changes when the server renumbers the folder or the message moves.

pub mod mime;
pub mod smtp;
pub mod utf7;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use imap::types::Flag;
use imap_proto::NameAttribute;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use serde::{Deserialize, Serialize};

pub const PROVIDER_IMAP: &str = "imap";
const TIMEOUT: Duration = Duration::from_secs(60);
/// UIDs per FETCH command: keeps command lines short and progress visible.
const CHUNK: usize = 200;
/// Bytes of text fetched for the preview line.
const PREVIEW_BYTES: usize = 2048;

/// Everything needed to reach the account. Stored sealed (password included) in the store.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct Server {
    pub email: String,
    pub name: String,
    pub user: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// Signed in through the browser (Google): `password` then carries a short-lived access
    /// token at connection time, and `refresh` the long-lived token kept sealed.
    #[serde(default)]
    pub oauth: bool,
    #[serde(default)]
    pub refresh: String,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("email", &self.email)
            .field(
                "imap",
                &format_args!("{}:{}", self.imap_host, self.imap_port),
            )
            .field(
                "smtp",
                &format_args!("{}:{}", self.smtp_host, self.smtp_port),
            )
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Gmail files sent mail itself and archives with "All Mail".
    pub fn is_gmail(&self) -> bool {
        self.imap_host.eq_ignore_ascii_case("imap.gmail.com")
    }
}

/// Known servers for an address: (IMAP host, SMTP host, SMTP port). IMAP is always 993.
pub fn preset(email: &str) -> Option<(&'static str, &'static str, u16)> {
    let domain = email.rsplit_once('@')?.1.to_ascii_lowercase();
    Some(match domain.as_str() {
        "gmail.com" | "googlemail.com" => ("imap.gmail.com", "smtp.gmail.com", 465),
        "icloud.com" | "me.com" | "mac.com" => ("imap.mail.me.com", "smtp.mail.me.com", 587),
        "orange.fr" | "wanadoo.fr" => ("imap.orange.fr", "smtp.orange.fr", 465),
        "free.fr" => ("imap.free.fr", "smtp.free.fr", 465),
        "laposte.net" => ("imap.laposte.net", "smtp.laposte.net", 465),
        "sfr.fr" | "neuf.fr" => ("imap.sfr.fr", "smtp.sfr.fr", 465),
        "gmx.fr" | "gmx.com" | "gmx.net" | "gmx.de" => ("imap.gmx.net", "mail.gmx.net", 587),
        "aol.com" | "aol.fr" => ("imap.aol.com", "smtp.aol.com", 465),
        "zoho.com" | "zohomail.eu" => ("imap.zoho.eu", "smtp.zoho.eu", 465),
        d if d.starts_with("yahoo.") || d.starts_with("ymail.") => {
            ("imap.mail.yahoo.com", "smtp.mail.yahoo.com", 465)
        }
        _ => return None,
    })
}

/// Microsoft addresses go through the Microsoft sign-in, not IMAP.
pub fn is_microsoft(email: &str) -> bool {
    let domain = email
        .rsplit_once('@')
        .map_or("", |d| d.1)
        .to_ascii_lowercase();
    ["outlook.", "hotmail.", "live.", "msn.com"]
        .iter()
        .any(|p| domain.starts_with(p))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The server could not be reached: try again later.
    Network(String),
    /// Wrong address or password (or IMAP not enabled for the account).
    Auth(String),
    /// The message or folder is not there any more.
    NotFound,
    /// The server refused the command.
    Refused(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(e) => write!(f, "server unreachable: {e}"),
            Self::Auth(e) => write!(f, "sign-in refused: {e}"),
            Self::NotFound => f.write_str("message not found on the server"),
            Self::Refused(e) => write!(f, "server refused: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<imap::Error> for Error {
    fn from(e: imap::Error) -> Self {
        match e {
            imap::Error::Io(e) => Self::Network(e.to_string()),
            imap::Error::ConnectionLost => Self::Network("connection lost".into()),
            e => Self::Refused(e.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    /// Mailbox name as the server spells it (the folder id).
    pub id: String,
    /// Last level of the name, decoded.
    pub name: String,
    pub parent_id: Option<String>,
    /// MailCraft's well-known role (`inbox`, `sentitems`…).
    pub role: Option<&'static str>,
    /// Gmail's "All Mail", "Starred", "Important": views of other folders, not synced.
    pub is_view: bool,
    pub unread: i64,
    pub total: i64,
    /// `uidvalidity:uidnext:messages:highestmodseq` when the server gives them all (CONDSTORE):
    /// the same value as at the last sync means nothing changed in the folder.
    pub state: Option<String>,
}

/// What the list shows for a message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub id: String,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub preview: String,
    pub received: i64,
    pub is_read: bool,
    pub flagged: bool,
    pub has_attachments: bool,
}

/// State of a folder on the server, for the messages received since a date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub uid_validity: u32,
    /// uid → (read, flagged)
    pub flags: HashMap<u32, (bool, bool)>,
}

pub fn message_id(uid_validity: u32, uid: u32, mailbox: &str) -> String {
    format!("{uid_validity}:{uid}:{mailbox}")
}

/// (uidvalidity, uid, mailbox) of a message id.
pub fn parse_id(id: &str) -> Option<(u32, u32, &str)> {
    let mut it = id.splitn(3, ':');
    let v = it.next()?.parse().ok()?;
    let u = it.next()?.parse().ok()?;
    Some((v, u, it.next()?))
}

/// The TLS connection, wrapped so the IMAP client can change its read timeout (IDLE).
pub struct Tls(StreamOwned<ClientConnection, TcpStream>);

impl Read for Tls {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Tls {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl imap::extensions::idle::SetReadTimeout for Tls {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> imap::Result<()> {
        self.0
            .sock
            .set_read_timeout(timeout)
            .map_err(imap::Error::Io)
    }
}

fn tls_stream(host: &str, port: u16) -> Result<Tls> {
    let tcp =
        TcpStream::connect((host, port)).map_err(|e| Error::Network(format!("{host}: {e}")))?;
    tcp.set_read_timeout(Some(TIMEOUT)).ok();
    tcp.set_write_timeout(Some(TIMEOUT)).ok();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Network(e.to_string()))?;
    let config = rustls_platform_verifier::BuilderVerifierExt::with_platform_verifier(config)
        .map_err(|e| Error::Network(e.to_string()))?
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|e| Error::Network(format!("{host}: {e}")))?;
    let conn =
        ClientConnection::new(Arc::new(config), name).map_err(|e| Error::Network(e.to_string()))?;
    Ok(Tls(StreamOwned::new(conn, tcp)))
}

/// IMAP date for SEARCH SINCE: `1-Feb-2026`.
fn imap_date(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .unwrap_or_default()
        .format("%-d-%b-%Y")
        .to_string()
}

/// `1:3,7,9:10` for a sorted list of UIDs.
pub fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = String::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
        i += 1;
    }
    out
}

/// IMAP quoted string, for the commands the library sends raw (UID COPY, SEARCH). SELECT,
/// STATUS, MOVE, CREATE, RENAME, DELETE and APPEND are quoted by the library itself: never twice.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn role_of(name: &str, attributes: &[NameAttribute<'_>]) -> Option<&'static str> {
    if name.eq_ignore_ascii_case("INBOX") {
        return Some("inbox");
    }
    for a in attributes {
        let role = match a {
            NameAttribute::Drafts => "drafts",
            NameAttribute::Sent => "sentitems",
            NameAttribute::Trash => "deleteditems",
            NameAttribute::Junk => "junkemail",
            NameAttribute::Archive | NameAttribute::All => "archive",
            _ => continue,
        };
        return Some(role);
    }
    None
}

/// Role guessed from the name, for servers without SPECIAL-USE.
fn role_by_name(shown: &str) -> Option<&'static str> {
    let n = shown.to_lowercase();
    Some(match n.as_str() {
        "drafts" | "brouillons" | "draft" => "drafts",
        "sent"
        | "sent items"
        | "sent messages"
        | "sent mail"
        | "envoyés"
        | "messages envoyés"
        | "éléments envoyés"
        | "outbox" => "sentitems",
        "trash"
        | "deleted"
        | "deleted items"
        | "deleted messages"
        | "corbeille"
        | "éléments supprimés" => "deleteditems",
        "junk" | "spam" | "bulk mail" | "courrier indésirable" | "indésirables" | "pourriel" => {
            "junkemail"
        }
        "archive" | "archives" => "archive",
        _ => return None,
    })
}

pub struct Imap {
    session: imap::Session<Tls>,
    can_move: bool,
    /// CONDSTORE: STATUS gives a change counter (HIGHESTMODSEQ).
    condstore: bool,
    /// LIST-STATUS (RFC 5819): the folder list and every folder's STATUS in one command.
    list_status: bool,
    /// SPECIAL-USE: with an extended LIST (as LIST-STATUS is) the folder roles (Sent, Trash,
    /// Gmail's All Mail…) only come back when asked for.
    special_use: bool,
    selected: Option<String>,
}

impl Imap {
    pub fn connect(s: &Server) -> Result<Self> {
        let mut client = imap::Client::new(tls_stream(&s.imap_host, s.imap_port)?);
        client.read_greeting().map_err(Error::from)?;
        let user = if s.user.is_empty() { &s.email } else { &s.user };
        let refused = |(e, _)| match e {
            imap::Error::No(n) => Error::Auth(n.information),
            e => Error::from(e),
        };
        let mut session = if s.oauth {
            client
                .authenticate(
                    "XOAUTH2",
                    &XOauth2 {
                        user: user.to_owned(),
                        token: s.password.clone(),
                    },
                )
                .map_err(refused)?
        } else {
            client.login(user, &s.password).map_err(refused)?
        };
        let (can_move, condstore, list_status, special_use) = session
            .capabilities()
            .map(|c| {
                (
                    c.has_str("MOVE"),
                    c.has_str("CONDSTORE"),
                    c.has_str("LIST-STATUS"),
                    c.has_str("SPECIAL-USE"),
                )
            })
            .unwrap_or((false, false, false, false));
        Ok(Self {
            session,
            can_move,
            condstore,
            list_status,
            special_use,
            selected: None,
        })
    }

    /// Waits on the inbox with IDLE and calls `changed` each time the server reports new or
    /// removed mail. Returns when `changed` returns false, or on a connection error.
    pub fn watch_inbox(&mut self, mut changed: impl FnMut() -> bool) -> Result<()> {
        use imap::extensions::idle::{WaitOutcome, stop_on_any};
        let has_idle = self
            .session
            .capabilities()
            .map(|c| c.has_str("IDLE"))
            .unwrap_or(false);
        if !has_idle {
            return Err(Error::Refused("the server does not offer IDLE".into()));
        }
        self.select("INBOX")?;
        loop {
            // Servers drop an IDLE after 30 minutes; renewing it every 10 keeps it alive.
            let outcome = self
                .session
                .idle()
                .timeout(Duration::from_secs(600))
                .wait_while(stop_on_any)
                .map_err(Error::from)?;
            if outcome == WaitOutcome::MailboxChanged && !changed() {
                return Ok(());
            }
        }
    }

    pub fn logout(mut self) {
        let _ = self.session.logout();
    }

    /// Selects `mailbox` read-write (needed to change flags or move), once.
    fn select(&mut self, mailbox: &str) -> Result<imap::types::Mailbox> {
        let m = self.session.select(mailbox).map_err(|e| match e {
            imap::Error::No(_) => Error::NotFound,
            e => Error::from(e),
        })?;
        self.selected = Some(mailbox.to_owned());
        Ok(m)
    }

    fn ensure_selected(&mut self, mailbox: &str) -> Result<()> {
        if self.selected.as_deref() != Some(mailbox) {
            self.select(mailbox)?;
        }
        Ok(())
    }

    pub fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        // With LIST-STATUS the counts and states come with the list: one round trip instead
        // of one STATUS per folder. The STATUS answers arrive as unsolicited responses.
        // Only with SPECIAL-USE, which must then be asked for explicitly: an extended LIST
        // without it drops the roles, and Gmail's views would be synced as folders.
        let pattern = if self.list_status && self.special_use {
            format!("* RETURN (SPECIAL-USE STATUS {})", self.status_items())
        } else {
            "*".to_owned()
        };
        let names = self
            .session
            .list(Some(""), Some(&pattern))
            .map_err(Error::from)?;
        let mut listed: HashMap<String, Vec<imap_proto::StatusAttribute>> = self
            .session
            .take_all_unsolicited()
            .filter_map(|r| match r {
                imap::types::UnsolicitedResponse::Status {
                    mailbox,
                    attributes,
                } => Some((mailbox, attributes)),
                _ => None,
            })
            .collect();
        let mut raw: Vec<(String, Option<String>, Option<&'static str>, bool)> = Vec::new();
        for n in names.iter() {
            let attrs = n.attributes();
            if attrs
                .iter()
                .any(|a| matches!(a, NameAttribute::NoSelect))
                || attrs
                    .iter()
                    .any(|a| matches!(a, NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent")))
            {
                continue;
            }
            let id = n.name().to_owned();
            let parent = n
                .delimiter()
                .and_then(|d| id.rsplit_once(d).map(|(p, _)| p.to_owned()));
            let is_view = attrs.iter().any(|a| {
                matches!(a, NameAttribute::All | NameAttribute::Flagged)
                    || matches!(a, NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\Important"))
            });
            let leaf = n
                .delimiter()
                .and_then(|d| id.rsplit_once(d).map(|(_, l)| l.to_owned()))
                .unwrap_or_else(|| id.clone());
            let role = role_of(&id, attrs).or_else(|| role_by_name(&utf7::decode(&leaf)));
            raw.push((id, parent, role, is_view));
        }
        // A role goes to one folder only (the first the server lists).
        let mut taken = HashSet::new();
        let known: HashSet<String> = raw.iter().map(|r| r.0.clone()).collect();
        let mut out = Vec::new();
        for (id, parent, role, is_view) in raw {
            let role = role.filter(|r| taken.insert(*r));
            let listed = listed.remove(&id).map(|attrs| status_of(&attrs));
            let (unread, total, state) = match listed.map_or_else(|| self.status(&id), Ok) {
                Ok((m, state)) => (i64::from(m.unseen.unwrap_or(0)), i64::from(m.exists), state),
                Err(e @ (imap::Error::Io(_) | imap::Error::ConnectionLost)) => {
                    return Err(e.into());
                }
                Err(_) => (0, 0, None),
            };
            let leaf = parent
                .as_deref()
                .and_then(|p| id.strip_prefix(p))
                .map_or(id.as_str(), |l| &l[1..]);
            out.push(RemoteFolder {
                name: if role == Some("inbox") {
                    "Inbox".into()
                } else {
                    utf7::decode(leaf)
                },
                parent_id: parent.filter(|p| known.contains(p)),
                id,
                role,
                is_view,
                unread,
                total,
                state,
            });
        }
        Ok(out)
    }

    /// STATUS of a folder, and its state signature `uidvalidity:uidnext:messages:highestmodseq`
    /// when the server gives a change counter (CONDSTORE): an unchanged signature means nothing
    /// in the folder changed, not even a read mark.
    fn status(&mut self, mailbox: &str) -> imap::Result<(imap::types::Mailbox, Option<String>)> {
        let items = self.status_items();
        let m = self.session.status(mailbox, items)?;
        let state = state_of(&m);
        Ok((m, state))
    }

    fn status_items(&self) -> &'static str {
        if self.condstore {
            "(MESSAGES UNSEEN UIDNEXT UIDVALIDITY HIGHESTMODSEQ)"
        } else {
            "(MESSAGES UNSEEN)"
        }
    }

    /// The state signature of one folder (see [`Self::status`]), `None` without CONDSTORE.
    pub fn state(&mut self, mailbox: &str) -> Result<Option<String>> {
        Ok(self.status(mailbox).map_err(Error::from)?.1)
    }

    /// UIDs and flags of the messages received since `since` (unix seconds).
    pub fn scan(&mut self, mailbox: &str, since: i64) -> Result<Scan> {
        let m = self.select(mailbox)?;
        let uid_validity = m.uid_validity.unwrap_or(0);
        let mut flags = HashMap::new();
        if m.exists == 0 {
            return Ok(Scan {
                uid_validity,
                flags,
            });
        }
        let uids: Vec<u32> = self
            .session
            .uid_search(format!("SINCE {}", imap_date(since)))
            .map_err(Error::from)?
            .into_iter()
            .collect();
        for chunk in uids.chunks(CHUNK * 5) {
            let fetches = self
                .session
                .uid_fetch(uid_set(chunk), "(UID FLAGS)")
                .map_err(Error::from)?;
            for f in fetches.iter() {
                if let Some(uid) = f.uid {
                    flags.insert(uid, read_flagged(f.flags()));
                }
            }
        }
        Ok(Scan {
            uid_validity,
            flags,
        })
    }

    /// List entries for these messages (the folder must have been scanned just before).
    pub fn summaries(
        &mut self,
        mailbox: &str,
        uid_validity: u32,
        uids: &[u32],
    ) -> Result<Vec<Summary>> {
        self.ensure_selected(mailbox)?;
        let mut out = Vec::new();
        for chunk in uids.chunks(CHUNK) {
            let fetches = self
                .session
                .uid_fetch(
                    uid_set(chunk),
                    format!("(UID FLAGS INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.{PREVIEW_BYTES}>)"),
                )
                .map_err(Error::from)?;
            for f in fetches.iter() {
                let Some(uid) = f.uid else { continue };
                let (is_read, flagged) = read_flagged(f.flags());
                let header = f.header().unwrap_or_default();
                let text = f.text().unwrap_or_default();
                let mut s = mime::summary(header, text);
                s.id = message_id(uid_validity, uid, mailbox);
                s.is_read = is_read;
                s.flagged = flagged;
                if s.received == 0 {
                    s.received = f.internal_date().map_or(0, |d| d.timestamp());
                }
                out.push(s);
            }
        }
        Ok(out)
    }

    /// The whole message (RFC 5322 bytes), without marking it read.
    pub fn raw(&mut self, id: &str) -> Result<Vec<u8>> {
        self.fetch_one(id, "BODY.PEEK[]", |f| f.body().map(<[u8]>::to_vec))
    }

    /// Header block of a message, as (name, value) pairs.
    pub fn headers(&mut self, id: &str) -> Result<Vec<(String, String)>> {
        let raw = self.fetch_one(id, "BODY.PEEK[HEADER]", |f| f.header().map(<[u8]>::to_vec))?;
        Ok(mime::header_fields(&raw))
    }

    fn fetch_one(
        &mut self,
        id: &str,
        query: &str,
        take: impl Fn(&imap::types::Fetch<'_>) -> Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let (validity, uid, mailbox) = parse_id(id).ok_or(Error::NotFound)?;
        let m = self.select(mailbox)?;
        if m.uid_validity.is_some_and(|v| v != validity) {
            return Err(Error::NotFound);
        }
        let fetches = self
            .session
            .uid_fetch(uid.to_string(), format!("(UID {query})"))
            .map_err(Error::from)?;
        fetches
            .iter()
            .find(|f| f.uid == Some(uid))
            .and_then(take)
            .ok_or(Error::NotFound)
    }

    /// Selects the message's folder; `None` if the message cannot be there any more.
    fn locate(&mut self, id: &str) -> Result<Option<u32>> {
        let Some((validity, uid, mailbox)) = parse_id(id) else {
            return Ok(None);
        };
        let mailbox = mailbox.to_owned();
        match self.select(&mailbox) {
            Ok(m) if m.uid_validity.is_none_or(|v| v == validity) => Ok(Some(uid)),
            Ok(_) | Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_flag(&mut self, id: &str, flag: &str, on: bool) -> Result<()> {
        let Some(uid) = self.locate(id)? else {
            return Err(Error::NotFound);
        };
        let sign = if on { '+' } else { '-' };
        self.session
            .uid_store(uid.to_string(), format!("{sign}FLAGS.SILENT ({flag})"))
            .map_err(Error::from)?;
        Ok(())
    }

    pub fn move_to(&mut self, id: &str, dest: &str) -> Result<()> {
        let Some(uid) = self.locate(id)? else {
            return Err(Error::NotFound);
        };
        if self.can_move {
            self.session
                .uid_mv(uid.to_string(), dest)
                .map_err(Error::from)?;
        } else {
            self.session
                .uid_copy(uid.to_string(), quote(dest))
                .map_err(Error::from)?;
            self.expunge(uid)?;
        }
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        let Some(uid) = self.locate(id)? else {
            return Err(Error::NotFound);
        };
        self.expunge(uid)
    }

    fn expunge(&mut self, uid: u32) -> Result<()> {
        self.session
            .uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)")
            .map_err(Error::from)?;
        // UID EXPUNGE (UIDPLUS) removes only this one; plain EXPUNGE is the fallback.
        if self.session.uid_expunge(uid.to_string()).is_err() {
            self.session.expunge().map_err(Error::from)?;
        }
        Ok(())
    }

    /// Stores a message in a folder (sent copy, draft). Returns its id when the server says it.
    pub fn append(
        &mut self,
        mailbox: &str,
        bytes: &[u8],
        flags: &[Flag<'static>],
    ) -> Result<Option<String>> {
        let appended = self
            .session
            .append(mailbox, bytes)
            .flags(flags.iter().cloned())
            .finish()
            .map_err(Error::from)?;
        Ok(match (appended.uid_validity, appended.uids.as_deref()) {
            (Some(v), Some([imap_proto::UidSetMember::Uid(u), ..])) => {
                Some(message_id(v, *u, mailbox))
            }
            _ => None,
        })
    }

    pub fn create_folder(&mut self, name: &str) -> Result<()> {
        self.session.create(name).map_err(Error::from)
    }

    pub fn rename_folder(&mut self, from: &str, to: &str) -> Result<()> {
        self.selected = None;
        self.session.rename(from, to).map_err(Error::from)
    }

    pub fn delete_folder(&mut self, name: &str) -> Result<()> {
        self.selected = None;
        self.session.delete(name).map_err(Error::from)
    }

    /// Hierarchy separator of the server (`/` or `.`), for new sub-folders.
    pub fn delimiter(&mut self) -> String {
        self.session
            .list(Some(""), Some("\"\""))
            .ok()
            .and_then(|n| {
                n.iter()
                    .next()
                    .and_then(|n| n.delimiter().map(str::to_owned))
            })
            .unwrap_or_else(|| "/".into())
    }

    /// Every message of a folder (all dates) moved to `to`, or deleted for good when `to` is
    /// `None`. Returns how many.
    pub fn empty(&mut self, mailbox: &str, to: Option<&str>) -> Result<usize> {
        self.select(mailbox)?;
        let uids: Vec<u32> = self
            .session
            .uid_search("ALL")
            .map_err(Error::from)?
            .into_iter()
            .collect();
        for chunk in uids.chunks(CHUNK * 5) {
            let set = uid_set(chunk);
            match to {
                Some(dest) if self.can_move => {
                    self.session.uid_mv(&set, dest).map_err(Error::from)?;
                }
                Some(dest) => {
                    self.session
                        .uid_copy(&set, quote(dest))
                        .map_err(Error::from)?;
                    self.session
                        .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
                        .map_err(Error::from)?;
                }
                None => {
                    self.session
                        .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
                        .map_err(Error::from)?;
                }
            }
        }
        if to.is_none() || !self.can_move {
            self.session.expunge().map_err(Error::from)?;
        }
        Ok(uids.len())
    }

    /// Ids of the messages matching words, in a folder.
    pub fn search(&mut self, mailbox: &str, words: &str) -> Result<(u32, Vec<u32>)> {
        let m = self.select(mailbox)?;
        let q = quote(words);
        let found = self
            .session
            .uid_search(format!("OR OR SUBJECT {q} FROM {q} BODY {q}"))
            .map_err(Error::from)?;
        Ok((m.uid_validity.unwrap_or(0), found.into_iter().collect()))
    }

    /// UIDs of the messages from an address in a folder.
    pub fn from_sender(&mut self, mailbox: &str, address: &str) -> Result<Vec<String>> {
        let m = self.select(mailbox)?;
        let v = m.uid_validity.unwrap_or(0);
        Ok(self
            .session
            .uid_search(format!("FROM {}", quote(address)))
            .map_err(Error::from)?
            .into_iter()
            .map(|u| message_id(v, u, mailbox))
            .collect())
    }
}

/// SASL XOAUTH2 (Gmail): the user and a bearer token.
struct XOauth2 {
    user: String,
    token: String,
}

impl imap::Authenticator for XOauth2 {
    type Response = String;

    fn process(&self, _challenge: &[u8]) -> Self::Response {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

fn state_of(m: &imap::types::Mailbox) -> Option<String> {
    match (m.uid_validity, m.uid_next, m.highest_mod_seq) {
        (Some(v), Some(n), Some(q)) => Some(format!("{v}:{n}:{}:{q}", m.exists)),
        _ => None,
    }
}

/// A STATUS answer received with the folder list (LIST-STATUS), as STATUS gives it.
fn status_of(attrs: &[imap_proto::StatusAttribute]) -> (imap::types::Mailbox, Option<String>) {
    use imap_proto::StatusAttribute as A;
    let mut m = imap::types::Mailbox::default();
    for a in attrs {
        match *a {
            A::Messages(n) => m.exists = n,
            A::Unseen(n) => m.unseen = Some(n),
            A::UidNext(n) => m.uid_next = Some(n),
            A::UidValidity(n) => m.uid_validity = Some(n),
            A::HighestModSeq(n) => m.highest_mod_seq = Some(n),
            A::Recent(n) => m.recent = n,
            _ => {}
        }
    }
    let state = state_of(&m);
    (m, state)
}

fn read_flagged(flags: &[Flag<'_>]) -> (bool, bool) {
    (flags.contains(&Flag::Seen), flags.contains(&Flag::Flagged))
}

/// Keeps `Read + Write` in scope for the TLS stream type.
#[allow(dead_code)]
fn _assert_stream<T: Read + Write>() {}

#[cfg(test)]
mod tests;
