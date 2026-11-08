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
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct General {
    pub language: LangPref,
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
    /// Tracker blocker: remote images of messages are not loaded.
    pub block_remote_images: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            encrypt_local_copy: true,
            block_remote_images: false,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Accounts {
    /// Another Microsoft app registration to sign in with; empty for MailCraft's own.
    pub microsoft_client_id: String,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Prefs {
    pub general: General,
    pub appearance: Appearance,
    pub sync: Sync,
    pub privacy: Privacy,
    pub accounts: Accounts,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "mailcraft";
    const SCHEMA_VERSION: u32 = 1;
}
