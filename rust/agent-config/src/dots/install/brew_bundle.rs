//! 20-brew-bundle: the Mac apps of the Brewfile (casks, the tap, the font, VS Code extensions). macOS
//! only. The command-line tools are not here: tools.list lists them and step 12 installs them.
//! `brew bundle check` short-circuits when everything is present, so a satisfied Mac does no work.
//! Brewfile.local (gitignored) holds per-machine extras.

use super::super::os;
use super::only_macos;
use crate::out::{self, info, ok, plain, skip, verbose, warn};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

/// Whether `brew bundle` prints this line. It says "Using <pkg>" for each present package and a
/// closing count; both are noise.
fn keep_line(l: &str) -> bool { !l.starts_with("Using ") && !l.starts_with("`brew bundle` complete") }

/// `brew bundle` for one file. True when brew exits 0. Installs and warnings print as they come;
/// DOTFILES_VERBOSE=1 prints everything.
fn bundle_quiet(file: &Path) -> bool {
    let f = file.to_string_lossy();
    if verbose() {
        return Command::new("brew").args(["bundle", &format!("--file={}", f), "--no-upgrade"]).status().map(|s| s.success()).unwrap_or(false);
    }
    // stderr into stdout, like 2>&1
    let Ok(mut child) = Command::new("sh").arg("-c").arg(r#"exec brew bundle --file="$0" --no-upgrade 2>&1"#).arg(&*f)
        .stdin(Stdio::inherit()).stdout(Stdio::piped()).spawn() else { return false };
    if let Some(o) = child.stdout.take() {
        for l in BufReader::new(o).lines().map_while(Result::ok) {
            if keep_line(&l) { plain(&l); }
        }
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Install only what is missing from `file`; skip when everything is there. True when it changed.
fn bundle_run(repo: &Path, file: &Path, label: &str) -> bool {
    if !file.is_file() { return false; }
    let f = format!("--file={}", file.display());
    if super::quiet("brew", &["bundle", "check", "--no-upgrade", &f]) {
        skip(&format!("{} (every formula/cask already installed)", label));
        return false;
    }
    info(&format!("{}: installing only what is missing...", label));
    // Do not abort the whole run on partial failures (a pre-installed app that brew wants to
    // "adopt" needs sudo). Warn and continue; the rest of the setup should still run.
    if !bundle_quiet(file) {
        warn(&format!("Some {} entries failed (often a pre-installed app needing sudo to", label));
        warn("adopt, or a Homebrew permissions issue). Fix perms and re-run:");
        warn(&format!("  {}/scripts/fix-brew-permissions.sh  &&  ./dotfiles", repo.display()));
    }
    true
}

pub fn step(repo: &Path) {
    if !only_macos("Homebrew apps") { return; }
    out::step("Homebrew apps");
    os::brew_env();
    if !os::have("brew") {
        warn("Homebrew missing, so brew bundle is skipped.");
        return;
    }
    // The Nerd Font is one of the optional looks (LOOKS in the settings).
    if !crate::dots::looks::on("font") { os::set_env("HOMEBREW_BUNDLE_CASK_SKIP", "font-fira-code-nerd-font"); }
    let a = bundle_run(repo, &repo.join("Brewfile"), "Brewfile");
    let b = bundle_run(repo, &repo.join("Brewfile.local"), "Brewfile.local"); // per-machine extras (gitignored)
    if a || b { ok("Mac apps satisfied"); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brew_noise_is_dropped() {
        assert!(!keep_line("Using git"));
        assert!(!keep_line("`brew bundle` complete! 5 Brewfile dependencies now installed."));
        assert!(keep_line("Installing ghostty"));
        assert!(keep_line("Error: something broke"));
    }
}
