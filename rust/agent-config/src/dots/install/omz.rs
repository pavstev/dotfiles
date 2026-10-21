//! 30-omz-plugins: Oh My Zsh and its custom plugins (cloned, not vendored). Each piece is
//! checked first: present ones are skipped, missing ones cloned. Tolerant: a failed
//! network step warns and goes on.

use super::super::os;
use super::{curl_text, quiet};
use crate::out::{self, info, ok, ok_if_changed, skip, warn};
use std::path::{Path, PathBuf};
use std::process::Command;

const INSTALLER: &str = "https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh";

/// (repo URL, folder name) of each plugin.
const PLUGINS: [(&str, &str); 4] = [
    ("https://github.com/zsh-users/zsh-autosuggestions", "zsh-autosuggestions"),
    ("https://github.com/zdharma-continuum/fast-syntax-highlighting", "fast-syntax-highlighting"),
    ("https://github.com/Aloxaf/fzf-tab", "fzf-tab"),
    ("https://github.com/zsh-users/zsh-completions", "zsh-completions"),
];

/// $ZSH_CUSTOM when set and not empty, else ~/.oh-my-zsh/custom.
fn custom_dir(zsh_custom: Option<String>, home: &Path) -> PathBuf {
    match zsh_custom {
        Some(z) if !z.is_empty() => PathBuf::from(z),
        _ => home.join(".oh-my-zsh").join("custom"),
    }
}

fn clone_plugin(zc: &Path, url: &str, name: &str) {
    let dir = zc.join("plugins").join(name);
    if dir.is_dir() {
        skip(&format!("plugin {name}"));
        return;
    }
    info(&format!("cloning {name}…"));
    if quiet("git", &["clone", "--depth=1", url, &dir.to_string_lossy()]) {
        ok(name);
    } else {
        warn(&format!("clone of {name} failed (network?) — re-run: ./dotfiles"));
    }
}

/// Run the Oh My Zsh installer. REPO is emptied: our runner exports REPO=<repo path>, but
/// the installer reads $REPO as its GitHub slug (ohmyzsh/ohmyzsh), and the clone would 404.
fn install_omz() -> bool {
    let Some(script) = curl_text(&["-fsSL", INSTALLER]).filter(|s| !s.is_empty()) else { return false };
    Command::new("sh")
        .args(["-c", &script, "", "--unattended"])
        .env("RUNZSH", "no")
        .env("CHSH", "no")
        .env("KEEP_ZSHRC", "yes")
        .env("REPO", "")
        .status()
        .is_ok_and(|s| s.success())
}

pub fn step(_repo: &Path) {
    out::step("Oh My Zsh + plugins");
    // The installer needs zsh. Without it, say so once and try nothing (step 05 installs it).
    if !os::have("zsh") {
        warn("zsh is missing, so Oh My Zsh is not installed. Install zsh, then run dots again.");
        return;
    }
    let home = os::home();
    if home.join(".oh-my-zsh").is_dir() {
        skip("Oh My Zsh");
    } else {
        info("Installing Oh My Zsh…");
        if install_omz() {
            ok("Oh My Zsh installed");
        } else {
            warn("Oh My Zsh install failed (network?) — skipping plugins. Re-run: ./dotfiles");
            return;
        }
    }
    let zc = custom_dir(std::env::var("ZSH_CUSTOM").ok(), &home);
    let _ = std::fs::create_dir_all(zc.join("plugins"));
    for (url, name) in PLUGINS {
        clone_plugin(&zc, url, name);
    }
    ok_if_changed("Oh My Zsh + plugins ready");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_dir_default_and_override() {
        let h = Path::new("/h");
        assert_eq!(custom_dir(None, h), Path::new("/h/.oh-my-zsh/custom"));
        assert_eq!(custom_dir(Some(String::new()), h), Path::new("/h/.oh-my-zsh/custom"));
        assert_eq!(custom_dir(Some("/z".into()), h), Path::new("/z"));
    }

    #[test]
    fn an_existing_plugin_folder_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("plugins/fzf-tab")).unwrap();
        // A folder that is there never reaches git: no "cloning" line, no warning.
        let ((), lines) = out::capture(|| clone_plugin(d.path(), "https://invalid.example/none", "fzf-tab"));
        assert!(lines.iter().all(|(l, err)| !*err && !l.contains("cloning")), "{lines:?}");
    }
}
