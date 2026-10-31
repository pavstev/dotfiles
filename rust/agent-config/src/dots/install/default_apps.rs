//! 71-default-apps: the default app per file type, from default-apps.list. No Finder pop-up:
//! scripts/default-apps.swift edits the Launch Services preferences and restarts lsd.
//! This file reads and checks the list (pure, tested); the Swift script only touches the system.
//! Best-effort: a failure is a warning and the run goes on.

use super::only_macos;
use crate::out::{self, info, ok, uptodate, verbose, warn};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// One file type and the app that should open it.
#[derive(Debug, Clone, PartialEq)]
struct Pair {
    bundle: String,
    uti: String,
}

#[derive(Debug, PartialEq)]
struct Parsed {
    pairs: Vec<Pair>,
    /// Lines the list got wrong. Each one is a warning; the good lines still apply.
    problems: Vec<String>,
}

/// A bundle id or a UTI: letters, digits, dot, dash, underscore. It starts and ends with a
/// letter or digit, so `.json` (an extension, not a type) is refused.
fn is_name(s: &str) -> bool {
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    edge(s.chars().next())
        && edge(s.chars().last())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Reads default-apps.list. A line `@bundle.id` picks the app; each type line after it opens
/// with that app. `#` starts a comment (a whole line or the end of one). A type listed twice
/// keeps the last app, and the first one is reported.
fn parse_list(text: &str) -> Parsed {
    let mut pairs: Vec<Pair> = Vec::new();
    let mut problems = Vec::new();
    // `None` before the first `@`, and after an `@` line that is wrong.
    let mut bundle: Option<String> = None;
    let mut bundle_bad = false;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(id) = line.strip_prefix('@') {
            let id = id.trim();
            if is_name(id) {
                bundle = Some(id.to_string());
                bundle_bad = false;
            } else {
                problems.push(format!("line {n}: bad app id {line:?}"));
                bundle = None;
                bundle_bad = true;
            }
            continue;
        }
        if !is_name(line) {
            problems.push(format!("line {n}: bad type {line:?}"));
            continue;
        }
        let Some(b) = &bundle else {
            if !bundle_bad {
                problems.push(format!("line {n}: type {line} comes before any @app"));
            }
            continue;
        };
        match pairs.iter_mut().find(|p| p.uti.eq_ignore_ascii_case(line)) {
            Some(p) if p.bundle.eq_ignore_ascii_case(b) => {}
            Some(p) => {
                problems.push(format!("line {n}: {line} is set twice ({} then {b})", p.bundle));
                p.bundle.clone_from(b);
            }
            None => pairs.push(Pair { bundle: b.clone(), uti: line.to_string() }),
        }
    }
    Parsed { pairs, problems }
}

/// What the Swift script reads: one `bundle<TAB>uti` per line.
fn plan(pairs: &[Pair]) -> String {
    pairs.iter().fold(String::new(), |mut acc, p| {
        acc.push_str(&p.bundle);
        acc.push('\t');
        acc.push_str(&p.uti);
        acc.push('\n');
        acc
    })
}

/// What the script's result line says: the last non-empty stdout line.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// Every listed type already has its app.
    Same,
    /// This many types were changed and read back.
    Changed(usize),
    /// No result line.
    Nothing,
}

