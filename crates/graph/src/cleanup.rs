//! Graph calls for the clean-up view: headers of a message, every message of a sender (all
//! folders, older ones included), the unsubscribe e-mail, and a blocking rule.

use serde_json::json;

use crate::{GRAPH, Graph, Result};

/// OData string literal: single quotes doubled.
fn odata_string(s: &str) -> String {
    s.replace('\'', "''")
}

pub fn sender_filter_url(address: &str) -> String {
    let filter = format!("from/emailAddress/address eq '{}'", odata_string(address));
    let q: String = url::form_urlencoded::byte_serialize(filter.as_bytes()).collect();
    format!("{GRAPH}/me/messages?$filter={q}&$select=id,parentFolderId&$top=200")
}

impl Graph<'_> {
    /// Internet headers of a message: (name, value).
    pub fn headers(&self, id: &str) -> Result<Vec<(String, String)>> {
        let v = self.get(&format!(
            "{GRAPH}/me/messages/{id}?$select=internetMessageHeaders"
        ))?;
        Ok(v["internetMessageHeaders"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| {
                (
                    h["name"].as_str().unwrap_or("").to_owned(),
                    h["value"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect())
    }

    /// Out-of-office reply on the server: on (until the end of `until`, unix seconds, when
    /// given) or off. The same text goes to everyone.
    pub fn set_auto_reply(&self, on: bool, message: &str, until: Option<i64>) -> Result<()> {
        let html = format!(
            "<p>{}</p>",
            message
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\n', "<br>")
        );
        let utc = |t: i64| {
            chrono::DateTime::from_timestamp(t, 0)
                .unwrap_or_default()
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        };
        let mut setting = serde_json::json!({
            "status": if !on { "disabled" } else if until.is_some() { "scheduled" } else { "alwaysEnabled" },
            "externalAudience": "all",
            "internalReplyMessage": html,
            "externalReplyMessage": html,
        });
        if on && let Some(until) = until {
            let now = chrono::Utc::now().timestamp();
            setting["scheduledStartDateTime"] =
                serde_json::json!({ "dateTime": utc(now), "timeZone": "UTC" });
            setting["scheduledEndDateTime"] =
                serde_json::json!({ "dateTime": utc(until), "timeZone": "UTC" });
        }
        let body = serde_json::json!({ "automaticRepliesSetting": setting });
        self.send(|| {
            self.http
                .patch(format!("{GRAPH}/me/mailboxSettings"))
                .json(&body)
        })?;
        Ok(())
    }

    /// (message id, folder id) of every message from `address`, in every folder.
    pub fn messages_from(&self, address: &str) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        let mut url = sender_filter_url(address);
        loop {
            let v = self.get(&url)?;
            for m in v["value"].as_array().into_iter().flatten() {
                out.push((
                    m["id"].as_str().unwrap_or("").to_owned(),
                    m["parentFolderId"].as_str().unwrap_or("").to_owned(),
                ));
            }
            match v["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_owned(),
                None => return Ok(out),
            }
        }
    }

    /// Ids of every message of a folder, all dates (for emptying it).
    pub fn folder_message_ids(&self, folder_id: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut url = format!("{GRAPH}/me/mailFolders/{folder_id}/messages?$select=id&$top=200");
        loop {
            let v = self.get(&url)?;
            out.extend(
                v["value"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| m["id"].as_str().map(str::to_owned)),
            );
            match v["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_owned(),
                None => return Ok(out),
            }
        }
    }

    /// Sends a plain-text message without keeping it in Sent Items (unsubscribe by e-mail).
    pub fn send_plain(&self, to: &str, subject: &str, text: &str) -> Result<()> {
        let body = json!({
            "message": {
                "subject": subject,
                "body": { "contentType": "Text", "content": text },
                "toRecipients": [{ "emailAddress": { "address": to } }],
            },
            "saveToSentItems": false,
        });
        self.send(|| self.http.post(format!("{GRAPH}/me/sendMail")).json(&body))
            .map(|_| ())
    }

    /// Inbox rule: mail from `address` goes to `junk_folder_id` from now on.
    pub fn block_rule(&self, address: &str, junk_folder_id: &str) -> Result<()> {
        let body = json!({
            "displayName": format!("MailCraft : bloquer {address}"),
            "sequence": 1,
            "isEnabled": true,
            "conditions": { "senderContains": [address] },
            "actions": { "moveToFolder": junk_folder_id, "stopProcessingRules": true },
        });
        self.send(|| {
            self.http
                .post(format!("{GRAPH}/me/mailFolders/inbox/messageRules"))
                .json(&body)
        })
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::sender_filter_url;

    #[test]
    fn sender_filter_is_quoted_and_encoded() {
        let url = sender_filter_url("o'brien@example.com");
        assert!(
            url.contains(
                "$filter=from%2FemailAddress%2Faddress+eq+%27o%27%27brien%40example.com%27"
            ),
            "{url}"
        );
    }
}
