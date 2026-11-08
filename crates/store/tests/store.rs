use mailcraft_store::{Folder, MasterKey, MessageBody, MessageSummary, Store};

fn key() -> MasterKey {
    MasterKey::from_bytes([7; 32])
}

fn msg(id: &str, folder: &str, received: i64, subject: &str) -> MessageSummary {
    MessageSummary {
        account_id: 1,
        id: id.into(),
        folder_id: folder.into(),
        from_name: "Example Shop".into(),
        from_addr: "news@example.com".into(),
        subject: subject.into(),
        preview: "preview".into(),
        received,
        is_read: false,
        has_attachments: false,
        flagged: false,
    }
}

fn folder(id: &str, unread: i64) -> Folder {
    Folder {
        account_id: 1,
        id: id.into(),
        parent_id: None,
        name: id.into(),
        role: Some("inbox".into()),
        unread,
        total: 10,
    }
}

#[test]
fn encrypted_file_hides_content_and_needs_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let s = Store::open(&path, key(), true).unwrap();
        assert!(s.is_encrypted());
        s.upsert_account("microsoft", "user@example.com", "User", "secret-refresh")
            .unwrap();
    }
    let raw = std::fs::read(&path).unwrap();
    assert_ne!(&raw[..16], b"SQLite format 3\0");
    assert!(!raw.windows(16).any(|w| w == b"user@example.com"));

    // Reopening detects the encrypted form whatever the preference says now.
    let s = Store::open(&path, key(), false).unwrap();
    assert!(s.is_encrypted());
    assert_eq!(
        s.refresh_token(1).unwrap().as_deref(),
        Some("secret-refresh")
    );

    assert!(Store::open(&path, MasterKey::from_bytes([8; 32]), true).is_err());
}

#[test]
fn plain_file_still_seals_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let s = Store::open(&path, key(), false).unwrap();
    assert!(!s.is_encrypted());
    let id = s
        .upsert_account("microsoft", "user@example.com", "User", "secret-refresh")
        .unwrap();
    drop(s);
    let raw = std::fs::read(&path).unwrap();
    assert_eq!(&raw[..16], b"SQLite format 3\0");
    assert!(!raw.windows(14).any(|w| w == b"secret-refresh"));
    let s = Store::open(&path, key(), true).unwrap();
    assert!(!s.is_encrypted());
    assert_eq!(
        s.refresh_token(id).unwrap().as_deref(),
        Some("secret-refresh")
    );
}

#[test]
fn local_moves_deletes_flags_and_pending_queue() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("mail.db"), key(), true).unwrap();
    s.upsert_account("microsoft", "user@example.com", "User", "t")
        .unwrap();
    s.replace_folders(1, &[folder("inbox", 1), folder("trash", 0)])
        .unwrap();
    let mut read = msg("r", "inbox", 1, "read");
    read.is_read = true;
    s.apply_changes(1, "inbox", &[msg("u", "inbox", 2, "unread"), read], &[])
        .unwrap();
    let counts = |s: &Store, id: &str| {
        let f = s
            .folders(1)
            .unwrap()
            .into_iter()
            .find(|f| f.id == id)
            .unwrap();
        (f.unread, f.total)
    };

    let moved = s
        .move_local(1, &["u".into(), "r".into(), "missing".into()], "trash")
        .unwrap();
    assert_eq!(
        moved,
        [
            ("u".to_string(), "inbox".to_string()),
            ("r".into(), "inbox".into())
        ]
    );
    assert_eq!(counts(&s, "inbox"), (0, 8));
    assert_eq!(counts(&s, "trash"), (1, 12));
    // Moving into the folder it is already in changes nothing.
    assert!(s.move_local(1, &["u".into()], "trash").unwrap().is_empty());

    s.set_flag(1, "r", true).unwrap();
    assert!(
        s.messages(1, "trash", "", 10)
            .unwrap()
            .iter()
            .any(|m| m.id == "r" && m.flagged)
    );

    s.delete_local(1, &["u".into()]).unwrap();
    assert_eq!(counts(&s, "trash"), (0, 11));
    assert_eq!(s.messages(1, "trash", "", 10).unwrap().len(), 1);

    let a = s.push_pending(1, "{\"move\":1}").unwrap();
    let b = s.push_pending(1, "{\"move\":2}").unwrap();
    assert_eq!(
        s.pending().unwrap().iter().map(|p| p.0).collect::<Vec<_>>(),
        [a, b]
    );
    s.drop_pending(a).unwrap();
    assert_eq!(s.pending().unwrap(), [(b, 1, "{\"move\":2}".to_string())]);
}

