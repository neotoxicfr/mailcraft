//! User preferences, stored by `craft-settings` in `config.toml`.

use craft_settings::Settings;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

/// Accent colour: Outlook's blue, the one chosen in Windows, or MailCraft's own.
#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum AccentPref {
    #[default]
    Outlook,
    Windows,
    MailCraft,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LangPref {
    #[default]
    System,
    Fr,
    En,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemePref,
    pub accent: AccentPref,
    /// The list groups messages by conversation.
    pub conversations: bool,
    /// Messages open in their original look (Edge, scripts off) instead of the simplified view.
    pub original_view: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct General {
    pub language: LangPref,
    /// The welcome assistant was finished or skipped.
    pub onboarded: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Sync {
    /// How far back the first sync of a folder goes.
    pub months: u32,
}

impl Default for Sync {
    fn default() -> Self {
        Self { months: 6 }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Privacy {
    /// Encrypt the local copy of the mail (applies when it is created).
    pub encrypt_local_copy: bool,
    /// No remote image of a message is loaded (tracking pixels included).
    pub block_remote_images: bool,
    /// Tracking pixels (1 px images, hidden images, known open-tracking addresses) are never
    /// loaded, even when the other remote images are.
    pub block_trackers: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            encrypt_local_copy: true,
            block_remote_images: false,
            block_trackers: true,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Accounts {
    /// Another Microsoft app registration to sign in with; empty for MailCraft's own.
    pub microsoft_client_id: String,
    /// Signature added to new messages and replies, by account address.
    pub signatures: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Prefs {
    pub general: General,
    pub appearance: Appearance,
    pub sync: Sync,
    pub privacy: Privacy,
    pub accounts: Accounts,
    pub cleanup: Cleanup,
    pub startup: Startup,
    /// Where the main window was, to open it there again.
    pub window: WindowPlace,
    /// Folder pane: account and folder order, folded sections.
    pub folders: FolderLayout,
    /// What MailCraft does with new mail by itself.
    pub rules: Vec<mailcraft_sync::Rule>,
    /// Words added to the spell checker (names, jargon).
    pub spell_words: Vec<String>,
    pub away: mailcraft_sync::Away,
    /// Optional AI: provider, model, server address (the key is in the OS vault).
    pub ai: mailcraft_ai::Choice,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Cleanup {
    /// Clean-up actions are only logged (on until the user turns it off).
    pub dry_run: bool,
    /// Addresses or domains clean-up never touches.
    pub allow_list: Vec<String>,
    /// Screener: mail from senders never seen before waits in « À valider ».
    pub screener: bool,
    /// When the screener was turned on (unix seconds): only senders first seen after it count.
    pub screener_since: i64,
}

impl Default for Cleanup {
    fn default() -> Self {
        Self {
            dry_run: true,
            allow_list: Vec::new(),
            screener: false,
            screener_since: 0,
        }
    }
}

impl Settings for Prefs {
    const APP_ID: &'static str = "mailcraft";
    const SCHEMA_VERSION: u32 = 1;
}

/// Windows integration.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Startup {
    /// Start MailCraft when the user signs in to Windows.
    pub with_windows: bool,
    /// …in the notification area rather than as a window.
    pub minimized: bool,
    /// The close button hides the window to the notification area (mail keeps syncing).
    pub close_to_tray: bool,
}

/// How the folder pane is arranged. Accounts are named by address (stable across a rebuild of
/// the local copy), folders by their id within the account.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct FolderLayout {
    /// Accounts in this order (addresses); the others after them.
    pub accounts: Vec<String>,
    /// Per account address: folder ids in the user's order (siblings compared only).
    pub folders: std::collections::BTreeMap<String, Vec<String>>,
    /// Folded entries: `fav`, `a:<address>`, `f:<address>:<folder id>`.
    pub folded: std::collections::BTreeSet<String>,
}

impl FolderLayout {
    pub fn toggle(&mut self, key: &str) {
        if !self.folded.remove(key) {
            self.folded.insert(key.to_owned());
        }
    }

    /// Puts `item` just before (or after) `target` within `current` (the order as shown) and
    /// records the result in `list`.
    pub fn place(
        list: &mut Vec<String>,
        current: &[String],
        item: &str,
        target: &str,
        after: bool,
    ) {
        if item == target || !current.iter().any(|x| x == item) {
            return;
        }
        let mut order: Vec<String> = current.iter().filter(|x| *x != item).cloned().collect();
        let Some(t) = order.iter().position(|x| x == target) else {
            return;
        };
        order.insert(if after { t + 1 } else { t }, item.to_owned());
        list.retain(|x| !order.contains(x));
        list.extend(order);
    }

    /// Where Move up / Move down puts `item` in `current`: (entry to go next to, after it).
    /// `None` at either end.
    pub fn neighbour(current: &[String], item: &str, up: bool) -> Option<(String, bool)> {
        let i = current.iter().position(|x| x == item)?;
        if up {
            Some((current.get(i.checked_sub(1)?)?.clone(), false))
        } else {
            Some((current.get(i + 1)?.clone(), true))
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::FolderLayout;

    #[test]
    fn moving_keeps_other_entries() {
        let shown: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(
            FolderLayout::neighbour(&shown, "c", true),
            Some(("b".to_owned(), false))
        );
        assert_eq!(FolderLayout::neighbour(&shown, "a", true), None); // already first
        assert_eq!(FolderLayout::neighbour(&shown, "c", false), None); // already last
        let mut list = vec!["x".to_owned()];
        FolderLayout::place(&mut list, &shown, "c", "b", false);
        assert_eq!(list, ["x", "a", "c", "b"]);
        let mut list3 = Vec::new();
        FolderLayout::place(&mut list3, &shown, "a", "c", true);
        assert_eq!(list3, ["b", "c", "a"]);
        FolderLayout::place(&mut list3, &shown, "c", "a", false);
        assert_eq!(list3, ["c", "a", "b"]);
        let mut l = FolderLayout::default();
        l.toggle("fav");
        assert!(l.folded.contains("fav"));
        l.toggle("fav");
        assert!(l.folded.is_empty());
    }
}

/// Position (outer, top-left) and size (inner) of the main window, in points; `None` before the
/// first run.
#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct WindowPlace {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub maximized: bool,
}

impl WindowPlace {
    /// A saved place that can be used: a sensible size, and a position not far off any screen
    /// (a monitor unplugged since).
    pub fn usable(&self) -> Option<([f32; 2], [f32; 2])> {
        let (x, y, w, h) = (self.x?, self.y?, self.width?, self.height?);
        let fits = (200.0..10_000.0).contains(&w) && (200.0..10_000.0).contains(&h);
        let near = (-200.0..8_000.0).contains(&x) && (-200.0..5_000.0).contains(&y);
        (fits && near).then_some(([x, y], [w, h]))
    }
}
