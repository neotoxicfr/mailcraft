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

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Prefs {
    pub general: General,
    pub appearance: Appearance,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "mailcraft";
    const SCHEMA_VERSION: u32 = 1;
}
