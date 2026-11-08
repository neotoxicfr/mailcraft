//! Microsoft Graph mail: sign-in ([`auth`]) and the few calls MailCraft needs.

pub mod auth;

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::Value;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
// Immutable ids: a message keeps its id when moved between folders.
const PREFER: &str = "odata.maxpagesize=100, IdType=\"ImmutableId\"";
const MESSAGE_FIELDS: &str =
    "id,subject,from,receivedDateTime,bodyPreview,isRead,hasAttachments,flag";

#[derive(Debug)]
pub enum GraphError {
    /// The access token is no longer accepted: refresh it and retry.
    Unauthorized,
    /// The delta state expired: start the folder again from scratch.
    SyncReset,
    NotFound,
    Network(String),
    Api(u16, String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("Microsoft no longer accepts the session"),
            Self::SyncReset => f.write_str("sync state expired"),
            Self::NotFound => f.write_str("not found"),
            Self::Network(e) => write!(f, "network: {e}"),
            Self::Api(code, e) => write!(f, "Microsoft Graph {code}: {e}"),
        }
    }
}

impl std::error::Error for GraphError {}

pub type Result<T> = std::result::Result<T, GraphError>;

/// HTTPS client shared by sign-in and Graph calls (rustls on ring, system certificates).
pub fn http_client() -> std::result::Result<Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::builder()
        .user_agent(concat!("MailCraft/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFolder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub unread: i64,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMessage {
    pub id: String,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub preview: String,
    pub received: i64,
    pub is_read: bool,
    pub has_attachments: bool,
    pub flagged: bool,
}

#[derive(Debug, Default)]
pub struct DeltaPage {
    pub messages: Vec<RemoteMessage>,
    pub removed: Vec<String>,
    pub next: Option<String>,
    pub delta_link: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RemoteBody {
    /// HTML when `is_html`, plain text otherwise.
    pub content: String,
    pub is_html: bool,
    pub to: String,
    pub cc: String,
    pub attachments: Vec<RemoteAttachment>,
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct RemoteAttachment {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub is_inline: bool,
}

/// An inline attachment's content, to show `cid:` images.
#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct InlineContent {
    pub content_id: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

pub struct Graph<'a> {
    http: &'a Client,
    token: &'a str,
}

impl<'a> Graph<'a> {
    pub fn new(http: &'a Client, token: &'a str) -> Self {
        Self { http, token }
    }

    fn send(&self, build: impl Fn() -> RequestBuilder) -> Result<Value> {
        for attempt in 0..4 {
            let resp = build()
                .bearer_auth(self.token)
                .send()
                .map_err(|e| GraphError::Network(e.to_string()))?;
            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE
            {
                let wait = resp
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2u64 << attempt);
                std::thread::sleep(Duration::from_secs(wait.min(60)));
                continue;
            }
            return match status {
                s if s.is_success() => {
                    let text = resp
                        .text()
                        .map_err(|e| GraphError::Network(e.to_string()))?;
                    Ok(if text.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_str(&text).unwrap_or(Value::Null)
                    })
                }
                StatusCode::UNAUTHORIZED => Err(GraphError::Unauthorized),
                StatusCode::NOT_FOUND => Err(GraphError::NotFound),
                StatusCode::GONE => Err(GraphError::SyncReset),
                s => {
                    let body: Value = resp.json().unwrap_or(Value::Null);
                    let msg = body["error"]["message"].as_str().unwrap_or("").to_owned();
                    if s == StatusCode::BAD_REQUEST && body["error"]["code"] == "SyncStateNotFound"
                    {
                        return Err(GraphError::SyncReset);
                    }
                    Err(GraphError::Api(s.as_u16(), msg))
                }
            };
        }
        Err(GraphError::Api(
            429,
            "Microsoft keeps asking to slow down".into(),
        ))
    }

    fn get(&self, url: &str) -> Result<Value> {
        self.send(|| self.http.get(url).header("Prefer", PREFER))
    }

    /// (e-mail address, display name) of the signed-in user.
    pub fn me(&self) -> Result<(String, String)> {
        let v = self.get(&format!(
            "{GRAPH}/me?$select=displayName,mail,userPrincipalName"
        ))?;
        let email = v["mail"]
            .as_str()
            .or(v["userPrincipalName"].as_str())
            .unwrap_or("")
            .to_owned();
        Ok((email, v["displayName"].as_str().unwrap_or("").to_owned()))
    }

    /// Every mail folder, children included (Graph lists one level at a time).
    pub fn folders(&self) -> Result<Vec<RemoteFolder>> {
        let mut out = Vec::new();
        let mut pending = vec![(None, format!("{GRAPH}/me/mailFolders?$top=100"))];
        while let Some((parent, mut url)) = pending.pop() {
            loop {
                let v = self.get(&url)?;
                for f in v["value"].as_array().into_iter().flatten() {
                    let id = f["id"].as_str().unwrap_or("").to_owned();
                    if f["childFolderCount"].as_i64().unwrap_or(0) > 0 {
                        pending.push((
                            Some(id.clone()),
                            format!("{GRAPH}/me/mailFolders/{id}/childFolders?$top=100"),
                        ));
                    }
                    out.push(RemoteFolder {
                        id,
                        parent_id: parent.clone(),
                        name: f["displayName"].as_str().unwrap_or("").to_owned(),
                        unread: f["unreadItemCount"].as_i64().unwrap_or(0),
                        total: f["totalItemCount"].as_i64().unwrap_or(0),
                    });
                }
                match v["@odata.nextLink"].as_str() {
                    Some(next) => url = next.to_owned(),
                    None => break,
                }
            }
        }
        Ok(out)
    }

    /// Id of a well-known folder (`inbox`, `sentitems`…), `None` if the mailbox has none.
    pub fn well_known_id(&self, role: &str) -> Result<Option<String>> {
        match self.get(&format!("{GRAPH}/me/mailFolders/{role}?$select=id")) {
            Ok(v) => Ok(v["id"].as_str().map(str::to_owned)),
            Err(GraphError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// First delta request of a folder: messages received since `since_unix`, newest first.
    pub fn delta_start(folder_id: &str, since_unix: i64) -> String {
        let since = chrono::DateTime::from_timestamp(since_unix, 0)
            .unwrap_or_default()
            .format("%Y-%m-%dT%H:%M:%SZ");
        format!(
            "{GRAPH}/me/mailFolders/{folder_id}/messages/delta?$select={MESSAGE_FIELDS}\
             &$filter=receivedDateTime+ge+{since}&$orderby=receivedDateTime+desc"
        )
    }

    pub fn delta_page(&self, url: &str) -> Result<DeltaPage> {
        let v = self.get(url)?;
        Ok(parse_delta(&v))
    }

    /// Body as plain text (Microsoft converts HTML), with recipients.
    /// Body as sent (HTML or text), recipients and the list of attachments (without content).
    pub fn body(&self, id: &str) -> Result<RemoteBody> {
        let url = format!(
            "{GRAPH}/me/messages/{id}?$select=body,toRecipients,ccRecipients\
             &$expand=attachments($select=id,name,contentType,size,isInline)"
        );
        let v = self.get(&url)?;
        let attachments = v["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| RemoteAttachment {
                id: a["id"].as_str().unwrap_or("").to_owned(),
                name: a["name"].as_str().unwrap_or("").to_owned(),
                content_type: a["contentType"].as_str().unwrap_or("").to_owned(),
                size: a["size"].as_i64().unwrap_or(0),
                is_inline: a["isInline"].as_bool().unwrap_or(false),
            })
            .collect();
        Ok(RemoteBody {
            content: v["body"]["content"].as_str().unwrap_or("").to_owned(),
            is_html: v["body"]["contentType"].as_str() == Some("html"),
            to: recipients(&v["toRecipients"]),
            cc: recipients(&v["ccRecipients"]),
            attachments,
        })
    }

    /// Raw content of a file attachment, for saving or opening it.
    pub fn attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        let url = format!("{GRAPH}/me/messages/{message_id}/attachments/{attachment_id}/$value");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(self.token)
            .header("Prefer", "IdType=\"ImmutableId\"")
            .timeout(Duration::from_secs(600))
            .send()
            .map_err(|e| GraphError::Network(e.to_string()))?;
        match resp.status() {
            s if s.is_success() => Ok(resp
                .bytes()
                .map_err(|e| GraphError::Network(e.to_string()))?
                .to_vec()),
            StatusCode::UNAUTHORIZED => Err(GraphError::Unauthorized),
            StatusCode::NOT_FOUND => Err(GraphError::NotFound),
            s => Err(GraphError::Api(
                s.as_u16(),
                "attachment download refused".into(),
            )),
        }
    }

    /// One attachment with its content (base64 in Graph's answer). For `cid:` images and files.
    pub fn attachment(&self, message_id: &str, attachment_id: &str) -> Result<InlineContent> {
        use base64::Engine;
        let v = self.get(&format!(
            "{GRAPH}/me/messages/{message_id}/attachments/{attachment_id}"
        ))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(v["contentBytes"].as_str().unwrap_or(""))
            .map_err(|e| GraphError::Api(200, format!("attachment content: {e}")))?;
        Ok(InlineContent {
            content_id: v["contentId"]
                .as_str()
                .unwrap_or("")
                .trim_matches(['<', '>'])
                .to_owned(),
            content_type: v["contentType"].as_str().unwrap_or("").to_owned(),
            bytes,
        })
    }

    /// Runs a change on the mailbox. For `CreateFolder`, returns the new folder's JSON.
    pub fn apply(&self, op: &Op<'_>) -> Result<Value> {
        let (method, url, body) = op.request();
        self.send(|| {
            let req = self
                .http
                .request(method.clone(), &url)
                .header("Prefer", "IdType=\"ImmutableId\"");
            match &body {
                Some(b) => req.json(b),
                None => req,
            }
        })
    }

    /// Searches the whole mailbox on the server (older mail included): (folder id, message).
    pub fn search(&self, query: &str, top: usize) -> Result<Vec<(String, RemoteMessage)>> {
        let v = self.get(&search_url(query, top))?;
        Ok(v["value"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                (
                    m["parentFolderId"].as_str().unwrap_or("").to_owned(),
                    parse_message(m),
                )
            })
            .collect())
    }

    /// Creates a folder under `parent` (top level when `None`).
    pub fn create_folder(&self, parent: Option<&str>, name: &str) -> Result<RemoteFolder> {
        let v = self.apply(&Op::CreateFolder { parent, name })?;
        Ok(RemoteFolder {
            id: v["id"].as_str().unwrap_or("").to_owned(),
            parent_id: parent.map(str::to_owned),
            name: v["displayName"].as_str().unwrap_or(name).to_owned(),
            unread: 0,
            total: 0,
        })
    }
}

