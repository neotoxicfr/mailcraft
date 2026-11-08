//! MailCraft local store: accounts, folders and messages in SQLite, encrypted with SQLCipher when
//! the user wants it, secrets sealed with the master key from the OS vault (see [`vault`]).
//!
//! Open one [`Store`] per thread (UI and sync worker); WAL mode lets them read and write together.

pub mod vault;

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

pub use crate::vault::{MasterKey, VaultError};

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Vault(VaultError),
    Io(std::io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(e) => write!(f, "database: {e}"),
            Self::Vault(e) => e.fmt(f),
            Self::Io(e) => write!(f, "database file: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<VaultError> for StoreError {
    fn from(e: VaultError) -> Self {
        Self::Vault(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub provider: String,
    pub email: String,
    pub name: String,
}

/// Well-known folders, in the order the folder pane shows them.
pub const ROLES: [&str; 6] = [
    "inbox",
    "drafts",
    "sentitems",
    "deleteditems",
    "junkemail",
    "archive",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub account_id: i64,
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    /// One of [`ROLES`], or `None` for a user folder.
    pub role: Option<String>,
    pub unread: i64,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageSummary {
    pub account_id: i64,
    pub id: String,
    pub folder_id: String,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub preview: String,
    /// Unix seconds, UTC.
    pub received: i64,
    pub is_read: bool,
    pub has_attachments: bool,
    pub flagged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageBody {
    pub text: String,
    pub to: String,
    pub cc: String,
}

const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

pub struct Store {
    conn: Connection,
    key: MasterKey,
    encrypted: bool,
}

impl Store {
    /// Opens the database at `path`. A new file is encrypted when `encrypt_new` is set; an
    /// existing one keeps the form it was created with (changing it is an explicit migration).
    pub fn open(path: &Path, key: MasterKey, encrypt_new: bool) -> Result<Self> {
        let encrypted = match std::fs::File::open(path) {
            Ok(mut f) => {
                let mut head = [0u8; 16];
                let n = f.read(&mut head).map_err(StoreError::Io)?;
                !(n == 0 || &head == SQLITE_MAGIC) || (n == 0 && encrypt_new)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => encrypt_new,
            Err(e) => return Err(StoreError::Io(e)),
        };
        let conn = Connection::open(path)?;
        // SQLCipher logs a failed key check to stderr; the error returned is enough.
        conn.execute_batch("PRAGMA cipher_log_level = NONE;")?;
        if encrypted {
            conn.execute_batch(&format!("PRAGMA key = \"{}\";", key.sqlcipher_pragma()))?;
        }
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            conn,
            key,
            encrypted,
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            self.conn.execute_batch(
                "BEGIN;
                CREATE TABLE accounts (
                    id INTEGER PRIMARY KEY,
                    provider TEXT NOT NULL,
                    email TEXT NOT NULL,
                    name TEXT NOT NULL,
                    refresh_token BLOB,
                    UNIQUE(provider, email)
                );
                CREATE TABLE folders (
                    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                    id TEXT NOT NULL,
                    parent_id TEXT,
                    name TEXT NOT NULL,
                    role TEXT,
                    unread INTEGER NOT NULL DEFAULT 0,
                    total INTEGER NOT NULL DEFAULT 0,
                    delta_link TEXT,
                    PRIMARY KEY (account_id, id)
                );
                CREATE TABLE messages (
                    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                    id TEXT NOT NULL,
                    folder_id TEXT NOT NULL,
                    from_name TEXT NOT NULL,
                    from_addr TEXT NOT NULL,
                    subject TEXT NOT NULL,
                    preview TEXT NOT NULL,
                    received INTEGER NOT NULL,
                    is_read INTEGER NOT NULL,
                    has_attachments INTEGER NOT NULL,
                    flagged INTEGER NOT NULL,
                    PRIMARY KEY (account_id, id)
                );
                CREATE INDEX messages_by_folder ON messages(account_id, folder_id, received DESC);
                CREATE TABLE bodies (
                    account_id INTEGER NOT NULL,
                    id TEXT NOT NULL,
                    text TEXT NOT NULL,
                    to_list TEXT NOT NULL,
                    cc_list TEXT NOT NULL,
                    PRIMARY KEY (account_id, id),
                    FOREIGN KEY (account_id, id) REFERENCES messages(account_id, id) ON DELETE CASCADE
                );
                PRAGMA user_version = 1;
                COMMIT;",
            )?;
        }
        Ok(())
    }

    // ---- accounts ----

    /// Adds the account, or updates its name and token if it is already there. Returns its id.
    pub fn upsert_account(
        &self,
        provider: &str,
        email: &str,
        name: &str,
        refresh_token: &str,
    ) -> Result<i64> {
        let sealed = self.key.seal(refresh_token.as_bytes())?;
        self.conn.execute(
            "INSERT INTO accounts (provider, email, name, refresh_token) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider, email) DO UPDATE SET name = excluded.name, refresh_token = excluded.refresh_token",
            params![provider, email, name, sealed],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM accounts WHERE provider = ?1 AND email = ?2",
            params![provider, email],
            |r| r.get(0),
        )?)
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut st = self
            .conn
            .prepare("SELECT id, provider, email, name FROM accounts ORDER BY id")?;
        let rows = st.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                provider: r.get(1)?,
                email: r.get(2)?,
                name: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn refresh_token(&self, account_id: i64) -> Result<Option<String>> {
        let sealed: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT refresh_token FROM accounts WHERE id = ?1",
                [account_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        match sealed {
            None => Ok(None),
            Some(s) => Ok(Some(
                String::from_utf8_lossy(&self.key.open(&s)?).into_owned(),
            )),
        }
    }

    pub fn set_refresh_token(&self, account_id: i64, token: &str) -> Result<()> {
        let sealed = self.key.seal(token.as_bytes())?;
        self.conn.execute(
            "UPDATE accounts SET refresh_token = ?2 WHERE id = ?1",
            params![account_id, sealed],
        )?;
        Ok(())
    }

    // ---- folders ----

    /// Replaces the account's folder list, keeping the sync state of folders that remain.
    pub fn replace_folders(&mut self, account_id: i64, folders: &[Folder]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "CREATE TEMP TABLE IF NOT EXISTS keep (id TEXT PRIMARY KEY)",
            [],
        )?;
        tx.execute("DELETE FROM keep", [])?;
        for f in folders {
            tx.execute(
                "INSERT INTO folders (account_id, id, parent_id, name, role, unread, total)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id, id) DO UPDATE SET parent_id = excluded.parent_id,
                    name = excluded.name, role = excluded.role, unread = excluded.unread, total = excluded.total",
                params![account_id, f.id, f.parent_id, f.name, f.role, f.unread, f.total],
            )?;
            tx.execute("INSERT OR IGNORE INTO keep VALUES (?1)", [&f.id])?;
        }
        tx.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND folder_id NOT IN (SELECT id FROM keep)",
            [account_id],
        )?;
        tx.execute(
            "DELETE FROM folders WHERE account_id = ?1 AND id NOT IN (SELECT id FROM keep)",
            [account_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn folders(&self, account_id: i64) -> Result<Vec<Folder>> {
        let mut st = self.conn.prepare(
            "SELECT account_id, id, parent_id, name, role, unread, total FROM folders WHERE account_id = ?1",
        )?;
        let rows = st.query_map([account_id], |r| {
            Ok(Folder {
                account_id: r.get(0)?,
                id: r.get(1)?,
                parent_id: r.get(2)?,
                name: r.get(3)?,
                role: r.get(4)?,
                unread: r.get(5)?,
                total: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delta_link(&self, account_id: i64, folder_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT delta_link FROM folders WHERE account_id = ?1 AND id = ?2",
                params![account_id, folder_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_delta_link(
        &self,
        account_id: i64,
        folder_id: &str,
        link: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET delta_link = ?3 WHERE account_id = ?1 AND id = ?2",
            params![account_id, folder_id, link],
        )?;
        Ok(())
    }

    // ---- messages ----

    /// Applies one page of changes of one folder in a single transaction. `removed` only drops
    /// messages still filed in `folder_id`: a message moved elsewhere (and already synced there)
    /// is reported as removed by its old folder too.
    pub fn apply_changes(
        &mut self,
        account_id: i64,
        folder_id: &str,
        upserts: &[MessageSummary],
        removed: &[String],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        for m in upserts {
            tx.execute(
                "INSERT INTO messages (account_id, id, folder_id, from_name, from_addr, subject, preview,
                    received, is_read, has_attachments, flagged)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(account_id, id) DO UPDATE SET folder_id = excluded.folder_id,
                    from_name = excluded.from_name, from_addr = excluded.from_addr,
                    subject = excluded.subject, preview = excluded.preview, received = excluded.received,
                    is_read = excluded.is_read, has_attachments = excluded.has_attachments,
                    flagged = excluded.flagged",
                params![
                    m.account_id,
                    m.id,
                    m.folder_id,
                    m.from_name,
                    m.from_addr,
                    m.subject,
                    m.preview,
                    m.received,
                    m.is_read,
                    m.has_attachments,
                    m.flagged
                ],
            )?;
        }
        for id in removed {
            tx.execute(
                "DELETE FROM messages WHERE account_id = ?1 AND id = ?2 AND folder_id = ?3",
                params![account_id, id, folder_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Newest first. `search` filters on sender and subject (case-insensitive), empty for all.
    pub fn messages(
        &self,
        account_id: i64,
        folder_id: &str,
        search: &str,
        limit: usize,
    ) -> Result<Vec<MessageSummary>> {
        let pattern = format!(
            "%{}%",
            search
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut st = self.conn.prepare(
            "SELECT account_id, id, folder_id, from_name, from_addr, subject, preview, received,
                    is_read, has_attachments, flagged
             FROM messages
             WHERE account_id = ?1 AND folder_id = ?2
               AND (?3 = '%%' OR subject LIKE ?3 ESCAPE '\\' OR from_name LIKE ?3 ESCAPE '\\'
                    OR from_addr LIKE ?3 ESCAPE '\\')
             ORDER BY received DESC LIMIT ?4",
        )?;
        let rows = st.query_map(params![account_id, folder_id, pattern, limit as i64], |r| {
            Ok(MessageSummary {
                account_id: r.get(0)?,
                id: r.get(1)?,
                folder_id: r.get(2)?,
                from_name: r.get(3)?,
                from_addr: r.get(4)?,
                subject: r.get(5)?,
                preview: r.get(6)?,
                received: r.get(7)?,
                is_read: r.get(8)?,
                has_attachments: r.get(9)?,
                flagged: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marks a message read or unread and keeps the folder's unread count in step.
    pub fn set_read(&mut self, account_id: i64, id: &str, read: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE messages SET is_read = ?3 WHERE account_id = ?1 AND id = ?2 AND is_read != ?3",
            params![account_id, id, read],
        )?;
        if changed > 0 {
            tx.execute(
                "UPDATE folders SET unread = MAX(0, unread + ?3)
                 WHERE account_id = ?1 AND id = (SELECT folder_id FROM messages WHERE account_id = ?1 AND id = ?2)",
                params![account_id, id, if read { -1 } else { 1 }],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn body(&self, account_id: i64, id: &str) -> Result<Option<MessageBody>> {
        Ok(self
            .conn
            .query_row(
                "SELECT text, to_list, cc_list FROM bodies WHERE account_id = ?1 AND id = ?2",
                params![account_id, id],
                |r| {
                    Ok(MessageBody {
                        text: r.get(0)?,
                        to: r.get(1)?,
                        cc: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn put_body(&self, account_id: i64, id: &str, body: &MessageBody) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO bodies (account_id, id, text, to_list, cc_list) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![account_id, id, body.text, body.to, body.cc],
        )?;
        Ok(())
    }
}
