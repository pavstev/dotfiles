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

/// The commit the repo is on, or "" when git cannot say.
pub fn head(repo: &Path) -> String {
    out(&["-C", &repo.to_string_lossy(), "rev-parse", "HEAD"]).unwrap_or_default()
}

/// `git pull --ff-only`, with no autostash and no password prompt. A pull that would
/// change a file edited here stops, instead of leaving conflict markers in a live config.
/// When the origin history was rewritten and this clone has no work of its own, it moves
/// to the new history (`heal_rewrite`).
pub fn pull(repo: &Path) -> bool { pull_saving_to(repo, &os::home().join(".config").join("dotfiles")) }

/// `pull`, with the folder that gets the patch of saved edits.
fn pull_saving_to(repo: &Path, saved_dir: &Path) -> bool {
    let r = repo.to_string_lossy();
    let before = out(&["-C", &r, "rev-parse", "--verify", "-q", "@{u}"]).unwrap_or_default();
    let pulled = Command::new("git")
        .args(["-C", &r, "-c", "rebase.autoStash=false", "-c", "merge.autoStash=false",
            "pull", "--ff-only", "--no-rebase", "--quiet"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null()) // git's hint block; the caller warns in one line
        .status().map(|s| s.success()).unwrap_or(false);
    pulled || heal_rewrite(repo, &before, saved_dir)
}

/// The pull failed. Safe to reset to the new upstream only when all of these hold: an old
/// upstream tip (the one before this pull, or one in the upstream reflog, for a clone that
/// fetched earlier) is missing from the new upstream (a force push), HEAD was inside that old
/// tip (no commit of our own). Edits to tracked files are saved as a patch. The old HEAD stays
/// reachable as the branch `before-rewrite-<time>`.
fn heal_rewrite(repo: &Path, before: &str, saved_dir: &Path) -> bool {
    let r = repo.to_string_lossy();
    let Some(after) = out(&["-C", &r, "rev-parse", "--verify", "-q", "@{u}"]) else { return false };
    let anc = |a: &str, b: &str| ok_run(&["-C", &r, "merge-base", "--is-ancestor", a, b]);
    let up = out(&["-C", &r, "rev-parse", "--symbolic-full-name", "@{u}"]).unwrap_or_default();
    // The raw reflog lines are "<old> <new> <who> <when>\t<what>": both columns are tips it had.
    let log = out(&["-C", &r, "rev-parse", "--git-path", &format!("logs/{}", up)])
        .filter(|_| !up.is_empty())
        .and_then(|p| fs::read_to_string(repo.join(p)).ok())
        .unwrap_or_default();
    let tips: Vec<&str> = log.lines().flat_map(|l| l.split(' ').take(2)).collect();
    let old_tip = std::iter::once(before).chain(tips.into_iter().rev())
        .find(|t| !t.is_empty() && *t != after && !anc(t, &after) && anc("HEAD", t));
    if old_tip.is_none() { return false; }
    let keep = format!("before-rewrite-{}", os::timestamp());
    // Edited files (an editor writes through its linked settings file) go to a patch first.
    let saved = if changed(repo).is_empty() { None } else {
        let Some(p) = save_edits(repo, saved_dir, &keep) else { return false };
        Some(p)
    };
    if !ok_run(&["-C", &r, "branch", &keep, "HEAD"]) { return false; }
    if !ok_run(&["-C", &r, "-c", "core.autocrlf=false", "reset", "--hard", "--quiet", "@{u}"]) { return false; }
    info(&format!("the dotfiles history was rewritten upstream; this clone had no work of its own, so it moved to the new one (old state: branch {})", keep));
    if let Some(p) = saved { info(&format!("your edits to tracked files are saved in {} (git apply it to bring them back)", p.display())); }
    true
}

/// Write the edits to tracked files as a patch next to the private settings. None when
/// the diff is empty, git fails or the file can't be written.
fn save_edits(repo: &Path, dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    let o = Command::new("git").args(["-C", &repo.to_string_lossy(), "diff", "HEAD", "--binary"])
        .stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !o.status.success() || o.stdout.is_empty() { return None; }
    fs::create_dir_all(dir).ok()?;
    let p = dir.join(format!("{}.patch", name));
    fs::write(&p, &o.stdout).ok()?;
    Some(p)
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git").arg("-C").arg(dir).args(args)
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@example.com")
            .stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    }
    fn commit(dir: &Path, file: &str, text: &str) {
        fs::write(dir.join(file), text).unwrap();
        assert!(git(dir, &["add", "-A"]) && git(dir, &["commit", "-qm", text]));
    }

    /// origin with two commits, a clone of it, then origin squashes them (a force push).
    fn rewritten() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let (o, c) = (t.path().join("o"), t.path().join("c"));
        fs::create_dir(&o).unwrap();
        assert!(git(&o, &["init", "-q", "-b", "main"]));
        commit(&o, "a", "one");
        commit(&o, "a", "two");
        assert!(Command::new("git").args(["clone", "-q"]).arg(&o).arg(&c).status().unwrap().success());
        assert!(git(&o, &["reset", "--soft", "HEAD~1"]) && git(&o, &["commit", "--amend", "-qm", "squashed"]));
        (t, o, c)
    }

    #[test]
    fn heals_a_clean_clone_after_a_rewrite() {
        let (_t, o, c) = rewritten();
        assert!(pull(&c));
        assert_eq!(head(&c), head(&o));
        assert!(out(&["-C", &c.to_string_lossy(), "branch", "--list", "before-rewrite-*"]).unwrap().contains("before-rewrite-"));
    }

    #[test]
    fn heals_after_an_earlier_fetch() {
        let (_t, o, c) = rewritten();
        assert!(git(&c, &["fetch", "-q"])); // an editor or lazygit fetched first
        assert!(pull(&c));
        assert_eq!(head(&c), head(&o));
    }

    #[test]
    fn keeps_own_commits() {
        let (_t, o, c) = rewritten();
        commit(&c, "b", "mine");
        assert!(!pull(&c));
        assert_ne!(head(&c), head(&o));
    }

    #[test]
    fn heals_an_edited_clone_and_saves_the_edit() {
        let (_t, o, c) = rewritten();
        fs::write(c.join("a"), "edited").unwrap();
        let home = tempfile::tempdir().unwrap();
        assert!(pull_saving_to(&c, home.path()));
        assert_eq!(head(&c), head(&o));
        let saved: Vec<_> = fs::read_dir(home.path()).unwrap().flatten().collect();
        assert_eq!(saved.len(), 1);
        assert!(fs::read_to_string(saved[0].path()).unwrap().contains("+edited"));
    }
}
