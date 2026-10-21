//! Every test runs the `guard_cursor` binary (`CARGO_BIN_EXE_guard_cursor`). The Cursor.app chat
//! store is a SQLite file read through the `sqlite3` command line tool. macOS always has it, so
//! there a missing tool fails the tests; elsewhere they skip (the hook itself reads 0 there).
mod common;

use common::run_bin;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

type R = (i32, String, String);

const HOOK: &str = env!("CARGO_BIN_EXE_guard_cursor");
// The limits of src/bin/guard_cursor.rs (the Python hook borrowed them from guard.py).
const NUDGE_CONTEXT: i64 = 300_000;
const SPAWN_CONTEXT: i64 = 350_000;
const NUDGE_EVERY: usize = 5;
const MAX_STATE_FILES: usize = 200;
const MAX_STATE_AGE: u64 = 7 * 24 * 3600;

fn assistant(total: i64) -> Value {
    json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "ok"}],
        "usage": {"input_tokens": 10, "cache_creation_input_tokens": total - 10, "output_tokens": 5}}})
}

fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn have_sqlite() -> bool {
    Command::new("sqlite3").arg("--version").output().is_ok_and(|o| o.status.success())
}

struct T {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    app_db: String,
}

impl T {
    fn new() -> T {
        let tmp = tempfile::Builder::new().prefix("cursor-guard-test-").tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let app_db = dir.join("no-app.vscdb").to_string_lossy().into_owned(); // never the real app store
        T { _tmp: tmp, dir, app_db }
    }

