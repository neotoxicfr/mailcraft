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

/// Text colours, Fluent/Outlook values: near-black and a secondary grey on white; near-white and
/// a lighter grey on dark. Each one reaches 4.5:1 (WCAG AA) on every pane background.
pub const TEXT_LIGHT: Color32 = Color32::from_rgb(0x24, 0x24, 0x24);
pub const WEAK_LIGHT: Color32 = Color32::from_rgb(0x5C, 0x5C, 0x5C);
pub const TEXT_DARK: Color32 = Color32::from_rgb(0xED, 0xEB, 0xE9);
pub const WEAK_DARK: Color32 = Color32::from_rgb(0xAD, 0xAD, 0xAD);
/// Panel background of the dark theme (egui's), and the dark ribbon.
pub const DARK_PANEL: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x1B);
/// Share of the accent in the background of a selected row.
pub const SELECTION_TINT: f32 = 0.18;
/// WCAG AA for normal text.
pub const MIN_CONTRAST: f32 = 4.5;
/// WCAG AA for the outline of controls (check boxes, fields, buttons).
pub const MIN_CONTROL_CONTRAST: f32 = 3.0;
/// Outline of controls, Fluent's accessible stroke: 3:1 or more on both themes.
pub const CONTROL_BORDER: Color32 = Color32::from_rgb(0x8A, 0x88, 0x86);

