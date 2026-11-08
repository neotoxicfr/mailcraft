//! Light/dark visuals and the accent colour, Outlook classic style.

use egui::{Color32, Context, Theme, ThemePreference};

use crate::settings::{AccentPref, Appearance, ThemePref};

/// Outlook's brand blue.
pub const OUTLOOK_BLUE: Color32 = Color32::from_rgb(0x0F, 0x6C, 0xBD);
/// MailCraft's own accent, a teal distinct from Outlook.
pub const MAILCRAFT_TEAL: Color32 = Color32::from_rgb(0x0E, 0x80, 0x74);

pub fn accent(pref: AccentPref) -> Color32 {
    match pref {
        AccentPref::Outlook => OUTLOOK_BLUE,
        AccentPref::MailCraft => MAILCRAFT_TEAL,
        AccentPref::Windows => windows_accent().unwrap_or(OUTLOOK_BLUE),
    }
}

/// The accent colour picked in Windows settings (`AccentColor`, stored as 0xAABBGGRR).
#[cfg(windows)]
fn windows_accent() -> Option<Color32> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\DWM")
        .ok()?;
    let v: u32 = key.get_value("AccentColor").ok()?;
    let [r, g, b, _] = v.to_le_bytes();
    Some(Color32::from_rgb(r, g, b))
}

#[cfg(not(windows))]
fn windows_accent() -> Option<Color32> {
    None
}

/// Applies theme and accent to both light and dark styles. Call when the preferences change.
pub fn apply(ctx: &Context, appearance: &Appearance) {
    ctx.set_theme(match appearance.theme {
        ThemePref::System => ThemePreference::System,
        ThemePref::Light => ThemePreference::Light,
        ThemePref::Dark => ThemePreference::Dark,
    });
    let accent = accent(appearance.accent);
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            let v = &mut style.visuals;
            v.selection.bg_fill = accent;
            v.hyperlink_color = accent;
            v.widgets.hovered.bg_stroke.color = accent;
            v.widgets.active.bg_fill = accent;
            if theme == Theme::Light {
                v.panel_fill = Color32::WHITE;
                v.window_fill = Color32::WHITE;
                v.extreme_bg_color = Color32::WHITE;
                v.faint_bg_color = Color32::from_rgb(0xF3, 0xF2, 0xF1);
                // White fields on white panes need an outline (search box, lists).
                v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, separator(false));
            }
        });
    }
}

/// Background of the ribbon body (below the tab row).
pub fn ribbon_fill(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(0x2B, 0x2B, 0x2B)
    } else {
        Color32::from_rgb(0xF3, 0xF2, 0xF1)
    }
}

/// Thin lines between panes.
pub fn separator(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(0x44, 0x44, 0x44)
    } else {
        Color32::from_rgb(0xE1, 0xDF, 0xDD)
    }
}
