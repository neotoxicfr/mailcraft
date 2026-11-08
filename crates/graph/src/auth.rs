//! Microsoft sign-in for a public desktop client: OAuth 2.0 authorization code with PKCE,
//! redirected to a one-shot loopback listener (`http://localhost:<any port>`, as registered).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// MailCraft's app registration (public client: an identifier, not a secret).
pub const DEFAULT_CLIENT_ID: &str = "ca8a6658-b155-4705-8186-a72a35a00f1c";
/// Asked at sign-in: the 8 permissions of the app registration (mail, out-of-office, contacts,
/// calendar, frequent people). Accounts signed in before some of them keep working: sessions
/// are refreshed with [`MAIL_SCOPES`] only, and each extra part asks for its own token.
pub const SCOPES: &str = "offline_access User.Read Mail.ReadWrite Mail.Send \
     MailboxSettings.ReadWrite Contacts.ReadWrite Calendars.ReadWrite People.Read";
pub const MAIL_SCOPES: &str =
    "offline_access User.Read Mail.ReadWrite Mail.Send MailboxSettings.ReadWrite";
/// Token for the address book; fails if the user never consented to it.
pub const CONTACTS_SCOPES: &str = "offline_access Contacts.ReadWrite";
/// Accounts that consented only to reading contacts (signed in before Contacts.ReadWrite).
pub const CONTACTS_READ_SCOPES: &str = "offline_access Contacts.Read";
/// Token for the calendar (for the calendar view to come).
pub const CALENDAR_SCOPES: &str = "offline_access Calendars.ReadWrite";
/// Token for the people the user writes to most.
pub const PEOPLE_SCOPES: &str = "offline_access People.Read";