    /// A fake Cursor.app state.vscdb: {conversation_id: composerData value}. False when there is no
    /// `sqlite3` tool to build it with (the caller returns early). Never false on macOS.
    fn app_store(&mut self, rows: &[(&str, &str)]) -> bool {
        if !have_sqlite() {
            if cfg!(target_os = "macos") {
                panic!("macOS ships sqlite3; these tests must not skip here");
            }
            eprintln!("skipped: no sqlite3 command line tool");
            return false;
        }
        self.app_db = self.dir.join("state.vscdb").to_string_lossy().into_owned();
        let mut sql = String::from(
            "CREATE TABLE IF NOT EXISTS cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);\n",
        );
        for (conv, value) in rows {
            sql.push_str(&format!(
                "INSERT INTO cursorDiskKV VALUES ('composerData:{}', '{}');\n",
                conv,
                value.replace('\'', "''")
            ));
        }
        let mut child = Command::new("sqlite3")
            .arg(&self.app_db)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(sql.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        true
    }

    fn run_hook_raw(&self, mode: &str, stdin: &str) -> R {
        let d = self.dir.to_string_lossy().into_owned();
        run_bin(HOOK, &[mode], stdin, &[("TMPDIR", &d), ("CURSOR_GUARD_APP_DB", &self.app_db)], None)
    }

    fn run_hook(&self, mode: &str, data: &Value) -> R {
        self.run_hook_raw(mode, &data.to_string())
    }

    fn transcript(&self, total: i64, name: &str) -> String {
        let path = self.dir.join(name);
        let text = format!(
            "{}\n{}\n",
            json!({"type": "user", "message": {"role": "user", "content": "hi"}}),
            assistant(total)
        );
        fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Record a context size the way Cursor would report one (afterAgentResponse).
    fn seed(&self, tokens: i64, conv: &str) -> R {
        self.run_hook(
            "response",
            &json!({"hook_event_name": "afterAgentResponse", "conversation_id": conv,
            "text": "ok", "context_tokens": tokens}),
        )
    }

    fn compact(&self, conv: &str) -> R {
        self.run_hook(
            "compact",
            &json!({"hook_event_name": "preCompact", "conversation_id": conv,
            "context_tokens": 900_000, "context_window_size": 1_000_000, "context_usage_percent": 90}),
        )
    }

    fn spawn(&self, conv: &str, parent: Option<&str>, extra: Value) -> R {
        let mut data = json!({"hook_event_name": "subagentStart", "conversation_id": conv, "subagent_type": "general"});
        if let Some(p) = parent {
            data["parent_conversation_id"] = json!(p);
        }
        self.run_hook("subagent", &merge(data, extra))
    }

    fn spawn_c1(&self) -> R {
        self.spawn("c1", None, json!({}))
    }

    fn prompt_x(&self, conv: &str, extra: Value) -> Value {
        let data =
            merge(json!({"hook_event_name": "beforeSubmitPrompt", "conversation_id": conv, "prompt": "go"}), extra);
        let (rc, out, err) = self.run_hook("prompt", &data);
        assert_eq!(rc, 0, "{out} {err}");
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out.get("continue"), Some(&json!(true)));
        out
    }

    fn prompt(&self, conv: &str) -> Value {
        self.prompt_x(conv, json!({}))
    }
}

fn assert_silent(res: &R) {
    assert_eq!(res, &(0, String::new(), String::new()), "{res:?}");
}

fn assert_deny(res: &R) {
    let (rc, out, err) = res;
    assert_eq!(*rc, 2, "{res:?}");
    let body: Value = serde_json::from_str(out).unwrap();
    assert_eq!(body["permission"], "deny");
    let msg = body["user_message"].as_str().unwrap();
    assert!(msg.contains("/ps:handoff"), "{msg}");
    assert_eq!(err.trim(), msg);
}

fn has_user_message(v: &Value) -> bool {
    v.get("user_message").is_some()
}

// --- limits come from guard.py ---

// --- subagent ---

#[test]
fn test_spawn_context_limit() {
    let t = T::new();
    assert_silent(&t.spawn_c1()); // nothing known
    t.seed(SPAWN_CONTEXT, "c1");
    assert_silent(&t.spawn_c1());
    t.seed(SPAWN_CONTEXT + 1, "c1");
    assert_deny(&t.spawn_c1());
}

#[test]
fn test_subagent_uses_parent_conversation() {
    let t = T::new();
    t.seed(600_000, "parent");
    assert_deny(&t.spawn("child", Some("parent"), json!({})));
    assert_silent(&t.spawn("other", None, json!({})));
}

#[test]
fn test_compact_clears_record() {
    let t = T::new();
    t.seed(600_000, "c1");
    assert_deny(&t.spawn_c1());
    assert_silent(&t.compact("c1")); // pre-compaction size is stale afterwards
    assert_silent(&t.spawn_c1());
}

#[test]
fn test_transcript_usage_wins_over_record() {
    let t = T::new();
    t.seed(600_000, "c1");
    assert_silent(&t.spawn("c1", None, json!({"transcript_path": t.transcript(100_000, "t.jsonl")})));
    assert_deny(&t.spawn("c1", None, json!({"transcript_path": t.transcript(550_000, "big.jsonl")})));
    // unreadable or empty transcript falls back to the record
    let missing = t.dir.join("missing.jsonl").to_string_lossy().into_owned();
    assert_deny(&t.spawn("c1", None, json!({"transcript_path": missing})));
}

// --- Cursor.app chat store ---

#[test]
fn test_app_store_size_denies_and_nudges() {
    let mut t = T::new();
    let row = json!({"contextTokensUsed": 600_000, "contextTokenLimit": 1_000_000}).to_string();
    if !t.app_store(&[("c1", &row)]) {
        return;
    }
    assert_deny(&t.spawn_c1());
    let (rc, out, _) = t.run_hook("prompt", &json!({"hook_event_name": "beforeSubmitPrompt", "conversation_id": "c1"}));
    assert_eq!(rc, 0);
    let body: Value = serde_json::from_str(&out).unwrap();
    let um = body["user_message"].as_str().unwrap();
    assert!(um.contains("600K"), "{um}");
    let ctx = body["additional_context"].as_str().unwrap();
    assert!(ctx.starts_with("Start your reply by telling the user"), "{ctx}");
    assert!(ctx.contains(um), "{ctx}");
}

#[test]
fn test_app_store_small_size_wins_over_stale_record() {
    let mut t = T::new();
    t.seed(600_000, "c1");
    if !t.app_store(&[("c1", &json!({"contextTokensUsed": 32_539}).to_string())]) {
        return;
    }
    assert_silent(&t.spawn_c1());
}

#[test]
fn test_app_store_breakdown_fallback_and_parent_conversation() {
    let mut t = T::new();
    let row = json!({"promptTokenBreakdown": {"totalUsedTokens": 700_000}}).to_string();
    if !t.app_store(&[("parent", &row)]) {
        return;
    }
    assert_deny(&t.spawn(
        "child",
        Some("parent"),
        json!({"conversation_id": "child", "parent_conversation_id": "parent"}),
    ));
}

#[test]
fn test_app_store_bad_rows_and_missing_conversation_fall_back() {
    let mut t = T::new();
    let c2 = json!([1, 2]).to_string();
    let c3 = json!({"contextTokensUsed": "x"}).to_string();
    if !t.app_store(&[("c1", "not json"), ("c2", &c2), ("c3", &c3)]) {
        return;
    }
    for conv in ["c1", "c2", "c3", "c4"] {
        assert_silent(&t.spawn("c1", None, json!({"conversation_id": conv})));
    }
    t.seed(600_000, "c4");
    assert_deny(&t.spawn("c1", None, json!({"conversation_id": "c4"})));
}

#[test]
fn test_app_store_not_a_database_fails_open() {
    let mut t = T::new();
    t.app_db = t.dir.join("state.vscdb").to_string_lossy().into_owned();
    fs::write(&t.app_db, "garbage").unwrap();
    assert_silent(&t.spawn_c1());
}

// --- response ---

#[test]
fn test_response_usage_recorded() {
    let t = T::new();
    let payloads = [
        json!({"usage": {"context_tokens": 600_000}}),
        json!({"context_tokens": 600_000}),
        json!({"tokens": {"contextTokens": 510_000}}),
    ];
    for (i, payload) in payloads.into_iter().enumerate() {
        let conv = format!("r{i}");
        assert_silent(&t.spawn(&conv, None, json!({})));
        let data = merge(payload, json!({"conversation_id": conv, "text": "done"}));
        assert_silent(&t.run_hook("response", &data));
        assert_deny(&t.spawn(&conv, None, json!({})));
    }
}

#[test]
fn test_pretooluse_task_gated_other_tools_ignored() {
    let t = T::new();
    t.seed(600_000, "c1");
    let task = json!({"hook_event_name": "preToolUse", "conversation_id": "c1", "tool_name": "Task",
        "tool_input": {"prompt": "count"}});
    assert_deny(&t.run_hook("subagent", &task));
    assert_silent(&t.run_hook("subagent", &merge(task, json!({"tool_name": "Shell"}))));
}

#[test]
fn test_summed_input_tokens_ignored() {
    // may be per-turn totals in Cursor; must not deny spawns
    let t = T::new();
    let data = json!({"conversation_id": "c1", "usage": {"input_tokens": 400_000, "cache_read_input_tokens": 200_000}});
    assert_silent(&t.run_hook("response", &data));
    assert_silent(&t.spawn_c1());
}

#[test]
fn test_response_without_usage_is_noop() {
    let t = T::new();
    t.seed(600_000, "c1");
    assert_silent(&t.run_hook("response", &json!({"conversation_id": "c1", "text": "done"})));
    assert_deny(&t.spawn_c1()); // record untouched
}

// --- prompt ---

#[test]
fn test_prompt_below_threshold() {
    let t = T::new();
    assert!(!has_user_message(&t.prompt("c1")));
    t.seed(NUDGE_CONTEXT - 1, "c1");
    assert!(!has_user_message(&t.prompt("c1")));
    t.seed(NUDGE_CONTEXT, "m");
    assert!(has_user_message(&t.prompt("m")));
}

#[test]
fn test_nudge_cadence() {
    let t = T::new();
    t.seed(350_000, "c1");
    assert!(t.prompt("c1")["user_message"].as_str().unwrap().contains("/ps:handoff")); // 1st
    for _ in 0..NUDGE_EVERY - 1 {
        assert!(!has_user_message(&t.prompt("c1")));
    }
    assert!(has_user_message(&t.prompt("c1"))); // 6th
    assert!(!has_user_message(&t.prompt("c1")));
}

#[test]
fn test_nudge_resets_below_threshold_and_conversations_independent() {
    let t = T::new();
    t.seed(350_000, "a");
    assert!(has_user_message(&t.prompt("a")));
    assert!(!has_user_message(&t.prompt("b")));
    t.seed(100_000, "a");
    assert!(!has_user_message(&t.prompt("a")));
    t.seed(350_000, "a");
    assert!(has_user_message(&t.prompt("a"))); // counter restarted
}

// --- fail-open ---

#[test]
fn test_fail_open() {
    let t = T::new();
    assert_silent(&t.run_hook_raw("prompt", "not json"));
    assert_silent(&t.run_hook_raw("subagent", "[1, 2]"));
    assert_silent(&t.run_hook("bogus", &json!({"conversation_id": "c1"})));
    assert_silent(&t.run_hook_raw("", ""));
    assert_silent(&t.run_hook("compact", &json!({}))); // missing fields
    assert_silent(&t.run_hook("response", &json!({})));
    let spawn_without_conv =
        json!({"hook_event_name": "subagentStart", "conversation_id": null, "subagent_type": "general"});
    assert_silent(&t.run_hook("subagent", &spawn_without_conv));
    assert_silent(&t.run_hook("compact", &json!({"conversation_id": "c1", "context_tokens": "lots"})));
}

// --- state ---

#[test]
fn test_state_isolated_in_tmpdir() {
    let t = T::new();
    t.seed(600_000, "x/../y");
    let state = t.dir.join("cursor-guard");
    let files: Vec<String> =
        fs::read_dir(&state).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, vec!["x_.._y.json".to_string()]);
    let st: Value = serde_json::from_str(&fs::read_to_string(state.join(&files[0])).unwrap()).unwrap();
    assert_eq!(st["ctx"], 600_000);
    assert!(st.get("updated").is_some(), "{st}");
}

