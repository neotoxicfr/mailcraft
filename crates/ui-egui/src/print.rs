//! Printing: the message as a page opened in the default browser, which shows its print dialog.
//! A content security policy lets only our one-line print script run: the mail's own scripts
//! never do, and remote images follow the tracker blocker.

use mailcraft_store::{MessageBody, MessageSummary};

use crate::i18n::{Lang, T};

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn page(
    m: &MessageSummary,
    body: &MessageBody,
    date: &str,
    lang: Lang,
    (block_images, block_trackers): (bool, bool),
) -> String {
    let from = if m.from_name.is_empty() {
        escape(&m.from_addr)
    } else {
        format!("{} &lt;{}&gt;", escape(&m.from_name), escape(&m.from_addr))
    };
    let mut header = format!(
        "<h2>{}</h2><p><b>{}</b> {from}<br><b>{}</b> {}",
        escape(&m.subject),
        T::PrintFrom.get(lang),
        T::PrintDate.get(lang),
        escape(date)
    );
    for (label, value) in [(T::To, &body.to), (T::Cc, &body.cc)] {
        if !value.is_empty() {
            header.push_str(&format!(
                "<br><b>{} :</b> {}",
                label.get(lang),
                escape(value)
            ));
        }
    }
    header.push_str("</p><hr>");
    let content = if body.html.trim().is_empty() {
        format!(
            "<pre style=\"white-space:pre-wrap;font-family:inherit\">{}</pre>",
            escape(&body.text)
        )
    } else {
        mailcraft_html::prepare(&body.html, block_images, block_trackers).into_owned()
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">\
<title>{}</title><style>body{{font-family:Segoe UI,Arial,sans-serif;margin:24px;color:#242424}}</style></head>\
<body>{header}<div>{content}</div>\
<script nonce=\"mailcraftprint\">window.addEventListener('load',function(){{window.print();}});</script></body></html>",
        escape(&m.subject),
        policy = mailcraft_html::page_policy(block_images, Some("mailcraftprint")),
    )
}

/// `2026-11-08 Votre facture.eml`: date and subject, without characters Windows refuses.
pub fn eml_name(m: &MessageSummary, used: &mut std::collections::HashSet<String>) -> String {
    let date = chrono::DateTime::from_timestamp(m.received, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let subject: String = m
        .subject
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
    let subject = subject.trim_end_matches('.').trim();
    let base = if subject.is_empty() {
        format!("{date} message")
    } else {
        format!("{date} {subject}")
    };
    let mut name = format!("{base}.eml");
    let mut n = 2;
    while !used.insert(name.to_lowercase()) {
        name = format!("{base} ({n}).eml");
        n += 1;
    }
    name
}

#[cfg(test)]
mod tests {
    use super::{eml_name, page};
    use crate::i18n::Lang;
    use mailcraft_store::{MessageBody, MessageSummary};

    fn m(subject: &str) -> MessageSummary {
        MessageSummary {
            account_id: 1,
            id: "x".into(),
            folder_id: "in".into(),
            from_name: "Shop <b>".into(),
            from_addr: "s@example.com".into(),
            subject: subject.into(),
            preview: String::new(),
            received: 1_794_130_200,
            is_read: true,
            has_attachments: false,
            flagged: false,
        }
    }

    #[test]
    fn printed_page_blocks_mail_scripts() {
        let body = MessageBody {
            html: "<p>Bonjour</p><script>alert(1)</script>".into(),
            ..MessageBody::default()
        };
        let html = page(&m("Facture"), &body, "08/11/2026", Lang::Fr, (true, true));
        assert!(html.contains("script-src 'nonce-mailcraftprint'"));
        assert!(html.contains("img-src data:;") && html.contains("default-src 'none'"));
        assert!(html.contains("Shop &lt;b&gt;"));
        assert!(html.contains("window.print()"));
    }

    #[test]
    fn eml_names_are_safe_and_unique() {
        let mut used = std::collections::HashSet::new();
        let a = eml_name(&m("RE: Facture n°12 / mars?"), &mut used);
        assert!(
            a.ends_with("RE  Facture n°12 mars.eml".replace("  ", " ").as_str()),
            "{a}"
        );
        let b = eml_name(&m("RE: Facture n°12 / mars?"), &mut used);
        assert!(b.ends_with("(2).eml"));
        assert!(eml_name(&m("..."), &mut used).ends_with("message.eml"));
    }
}
