//! Quiet output of `agent-config`: the default output is a count line per
//! command, not one line per file that is up to date. `skip`, `ok`, `header` and `summary` are
//! called directly and their output is taken with `out::capture`.
//!
//! The skip and written counters and the verbose switch are process-wide. They live in a test file of
//! their own, with a lock, so no other test spends the counts.
use agent_config::out::{self, capture, header, ok, set_verbose, skip, wrote};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

/// A quiet run (verbose off, no colour) with the counters spent.
fn quiet() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    set_verbose(false);
    out::set_color(false);
    call(|| out::summary("drain"));
    g
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
        wrote("wrote ~/x");
        ok("removed ~/y (gone)");
    });
    assert!(o.contains("✓ wrote ~/x"), "{o}");
    assert!(o.contains("✓ removed ~/y"), "{o}");
    assert_eq!(call(|| out::summary("Cursor config")), "• Cursor config: 1 up to date, 1 written\n");
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
