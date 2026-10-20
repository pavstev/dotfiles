//! Port of the skill-script tests that the unit tests inside the bins do not already cover:
//! `test_route.py::test_wrong_types_stay_bad_input`, `test_whatsleft_scan.py` (`FreshnessTest`) and
//! `test_harness_stats.py`. Every test runs the real binary against a temp HOME and temp files.
//! The spawn and route tests that call Python functions directly are unit tests in
//! `src/bin/spawn.rs` and `src/bin/route.rs`. `tests/PORTED.md` maps each Python test to its twin.
mod common;

use common::{run_bin, temp_home};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ROUTE: &str = env!("CARGO_BIN_EXE_route");
const SCAN: &str = env!("CARGO_BIN_EXE_whatsleft_scan");
const STATS: &str = env!("CARGO_BIN_EXE_harness_stats");

// ---------------------------------------------------------------------------
// test_route.py
// ---------------------------------------------------------------------------

#[test]
fn test_wrong_types_stay_bad_input() {
    let bad = [
        r#"[{"id": 5, "kind": "work", "text": "t"}]"#,
        r#"[{"id": ["a"], "kind": "work", "text": "t"}]"#,
        r#"[{"kind": "nope", "text": "t"}]"#,
        r#"[{"kind": "work", "risk": "rm -rf", "text": "t"}]"#,
        r#"[{"kind": "work", "text": "t", "after": ["gone"]}]"#,
    ];
    for rows in bad {
        let (code, out, _) = run_bin(ROUTE, &[], rows, &[], None);
        assert_eq!(code, 2, "{rows}: {out}");
        assert!(out.starts_with("route.py: bad input: "), "{rows}: {out}");
    }
}

// ---------------------------------------------------------------------------
// test_whatsleft_scan.py: FreshnessTest, through the binary.
// The Python tests called `check_freshness` on a parsed transcript; the Rust function is private, so
// each test writes the same small transcript to a temp repo and reads the section of `whatsleft_scan`.
// ---------------------------------------------------------------------------

const PASSED: &str = "a check ran after the last edit, and it passed";
const STALE: &str = "files edited after the last check run";
const FAILED: &str = "the last check run failed";
const NO_CHECK: &str = "no test/lint/check ran";
const RUNNING: &str = "the last check is still running: wait for its notice, then read its exit line";

struct Transcript {
    home: PathBuf,
    root: PathBuf,
    lines: Vec<Value>,
    n: usize,
}

impl Transcript {
    /// A temp HOME with a fresh git repo in it (its real path, as git reports it).
    fn new() -> Transcript {
        let home = temp_home("scan");
        let root = home.join("repo");
        fs::create_dir_all(&root).unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .expect("git init");
        assert!(ok.success());
        let root = fs::canonicalize(&root).unwrap();
        Transcript { home, root, lines: vec![], n: 0 }
    }

    fn ts(&mut self) -> String {
        self.n += 1;
        format!("2026-10-14T10:{:02}:00+00:00", self.n)
    }

    fn tool(&mut self, name: &str, inp: Value, result: &str, error: bool, cwd: Option<&Path>) -> String {
        let tid = format!("toolu_{}", self.n);
        let cwd = cwd.unwrap_or(&self.root).to_string_lossy().into_owned();
        let t = self.ts();
        self.lines.push(json!({"type": "assistant", "timestamp": t, "cwd": cwd,
            "message": {"content": [{"type": "tool_use", "id": tid, "name": name, "input": inp}]}}));
        let t = self.ts();
        self.lines.push(json!({"type": "user", "timestamp": t,
            "message": {"content": [{"type": "tool_result", "tool_use_id": tid, "content": result, "is_error": error}]}}));
        tid
    }

    fn edit(&mut self) -> String {
        let p = self.root.join("app.py").to_string_lossy().into_owned();
        self.tool("Edit", json!({"file_path": p}), "ok", false, None)
    }

    fn bash(&mut self, cmd: &str) -> String {
        self.tool("Bash", json!({"command": cmd}), "ok", false, None)
    }

    fn bash_failing(&mut self, cmd: &str) -> String {
        self.tool("Bash", json!({"command": cmd}), "ok", true, None)
    }

    fn bash_in(&mut self, cmd: &str, cwd: &Path) -> String {
        self.tool("Bash", json!({"command": cmd}), "ok", false, Some(cwd))
    }

    fn bash_bg(&mut self, cmd: &str, bg_id: &str) -> String {
        let res = format!("Command running in background with ID: {bg_id}. Output is being written to: /x/{bg_id}.output");
        self.tool("Bash", json!({"command": cmd, "run_in_background": true}), &res, false, None)
    }

