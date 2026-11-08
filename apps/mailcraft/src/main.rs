// A desktop app: never a console window, even in development builds (errors go to stderr,
// which the launcher can redirect to a file).
#![windows_subsystem = "windows"]

#[cfg(windows)]
mod webview;

use eframe::egui;
use mailcraft_ui_egui::{Command, MailCraftApp};

struct App {
    ui: MailCraftApp,
    #[cfg(windows)]
    original: webview::OriginalView,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.ui.ui(ui) == Some(Command::Exit) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(windows)]
        {
            self.original
                .update(frame, ui.ctx().pixels_per_point(), self.ui.original_view());
            if self.original.failed() {
                self.ui.original_unavailable();
            }
        }
        #[cfg(not(windows))]
        let _ = frame;
    }

    fn on_exit(&mut self) {
        self.ui.flush();
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
        Box::new(|cc| {
            Ok(Box::new(App {
                ui: MailCraftApp::new(&cc.egui_ctx),
                // The WebView keeps its profile (cache, cookies of images) apart from Edge's.
                #[cfg(windows)]
                original: webview::OriginalView::new(dirs_webview()),
            }))
        }),
    )
}

#[cfg(windows)]
fn dirs_webview() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| {
        std::path::PathBuf::from(d)
            .join("mailcraft")
            .join("webview")
    })
}