#[test]
fn sync_changes_listing_search_and_read_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("mail.db"), key(), true).unwrap();
    let id = s
        .upsert_account("microsoft", "user@example.com", "User", "t")
        .unwrap();
    assert_eq!(id, 1);
    s.replace_folders(1, &[folder("inbox", 2), folder("old", 0)])
        .unwrap();
    s.apply_changes(
        1,
        "inbox",
        &[
            msg("a", "inbox", 100, "Hello"),
            msg("b", "inbox", 200, "50% off"),
        ],
        &[],
    )
    .unwrap();
    s.apply_changes(1, "old", &[msg("c", "old", 50, "x")], &[])
        .unwrap();

    let list = s.messages(1, "inbox", "", 100).unwrap();
    assert_eq!(
        list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(s.messages(1, "inbox", "50%", 100).unwrap().len(), 1);
    assert_eq!(s.messages(1, "inbox", "hello", 100).unwrap().len(), 1);
    assert_eq!(s.messages(1, "inbox", "_", 100).unwrap().len(), 0);

    s.set_read(1, "a", true).unwrap();
    s.set_read(1, "a", true).unwrap();
    let inbox = s
        .folders(1)
        .unwrap()
        .into_iter()
        .find(|f| f.id == "inbox")
        .unwrap();
    assert_eq!(inbox.unread, 1);

    s.put_body(
        1,
        "a",
        &MessageBody {
            text: "Body".into(),
            to: "me".into(),
            ..MessageBody::default()
        },
        &[],
    )
    .unwrap();
    assert_eq!(s.body(1, "a").unwrap().unwrap().text, "Body");

    // A folder that disappears takes its messages with it; the delta link of kept folders stays.
    s.set_delta_link(1, "inbox", Some("https://delta")).unwrap();
    s.replace_folders(1, &[folder("inbox", 1)]).unwrap();
    assert!(s.messages(1, "old", "", 10).unwrap().is_empty());
    assert_eq!(
        s.delta_link(1, "inbox").unwrap().as_deref(),
        Some("https://delta")
    );

    // Moved from inbox to old: the old folder keeps it, then inbox reports it removed.
    s.replace_folders(1, &[folder("inbox", 1), folder("old", 0)])
        .unwrap();
    s.apply_changes(1, "old", &[msg("b", "old", 200, "50% off")], &[])
        .unwrap();
    s.apply_changes(1, "inbox", &[], &["b".into()]).unwrap();
    assert_eq!(s.messages(1, "old", "", 10).unwrap().len(), 1);
    s.apply_changes(1, "inbox", &[], &["a".into()]).unwrap();
    assert!(s.body(1, "a").unwrap().is_none());
    assert!(s.messages(1, "inbox", "", 10).unwrap().is_empty());
}

#[test]
fn full_text_search_ignores_accents_and_follows_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("mail.db"), key(), true).unwrap();
    s.upsert_account("microsoft", "user@example.com", "User", "t")
        .unwrap();
    s.replace_folders(1, &[folder("inbox", 0), folder("old", 0)])
        .unwrap();
    s.apply_changes(
        1,
        "inbox",
        &[msg("a", "inbox", 100, "Votre facture d'électricité")],
        &[],
    )
    .unwrap();
    s.apply_changes(1, "old", &[msg("b", "old", 200, "Réunion lundi")], &[])
        .unwrap();
    let ids = |s: &Store, q: &str| {
        s.search(1, q, 50)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>()
    };

    assert_eq!(ids(&s, "electricite"), ["a"]);
    assert_eq!(ids(&s, "FACT"), ["a"]); // prefix of the last word, any case
    assert_eq!(ids(&s, "reunion"), ["b"]); // every folder
    assert_eq!(ids(&s, "example"), ["b", "a"]); // sender address, newest first
    assert!(ids(&s, "facture lundi").is_empty()); // all words required
    for trap in ["\"", "*", "-", "NEAR", "a OR", "(", "facture\"", "AND NOT"] {
        s.search(1, trap, 50).unwrap();
    }

    s.put_body(
        1,
        "b",
        &MessageBody {
            text: "Ordre du jour : budget 2027".into(),
            to: String::new(),
            ..MessageBody::default()
        },
        &[],
    )
    .unwrap();
    assert_eq!(ids(&s, "budget"), ["b"]);
    s.apply_changes(1, "old", &[msg("b", "old", 200, "Réunion annulée")], &[])
        .unwrap();
    assert_eq!(ids(&s, "annulee"), ["b"]);
    assert!(ids(&s, "lundi").is_empty());
    s.apply_changes(1, "old", &[], &["b".into()]).unwrap();
    assert!(ids(&s, "budget").is_empty());
}

#[test]
fn address_suggestions_prefer_contacts_then_frequent_senders() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("mail.db"), key(), true).unwrap();
    s.upsert_account("microsoft", "user@example.com", "User", "t")
        .unwrap();
    s.replace_folders(1, &[folder("inbox", 0)]).unwrap();
    let from = |id: &str, name: &str, addr: &str| {
        let mut m = msg(id, "inbox", 1, "x");
        m.from_name = name.into();
        m.from_addr = addr.into();
        m
    };
    s.apply_changes(
        1,
        "inbox",
        &[
            from("1", "Marie Curie", "marie@example.com"),
            from("2", "Marie Curie", "marie@example.com"),
            from("3", "Martin Shop", "news@martin.example.com"),
        ],
        &[],
    )
    .unwrap();
    s.replace_contacts(1, &[("Marc Dupont".into(), "marc@example.org".into())])
        .unwrap();
    let names = |q: &str| {
        s.address_suggestions(1, q, 10)
            .unwrap()
            .into_iter()
            .map(|(_, a)| a)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names("mar"),
        [
            "marc@example.org",
            "marie@example.com",
            "news@martin.example.com"
        ]
    );
    assert_eq!(names("curie"), ["marie@example.com"]);
    assert!(names("m").is_empty());
    assert!(names("%").is_empty());
}
