//! The bins that replaced the runtime shell scripts: `statusline` (statusline.sh), `skill_check`
//! (skills/check.py), `mic_check` (mic-check.sh), `bag_check` (bag-check.sh) and
//! `yt_transcribe fetch` (fetch.sh). Each test runs the real binary in a temp HOME.
//!
//! The status line sample outputs in `tests/fixtures/statusline/*.out` were captured once from
//! `claude/statusline.sh` (with `jq`) for the matching `*.json`; the Rust output is byte-identical.
mod common;

use common::{run_bin, temp_home};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const STATUSLINE: &str = env!("CARGO_BIN_EXE_statusline");
const SKILL_CHECK: &str = env!("CARGO_BIN_EXE_skill_check");
const MIC_CHECK: &str = env!("CARGO_BIN_EXE_mic_check");
const BAG_CHECK: &str = env!("CARGO_BIN_EXE_bag_check");
const YT: &str = env!("CARGO_BIN_EXE_yt_transcribe");

fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/statusline").join(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

// ---------------------------------------------------------------------------
// statusline
// ---------------------------------------------------------------------------

/// Color on: the run helper sets NO_COLOR=1, an empty NO_COLOR counts as unset.
const COLOR: [(&str, &str); 1] = [("NO_COLOR", "")];

#[test]
fn statusline_matches_shell_on_sample() {
    for name in ["sample", "small"] {
        let (code, out, err) = run_bin(STATUSLINE, &[], &fixture(&format!("{name}.json")), &COLOR, None);
        assert_eq!(code, 0, "{err}");
        assert_eq!(out, fixture(&format!("{name}.out")), "fixture {name}");
    }
}

#[test]
fn statusline_plain_prints_the_same_text_without_escape_codes() {
    let want = "\u{20}sample-project  Opus 4.7  ctx 354K 35%  5h 24% · 7d 41%";
    // NO_COLOR set
    let (_, out, _) = run_bin(STATUSLINE, &[], &fixture("sample.json"), &[("NO_COLOR", "1")], None);
    assert_eq!(out, want);
    // THEME=plain in the dotfiles settings
    let home = temp_home("sl-plain");
    fs::create_dir_all(home.join(".config/dotfiles")).unwrap();
    fs::write(home.join(".config/dotfiles/settings"), "THEME=\"plain\"\n").unwrap();
    let env = [("HOME", s(&home)), ("NO_COLOR", "")];
    let (_, out, _) = run_bin(STATUSLINE, &[], &fixture("sample.json"), &env, None);
    assert_eq!(out, want);
    assert!(!out.contains('\u{1b}'));
}

#[test]
fn statusline_writes_pace_cache() {
    let home = temp_home("sl-pace");
    let cache = home.join("xdg");
    let env = [("HOME", s(&home)), ("XDG_CACHE_HOME", s(&cache))];
    let (code, _, err) = run_bin(STATUSLINE, &[], &fixture("sample.json"), &env, None);
    assert_eq!(code, 0, "{err}");
    let text = fs::read_to_string(cache.join("claude-pace.json")).expect("pace cache written");
    let v: Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["updated", "five_hour", "seven_day"]);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert!(v["updated"].as_u64().unwrap().abs_diff(now) < 60, "{text}");
    assert_eq!(v["five_hour"], serde_json::json!({"used_percentage": 23.5, "resets_at": 1790000000}));
    assert_eq!(v["seven_day"], serde_json::json!({"used_percentage": 41.2, "resets_at": 1790500000}));
    assert!(text.contains(r#""five_hour":{"used_percentage":23.5,"resets_at":1790000000}"#), "compact: {text}");
    // written through a temp file: nothing else is left in the cache folder
    let left: Vec<String> = fs::read_dir(&cache).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, ["claude-pace.json"]);
}

