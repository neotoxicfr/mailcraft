// A desktop app: never a console window, even in development builds (errors go to stderr,
// which the launcher can redirect to a file).
#![windows_subsystem = "windows"]

mod art;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod webview;

use eframe::egui;
use mailcraft_ui_egui::{Command, MailCraftApp};

struct App {
    ui: MailCraftApp,
    /// The window is hidden in the notification area.
    hidden: bool,
    /// Leaving for real (File › Exit or the tray menu), not hiding.
    quitting: bool,
    #[cfg(windows)]
    original: webview::OriginalView,
    #[cfg(windows)]
    tray: Option<desktop::Tray>,
    #[cfg(windows)]
    instance: std::sync::mpsc::Receiver<desktop::Action>,
}

impl App {
    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}

impl eframe::App for App {
    /// Also runs while the window is hidden: tray menu, second launch, mail sync, notifications.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            let mut actions: Vec<desktop::Action> = self.instance.try_iter().collect();
            if let Some(tray) = &self.tray {
                actions.extend(std::iter::from_fn(|| tray.poll()));
            }
            for action in actions {
                match action {
                    desktop::Action::Show => self.show_window(ctx),
                    desktop::Action::Quit => {
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        if self.hidden {
            self.ui.background(ctx);
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        if let Some((title, text)) = self.ui.take_notification()
            && (self.hidden || !focused)
        {
            #[cfg(windows)]
            desktop::notify(&title, &text);
            #[cfg(not(windows))]
            let _ = (title, text);
        }
        // Closing hides the window when the user asked for it (mail keeps arriving).
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self.ui.prefs().startup.close_to_tray
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.ui.ui(ui) == Some(Command::Exit) {
            self.quitting = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(windows)]
        {
            let want = if self.hidden {
                None
            } else {
                self.ui.original_view()
            };
            self.original
                .update(frame, ui.ctx().pixels_per_point(), want);
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
    // A second launch (Start menu, sign-in while already running…) shows the first window.
    #[cfg(windows)]
    if desktop::already_running() {
        return Ok(());
    }
    let start_hidden = std::env::args().any(|a| a == "--minimized");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MailCraft")
            .with_app_id("mailcraft")
            .with_icon(egui::IconData {
                rgba: art::icon_rgba(64),
                width: 64,
                height: 64,
            })
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 520.0])
            .with_visible(!start_hidden),
        ..Default::default()
    };
    eframe::run_native(
        "MailCraft",
        options,
        Box::new(move |cc| {
            let ui = MailCraftApp::new(&cc.egui_ctx);
            Ok(Box::new(App {
                hidden: start_hidden,
                quitting: false,
                #[cfg(windows)]
                tray: desktop::Tray::new(&cc.egui_ctx, ui.french()),
                #[cfg(windows)]
                instance: desktop::listen(&cc.egui_ctx),
                // The WebView keeps its profile (cache of images) apart from Edge's.
                #[cfg(windows)]
                original: webview::OriginalView::new(dirs_webview()),
                ui,
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
