//! 70-macos-defaults: curated macOS defaults for development. Best-effort: if one key is
//! unsupported on a given macOS version, the remaining settings still apply.

use super::super::os;
use super::{only_macos, quiet, run, stdout_of};
use crate::out::{self, info, ok, uptodate, verbose};
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
    let line = |(d, k, t, v): Pref| -> Vec<String> { ["write", d, k, t, v].iter().map(ToString::to_string).collect() };
    let mut all: Vec<Vec<String>> = BEFORE_SCREENSHOTS.iter().copied().map(line).collect();
    all.push(
        ["write", "com.apple.screencapture", "location", "-string", &shots].iter().map(ToString::to_string).collect(),
    );
    all.extend(SCREENSHOTS.iter().copied().map(line));
    all.extend(AFTER_SCREENSHOTS.iter().copied().map(line));
    all
}

/// True when `defaults read` printed `read` for a key written with type flag `ty` and value `want`.
/// `defaults read` prints a bool as 1 or 0 and a float in its own format.
fn same_value(ty: &str, want: &str, read: &str) -> bool {
    let read = read.trim();
    match ty {
        "-bool" => read == if want == "true" { "1" } else { "0" },
        "-float" => matches!((read.parse::<f64>(), want.parse::<f64>()), (Ok(r), Ok(w)) if (r - w).abs() < 1e-9),
        _ => read == want,
    }
}

/// The app to restart so a changed key takes effect (the global domain needs a logout instead).
fn owner_app(domain: &str) -> Option<&'static str> {
    match domain {
        "com.apple.finder" | "com.apple.desktopservices" => Some("Finder"),
        "com.apple.dock" => Some("Dock"),
        "com.apple.screencapture" => Some("SystemUIServer"),
        _ => None,
    }
}

pub fn step(_repo: &Path) {
    if !only_macos("macOS defaults") {
        return;
    }
    out::step("macOS defaults");
    if verbose() {
        info("Applying curated development defaults…");
    }
    let home = os::home();
    let _ = std::fs::create_dir_all(home.join("Screenshots"));
    let mut changed = 0;
    let mut restart: Vec<&str> = Vec::new();
    for args in writes(&home) {
        let [_, domain, key, ty, value] = <[String; 5]>::try_from(args.clone()).expect("five words");
        if same_value(&ty, &value, &stdout_of("defaults", &["read", &domain, &key])) {
            continue;
        }
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        if run("defaults", &a) {
            changed += 1;
            if let Some(app) = owner_app(&domain).filter(|app| !restart.contains(app)) {
                restart.push(app);
            }
        }
    }
    if changed == 0 {
        uptodate("macOS defaults: all set");
        return;
    }
    for app in restart {
        quiet("killall", &[app]);
    }
    ok(&format!("macOS defaults: {changed} changed (a few need logout/login to take effect)"));
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

    #[test]
    fn same_value_reads_defaults_formats() {
        assert!(same_value("-bool", "true", "1\n"));
        assert!(same_value("-bool", "false", "0"));
        assert!(!same_value("-bool", "true", "0"));
        assert!(!same_value("-bool", "true", ""), "a missing key is a change");
        assert!(same_value("-float", "0.15", "0.15"));
        assert!(same_value("-int", "2", "2"));
        assert!(same_value("-string", "Nlsv", "Nlsv\n"));
        assert!(!same_value("-string", "png", "jpg"));
    }
}