#[test]
fn statusline_pace_cache_five_hour_null_and_absent_week() {
    let home = temp_home("sl-pace2");
    let cache = home.join("xdg");
    let env = [("HOME", s(&home)), ("XDG_CACHE_HOME", s(&cache))];
    run_bin(STATUSLINE, &[], r#"{"rate_limits":{"seven_day":{"used_percentage":7,"resets_at":5}}}"#, &env, None);
    let v: Value = serde_json::from_str(&fs::read_to_string(cache.join("claude-pace.json")).unwrap()).unwrap();
    assert_eq!(v["five_hour"], Value::Null);
    assert_eq!(v["seven_day"]["used_percentage"], 7);
    // no weekly limit yet: no cache file
    let other = temp_home("sl-pace3");
    let xdg = other.join("xdg");
    let env = [("HOME", s(&other)), ("XDG_CACHE_HOME", s(&xdg))];
    run_bin(STATUSLINE, &[], &fixture("small.json"), &env, None);
    assert!(!other.join("xdg/claude-pace.json").exists());
}

#[test]
fn statusline_no_jq_needed() {
    // a PATH with no tools at all: the shell script printed only the folder name here
    let empty = temp_home("sl-path");
    let (code, out, err) = run_bin(STATUSLINE, &[], &fixture("sample.json"), &[("PATH", s(&empty)), ("NO_COLOR", "")], None);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, fixture("sample.out"));
}

#[test]
fn statusline_shows_the_git_branch_and_falls_back_to_the_cwd() {
    let home = temp_home("sl-git");
    let repo = home.join("my-repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/feature-x"]);
    let json = format!(r#"{{"workspace":{{"current_dir":{:?}}}}}"#, s(&repo));
    let (_, out, _) = run_bin(STATUSLINE, &[], &json, &[("HOME", s(&home))], None);
    assert_eq!(out, " my-repo   feature-x");
    // no workspace and no cwd in the JSON: the process folder; garbage on stdin works too
    for input in ["{}", "not json", ""] {
        let (code, out, _) = run_bin(STATUSLINE, &[], input, &[("HOME", s(&home))], Some(&repo));
        assert_eq!(code, 0);
        assert_eq!(out, " my-repo   feature-x", "input {input:?}");
    }
}

#[test]
fn statusline_context_turns_copper_at_300k() {
    let low = r#"{"context_window":{"current_usage":{"input_tokens":299999}}}"#;
    let high = r#"{"context_window":{"current_usage":{"input_tokens":300000}}}"#;
    let copper = "\u{1b}[38;2;224;130;95mctx 300K";
    let (_, a, _) = run_bin(STATUSLINE, &[], low, &COLOR, Some(Path::new("/")));
    let (_, b, _) = run_bin(STATUSLINE, &[], high, &COLOR, Some(Path::new("/")));
    assert!(a.contains("\u{1b}[38;2;168;162;158mctx 299K"), "{a:?}");
    assert!(b.contains(copper), "{b:?}");
    // the session total is the fallback when the last call's usage is absent or zero
    let (_, c, _) = run_bin(STATUSLINE, &[], r#"{"context_window":{"total_input_tokens":4500}}"#, &[], Some(Path::new("/")));
    assert!(c.contains("ctx 4K"), "{c:?}");
}

/// The "what is left" file of a session in a temp HOME: the guard's Stop pass writes it
/// (`agent_config::status_path`, `dirs::cache_dir()/dots/status/<session>.txt`).
#[cfg(unix)]
fn status_file(home: &Path, session: &str) -> PathBuf {
    let cache = if cfg!(target_os = "macos") { home.join("Library/Caches") } else { home.join(".cache") };
    cache.join("dots/status").join(format!("{session}.txt"))
}

#[cfg(unix)]
fn left_line(home: &Path, session: &str, body: Option<&str>, input: &str) -> String {
    let file = status_file(home, session);
    if let Some(body) = body {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, body).unwrap();
    }
    let (code, out, err) = run_bin(STATUSLINE, &[], input, &[("HOME", s(home))], Some(Path::new("/")));
    assert_eq!(code, 0, "{err}");
    out
}