/// A change to the mailbox. Message ids are immutable ids; `dest` may be a folder id or a
/// well-known name (`deleteditems`, `archive`…).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op<'a> {
    Move {
        id: &'a str,
        dest: &'a str,
    },
    /// Permanent: the message leaves the mailbox (recoverable only by an administrator tool).
    Delete {
        id: &'a str,
    },
    SetRead {
        id: &'a str,
        read: bool,
    },
    SetFlag {
        id: &'a str,
        flagged: bool,
    },
    CreateFolder {
        parent: Option<&'a str>,
        name: &'a str,
    },
    RenameFolder {
        id: &'a str,
        name: &'a str,
    },
    DeleteFolder {
        id: &'a str,
    },
}

impl Op<'_> {
    pub fn request(&self) -> (reqwest::Method, String, Option<Value>) {
        use reqwest::Method;
        use serde_json::json;
        match *self {
            Op::Move { id, dest } => (
                Method::POST,
                format!("{GRAPH}/me/messages/{id}/move"),
                Some(json!({ "destinationId": dest })),
            ),
            Op::Delete { id } => (Method::DELETE, format!("{GRAPH}/me/messages/{id}"), None),
            Op::SetRead { id, read } => (
                Method::PATCH,
                format!("{GRAPH}/me/messages/{id}"),
                Some(json!({ "isRead": read })),
            ),
            Op::SetFlag { id, flagged } => (
                Method::PATCH,
                format!("{GRAPH}/me/messages/{id}"),
                Some(
                    json!({ "flag": { "flagStatus": if flagged { "flagged" } else { "notFlagged" } } }),
                ),
            ),
            Op::CreateFolder { parent, name } => (
                Method::POST,
                match parent {
                    Some(p) => format!("{GRAPH}/me/mailFolders/{p}/childFolders"),
                    None => format!("{GRAPH}/me/mailFolders"),
                },
                Some(json!({ "displayName": name })),
            ),
            Op::RenameFolder { id, name } => (
                Method::PATCH,
                format!("{GRAPH}/me/mailFolders/{id}"),
                Some(json!({ "displayName": name })),
            ),
            Op::DeleteFolder { id } => {
                (Method::DELETE, format!("{GRAPH}/me/mailFolders/{id}"), None)
            }
        }
    }
}

