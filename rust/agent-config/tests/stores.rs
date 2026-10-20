//! Store readers: Claude Code sessions in a temp HOME, and the path functions.

use agent_config::stores::{Doc, claude_root, cursor_db, opencode_db, read_claude};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SESSION: &str = "11111111-2222-3333-4444-555555555555";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn project_dir(home: &Path) -> PathBuf {
    claude_root(home).join("-work-demo")
}

fn main_file(home: &Path) -> PathBuf {
    project_dir(home).join(format!("{SESSION}.jsonl"))
}

fn sub_file(home: &Path) -> PathBuf {
    project_dir(home).join(SESSION).join("subagents").join("agent-a1.jsonl")
}

const MAIN: &str = concat!(
    r#"{"type":"user","timestamp":"2026-10-20T10:00:00.000Z","message":{"role":"user","content":"fix the login bug"}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-10-20T10:00:05.000Z","message":{"id":"m1","content":[{"type":"thinking","thinking":"private thoughts"},{"type":"text","text":"Looking at it now."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la","description":"list"}}]}}"#,
    "\n",
    r#"{"type":"user","timestamp":"2026-10-20T10:00:06.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"TOOL-OUTPUT-BODY"}]}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-10-20T10:00:07.000Z","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/work/demo/src/lib.rs"}}]}}"#,
    "\n",
    r#"{"type":"user","timestamp":"2026-10-20T10:00:08.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":"File does not exist."}]}}"#,
    "\n",
    r#"{"type":"user","timestamp":"2026-10-20T10:00:09.000Z","isMeta":true,"message":{"content":"meta noise"}}"#,
    "\n",
);

const SUB: &str = concat!(
    r#"{"type":"user","timestamp":"2026-10-20T10:01:00.000Z","isSidechain":true,"message":{"role":"user","content":"survey the tests folder"}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-10-20T10:01:05.000Z","isSidechain":true,"message":{"id":"s1","content":[{"type":"text","text":"Survey done."}]}}"#,
    "\n",
);

fn collect(home: &Path) -> Vec<Doc> {
    let mut docs = Vec::new();
    read_claude(home, &BTreeMap::new(), &mut |d| docs.push(d)).expect("read_claude");
    docs
}

fn find<'a>(docs: &'a [Doc], role: &str, needle: &str) -> Option<&'a Doc> {
    docs.iter().find(|d| d.role == role && d.text.contains(needle))
}

#[test]
fn paths_per_os() {
    let home = Path::new("/h");
    assert_eq!(claude_root(home), home.join(".claude").join("projects"));
    assert!(opencode_db(home).ends_with(".local/share/opencode/opencode.db"));
    let tail = if cfg!(target_os = "macos") {
        "Library/Application Support/Cursor/User/globalStorage/state.vscdb"
    } else if cfg!(windows) {
        "AppData/Roaming/Cursor/User/globalStorage/state.vscdb"
    } else {
        ".config/Cursor/User/globalStorage/state.vscdb"
    };
    assert!(cursor_db(home).ends_with(tail), "{}", cursor_db(home).display());
}

#[test]
fn claude_reads_main_and_subagents() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    write(&main_file(home), MAIN);
    write(&sub_file(home), SUB);
    write(
        &home.join("Library/Application Support/Claude/claude-code-sessions/a/b/local_x.json"),
        &format!(r#"{{"cliSessionId":"{SESSION}","title":"Login bug","isArchived":false}}"#),
    );

    let docs = collect(home);

    let user = find(&docs, "user", "fix the login bug").expect("user doc");
    assert_eq!(user.source, "claude");
    assert_eq!(user.session, SESSION);
    assert_eq!(user.title, "Login bug");
    assert_eq!(user.project, "demo"); // the leading "work-" is dropped, as in harness_stats
    assert_eq!(user.time, 1_792_490_400); // 2026-10-20T10:00:00Z

    assert!(find(&docs, "assistant", "Looking at it now.").is_some());
    assert!(find(&docs, "tool", "Bash ls -la").is_some());
    assert!(find(&docs, "tool", "Read /work/demo/src/lib.rs").is_some());
    assert!(find(&docs, "tool_error", "File does not exist.").is_some());

    // The subagent file belongs to the parent session.
    let sub = find(&docs, "user", "survey the tests folder").expect("subagent doc");
    assert_eq!(sub.session, SESSION);
    assert!(find(&docs, "assistant", "Survey done.").is_some());

    // Never indexed: tool output, thinking, meta lines, the tool description.
    for d in &docs {
        for bad in ["TOOL-OUTPUT-BODY", "private thoughts", "meta noise", "list"] {
            assert!(!d.text.contains(bad), "{bad} leaked into {d:?}");
        }
    }
    assert_eq!(docs.len(), 7, "{docs:#?}");
}

