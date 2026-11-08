//! Pictures of the main views, light and dark, for a visual review:
//! `cargo test -p mailcraft-ui-egui --test shots -- --ignored` writes them to `target/shots/`.
//! Needs a GPU (wgpu), so it is not part of the normal test run.

use craft_settings::Location;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailcraft_ui_egui::MailCraftApp;
use mailcraft_ui_egui::settings::{LangPref, ThemePref};

fn store(dir: &std::path::Path) -> mailcraft_store::Store {
    use mailcraft_store::{Folder, MasterKey, MessageBody, MessageSummary, Store};
    let mut s = Store::open(&dir.join("mail.db"), MasterKey::from_bytes([3; 32]), true).unwrap();
    s.upsert_account("microsoft", "user@example.com", "Camille Martin", "token")
        .unwrap();
    let folder = |id: &str, name: &str, role: Option<&str>, parent: Option<&str>, unread| Folder {
        account_id: 1,
        id: id.into(),
        parent_id: parent.map(Into::into),
        name: name.into(),
        role: role.map(Into::into),
        unread,
        total: 9,
    };
    s.replace_folders(
        1,
        &[
            folder("in", "Boîte de réception", Some("inbox"), None, 3),
            folder("drafts", "Brouillons", Some("drafts"), None, 0),
            folder("sent", "Éléments envoyés", Some("sentitems"), None, 0),
            folder("trash", "Éléments supprimés", Some("deleteditems"), None, 0),
            folder("junk", "Courrier indésirable", Some("junkemail"), None, 2),
            folder("archive", "Archive", Some("archive"), None, 0),
            folder("bills", "Factures", None, Some("in"), 0),
            folder("trips", "Voyages", None, Some("in"), 1),
        ],
    )
    .unwrap();
    let now = 1_794_130_200;
    let rows = [
        (
            "m1",
            "Example Shop",
            "news@shop.example",
            "Votre facture de novembre",
            "Bonjour Camille, votre facture est disponible dans votre espace client.",
            0,
            false,
            true,
            false,
        ),
        (
            "m2",
            "Alex Dupont",
            "alex@example.org",
            "Réunion de jeudi",
            "Je te propose 14 h, dis-moi si ça te va.",
            3_600,
            false,
            false,
            true,
        ),
        (
            "m3",
            "Banque Exemple",
            "contact@banque.example",
            "Nouveau relevé disponible",
            "Votre relevé d'octobre est en ligne.",
            7_200,
            true,
            true,
            false,
        ),
        (
            "m4",
            "Lettre du jardin",
            "info@jardin.example",
            "Que planter en novembre ?",
            "Ail, échalotes et fèves : c'est le moment.",
            90_000,
            true,
            false,
            false,
        ),
        (
            "m5",
            "Service Livraison",
            "suivi@colis.example",
            "Votre colis arrive demain",
            "Créneau 8 h – 13 h, signature demandée.",
            180_000,
            false,
            false,
            false,
        ),
        (
            "m6",
            "Camille Martin",
            "user@example.com",
            "Photos du week-end",
            "Les voici en pièce jointe.",
            700_000,
            true,
            true,
            false,
        ),
    ];
    let list: Vec<MessageSummary> = rows
        .iter()
        .map(
            |(id, name, addr, subject, preview, ago, read, att, flag)| MessageSummary {
                account_id: 1,
                id: (*id).into(),
                folder_id: "in".into(),
                from_name: (*name).into(),
                from_addr: (*addr).into(),
                subject: (*subject).into(),
                preview: (*preview).into(),
                received: now - ago,
                is_read: *read,
                has_attachments: *att,
                flagged: *flag,
            },
        )
        .collect();
    s.apply_changes(1, "in", &list, &[]).unwrap();
    s.put_body(
        1,
        "m2",
        &MessageBody {
            text: "Salut Camille,\n\nJe te propose jeudi à 14 h pour faire le point sur le projet.\nDis-moi si ça te va, sinon vendredi matin.\n\nBonne journée,\nAlex".into(),
            html: "<p>Salut Camille,</p><p>Je te propose <b>jeudi à 14 h</b> pour faire le point sur le projet.<br>Dis-moi si ça te va, sinon vendredi matin.</p><ul><li>Bilan du mois</li><li>Prochaines étapes</li></ul><p>Bonne journée,<br>Alex</p>".into(),
            to: "Camille Martin <user@example.com>".into(),
            cc: "Équipe <team@example.org>".into(),
        },
        &[],
    )
    .unwrap();
    s
}

fn harness(dir: std::path::PathBuf, dark: bool) -> Harness<'static, Option<MailCraftApp>> {
    Harness::builder()
        .with_size(egui::vec2(1366.0, 768.0))
        .wgpu()
        .build_ui_state(
            move |ui, app: &mut Option<MailCraftApp>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = MailCraftApp::with_location(ui.ctx(), Location::Memory);
                    app.update_prefs(|p| {
                        p.general.language = LangPref::Fr;
                        p.appearance.theme = if dark {
                            ThemePref::Dark
                        } else {
                            ThemePref::Light
                        };
                    });
                    app.attach_store(store(&dir));
                    app
                });
                app.ui(ui);
            },
            None,
        )
}

fn save(h: &mut Harness<'_, Option<MailCraftApp>>, name: &str) {
    h.run();
    let image = h.render().expect("render");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore = "needs a GPU; writes pictures for a visual review"]
fn shots() {
    for dark in [false, true] {
        let theme = if dark { "sombre" } else { "clair" };
        let dir = tempfile::tempdir().unwrap();
        let mut h = harness(dir.path().to_owned(), dark);
        h.run();
        h.get_by_label("Alex Dupont — Réunion de jeudi").click();
        save(&mut h, &format!("1-lecture-{theme}"));

        h.get_by_label("Fichier").click();
        h.run();
        h.get_by_label("Options").click();
        save(&mut h, &format!("2-options-{theme}"));
        h.get_by_label("Retour aux messages").click();
        h.run();

        h.get_by_label("Accueil").click();
        h.run();
        if let Some(n) = h.query_by_label("Nouveau courrier") {
            n.click();
        } else {
            h.get_by_label("Nouveau\ncourrier").click();
        }
        save(&mut h, &format!("3-redaction-{theme}"));
    }
}