fn recipients(v: &Value) -> String {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|r| address(&r["emailAddress"]))
        .collect::<Vec<_>>()
        .join("; ")
}

fn address(e: &Value) -> String {
    let name = e["name"].as_str().unwrap_or("");
    let addr = e["address"].as_str().unwrap_or("");
    if name.is_empty() || name == addr {
        addr.to_owned()
    } else {
        format!("{name} <{addr}>")
    }
}

fn parse_delta(v: &Value) -> DeltaPage {
    let mut page = DeltaPage {
        next: v["@odata.nextLink"].as_str().map(str::to_owned),
        delta_link: v["@odata.deltaLink"].as_str().map(str::to_owned),
        ..DeltaPage::default()
    };
    for m in v["value"].as_array().into_iter().flatten() {
        let id = m["id"].as_str().unwrap_or("").to_owned();
        if !m["@removed"].is_null() {
            page.removed.push(id);
            continue;
        }
        page.messages.push(parse_message(m));
    }
    page
}

fn parse_message(m: &Value) -> RemoteMessage {
    let from = &m["from"]["emailAddress"];
    RemoteMessage {
        id: m["id"].as_str().unwrap_or("").to_owned(),
        from_name: from["name"].as_str().unwrap_or("").to_owned(),
        from_addr: from["address"].as_str().unwrap_or("").to_owned(),
        subject: m["subject"].as_str().unwrap_or("").to_owned(),
        preview: m["bodyPreview"].as_str().unwrap_or("").to_owned(),
        received: m["receivedDateTime"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map_or(0, |d| d.timestamp()),
        is_read: m["isRead"].as_bool().unwrap_or(true),
        has_attachments: m["hasAttachments"].as_bool().unwrap_or(false),
        flagged: m["flag"]["flagStatus"] == "flagged",
    }
}

/// `$search` value: the user's words as one quoted KQL phrase-free term list.
fn search_url(query: &str, top: usize) -> String {
    let words: Vec<&str> = query
        .split(|c: char| c.is_whitespace() || c == '"')
        .filter(|w| !w.is_empty())
        .collect();
    let q = url::form_urlencoded::byte_serialize(format!("\"{}\"", words.join(" ")).as_bytes())
        .collect::<String>();
    format!("{GRAPH}/me/messages?$search={q}&$top={top}&$select={MESSAGE_FIELDS},parentFolderId")
}

#[cfg(test)]
mod tests {
    use super::{Graph, parse_delta};

    #[test]
    fn delta_page_parsing() {
        let v = serde_json::json!({
            "@odata.nextLink": "https://next",
            "value": [
                {
                    "id": "m1",
                    "subject": "Votre facture",
                    "bodyPreview": "Bonjour",
                    "receivedDateTime": "2026-11-08T09:30:00Z",
                    "isRead": false,
                    "hasAttachments": true,
                    "flag": { "flagStatus": "flagged" },
                    "from": { "emailAddress": { "name": "Example Shop", "address": "billing@example.com" } }
                },
                { "id": "m2", "@removed": { "reason": "deleted" } }
            ]
        });
        let p = parse_delta(&v);
        assert_eq!(p.next.as_deref(), Some("https://next"));
        assert!(p.delta_link.is_none());
        assert_eq!(p.removed, ["m2"]);
        let m = &p.messages[0];
        assert_eq!(
            (m.from_addr.as_str(), m.received, m.is_read, m.flagged),
            ("billing@example.com", 1_794_130_200, false, true)
        );
    }

    #[test]
    fn search_url_quotes_the_words() {
        let url = super::search_url("  facture \"EDF\"  2026 ", 50);
        assert!(
            url.contains("$search=%22facture+EDF+2026%22&$top=50"),
            "{url}"
        );
        assert!(url.ends_with(",parentFolderId"));
    }

    #[test]
    fn change_requests() {
        use super::Op;
        use reqwest::Method;
        let (m, url, body) = Op::Move {
            id: "M1",
            dest: "deleteditems",
        }
        .request();
        assert_eq!(
            (m, url.as_str()),
            (
                Method::POST,
                "https://graph.microsoft.com/v1.0/me/messages/M1/move"
            )
        );
        assert_eq!(body.unwrap()["destinationId"], "deleteditems");
        let (m, _, body) = Op::SetFlag {
            id: "M1",
            flagged: false,
        }
        .request();
        assert_eq!(
            (m, body.unwrap()["flag"]["flagStatus"].as_str()),
            (Method::PATCH, Some("notFlagged"))
        );
        let (_, url, _) = Op::CreateFolder {
            parent: Some("P"),
            name: "Factures",
        }
        .request();
        assert!(url.ends_with("/me/mailFolders/P/childFolders"));
        let (_, url, _) = Op::CreateFolder {
            parent: None,
            name: "Factures",
        }
        .request();
        assert!(url.ends_with("/me/mailFolders"));
        assert_eq!(Op::Delete { id: "M1" }.request().0, Method::DELETE);
    }

    #[test]
    fn delta_start_url_filters_by_date() {
        let url = Graph::delta_start("AQMk", 1_794_130_200);
        assert!(url.contains("/me/mailFolders/AQMk/messages/delta?$select=id,"));
        assert!(url.contains("$filter=receivedDateTime+ge+2026-11-08T09:30:00Z"));
    }
}
