//! The short output (AGENTS.md "Short output"), used by `dots` and `agent-config`.
//!
//! A run prints one short line per step. A step header waits until the step prints
//! something. A step that only finds things up to date ends as one line:
//! "• Title: 5 up to date". DOTFILES_VERBOSE=1 (or `dots -v`) prints everything.
//! Warnings always print, and each one adds a line to DOTFILES_WARN_FILE so the last
//! line of a run can count the warnings of every step (bash steps are child processes).
//! Outside a step, skips count toward `summary`, the one line agent-config prints.

use std::env;
use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicUsize, Ordering};

/// `THEME=plain` in the dotfiles settings: no color codes at all. Read once per process.
fn plain_theme() -> bool {
    static PLAIN: OnceLock<bool> = OnceLock::new();
    *PLAIN.get_or_init(|| crate::setting(&crate::home_dir(), "THEME") == "plain")
}

/// Colors print when NO_COLOR is unset, the theme is not plain and stdout is a terminal.
fn colors_on(no_color: bool, plain: bool, tty: bool) -> bool { !no_color && !plain && tty }

/// True when this process prints color codes.
pub fn color_enabled() -> bool {
    if env::var("NO_COLOR").is_ok() { return false; }
    colors_on(false, plain_theme(), io::stdout().is_terminal())
}
fn c(code: &'static str) -> &'static str { if color_enabled() { code } else { "" } }
/// Sage color code for success; empty when color is off.
pub fn g() -> &'static str { c("\x1b[38;5;108m") }
/// Amber color code for warnings; empty when color is off.
pub fn y() -> &'static str { c("\x1b[38;5;179m") }
/// Stone color code for skipped and dim text; empty when color is off.
pub fn d() -> &'static str { c("\x1b[38;5;245m") }
/// Teal color code for info; empty when color is off.
pub fn b() -> &'static str { c("\x1b[38;5;73m") }
/// Copper color code for step headings; empty when color is off.
pub fn a() -> &'static str { c("\x1b[1;38;5;173m") }
/// The reset code that ends a color; empty when color is off.
pub fn z() -> &'static str { c("\x1b[0m") }

/// True when `DOTFILES_VERBOSE` is set (`dots -v`): print every line, count nothing.
pub fn verbose() -> bool { env::var("DOTFILES_VERBOSE").map(|v| !v.is_empty() && v != "0").unwrap_or(false) }

struct Step { title: String, shown: u8, skips: usize }
static STEP: Mutex<Step> = Mutex::new(Step { title: String::new(), shown: 0, skips: 0 });
static SKIPPED: AtomicUsize = AtomicUsize::new(0);
static WRITTEN: AtomicUsize = AtomicUsize::new(0);
static WARNS: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    /// While set, lines go here instead of the terminal (`capture`).
    static CAPTURED: std::cell::RefCell<Option<Vec<(String, bool)>>> = const { std::cell::RefCell::new(None) };
}

fn step_state() -> std::sync::MutexGuard<'static, Step> { STEP.lock().unwrap_or_else(|e| e.into_inner()) }

/// Print one line, on stderr when `err`. Captured lines are kept, in order.
fn emit(line: &str, err: bool) {
    let kept = CAPTURED.with(|c| match c.borrow_mut().as_mut() {
        Some(buf) => { buf.push((line.to_string(), err)); true }
        None => false,
    });
    if kept { return; }
    if err {
        let _ = io::stdout().flush();
        eprintln!("{}", line);
    } else {
        println!("{}", line);
    }
}

/// Run `f` and return what it printed, one entry per line (and whether it went to
/// stderr), instead of printing it.
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<(String, bool)>) {
    let outer = CAPTURED.with(|c| c.borrow_mut().replace(Vec::new()));
    let r = f();
    let lines = CAPTURED.with(|c| std::mem::replace(&mut *c.borrow_mut(), outer)).unwrap_or_default();
    (r, lines)
}

/// Print lines kept by `capture`, each on the stream it was meant for.
pub fn replay(lines: &[(String, bool)]) {
    for (l, err) in lines { emit(l, *err); }
}

/// Print the waiting step header, once.
fn head() {
    let mut s = step_state();
    if !s.title.is_empty() && s.shown == 0 {
        let t = s.title.clone();
        s.shown = 1;
        drop(s);
        emit(&format!("\n{}▸ {}{}", a(), t, z()), false);
    }
}

/// Start a step. The header prints when the step first prints something.
pub fn step(title: &str) {
    step_end();
    let v = verbose();
    if v { emit(&format!("\n{}▸ {}{}", a(), title, z()), false); }
    let mut s = step_state();
    s.title = title.to_string();
    s.shown = if v { 1 } else { 0 };
}