#[test]
fn claude_half_last_line_skipped() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let half = r#"{"type":"user","timestamp":"2026-10-20T10:02:00.000Z","message":{"content":"never finished"#;
    write(&main_file(home), &format!("{MAIN}{half}"));

    let docs = collect(home);
    assert_eq!(docs.len(), 5, "{docs:#?}");
    assert!(find(&docs, "user", "never finished").is_none());
}

#[test]
fn unchanged_files_are_skipped_and_kept_in_the_state() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    write(&main_file(home), MAIN);

    let mut first = Vec::new();
    let state = read_claude(home, &BTreeMap::new(), &mut |d| first.push(d)).unwrap();
    assert_eq!(state.len(), 1);
    assert!(!first.is_empty());

    let mut second = Vec::new();
    let again = read_claude(home, &state, &mut |d| second.push(d)).unwrap();
    assert!(second.is_empty());
    assert_eq!(again, state);
}

#[test]
fn missing_root_is_a_skip() {
    let t = tempfile::tempdir().unwrap();
    let r = read_claude(t.path(), &BTreeMap::new(), &mut |_| {});
    assert!(matches!(r, Err(agent_config::stores::Skip::Missing)));
}

#[test]
fn forbidden_tables_never_named() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("stores");
    let mut seen = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        seen += 1;
        let text = fs::read_to_string(&p).unwrap();
        // A SQL string is a string literal that holds a SQL verb.
        for line in text.lines() {
            let l = line.to_lowercase();
            let sql = l.contains('"') && (l.contains("select ") || l.contains(" from ") || l.contains("join "));
            if !sql {
                continue;
            }
            for name in ["credential", "account", "account_state", "control_account", " kv"] {
                assert!(!l.contains(name), "{}: SQL names {name}: {line}", p.display());
            }
        }
    }
    assert!(seen >= 1, "no source files found in {}", dir.display());
}

#[test]
fn secrets_redacted() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let key = ["sk", "-", &"a1B2c3D4e5".repeat(4)].concat();
    let prompt = format!(r#"{{"type":"user","timestamp":"2026-10-20T10:00:00.000Z","message":{{"content":"use {key} for the call"}}}}"#);
    let cmd = format!(r#"{{"type":"assistant","timestamp":"2026-10-20T10:00:01.000Z","message":{{"id":"m","content":[{{"type":"tool_use","id":"t","name":"Bash","input":{{"command":"curl -H 'Authorization: Bearer {}'"}}}}]}}}}"#, "abcdefghij".repeat(3));
    write(&main_file(home), &format!("{prompt}\n{cmd}\n"));

    let docs = collect(home);
    assert_eq!(docs.len(), 2);
    for d in &docs {
        assert!(!d.text.contains(&key), "{d:?}");
        assert!(!d.text.contains("abcdefghijabcdefghij"), "{d:?}");
        assert!(d.text.contains("[REDACTED]"), "{d:?}");
    }
}

#[test]
fn error_text_is_redacted_before_the_cut() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    // A token that starts at char 965 of the error text: a cut at 1,000 chars would leave 35 of 36.
    let token = ["gh", "p_", &"A1b2C3d4E5".repeat(4)].concat(); // # scan-allowlist
    let body = format!("{}{} tail", "x ".repeat(482), token);
    let call = r#"{"type":"assistant","timestamp":"2026-10-20T10:00:00.000Z","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"}}]}}"#;
    let result = format!(r#"{{"type":"user","timestamp":"2026-10-20T10:00:01.000Z","message":{{"content":[{{"type":"tool_result","tool_use_id":"t","is_error":true,"content":"{body}"}}]}}}}"#);
    write(&main_file(home), &format!("{call}\n{result}\n"));
    let docs = collect(home);
    let err = docs.iter().find(|d| d.role == "tool_error").expect("tool_error doc");
    assert!(!err.text.contains("ghp_"), "{}", err.text);
    assert!(!err.text.contains(&token[..30]), "{}", err.text);
}
