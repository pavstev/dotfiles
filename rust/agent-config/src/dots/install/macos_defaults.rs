//! 70-macos-defaults: curated macOS defaults for development. Best-effort: if one key is
//! unsupported on a given macOS version, the remaining settings still apply.

use super::super::os;
use super::{only_macos, quiet, run};
use crate::out::{self, info, uptodate, verbose};
use std::path::Path;

/// (domain, key, type flag, value). The domain `-g` is the global one.
type Pref = (&'static str, &'static str, &'static str, &'static str);

/// Keyboard: fast key repeat, no press-and-hold accent popup. Finder: extensions, path and
/// status bars, list view, sane search. Save and print panels expanded.
const BEFORE_SCREENSHOTS: [Pref; 16] = [
    ("-g", "KeyRepeat", "-int", "2"),
    ("-g", "InitialKeyRepeat", "-int", "15"),
    ("-g", "ApplePressAndHoldEnabled", "-bool", "false"),
    ("-g", "AppleShowAllExtensions", "-bool", "true"),
    ("com.apple.finder", "AppleShowAllFiles", "-bool", "true"),
    ("com.apple.finder", "ShowPathbar", "-bool", "true"),
    ("com.apple.finder", "ShowStatusBar", "-bool", "true"),
    ("com.apple.finder", "_FXShowPosixPathInTitle", "-bool", "true"),
    ("com.apple.finder", "FXPreferredViewStyle", "-string", "Nlsv"),
    ("com.apple.finder", "FXDefaultSearchScope", "-string", "SCcf"), // search current folder
    ("com.apple.finder", "FXEnableExtensionChangeWarning", "-bool", "false"),
    ("com.apple.desktopservices", "DSDontWriteNetworkStores", "-bool", "true"),
    ("com.apple.desktopservices", "DSDontWriteUSBStores", "-bool", "true"),
    ("-g", "NSNavPanelExpandedStateForSaveMode", "-bool", "true"),
    ("-g", "NSNavPanelExpandedStateForSaveMode2", "-bool", "true"),
    ("-g", "PMPrintingExpandedStateForPrint", "-bool", "true"),
];

/// Screenshots: PNG, no window shadow (the location comes first, from the home folder).
const SCREENSHOTS: [Pref; 2] = [
    ("com.apple.screencapture", "type", "-string", "png"),
    ("com.apple.screencapture", "disable-shadow", "-bool", "true"),
];

/// Dock: snappier autohide, no auto-rearranged Spaces. Misc: no auto-capitalisation or
/// auto-correct in code.
const AFTER_SCREENSHOTS: [Pref; 4] = [
    ("com.apple.dock", "autohide-time-modifier", "-float", "0.15"),
    ("com.apple.dock", "mru-spaces", "-bool", "false"),
    ("-g", "NSAutomaticCapitalizationEnabled", "-bool", "false"),
    ("-g", "NSAutomaticSpellingCorrectionEnabled", "-bool", "false"),
];

/// Every `defaults write` argument list, in the order they run.
fn writes(home: &Path) -> Vec<Vec<String>> {
    let shots = home.join("Screenshots").to_string_lossy().into_owned();
    let line = |(d, k, t, v): Pref| -> Vec<String> { ["write", d, k, t, v].iter().map(|s| s.to_string()).collect() };
    let mut all: Vec<Vec<String>> = BEFORE_SCREENSHOTS.iter().copied().map(line).collect();
    all.push(["write", "com.apple.screencapture", "location", "-string", &shots].iter().map(|s| s.to_string()).collect());
    all.extend(SCREENSHOTS.iter().copied().map(line));
    all.extend(AFTER_SCREENSHOTS.iter().copied().map(line));
    all
}

pub fn step(_repo: &Path) {
    if !only_macos("macOS defaults") { return; }
    out::step("macOS defaults");
    if verbose() { info("Applying curated development defaults…"); }
    let home = os::home();
    let _ = std::fs::create_dir_all(home.join("Screenshots"));
    for args in writes(&home) {
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        run("defaults", &a);
    }
    for app in ["Finder", "Dock", "SystemUIServer"] { quiet("killall", &[app]); }
    uptodate("macOS defaults applied (a few settings need logout/login to fully take effect)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_one_write() {
        let w = writes(Path::new("/h"));
        assert!(w.iter().all(|a| a.len() == 5 && a[0] == "write"), "{w:?}");
        let shots = "write com.apple.screencapture location -string /h/Screenshots";
        assert!(w.iter().any(|a| a.join(" ") == shots), "the screenshot folder is under HOME");
    }
}
