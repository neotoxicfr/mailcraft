//! Drives the real window through egui_kittest (accessibility tree), settings kept in memory.

use craft_settings::Location;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailcraft_ui_egui::MailCraftApp;
use mailcraft_ui_egui::settings::{LangPref, ThemePref};

fn harness(lang: LangPref) -> Harness<'static, Option<MailCraftApp>> {
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            move |ui, app: &mut Option<MailCraftApp>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = MailCraftApp::with_location(ui.ctx(), Location::Memory);
                    app.update_prefs(|p| p.general.language = lang);
                    app
                });
                app.ui(ui);
            },
            None,
        )
}

fn sample_store(dir: &std::path::Path) -> mailcraft_store::Store {
    use mailcraft_store::{Folder, MasterKey, MessageBody, MessageSummary, Store};
    let mut s = Store::open(&dir.join("mail.db"), MasterKey::from_bytes([3; 32]), true).unwrap();
    s.upsert_account("microsoft", "user@example.com", "User", "token")
        .unwrap();
    let folder = |id: &str, name: &str, role: Option<&str>, parent: Option<&str>, unread| Folder {
        account_id: 1,
        id: id.into(),
        parent_id: parent.map(Into::into),
        name: name.into(),
        role: role.map(Into::into),
        unread,
        total: 2,
    };
    s.replace_folders(
        1,
        &[
            folder("in", "Boîte de réception", Some("inbox"), None, 1),
            folder("sent", "Éléments envoyés", Some("sentitems"), None, 0),
            folder("bills", "Factures", None, Some("in"), 0),
            folder("trash", "Éléments supprimés", Some("deleteditems"), None, 0),
        ],
    )
    .unwrap();
    let msg = |id: &str, subject: &str, read| MessageSummary {
        account_id: 1,
        id: id.into(),
        folder_id: "in".into(),
        from_name: "Example Shop".into(),
        from_addr: "news@example.com".into(),
        subject: subject.into(),
        preview: "Bonjour".into(),
        received: 1_794_130_200,
        is_read: read,
        has_attachments: true,
        flagged: false,
    };
    s.apply_changes(
        1,
        "in",
        &[msg("m1", "Votre facture", false), msg("m2", "Promo", true)],
        &[],
    )
    .unwrap();
    s.put_body(
        1,
        "m1",
        &MessageBody {
            text: "Montant : 42 €".into(),
            to: "user@example.com".into(),
            ..MessageBody::default()
        },
        &[],
    )
    .unwrap();
    s
}

fn mailbox(path: std::path::PathBuf) -> Harness<'static, Option<MailCraftApp>> {
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            move |ui, app: &mut Option<MailCraftApp>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = MailCraftApp::with_location(ui.ctx(), Location::Memory);
                    app.update_prefs(|p| p.general.language = LangPref::Fr);
                    app.attach_store(sample_store(&path));
                    app
                });
                app.ui(ui);
            },
            None,
        )
}

#[test]
fn mailbox_lists_folders_and_reads_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = mailbox(dir.path().to_owned());
    h.run();
    h.get_by_label("Factures");
    h.get_by_label("Example Shop — Promo");
    h.get_by_label("Copie locale chiffrée");
    h.get_by_label("Example Shop — Votre facture").click();
    h.run();
    h.get_by_label("Montant : 42 €");
    h.get_by_label("À : user@example.com");
}

#[test]
fn french_shell_shows_outlook_layout() {
    let mut h = harness(LangPref::Fr);
    h.run();
    h.get_by_label("Accueil");
    h.get_by_label("Aucun élément à afficher dans cette vue.");
    h.get_by_label("Sélectionnez un élément à lire");
}

#[test]
fn view_tab_switches_language_and_theme() {
    let mut h = harness(LangPref::Fr);
    h.run();
    h.get_by_label("Affichage").click();
    h.run();
    h.get_by_label("Thème");
    if let Some(app) = h.state_mut() {
        app.update_prefs(|p| p.general.language = LangPref::En);
    }
    h.run();
    h.get_by_label("Home");
    if let Some(app) = h.state_mut() {
        app.update_prefs(|p| p.appearance.theme = ThemePref::Dark);
    }
    h.run();
    assert_eq!(h.ctx.theme(), egui::Theme::Dark);
}

