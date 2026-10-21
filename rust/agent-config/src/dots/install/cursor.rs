//! 60-cursor: Cursor (app + CLI) from the same claude/ source. Runs only when Cursor is
//! present. See docs/cursor-port.md.
//!   1. the `agent` / `cursor-agent` CLI, via Cursor's official installer, if missing
//!   2. the Brewfile `vscode` extensions, into Cursor (brew bundle only feeds VS Code)
//!   3. generated rules and agents, merged hooks / MCP / CLI config (agent-config)

use super::super::{os, settings};
use super::{brew_extensions, curl_pipe_bash};
use crate::out::{self, note, ok, skip, warn};
use std::path::{Path, PathBuf};

/// Pylance is not on Open VSX; Cursor ships its own Python server.
const LEAVE: [&str; 1] = ["ms-python.vscode-pylance"];

/// The `cursor` command inside the app folder.
fn app_cursor(app: &Path) -> PathBuf { app.join("Contents").join("Resources").join("app").join("bin").join("cursor") }

pub fn step(repo: &Path) {
    os::brew_env();
    out::step("Cursor");
    let home = os::home();
    let app = [PathBuf::from("/Applications/Cursor.app"), home.join("Applications").join("Cursor.app")].into_iter().find(|a| a.is_dir());
    if app.is_none() && !home.join(".cursor").is_dir() && !os::have("cursor") {
        note("not installed, nothing to set up");
        return;
    }

    // 1. CLI. Never probe it with `cursor agent ...`: that runs the installer too.
    if os::is_executable(&home.join(".local").join("bin").join("agent")) || os::have("cursor-agent") {
        skip("Cursor CLI");
    } else if curl_pipe_bash(&["-fsS", "--max-time", "60", "https://cursor.com/install"]) {
        ok("Cursor CLI installed (~/.local/bin/agent). Sign in once: agent login");
    } else {
        warn("Cursor CLI install failed. Try later: curl https://cursor.com/install -fsS | bash");
    }

    // 2. Extensions.
    let bin = os::which("cursor").or_else(|| app.as_deref().map(app_cursor));
    match bin {
        Some(b) if os::is_executable(&b) && repo.join("Brewfile").is_file() => brew_extensions(&b, repo, "Cursor", &LEAVE),
        _ => warn("no cursor command found, extensions not installed"),
    }

    // 3. Rules, agents, hooks, MCP, CLI config. agent-config prints its own count line.
    out::step_end();
    if crate::run_args(&["cursor", "--profile", &settings::profile()]) != 0 {
        warn("agent-config cursor reported an issue, see above");
    }
}
