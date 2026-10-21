//! 40-rust-toolchain: a Rust toolchain via rustup (Homebrew installs the manager, not a
//! toolchain). Tolerant: a failed network step warns and goes on.

use super::super::os;
use super::{quiet, run, stdout_of};
use crate::out::{self, info, ok_if_changed, skip, warn};
use std::path::Path;

/// The rustup settings file names a default toolchain (a line that starts with it).
fn has_default_toolchain(settings: &str) -> bool {
    settings.lines().any(|l| l.starts_with("default_toolchain"))
}

/// Homebrew's rustup also reads its own fallback settings file, so it can show a toolchain
/// while ~/.rustup has no default. A rustup-init copy in ~/.cargo/bin then finds none and
/// every cargo call fails. So the toolchain counts as set only when the user settings name
/// a default too.
fn toolchain_set() -> bool {
    if !quiet("rustup", &["show", "active-toolchain"]) {
        return false;
    }
    let home = std::env::var_os("RUSTUP_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| os::home().join(".rustup"), std::path::PathBuf::from);
    std::fs::read_to_string(home.join("settings.toml")).is_ok_and(|t| has_default_toolchain(&t))
}

pub fn step(_repo: &Path) {
    out::step("Rust toolchain");
    os::brew_env();
    if os::have("rustup") {
        if toolchain_set() {
            skip("Rust toolchain");
        } else {
            info("Installing Rust stable toolchain…");
            if !run("rustup", &["default", "stable"]) {
                warn("rustup default stable failed (network?) — re-run: ./dotfiles");
                return;
            }
        }
    } else if os::have("rustup-init") {
        info("Initialising rustup…");
        if !run("rustup-init", &["-y", "--no-modify-path", "--default-toolchain", "stable"]) {
            warn("rustup-init failed (network?) — re-run: ./dotfiles");
            return;
        }
    } else {
        warn("rustup not found — skipping Rust toolchain.");
        return;
    }
    quiet("rustup", &["component", "add", "clippy", "rust-analyzer"]);
    let v = stdout_of("rustc", &["--version"]);
    let v = v.trim_end_matches(['\n', '\r']);
    ok_if_changed(&format!("Rust ready ({})", if v.is_empty() { "rustc pending" } else { v }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toolchain_line() {
        assert!(has_default_toolchain("version = \"12\"\ndefault_toolchain = \"stable\"\n"));
        assert!(!has_default_toolchain("version = \"12\"\n [overrides]\n  default_toolchain = \"x\"\n"));
        assert!(!has_default_toolchain(""));
    }
}
