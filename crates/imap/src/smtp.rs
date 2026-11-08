//! Sending through the account's SMTP server.

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

use crate::{Error, Result, Server};

pub struct Mail<'a> {
    pub to: &'a [String],
    pub cc: &'a [String],
    pub bcc: &'a [String],
    pub subject: &'a str,
    pub html: &'a str,
    pub text: &'a str,
    /// `<id@host>` of the message answered.
    pub in_reply_to: Option<&'a str>,
    pub references: Option<&'a str>,
    /// (file name, bytes)
    pub files: &'a [(String, Vec<u8>)],
}

fn mailbox(text: &str) -> Result<Mailbox> {
    text.trim()
        .parse()
        .map_err(|e| Error::Refused(format!("{text}: {e}")))
}

fn content_type(name: &str) -> ContentType {
    let ext = name
        .rsplit_once('.')
        .map_or("", |e| e.1)
        .to_ascii_lowercase();
    let t = match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "zip" => "application/zip",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    };
    ContentType::parse(t).unwrap_or(ContentType::TEXT_PLAIN)
}

pub fn build(from: &Server, m: &Mail<'_>) -> Result<Message> {
    let me = if from.name.trim().is_empty() {
        mailbox(&from.email)?
    } else {
        mailbox(&format!(
            "{} <{}>",
            from.name.replace(['<', '>', '"'], ""),
            from.email
        ))?
    };
    let mut b = Message::builder().from(me).subject(m.subject);
    for t in m.to {
        b = b.to(mailbox(t)?);
    }
    for t in m.cc {
        b = b.cc(mailbox(t)?);
    }
    for t in m.bcc {
        b = b.bcc(mailbox(t)?);
    }
    if let Some(id) = m.in_reply_to.filter(|i| !i.is_empty()) {
        b = b.in_reply_to(id.to_owned());
    }
    if let Some(r) = m.references.filter(|r| !r.is_empty()) {
        b = b.references(r.to_owned());
    }
    let body = MultiPart::alternative_plain_html(m.text.to_owned(), m.html.to_owned());
    let message = if m.files.is_empty() {
        b.multipart(body)
    } else {
        let mut mixed = MultiPart::mixed().multipart(body);
        for (name, bytes) in m.files {
            let part: SinglePart =
                Attachment::new(name.clone()).body(bytes.clone(), content_type(name));
            mixed = mixed.singlepart(part);
        }
        b.multipart(mixed)
    };
    message.map_err(|e| Error::Refused(e.to_string()))
}

pub fn send(server: &Server, message: &Message) -> Result<()> {
    let user = if server.user.is_empty() {
        &server.email
    } else {
        &server.user
    };
    let builder = if server.smtp_port == 465 {
        SmtpTransport::relay(&server.smtp_host)
    } else {
        SmtpTransport::starttls_relay(&server.smtp_host)
    }
    .map_err(|e| Error::Network(e.to_string()))?;
    let transport = builder
        .port(server.smtp_port)
        .credentials(Credentials::new(user.clone(), server.password.clone()))
        .timeout(Some(std::time::Duration::from_secs(60)))
        .build();
    transport.send(message).map(|_| ()).map_err(|e| {
        if e.is_permanent() {
            Error::Refused(e.to_string())
        } else {
            Error::Network(e.to_string())
        }
    })
}

/// Checks that the SMTP server accepts the password, without sending anything.
pub fn check(server: &Server) -> Result<()> {
    let user = if server.user.is_empty() {
        &server.email
    } else {
        &server.user
    };
    let builder = if server.smtp_port == 465 {
        SmtpTransport::relay(&server.smtp_host)
    } else {
        SmtpTransport::starttls_relay(&server.smtp_host)
    }
    .map_err(|e| Error::Network(e.to_string()))?;
    let ok = builder
        .port(server.smtp_port)
        .credentials(Credentials::new(user.clone(), server.password.clone()))
        .timeout(Some(std::time::Duration::from_secs(30)))
        .build()
        .test_connection()
        .map_err(|e| Error::Auth(e.to_string()))?;
    if ok {
        Ok(())
    } else {
        Err(Error::Network("SMTP server did not answer".into()))
    }
}
