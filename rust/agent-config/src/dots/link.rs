//! Links from $HOME into the repo: idempotent and non-destructive.

use super::os;
use crate::out::{count_skip, ok, warn};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static MADE: AtomicUsize = AtomicUsize::new(0);

/// The links made since the last call, and reset the count.
pub fn take_count() -> usize { MADE.swap(0, Ordering::Relaxed) }

fn base(p: &Path) -> String { p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() }

/// Link `dst` -> `src`. A link that is right is counted as up to date. Anything else at
/// `dst` is moved to `<dst>.bak-<timestamp>` first. `quiet`: count, print no line per link.
pub fn link_item(src: &Path, dst: &Path, quiet: bool) {
    if !src.exists() { return; }
    if os::is_symlink(dst) && os::readlink(dst).as_deref() == Some(src) {
        count_skip();
        return;
    }
    if os::exists_or_link(dst) {
        let bak = format!("{}.bak-{}", dst.display(), os::timestamp());
        if fs::rename(dst, &bak).is_ok() {
            warn(&format!("backed up existing {} → {}", base(dst), bak));
        } else {
            warn(&format!("couldn't back up {} — leaving it in place, skipping link", dst.display()));
            return;
        }
    }
    if let Some(parent) = dst.parent() { let _ = fs::create_dir_all(parent); }
    if os::symlink(src, dst).is_ok() {
        MADE.fetch_add(1, Ordering::Relaxed);
        if !quiet { ok(&format!("linked {}", base(dst))); }
    } else {
        warn(&format!("couldn't link {} → {}", dst.display(), src.display()));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn links_backs_up_and_skips() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src.txt");
        let dst = t.path().join("sub/dst.txt");
        fs::write(&src, "x").unwrap();
        take_count();
        link_item(&src, &dst, true);
        assert_eq!(os::readlink(&dst).unwrap(), src);
        assert_eq!(take_count(), 1);
        link_item(&src, &dst, true);
        assert_eq!(take_count(), 0);
        fs::remove_file(&dst).unwrap();
        fs::write(&dst, "mine").unwrap();
        link_item(&src, &dst, true);
        assert!(os::is_symlink(&dst));
        let baks: Vec<_> = fs::read_dir(t.path().join("sub")).unwrap().flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("dst.txt.bak-")).collect();
        assert_eq!(baks.len(), 1);
        link_item(&t.path().join("missing"), &t.path().join("never"), true);
        assert!(!os::exists_or_link(&t.path().join("never")));
    }
}
