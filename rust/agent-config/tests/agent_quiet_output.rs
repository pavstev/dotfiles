//! `QuietOutputTest` of `scripts/test_agent_config.py`: the default output is a count line per
//! command, not one line per file that is up to date. `skip`, `ok`, `header` and `summary` are
//! called directly and their output is taken with `out::capture`.
//!
//! The skip and written counters and the verbose switch are process-wide. They live in a test file of
//! their own, with a lock, so no other test spends the counts. `tests/PORTED.md` maps each Python test
//! to its twin here.
use agent_config::out::{self, capture, header, ok, skip};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

/// A quiet run (no DOTFILES_VERBOSE, no colour) with the counters spent.
fn quiet() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: every test of this file holds LOCK, and no other thread reads these variables.
    unsafe {
        std::env::remove_var("DOTFILES_VERBOSE");
        std::env::set_var("NO_COLOR", "1");
    }
    call(|| out::summary("drain"));
    g
}

fn set_verbose(on: bool) {
    // SAFETY: as in `quiet`.
    unsafe {
        if on {
            std::env::set_var("DOTFILES_VERBOSE", "1");
        } else {
            std::env::remove_var("DOTFILES_VERBOSE");
        }
    }
}

/// What `f` printed to stdout.
fn call(f: impl FnOnce()) -> String {
    let ((), lines) = capture(f);
    lines.iter().filter(|(_, err)| !err).map(|(l, _)| format!("{l}\n")).collect()
}

#[test]
fn test_skip_counts_and_summary_prints_one_line() {
    let _g = quiet();
    let o = call(|| {
        skip("a");
        skip("b");
        skip("c");
    });
    assert_eq!(o, "");
    assert_eq!(call(|| out::summary("Opencode config")), "• Opencode config: 3 up to date\n");
    assert_eq!(call(|| out::summary("Opencode config")), ""); // the count is spent
}

#[test]
fn test_written_files_keep_their_line_and_join_the_count() {
    let _g = quiet();
    let o = call(|| {
        skip("a");
        ok("wrote ~/x");
        ok("removed ~/y (gone)");
    });
    assert!(o.contains("✓ wrote ~/x"), "{o}");
    assert!(o.contains("✓ removed ~/y"), "{o}");
    assert_eq!(call(|| out::summary("Cursor config")), "• Cursor config: 1 up to date, 1 written\n");
}

#[test]
fn test_nothing_skipped_prints_no_count_line() {
    let _g = quiet();
    assert_eq!(call(|| out::summary("Cursor config")), "");
}

#[test]
fn test_header_is_verbose_only() {
    let _g = quiet();
    assert_eq!(call(|| header("Cursor config from x")), "");
    set_verbose(true);
    let o = call(|| header("Cursor config from x"));
    set_verbose(false);
    assert!(o.contains("Cursor config from x"), "{o}");
}

#[test]
fn test_verbose_prints_every_skip_line() {
    let _g = quiet();
    set_verbose(true);
    let o = call(|| skip("~/a"));
    set_verbose(false);
    assert!(o.contains("(already set up, skipping)"), "{o}");
}