#[test]
fn delete_two_messages_then_undo() {
    use egui::{Key, Modifiers};
    let dir = tempfile::tempdir().unwrap();
    let mut h = mailbox(dir.path().to_owned());
    h.run();
    h.get_by_label("Example Shop — Votre facture").click();
    h.run();
    h.get_by_label("Example Shop — Promo")
        .click_modifiers(Modifiers::COMMAND);
    h.run();
    h.get_by_label("2 éléments sélectionnés");

    h.key_press(Key::Delete);
    h.run();
    assert!(h.query_by_label("Example Shop — Promo").is_none());
    assert!(h.query_by_label("Example Shop — Votre facture").is_none());

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    h.get_by_label("Example Shop — Promo");
    h.get_by_label("Example Shop — Votre facture");
}

#[test]
fn shift_delete_asks_before_deleting_for_good() {
    use egui::{Key, Modifiers};
    let dir = tempfile::tempdir().unwrap();
    let mut h = mailbox(dir.path().to_owned());
    h.run();
    h.get_by_label("Example Shop — Promo").click();
    h.run();
    h.key_press_modifiers(Modifiers::SHIFT, Key::Delete);
    h.run();
    h.get_by_label("Supprimer définitivement ?");
    h.get_by_label("Annuler").click();
    h.run();
    h.get_by_label("Example Shop — Promo");

    h.key_press_modifiers(Modifiers::SHIFT, Key::Delete);
    h.run();
    h.get_by_label("Supprimer définitivement").click();
    h.run();
    assert!(h.query_by_label("Example Shop — Promo").is_none());
    // Nothing to put back after a permanent deletion.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert!(h.query_by_label("Example Shop — Promo").is_none());
}

#[test]
fn search_box_filters_all_folders_and_escape_clears() {
    use egui::Key;
    let dir = tempfile::tempdir().unwrap();
    let mut h = mailbox(dir.path().to_owned());
    h.run();
    let search = h.get_by_role(egui::accesskit::Role::TextInput);
    search.focus();
    search.type_text("promo");
    h.run();
    h.get_by_label("Example Shop — Promo");
    assert!(h.query_by_label("Example Shop — Votre facture").is_none());
    h.get_by_label("Chercher aussi sur le serveur");

    h.key_press(Key::Escape);
    h.run();
    h.get_by_label("Example Shop — Votre facture");
    assert!(h.query_by_label("Chercher aussi sur le serveur").is_none());
}

#[test]
fn reply_opens_in_the_reading_pane_prefilled() {
    use egui_kittest::kittest::by;
    let dir = tempfile::tempdir().unwrap();
    let mut h = mailbox(dir.path().to_owned());
    h.run();
    h.get_by_label("Example Shop — Votre facture").click();
    h.run();
    h.get_by_label("Répondre").click();
    h.run();
    h.get(
        by().role(egui::accesskit::Role::TextInput)
            .value("RE : Votre facture"),
    );
    h.get(
        by().role(egui::accesskit::Role::TextInput)
            .value("Example Shop <news@example.com>"),
    );
    h.get_by_label("Envoyer");
    // Nothing typed: closing asks nothing and gives the pane back to the message.
    h.get_by_label("Fermer").click();
    h.run();
    h.get_by_label("Montant : 42 €");

    // Typed text: closing asks first.
    h.get_by_label("Répondre").click();
    h.run();
    let editor = h.get_by_label("message");
    editor.focus();
    editor.type_text("Merci");
    h.run();
    h.get_by_label("Fermer").click();
    h.run();
    h.get_by_label("Enregistrer ce message dans les brouillons ?");
    h.get_by_label("Ne pas enregistrer").click();
    h.run();
    h.get_by_label("Montant : 42 €");
}