#[derive(Debug)]
pub enum AuthError {
    Cancelled,
    TimedOut,
    Denied(String),
    Network(String),
    Local(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("sign-in cancelled"),
            Self::TimedOut => f.write_str("sign-in timed out"),
            Self::Denied(e) => write!(f, "sign-in refused: {e}"),
            Self::Network(e) => write!(f, "network: {e}"),
            Self::Local(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AuthError {}

#[derive(Clone, Debug)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_at: Instant,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

fn random_b64(len: usize) -> Result<String, AuthError> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes)
        .map_err(|_| AuthError::Local("no random numbers from the system".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// An OAuth 2.0 provider for a desktop app (authorization code + PKCE, loopback redirect).
pub struct OAuth<'a> {
    pub authorize_url: &'a str,
    pub token_url: &'a str,
    pub client_id: &'a str,
    /// Google requires its "client secret" even for desktop apps (it is not secret there).
    pub client_secret: Option<&'a str>,
    pub scopes: &'a str,
    /// More parameters for the sign-in page.
    pub extra: &'a [(&'a str, &'a str)],
}

fn microsoft<'a>(client_id: &'a str, scopes: &'a str) -> OAuth<'a> {
    OAuth {
        authorize_url: concat!(
            "https://login.microsoftonline.com/common/oauth2/v2.0",
            "/authorize"
        ),
        token_url: concat!(
            "https://login.microsoftonline.com/common/oauth2/v2.0",
            "/token"
        ),
        client_id,
        client_secret: None,
        scopes,
        extra: &[("response_mode", "query"), ("prompt", "select_account")],
    }
}

/// Opens the browser on Microsoft's sign-in page and waits for the redirect (or `timeout`).
/// `cancel` is polled so the user can give up from the app.
pub fn sign_in(
    http: &Client,
    client_id: &str,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<Tokens, AuthError> {
    sign_in_with(http, &microsoft(client_id, SCOPES), timeout, cancel)
}

/// Same sign-in with any provider.
pub fn sign_in_with(
    http: &Client,
    o: &OAuth<'_>,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<Tokens, AuthError> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| AuthError::Local(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| AuthError::Local(e.to_string()))?
        .port();
    let redirect = format!("http://localhost:{port}");
    let verifier = random_b64(32)?;
    let state = random_b64(16)?;
    let code_challenge = challenge(&verifier);
    let mut params = vec![
        ("client_id", o.client_id),
        ("response_type", "code"),
        ("redirect_uri", redirect.as_str()),
        ("scope", o.scopes),
        ("state", state.as_str()),
        ("code_challenge", code_challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    params.extend_from_slice(o.extra);
    let url = url::Url::parse_with_params(o.authorize_url, params)
        .map_err(|e| AuthError::Local(e.to_string()))?;
    webbrowser::open(url.as_str())
        .map_err(|e| AuthError::Local(format!("cannot open the browser: {e}")))?;

    let code = wait_for_code(&listener, &state, Instant::now() + timeout, cancel)?;
    let mut form = vec![
        ("client_id", o.client_id),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("code_verifier", verifier.as_str()),
        ("scope", o.scopes),
    ];
    if let Some(secret) = o.client_secret {
        form.push(("client_secret", secret));
    }
    token_request(http, o.token_url, &form)
}

/// New access token from a refresh token, for `scopes` (usually [`MAIL_SCOPES`]). Microsoft may
/// rotate the refresh token: keep the returned one.
pub fn refresh(
    http: &Client,
    client_id: &str,
    refresh_token: &str,
    scopes: &str,
) -> Result<Tokens, AuthError> {
    refresh_with(http, &microsoft(client_id, scopes), refresh_token)
}

/// New access token with any provider; keeps the old refresh token when none comes back.
pub fn refresh_with(
    http: &Client,
    o: &OAuth<'_>,
    refresh_token: &str,
) -> Result<Tokens, AuthError> {
    let mut form = vec![
        ("client_id", o.client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("scope", o.scopes),
    ];
    if let Some(secret) = o.client_secret {
        form.push(("client_secret", secret));
    }
    token_request(http, o.token_url, &form).map(|mut t| {
        if t.refresh.is_empty() {
            t.refresh = refresh_token.to_owned();
        }
        t
    })
}

fn token_request(http: &Client, url: &str, form: &[(&str, &str)]) -> Result<Tokens, AuthError> {
    let resp = http
        .post(url)
        .form(form)
        .send()
        .map_err(|e| AuthError::Network(e.to_string()))?;
    let body: TokenResponse = resp.json().map_err(|e| AuthError::Network(e.to_string()))?;
    match (body.access_token, body.error) {
        (Some(access), None) => Ok(Tokens {
            access,
            refresh: body.refresh_token.unwrap_or_default(),
            // A minute early, so a token never expires in the middle of a request.
            expires_at: Instant::now()
                + Duration::from_secs(body.expires_in.unwrap_or(3600).saturating_sub(60)),
        }),
        (_, error) => Err(AuthError::Denied(
            body.error_description
                .or(error)
                .unwrap_or_else(|| "no token in the answer".into()),
        )),
    }
}

fn wait_for_code(
    listener: &TcpListener,
    state: &str,
    deadline: Instant,
    cancel: &dyn Fn() -> bool,
) -> Result<String, AuthError> {
    listener
        .set_nonblocking(true)
        .map_err(|e| AuthError::Local(e.to_string()))?;
    loop {
        if cancel() {
            return Err(AuthError::Cancelled);
        }
        if Instant::now() > deadline {
            return Err(AuthError::TimedOut);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = handle_redirect(stream, state) {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(AuthError::Local(e.to_string())),
        }
    }
}

/// `None` for unrelated requests (favicon…), so the listener keeps waiting.
fn handle_redirect(mut stream: TcpStream, state: &str) -> Option<Result<String, AuthError>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?;
    let url = url::Url::parse(&format!("http://localhost{target}")).ok()?;
    if url.path() != "/" {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let get = |k: &str| {
        url.query_pairs()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.into_owned())
    };
    let result = match (get("code"), get("state"), get("error")) {
        (Some(code), Some(s), None) if s == state => Ok(code),
        (_, _, Some(err)) if err == "access_denied" => Err(AuthError::Cancelled),
        (_, _, Some(err)) => Err(AuthError::Denied(get("error_description").unwrap_or(err))),
        _ => Err(AuthError::Denied(
            "unexpected answer (state mismatch)".into(),
        )),
    };
    let message = if result.is_ok() {
        "Connexion réussie, vous pouvez fermer cet onglet et revenir à MailCraft.<br>Signed in: you can close this tab and go back to MailCraft."
    } else {
        "La connexion n'a pas abouti, revenez à MailCraft.<br>Sign-in did not complete: go back to MailCraft."
    };
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>MailCraft</title><body style=\"font-family:sans-serif;margin:3em\">{message}</body>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::{challenge, handle_redirect};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn pkce_challenge_is_base64url_sha256() {
        // Reference computed independently with .NET's SHA256 + base64url.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW-gXk7XcQ0k"),
            "aA1YZ14lwcqE2EJhCevDqnuF3O7IAIo3AaD35ocCAxY"
        );
    }

    fn redirect(path: &str) -> (Option<Result<String, super::AuthError>>, String) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        write!(client, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let (server, _) = listener.accept().unwrap();
        let result = handle_redirect(server, "st");
        let mut page = String::new();
        let _ = client.read_to_string(&mut page);
        (result, page)
    }

    #[test]
    fn redirect_accepts_only_matching_state() {
        let (r, page) = redirect("/?code=abc&state=st");
        assert_eq!(r.unwrap().unwrap(), "abc");
        assert!(page.contains("200 OK"));
        assert!(redirect("/?code=abc&state=other").0.unwrap().is_err());
        assert!(matches!(
            redirect("/?error=access_denied&state=st").0.unwrap(),
            Err(super::AuthError::Cancelled)
        ));
        assert!(redirect("/favicon.ico").0.is_none());
    }
}
