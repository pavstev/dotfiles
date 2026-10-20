//! 10-homebrew: make sure Homebrew is installed (macOS only: it installs the apps). Also the setup-lib.sh helper `ensure_brew`.
//! Tolerant: a failed install warns and the run goes on (the brew steps skip without brew).

use super::super::os;
use super::{curl_text, linux_prereqs, run};
use crate::out::{self, info, ok, skip, verbose, warn};
use std::path::Path;
use std::process::Command;

const INSTALLER: &str = "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";

/// Install Homebrew when it is missing, then load it. At a terminal it asks for the password
/// first (the installer needs sudo). On Linux the prerequisites go in first. Homebrew is not
/// for Windows or musl. True = brew works.
pub fn ensure_brew(repo: &Path) -> bool {
    if os::is_windows() {
        warn("Homebrew is not for Windows — WinGet installs the tools instead");
        return true;
    }
    os::brew_env();
    if os::have("brew") { return true; }
    if os::is_musl() { return false; } // Homebrew needs glibc; step 10 says so
    if !linux_prereqs(repo) { warn("going on without the prerequisites. The Homebrew installer may complain."); }
    if os::at_terminal() {
        info(if os::is_macos() { "Homebrew needs your Mac password once." } else { "Homebrew needs your password once." });
        if !run("sudo", &["-v"]) { return false; }
    }
    // The bash ran `bash -c "$(curl ...)"`: a failed download gave an empty script and brew
    // was still missing below. Same result here.
    let Some(script) = curl_text(&["-fsSL", INSTALLER]) else { return false };
    let done = Command::new("/bin/bash").args(["-c", &script]).env("NONINTERACTIVE", "1").status();
    if !done.map(|s| s.success()).unwrap_or(false) { return false; }
    os::brew_env();
    os::have("brew")
}

pub fn step(repo: &Path) {
    if !super::only_macos("Homebrew") { return; } // Homebrew installs the Mac apps; tools come from 12-tools
    out::step("Homebrew");
    os::brew_env();
    if os::have("brew") {
        if verbose() { skip("Homebrew"); } // the Homebrew apps step shows brew works
        return;
    }
    info("installing Homebrew");
    if ensure_brew(repo) {
        ok("Homebrew installed");
    } else {
        warn("Homebrew install failed. The Mac apps do not install. Fix the error above, then run dots again.");
    }
}
