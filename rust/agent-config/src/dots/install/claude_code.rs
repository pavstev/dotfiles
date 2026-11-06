//! 57-claude-code: Claude Code (the `claude` CLI). Mac and Linux: Anthropic's installer, which
//! keeps it up to date. Windows: `WinGet` (Anthropic.ClaudeCode), else Anthropic's PowerShell
//! installer. Skips when `claude` is already there. Its settings come from claude/settings.json;
//! the rest is the ps plugin, which `dots` installs.

use super::super::os;
use super::{curl_pipe_bash, quiet};
use crate::out::{self, ok, skip, warn};
use std::path::Path;

pub(super) fn step(_repo: &Path) {
    out::step("Claude Code");
    if os::have("claude")
        || os::is_executable(&os::home().join(".local").join("bin").join("claude"))
    {
        skip("Claude Code");
    } else if os::is_windows() {
        let winget = os::have("winget")
            && quiet(
                "winget",
                &[
                    "install",
                    "--id",
                    "Anthropic.ClaudeCode",
                    "-e",
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                    "--silent",
                ],
            );
        if winget
            || quiet(
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    "irm https://claude.ai/install.ps1 | iex",
                ],
            )
        {
            ok("Claude Code installed");
        } else {
            warn(
                "Claude Code did not install. In PowerShell run: irm https://claude.ai/install.ps1 | iex",
            );
        }
    } else if curl_pipe_bash(&["-fsSL", "--max-time", "300", "https://claude.ai/install.sh"]) {
        ok("Claude Code installed (~/.local/bin/claude). Sign in once: claude auth login");
    } else {
        warn(
            "Claude Code install failed. Try later: curl -fsSL https://claude.ai/install.sh | bash",
        );
    }
}
