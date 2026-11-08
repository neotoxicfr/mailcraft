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
            cc: String::new(),
        },
    )
    .unwrap();
    s
}

#[test]
fn mailbox_lists_folders_and_reads_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_owned();
    let mut h = Harness::builder()
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
        );
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
