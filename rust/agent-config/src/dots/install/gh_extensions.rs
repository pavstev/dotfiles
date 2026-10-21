//! 55-gh-extensions: gh CLI extensions. gh-dash (the GitHub PR and issue dashboard) is a gh
//! extension, run as `gh dash`. Idempotent and tolerant.

use super::super::os;
use super::{quiet, stdout_of};
use crate::out::{self, info, ok, skip, warn};
use std::path::{Path, PathBuf};

/// The folder gh keeps its extensions in: $XDG_DATA_HOME (when set and not empty), else ~/.local/share.
fn extensions_dir(xdg: Option<String>, home: &Path) -> PathBuf {
    match xdg {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => home.join(".local").join("share"),
    }
    .join("gh")
    .join("extensions")
}

pub fn step(_repo: &Path) {
    out::step("gh CLI extensions");
    os::brew_env();
    if !os::have("gh") {
        warn("gh not found — skipping extensions.");
        return;
    }
    // `gh extension list` wants a login on some systems, so the extension folder counts too.
    let listed = stdout_of("gh", &["extension", "list"]).contains("dlvhdr/gh-dash");
    if listed || extensions_dir(std::env::var("XDG_DATA_HOME").ok(), &os::home()).join("gh-dash").is_dir() {
        skip("gh-dash extension");
    } else {
        info("Installing gh-dash extension…");
        if quiet("gh", &["extension", "install", "dlvhdr/gh-dash"]) {
            ok("gh-dash installed (run: gh dash)");
        } else {
            warn("gh-dash install failed (network or gh auth) — retry: gh extension install dlvhdr/gh-dash");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_folder() {
        let h = Path::new("/h");
        assert_eq!(extensions_dir(None, h), Path::new("/h/.local/share/gh/extensions"));
        assert_eq!(extensions_dir(Some(String::new()), h), Path::new("/h/.local/share/gh/extensions"));
        assert_eq!(extensions_dir(Some("/x".into()), h), Path::new("/x/gh/extensions"));
    }
}
