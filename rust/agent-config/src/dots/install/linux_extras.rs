//! 78-linux-extras: small Linux-only pieces. Linux only: a Mac prints nothing here.
//!   1. VS Code extensions from the Brewfile `vscode` lines, when `code` is installed
//!      (brew bundle only does this on a Mac)
//!   2. a note when the login shell is not zsh (this step never changes it)

use super::super::os;
use super::{brew_extensions, stdout_of};
use crate::out::{self, info, note, skip};
use std::path::Path;

/// The shell (7th field) of a `getent passwd` line.
fn shell_of(passwd_line: &str) -> String {
    passwd_line.trim_end().split(':').nth(6).unwrap_or("").to_string()
}

pub fn step(repo: &Path) {
    if !os::is_linux() {
        return;
    }
    os::brew_env();
    out::step("Linux extras");

    // 1. VS Code extensions.
    match os::which("code") {
        Some(code) if repo.join("Brewfile").is_file() => brew_extensions(&code, repo, "VS Code", &[]),
        _ => note("VS Code not installed, so no extensions to add"),
    }

    // 2. Login shell.
    let user = stdout_of("id", &["-un"]);
    let shell = shell_of(&stdout_of("getent", &["passwd", user.trim()]));
    if shell.ends_with("/zsh") {
        skip("login shell (zsh)");
    } else if let Some(zsh) = os::which("zsh") {
        let shown = if shell.is_empty() { "unknown" } else { &shell };
        info(&format!("your login shell is {}. To use zsh everywhere: chsh -s {}", shown, zsh.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_field() {
        assert_eq!(shell_of("jo:x:1000:1000:Jo,,,:/home/jo:/usr/bin/zsh\n"), "/usr/bin/zsh");
        assert_eq!(shell_of("jo:x:1000:1000:Jo:/home/jo:/bin/bash"), "/bin/bash");
        assert_eq!(shell_of(""), "");
        assert_eq!(shell_of("short:line"), "");
    }
}