#[test]
fn cleanup_view_flags_suspects_and_starts_in_simulation() {
    use mailcraft_store::{MasterKey, MessageSummary, SenderAnalysis, Store};
    let dir = tempfile::tempdir().unwrap();
    {
        // Same database as the sample, plus a phishing sender and the analysis of both.
        let mut s = sample_store(dir.path());
        let mut m = MessageSummary {
            account_id: 1,
            id: "p1".into(),
            folder_id: "in".into(),
            from_name: "PayPal".into(),
            from_addr: "security@pay-check.example".into(),
            subject: "Compte suspendu".into(),
            preview: String::new(),
            received: 1_794_000_000,
            is_read: false,
            has_attachments: false,
            flagged: false,
        };
        s.apply_changes(1, "in", &[m.clone()], &[]).unwrap();
        m.id = "p2".into();
        s.apply_changes(1, "in", &[m], &[]).unwrap();
        let suspect = SenderAnalysis {
            warnings: "impersonation:paypal,dmarc-fail".into(),
            category: "suspect".into(),
            dmarc: 2,
            analyzed_at: 1,
            ..Default::default()
        };
        s.put_sender_analysis(1, "security@pay-check.example", &suspect)
            .unwrap();
        let news = SenderAnalysis {
            one_click: Some("https://example.com/u".into()),
            category: "newsletter".into(),
            spf: 1,
            dkim: 1,
            dmarc: 1,
            analyzed_at: 1,
            ..Default::default()
        };
        s.put_sender_analysis(1, "news@example.com", &news).unwrap();
        drop(s);
        let _ = Store::open(
            &dir.path().join("mail.db"),
            MasterKey::from_bytes([3; 32]),
            true,
        )
        .unwrap();
    }
    let path = dir.path().to_owned();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .build_ui_state(
            move |ui, app: &mut Option<MailCraftApp>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = MailCraftApp::with_location(ui.ctx(), Location::Memory);
                    app.update_prefs(|p| p.general.language = LangPref::Fr);
                    let store = mailcraft_store::Store::open(
                        &path.join("mail.db"),
                        mailcraft_store::MasterKey::from_bytes([3; 32]),
                        true,
                    )
                    .unwrap();
                    app.attach_store(store);
                    app
                });
                app.ui(ui);
            },
            None,
        );
    h.run();
    h.get_by_label("Analyser les expéditeurs").click();
    h.run();
    h.get_by_label("Tri des expéditeurs");
    h.get_by_label("SUSPECT");
    h.get_by_label("En un clic");
    h.get_by_label("Mode simulation");
    h.get_by_label_contains("Simulation : MailCraft montre");
    h.get_by_label("Suspects (1)").click();
    h.run();
    assert!(h.query_by_label("news@example.com").is_none());
    assert!(
        h.query_all_by_label("security@pay-check.example")
            .next()
            .is_some()
    );
    h.get_by_label("Retour aux messages").click();
    h.run();
    h.get_by_label("Example Shop — Votre facture");
}

#[test]
fn welcome_assistant_steps_and_options_view() {
    let mut h = harness(LangPref::Fr);
    h.run();
    if let Some(app) = h.state_mut() {
        app.start_welcome();
    }
    h.run();
    h.get_by_label("Bienvenue dans MailCraft");
    h.get_by_label("Suivant").click();
    h.run();
    h.get_by_label("Vie privée");
    h.get_by_label("Bloquer les images distantes et les pixels espions")
        .click();
    h.run();
    h.get_by_label("Passer").click();
    h.run();
    let prefs = h.state().as_ref().unwrap().prefs().clone();
    assert!(prefs.general.onboarded && prefs.privacy.block_remote_images);
    assert!(h.query_by_label("Vie privée").is_none());

    // File › Options shows every setting; simulation is on by default.
    h.get_by_label("Fichier").click();
    h.run();
    h.get_by_label("Options").click();
    h.run();
    h.get_by_label("Lancer MailCraft à l'ouverture de session Windows")
        .click();
    h.run();
    let prefs = h.state().as_ref().unwrap().prefs().clone();
    assert!(prefs.startup.with_windows && prefs.cleanup.dry_run);
    h.get_by_label("Retour aux messages").click();
    h.run();
    h.get_by_label("Accueil");
}

#[test]
fn screener_lists_new_senders_until_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_owned();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            move |ui, app: &mut Option<MailCraftApp>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = MailCraftApp::with_location(ui.ctx(), Location::Memory);
                    app.update_prefs(|p| {
                        p.general.language = LangPref::Fr;
                        p.cleanup.screener = true;
                        p.cleanup.screener_since = 0;
                    });
                    app.attach_store(sample_store(&path));
                    app
                });
                app.ui(ui);
            },
            None,
        );
    h.run();
    h.get_by_label("À valider").click();
    h.run();
    h.get_by_label("Example Shop — Votre facture").click();
    h.run();
    h.get_by_label("Nouvel expéditeur : voulez-vous recevoir ses mails ?");
    h.get_by_label("Accepter").click();
    h.run();
    assert!(h.query_by_label("Example Shop — Votre facture").is_none());
    assert!(
        h.query_by_label("Example Shop — Promo").is_none(),
        "same sender: accepted once for all its mail"
    );
}