#[cfg(unix)]
#[test]
fn statusline_shows_what_is_left() {
    let home = temp_home("sl-left");
    let input = r#"{"session_id":"abc-1","model":{"display_name":"Opus"}}"#;
    let out = left_line(&home, "abc-1", Some("2\tRun tests\n"), input);
    assert!(out.ends_with("  Opus  ▸ 2 left: Run tests"), "{out:?}");
    // another session id: no segment
    let other = left_line(&home, "abc-1", None, r#"{"session_id":"zzz","model":{"display_name":"Opus"}}"#);
    assert!(!other.contains('▸'), "{other:?}");
    // no session id in the input: no segment
    assert!(!left_line(&home, "abc-1", None, r#"{"model":{"display_name":"Opus"}}"#).contains('▸'));
    // a session id with odd characters keeps only [A-Za-z0-9_-] in the name
    let odd = left_line(&home, "abc", Some("1\tOne\n"), r#"{"session_id":"a/../b c"}"#);
    assert!(odd.ends_with("▸ 1 left: One"), "{odd:?}");
}

#[cfg(unix)]
#[test]
fn statusline_cuts_a_long_step_and_survives_a_bad_file() {
    let home = temp_home("sl-left-cut");
    let long = "x".repeat(60);
    let out = left_line(&home, "s", Some(&format!("3\t{long}\n")), r#"{"session_id":"s"}"#);
    let want = format!("▸ 3 left: {}…", "x".repeat(39));
    assert!(out.ends_with(&want), "{out:?}");
    let short = "y".repeat(40);
    let out = left_line(&home, "s", Some(&format!("3\t{short}\n")), r#"{"session_id":"s"}"#);
    assert!(out.ends_with(&format!("▸ 3 left: {short}")), "{out:?}");
    for bad in ["", "garbage", "0\tNone\n", "x\tStep\n", "2\t\n"] {
        let out = left_line(&home, "s", Some(bad), r#"{"session_id":"s"}"#);
        assert!(!out.contains('▸'), "{bad:?} -> {out:?}");
    }
}

#[cfg(unix)]
#[test]
fn statusline_left_segment_is_copper_with_color() {
    let home = temp_home("sl-left-color");
    let file = status_file(&home, "s");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "2\tRun tests\n").unwrap();
    let env = [("HOME", s(&home)), ("NO_COLOR", "")];
    let (_, out, _) = run_bin(STATUSLINE, &[], r#"{"session_id":"s"}"#, &env, Some(Path::new("/")));
    assert!(out.ends_with("\u{1b}[38;2;224;130;95m▸ 2 left: Run tests\u{1b}[0m"), "{out:?}");
}

// ---------------------------------------------------------------------------
// skill_check
// ---------------------------------------------------------------------------

const GOOD: &str = "---
name: NAME
description: \"Use when the user asks for a demo: a short recipe. Third person.\"
allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/*)
---

# Demo

Run `bash ${CLAUDE_SKILL_DIR}/scripts/go.sh`. See [notes](notes.md).
";

/// A folder `<root>/<name with : as _>` holding `text` as SKILL.md, plus the files GOOD points at.
fn make_skill(root: &Path, name: &str, text: &str, junk: &[&str]) -> PathBuf {
    let d = root.join(name.replace(':', "_"));
    fs::create_dir_all(d.join("scripts")).unwrap();
    fs::write(d.join("SKILL.md"), text).unwrap();
    fs::write(d.join("scripts/go.sh"), "#!/bin/sh\n").unwrap();
    fs::write(d.join("notes.md"), "# notes\n").unwrap();
    for j in junk {
        fs::create_dir_all(d.join(j)).unwrap();
    }
    d
}

#[test]
fn skill_check_passes_repo_skills() {
    let root = repo_root();
    let (code, out, err) = run_bin(SKILL_CHECK, &["-q"], "", &[("DOTFILES_REPO", s(&root))], None);
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    // without -q the last line counts the skills
    let (code, out, _) = run_bin(SKILL_CHECK, &[], "", &[("DOTFILES_REPO", s(&root))], None);
    assert_eq!(code, 0);
    let last = out.lines().last().unwrap();
    assert!(last.ends_with(" skills checked, 0 failing"), "{last}");
    assert!(out.lines().any(|l| l.starts_with("OK   ") || l.starts_with("WARN ")), "{out}");
}

#[test]
fn skill_check_flags_bad_skill() {
    let root = temp_home("sc-bad");
    make_skill(&root, "ps:nameless", &GOOD.replace("name: NAME\n", ""), &[]);
    let (code, out, _) = run_bin(SKILL_CHECK, &[s(&root)], "", &[], None);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("FAIL ps_nameless\n       ✖ name missing\n"), "{out}");
    assert!(out.ends_with("1 skills checked, 1 failing\n"), "{out}");
    // -q: only the failure shows, and the count line because something failed
    let (code, out, _) = run_bin(SKILL_CHECK, &["-q", s(&root)], "", &[], None);
    assert_eq!(code, 1);
    assert!(out.starts_with("FAIL ps_nameless"), "{out}");
}

#[test]
fn skill_check_cases_from_the_python_self_test() {
    let root = temp_home("sc-cases");
    let good = GOOD.replace("NAME", "ps:good");
    let desc = "\"Use when the user asks for a demo: a short recipe. Third person.\"";
    let cases: Vec<(&str, String, Vec<&str>, i32)> = vec![
        ("ps:good", good.clone(), vec![], 0),
        ("ps:colon", good.replace("ps:good", "ps:colon").replace(desc, "Use when: a short recipe"), vec![], 1),
        ("ps:mismatch", good.replace("ps:good", "ps:other"), vec![], 1),
        ("ps:badname", good.replace("ps:good", "ps:Bad_Name"), vec![], 1),
        (
            "ps:hardpath",
            good.replace("ps:good", "ps:hardpath").replace("${CLAUDE_SKILL_DIR}/scripts/go.sh", "~/.claude/skills/ps_hardpath/scripts/go.sh"),
            vec![],
            1,
        ),
        ("ps:deadlink", good.replace("ps:good", "ps:deadlink").replace("notes.md", "nope.md"), vec![], 1),
        ("ps:junk", good.replace("ps:good", "ps:junk"), vec![".ruff_cache"], 1),
        (
            "ps:firstperson",
            good.replace("ps:good", "ps:firstperson").replace("Use when the user asks for a demo: a short recipe. Third person.", "I can help you with demos when asked."),
            vec![],
            1,
        ),
        ("ps:toolong", good.replace("ps:good", "ps:toolong") + &"xxxx\n".repeat(600), vec![], 1),
    ];
    for (name, text, junk, want) in cases {
        let d = make_skill(&root, name, &text, &junk);
        let (code, out, _) = run_bin(SKILL_CHECK, &[s(&d)], "", &[], None);
        assert_eq!(code, want, "{name}: {out}");
    }
    // the messages the Python gave for two of them
    let (_, out, _) = run_bin(SKILL_CHECK, &[s(&root.join("ps_mismatch"))], "", &[], None);
    assert!(out.contains("✖ name 'ps:other' needs folder 'ps_other', found 'ps_mismatch'"), "{out}");
    let (_, out, _) = run_bin(SKILL_CHECK, &[s(&root.join("ps_junk"))], "", &[], None);
    assert!(out.contains("✖ junk in skill folder: .ruff_cache"), "{out}");
    let (_, out, _) = run_bin(SKILL_CHECK, &[s(&root.join("ps_colon"))], "", &[], None);
    assert!(out.contains("✖ description: unquoted value contains ': ' — quote the whole value"), "{out}");
}

#[test]
fn skill_check_names_resolve_under_the_repo_skills() {
    let root = repo_root();
    let (code, out, _) = run_bin(SKILL_CHECK, &["ps:askme"], "", &[("DOTFILES_REPO", s(&root))], None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("ps_askme"), "{out}");
    assert!(out.ends_with("1 skills checked, 0 failing\n"), "{out}");
    let (code, out, _) = run_bin(SKILL_CHECK, &["ps:nope-nothing"], "", &[("DOTFILES_REPO", s(&root))], None);
    assert_eq!(code, 1);
    assert!(out.contains("✖ no SKILL.md"), "{out}");
}

#[test]
fn skill_check_with_no_skills_found_is_an_error_not_a_pass() {
    let empty = temp_home("sc-empty");
    let (code, out, err) = run_bin(SKILL_CHECK, &[], "", &[("DOTFILES_REPO", s(&empty))], Some(&empty));
    assert_eq!(code, 1, "{out}{err}");
    assert_eq!(out, "");
    assert!(err.contains("no skills found in "), "{err}");
    assert!(err.contains("DOTFILES_REPO"), "{err}");
    assert!(err.contains(&format!("{}", empty.join("claude").join("skills").display())), "{err}");
    // -q does not turn the miss into a pass either
    let (code, _, _) = run_bin(SKILL_CHECK, &["-q"], "", &[("DOTFILES_REPO", s(&empty))], Some(&empty));
    assert_eq!(code, 1);
}

// ---------------------------------------------------------------------------
// mic_check, bag_check
// ---------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=Tester", "-c", "user.email=tester@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo on branch `main` with `n` commits.
fn temp_repo(tag: &str, n: usize) -> (PathBuf, PathBuf) {
    let home = temp_home(tag);
    let repo = home.join("demo-repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    for i in 0..n {
        fs::write(repo.join("f.txt"), format!("{i}\n")).unwrap();
        git(&repo, &["add", "f.txt"]);
        git(&repo, &["commit", "-q", "-m", &format!("commit {i}")]);
    }
    (home, repo)
}

#[test]
fn mic_check_help() {
    let (code, out, _) = run_bin(MIC_CHECK, &["--help"], "", &[], None);
    assert_eq!(code, 0);
    for want in ["mic_check [repo...]", "--no-fetch", "--note", "--leftovers", "--since", "VERDICT: CLEAN"] {
        assert!(out.contains(want), "{want}: {out}");
    }
}

#[test]
fn mic_check_reports_a_temp_repo() {
    let (home, repo) = temp_repo("mic-run", 1);
    let (code, out, err) = run_bin(MIC_CHECK, &["--no-fetch"], "", &[("HOME", s(&home))], Some(&repo));
    assert_eq!(code, 1, "{out}{err}");
    let want = [
        "== demo-repo  (",
        "  OK    no conflicts, no half-done merge/rebase\n",
        // no origin: the main branch falls back to `master`, like the shell script
        "  WARN  on branch 'main', not 'master' — merge it, or open a PR (ask the user)\n",
        "  OK    working tree clean\n",
        "  FIX   branch 'main' has no upstream — never pushed\n",
        "  WARN  no known check command found",
    ];
    for w in want {
        assert!(out.contains(w), "{w}: {out}");
    }
    assert!(out.ends_with("VERDICT: NOT CLEAN (1 mandatory)\n"), "{out}");
    // a dirty tree is a FIX, and the status lines follow it
    fs::write(repo.join("new.txt"), "x\n").unwrap();
    let (code, out, _) = run_bin(MIC_CHECK, &["--no-fetch"], "", &[("HOME", s(&home))], Some(&repo));
    assert_eq!(code, 1);
    assert!(out.contains("  FIX   not committed: 0 staged, 0 changed, 1 new file(s)\n          ?? new.txt\n"), "{out}");
    assert!(out.ends_with("VERDICT: NOT CLEAN (2 mandatory)\n"), "{out}");
}

#[test]
fn mic_check_clean_with_an_upstream_and_a_check_command() {
    let (home, repo) = temp_repo("mic-clean", 1);
    // a bare origin the repo is in sync with
    let origin = home.join("origin.git");
    git(&home, &["init", "-q", "--bare", s(&origin)]);
    git(&repo, &["remote", "add", "origin", s(&origin)]);
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    fs::write(repo.join("Cargo.toml"), "[package]\n").unwrap();
    git(&repo, &["add", "Cargo.toml"]);
    git(&repo, &["commit", "-q", "-m", "cargo"]);
    git(&repo, &["push", "-q"]);
    let (code, out, err) = run_bin(MIC_CHECK, &[], "", &[("HOME", s(&home))], Some(&repo));
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("  OK    in sync with origin/main ("), "{out}");
    assert!(out.contains("  RUN   verification checks for this repo:\n          cargo clippy --all-targets && cargo nextest run\n"), "{out}");
    assert!(out.ends_with("VERDICT: CLEAN\n"), "{out}");
}

#[test]
fn mic_check_since_note_and_leftovers() {
    let (home, repo) = temp_repo("mic-since", 3);
    let second = git(&repo, &["rev-parse", "--short", "HEAD~1"]);
    let first = git(&repo, &["rev-parse", "--short", "HEAD~2"]);
    let left = home.join("left.md");
    let env = [("HOME", s(&home)), ("MIC_LEFTOVERS", s(&left))];
    let (_, out, _) = run_bin(MIC_CHECK, &["--no-fetch", "--since", &second], "", &env, Some(&repo));
    assert!(out.contains(&format!("  OK    2 session commit(s) on top of base {first} (squash: git reset --soft {first})\n")), "{out}");
    // newest first, each line indented ten spaces: "<sha> <author>  <subject>"
    let sha = |rev: &str| git(&repo, &["rev-parse", "--short", rev]);
    let want = format!("          {} Tester  commit 2\n          {} Tester  commit 1\n", sha("HEAD"), sha("HEAD~1"));
    assert!(out.contains(&want), "{out}");
    // a sha that is not an ancestor
    let (_, out, _) = run_bin(MIC_CHECK, &["--no-fetch", "--since", "deadbeef"], "", &env, Some(&repo));
    assert!(out.contains("  WARN  --since deadbeef is not an ancestor of HEAD here: no squash range for this repo\n"), "{out}");
    // --leftovers before and after a note
    let (code, out, _) = run_bin(MIC_CHECK, &["--leftovers"], "", &env, None);
    assert_eq!(code, 0);
    assert_eq!(out, format!("no leftovers file yet ({})\n", left.display()));
    let (code, out, _) = run_bin(MIC_CHECK, &["--note", "tidy later"], "", &env, Some(&repo));
    assert_eq!((code, out), (0, format!("saved to {}\n", left.display())));
    let text = fs::read_to_string(&left).unwrap();
    assert!(text.starts_with("# Leftovers\n\nOptional fixes saved by ps:mic-drop. Delete a line when done.\n- [ ] "), "{text}");
    assert!(text.ends_with(" · **demo-repo** · tidy later\n"), "{text}");
    let (_, out, _) = run_bin(MIC_CHECK, &["--no-fetch"], "", &env, Some(&repo));
    assert!(out.contains(&format!("leftovers file: {} (1 open)\n", left.display())), "{out}");
    // --since needs a value
    let (code, _, err) = run_bin(MIC_CHECK, &["--since"], "", &env, Some(&repo));
    assert_eq!(code, 1);
    assert!(err.contains("--since needs a sha or time"), "{err}");
}

#[test]
fn mic_check_skips_a_folder_that_is_not_a_repo() {
    let home = temp_home("mic-norepo");
    let plain = home.join("plain");
    fs::create_dir_all(&plain).unwrap();
    let (code, out, _) = run_bin(MIC_CHECK, &["--no-fetch", s(&plain)], "", &[("HOME", s(&home)), ("GIT_CEILING_DIRECTORIES", s(&home))], None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&format!("== {} ==\n  WARN  not a git repo — skipped\n", plain.display())), "{out}");
    assert!(out.ends_with("VERDICT: CLEAN\n"), "{out}");
}

#[test]
fn a_closed_pipe_ends_the_bins_quietly() {
    // `mic_check | head` once the reader is gone: no panic, exit 0, like the shell scripts
    for (bin, args) in [(MIC_CHECK, "--help"), (BAG_CHECK, "--help"), (SKILL_CHECK, "--help")] {
        let mut child = Command::new(bin)
            .arg(args)
            .env("HOME", temp_home("pipe"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("start");
        drop(child.stdout.take()); // close the read end before the bin prints
        let out = child.wait_with_output().expect("wait");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("panicked") && !err.contains("failed printing"), "{bin}: {err}");
        assert_eq!(out.status.code(), Some(0), "{bin}: {err}");
    }
}

#[test]
fn bag_check_help() {
    let (code, out, _) = run_bin(BAG_CHECK, &["--help"], "", &[], None);
    assert_eq!(code, 0);
    for want in ["bag_check [--eta 45m]", "--throttle", "--restore", "--watch [N]", "VERDICT:"] {
        assert!(out.contains(want), "{want}: {out}");
    }
}

#[test]
fn bag_check_card_or_not_applicable() {
    let (code, out, err) = run_bin(BAG_CHECK, &["--eta", "5m"], "", &[], None);
    assert_eq!(code, 0, "{err}");
    let last = out.lines().last().unwrap();
    if cfg!(target_os = "macos") {
        assert!(out.starts_with("## 🎒 Bag Mode — "), "{out}");
        assert!(out.contains("| ⏱ ETA | ~"), "{out}");
        assert!(last.starts_with("VERDICT: "), "{last}");
    } else {
        // scripts/test-linux.sh greps for this
        assert!(out.contains("bag-mode is for a Mac laptop"), "{out}");
        assert_eq!(last, "VERDICT: NOT APPLICABLE (not a Mac)");
    }
}

// ---------------------------------------------------------------------------
// yt_transcribe fetch
// ---------------------------------------------------------------------------

#[test]
fn yt_fetch_uses_cache() {
    let home = temp_home("yt-cache");
    let dir = home.join(".cache/yt-learn");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("dQw4w9WgXcQ.md");
    fs::write(&file, "# Transcript: dQw4w9WgXcQ\n").unwrap();
    // an empty PATH: nothing can reach the network, so a hit must come from the file
    let empty = temp_home("yt-path");
    let env = [("HOME", s(&home)), ("PATH", s(&empty))];
    for url in ["https://youtu.be/dQw4w9WgXcQ", "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3s", "dQw4w9WgXcQ"] {
        let (code, out, err) = run_bin(YT, &["fetch", url], "", &env, None);
        assert_eq!(code, 0, "{url}: {err}");
        assert_eq!(out, format!("{}\n", file.display()), "{url}");
    }
    assert_eq!(fs::read_to_string(&file).unwrap(), "# Transcript: dQw4w9WgXcQ\n");
}

#[test]
fn yt_fetch_misses_and_errors() {
    let home = temp_home("yt-miss");
    let empty = temp_home("yt-path2");
    let env = [("HOME", s(&home)), ("PATH", s(&empty))];
    // no argument: usage, exit 2
    let (code, _, err) = run_bin(YT, &["fetch"], "", &env, None);
    assert_eq!(code, 2);
    assert!(err.contains("usage: yt_transcribe fetch <youtube url>"), "{err}");
    // not a YouTube link: the parser's message, exit 1
    let (code, _, err) = run_bin(YT, &["fetch", "https://example.com/x"], "", &env, None);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: Cannot find a YouTube video ID"), "{err}");
    // an empty cache file is not a hit: it goes on to the tools (missing here) and fails
    let dir = home.join(".cache/yt-learn");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("dQw4w9WgXcQ.md"), "").unwrap();
    let (code, out, err) = run_bin(YT, &["fetch", "dQw4w9WgXcQ"], "", &env, None);
    assert_eq!(code, 1, "{out}");
    assert!(err.contains("yt-dlp is not installed"), "{err}");
    assert_eq!(out, "");
    // the old single-video form still works
    let (code, out, _) = run_bin(YT, &["--print-id", "https://youtu.be/dQw4w9WgXcQ"], "", &env, None);
    assert_eq!((code, out.as_str()), (0, "dQw4w9WgXcQ\n"));
}