#[test]
fn test_state_pruned() {
    let t = T::new();
    let d = t.dir.join("cursor-guard");
    fs::create_dir_all(&d).unwrap();
    let old = SystemTime::now() - Duration::from_secs(MAX_STATE_AGE + 60);
    for i in 0..MAX_STATE_FILES + 20 {
        let p = d.join(format!("k{i}.json"));
        fs::write(&p, "{}").unwrap();
        if i < 10 {
            fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(old).unwrap();
        }
    }
    t.seed(1000, "new");
    let names: Vec<String> =
        fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.len() <= MAX_STATE_FILES, "{}", names.len());
    assert!(names.contains(&"new.json".to_string()));
    assert!(!names.contains(&"k0.json".to_string()));
}

// --- tool mode: the shared checks G14-G17 (Task 13) ---
//
// Cursor hook docs: https://cursor.com/docs/hooks (the `.md` of that page). `preToolUse` input holds
// `tool_name` (`Shell`, `Read`, `Grep`, `Write`, `Task`, `MCP:<tool>`), `tool_input`, `cwd`; its answer
// is `{"permission": "allow"|"deny", "user_message", "agent_message"}` and `ask` is not enforced there.
// `beforeShellExecution` (`command`, `cwd`) and `beforeMCPExecution` (`tool_name`, `tool_input` as a
// JSON string, `mcp_server_name`) do enforce `ask`. Exit code 2 also blocks.

