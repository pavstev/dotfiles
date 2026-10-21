//! The install steps, one Rust function each, in the order they run. Each prints through `crate::out`. The runner (steps.rs) closes the
//! step with `out::step_end` after the function returns.

mod brew_bundle;
mod claude_code;
mod cursor;
mod gh_extensions;
mod git_hooks;
mod homebrew;
mod linux_extras;
mod macos_defaults;
mod node_pnpm;
mod omz;
mod opencode;
mod prereqs;
mod rust_toolchain;
mod sign_in;
mod terminal;
mod tools;
mod touch_id;

pub use homebrew::ensure_brew;
pub use prereqs::{linux_prereqs, linux_prereqs_missing};

use super::os;
use crate::out;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The step names, in run order. `dots settings` offers them for "Steps to skip".
pub const STEPS: [&str; 17] = [
    "05-linux-prereqs", "10-homebrew", "12-tools", "20-brew-bundle", "30-omz-plugins", "40-rust-toolchain",
    "50-node-pnpm", "55-gh-extensions", "57-claude-code", "58-opencode", "60-cursor", "70-macos-defaults",
    "72-touch-id-sudo", "75-terminal-appearance", "77-dotfiles-git-hooks", "78-linux-extras", "90-sign-in",
];

/// A step by name. The runner (steps.rs) calls it, then closes the step (out::step_end).
pub fn find(name: &str) -> Option<fn(&Path)> {
    Some(match name {
        "05-linux-prereqs" => prereqs::step,
        "10-homebrew" => homebrew::step,
        "12-tools" => tools::step,
        "20-brew-bundle" => brew_bundle::step,
        "30-omz-plugins" => omz::step,
        "40-rust-toolchain" => rust_toolchain::step,
        "50-node-pnpm" => node_pnpm::step,
        "55-gh-extensions" => gh_extensions::step,
        "57-claude-code" => claude_code::step,
        "58-opencode" => opencode::step,
        "60-cursor" => cursor::step,
        "70-macos-defaults" => macos_defaults::step,
        "72-touch-id-sudo" => touch_id::step,
        "75-terminal-appearance" => terminal::step,
        "77-dotfiles-git-hooks" => git_hooks::step,
        "78-linux-extras" => linux_extras::step,
        "90-sign-in" => sign_in::step,
        _ => return None,
    })
}

/// False on another OS (a line in verbose mode). Call it before `step`.
pub(crate) fn only_macos(what: &str) -> bool {
    if os::is_macos() { return true; }
    if out::verbose() { out::info(&format!("{}: macOS only, skipping", what)); }
    false
}

/// Run a command with its output dropped (`>/dev/null 2>&1 </dev/null`). True when it exits 0.
/// No stdin: a hidden question (corepack asks before a download) fails at once instead of
/// waiting for an answer nobody can see.
pub(crate) fn quiet(prog: &str, args: &[&str]) -> bool {
    Command::new(prog).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

/// Run a command with the terminal's stdin, stdout and stderr. True when it exits 0.
pub(crate) fn run(prog: &str, args: &[&str]) -> bool {
    Command::new(prog).args(args).status().map(|s| s.success()).unwrap_or(false)
}

/// stdout of a command (`2>/dev/null`), whatever its exit code. "" when it can't start.
pub(crate) fn stdout_of(prog: &str, args: &[&str]) -> String {
    Command::new(prog).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// What `curl <args>` prints, or None when curl fails. Its errors show on stderr.
/// A stuck server fails after the timeouts instead of hanging `dots`.
pub(crate) fn curl_text(args: &[&str]) -> Option<String> {
    let o = Command::new("curl").args(["--connect-timeout", "20", "--max-time", "300"]).args(args).stdin(Stdio::null()).stderr(Stdio::inherit()).output().ok()?;
    if !o.status.success() { return None; }
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `curl <args> | bash >/dev/null 2>&1` with pipefail: both must succeed.
pub(crate) fn curl_pipe_bash(args: &[&str]) -> bool {
    let Some(script) = curl_text(args) else { return false };
    let Ok(mut child) = Command::new("bash").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
    if let Some(mut stdin) = child.stdin.take() { let _ = stdin.write_all(script.as_bytes()); }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Run a command with `text` on its stdin and its stdout dropped. Its stderr shows when
/// `show_err`. True when it exits 0.
pub(crate) fn feed(prog: &str, args: &[&str], text: &str, show_err: bool) -> bool {
    let err = if show_err { Stdio::inherit() } else { Stdio::null() };
    let Ok(mut child) = Command::new(prog).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(err).spawn() else { return false };
    if let Some(mut stdin) = child.stdin.take() { let _ = stdin.write_all(text.as_bytes()); }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// The `vscode "<id>"` lines of a Brewfile: the ids, lowercased.
pub(crate) fn vscode_extensions(brewfile: &str) -> Vec<String> {
    brewfile.lines().filter_map(|l| {
        let rest = l.strip_prefix("vscode \"")?;
        let id = &rest[..rest.find('"')?];
        if id.is_empty() { None } else { Some(id.to_ascii_lowercase()) }
    }).collect()
}

/// The Brewfile extensions that `bin` (Cursor or VS Code) lacks get installed. `label` names
/// the editor in the messages; `leave` are ids never installed. Nothing to do counts one skip.
pub(crate) fn brew_extensions(bin: &Path, repo: &Path, label: &str, leave: &[&str]) {
    let bin_s = bin.to_string_lossy();
    let installed: Vec<String> = stdout_of(&bin_s, &["--list-extensions"]).lines().map(|l| l.to_ascii_lowercase()).collect();
    let brewfile = std::fs::read_to_string(repo.join("Brewfile")).unwrap_or_default();
    let (mut added, mut failed) = (0, 0);
    for ext in vscode_extensions(&brewfile) {
        if leave.contains(&ext.as_str()) || installed.contains(&ext) { continue; }
        if quiet(&bin_s, &["--install-extension", &ext]) {
            out::ok(&format!("extension {}", ext));
            added += 1;
        } else {
            out::warn(&format!("couldn't install extension {} into {}", ext, label));
            failed += 1;
        }
    }
    if added == 0 && failed == 0 { out::skip(&format!("{} extensions", label)); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_step_has_a_function_and_the_list_is_sorted() {
        for n in STEPS { assert!(find(n).is_some(), "{}", n); }
        assert!(STEPS.windows(2).all(|w| w[0] < w[1]), "STEPS is not sorted");
        for n in ["15-winget", "", "99-none"] { assert!(find(n).is_none(), "{}", n); }
    }

    #[test]
    fn brewfile_vscode_lines() {
        let b = "tap \"x/y\"\nbrew \"git\"\nvscode \"Foo.Bar\"\n  vscode \"indented.no\"\nvscode \"ms-python.vscode-pylance\" # note\nvscode \"\"\nvscode \"open\n# vscode \"c.c\"\n";
        assert_eq!(vscode_extensions(b), vec!["foo.bar", "ms-python.vscode-pylance"]);
    }
}
