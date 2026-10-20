//! git: the pull at the start, the identity in ~/.gitconfig.local, the Windows .gitconfig.

use super::os;
use crate::out::{info, ok, skip, verbose, warn};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

/// stdout of a git command, trimmed, or None when it fails.
pub fn out(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !o.status.success() { return None; }
    Some(String::from_utf8_lossy(&o.stdout).trim_end_matches(['\n', '\r']).to_string())
}

fn ok_run(args: &[&str]) -> bool {
    Command::new("git").args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

pub fn head(repo: &Path) -> String {
    out(&["-C", &repo.to_string_lossy(), "rev-parse", "HEAD"]).unwrap_or_default()
}

/// `git pull --ff-only`, with no autostash and no password prompt. A pull that would
/// change a file edited here stops, instead of leaving conflict markers in a live config.
pub fn pull(repo: &Path) -> bool {
    Command::new("git")
        .args(["-C", &repo.to_string_lossy(), "-c", "rebase.autoStash=false", "-c", "merge.autoStash=false",
            "pull", "--ff-only", "--no-rebase", "--quiet"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

/// The tracked files changed here, each followed by a space (as `tr '\n' ' '` gives).
pub fn changed(repo: &Path) -> String {
    let s = out(&["-C", &repo.to_string_lossy(), "status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
    s.lines().filter(|l| l.len() > 3).map(|l| format!("{} ", &l[3..])).collect()
}

/// Write the name and email to ~/.gitconfig.local when they differ.
pub fn identity(name: &str, email: &str) {
    let f = os::home().join(".gitconfig.local");
    let fs_ = f.to_string_lossy().into_owned();
    let cur_n = out(&["config", "-f", &fs_, "user.name"]).unwrap_or_default();
    let cur_e = out(&["config", "-f", &fs_, "user.email"]).unwrap_or_default();
    if cur_n == name && cur_e == email {
        if verbose() { skip(&format!("git identity ({} <{}>)", name, email)); }
    } else if ok_run(&["config", "-f", &fs_, "user.name", name]) && ok_run(&["config", "-f", &fs_, "user.email", email]) {
        ok(&format!("git commits use: {} <{}>", name, email));
    } else {
        warn(&format!("couldn't write {}", f.display()));
    }
}

/// Windows: the repo's ~/.gitconfig is not linked (it sets delta and VS Code, which a
/// Windows machine lacks). A repo copy left by an earlier run goes, the newest backup
/// comes back, and ~/.gitconfig includes ~/.gitconfig.local for the identity.
pub fn windows_gitconfig() {
    let g = os::home().join(".gitconfig");
    let ours = os::is_symlink(&g) || fs::read_to_string(&g).map(|t| t.contains("symlinked from this repo")).unwrap_or(false);
    if ours {
        let _ = fs::remove_file(&g);
        let name = ".gitconfig.bak-";
        let mut baks: Vec<_> = fs::read_dir(os::home()).into_iter().flatten().flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(name))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        baks.sort();
        if let Some((_, b)) = baks.pop() && fs::rename(&b, &g).is_ok() { info("restored your own .gitconfig"); }
    }
    let inc = out(&["config", "--global", "--get-all", "include.path"]).unwrap_or_default();
    if !inc.contains("gitconfig.local") {
        let _ = ok_run(&["config", "--global", "--add", "include.path", "~/.gitconfig.local"]);
    }
}