/// Close the step in progress with its count line.
pub fn step_end() {
    let mut s = step_state();
    let line = if !s.title.is_empty() && s.skips > 0 {
        Some(if s.shown == 1 {
            format!("{}•{} {} up to date", d(), z(), s.skips)
        } else if s.skips == 1 {
            format!("{}•{} {}: up to date", d(), z(), s.title)
        } else {
            format!("{}•{} {}: {} up to date", d(), z(), s.title, s.skips)
        })
    } else { None };
    s.title.clear();
    s.shown = 0;
    s.skips = 0;
    drop(s);
    if let Some(l) = line { emit(&l, false); }
}

/// The title of the step in progress, or "" between steps.
pub fn step_title() -> String { step_state().title.clone() }

/// An info line (`»»`), under the step header.
pub fn info(msg: &str) { head(); emit(&format!("{}»»{} {}", b(), z(), msg), false); }

/// A success line (`✓`). A message that starts with "wrote ", "updated " or "built " counts as a written file.
pub fn ok(msg: &str) {
    if msg.starts_with("wrote ") || msg.starts_with("updated ") || msg.starts_with("built ") { WRITTEN.fetch_add(1, Ordering::Relaxed); }
    head();
    emit(&format!("{}✓{} {}", g(), z(), msg), false);
}

/// A warning line on stderr. It is counted, and appended to `DOTFILES_WARN_FILE` when that is set.
pub fn warn(msg: &str) {
    head();
    emit(&format!("{}!{}  {}", y(), z(), msg), true);
    WARNS.fetch_add(1, Ordering::Relaxed);
    if let Ok(f) = env::var("DOTFILES_WARN_FILE") && !f.is_empty()
        && let Ok(mut fh) = OpenOptions::new().append(true).create(true).open(&f) {
        let _ = writeln!(fh, "{}", msg);
    }
}

/// The warnings printed by this process.
pub fn warn_count() -> usize { WARNS.load(Ordering::Relaxed) }

/// It was already there. Counted in a step (or toward `summary`), printed in verbose mode.
pub fn skip(msg: &str) {
    if verbose() {
        head();
        emit(&format!("{}•{} {} {}(already set up, skipping){}", d(), z(), msg, d(), z()), false);
        return;
    }
    let mut s = step_state();
    if s.title.is_empty() { SKIPPED.fetch_add(1, Ordering::Relaxed); } else { s.skips += 1; }
}

/// Count one more up-to-date item in the step, with no message (a link that is right).
pub fn count_skip() {
    if verbose() { return; }
    let mut s = step_state();
    if !s.title.is_empty() { s.skips += 1; }
}

/// The step found nothing to do: a line in verbose mode, a count otherwise.
pub fn uptodate(msg: &str) { if verbose() { ok(msg) } else { skip(msg) } }

/// A closing line, only when the step did or said something before.
pub fn ok_if_changed(msg: &str) {
    let (shown, empty) = { let s = step_state(); (s.shown, s.title.is_empty()) };
    if verbose() || shown != 0 || empty { ok(msg); }
}

/// The whole step in one line ("• Title: msg"), when there is nothing to do.
pub fn note(msg: &str) {
    let title = step_title();
    if verbose() || title.is_empty() { info(msg); return; }
    emit(&format!("{}•{} {}: {}", d(), z(), title, msg), false);
    step_state().shown = 2;
}

/// The line that says what a command reads. Verbose runs only.
pub fn header(msg: &str) { if verbose() { info(msg); } }

/// The count line for the files that were up to date, e.g. "Opencode config: 12 up to date, 1 written".
pub fn summary(label: &str) {
    let skipped = SKIPPED.swap(0, Ordering::Relaxed);
    let written = WRITTEN.swap(0, Ordering::Relaxed);
    if !verbose() && skipped > 0 {
        let more = if written > 0 { format!(", {} written", written) } else { String::new() };
        emit(&format!("{}•{} {}: {} up to date{}", d(), z(), label, skipped, more), false);
    }
}

/// Seconds as a short time: "42s", "6m12s", "1h03m".
pub fn duration(secs: u64) -> String {
    if secs < 60 { format!("{}s", secs) }
    else if secs < 3600 { format!("{}m{:02}s", secs / 60, secs % 60) }
    else { format!("{}h{:02}m", secs / 3600, secs % 3600 / 60) }
}

/// A plain line, the way a script prints it (no prefix, no step header).
pub fn plain(msg: &str) { emit(msg, false); }
/// Like `plain`, on stderr.
pub fn plain_err(msg: &str) { emit(msg, true); }

#[cfg(test)]
mod tests {
    use super::{colors_on, duration};
    #[test]
    fn durations() {
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(372), "6m12s");
        assert_eq!(duration(3780), "1h03m");
    }
    #[test]
    fn plain_theme_drops_color() {
        assert!(colors_on(false, false, true));
        assert!(!colors_on(false, true, true));
        assert!(!colors_on(true, false, true));
        assert!(!colors_on(false, false, false));
    }
}
