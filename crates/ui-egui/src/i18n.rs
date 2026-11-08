//! French and English UI text. One enum so a missing translation is a compile error.

use crate::settings::LangPref;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    pub fn resolve(pref: LangPref) -> Self {
        match pref {
            LangPref::Fr => Self::Fr,
            LangPref::En => Self::En,
            LangPref::System => from_locale(sys_locale::get_locale().as_deref()),
        }
    }
}

fn from_locale(locale: Option<&str>) -> Lang {
    match locale {
        Some(l) if l.to_ascii_lowercase().starts_with("fr") => Lang::Fr,
        _ => Lang::En,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum T {
    // ribbon tabs
    TabFile,
    TabHome,
    TabSendReceive,
    TabFolder,
    TabView,
    // Home
    GroupNew,
    NewEmail,
    GroupDelete,
    Delete,
    Archive,
    GroupRespond,
    Reply,
    ReplyAll,
    Forward,
    GroupMove,
    Move,
    GroupTags,
    UnreadRead,
    Flag,
    GroupSort,
    Unsubscribe,
    Analyze,
    // Send/Receive
    SendReceiveAll,
    // Folder
    NewFolder,
    RenameFolder,
    DeleteFolder,
    // File
    AddAccount,
    Exit,
    // View
    GroupAppearance,
    Theme,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    Accent,
    AccentOutlook,
    AccentWindows,
    AccentMailCraft,
    Language,
    LangSystem,
    // panes
    Favorites,
    NoAccount,
    SearchHint,
    ColFrom,
    ColSubject,
    ColReceived,
    ColSize,
    EmptyList,
    ReadingEmpty,
    StatusNoAccount,
    AvailableLater,
    ComingSoon,
    SaveFailed,
}

impl T {
    pub fn get(self, lang: Lang) -> &'static str {
        let (fr, en) = self.pair();
        match lang {
            Lang::Fr => fr,
            Lang::En => en,
        }
    }

    fn pair(self) -> (&'static str, &'static str) {
        match self {
            Self::TabFile => ("Fichier", "File"),
            Self::TabHome => ("Accueil", "Home"),
            Self::TabSendReceive => ("Envoi/réception", "Send / Receive"),
            Self::TabFolder => ("Dossier", "Folder"),
            Self::TabView => ("Affichage", "View"),
            Self::GroupNew => ("Nouveau", "New"),
            Self::NewEmail => ("Nouveau\ncourrier", "New\nEmail"),
            Self::GroupDelete => ("Supprimer", "Delete"),
            Self::Delete => ("Supprimer", "Delete"),
            Self::Archive => ("Archiver", "Archive"),
            Self::GroupRespond => ("Répondre", "Respond"),
            Self::Reply => ("Répondre", "Reply"),
            Self::ReplyAll => ("Répondre à tous", "Reply All"),
            Self::Forward => ("Transférer", "Forward"),
            Self::GroupMove => ("Déplacer", "Move"),
            Self::Move => ("Déplacer", "Move"),
            Self::GroupTags => ("Indicateurs", "Tags"),
            Self::UnreadRead => ("Lu/non lu", "Unread/Read"),
            Self::Flag => ("Assurer un suivi", "Follow Up"),
            Self::GroupSort => ("Tri", "Clean up"),
            Self::Unsubscribe => ("Se\ndésabonner", "Unsubscribe"),
            Self::Analyze => ("Analyser les\nexpéditeurs", "Analyze\nsenders"),
            Self::SendReceiveAll => (
                "Envoyer/recevoir\ntous les dossiers",
                "Send/Receive\nAll Folders",
            ),
            Self::NewFolder => ("Nouveau\ndossier", "New\nFolder"),
            Self::RenameFolder => ("Renommer", "Rename"),
            Self::DeleteFolder => ("Supprimer le dossier", "Delete Folder"),
            Self::AddAccount => ("Ajouter\nun compte", "Add\nAccount"),
            Self::Exit => ("Quitter", "Exit"),
            Self::GroupAppearance => ("Apparence", "Appearance"),
            Self::Theme => ("Thème", "Theme"),
            Self::ThemeSystem => ("Comme Windows", "Same as Windows"),
            Self::ThemeLight => ("Clair", "Light"),
            Self::ThemeDark => ("Sombre", "Dark"),
            Self::Accent => ("Couleur", "Colour"),
            Self::AccentOutlook => ("Bleu Outlook", "Outlook blue"),
            Self::AccentWindows => ("Couleur de Windows", "Windows colour"),
            Self::AccentMailCraft => ("Sarcelle MailCraft", "MailCraft teal"),
            Self::Language => ("Langue", "Language"),
            Self::LangSystem => ("Comme Windows", "Same as Windows"),
            Self::Favorites => ("Favoris", "Favorites"),
            Self::NoAccount => (
                "Aucun compte.\nFichier › Ajouter un compte",
                "No account.\nFile › Add Account",
            ),
            Self::SearchHint => ("Rechercher dans la boîte aux lettres", "Search the mailbox"),
            Self::ColFrom => ("De", "From"),
            Self::ColSubject => ("Objet", "Subject"),
            Self::ColReceived => ("Reçu", "Received"),
            Self::ColSize => ("Taille", "Size"),
            Self::EmptyList => (
                "Aucun élément à afficher dans cette vue.",
                "There are no items to show in this view.",
            ),
            Self::ReadingEmpty => ("Sélectionnez un élément à lire", "Select an item to read"),
            Self::StatusNoAccount => ("Aucun compte configuré", "No account set up"),
            Self::AvailableLater => (
                "Disponible une fois un compte ajouté",
                "Available once an account is added",
            ),
            Self::ComingSoon => (
                "Arrive dans une prochaine version",
                "Coming in a later version",
            ),
            Self::SaveFailed => (
                "Impossible d'enregistrer les réglages",
                "Could not save settings",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Lang, from_locale};

    #[test]
    fn locale_detection() {
        assert_eq!(from_locale(Some("fr-FR")), Lang::Fr);
        assert_eq!(from_locale(Some("FR")), Lang::Fr);
        assert_eq!(from_locale(Some("en-US")), Lang::En);
        assert_eq!(from_locale(None), Lang::En);
    }
}
