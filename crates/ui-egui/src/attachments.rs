//! Attachment safety: which files MailCraft refuses to open directly, and file names that are
//! safe to write on disk.

/// Extensions Windows would run rather than open: such files are only saved, never opened from
/// MailCraft, so a mail cannot start a program with one click.
const RISKY: &[&str] = &[
    "exe",
    "com",
    "scr",
    "pif",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "msi",
    "msp",
    "msix",
    "appx",
    "lnk",
    "url",
    "reg",
    "cpl",
    "jar",
    "dll",
    "sys",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "scf",
    "inf",
    "chm",
    "application",
    "gadget",
    "settingcontent-ms",
    "library-ms",
    "search-ms",
];

pub fn is_risky(name: &str) -> bool {
    let lower = name
        .trim()
        .trim_end_matches(['.', ' '])
        .to_ascii_lowercase();
    lower
        .rsplit_once('.')
        .is_some_and(|(_, ext)| RISKY.contains(&ext))
}

/// A file name without folders, reserved characters or Windows device names.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4);
    if cleaned.is_empty() || reserved {
        format!("piece-jointe-{cleaned}")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::{is_risky, safe_file_name};

    #[test]
    fn risky_and_safe_names() {
        assert!(is_risky("Facture.PDF.exe"));
        assert!(is_risky("photo.jpg.lnk. "));
        assert!(!is_risky("Facture.pdf"));
        assert!(!is_risky("sans-extension"));
        assert_eq!(
            safe_file_name("..\\..\\Windows\\evil.pdf"),
            ".._.._Windows_evil.pdf"
        );
        assert_eq!(safe_file_name("con.txt"), "piece-jointe-con.txt");
        assert_eq!(safe_file_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(safe_file_name(""), "piece-jointe-");
    }
}
