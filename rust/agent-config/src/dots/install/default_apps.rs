//! 71-default-apps: the default app per file type, from default-apps.list. No Finder pop-up:
//! scripts/default-apps.swift edits the Launch Services preferences and restarts lsd.
//! Best-effort: a failure is a warning and the run goes on.

use super::only_macos;
use crate::out::{self, info, ok, uptodate, verbose, warn};
use std::path::Path;
use std::process::{Command, Stdio};

/// What the script's one stdout line says.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// Every listed type already has its app.
    Same,
    /// This many types were changed and read back.
    Changed(usize),
    /// No result line (swift is missing, or the list is).
    Nothing,
}

fn parse(stdout: &str) -> Outcome {
    let mut words = stdout.split_whitespace();
    match (words.next(), words.next().and_then(|n| n.parse().ok())) {
        (Some("ok"), Some(_)) => Outcome::Same,
        (Some("changed"), Some(n)) => Outcome::Changed(n),
        _ => Outcome::Nothing,
    }
}

pub fn step(repo: &Path) {
    if !only_macos("default apps") {
        return;
    }
    out::step("default apps");
    let script = repo.join("scripts/default-apps.sh");
    if !script.is_file() {
        warn("scripts/default-apps.sh not found, skipping");
        return;
    }
    if verbose() {
        info("Setting the default app per file type (default-apps.list)…");
    }
    let Ok(res) = Command::new("bash").arg(&script).stdin(Stdio::null()).output() else {
        warn("default apps: could not run scripts/default-apps.sh");
        return;
    };
    let stderr = String::from_utf8_lossy(&res.stderr);
    for line in stderr.lines().filter(|l| !l.trim().is_empty()) {
        if res.status.success() && line.starts_with("skip ") {
            if verbose() {
                info(&format!("default apps: {line}"));
            }
        } else {
            warn(&format!("default apps: {line}"));
        }
    }
    if !res.status.success() {
        warn("default apps: some types did not change; run scripts/default-apps.sh to see which");
        return;
    }
    match parse(&String::from_utf8_lossy(&res.stdout)) {
        Outcome::Same => uptodate("default apps: all set"),
        Outcome::Changed(n) => ok(&format!("default apps: {n} types changed")),
        Outcome::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_result_line() {
        assert_eq!(parse("ok 27\n"), Outcome::Same);
        assert_eq!(parse("changed 3\n"), Outcome::Changed(3));
        assert_eq!(parse(""), Outcome::Nothing);
        assert_eq!(parse("changed x"), Outcome::Nothing);
    }
}
