//! "Add an account": Microsoft sign-in, or any other provider over IMAP + SMTP.

use egui::{RichText, Ui};
use mailcraft_imap::Server;

use crate::i18n::{Lang, T};
use crate::shell::Command;

/// What the user typed. The password lives only here until the account is added.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Form {
    pub email: String,
    pub name: String,
    pub password: String,
    pub user: String,
    pub imap_host: String,
    pub imap_port: String,
    pub smtp_host: String,
    pub smtp_port: String,
    /// Address the server fields were last filled for.
    filled_for: String,
}

impl std::fmt::Debug for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Form")
            .field("email", &self.email)
            .finish_non_exhaustive()
    }
}

impl Form {
    /// Fills the servers from the address (known provider, or `imap.<domain>`), once per address.
    fn fill_servers(&mut self) {
        let email = self.email.trim().to_owned();
        if email == self.filled_for || !email.contains('@') {
            return;
        }
        self.filled_for.clone_from(&email);
        let domain = email
            .rsplit_once('@')
            .map_or("", |d| d.1)
            .to_ascii_lowercase();
        let (imap, smtp, port) = match mailcraft_imap::preset(&email) {
            Some((i, s, p)) => (i.to_owned(), s.to_owned(), p),
            None => (format!("imap.{domain}"), format!("smtp.{domain}"), 465),
        };
        self.imap_host = imap;
        self.imap_port = "993".into();
        self.smtp_host = smtp;
        self.smtp_port = port.to_string();
    }

    /// The account to add, when everything needed is there.
    pub fn server(&self) -> Option<Server> {
        let email = self.email.trim();
        let (local, domain) = email.split_once('@')?;
        if local.is_empty() || !domain.contains('.') || self.password.is_empty() {
            return None;
        }
        let imap_port = self.imap_port.trim().parse().ok().filter(|p| *p > 0)?;
        let smtp_port = self.smtp_port.trim().parse().ok().filter(|p| *p > 0)?;
        let (imap_host, smtp_host) = (self.imap_host.trim(), self.smtp_host.trim());
        if imap_host.is_empty() || smtp_host.is_empty() {
            return None;
        }
        Some(Server {
            email: email.to_owned(),
            name: self.name.trim().to_owned(),
            user: self.user.trim().to_owned(),
            // App passwords are often shown in groups of four: "abcd efgh ijkl mnop".
            password: if imap_host.eq_ignore_ascii_case("imap.gmail.com") {
                self.password.replace(' ', "")
            } else {
                self.password.clone()
            },
            imap_host: imap_host.to_owned(),
            imap_port,
            smtp_host: smtp_host.to_owned(),
            smtp_port,
        })
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String, password: bool) {
    ui.horizontal(|ui| {
        let l = ui.add_sized([150.0, 20.0], egui::Label::new(label));
        ui.add(
            egui::TextEdit::singleline(value)
                .password(password)
                .desired_width(200.0),
        )
        .labelled_by(l.id);
    });
}

/// Draws the form. Returns the command to run and whether the dialog closes.
pub fn show(ui: &mut Ui, lang: Lang, form: &mut Form) -> (Option<Command>, bool) {
    let mut result = None;
    let mut close = false;
    ui.heading(T::AddAccount.get(lang).replace('\n', " "));
    ui.add_space(4.0);
    if ui.button(T::MicrosoftAccount.get(lang)).clicked() {
        result = Some(Command::AddMicrosoftAccount);
        close = true;
    }
    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new(T::OtherAccount.get(lang)).strong());
    field(ui, T::EmailAddress.get(lang), &mut form.email, false);
    form.fill_servers();
    let microsoft = mailcraft_imap::is_microsoft(form.email.trim());
    if microsoft {
        ui.label(RichText::new(T::UseMicrosoftButton.get(lang)).small());
    }
    field(ui, T::DisplayName.get(lang), &mut form.name, false);
    field(ui, T::AppPassword.get(lang), &mut form.password, true);
    ui.label(RichText::new(T::AppPasswordHelp.get(lang)).small());
    egui::CollapsingHeader::new(T::Servers.get(lang))
        .id_salt("imap_servers")
        .show(ui, |ui| {
            field(ui, T::ImapServer.get(lang), &mut form.imap_host, false);
            field(ui, T::ImapPort.get(lang), &mut form.imap_port, false);
            field(ui, T::SmtpServer.get(lang), &mut form.smtp_host, false);
            field(ui, T::SmtpPort.get(lang), &mut form.smtp_port, false);
            field(ui, T::UserName.get(lang), &mut form.user, false);
        });
    ui.add_space(8.0);
    let server = form.server().filter(|_| !microsoft);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(server.is_some(), egui::Button::new(T::AddButton.get(lang)))
            .clicked()
            && let Some(server) = server
        {
            result = Some(Command::AddImapAccount {
                server: Box::new(server),
            });
            close = true;
        }
        if ui.button(T::Cancel.get(lang)).clicked() {
            close = true;
        }
    });
    (result, close)
}
