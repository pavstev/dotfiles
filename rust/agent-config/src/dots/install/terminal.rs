//! 75-terminal-appearance: Apple Terminal appearance, a Nerd Font and a smaller, compact
//! profile. Ghostty is configured by its own dotfile; Terminal.app keeps font and window
//! geometry in the app's profile, so AppleScript drives it.
//!
//! Uses the PostScript name "FiraCodeNFM-Reg" (FiraCode Nerd Font Mono, Regular). Needs the
//! font-fira-code-nerd-font cask (installed by brew). Tolerant: without the Automation
//! permission it warns and moves on.

use super::super::os;
use super::{feed, only_macos};
use crate::out::{self, uptodate, warn};
use std::path::Path;

const FONT_PS: &str = "FiraCodeNFM-Reg"; // FiraCode Nerd Font Mono, Regular
const FONT_SIZE: u32 = 12; // compact
const COLUMNS: u32 = 100; // tidy, compact default window...
const ROWS: u32 = 28; // ...geometry (Terminal has no padding control)

/// The AppleScript that sets the font and window size.
fn script() -> String {
    format!(
        r#"tell application "Terminal"
    -- "Basic" may have been renamed; wrap it so a miss doesn't block the rest.
    try
        set font name of settings set "Basic" to "{f}"
        set font size of settings set "Basic" to {s}
        set number of columns of settings set "Basic" to {c}
        set number of rows of settings set "Basic" to {r}
    end try
    -- The default + startup profiles always resolve, whatever they're named.
    set font name of default settings to "{f}"
    set font size of default settings to {s}
    set number of columns of default settings to {c}
    set number of rows of default settings to {r}
    set font name of startup settings to "{f}"
    set font size of startup settings to {s}
end tell
"#,
        f = FONT_PS,
        s = FONT_SIZE,
        c = COLUMNS,
        r = ROWS
    )
}

pub fn step(_repo: &Path) {
    if !only_macos("Apple Terminal appearance") {
        return;
    }
    if !crate::dots::looks::on("font") {
        if out::verbose() {
            out::info("Apple Terminal appearance: off (Looks, in dots settings)");
        }
        return;
    }
    out::step("Apple Terminal appearance");
    if !os::have("osascript") {
        warn("osascript unavailable — skipping Terminal.");
        return;
    }
    // Confirm the font is installed (in EITHER location) before touching Terminal.
    let font = "FiraCodeNerdFontMono-Regular.ttf";
    if !os::home().join("Library").join("Fonts").join(font).is_file()
        && !Path::new("/Library/Fonts").join(font).is_file()
    {
        warn("FiraCode Nerd Font not installed yet — skipping Terminal appearance.");
        return;
    }
    if feed("osascript", &[], &script(), false) {
        uptodate(&format!(
            "Apple Terminal → FiraCode Nerd Font Mono {}pt, {}×{} (compact, icons on)",
            FONT_SIZE, COLUMNS, ROWS
        ));
    } else {
        warn("Couldn't set Apple Terminal appearance automatically.");
        warn("Grant Automation permission (System Settings → Privacy → Automation), then re-run: ./dotfiles");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_has_the_values() {
        let s = script();
        assert!(s.starts_with("tell application \"Terminal\"\n"));
        assert!(s.ends_with("\nend tell\n"));
        assert!(!s.contains('{')); // no format hole left
    }
}
