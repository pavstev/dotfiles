//! The per-machine settings file (AGENTS.md "Settings"): NAME="value" lines in
//! ~/.config/dotfiles/settings, mode 600, never in git. The last NAME line wins.

use super::os;
use crate::out::warn;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The folder for this machine's private files: `~/.config/dotfiles`.
pub fn conf_dir() -> PathBuf {
    os::home().join(".config").join("dotfiles")
}
/// The settings file.
pub fn settings_path() -> PathBuf {
    conf_dir().join("settings")
}
/// The private rules, free text; never in the repo.
pub fn owner_md() -> PathBuf {
    conf_dir().join("owner.md")
}

/// The value of NAME, or "".
pub fn get(name: &str) -> String {
    crate::setting(&os::home(), name)
}

/// A value can't hold " \ $ ` or a new line: zsh sources the file.
pub fn valid_value(v: &str) -> bool {
    !v.contains(['"', '\\', '$', '`', '\n'])
}

/// The new file text: the first NAME line replaced, later ones dropped, or the line
/// added at the end. Other lines stay.
pub fn replaced(old: &str, name: &str, value: &str) -> String {
    let key = format!("{}=", name);
    let line = format!("{}=\"{}\"", name, value);
    let mut out = String::new();
    let mut done = false;
    let mut lines: Vec<&str> = old.split('\n').collect();
    if old.ends_with('\n') || old.is_empty() {
        lines.pop();
    }
    for l in lines {
        if l.starts_with(&key) {
            if !done {
                out.push_str(&line);
                out.push('\n');
            }
            done = true;
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    if !done {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Save NAME="value" (mode 600). A new file starts as a copy of settings.example.
/// When the file is a link, the file it points to changes and the link stays.
pub fn set(name: &str, value: &str) -> bool {
    if !valid_value(value) {
        warn(&format!("{} not saved: a value can't hold \" \\ $ ` or a new line", name));
        return false;
    }
    let conf = conf_dir();
    if fs::create_dir_all(&conf).is_err() {
        return false;
    }
    set_mode(&conf, 0o700);
    let mut dest = settings_path();
    for _ in 0..8 {
        if !os::is_symlink(&dest) {
            break;
        }
        let Some(l) = os::readlink(&dest) else { return false };
        dest = if l.is_absolute() { l } else { dest.parent().unwrap_or(Path::new(".")).join(l) };
    }
    let old = if dest.is_file() {
        fs::read_to_string(&dest).unwrap_or_default()
    } else {
        let ex = super::repo().join("settings.example");
        fs::read_to_string(ex).unwrap_or_default()
    };
    let text = replaced(&old, name, value);
    let dir = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
    // The new file goes next to the old one, so the rename replaces it in one step.
    let Ok(mut tmp) = tempfile::Builder::new().prefix(".settings.").tempfile_in(&dir) else { return false };
    if tmp.write_all(text.as_bytes()).is_err() {
        return false;
    }
    if tmp.persist(&dest).is_err() {
        return false;
    }
    set_mode(&dest, 0o600);
    true
}

/// Sets the file mode. Does nothing on Windows.
#[cfg(unix)]
pub fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(mode));
}
/// Sets the file mode. Does nothing on Windows.
#[cfg(not(unix))]
pub fn set_mode(_p: &Path, _mode: u32) {}

/// "owner" or "shared" (AGENTS.md "Profiles"): DOTFILES_PROFILE, then PROFILE in the
/// settings, then "owner" when the private rules exist (saved as PROFILE), else "shared".
pub fn profile() -> String {
    let mut p = std::env::var("DOTFILES_PROFILE").unwrap_or_default();
    if p.is_empty() {
        p = get("PROFILE");
    }
    if p.is_empty() && owner_md().is_file() {
        p = "owner".into();
        let _ = crate::out::capture(|| set("PROFILE", "owner"));
    }
    if p == "owner" || p == "shared" { p } else { "shared".into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replaced_cases() {
        // the first line of a name is replaced and later ones dropped
        assert_eq!(replaced("A=\"1\"\nB=\"2\"\nA=\"3\"\n", "A", "x"), "A=\"x\"\nB=\"2\"\n");
        // a missing name is appended
        assert_eq!(replaced("# note\nB=\"2\"", "A", "x"), "# note\nB=\"2\"\nA=\"x\"\n");
        assert_eq!(replaced("", "A", ""), "A=\"\"\n");
        // a longer name with the same start is no match
        assert_eq!(replaced("AB=\"1\"\n", "A", "x"), "AB=\"1\"\nA=\"x\"\n");
    }
    #[test]
    fn bad_values() {
        for v in ["a\"b", "a\\b", "$HOME", "`x`", "a\nb"] {
            assert!(!valid_value(v));
        }
        assert!(valid_value("Jane Doe"));
    }
}
