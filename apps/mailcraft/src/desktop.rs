//! Windows desktop integration: notification-area icon with its menu, one running instance (a
//! second launch brings the first one forward), and new-mail notifications.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    actions: Receiver<Action>,
}

impl Tray {
    /// Must run on the UI thread (the window's event loop receives the icon's messages).
    pub fn new(ctx: &egui::Context, french: bool) -> Option<Self> {
        let (open, quit) = if french {
            ("Ouvrir MailCraft", "Quitter")
        } else {
            ("Open MailCraft", "Quit")
        };
        let show_item = MenuItem::new(open, true, None);
        let quit_item = MenuItem::new(quit, true, None);
        let menu = Menu::new();
        menu.append_items(&[&show_item, &PredefinedMenuItem::separator(), &quit_item])
            .ok()?;
        let icon = tray_icon::Icon::from_rgba(crate::art::icon_rgba(32), 32, 32).ok()?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("MailCraft")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;
        let (tx, rx) = mpsc::channel();
        let (show_id, quit_id) = (show_item.id().clone(), quit_item.id().clone());
        let (wake, events) = (ctx.clone(), tx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let action = if e.id == quit_id {
                Some(Action::Quit)
            } else if e.id == show_id {
                Some(Action::Show)
            } else {
                None
            };
            if let Some(a) = action {
                let _ = events.send(a);
                wake.request_repaint();
            }
        }));
        let wake = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            // A left click on the icon opens the window; the menu is on the right button.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = tx.send(Action::Show);
                wake.request_repaint();
            }
        }));
        Some(Self {
            _icon: tray,
            actions: rx,
        })
    }

    pub fn poll(&self) -> Option<Action> {
        self.actions.try_recv().ok()
    }
}

fn port_file() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("mailcraft").join("instance.port"))
}

/// If MailCraft already runs, asks it to show its window and returns `true` (this launch should
/// stop). Otherwise returns `false`.
pub fn already_running() -> bool {
    let Some(port) = port_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|p| p.trim().parse::<u16>().ok())
    else {
        return false;
    };
    let Ok(mut stream) =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(400))
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    if stream.write_all(b"MailCraft:show\n").is_err() {
        return false;
    }
    // Only a MailCraft answers "ok": another program on a reused port is not us.
    let mut answer = [0u8; 2];
    stream.read_exact(&mut answer).is_ok() && &answer == b"ok"
}

/// Listens for later launches; each one sends `Action::Show`.
pub fn listen(ctx: &egui::Context) -> Receiver<Action> {
    let (tx, rx): (Sender<Action>, Receiver<Action>) = mpsc::channel();
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
        return rx;
    };
    if let (Some(file), Ok(addr)) = (port_file(), listener.local_addr()) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, addr.port().to_string());
    }
    let wake = ctx.clone();
    std::thread::Builder::new()
        .name("mailcraft-instance".into())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
                let mut buf = [0u8; 15];
                if stream.read_exact(&mut buf).is_ok() && &buf == b"MailCraft:show\n" {
                    let _ = stream.write_all(b"ok");
                    let _ = tx.send(Action::Show);
                    wake.request_repaint();
                }
            }
        })
        .ok();
    rx
}

/// A Windows notification. Installed MailCraft has its own identity (set on its Start-menu
/// shortcut by the installer); a development build borrows PowerShell's so toasts still show.
pub fn notify(title: &str, text: &str) {
    use tauri_winrt_notification::Toast;
    let installed = std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.to_str()
                .map(|s| s.to_ascii_lowercase().contains("\\program files"))
        })
        .unwrap_or(false);
    let app_id = if installed {
        APP_ID
    } else {
        Toast::POWERSHELL_APP_ID
    };
    let _ = Toast::new(app_id).title(title).text1(text).show();
}

/// AppUserModelID of the installed app (also in packaging/windows/mailcraft.wxs).
pub const APP_ID: &str = "Neotoxic.MailCraft";
