//! The skills step: link each `claude/skills/<name>` into ~/.claude/skills. Additive:
//! skills from other sources stay. Cursor and
//! Opencode read ~/.claude/skills themselves, so nothing is linked for them.
//! Prints only when it linked or pruned something (and the clashes it left alone).

use super::os;
use crate::out::{plain, plain_err, verbose};
use std::fs;
use std::path::{Path, PathBuf};

/// The real skill folders: a SKILL.md inside, not `_template`, not a dot folder.
pub fn list(src: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(src)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('_') && !n.starts_with('.'))
        .filter(|n| src.join(n).join("SKILL.md").is_file())
        .collect();
    names.sort();
    names
}

/// Remove our own broken links (the target was under `src` and is gone). Never a real
/// folder or a link that points anywhere else.
fn prune(root: &Path, src: &Path) {
    let mut n = 0;
    for e in fs::read_dir(root).into_iter().flatten().flatten() {
        let link = e.path();
        if !os::is_symlink(&link) {
            continue;
        }
        let Some(t) = os::readlink(&link) else { continue };
        if !t.starts_with(src) || t == src || t.exists() {
            continue;
        }
        if fs::remove_file(&link).is_ok() {
            n += 1;
        }
    }
    if n > 0 {
        plain(&format!("·  removed {} old skill links from {}", n, root.display()));
    }
}

/// Link every skill into `root`. Returns false when something could not be linked.
pub fn link_each(root: &Path, src: &Path) -> bool {
    if os::is_symlink(root) {
        plain_err(&format!("!  {} is a symlink, left untouched (no skills added there)", root.display()));
        return true;
    }
    if fs::create_dir_all(root).is_err() {
        plain_err(&format!("!  can't create {} — skipping", root.display()));
        return true;
    }
    prune(root, src);
    let mut made = 0;
    let mut good = true;
    for name in list(src) {
        let link = root.join(&name);
        let target: PathBuf = src.join(&name);
        if os::is_symlink(&link) && os::readlink(&link).as_deref() == Some(target.as_path()) {
            continue;
        }
        // Windows: an earlier run (before native links) left a copy of our own skill. A copy
        // never follows the repo, so it is replaced by a link. Only our ps_* names.
        if os::is_windows() && name.starts_with("ps_") && link.is_dir() && !os::is_symlink(&link) {
            let _ = fs::remove_dir_all(&link);
        }
        if os::exists_or_link(&link) {
            plain(&format!("·  {} exists (not ours) — left untouched", link.display()));
            continue;
        }
        if os::symlink(&target, &link).is_ok() {
            made += 1;
        } else {
            plain_err(&format!("!  can't link {}", link.display()));
            good = false;
        }
    }
    if made > 0 {
        plain(&format!("✓ linked {} skills into {}", made, root.display()));
    }
    good
}

/// The whole sync. Returns the lines it printed (none: up to date) and whether it worked.
pub fn sync(repo: &Path) -> (bool, Vec<(String, bool)>) {
    let src = repo.join("claude").join("skills");
    let claude = os::home().join(".claude");
    crate::out::capture(|| {
        let _ = fs::create_dir_all(&claude);
        let good = link_each(&claude.join("skills"), &src);
        if verbose() {
            plain("✓ skill links complete");
        }
        good
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn links_prunes_and_leaves_others() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("skills");
        for n in ["ps_a", "ps_b", "_template", ".hidden"] {
            fs::create_dir_all(src.join(n)).unwrap();
            fs::write(src.join(n).join("SKILL.md"), "x").unwrap();
        }
        fs::create_dir_all(src.join("no_skill_md")).unwrap();
        assert_eq!(list(&src), vec!["ps_a", "ps_b"]);
        let root = t.path().join("dst");
        fs::create_dir_all(root.join("ps_b")).unwrap(); // the user's own folder
        os::symlink(&src.join("gone"), &root.join("gone")).unwrap(); // our old link
        let (good, lines) = crate::out::capture(|| link_each(&root, &src));
        assert!(good);
        assert_eq!(os::readlink(&root.join("ps_a")).unwrap(), src.join("ps_a"));
        assert!(!os::is_symlink(&root.join("ps_b")));
        assert!(!os::exists_or_link(&root.join("gone")));
        let text: Vec<_> = lines.iter().map(|l| l.0.clone()).collect();
        assert!(text.iter().any(|l| l.contains("removed 1 old skill links")));
        assert!(text.iter().any(|l| l.contains("linked 1 skills")));
        let (_, again) = crate::out::capture(|| link_each(&root, &src));
        assert_eq!(again.len(), 1); // only the clash line
    }
}
