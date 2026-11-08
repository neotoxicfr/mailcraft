//! Writing mail through Graph: drafts (new, reply, reply all, forward), attachments (inline up
//! to 3 MB, upload session above) and sending.

use serde_json::{Value, json};

use crate::{GRAPH, Graph, GraphError, Result};

/// What the user wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outgoing {
    pub subject: String,
    /// Our part of the body, HTML.
    pub html: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
}

/// What the draft starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base<'a> {
    New,
    Reply(&'a str),
    ReplyAll(&'a str),
    Forward(&'a str),
}

/// Graph's limit for an attachment sent in one request.
pub const INLINE_LIMIT: usize = 3 * 1024 * 1024;
/// Upload-session chunks must be multiples of 320 KiB; 12 × 320 KiB = 3.75 MiB.
pub const CHUNK: usize = 12 * 320 * 1024;

/// `Name <address>` or a bare address to Graph's recipient object; `None` if no address.
pub fn recipient(text: &str) -> Option<Value> {
    let t = text.trim();
    let (name, addr) = match (t.rfind('<'), t.rfind('>')) {
        (Some(a), Some(b)) if a < b => (t[..a].trim().trim_matches('"').trim(), t[a + 1..b].trim()),
        _ => ("", t),
    };
    let valid = addr
        .split_once('@')
        .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.ends_with('.'))
        && !addr
            .chars()
            .any(|c| c.is_whitespace() || c == ',' || c == ';');
    valid.then(|| {
        if name.is_empty() {
            json!({ "emailAddress": { "address": addr } })
        } else {
            json!({ "emailAddress": { "address": addr, "name": name } })
        }
    })
}

/// Splits a header field typed by the user ("a@x.fr; Bob <b@y.com>, …").
pub fn split_recipients(field: &str) -> Vec<String> {
    field
        .split([';', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Content-Range windows for an upload session: (start, end inclusive).
pub fn chunks(size: usize) -> Vec<(usize, usize)> {
    (0..size)
        .step_by(CHUNK)
        .map(|start| (start, (start + CHUNK).min(size) - 1))
        .collect()
}

fn recipients(list: &[String]) -> Result<Vec<Value>> {
    list.iter()
        .map(|r| recipient(r).ok_or_else(|| GraphError::Api(400, format!("invalid address: {r}"))))
        .collect()
}

impl Graph<'_> {
    /// Creates the draft (or updates `draft_id`) and returns its id and the quoted part Microsoft
    /// put in a reply/forward draft (to keep under our text on later saves).
    pub fn save_draft(
        &self,
        draft_id: Option<&str>,
        base: Base<'_>,
        out: &Outgoing,
        quoted: Option<&str>,
    ) -> Result<(String, String)> {
        let (id, quoted) = match draft_id {
            Some(id) => (id.to_owned(), quoted.unwrap_or("").to_owned()),
            None => {
                let created = match base {
                    Base::New => self.post(
                        &format!("{GRAPH}/me/messages"),
                        &json!({ "subject": out.subject }),
                    )?,
                    Base::Reply(m) => {
                        self.post(&format!("{GRAPH}/me/messages/{m}/createReply"), &json!({}))?
                    }
                    Base::ReplyAll(m) => self.post(
                        &format!("{GRAPH}/me/messages/{m}/createReplyAll"),
                        &json!({}),
                    )?,
                    Base::Forward(m) => self.post(
                        &format!("{GRAPH}/me/messages/{m}/createForward"),
                        &json!({}),
                    )?,
                };
                let id = created["id"].as_str().unwrap_or("").to_owned();
                let quoted = if base == Base::New {
                    String::new()
                } else {
                    created["body"]["content"].as_str().unwrap_or("").to_owned()
                };
                (id, quoted)
            }
        };
        let body = format!("{}{}", out.html, quoted);
        let patch = json!({
            "subject": out.subject,
            "body": { "contentType": "html", "content": body },
            "toRecipients": recipients(&out.to)?,
            "ccRecipients": recipients(&out.cc)?,
            "bccRecipients": recipients(&out.bcc)?,
        });
        self.send(|| {
            self.http
                .patch(format!("{GRAPH}/me/messages/{id}"))
                .json(&patch)
        })?;
        Ok((id, quoted))
    }

    /// Adds a file to a draft: one request up to 3 MB, an upload session in 3.75 MiB chunks above.
    pub fn attach(&self, draft_id: &str, name: &str, bytes: &[u8]) -> Result<()> {
        use base64::Engine;
        if bytes.len() <= INLINE_LIMIT {
            let content = base64::engine::general_purpose::STANDARD.encode(bytes);
            self.post(
                &format!("{GRAPH}/me/messages/{draft_id}/attachments"),
                &json!({ "@odata.type": "#microsoft.graph.fileAttachment", "name": name, "contentBytes": content }),
            )?;
            return Ok(());
        }
        let session = self.post(
            &format!("{GRAPH}/me/messages/{draft_id}/attachments/createUploadSession"),
            &json!({ "AttachmentItem": { "attachmentType": "file", "name": name, "size": bytes.len() } }),
        )?;
        let url = session["uploadUrl"]
            .as_str()
            .ok_or(GraphError::Api(500, "no upload URL".into()))?;
        for (start, end) in chunks(bytes.len()) {
            // The upload URL carries its own authorisation: no bearer token here.
            let resp = self
                .http
                .put(url)
                .header(
                    "Content-Range",
                    format!("bytes {start}-{end}/{}", bytes.len()),
                )
                .body(bytes[start..=end].to_vec())
                .timeout(std::time::Duration::from_secs(300))
                .send()
                .map_err(|e| GraphError::Network(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(GraphError::Api(
                    resp.status().as_u16(),
                    format!("upload of {name} refused"),
                ));
            }
        }
        Ok(())
    }

    /// The address book: (name, address) for each e-mail address of each contact.
    pub fn contacts(&self) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        let mut url = format!("{GRAPH}/me/contacts?$select=displayName,emailAddresses&$top=200");
        loop {
            let v = self.get(&url)?;
            for c in v["value"].as_array().into_iter().flatten() {
                let name = c["displayName"].as_str().unwrap_or("");
                for e in c["emailAddresses"].as_array().into_iter().flatten() {
                    if let Some(addr) = e["address"].as_str().filter(|a| a.contains('@')) {
                        out.push((name.to_owned(), addr.to_owned()));
                    }
                }
            }
            match v["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_owned(),
                None => return Ok(out),
            }
        }
    }

    pub fn send_draft(&self, draft_id: &str) -> Result<()> {
        self.post(&format!("{GRAPH}/me/messages/{draft_id}/send"), &json!({}))
            .map(|_| ())
    }

    fn post(&self, url: &str, body: &Value) -> Result<Value> {
        self.send(|| {
            self.http
                .post(url)
                .header("Prefer", "IdType=\"ImmutableId\"")
                .json(body)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CHUNK, chunks, recipient, split_recipients};

    #[test]
    fn recipients_parse() {
        assert_eq!(
            recipient("a@example.com").unwrap()["emailAddress"]["address"],
            "a@example.com"
        );
        let r = recipient("\"Jean Dupont\" <jean@example.fr>").unwrap();
        assert_eq!(
            (
                r["emailAddress"]["name"].as_str(),
                r["emailAddress"]["address"].as_str()
            ),
            (Some("Jean Dupont"), Some("jean@example.fr"))
        );
        for bad in [
            "",
            "jean",
            "@example.com",
            "a@b",
            "a b@example.com",
            "a@example.",
        ] {
            assert!(recipient(bad).is_none(), "{bad}");
        }
        assert_eq!(
            split_recipients(" a@x.fr; B <b@y.com> ,, "),
            ["a@x.fr", "B <b@y.com>"]
        );
    }

    #[test]
    fn upload_chunks_are_320k_multiples_and_cover_the_file() {
        assert_eq!(CHUNK % (320 * 1024), 0);
        let size = 2 * CHUNK + 1234;
        let c = chunks(size);
        assert_eq!(
            c,
            [
                (0, CHUNK - 1),
                (CHUNK, 2 * CHUNK - 1),
                (2 * CHUNK, size - 1)
            ]
        );
        assert_eq!(chunks(10), [(0, 9)]);
    }
}
