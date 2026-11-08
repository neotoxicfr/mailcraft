use super::{is_microsoft, mime, parse_id, preset, role_by_name, uid_set, utf7};

#[test]
fn folder_names_in_modified_utf7() {
    assert_eq!(utf7::decode("Envoy&AOk-s"), "Envoyés");
    assert_eq!(
        utf7::decode("&AMk-l&AOk-ments supprim&AOk-s"),
        "Éléments supprimés"
    );
    assert_eq!(utf7::decode("A&-B"), "A&B");
    assert_eq!(utf7::decode("broken&AOk"), "broken&AOk");
    for name in ["Envoyés", "Éléments supprimés", "A&B", "日本語", "plain"] {
        assert_eq!(utf7::decode(&utf7::encode(name)), name);
    }
    assert_eq!(utf7::encode("Envoyés"), "Envoy&AOk-s");
}

#[test]
fn ids_sets_and_presets() {
    assert_eq!(
        parse_id("7:42:Dossier:avec:deux-points"),
        Some((7, 42, "Dossier:avec:deux-points"))
    );
    assert_eq!(parse_id("x:1:INBOX"), None);
    assert_eq!(uid_set(&[9, 1, 2, 3, 7, 10, 3]), "1:3,7,9:10");
    assert_eq!(
        preset("someone@Gmail.com").map(|p| p.0),
        Some("imap.gmail.com")
    );
    assert_eq!(
        preset("someone@yahoo.fr").map(|p| p.1),
        Some("smtp.mail.yahoo.com")
    );
    assert_eq!(preset("someone@example.com"), None);
    assert!(is_microsoft("someone@hotmail.fr") && !is_microsoft("someone@gmail.com"));
    assert_eq!(role_by_name("Envoyés"), Some("sentitems"));
    assert_eq!(role_by_name("Spam"), Some("junkemail"));
}

const MAIL: &str = "From: \"Example Shop\" <news@shop.example>\r\n\
To: Someone <user@example.com>, other@example.com\r\n\
Subject: =?utf-8?q?Votre_commande_exp=C3=A9di=C3=A9e?=\r\n\
Date: Mon, 2 Nov 2026 10:00:00 +0100\r\n\
Message-ID: <abc@shop.example>\r\n\
List-Unsubscribe: <https://shop.example/u?x=1>,\r\n <mailto:u@shop.example>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: multipart/alternative; boundary=\"b2\"\r\n\
\r\n\
--b2\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Bonjour, votre colis arrive demain.\r\n\
--b2\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Bonjour, votre <b>colis</b> arrive demain.</p><img src=\"cid:logo@shop\">\r\n\
--b2--\r\n\
--b1\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@shop>\r\n\
Content-Disposition: inline\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--b1\r\n\
Content-Type: application/pdf; name=\"facture.pdf\"\r\n\
Content-Disposition: attachment; filename=\"facture.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--b1--\r\n";

#[test]
fn parse_full_message() {
    let p = mime::parse(MAIL.as_bytes());
    assert_eq!(p.subject, "Votre commande expédiée");
    assert_eq!(p.from_addr, "news@shop.example");
    assert_eq!(p.to, "Someone <user@example.com>; other@example.com");
    assert!(p.html.contains("<b>colis</b>"));
    assert!(p.text.contains("colis arrive demain"));
    assert_eq!(p.message_id, "abc@shop.example");
    let pdf = p
        .parts
        .iter()
        .find(|x| x.name == "facture.pdf")
        .expect("pdf part");
    assert!(!pdf.is_inline && pdf.content_type == "application/pdf");
    assert_eq!(
        mime::part_bytes(MAIL.as_bytes(), &pdf.id).unwrap(),
        b"%PDF-1.4\n"
    );
    let logo = p
        .parts
        .iter()
        .find(|x| x.content_id == "logo@shop")
        .expect("inline image");
    assert!(logo.is_inline);
}

#[test]
fn summary_from_header_and_cut_text() {
    let (header, text) = MAIL.split_once("\r\n\r\n").unwrap();
    let cut = &text[..text.len().min(300)];
    let s = mime::summary(format!("{header}\r\n\r\n").as_bytes(), cut.as_bytes());
    assert_eq!(s.from_name, "Example Shop");
    assert_eq!(s.subject, "Votre commande expédiée");
    assert!(s.has_attachments);
    assert!(s.received > 1_700_000_000);
    assert!(s.preview.starts_with("Bonjour"), "{}", s.preview);
    let h = mime::header_fields(header.as_bytes());
    let unsub = h.iter().find(|(k, _)| k == "List-Unsubscribe").unwrap();
    assert_eq!(
        unsub.1,
        "<https://shop.example/u?x=1>, <mailto:u@shop.example>"
    );
}

/// Real network: TLS with the Windows certificate store, IMAP greeting and a refused login.
#[test]
#[ignore = "needs the network"]
fn live_server_refuses_a_fake_password() {
    let server = super::Server {
        email: "user@example.com".into(),
        password: "not-a-password".into(),
        imap_host: "imap.gmail.com".into(),
        imap_port: 993,
        ..Default::default()
    };
    match super::Imap::connect(&server) {
        Err(super::Error::Auth(_)) => {}
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("fake password accepted"),
    }
}

#[test]
#[ignore = "needs the network"]
fn live_smtp_refuses_a_fake_password() {
    for port in [465, 587] {
        let server = super::Server {
            email: "user@example.com".into(),
            password: "not-a-password".into(),
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: port,
            ..Default::default()
        };
        let r = super::smtp::check(&server);
        assert!(matches!(r, Err(super::Error::Auth(_))), "{port}: {r:?}");
    }
}