    fn notice(&mut self, tool_id: &str, status: &str, summary: &str, task: &str) {
        let text = format!(
            "<task-notification>\n<task-id>{task}</task-id>\n<tool-use-id>{tool_id}</tool-use-id>\n\
             <status>{status}</status>\n<summary>{summary}</summary>\n</task-notification>"
        );
        let t = self.ts();
        self.lines.push(json!({"type": "attachment", "timestamp": t,
            "attachment": {"commandMode": "task-notification", "prompt": text}}));
    }

    /// What `whatsleft_scan` prints for this transcript.
    fn freshness(&self) -> String {
        let path = self.home.join("session.jsonl");
        let body: String = self.lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(&path, body).unwrap();
        let home = self.home.to_string_lossy().into_owned();
        let (code, out, err) = run_bin(
            SCAN,
            &["--session", path.to_str().unwrap(), "--no-fetch", "--no-ci"],
            "",
            &[("HOME", &home)],
            Some(&self.root),
        );
        assert_eq!(code, 0, "{err}");
        out
    }
}

impl Drop for Transcript {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

/// True when a line of the report has `tag` first and holds `text`.
fn tagged(out: &str, tag: &str, text: &str) -> bool {
    out.lines().any(|l| l.trim_start().starts_with(tag) && l.contains(text))
}

fn no_alarm(out: &str) {
    for bad in [STALE, FAILED, NO_CHECK, "exit code is hidden"] {
        assert!(!out.contains(bad), "unexpected `{bad}` in:\n{out}");
    }
}

#[test]
fn test_foreground_unmasked_check_after_edit_passes() {
    let mut t = Transcript::new();
    t.edit();
    t.bash("pnpm check");
    let out = t.freshness();
    assert!(tagged(&out, "OK", PASSED), "{out}");
    no_alarm(&out);
}

#[test]
fn test_foreground_pipe_with_pipefail_passes() {
    let mut t = Transcript::new();
    t.edit();
    t.bash("set -o pipefail; pnpm check 2>&1 | tail -20");
    assert!(t.freshness().contains(PASSED));
}

#[test]
fn test_check_chained_with_and_is_not_masked() {
    let mut t = Transcript::new();
    t.edit();
    t.bash("pnpm check && echo done");
    assert!(t.freshness().contains(PASSED));
}

#[test]
fn test_failed_foreground_check_still_fails() {
    let mut t = Transcript::new();
    t.edit();
    t.bash_failing("pnpm check");
    let out = t.freshness();
    assert!(tagged(&out, "ME", FAILED), "{out}");
    assert!(!out.contains(PASSED));
}

// --- a background check counts by its notice ---

#[test]
fn test_background_check_without_notice_is_still_running() {
    let mut t = Transcript::new();
    t.edit();
    t.bash_bg("pnpm check", "bg1");
    let out = t.freshness();
    assert!(tagged(&out, "ME", RUNNING), "{out}");
    assert!(!out.contains(PASSED), "{out}");
}

#[test]
fn test_background_check_failed_or_non_zero_exit_failed() {
    let cases = [
        ("failed", "Background command \"pnpm check\" failed with exit code 1"),
        ("completed", "Background command \"pnpm check\" completed (exit code 2)"),
        ("killed", "Background command \"pnpm check\" was stopped"),
    ];
    for (status, summary) in cases {
        let mut t = Transcript::new();
        t.edit();
        let tid = t.bash_bg("pnpm check", "bg1");
        t.notice(&tid, status, summary, "bg1");
        let out = t.freshness();
        assert!(out.contains(FAILED), "{status}: {out}");
        assert!(!out.contains(PASSED), "{status}: {out}");
    }
}

// --- a masked exit code ---

#[test]
fn test_masked_exit_code_is_named() {
    let cases = [
        ("pnpm check; echo", "; echo"),
        ("pnpm check; echo done", "; echo"),
        ("pnpm check || true", "|| true"),
        ("pnpm check; true", "; true"),
        ("pnpm check 2>&1 | tail -20", "| tail"),
        ("pnpm check | tee check.log", "| tee"),
        ("pnpm check | head -5", "| head"),
        ("cd web && pnpm check 2>&1 | tail", "| tail"),
    ];
    for (cmd, mask) in cases {
        let mut t = Transcript::new();
        t.edit();
        t.bash(cmd);
        let out = t.freshness();
        let want = format!("the last check's exit code is hidden ({mask}): read its log or run it again without it");
        assert!(tagged(&out, "ME", &want), "{cmd}: {out}");
        assert!(!out.contains(PASSED), "{cmd}: {out}");
    }
}

#[test]
fn test_masked_background_check_with_exit_0_notice_is_hidden() {
    let mut t = Transcript::new();
    t.edit();
    let tid = t.bash_bg("pnpm check; echo", "bg1");
    t.notice(&tid, "completed", "Background command \"pnpm check; echo\" completed (exit code 0)", "bg1");
    let out = t.freshness();
    assert!(out.contains("exit code is hidden (; echo)"), "{out}");
    assert!(!out.contains(PASSED), "{out}");
}

// --- a Bash in-place edit counts as an edit ---

#[test]
fn test_bash_in_place_edit_counts() {
    let cmds = [
        "sed -i 's/a/b/' app.py",
        "sed -i.bak 's/a/b/' app.py",
        "sed -E -i 's/a/b/' app.py",
        "sed --in-place 's/a/b/' app.py",
        "perl -pi -e 's/a/b/' app.py",
        "perl -i -pe 's/a/b/' app.py",
        "perl -0pi -e 's/a/b/' app.py",
        "perl -pi.bak -e 's/a/b/' app.py",
    ];
    for cmd in cmds {
        let mut t = Transcript::new();
        t.edit();
        t.bash("pnpm check");
        t.bash(cmd);
        let out = t.freshness();
        assert!(out.contains(STALE), "{cmd}: {out}");
        assert!(!out.contains(PASSED), "{cmd}: {out}");
    }
}

#[test]
fn test_in_place_edit_without_any_check() {
    let mut t = Transcript::new();
    t.bash("sed -i 's/a/b/' app.py");
    let out = t.freshness();
    assert!(out.contains(NO_CHECK), "{out}");
}

// --- the 2026-10-14 session ---

#[test]
fn test_replay_2026_10_14() {
    let mut t = Transcript::new();
    t.edit();
    t.bash("sed -i 's/old/new/' app.py");
    let first = t.bash_bg("pnpm check; echo", "bg1");
    t.notice(&first, "completed", "Background command \"pnpm check; echo\" completed (exit code 0)", "bg1");
    t.bash_bg("pnpm check", "bg2"); // no notice yet
    let out = t.freshness();
    assert!(out.contains(RUNNING), "{out}");
    assert!(!out.contains(PASSED), "{out}");
}

#[test]
fn test_background_check_completed_exit_0_passed() {
    let mut t = Transcript::new();
    t.edit();
    let tid = t.bash_bg("pnpm check", "bg1");
    t.notice(&tid, "completed", "Background command \"pnpm check\" completed (exit code 0)", "bg1");
    assert!(t.freshness().contains(PASSED));
}

#[test]
fn test_bash_reads_are_not_edits() {
    let cmds = [
        "sed -n 1,5p app.py",
        "grep -i todo app.py",
        "sed -n 1p app.py | grep -i x",
        "perl -MList::Util -e 'print 1'",
        "perl -ne 'print if /x/' app.py",
    ];
    for cmd in cmds {
        let mut t = Transcript::new();
        t.edit();
        t.bash("pnpm check");
        t.bash(cmd);
        assert!(t.freshness().contains(PASSED), "{cmd}");
    }
}

#[test]
fn test_in_place_edit_outside_the_repo_is_not_an_edit() {
    let mut t = Transcript::new();
    let other = t.root.parent().unwrap().join("elsewhere");
    t.edit();
    t.bash("pnpm check");
    t.bash_in("sed -i 's/a/b/' notes.txt", &other);
    assert!(t.freshness().contains(PASSED));
}

// ---------------------------------------------------------------------------
// test_harness_stats.py
// ---------------------------------------------------------------------------

fn usage_rec(ts: &str, ctx: i64, mid: &str, blocks: Option<Value>) -> Value {
    json!({"type": "assistant", "timestamp": ts, "message": {
        "id": mid, "model": "claude-sonnet-5-5",
        "content": blocks.unwrap_or_else(|| json!([{"type": "text", "text": "ok"}])),
        "usage": {"input_tokens": 10, "cache_read_input_tokens": ctx - 10, "cache_creation_input_tokens": 0,
                  "output_tokens": 5}}})
}

fn agent_use(tid: &str, kind: Option<&str>) -> Value {
    let mut inp = json!({"model": "sonnet", "effort": "high", "prompt": "x"});
    if let Some(k) = kind {
        inp["subagent_type"] = json!(k);
    }
    json!({"type": "tool_use", "id": tid, "name": "Agent", "input": inp})
}

/// One main session in `<home>/.claude/projects/-tmp-proj` with its subagent transcripts and meta files.
fn session(home: &Path, sid: &str, day: &str, spawns: &[(Option<&str>, i64)]) {
    let folder = home.join(".claude/projects/-tmp-proj");
    fs::create_dir_all(folder.join(sid).join("subagents")).unwrap();
    let t = format!("2026-10-{day}T10:00:00Z");
    let blocks: Vec<Value> = spawns.iter().enumerate().map(|(i, (k, _))| agent_use(&format!("a{i}"), *k)).collect();
    let recs = [
        json!({"type": "user", "timestamp": t, "message": {"role": "user", "content": "go"}}),
        usage_rec(&t, 20000, &format!("{sid}m0"), Some(Value::Array(blocks))),
    ];
    let body: String = recs.iter().map(|r| format!("{r}\n")).collect();
    fs::write(folder.join(format!("{sid}.jsonl")), body).unwrap();
    for (i, (kind, ctx)) in spawns.iter().enumerate() {
        let base = folder.join(sid).join("subagents").join(format!("agent-{sid}{i}"));
        fs::write(format!("{}.jsonl", base.display()), format!("{}\n", usage_rec(&t, *ctx, &format!("{sid}-s{i}"), None))).unwrap();
        let meta = json!({"agentType": kind.unwrap_or("general-purpose"), "model": "sonnet"});
        fs::write(format!("{}.meta.json", base.display()), meta.to_string()).unwrap();
    }
}

fn stats(home: &Path, args: &[&str]) -> String {
    let h = home.to_string_lossy().into_owned();
    let (code, out, err) = run_bin(STATS, args, "", &[("HOME", &h)], None);
    assert_eq!(code, 0, "{args:?}: {err}");
    out
}

#[test]
fn test_spawns_and_agents_table() {
    let home = temp_home("stats");
    session(&home, "s1", "10", &[(None, 60000), (Some("general-purpose"), 64000), (Some("researcher"), 15000)]);
    session(&home, "s2", "16", &[(Some("worker"), 14000), (Some("worker"), 16000)]);
    let rows_path = home.join("rows.json");
    let rows_arg = rows_path.to_string_lossy().into_owned();
    stats(&home, &["collect", "--since", "2026-10-01", "--out", &rows_arg]);
    let rows: Vec<Value> = serde_json::from_str(&fs::read_to_string(&rows_path).unwrap()).unwrap();
    let s1 = rows.iter().find(|r| r["sid"] == "s1").expect("row s1");
    let mut spawns: Vec<String> = s1["sub"]["spawns"].as_array().unwrap().iter().map(|v| v.to_string()).collect();
    spawns.sort();
    assert_eq!(spawns, [r#"["general-purpose",60000]"#, r#"["general-purpose",64000]"#, r#"["researcher",15000]"#]);
    let types: Vec<&str> = s1["agent_calls"].as_array().unwrap().iter().map(|c| c["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["general-purpose", "general-purpose", "researcher"]);

    let cut = "2026-10-15T00:00:00+00:00";
    let out = stats(&home, &["agents", "--in", &rows_arg, "--cut", cut]);
    assert!(out.contains("| general-purpose | 2 (67%) | 62 | 0 (0%) | - |"), "{out}");
    assert!(out.contains("| worker | 0 (0%) | - | 2 (100%) | 15 |"), "{out}");
    assert!(out.contains("Sessions: P0 1, P1 1"), "{out}");
    assert!(out.contains("Early (under 20 sessions): P0, P1."), "{out}");
    let rep = stats(&home, &["report", "--in", &rows_arg, "--cut", cut]);
    assert!(rep.contains("| general-purpose spawns % | 67 | 0 |"), "{rep}");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn test_scan_survives_null_token_fields() {
    let home = temp_home("stats-null");
    let mut rec = usage_rec("2026-10-10T10:00:00Z", 1000, "m1", None);
    for k in ["input_tokens", "cache_creation_input_tokens", "output_tokens"] {
        rec["message"]["usage"][k] = Value::Null;
    }
    let folder = home.join(".claude/projects/-tmp-proj");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("s.jsonl"), format!("{rec}\n")).unwrap();
    let rows_path = home.join("rows.json");
    let rows_arg = rows_path.to_string_lossy().into_owned();
    stats(&home, &["collect", "--since", "2026-10-01", "--out", &rows_arg]);
    let rows: Vec<Value> = serde_json::from_str(&fs::read_to_string(&rows_path).unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r["stats"]["calls"], 1);
    assert_eq!(r["stats"]["out"], 0);
    assert_eq!(r["stats"]["fresh_in"], 0);
    assert_eq!(r["stats"]["cache_read"], 990);
    assert_eq!(r["peak_ctx"], 990);
    let _ = fs::remove_dir_all(&home);
}