fn parse_outcome(stdout: &str) -> Outcome {
    let last = stdout.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut words = last.split_whitespace();
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
    let list = repo.join("default-apps.list");
    let script = repo.join("scripts/default-apps.swift");
    let Ok(text) = std::fs::read_to_string(&list) else {
        warn("default apps: default-apps.list not found, skipping");
        return;
    };
    if !script.is_file() {
        warn("scripts/default-apps.swift not found, skipping");
        return;
    }
    let parsed = parse_list(&text);
    for p in &parsed.problems {
        warn(&format!("default apps: {p}"));
    }
    if parsed.pairs.is_empty() {
        return;
    }
    if verbose() {
        info("Setting the default app per file type (default-apps.list)…");
    }
    let Ok(mut child) =
        Command::new("swift").arg(&script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
    else {
        warn("default apps: swift not found (run: xcode-select --install)");
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        // A write error means swift already quit; its exit status says why.
        let _ = stdin.write_all(plan(&parsed.pairs).as_bytes());
    }
    let Ok(res) = child.wait_with_output() else {
        warn("default apps: could not run scripts/default-apps.swift");
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
        warn("default apps: some types did not change (see the lines above)");
        return;
    }
    match parse_outcome(&String::from_utf8_lossy(&res.stdout)) {
        Outcome::Same => uptodate("default apps: all set"),
        Outcome::Changed(n) => ok(&format!("default apps: {n} types changed")),
        Outcome::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(bundle: &str, uti: &str) -> Pair {
        Pair { bundle: bundle.into(), uti: uti.into() }
    }

    #[test]
    fn real_list_parses_clean() {
        let text = include_str!("../../../../../default-apps.list");
        let p = parse_list(text);
        assert_eq!(p.problems, Vec::<String>::new());
        assert!(p.pairs.contains(&pair("org.videolan.vlc", "public.mpeg-4")));
        assert!(p.pairs.contains(&pair("com.microsoft.VSCode", "net.daringfireball.markdown")));
        // The .ts line belongs to VS Code, not VLC.
        assert!(p.pairs.contains(&pair("com.microsoft.VSCode", "public.mpeg-2-transport-stream")));
        let mut utis: Vec<_> = p.pairs.iter().map(|p| p.uti.to_lowercase()).collect();
        utis.sort();
        utis.dedup();
        assert_eq!(utis.len(), p.pairs.len(), "the real list has a duplicate type");
    }

    #[test]
    fn groups_types_under_the_last_app() {
        let p = parse_list("@org.videolan.vlc\npublic.mp3\n\n@com.microsoft.VSCode\npublic.json\n");
        assert_eq!(p.pairs, vec![pair("org.videolan.vlc", "public.mp3"), pair("com.microsoft.VSCode", "public.json")]);
        assert_eq!(p.problems, Vec::<String>::new());
    }

    #[test]
    fn empty_and_comment_only_lists_give_nothing() {
        for text in ["", "\n\n", "# only a comment\n   # another\n", "   \t \n"] {
            assert_eq!(parse_list(text), Parsed { pairs: vec![], problems: vec![] });
        }
    }

    #[test]
    fn trailing_spaces_crlf_and_inline_comments_are_ignored() {
        let p = parse_list("@com.microsoft.VSCode  \r\npublic.json \t\r\npublic.yaml # YAML files\r\n");
        assert_eq!(
            p.pairs,
            vec![pair("com.microsoft.VSCode", "public.json"), pair("com.microsoft.VSCode", "public.yaml")]
        );
        assert_eq!(p.problems, Vec::<String>::new());
    }

    #[test]
    fn space_after_at_is_allowed() {
        let p = parse_list("@ org.videolan.vlc\npublic.avi");
        assert_eq!(p.pairs, vec![pair("org.videolan.vlc", "public.avi")]);
    }

    #[test]
    fn type_before_any_app_is_reported() {
        let p = parse_list("public.json\n@com.microsoft.VSCode\npublic.css\n");
        assert_eq!(p.pairs, vec![pair("com.microsoft.VSCode", "public.css")]);
        assert_eq!(p.problems.len(), 1);
        assert!(p.problems[0].starts_with("line 1:"));
    }

    #[test]
    fn bad_app_id_skips_its_types_with_one_report() {
        // An app name with spaces is not a bundle id.
        let p = parse_list("@Visual Studio Code\npublic.json\npublic.css\n@org.videolan.vlc\npublic.mp3\n");
        assert_eq!(p.pairs, vec![pair("org.videolan.vlc", "public.mp3")]);
        assert_eq!(p.problems.len(), 1);
        assert!(p.problems[0].contains("Visual Studio Code"));
        assert_eq!(parse_list("@\npublic.json").problems.len(), 1);
    }

    #[test]
    fn bad_type_is_reported_and_the_rest_still_applies() {
        let p = parse_list("@com.microsoft.VSCode\n.json\npublic.json extra\npublic/json\npublic.yaml\n");
        assert_eq!(p.pairs, vec![pair("com.microsoft.VSCode", "public.yaml")]);
        assert_eq!(p.problems.len(), 3);
    }

    #[test]
    fn same_type_twice_for_the_same_app_is_one_pair() {
        let p = parse_list("@com.microsoft.VSCode\npublic.json\nPUBLIC.JSON\n");
        assert_eq!(p.pairs, vec![pair("com.microsoft.VSCode", "public.json")]);
        assert_eq!(p.problems, Vec::<String>::new());
    }

    #[test]
    fn same_type_for_two_apps_keeps_the_last_and_reports() {
        let p = parse_list("@org.videolan.vlc\npublic.mpeg\n@com.microsoft.VSCode\npublic.mpeg\n");
        assert_eq!(p.pairs, vec![pair("com.microsoft.VSCode", "public.mpeg")]);
        assert_eq!(p.problems.len(), 1);
        assert!(p.problems[0].contains("org.videolan.vlc"));
    }

    #[test]
    fn plan_is_one_tab_line_per_pair() {
        let pairs = vec![pair("org.videolan.vlc", "public.mp3"), pair("com.microsoft.VSCode", "public.json")];
        assert_eq!(plan(&pairs), "org.videolan.vlc\tpublic.mp3\ncom.microsoft.VSCode\tpublic.json\n");
        assert_eq!(plan(&[]), "");
    }

    #[test]
    fn outcome_reads_the_last_result_line() {
        assert_eq!(parse_outcome("ok 27\n"), Outcome::Same);
        assert_eq!(parse_outcome("changed 3\n"), Outcome::Changed(3));
        assert_eq!(parse_outcome("warning: noise\nchanged 12\n\n"), Outcome::Changed(12));
        assert_eq!(parse_outcome(""), Outcome::Nothing);
        assert_eq!(parse_outcome("changed x"), Outcome::Nothing);
        assert_eq!(parse_outcome("not applied: public.json"), Outcome::Nothing);
    }
}
