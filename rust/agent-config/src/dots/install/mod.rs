//! The install steps, one Rust function each, in the order they run. Each prints through `crate::out`. The runner (steps.rs) closes the
//! step with `out::step_end` after the function returns.

mod brew_bundle;
mod claude_code;
mod cursor;
mod default_apps;
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

/// One install step. The runner (steps.rs) calls `run`, then closes the step (out::step_end).
pub struct Step {
    /// `NN-name`; the number sets the run order and the name is what `dots settings` offers.
    pub name: &'static str,
    /// Runs on Windows too (the other steps are skipped there).
    pub windows: bool,
    /// The step itself.
    pub run: fn(&Path),
}

const fn step(name: &'static str, run: fn(&Path)) -> Step {
    Step { name, windows: false, run }
}

/// The steps, in run order.
pub const STEPS: &[Step] = &[
    step("05-linux-prereqs", prereqs::step),
    step("10-homebrew", homebrew::step),
    Step { windows: true, ..step("12-tools", tools::step) },
    step("20-brew-bundle", brew_bundle::step),
    step("30-omz-plugins", omz::step),
    step("40-rust-toolchain", rust_toolchain::step),
    step("50-node-pnpm", node_pnpm::step),
    step("55-gh-extensions", gh_extensions::step),
    Step { windows: true, ..step("57-claude-code", claude_code::step) },
    step("58-opencode", opencode::step),
    step("60-cursor", cursor::step),
    step("70-macos-defaults", macos_defaults::step),
    step("71-default-apps", default_apps::step),
    step("72-touch-id-sudo", touch_id::step),
    step("75-terminal-appearance", terminal::step),
    step("77-dotfiles-git-hooks", git_hooks::step),
    step("78-linux-extras", linux_extras::step),
    step("90-sign-in", sign_in::step),
];

/// A step by name.
pub fn find(name: &str) -> Option<&'static Step> {
    STEPS.iter().find(|s| s.name == name)
}

/// False on another OS (a line in verbose mode). Call it before `step`.
pub(crate) fn only_macos(what: &str) -> bool {
    if os::is_macos() {
        return true;
    }
    if out::verbose() {
        out::info(&format!("{what}: macOS only, skipping"));
    }
    false
}

/// Run a command with its output dropped (`>/dev/null 2>&1 </dev/null`). True when it exits 0.
/// No stdin: a hidden question (corepack asks before a download) fails at once instead of
/// waiting for an answer nobody can see.
pub(crate) fn quiet(prog: &str, args: &[&str]) -> bool {
    Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Run a command with the terminal's stdin, stdout and stderr. True when it exits 0.
pub(crate) fn run(prog: &str, args: &[&str]) -> bool {
    Command::new(prog).args(args).status().is_ok_and(|s| s.success())
}

/// stdout of a command (`2>/dev/null`), whatever its exit code. "" when it can't start.
pub(crate) fn stdout_of(prog: &str, args: &[&str]) -> String {
    Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// What `curl <args>` prints, or None when curl fails. Its errors show on stderr.
/// A stuck server fails after the timeouts instead of hanging `dots`.
pub(crate) fn curl_text(args: &[&str]) -> Option<String> {
    let o = Command::new("curl")
        .args(["--connect-timeout", "20", "--max-time", "300"])
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `curl <args> | bash >/dev/null 2>&1` with pipefail: both must succeed.
pub(crate) fn curl_pipe_bash(args: &[&str]) -> bool {
    let Some(script) = curl_text(args) else { return false };
    let Ok(mut child) = Command::new("bash").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    child.wait().is_ok_and(|s| s.success())
}

/// Run a command with `text` on its stdin and its stdout dropped. Its stderr shows when
/// `show_err`. True when it exits 0.
pub(crate) fn feed(prog: &str, args: &[&str], text: &str, show_err: bool) -> bool {
    let err = if show_err { Stdio::inherit() } else { Stdio::null() };
    let Ok(mut child) = Command::new(prog).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(err).spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    child.wait().is_ok_and(|s| s.success())
}

/// The `vscode "<id>"` lines of a Brewfile: the ids, lowercased.
pub(crate) fn vscode_extensions(brewfile: &str) -> Vec<String> {
    brewfile
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("vscode \"")?;
            let id = &rest[..rest.find('"')?];
            if id.is_empty() { None } else { Some(id.to_ascii_lowercase()) }
        })
        .collect()
}

/// The Brewfile extensions that `bin` (Cursor or VS Code) lacks get installed. `label` names
/// the editor in the messages; `leave` are ids never installed. Nothing to do counts one skip.
pub(crate) fn brew_extensions(bin: &Path, repo: &Path, label: &str, leave: &[&str]) {
    let bin_s = bin.to_string_lossy();
    let installed: Vec<String> =
        stdout_of(&bin_s, &["--list-extensions"]).lines().map(str::to_ascii_lowercase).collect();
    let brewfile = std::fs::read_to_string(repo.join("Brewfile")).unwrap_or_default();
    let (mut added, mut failed) = (0, 0);
    for ext in vscode_extensions(&brewfile) {
        if leave.contains(&ext.as_str()) || installed.contains(&ext) {
            continue;
        }
        if quiet(&bin_s, &["--install-extension", &ext]) {
            out::ok(&format!("extension {ext}"));
            added += 1;
        } else {
            out::warn(&format!("couldn't install extension {ext} into {label}"));
            failed += 1;
        }
    }
    if added == 0 && failed == 0 {
        out::skip(&format!("{label} extensions"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_step_is_found_and_the_list_is_sorted() {
        for s in STEPS {
            assert!(find(s.name).is_some(), "{}", s.name);
        }
        assert!(STEPS.windows(2).all(|w| w[0].name < w[1].name), "STEPS is not sorted");
        let on_windows: Vec<&str> = STEPS.iter().filter(|s| s.windows).map(|s| s.name).collect();
        assert_eq!(on_windows, ["12-tools", "57-claude-code"]);
        for n in ["15-winget", "", "99-none"] {
            assert!(find(n).is_none(), "{}", n);
        }
    }

    #[test]
    fn brewfile_vscode_lines() {
        let b = "tap \"x/y\"\nbrew \"git\"\nvscode \"Foo.Bar\"\n  vscode \"indented.no\"\nvscode \"ms-python.vscode-pylance\" # note\nvscode \"\"\nvscode \"open\n# vscode \"c.c\"\n";
        assert_eq!(vscode_extensions(b), vec!["foo.bar", "ms-python.vscode-pylance"]);
    }
}
