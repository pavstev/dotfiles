//! 72-touch-id-sudo: Touch ID for sudo via /etc/pam.d/sudo_local. sudo_local is included by
//! /etc/pam.d/sudo and survives macOS upgrades. Safe to re-run: stops early when the file
//! already holds pam_tid.

use super::{feed, only_macos, quiet};
use crate::out::{self, info, ok, skip, warn};
use std::path::Path;

const SUDO_LOCAL: &str = "/etc/pam.d/sudo_local";

/// What goes into sudo_local.
const BODY: &str = "# sudo_local: local config file which survives system update and is included by /etc/pam.d/sudo\nauth       sufficient     pam_tid.so\n";

fn has_pam_tid(text: &str) -> bool { text.contains("pam_tid") }

pub fn step(_repo: &Path) {
    if !only_macos("Touch ID for sudo") { return; }
    out::step("Touch ID for sudo");
    if std::fs::read_to_string(SUDO_LOCAL).map(|t| has_pam_tid(&t)).unwrap_or(false) {
        skip("Touch ID for sudo");
        return;
    }
    if !quiet("sudo", &["-n", "true"]) {
        info(&format!("Need sudo to write {} — you may be prompted for your password once.", SUDO_LOCAL));
    }
    if feed("sudo", &["tee", SUDO_LOCAL], BODY, true) {
        ok(&format!("Touch ID for sudo enabled ({} created).", SUDO_LOCAL));
    } else {
        warn(&format!("Couldn't write {} (no sudo/no TTY for a password prompt) — re-run: ./dotfiles", SUDO_LOCAL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pam_tid_check_and_body() {
        assert!(has_pam_tid("auth sufficient pam_tid.so\n"));
        assert!(!has_pam_tid("# nothing here\n"));
        assert!(has_pam_tid(BODY));
        assert_eq!(BODY.lines().count(), 2);
    }
}
