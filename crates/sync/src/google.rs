//! Gmail through the browser sign-in (OAuth), instead of an app password.
//!
//! The Google project identifiers are not in the source: they are read when MailCraft is built
//! (see `build.rs`: environment variables, or a `google-client.json` file kept out of git).
//! Without them the Google button stays hidden and Gmail works with an app password.

use std::time::Instant;

use mailcraft_graph::auth::{self, OAuth, Tokens};
use mailcraft_imap::Server;
use reqwest::blocking::Client;

use crate::{Event, Runner};

const CLIENT_ID: Option<&str> = option_env!("MAILCRAFT_GOOGLE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("MAILCRAFT_GOOGLE_CLIENT_SECRET");
/// Full mail access over IMAP/SMTP, contacts (and the people written to), calendar, and the
/// address and name of the account: asked once, so contacts and calendar need no new consent.
const SCOPES: &str = concat!(
    "https://mail.google.com/ ",
    "https://www.googleapis.com/auth/contacts ",
    "https://www.googleapis.com/auth/contacts.other.readonly ",
    "https://www.googleapis.com/auth/calendar ",
    "openid email profile"
);

/// This build can sign in to Google.
pub fn configured() -> bool {
    CLIENT_ID.is_some_and(|c| !c.is_empty()) && CLIENT_SECRET.is_some_and(|c| !c.is_empty())
}

fn oauth() -> Option<OAuth<'static>> {
    Some(OAuth {
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        client_id: CLIENT_ID.filter(|c| !c.is_empty())?,
        client_secret: CLIENT_SECRET.filter(|c| !c.is_empty()),
        scopes: SCOPES,
        // A refresh token every time, and the account chooser.
        extra: &[
            ("access_type", "offline"),
            ("prompt", "consent select_account"),
        ],
    })
}

/// (address, name) of the signed-in account.
fn who(http: &Client, access: &str) -> Result<(String, String), String> {
    let v: serde_json::Value = http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(access)
        .send()
        .and_then(reqwest::blocking::Response::json)
        .map_err(|e| e.to_string())?;
    let email = v["email"].as_str().unwrap_or("").to_owned();
    if email.is_empty() {
        return Err("Google gave no address for this account".into());
    }
    Ok((email, v["name"].as_str().unwrap_or("").to_owned()))
}

impl Runner {
    pub(crate) fn add_google_account(&mut self, emit: &dyn Fn(Event)) -> Result<(), String> {
        let Some(o) = oauth() else {
            return Err("this MailCraft was built without Google sign-in".into());
        };
        emit(Event::SigningIn);
        let cancel = self.cancel.clone();
        let tokens = match auth::sign_in_with(&self.http, &o, crate::SIGN_IN_TIMEOUT, &move || {
            cancel.load(std::sync::atomic::Ordering::Relaxed)
        }) {
            Ok(t) => t,
            Err(auth::AuthError::Cancelled) => {
                emit(Event::SignInCancelled);
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        if tokens.refresh.is_empty() {
            return Err("Google gave no lasting session: try again".into());
        }
        let (email, name) = who(&self.http, &tokens.access)?;
        let server = Server {
            email: email.clone(),
            name,
            imap_host: "imap.gmail.com".into(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: 465,
            oauth: true,
            refresh: tokens.refresh.clone(),
            ..Server::default()
        };
        let secret = serde_json::to_string(&server).map_err(|e| e.to_string())?;
        let id = self
            .store
            .upsert_account(mailcraft_imap::PROVIDER_IMAP, &email, &server.name, &secret)
            .map_err(|e| e.to_string())?;
        self.servers.insert(id, server);
        self.tokens.insert(id, tokens);
        self.imap.remove(&id);
        emit(Event::AccountAdded { email });
        self.imap_sync_account(id, emit)?;
        emit(Event::SyncDone);
        Ok(())
    }

    /// A valid Google access token for the account (renewed a minute before it expires).
    pub(crate) fn google_access(
        &mut self,
        account_id: i64,
        server: &Server,
    ) -> Result<String, String> {
        if let Some(t) = self.tokens.get(&account_id)
            && Instant::now() < t.expires_at
        {
            return Ok(t.access.clone());
        }
        let o = oauth().ok_or("this MailCraft was built without Google sign-in")?;
        let tokens: Tokens =
            auth::refresh_with(&self.http, &o, &server.refresh).map_err(|e| e.to_string())?;
        let access = tokens.access.clone();
        self.tokens.insert(account_id, tokens);
        Ok(access)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scopes_are_single_spaced() {
        assert!(!super::SCOPES.contains("  ") && !super::SCOPES.ends_with(' '));
        assert_eq!(super::SCOPES.split(' ').count(), 7);
    }

    /// Built with google-client.json (or the variables) present: the identifiers are in.
    #[test]
    #[ignore = "needs google-client.json at build time"]
    fn identifiers_come_from_the_build() {
        assert!(super::configured());
    }
}
