//! Start with Windows: the per-user `Run` key (no administrator rights, removed by the
//! uninstaller too). `--minimized` starts in the notification area.

#[cfg(windows)]
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const NAME: &str = "MailCraft";

/// The command Windows runs at sign-in.
pub fn command_line(exe: &std::path::Path, minimized: bool) -> String {
    let mut cmd = format!("\"{}\"", exe.display());
    if minimized {
        cmd.push_str(" --minimized");
    }
    cmd
}

/// Writes or removes the entry so it matches the settings.
#[cfg(windows)]
pub fn apply(enabled: bool, minimized: bool) -> std::io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, KEY_SET_VALUE)?;
    if enabled {
        let exe = std::env::current_exe()?;
        key.set_value(NAME, &command_line(&exe, minimized))
    } else {
        match key.delete_value(NAME) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

#[cfg(not(windows))]
pub fn apply(_enabled: bool, _minimized: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::command_line;

    #[test]
    fn quoted_path_and_flag() {
        let exe = std::path::Path::new(r"C:\Program Files\MailCraft\mailcraft.exe");
        assert_eq!(
            command_line(exe, true),
            r#""C:\Program Files\MailCraft\mailcraft.exe" --minimized"#
        );
        assert_eq!(
            command_line(exe, false),
            r#""C:\Program Files\MailCraft\mailcraft.exe""#
        );
    }
}