/// Relative luminance (WCAG 2.x).
pub fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// Contrast ratio between two opaque colours, 1 to 21.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// `color`, moved towards black (light backgrounds) or white (dark ones) just enough to reach
/// `min` against every background in `bgs`.
pub fn readable(color: Color32, bgs: &[Color32], min: f32) -> Color32 {
    let dark_bg = bgs.iter().any(|b| luminance(*b) < 0.18);
    let target = if dark_bg {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    (0..=20)
        .map(|i| mix(color, target, i as f32 / 20.0))
        .find(|c| bgs.iter().all(|b| contrast(*c, *b) >= min))
        .unwrap_or(target)
}

/// Accent for solid fills under white text (title bar, Send button): dark enough for white.
pub fn accent_fill(pref: AccentPref) -> Color32 {
    readable(accent(pref), &[Color32::WHITE], MIN_CONTRAST)
}

/// Accent for text and icons on the panes of the current theme (unread subjects, counts,
/// links, ribbon icons, the selected tab).
pub fn accent_text(fill: Color32, dark: bool) -> Color32 {
    if dark {
        readable(fill, &[DARK_PANEL, ribbon_fill(true)], MIN_CONTRAST)
    } else {
        readable(fill, &[Color32::WHITE, ribbon_fill(false)], MIN_CONTRAST)
    }
}

/// Applies theme and accent to both light and dark styles. Call when the preferences change.
pub fn apply(ctx: &Context, appearance: &Appearance) {
    ctx.set_theme(match appearance.theme {
        ThemePref::System => ThemePreference::System,
        ThemePref::Light => ThemePreference::Light,
        ThemePref::Dark => ThemePreference::Dark,
    });
    let fill = accent_fill(appearance.accent);
    for theme in [Theme::Light, Theme::Dark] {
        let dark = theme == Theme::Dark;
        ctx.style_mut_of(theme, |style| {
            // egui's small text is 9 px: too small to read comfortably (Outlook stays at 11+).
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(11.5));
            let v = &mut style.visuals;
            v.selection.bg_fill = fill;
            v.selection.stroke.color = Color32::WHITE;
            v.hyperlink_color = accent_text(fill, dark);
            v.widgets.hovered.bg_stroke.color = fill;
            v.widgets.active.bg_fill = fill;
            let (text, weak) = if dark {
                (TEXT_DARK, WEAK_DARK)
            } else {
                (TEXT_LIGHT, WEAK_LIGHT)
            };
            v.widgets.noninteractive.fg_stroke.color = text;
            v.widgets.inactive.fg_stroke.color = text;
            v.widgets.hovered.fg_stroke.color = text;
            v.weak_text_color = Some(weak);
            // Check boxes and fields must stand out from the pane (3:1).
            v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, CONTROL_BORDER);
            // Windows 11 controls: a field-coloured button with an outline, a light tint on
            // hover, darker when pressed or open, rounded corners; menus and drop-down lists
            // float with a soft shadow.
            let (field, hover, press) = if dark {
                (
                    Color32::from_rgb(0x2D, 0x2D, 0x2D),
                    Color32::from_rgb(0x38, 0x38, 0x38),
                    Color32::from_rgb(0x24, 0x24, 0x24),
                )
            } else {
                (
                    Color32::WHITE,
                    Color32::from_rgb(0xF5, 0xF5, 0xF5),
                    Color32::from_rgb(0xE6, 0xE6, 0xE6),
                )
            };
            v.widgets.inactive.weak_bg_fill = field;
            v.widgets.inactive.bg_fill = field;
            v.widgets.hovered.weak_bg_fill = hover;
            v.widgets.hovered.bg_fill = hover;
            v.widgets.active.weak_bg_fill = press;
            v.widgets.open.weak_bg_fill = press;
            v.widgets.open.bg_stroke = egui::Stroke::new(1.0, fill);
            for w in [
                &mut v.widgets.noninteractive,
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = egui::CornerRadius::same(4);
            }
            v.menu_corner_radius = egui::CornerRadius::same(6);
            v.window_corner_radius = egui::CornerRadius::same(8);
            v.popup_shadow = egui::epaint::Shadow {
                offset: [0, 4],
                blur: 14,
                spread: 0,
                color: Color32::from_black_alpha(if dark { 110 } else { 45 }),
            };
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
            style.spacing.interact_size.y = 24.0;
            if dark {
                v.panel_fill = DARK_PANEL;
            } else {
                v.panel_fill = Color32::WHITE;
                v.window_fill = Color32::WHITE;
                v.extreme_bg_color = Color32::WHITE;
                v.faint_bg_color = Color32::from_rgb(0xF3, 0xF2, 0xF1);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What a selected row looks like: the accent at 22 % over the pane.
    fn selected(pane: Color32, accent: Color32) -> Color32 {
        mix(pane, accent, SELECTION_TINT)
    }

    #[test]
    fn every_text_colour_reaches_wcag_aa() {
        let accents = [
            OUTLOOK_BLUE,
            MAILCRAFT_TEAL,
            Color32::from_rgb(0xFF, 0xB9, 0x00), // Windows yellow
            Color32::from_rgb(0x00, 0xB7, 0xC3), // Windows cyan
            Color32::from_rgb(0xE3, 0x00, 0x8C), // Windows pink
            Color32::from_rgb(0x10, 0x10, 0x40), // very dark navy
        ];
        let light = [Color32::WHITE, ribbon_fill(false)];
        let dark = [DARK_PANEL, ribbon_fill(true)];
        for bg in light {
            assert!(contrast(TEXT_LIGHT, bg) >= MIN_CONTRAST);
            assert!(contrast(WEAK_LIGHT, bg) >= MIN_CONTRAST, "{bg:?}");
        }
        for bg in dark {
            assert!(contrast(TEXT_DARK, bg) >= MIN_CONTRAST);
            assert!(contrast(WEAK_DARK, bg) >= MIN_CONTRAST, "{bg:?}");
        }
        for a in accents {
            let fill = readable(a, &[Color32::WHITE], MIN_CONTRAST);
            assert!(contrast(Color32::WHITE, fill) >= MIN_CONTRAST, "{a:?}");
            for (is_dark, bgs, text, weak) in [
                (false, light, TEXT_LIGHT, WEAK_LIGHT),
                (true, dark, TEXT_DARK, WEAK_DARK),
            ] {
                let t = accent_text(fill, is_dark);
                for bg in bgs {
                    let c = contrast(t, bg);
                    assert!(c >= MIN_CONTRAST, "{a:?} dark={is_dark}: {c:.2}");
                }
                let row = selected(bgs[0], t);
                assert!(
                    contrast(text, row) >= MIN_CONTRAST,
                    "{a:?} dark={is_dark} text on selection"
                );
                let w = contrast(weak, row);
                assert!(
                    w >= MIN_CONTRAST,
                    "{a:?} dark={is_dark} weak on selection: {w:.2}"
                );
            }
        }
        for bg in light.into_iter().chain(dark) {
            assert!(
                contrast(CONTROL_BORDER, bg) >= MIN_CONTROL_CONTRAST,
                "{bg:?}"
            );
        }
        // Outlook blue is already fine on white: kept as it is.
        assert_eq!(accent_text(OUTLOOK_BLUE, false), OUTLOOK_BLUE);
    }
}
