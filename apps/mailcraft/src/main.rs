// No console window behind the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use mailcraft_ui_egui::{Command, MailCraftApp};

struct App(MailCraftApp);

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.0.ui(ui) == Some(Command::Exit) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self) {
        self.0.flush();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MailCraft")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MailCraft",
        options,
        Box::new(|cc| Ok(Box::new(App(MailCraftApp::new(&cc.egui_ctx))))),
    )
}
