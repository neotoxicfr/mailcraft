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
    EmptyList,
    ReadingEmpty,
    NoSubject,
    To,
    Cc,
    Loading,
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    Older,
    // status bar
    StatusNoAccount,
    SigningIn,
    Cancel,
    Syncing,
    MessagesWord,
    UpToDate,
    Error,
    CacheEncrypted,
    CacheClear,
    AvailableLater,
    SaveFailed,
    // actions and dialogs
    SelectFirst,
    OwnFoldersOnly,
    ItemsWord,
    ItemsSelected,
    MarkReadUnread,
    MoveTo,
    NewSubfolder,
    SimplifiedView,
    OriginalView,
    NothingSimplified,
    Open,
    SaveAs,
    RiskyFile,
    OpensInBrowser,
    AttachmentSaved,
    SearchServer,
    SearchingServer,
    ServerFound,
    FolderName,
    Ok,
    ConfirmPermanentTitle,
    DeleteForGood,
    ConfirmPermanentBody,
    ConfirmDeleteFolderBody,
    Offline,
    PendingChanges,
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
            Self::EmptyList => (
                "Aucun élément à afficher dans cette vue.",
                "There are no items to show in this view.",
            ),
            Self::ReadingEmpty => ("Sélectionnez un élément à lire", "Select an item to read"),
            Self::NoSubject => ("(Aucun objet)", "(No subject)"),
            Self::To => ("À", "To"),
            Self::Cc => ("Cc", "Cc"),
            Self::Loading => ("Chargement…", "Loading…"),
            Self::Today => ("Aujourd'hui", "Today"),
            Self::Yesterday => ("Hier", "Yesterday"),
            Self::ThisWeek => ("Cette semaine", "This week"),
            Self::LastWeek => ("La semaine dernière", "Last week"),
            Self::ThisMonth => ("Plus tôt ce mois-ci", "Earlier this month"),
            Self::Older => ("Plus ancien", "Older"),
            Self::StatusNoAccount => ("Aucun compte configuré", "No account set up"),
            Self::SigningIn => (
                "Connexion à Microsoft dans votre navigateur…",
                "Signing in to Microsoft in your browser…",
            ),
            Self::Cancel => ("Annuler", "Cancel"),
            Self::Syncing => ("Synchronisation", "Syncing"),
            Self::MessagesWord => ("messages", "messages"),
            Self::UpToDate => ("À jour", "Up to date"),
            Self::Error => ("Erreur", "Error"),
            Self::CacheEncrypted => ("Copie locale chiffrée", "Local copy encrypted"),
            Self::CacheClear => ("Copie locale non chiffrée", "Local copy not encrypted"),
            Self::AvailableLater => (
                "Arrive dans une prochaine version",
                "Coming in a later version",
            ),
            Self::SelectFirst => ("Sélectionnez d'abord un élément", "Select an item first"),
            Self::OwnFoldersOnly => (
                "Seulement pour vos propres dossiers",
                "Only for folders you created",
            ),
            Self::ItemsWord => ("élément(s)", "item(s)"),
            Self::ItemsSelected => ("éléments sélectionnés", "items selected"),
            Self::MarkReadUnread => ("Marquer comme lu/non lu", "Mark as read/unread"),
            Self::MoveTo => ("Déplacer vers", "Move to"),
            Self::NewSubfolder => ("Nouveau sous-dossier", "New subfolder"),
            Self::SimplifiedView => ("Rendu simplifié", "Simplified view"),
            Self::OriginalView => ("Afficher l'original", "Show original"),
            Self::NothingSimplified => (
                "Ce message n'a rien à afficher en rendu simplifié.",
                "This message has nothing to show in the simplified view.",
            ),
            Self::Open => ("Ouvrir", "Open"),
            Self::SaveAs => ("Enregistrer sous…", "Save as…"),
            Self::RiskyFile => (
                "Fichier qui lance un programme : enregistrez-le, MailCraft ne l'ouvre pas",
                "This file runs a program: save it, MailCraft does not open it",
            ),
            Self::OpensInBrowser => ("S'ouvre dans le navigateur :", "Opens in the browser:"),
            Self::AttachmentSaved => ("Pièce jointe enregistrée", "Attachment saved"),
            Self::SearchServer => ("Chercher aussi sur le serveur", "Also search the server"),
            Self::SearchingServer => ("Recherche sur le serveur…", "Searching the server…"),
            Self::ServerFound => (
                "résultat(s) trouvé(s) sur le serveur",
                "result(s) found on the server",
            ),
            Self::FolderName => ("Nom du dossier", "Folder name"),
            Self::Ok => ("OK", "OK"),
            Self::ConfirmPermanentTitle => ("Supprimer définitivement ?", "Delete permanently?"),
            Self::DeleteForGood => ("Supprimer définitivement", "Delete permanently"),
            Self::ConfirmPermanentBody => (
                "élément(s) seront supprimés définitivement. Cette action est irréversible.",
                "item(s) will be deleted permanently. This cannot be undone.",
            ),
            Self::ConfirmDeleteFolderBody => (
                "ce dossier et son contenu iront dans Éléments supprimés.",
                "this folder and its content will go to Deleted Items.",
            ),
            Self::Offline => ("Hors connexion", "Offline"),
            Self::PendingChanges => ("modification(s) en attente", "change(s) waiting"),
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