impl T {
    /// A tool call in this test's world: HOME and TMPDIR are the temp dir.
    fn tool(&self, data: &Value, env: &[(&str, &str)]) -> R {
        let d = self.dir.to_string_lossy().into_owned();
        let mut e: Vec<(&str, &str)> = vec![("TMPDIR", &d), ("HOME", &d), ("CURSOR_GUARD_APP_DB", &self.app_db)];
        e.extend_from_slice(env);
        run_bin(HOOK, &["tool"], &data.to_string(), &e, None)
    }

    fn settings(&self, text: &str) {
        let p = self.dir.join(".config/dotfiles/settings");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn shell(&self, cmd: &str, env: &[(&str, &str)]) -> R {
        let cwd = self.dir.to_string_lossy().into_owned();
        self.tool(
            &json!({"hook_event_name": "preToolUse", "tool_name": "Shell",
            "tool_input": {"command": cmd, "working_directory": cwd}, "cwd": cwd, "conversation_id": "c1"}),
            env,
        )
    }
}

fn answer(res: &R) -> Value {
    serde_json::from_str(&res.1).unwrap_or_else(|e| panic!("not JSON ({e}): {res:?}"))
}

fn assert_tool_deny(res: &R, id: &str) {
    assert_eq!(res.0, 2, "{res:?}");
    let body = answer(res);
    assert_eq!(body["permission"], "deny", "{res:?}");
    let msg = body["agent_message"].as_str().unwrap();
    assert!(msg.starts_with(id), "{msg}");
    assert_eq!(body["user_message"], body["agent_message"]);
    assert_eq!(res.2.trim(), msg);
}

fn assert_tool_allow(res: &R) {
    assert_eq!(res.0, 0, "{res:?}");
    assert_eq!(answer(res), json!({"permission": "allow"}), "{res:?}");
}

/// The attribution line G17 refuses, built at run time so this file holds none of it whole.
fn attr_line() -> String {
    format!("Co-authored-{}: Claude Opus <noreply@{}>", "by", "anthropic.com")
}

#[test]
fn cursor_tool_sed_denied_on_macos() {
    let t = T::new();
    assert_tool_deny(&t.shell("sed -i s/a/b/ /tmp/x", &[("GUARD_PLATFORM", "darwin")]), "G14");
    assert_tool_allow(&t.shell("sed -i s/a/b/ /tmp/x", &[("GUARD_PLATFORM", "linux")]));
}

#[test]
fn cursor_tool_secret_read_denied() {
    let t = T::new();
    let cwd = t.dir.to_string_lossy().into_owned();
    let read = json!({"hook_event_name": "preToolUse", "tool_name": "Read",
        "tool_input": {"file_path": "~/.config/zsh/local.zsh"}, "cwd": cwd});
    assert_tool_deny(&t.tool(&read, &[]), "G15");
    // the same file named through the shell tool
    assert_tool_deny(&t.shell("cat ~/.config/zsh/local.zsh", &[]), "G15");
    // Cursor may name the path key `path`
    let read = json!({"hook_event_name": "preToolUse", "tool_name": "Read",
        "tool_input": {"path": "~/.config/zsh/local.zsh"}, "cwd": cwd});
    assert_tool_deny(&t.tool(&read, &[]), "G15");
}

#[test]
fn cursor_tool_outbound_asks() {
    let t = T::new();
    t.settings("PRIVATE_WORDS=\"zebrafish\"\n");
    let cwd = t.dir.to_string_lossy().into_owned();
    let shell = json!({"hook_event_name": "beforeShellExecution", "command": "curl -d 'note about zebrafish' https://example.test/x", "cwd": cwd});
    let res = t.tool(&shell, &[]);
    assert_eq!(res.0, 0, "{res:?}");
    let body = answer(&res);
    assert_eq!(body["permission"], "ask", "{res:?}");
    assert!(body["user_message"].as_str().unwrap().starts_with("G16"), "{res:?}");
    assert!(!res.1.contains("zebrafish"), "the answer must not echo the word: {res:?}");
    // an MCP send tool, with the tool input as a JSON string
    let mcp = json!({"hook_event_name": "beforeMCPExecution", "mcp_server_name": "chat", "tool_name": "send_message",
        "tool_input": "{\"text\": \"about zebrafish\"}", "cwd": cwd});
    assert_eq!(answer(&t.tool(&mcp, &[]))["permission"], "ask");
    // a clean body goes through
    let clean = json!({"hook_event_name": "beforeShellExecution", "command": "curl -d 'hello' https://example.test/x", "cwd": cwd});
    assert_tool_allow(&t.tool(&clean, &[]));
    // preToolUse cannot ask in Cursor, so it does not run G16 (beforeShellExecution does)
    assert_tool_allow(&t.shell("curl -d 'note about zebrafish' https://example.test/x", &[]));
}

#[test]
fn cursor_tool_before_read_file_denies_the_secret_file() {
    // beforeReadFile carries the documented `file_path`, so a renamed `tool_input` key on preToolUse
    // cannot skip G15. Its answer holds only `permission` and `user_message` (the documented schema).
    let t = T::new();
    let read = json!({"hook_event_name": "beforeReadFile", "file_path": "~/.config/zsh/local.zsh", "content": "x", "attachments": []});
    let res = t.tool(&read, &[]);
    assert_eq!(res.0, 2, "{res:?}");
    let body = answer(&res);
    assert_eq!(body["permission"], "deny", "{res:?}");
    assert!(body["user_message"].as_str().unwrap().starts_with("G15"), "{res:?}");
    assert!(body.get("agent_message").is_none(), "{res:?}");
    assert_tool_allow(
        &t.tool(&json!({"hook_event_name": "beforeReadFile", "file_path": "/etc/hosts", "content": ""}), &[]),
    );
    // GUARD_OFF turns it off like the other G15 entries
    assert_tool_allow(&t.tool(&read, &[("GUARD_OFF", "G15")]));
}

#[test]
fn cursor_tool_attribution_denied() {
    let t = T::new();
    assert_tool_deny(&t.shell(&format!("git commit -m \"x\" -m \"{}\"", attr_line()), &[]), "G17");
    let res = t.shell(&format!("git commit -m \"{}\"", attr_line()), &[]);
    assert!(!res.1.contains("noreply") && !res.2.contains("noreply"), "the answer must not echo the line: {res:?}");
}

#[test]
fn cursor_tool_clean_allows() {
    let t = T::new();
    assert_tool_allow(&t.shell("ls -la", &[]));
    assert_tool_allow(&t.shell("git commit -m \"fix the thing\"", &[]));
    let cwd = t.dir.to_string_lossy().into_owned();
    let read = json!({"hook_event_name": "preToolUse", "tool_name": "Read", "tool_input": {"file_path": "/etc/hosts"}, "cwd": cwd});
    assert_tool_allow(&t.tool(&read, &[]));
    // a tool the checks do not know
    let other = json!({"hook_event_name": "preToolUse", "tool_name": "Write", "tool_input": {"file_path": "/tmp/x"}, "cwd": cwd});
    assert_tool_allow(&t.tool(&other, &[]));
}

#[test]
fn cursor_tool_fails_open_with_an_allow_answer() {
    let t = T::new();
    let d = t.dir.to_string_lossy().into_owned();
    for bad in ["", "not json", "[1, 2]", "{}"] {
        let res = run_bin(HOOK, &["tool"], bad, &[("TMPDIR", &d), ("HOME", &d)], None);
        assert_tool_allow(&res);
    }
}

#[test]
fn cursor_tool_uses_the_same_gate_as_the_guard() {
    let t = T::new();
    let mac = ("GUARD_PLATFORM", "darwin");
    // GUARD_OFF turns a check off
    assert_tool_allow(&t.shell("sed -i s/a/b/ x", &[mac, ("GUARD_OFF", "g14")]));
    t.settings("GUARD_OFF=\"G14\"\n");
    assert_tool_allow(&t.shell("sed -i s/a/b/ x", &[mac]));
    // the preset of G17 off: it allows; the preset on: it denies
    t.settings("PRESETS=\"workflow\"\n");
    assert_tool_allow(&t.shell(&format!("git commit -m \"{}\"", attr_line()), &[]));
    t.settings("PRESETS=\"no-ai-attribution\"\n");
    assert_tool_deny(&t.shell(&format!("git commit -m \"{}\"", attr_line()), &[]), "G17");
    // a core check ignores the presets
    assert_tool_deny(&t.shell("sed -i s/a/b/ x", &[mac, ("GUARD_OFF", "")]), "G14");
    // shadow mode logs and allows
    let cache = t.dir.join("cache").to_string_lossy().into_owned();
    let res =
        t.shell("sed -i s/a/b/ x", &[mac, ("GUARD_OFF", ""), ("GUARD_SHADOW", "G14"), ("XDG_CACHE_HOME", &cache)]);
    assert_tool_allow(&res);
    let log = fs::read_to_string(t.dir.join("cache/dots-guard-shadow.log")).unwrap();
    assert!(log.contains(" G14 deny "), "{log}");
}
