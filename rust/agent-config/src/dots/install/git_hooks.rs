//! 77-dotfiles-git-hooks: one-time, wire the git hooks of the dotfiles repo itself. Sets
//! core.hooksPath in the repo's .git/config (not in ~/.gitconfig). The hooks in
//! scripts/git-hooks/ then run on every `git commit` and `git push`.

use super::super::git;
use super::run;
use crate::out::{self, ok, skip, warn};
use std::path::Path;

const HOOKS_DIR: &str = "scripts/git-hooks";

pub fn step(repo: &Path) {
    out::step("Dotfiles repo git hooks");
    if !repo.join(HOOKS_DIR).is_dir() {
        warn("scripts/git-hooks/ not found — skipping hook wiring");
        return;
    }
    let r = repo.to_string_lossy();
    let current = git::out(&["-C", &r, "config", "core.hooksPath"]).unwrap_or_default();
    if current == HOOKS_DIR {
        skip(&format!("core.hooksPath (already {})", HOOKS_DIR));
        return;
    }
    if run("git", &["-C", &r, "config", "core.hooksPath", HOOKS_DIR]) {
        ok(&format!("core.hooksPath = {} (the git hooks are now active)", HOOKS_DIR));
    } else {
        warn("Failed to set core.hooksPath — run manually:");
        warn("  git -C ~/dotfiles config core.hooksPath scripts/git-hooks");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn lines(f: impl FnOnce()) -> String {
        out::capture(f).1.into_iter().map(|(l, _)| l).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn wires_the_hooks_once_in_a_temp_repo() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        let init = Command::new("git").arg("init").arg("-q").arg(r).status();
        assert!(init.is_ok_and(|s| s.success()), "git is needed for this test");
        std::fs::create_dir_all(r.join(HOOKS_DIR)).unwrap();
        lines(|| step(r));
        assert_eq!(git::out(&["-C", &r.to_string_lossy(), "config", "core.hooksPath"]).as_deref(), Some(HOOKS_DIR));
        let second = lines(|| step(r));
        assert!(!second.contains("now active"), "{}", second);
    }

    #[test]
    fn warns_when_the_hooks_folder_is_missing() {
        let d = tempfile::tempdir().unwrap();
        let (_, got) = out::capture(|| step(d.path()));
        assert!(got.iter().any(|(l, err)| *err && l.contains("scripts/git-hooks/ not found")), "{:?}", got);
    }
}
