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
