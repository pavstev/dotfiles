//! Port of claude/hooks/test_guard.py: every test runs the `guard` binary (`CARGO_BIN_EXE_guard`).
//! The few that called Python functions directly live in `src/bin/guard.rs` (`mod tests`).
//! `tests/PORTED.md` maps each Python test to its Rust twin; `ported_map_complete` checks the map.
mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Expected row count per Python file. The first three files are ported by this suite and
/// `pace.rs` / `guard_cursor.rs`; `scripts/test_agent_config.py` by `agent_config.rs` and
/// `src/agent_config_tests.rs`; the four skill-script files by the unit tests of the skill bins and
/// `tests/skill_bins.rs`. Every file listed must have exactly this many rows.
const REQUIRED: [(&str, usize); 8] = [
    ("claude/hooks/test_guard.py", 130),
    ("claude/hooks/test_pace.py", 64),
    ("cursor/hooks/test_guard_cursor.py", 23),
    ("scripts/test_agent_config.py", 199),
    ("claude/skills/ps_spawn/scripts/test_spawn.py", 11),
    ("claude/skills/ps_whatsleft/scripts/test_route.py", 18),
    ("claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py", 14),
    ("claude/skills/ps_harness-optimize/scripts/test_harness_stats.py", 2),
];

use common::run_bin;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

type R = (i32, String, String);

const GUARD: &str = env!("CARGO_BIN_EXE_guard");
const BRIEF: &str = "Build it. Deadline: 25 min. Hand back at most 4,000 chars; write the rest to a file.";
const WEEK: i64 = 7 * 86400;
/// The hook runs with a HOME that is not under the temp dir: `~` must resolve outside the scratch roots.
const SCRATCH_HOME: &str = "/home/tester";

static SID: AtomicUsize = AtomicUsize::new(0);

fn next_sid() -> usize {
    SID.fetch_add(1, Ordering::SeqCst) + 1
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

/// Python's `dict(base, **extra)`: keys of `extra` replace those of `base`.
fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn assistant_b(total: i64, blocks: Value) -> Value {
    json!({"type": "assistant", "message": {"role": "assistant", "content": blocks, "usage": {
        "input_tokens": 10, "cache_creation_input_tokens": total - 10 - 500,
        "cache_read_input_tokens": 500, "output_tokens": 5}}})
}

fn assistant(total: i64) -> Value {
    assistant_b(total, json!([{"type": "text", "text": "ok"}]))
}

fn human(text: &str) -> Value {
    json!({"type": "user", "message": {"role": "user", "content": text}})
}

fn tool_result(tid: &str) -> Value {
    json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": tid, "content": "x"}]}})
}

fn ask_block(tid: &str, q: &str) -> Value {
    json!({"type": "tool_use", "id": tid, "name": "AskUserQuestion", "input": {"questions": [q]}})
}

fn user_rec(content: Value) -> Value {
    json!({"type": "user", "message": {"role": "user", "content": content}})
}

/// A stand-in for the pace module: the binary reads the statusline cache first, so a fake pace is a
/// fresh cache file whose week offset gives the wanted budget (tools/hook_test_shim.py does the same).
#[derive(Clone, Copy)]
struct Pace {
    state: &'static str,
    used: f64,
    budget: f64,
}

fn fake_pace(state: &'static str, used: f64, budget: f64) -> Pace {
    Pace { state, used, budget }
}

fn pace_default(state: &'static str) -> Pace {
    fake_pace(state, 40.0, 50.0)
}

fn write_pace_cache(cache_dir: &std::path::Path, pace: Option<Pace>) {
    let path = cache_dir.join("claude-pace.json");
    match pace {
        Some(p) if p.state != "unknown" => {
            let now = now_secs() as i64;
            let idx = ((p.budget / 100.0 * 6.0).round() as i64 - 1).clamp(0, 5);
            let ws = now - idx * 86400;
            std::fs::create_dir_all(cache_dir).unwrap();
            let body = json!({"updated": now, "seven_day": {"used_percentage": p.used, "resets_at": ws + WEEK}});
            std::fs::write(path, body.to_string()).unwrap();
        }
        _ => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn ok_agent() -> Value {
    json!({"model": "sonnet", "effort": "high", "prompt": BRIEF})
}

fn assert_deny(res: &R, text: &str) {
    assert_eq!(res.0, 2, "{res:?}");
    assert!(res.2.contains(text), "stderr lacks {text:?}: {res:?}");
    assert_eq!(res.1, "", "{res:?}");
}

fn assert_allow(res: &R) {
    assert_eq!(res, &(0, String::new(), String::new()), "{res:?}");
}

fn assert_warn(res: &R, text: &str) -> String {
    assert_eq!(res.0, 0, "{res:?}");
    assert_eq!(res.2, "", "{res:?}");
    let v: Value = serde_json::from_str(&res.1).unwrap_or_else(|e| panic!("{e}: {res:?}"));
    let out = &v["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PreToolUse");
    let ctx = out["additionalContext"].as_str().unwrap().to_string();
    assert!(ctx.contains(text), "{ctx:?} lacks {text:?}");
    ctx
}

fn json_out(res: &R) -> Value {
    serde_json::from_str(&res.1).unwrap_or_else(|e| panic!("{e}: {res:?}"))
}

/// One test's world: a temp dir that is TMPDIR, HOME and the parent of the cache.
struct G {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    ho: String,
}

impl G {
    fn new() -> G {
        let tmp = tempfile::Builder::new().prefix("guard-test-").tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        // the G6 hand-off must exist on disk: Stop and archive skip a deleted one
        let ho = dir.join("repo/.agents/handoffs/2026-10-14-x.md");
        std::fs::create_dir_all(ho.parent().unwrap()).unwrap();
        std::fs::write(&ho, "# Task").unwrap();
        G { _tmp: tmp, dir, ho: ho.to_string_lossy().into_owned() }
    }

    fn d(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }

    fn join(&self, rel: &str) -> String {
        self.dir.join(rel).to_string_lossy().into_owned()
    }

    fn write(&self, path: &str, text: &str) {
        if let Some(p) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    fn lines(&self, path: &str, recs: &[Value]) {
        let mut text = String::new();
        for r in recs {
            text.push_str(&r.to_string());
            text.push('\n');
        }
        self.write(path, &text);
    }

    fn transcript(&self, recs: &[Value]) -> String {
        self.transcript_named(recs, "t.jsonl")
    }

    fn transcript_named(&self, recs: &[Value], name: &str) -> String {
        let path = self.join(name);
        self.lines(&path, recs);
        path
    }

    /// Run the hook. HOME, TMPDIR and the cache point at the temp dir: pace finds no data there and
    /// answers "unknown". `scratch` keeps a HOME outside the temp dir (see SCRATCH_HOME).
    fn run_env(&self, mode: &str, stdin: &str, env: &[(&str, &str)]) -> R {
        let d = self.d();
        let cache = self.join("cache");
        let mut e: Vec<(&str, &str)> = vec![("TMPDIR", &d)];
        if mode == "scratch" {
            e.push(("HOME", SCRATCH_HOME));
        } else {
            e.push(("HOME", &d));
            e.push(("XDG_CACHE_HOME", &cache));
        }
        e.extend_from_slice(env);
        run_bin(GUARD, &[mode], stdin, &e, None)
    }

    fn run_raw(&self, mode: &str, raw: &str) -> R {
        self.run_env(mode, raw, &[])
    }

    fn run(&self, mode: &str, data: &Value) -> R {
        self.run_env(mode, &data.to_string(), &[])
    }

    fn run_with(&self, mode: &str, data: &Value, env: &[(&str, &str)]) -> R {
        self.run_env(mode, &data.to_string(), env)
    }

    /// The Python `inproc`: run with a fake pace (a statusline cache file).
    fn inproc(&self, mode: &str, data: &Value, pace: Option<Pace>) -> R {
        write_pace_cache(&self.dir.join("cache"), pace);
        self.run(mode, data)
    }

    fn pre(&self, tool: &str, ti: Value) -> R {
        self.pre_x(tool, ti, 0, json!({}))
    }

    fn pre_ctx(&self, tool: &str, ti: Value, ctx: i64) -> R {
        self.pre_x(tool, ti, ctx, json!({}))
    }

    fn pre_x(&self, tool: &str, mut ti: Value, ctx: i64, extra: Value) -> R {
        let path = if ctx > 0 { Some(self.transcript(&[human("hi"), assistant(ctx)])) } else { None };
        if tool == "Agent" && ti.is_object() && ti.get("subagent_type").is_none() {
            ti["subagent_type"] = json!("worker"); // not a refused type
        }
        // a fresh session per call: the concurrency cap counts per session
        let mut data = json!({"hook_event_name": "PreToolUse", "session_id": format!("s{}", next_sid()),
            "tool_name": tool, "tool_input": ti});
        if let Some(p) = path {
            data["transcript_path"] = json!(p);
        }
        self.run("pretool", &merge(data, extra))
    }

    // --- router ---
    fn route(&self, kind: &str, model: Option<&str>, effort: Option<&str>) -> (Value, Option<Value>) {
        self.route_full(kind, model, effort, "Do it", BRIEF, None, json!({}))
    }

    #[allow(clippy::too_many_arguments)]
    fn route_full(
        &self,
        kind: &str,
        model: Option<&str>,
        effort: Option<&str>,
        desc: &str,
        prompt: &str,
        pace: Option<Pace>,
        more: Value,
    ) -> (Value, Option<Value>) {
        let mut ti = json!({"subagent_type": kind, "description": desc, "prompt": prompt});
        if let Some(m) = model {
            ti["model"] = json!(m);
        }
        if let Some(e) = effort {
            ti["effort"] = json!(e);
        }
        let ti = merge(ti, more);
        let data = json!({"hook_event_name": "PreToolUse", "session_id": format!("r{}", next_sid()),
            "tool_name": "Agent", "tool_input": ti});
        let res = if pace.is_some() { self.inproc("pretool", &data, pace) } else { self.run("pretool", &data) };
        assert_eq!(res.0, 0, "{res:?}");
        assert_eq!(res.2, "", "{res:?}");
        let out = if res.1.is_empty() { None } else { Some(json_out(&res)["hookSpecificOutput"].clone()) };
        (ti, out)
    }

    fn spawn_session(&self, n: usize, tokens_each: i64) -> String {
        let mut recs = vec![human("hi")];
        for i in 0..n {
            recs.push(assistant_b(
                1000,
                json!([{"type": "tool_use", "id": format!("a{i}"), "name": "Agent", "input": {}}]),
            ));
            let mut res = tool_result(&format!("a{i}"));
            if tokens_each > 0 {
                res["toolUseResult"] = json!({"totalTokens": tokens_each, "agentType": "worker"});
            }
            recs.push(res);
        }
        self.transcript(&recs)
    }

    fn spawn(&self, path: &str, tid: &str) -> R {
        let data = json!({"hook_event_name": "PreToolUse", "session_id": format!("w{}", next_sid()),
            "tool_name": "Agent", "tool_use_id": tid, "transcript_path": path,
            "tool_input": {"subagent_type": "worker", "model": "sonnet", "effort": "high", "prompt": BRIEF}});
        self.run("pretool", &data)
    }

    fn browser_turn(&self, n: usize, older: usize, name: &str) -> String {
        let mut recs = vec![human("old")];
        let tu = |id: String| assistant_b(1000, json!([{"type": "tool_use", "id": id, "name": name, "input": {}}]));
        for i in 0..older {
            recs.push(tu(format!("o{i}")));
        }
        recs.push(human("check the page"));
        for i in 0..n {
            recs.push(tu(format!("b{i}")));
        }
        self.transcript(&recs)
    }

    fn browser_call(&self, path: Option<&str>, extra: Value) -> R {
        let data = json!({"hook_event_name": "PreToolUse", "session_id": "s",
            "tool_name": "mcp__Claude_Browser__navigate", "tool_input": {"url": "https://example.com"},
            "tool_use_id": "now", "transcript_path": path});
        self.run("pretool", &merge(data, extra))
    }

    fn browser(&self, n: usize) -> String {
        self.browser_turn(n, 0, "mcp__Claude_Browser__get_page_text")
    }

    // --- G3 / G4 ---
    fn prompt_run(&self, ctx: i64, sid: &str) -> R {
        let path = self.transcript_named(&[human("hi"), assistant(ctx)], &format!("{sid}.jsonl"));
        self.run(
            "prompt",
            &json!({"hook_event_name": "UserPromptSubmit", "session_id": sid, "transcript_path": path, "prompt": "go"}),
        )
    }

    fn ask(&self, records: &[Value], tool_use_id: Option<&str>, q: &str) -> R {
        let path = self.transcript(records);
        let mut data = json!({"transcript_path": path, "tool_name": "AskUserQuestion",
            "tool_input": {"questions": [q]}});
        if let Some(t) = tool_use_id {
            data["tool_use_id"] = json!(t);
        }
        self.run("pretool", &data)
    }

    // --- G5 hand-back cap ---
    /// Write a subagent transcript under <session>/subagents/ and return the main transcript path.
    fn sub(&self, prompt: Option<&str>, agent_id: &str) -> String {
        let main = self.transcript_named(&[human("go")], "main.jsonl");
        let mut recs = vec![json!({"type": "system", "subtype": "init"})];
        if let Some(p) = prompt {
            recs.push(user_rec(json!(p)));
        }
        recs.push(assistant(1000));
        self.lines(&self.join(&format!("main/subagents/agent-{agent_id}.jsonl")), &recs);
        main
    }

    fn handback_full(
        &self,
        msg: Value,
        prompt: Option<&str>,
        agent_type: Option<&str>,
        agent_id: &str,
        mode: &str,
        extra: Value,
    ) -> R {
        let mut data = json!({"session_id": "s", "agent_id": agent_id, "transcript_path": self.sub(prompt, agent_id)});
        if let Some(t) = agent_type {
            data["agent_type"] = json!(t);
        }
        data = merge(data, extra);
        if mode == "pretool" {
            data = merge(
                data,
                json!({"hook_event_name": "PreToolUse", "tool_name": "SubagentHandback", "tool_input": {"message": msg}}),
            );
        } else {
            data = merge(data, json!({"hook_event_name": "SubagentStop", "last_assistant_message": msg}));
        }
        self.run(mode, &data)
    }

    fn handback(&self, msg: &str) -> R {
        self.handback_full(json!(msg), None, None, "a1", "pretool", json!({}))
    }

    fn handback_p(&self, msg: &str, prompt: &str, agent_id: &str) -> R {
        self.handback_full(json!(msg), Some(prompt), None, agent_id, "pretool", json!({}))
    }

    fn handback_stop(&self, msg: Value, agent_id: &str, extra: Value) -> R {
        self.handback_full(msg, None, None, agent_id, "subagentstop", extra)
    }

    // --- scratch ---
    fn scratch(&self, path: &str, key: &str, extra: Value) -> R {
        let data = json!({"hook_event_name": "PreToolUse", "tool_name": "Write", "tool_input": {key: path}});
        self.run("scratch", &merge(data, extra))
    }

    // --- G6 hand-off ---
    fn write_ho(&self, tid: &str, path: Option<&str>) -> Value {
        assistant_b(
            1000,
            json!([{"type": "tool_use", "id": tid, "name": "Write",
                "input": {"file_path": path.unwrap_or(&self.ho), "content": "# Task"}}]),
        )
    }

    fn say(&self, text: &str) -> Value {
        assistant_b(1000, json!([{"type": "text", "text": text}]))
    }

    fn good_reply_text(&self) -> String {
        format!("Done. [2026-10-14-x.md](.agents/handoffs/2026-10-14-x.md)\n```\nRead {} and do it.\n```", self.ho)
    }

    fn good_reply(&self) -> Value {
        self.say(&self.good_reply_text())
    }

    fn stop_run(&self, recs: &[Value], extra: Value) -> R {
        self.stop_msg(recs, extra, &[])
    }

    fn stop_msg(&self, recs: &[Value], extra: Value, env: &[(&str, &str)]) -> R {
        let data = merge(
            json!({"hook_event_name": "Stop", "session_id": "s", "transcript_path": self.transcript(recs)}),
            extra,
        );
        self.run_with("stop", &data, env)
    }

    fn archive(&self, recs: &[Value], sid: &str) -> R {
        self.run(
            "pretool",
            &json!({"hook_event_name": "PreToolUse", "session_id": "s",
                "tool_name": "mcp__ccd_session_mgmt__archive_session", "tool_input": {"session_id": sid},
                "transcript_path": self.transcript(recs)}),
        )
    }

    // --- concurrency cap ---
    fn agent_data(&self, sid: &str, extra_ti: Value) -> Value {
        let base = json!({"subagent_type": "worker", "model": "sonnet", "effort": "high", "prompt": BRIEF});
        json!({"hook_event_name": "PreToolUse", "session_id": sid, "tool_name": "Agent",
            "tool_input": merge(base, extra_ti)})
    }

    fn agent(&self, sid: &str) -> R {
        self.run("pretool", &self.agent_data(sid, json!({})))
    }

    fn stop_data(&self, sid: &str, extra: Value) -> Value {
        merge(json!({"hook_event_name": "SubagentStop", "session_id": sid, "last_assistant_message": "done"}), extra)
    }

    fn subagent_stop(&self, sid: &str) -> R {
        self.run("subagentstop", &self.stop_data(sid, json!({})))
    }

    fn failure_data(&self, sid: &str, tid: Value, extra: Value) -> Value {
        merge(
            json!({"hook_event_name": "PostToolUseFailure", "session_id": sid, "tool_name": "Agent",
                "tool_use_id": tid, "tool_input": {}, "error": "boom"}),
            extra,
        )
    }

    fn spawn_with_id(&self, sid: &str, tid: &str) -> R {
        let mut d = self.agent_data(sid, json!({}));
        d["tool_use_id"] = json!(tid);
        self.run("pretool", &d)
    }

    fn failure(&self, sid: &str, tid: Value) -> R {
        self.run("posttoolfailure", &self.failure_data(sid, tid, json!({})))
    }

    fn state_dir(&self) -> PathBuf {
        self.dir.join("claude-guard")
    }
}

fn assert_routed(out: &Option<Value>, ti: &Value, model: &str, effort: Option<&str>) {
    let out = out.as_ref().expect("a routed call");
    assert_eq!(out["hookEventName"], "PreToolUse");
    assert_eq!(out["permissionDecision"], "allow");
    assert!(!out["additionalContext"].as_str().unwrap_or("").is_empty());
    let new = &out["updatedInput"];
    assert_eq!(new["model"], model);
    match effort {
        None => assert!(new.get("effort").is_none(), "{new}"),
        Some(e) => assert_eq!(new["effort"], e),
    }
    for (k, v) in ti.as_object().unwrap() {
        if k != "model" && k != "effort" {
            assert_eq!(&new[k], v, "field {k}");
        }
    }
}

// --- heavy agents are refused ---
#[test]
fn test_heavy_agents_denied() {
    let g = G::new();
    let base = ok_agent();
    for kind in [json!("general-purpose"), Value::Null, json!("claude")] {
        let res = g.pre("Agent", merge(base.clone(), json!({"subagent_type": kind})));
        assert_deny(&res, "`worker`");
        for name in ["`researcher`", "`Explore`", "`task-implementer`", "`Plan`"] {
            assert!(res.2.contains(name), "{name}: {res:?}");
        }
    }
    // a missing subagent_type key is the same as general-purpose
    let data = json!({"session_id": "h1", "tool_name": "Agent", "tool_input": base});
    assert_deny(&g.run("pretool", &data), "`worker`");
    for kind in ["worker", "task-implementer", "code-reviewer", "researcher"] {
        assert_allow(&g.pre("Agent", merge(base.clone(), json!({"subagent_type": kind}))));
    }
}

#[test]
fn test_router_table() {
    let g = G::new();
    let (ti, out) = g.route("Plan", Some("sonnet"), Some("high"));
    assert_routed(&out, &ti, "opus", Some("high"));
    let (ti, out) = g.route_full("code-reviewer", Some("sonnet"), Some("high"), "Final whole-branch review", BRIEF, None, json!({}));
    assert_routed(&out, &ti, "opus", Some("high"));
    let (ti, out) = g.route_full("code-reviewer", Some("sonnet"), Some("high"), "review",
        &format!("Final review of main..HEAD. {BRIEF}"), None, json!({}));
    assert_routed(&out, &ti, "opus", Some("high"));
    for kind in ["Explore", "claude-code-guide", "statusline-setup"] {
        let (ti, out) = g.route(kind, Some("sonnet"), Some("high"));
        assert_routed(&out, &ti, "haiku", None);
    }
    let (ti, out) = g.route("worker", Some("opus"), Some("max"));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    let (ti, out) = g.route("researcher", None, None);
    assert_routed(&out, &ti, "sonnet", Some("high"));
    let (ti, out) = g.route("task-implementer", Some("sonnet"), Some("xhigh"));
    assert_routed(&out, &ti, "sonnet", Some("high"));
}

#[test]
fn test_router_review_cases() {
    let g = G::new();
    // a per-task or plain review stays on sonnet; panel angles and re-reviews too
    let final_brief = format!("Final review of main..HEAD. {BRIEF}");
    let cases: [(&str, &str, String); 4] = [
        ("code-reviewer", "Review commit abc", BRIEF.to_string()),
        ("task-reviewer", "Review task 3", final_brief.clone()),
        ("code-reviewer", "panel: simplify", final_brief),
        ("code-reviewer", "Re-review blocking findings", format!("Final review. {BRIEF}")),
    ];
    for (kind, desc, prompt) in cases {
        let (ti, out) = g.route_full(kind, Some("opus"), Some("high"), desc, &prompt, None, json!({}));
        assert_routed(&out, &ti, "sonnet", Some("high"));
    }
    let (ti, out) = g.route_full("worker", Some("opus"), Some("high"), "final review of the branch", BRIEF, None, json!({}));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    // "final review" deep in an implementer brief is not a review call
    let (ti, out) = g.route_full("task-implementer", Some("opus"), Some("high"), "Do it",
        &format!("{BRIEF} A final review follows."), None, json!({}));
    assert_routed(&out, &ti, "sonnet", Some("high"));
}

#[test]
fn test_router_never_raises_cost() {
    let g = G::new();
    // a caller's cheaper model and low/medium effort stay
    let (ti, out) = g.route("worker", Some("haiku"), Some("low"));
    assert_routed(&out, &ti, "haiku", None); // haiku takes no effort
    let (_, out) = g.route("worker", Some("sonnet"), Some("low"));
    assert!(out.is_none());
    let (_, out) = g.route("worker", Some("claude-sonnet-4-5"), Some("medium"));
    assert!(out.is_none());
    let (ti, out) = g.route("task-implementer", None, Some("medium"));
    assert_routed(&out, &ti, "sonnet", Some("medium"));
    let (ti, out) = g.route("worker", Some("haiku"), Some("high"));
    assert_routed(&out, &ti, "haiku", None);
    let (ti, out) = g.route("worker", Some("opus"), Some("max"));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    let (ti, out) = g.route("worker", Some("fable"), Some("xhigh"));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    let (ti, out) = g.route("worker", Some("sonnet"), Some("max"));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    let (ti, out) = g.route("worker", Some("some-unknown-model"), Some("low"));
    assert_routed(&out, &ti, "sonnet", Some("low"));
    // lookups never go above haiku
    let (ti, out) = g.route("Explore", Some("opus"), Some("medium"));
    assert_routed(&out, &ti, "haiku", None);
}

#[test]
fn test_router_opus_rows_follow_the_table() {
    let g = G::new();
    let (ti, out) = g.route("Plan", Some("sonnet"), Some("low"));
    assert_routed(&out, &ti, "opus", Some("high"));
    let (ti, out) = g.route("Plan", Some("haiku"), None);
    assert_routed(&out, &ti, "opus", Some("high"));
    let (ti, out) = g.route_full("code-reviewer", Some("sonnet"), Some("medium"), "Final whole-branch review", BRIEF, None, json!({}));
    assert_routed(&out, &ti, "opus", Some("high"));
    let ahead = Some(fake_pace("ahead", 60.0, 50.0));
    let (ti, out) = g.route_full("Plan", Some("sonnet"), Some("low"), "Do it", BRIEF, ahead, json!({}));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    let (ti, out) = g.route_full("Plan", Some("opus"), Some("max"), "Do it", BRIEF, ahead, json!({}));
    assert_routed(&out, &ti, "sonnet", Some("high"));
}

#[test]
fn test_router_silent_when_nothing_changes() {
    let g = G::new();
    for (kind, model, effort) in [("worker", "sonnet", Some("high")), ("Plan", "opus", Some("high")), ("Explore", "haiku", None)] {
        let (_, out) = g.route(kind, Some(model), effort);
        assert!(out.is_none(), "{kind}");
    }
}

#[test]
fn test_router_skips_fork_and_subagents() {
    let g = G::new();
    let (_, out) = g.route("fork", None, None);
    assert!(out.is_none());
    let res = g.pre_x("Agent", json!({"subagent_type": "worker", "model": "opus", "effort": "max", "prompt": BRIEF}),
        0, json!({"agent_id": "a1"}));
    assert_allow(&res);
}

#[test]
fn test_router_pace_ahead_downgrades_opus() {
    let g = G::new();
    let pace = Some(fake_pace("ahead", 60.0, 50.0));
    let (ti, out) = g.route_full("Plan", Some("opus"), Some("high"), "Do it", BRIEF, pace, json!({}));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    assert!(out.unwrap()["additionalContext"].as_str().unwrap().contains("ahead"));
    let (ti, out) = g.route_full("code-reviewer", Some("opus"), Some("high"), "Final review", BRIEF, pace, json!({}));
    assert_routed(&out, &ti, "sonnet", Some("high"));
    let (ti, out) = g.route_full("Explore", Some("sonnet"), Some("high"), "Do it", BRIEF, pace, json!({}));
    assert_routed(&out, &ti, "haiku", None);
    // at ok the same Plan call is opus
    let (ti, out) = g.route_full("Plan", Some("sonnet"), Some("high"), "Do it", BRIEF, Some(pace_default("ok")), json!({}));
    assert_routed(&out, &ti, "opus", Some("high"));
}

#[test]
fn test_router_merges_with_warnings() {
    let g = G::new();
    // W4 note and the router note come out as one JSON line
    let path = g.spawn_session(15, 0);
    let data = json!({"hook_event_name": "PreToolUse", "session_id": "m1", "tool_name": "Agent", "tool_use_id": "new",
        "transcript_path": path,
        "tool_input": {"subagent_type": "worker", "model": "opus", "effort": "max", "prompt": BRIEF}});
    let (rc, out, err) = g.run("pretool", &data);
    assert_eq!((rc, err.as_str(), out.matches('\n').count()), (0, "", 1));
    let hso = &serde_json::from_str::<Value>(&out).unwrap()["hookSpecificOutput"];
    assert_eq!(hso["updatedInput"]["model"], "sonnet");
    assert!(hso["additionalContext"].as_str().unwrap().contains("spawned 15 agents"));
}

// --- W3 browser calls in a main window ---
#[test]
fn test_w3_browser() {
    let g = G::new();
    assert_allow(&g.browser_call(Some(&g.browser(4)), json!({})));
    assert_warn(&g.browser_call(Some(&g.browser(5)), json!({})), "`ui-verifier`");
    assert_allow(&g.browser_call(Some(&g.browser(6)), json!({})));
    assert_warn(&g.browser_call(Some(&g.browser(25)), json!({})), "26 browser calls");
    // last turn does not count
    assert_allow(&g.browser_call(Some(&g.browser_turn(2, 10, "mcp__Claude_Browser__get_page_text")), json!({})));
    let chrome = g.browser_turn(5, 0, "mcp__claude-in-chrome__computer");
    assert_warn(&g.browser_call(Some(&chrome), json!({})), "");
    // inside ui-verifier
    assert_allow(&g.browser_call(Some(&g.browser(5)), json!({"agent_id": "a1"})));
    assert_allow(&g.browser_call(None, json!({})));
}

// --- W4 budget ---
#[test]
fn test_w4_budget() {
    let g = G::new();
    assert_allow(&g.spawn(&g.spawn_session(14, 0), "new"));
    assert_warn(&g.spawn(&g.spawn_session(15, 0), "new"), "spawned 15 agents");
    assert_allow(&g.spawn(&g.spawn_session(16, 0), "new"));
    assert_warn(&g.spawn(&g.spawn_session(20, 0), "new"), "get_usage");
    assert_warn(&g.spawn(&g.spawn_session(5, 400_000), "new"), "2000K subagent tokens");
    assert_allow(&g.spawn(&g.spawn_session(6, 400_000), "new"));
    assert_allow(&g.spawn(&g.spawn_session(5, 100_000), "new"));
    // background agents report in task-notification attachments, once per task id
    let path = g.spawn_session(5, 0);
    let mut text = fs_read(&path);
    for tid in ["t1", "t1", "t2"] {
        let rec = json!({"type": "attachment", "attachment": {
            "type": "queued_command", "commandMode": "task-notification",
            "prompt": format!("<task-notification> <task-id>{tid}</task-id>"),
            "usage": {"totalTokens": 800_000}}});
        text.push_str(&format!("{rec}\n"));
    }
    g.write(&path, &text);
    assert_warn(&g.spawn(&path, "new"), "1600K subagent tokens");
    // the current call already written to the transcript is not counted twice
    assert_warn(&g.spawn(&g.spawn_session(16, 0), "a15"), "spawned 15 agents");
}

fn fs_read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap()
}

// --- G1 ---
#[test]
fn test_g1_missing_effort_or_model_is_filled_by_router() {
    let g = G::new();
    for ti in [json!({"model": "sonnet", "prompt": BRIEF}), json!({"effort": "high", "prompt": BRIEF}), json!({"prompt": BRIEF})] {
        let res = g.pre("Agent", ti);
        assert_eq!(res.0, 0, "{res:?}");
        let new = &json_out(&res)["hookSpecificOutput"]["updatedInput"];
        assert_eq!((new["model"].as_str(), new["effort"].as_str()), (Some("sonnet"), Some("high")));
    }
    // the brief check still runs
    assert_deny(&g.pre("Agent", json!({})), "a deadline and a hand-back cap");
}

#[test]
fn test_g1_ok() {
    let g = G::new();
    assert_allow(&g.pre("Agent", ok_agent()));
}

#[test]
fn test_g1_brief_needs_deadline_and_cap() {
    let g = G::new();
    let base = json!({"model": "sonnet", "effort": "high"});
    let with = |p: &str| merge(base.clone(), json!({"prompt": p}));
    let res = g.pre("Agent", with("build"));
    assert_deny(&res, "a deadline and a hand-back cap");
    assert!(res.2.contains("Deadline: 25 min."), "{res:?}");
    assert!(res.2.contains("Hand back at most 4,000 chars; write the rest to a file."), "{res:?}");
    let res = g.pre("Agent", with("Build it. Hand back at most 4,000 chars."));
    assert_deny(&res, "needs a deadline");
    assert!(!res.2.contains("hand-back cap"), "{res:?}");
    let res = g.pre("Agent", with("Build it. Deadline: 25 min."));
    assert_deny(&res, "needs a hand-back cap");
    assert_deny(&g.pre("Agent", base.clone()), "a deadline"); // no prompt at all
}

#[test]
fn test_g1_brief_accepted_forms() {
    let g = G::new();
    let with = |p: &str| json!({"model": "sonnet", "effort": "high", "prompt": p});
    for deadline in ["Deadline 30m", "finish in 20 minutes", "within 15 min", "time box 1h", "stop at 5pm"] {
        assert_allow(&g.pre("Agent", with(&format!("{deadline}. Hand back one line."))));
    }
    for cap in [
        "hand back the status", "at most 3,000 words", "2000 characters", "under 500 lines", "max 4,000 chars",
        "maximum 800 words", "reply in one line", "reply \u{2264} 4,000 chars", "reply \u{2264}4000",
    ] {
        assert_allow(&g.pre("Agent", with(&format!("Do it. Deadline: 25 min. {cap}"))));
    }
    assert_allow(&g.pre("Agent", with("DEADLINE 20 MIN. HAND BACK 3,000 CHARS")));
    for deadline in ["You have 1 hour.", "Time limit: 2 hours.", "a 25-minute budget", "within 30m", "ETA 1h"] {
        assert_allow(&g.pre("Agent", with(&format!("{deadline} Hand back one line."))));
    }
    assert_allow(&g.pre("Agent", with("Hand-back: 3000 chars. Time box: yes.")));
}

#[test]
fn test_g1_max_effort_is_clamped_not_denied() {
    let g = G::new();
    for effort in ["max", "xhigh"] {
        let res = g.pre("Agent", json!({"model": "opus", "effort": effort, "prompt": BRIEF, "description": "build it"}));
        assert_eq!(res.0, 0, "{res:?}");
        let new = &json_out(&res)["hookSpecificOutput"]["updatedInput"];
        assert_eq!((new["model"].as_str(), new["effort"].as_str()), (Some("sonnet"), Some("high")));
    }
}

// --- G2 ---
#[test]
fn test_g2_cron() {
    let g = G::new();
    assert_deny(&g.pre_ctx("CronCreate", json!({"cron": "*/5 * * * *", "prompt": "p"}), 150_000), "no polling");
    assert_allow(&g.pre_ctx("CronCreate", json!({"cron": "7 * * * *", "prompt": "p"}), 150_000));
    assert_allow(&g.pre_ctx("CronCreate", json!({"cron": "*/5 * * * *", "prompt": "p"}), 50_000));
    assert_allow(&g.pre("CronCreate", json!({"cron": "*/5 * * * *", "prompt": "p"}))); // no transcript
}

#[test]
fn test_g2_cron_one_shot() {
    let g = G::new();
    assert_allow(&g.pre_ctx("CronCreate", json!({"cron": "* * * * *", "prompt": "p", "recurring": false}), 150_000));
    assert_deny(&g.pre_ctx("CronCreate", json!({"cron": "* * * * *", "prompt": "p", "recurring": true}), 150_000), "");
}

#[test]
fn test_g2_cron_lists_and_hours() {
    let g = G::new();
    assert_deny(&g.pre_ctx("CronCreate", json!({"cron": "0,10,20 * * * *"}), 150_000), "");
    assert_deny(&g.pre_ctx("CronCreate", json!({"cron": "0,50 * * * *"}), 150_000), ""); // wrap gap 10
    assert_allow(&g.pre_ctx("CronCreate", json!({"cron": "0,30 * * * *"}), 150_000));
    assert_allow(&g.pre_ctx("CronCreate", json!({"cron": "0 */2 * * *"}), 150_000));
    assert_allow(&g.pre_ctx("CronCreate", json!({"cron": "garbage"}), 150_000));
}

#[test]
fn test_g2_wakeup() {
    let g = G::new();
    assert_deny(&g.pre_ctx("ScheduleWakeup", json!({"delaySeconds": 600}), 150_000), "no polling");
    assert_allow(&g.pre_ctx("ScheduleWakeup", json!({"delaySeconds": 1800}), 150_000));
    assert_allow(&g.pre_ctx("ScheduleWakeup", json!({"delaySeconds": 600}), 50_000));
}

#[test]
fn test_g2_loop_skill() {
    let g = G::new();
    let skill = |ti: Value, ctx: i64| g.pre_ctx("Skill", ti, ctx);
    assert_deny(&skill(json!({"skill": "loop", "args": "5m check deploy"}), 150_000), "no polling");
    assert_deny(&skill(json!({"skill": "loop", "args": "30s /foo"}), 150_000), "");
    assert_deny(&skill(json!({"skill": "loop", "args": "10min x"}), 150_000), "");
    assert_allow(&skill(json!({"skill": "loop", "args": "45m check deploy"}), 150_000));
    assert_allow(&skill(json!({"skill": "loop", "args": "1h x"}), 150_000));
    assert_allow(&skill(json!({"skill": "loop", "args": "2h30m x"}), 150_000));
    assert_allow(&skill(json!({"skill": "loop", "args": "check the deploy"}), 150_000));
    assert_allow(&skill(json!({"skill": "loop"}), 150_000));
    assert_allow(&skill(json!({"skill": "other", "args": "5m x"}), 150_000));
    assert_allow(&skill(json!({"skill": "loop", "args": "5m x"}), 50_000));
    assert_deny(&skill(json!({"skill": "loop", "args": "check deploy every 5m"}), 150_000), "");
    assert_allow(&skill(json!({"skill": "loop", "args": "check deploy every 45m"}), 150_000));
}

// --- G3 ---
const NUDGE_350: &str = "Context is 350K. Keep PROGRESS.md (or the run's ledger) current; auto-compact carries the run. Hand off only when the user asks.";

#[test]
fn test_g3_prompt_cadence() {
    let g = G::new();
    let (rc, out, err) = g.prompt_run(350_000, "sess");
    assert_eq!((rc, err.as_str()), (0, ""));
    let payload = &serde_json::from_str::<Value>(&out).unwrap()["hookSpecificOutput"];
    assert_eq!(payload["hookEventName"], "UserPromptSubmit");
    assert_eq!(payload["additionalContext"], NUDGE_350);
    for _ in 0..4 {
        assert_allow(&g.prompt_run(350_000, "sess"));
    }
    let (rc, out, _) = g.prompt_run(350_000, "sess"); // 6th
    assert_eq!(rc, 0);
    assert!(out.contains("additionalContext"), "{out}");
    assert_allow(&g.prompt_run(350_000, "sess"));
}

#[test]
fn test_g3_prompt_below_threshold_and_sessions_independent() {
    let g = G::new();
    assert_allow(&g.prompt_run(299_000, "sess"));
    let (_, out, _) = g.prompt_run(300_000, "a");
    assert!(out.contains("300K"), "{out}");
    let (_, out, _) = g.prompt_run(300_000, "b");
    assert!(out.contains("300K"), "{out}");
    assert!(g.state_dir().join("a.json").is_file());
}

#[test]
fn test_g3_agent_ceiling() {
    let g = G::new();
    let res = g.pre_ctx("Agent", ok_agent(), 600_000);
    assert_deny(&res, "context is 600K (> 350K)");
    assert!(!res.2.contains("/ps:handoff"), "{res:?}");
    assert_deny(&g.pre_ctx("Agent", ok_agent(), 351_000), "context is 351K");
    assert_allow(&g.pre_ctx("Agent", ok_agent(), 340_000));
    // ceiling runs before G1
    assert_deny(&g.pre_ctx("Agent", json!({}), 600_000), "context is 600K");
}

// --- G4 ---
#[test]
fn test_g4_first_ask_allowed_even_if_written() {
    let g = G::new();
    let recs = [human("start"), assistant_b(1000, json!([ask_block("a1", "which?")]))];
    assert_allow(&g.ask(&recs, Some("a1"), "which?"));
    assert_allow(&g.ask(&recs, None, "which?")); // no id: matched by input
    assert_allow(&g.ask(&[human("start"), assistant(1000)], Some("a1"), "which?")); // not yet written
}

#[test]
fn test_g4_second_ask_denied() {
    let g = G::new();
    let recs = [
        human("start"),
        assistant_b(1000, json!([ask_block("a1", "which?")])),
        tool_result("a1"),
        assistant_b(1000, json!([ask_block("a2", "which?")])),
    ];
    assert_deny(&g.ask(&recs, Some("a2"), "which?"), "one AskUserQuestion per turn");
    assert_deny(&g.ask(&recs, None, "which?"), ""); // no id: a1 still counted
    // current call not yet written, earlier one counts
    assert_deny(&g.ask(&recs[..3], Some("a2"), "which?"), "");
}

#[test]
fn test_g4_parallel_asks_first_allowed() {
    let g = G::new();
    let recs = [human("start"), assistant_b(1000, json!([ask_block("a1", "which?"), ask_block("a2", "other?")]))];
    assert_allow(&g.ask(&recs, Some("a1"), "which?"));
    assert_deny(&g.ask(&recs, Some("a2"), "other?"), "one AskUserQuestion per turn");
}

#[test]
fn test_g4_denied_ask_does_not_count() {
    let g = G::new();
    let denied = json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "a1", "is_error": true, "content": "guard: denied"}]}});
    let recs = [human("start"), assistant_b(1000, json!([ask_block("a1", "which?")])), denied,
        assistant_b(1000, json!([ask_block("a2", "which?")]))];
    assert_allow(&g.ask(&recs, Some("a2"), "which?"));
}

#[test]
fn test_g4_askme_interview_exempt() {
    let g = G::new();
    let two = || vec![
        assistant_b(1000, json!([ask_block("a1", "which?")])),
        tool_result("a1"),
        assistant_b(1000, json!([ask_block("a2", "which?")])),
    ];
    let mut recs = vec![human("/ps:askme plan the move")];
    recs.extend(two());
    assert_allow(&g.ask(&recs, Some("a2"), "which?"));
    let skill = assistant_b(1000, json!([{"type": "tool_use", "id": "s1", "name": "Skill", "input": {"skill": "ps:askme"}}]));
    let mut recs = vec![human("grill me"), skill];
    recs.extend(two());
    assert_allow(&g.ask(&recs, Some("a2"), "which?"));
}

#[test]
fn test_g4_new_human_prompt_resets() {
    let g = G::new();
    let recs = [
        human("start"),
        assistant_b(1000, json!([ask_block("a1", "which?")])),
        tool_result("a1"),
        human("answer"),
        assistant_b(1000, json!([ask_block("a2", "which?")])),
    ];
    assert_allow(&g.ask(&recs, Some("a2"), "which?"));
}

#[test]
fn test_g4_list_content_human_and_meta_skipped() {
    let g = G::new();
    let list_prompt = json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "hey"}]}});
    let recs = [
        human("start"),
        assistant_b(1000, json!([ask_block("a1", "which?")])),
        list_prompt,
        assistant_b(1000, json!([ask_block("a2", "which?")])),
    ];
    assert_allow(&g.ask(&recs, Some("a2"), "which?"));
    let meta = merge(human("<system-reminder>"), json!({"isMeta": true}));
    let compact = merge(human("summary"), json!({"isCompactSummary": true}));
    let recs = [human("start"), assistant_b(1000, json!([ask_block("a1", "which?")])), meta, compact,
        assistant_b(1000, json!([ask_block("a2", "which?")]))];
    assert_deny(&g.ask(&recs, Some("a2"), "which?"), "");
}

// --- G5 hand-back cap ---
#[test]
fn test_g5_default_cap_and_message() {
    let g = G::new();
    assert_allow(&g.handback(&"x".repeat(4400))); // 4,000 + 10%
    let res = g.handback(&"x".repeat(4401));
    assert_deny(&res, "guard: hand-back is 4401 chars; the cap is 4000.");
    assert!(res.2.contains("Write the full text to a file"), "{res:?}");
    assert!(res.2.contains("status on line 1, key results, the file path"), "{res:?}");
    assert!(res.2.contains("No Write tool: cut to the top findings."), "{res:?}");
}

#[test]
fn test_g5_cap_from_prompt() {
    let g = G::new();
    let prompt = "Review it. Hand back at most 1,000 chars; write the rest to a file.";
    assert_allow(&g.handback_p(&"x".repeat(1100), prompt, "a1"));
    assert_deny(&g.handback_p(&"x".repeat(1101), prompt, "a1"), "the cap is 1000");
}

#[test]
fn test_g5_smallest_tied_cap_wins() {
    let g = G::new();
    let prompt = "Reply in 2,000 chars. Final message under 1500 characters. Then hand back 800 chars.";
    assert_deny(&g.handback_p(&"x".repeat(900), prompt, "a1"), "the cap is 800");
}

#[test]
fn test_g5_cap_ignored_when_not_tied_to_reply() {
    let g = G::new();
    let prompt = format!("Write a description of at most 160 characters for the page. {} Do the work.", "z".repeat(100));
    assert_deny(&g.handback_p(&"x".repeat(5000), &prompt, "n1"), "the cap is 4000");
    // small numbers (< 200) never count, even next to a keyword
    assert_deny(&g.handback_p(&"x".repeat(5000), "Hand back 50 chars.", "n2"), "the cap is 4000");
    // keyword farther than 80 chars before the number is not a tie
    let far = format!("Reply first.{}Pad the file to 500 chars.", " ".repeat(90));
    assert_deny(&g.handback_p(&"x".repeat(5000), &far, "n3"), "the cap is 4000");
}

#[test]
fn test_g5_cap_phrasings() {
    let g = G::new();
    let cases: [(&str, i64); 9] = [
        ("Keep your response under 6,000 characters.", 6000),
        ("Limit the output to 5,000 chars.", 5000),
        ("6,000 chars max for the hand-back.", 6000),
        ("Make truncate() return at most 500 characters. Hand back at most 4,000 chars.", 4000),
        ("The API summary field holds 300 chars. Hand back at most 2,000 chars.", 2000),
        ("Hand back at most 4,000 chars. The summary field holds 300 chars.", 4000),
        ("Deadline: answer within 8 minutes. Stay \u{2264} 4,000 chars.", 4000),
        ("Keep it under 2,500 characters. Pad the file to 900 chars.", 2500),
        ("Hand back \u{2264} 3,000 chars. In researcher.md set a hard cap of 2,500 chars on the final message.", 3000),
    ];
    for (i, (prompt, cap)) in cases.iter().enumerate() {
        let n = (*cap as f64 * 1.1) as usize + 1;
        assert_deny(&g.handback_p(&"x".repeat(n), prompt, &format!("p{i}")), &format!("the cap is {cap}"));
    }
}

#[test]
fn test_g5_counter_write_fails_allows() {
    let g = G::new();
    let bad = g.join("not-a-dir");
    g.write(&bad, "x");
    let data = json!({"session_id": "s", "hook_event_name": "SubagentStop", "last_assistant_message": "x".repeat(5000)});
    for _ in 0..3 {
        let (rc, _, err) = g.run_with("subagentstop", &data, &[("TMPDIR", &bad)]);
        assert_eq!((rc, err.as_str()), (0, ""));
    }
}

#[test]
fn test_g5_defaults_by_agent_type() {
    let g = G::new();
    for (kind, cap) in [("task-implementer", 3000), ("task-reviewer", 3000), ("code-reviewer", 4000),
        ("researcher", 3000), ("repo-scanner", 2000), ("ui-verifier", 3000), ("general-purpose", 4000), ("Explore", 4000)]
    {
        let big = (cap as f64 * 1.1) as usize;
        let res = g.handback_full(json!("x".repeat(big + 1)), None, Some(kind), kind, "pretool", json!({}));
        assert_deny(&res, &format!("the cap is {cap}"));
        let res = g.handback_full(json!("x".repeat(big)), None, Some(kind), &format!("{kind}-ok"), "pretool", json!({}));
        assert_allow(&res);
    }
}

#[test]
fn test_g5_two_denies_then_allow() {
    let g = G::new();
    let big = "x".repeat(5000);
    assert_deny(&g.handback(&big), "");
    assert_deny(&g.handback(&big), "");
    assert_allow(&g.handback(&big));
    assert_allow(&g.handback(&big));
    // another agent has its own counter
    assert_deny(&g.handback_full(json!(big), None, None, "a2", "pretool", json!({})), "");
    assert!(g.state_dir().join("handback-a1.json").is_file());
}

#[test]
fn test_g5_counter_keyed_by_session_without_agent_id() {
    let g = G::new();
    let data = json!({"session_id": "sess9", "hook_event_name": "SubagentStop", "last_assistant_message": "x".repeat(5000)});
    assert_deny(&g.run("subagentstop", &data), "");
    assert!(g.state_dir().join("handback-sess9.json").is_file());
}

#[test]
fn test_g5_subagentstop() {
    let g = G::new();
    let stop = |msg: Value, id: &str, extra: Value| g.handback_stop(msg, id, extra);
    assert_deny(&stop(json!("x".repeat(5000)), "a1", json!({})), "hand-back is 5000 chars");
    assert_allow(&stop(json!("x".repeat(100)), "a1", json!({})));
    assert_allow(&stop(json!("x".repeat(5000)), "b1", json!({"stop_hook_active": true})));
    assert_deny(&stop(json!("x".repeat(5000)), "b2", json!({"stop_hook_active": false})), "");
    // missing or wrong-typed text allows
    assert_allow(&stop(Value::Null, "b3", json!({})));
    assert_allow(&stop(json!(12345), "b4", json!({})));
    assert_allow(&g.run("subagentstop", &json!({"session_id": "s"})));
}

#[test]
fn test_g5_subagentstop_cap_from_prompt() {
    let g = G::new();
    let prompt = "Return a final message of at most 600 characters.";
    let res = g.handback_full(json!("x".repeat(700)), Some(prompt), None, "a1", "subagentstop", json!({}));
    assert_deny(&res, "the cap is 600");
}

#[test]
fn test_g5_transcript_path_variants() {
    let g = G::new();
    let main = g.sub(Some("Hand back at most 500 chars."), "v1");
    let sub = g.join("main/subagents/agent-v1.jsonl");
    let msg = "x".repeat(1000);
    // agent_transcript_path wins
    let d = json!({"session_id": "s", "agent_id": "zz", "transcript_path": "/no/such.jsonl",
        "agent_transcript_path": sub, "last_assistant_message": msg});
    assert_deny(&g.run("subagentstop", &d), "the cap is 500");
    // transcript_path already inside subagents/
    let d = json!({"session_id": "s", "agent_id": "v2", "transcript_path": sub, "last_assistant_message": msg});
    assert_deny(&g.run("subagentstop", &d), "the cap is 500");
    // derived from the main transcript and agent_id
    let d = json!({"session_id": "s", "agent_id": "v1", "transcript_path": main, "last_assistant_message": msg});
    assert_deny(&g.run("subagentstop", &d), "the cap is 500");
}

#[test]
fn test_g5_prompt_as_text_blocks_and_first_user_record_only() {
    let g = G::new();
    let main = g.transcript_named(&[human("go")], "m2.jsonl");
    g.lines(&g.join("m2/subagents/agent-t1.jsonl"), &[
        user_rec(json!([{"type": "tool_result", "tool_use_id": "x", "content": "y"}])),
        user_rec(json!([{"type": "text", "text": "Hand back at most 400 chars."}])),
        user_rec(json!("Hand back at most 300 chars.")),
    ]);
    let d = json!({"session_id": "s", "agent_id": "t1", "transcript_path": main, "last_assistant_message": "x".repeat(600)});
    assert_deny(&g.run("subagentstop", &d), "the cap is 400");
}

#[test]
fn test_g5_prompt_read_from_start_limited_to_200_lines() {
    let g = G::new();
    let main = g.transcript_named(&[human("go")], "m3.jsonl");
    let mut recs: Vec<Value> = (0..200).map(|_| assistant(1000)).collect();
    recs.push(user_rec(json!("Hand back at most 300 chars.")));
    g.lines(&g.join("m3/subagents/agent-l1.jsonl"), &recs);
    let d = json!({"session_id": "s", "agent_id": "l1", "transcript_path": main, "last_assistant_message": "x".repeat(600)});
    assert_allow(&g.run("subagentstop", &d)); // prompt past line 200 ignored: default cap 4000
}

#[test]
fn test_g5_missing_transcript_falls_back_to_default() {
    let g = G::new();
    let mut d = json!({"session_id": "s", "agent_id": "q", "transcript_path": "/no/such/main.jsonl",
        "agent_type": "repo-scanner", "last_assistant_message": "x".repeat(2300)});
    assert_deny(&g.run("subagentstop", &d), "the cap is 2000");
    d["last_assistant_message"] = json!("x".repeat(100));
    assert_allow(&g.run("subagentstop", &d));
    assert_allow(&g.run("subagentstop", &json!({"agent_id": "q2", "last_assistant_message": "x".repeat(100)})));
}

#[test]
fn test_g5_pretool_ignores_other_tools_and_bad_input() {
    let g = G::new();
    assert_allow(&g.pre("Read", json!({"message": "x".repeat(9000)})));
    assert_allow(&g.pre("SubagentHandback", json!({})));
    assert_allow(&g.pre("SubagentHandback", json!({"message": 42})));
    assert_allow(&g.run("pretool", &json!({"tool_name": "SubagentHandback", "tool_input": "oops"})));
}

// --- G6 hand-off ---
fn cat(parts: &[&[Value]]) -> Vec<Value> {
    parts.iter().flat_map(|p| p.iter().cloned()).collect()
}

#[test]
fn test_g6_stop_needs_link_and_block() {
    let g = G::new();
    let base = vec![human("hand off"), g.write_ho("w1", None), tool_result("w1")];
    let with = |r: Value| cat(&[&base, &[r]]);
    assert_deny(&g.stop_run(&with(g.say("The hand-off is written.")), json!({})), &format!("Read {}", g.ho));
    assert_deny(&g.stop_run(&with(g.say(&format!("Read {} and do it.", g.ho))), json!({})), "file link");
    assert_allow(&g.stop_run(&with(g.good_reply()), json!({})));
    // last_assistant_message counts when the transcript is not flushed yet
    assert_allow(&g.stop_run(&base, json!({"last_assistant_message": g.good_reply_text()})));
}

#[test]
fn test_g6_stop_one_deny_and_scope() {
    let g = G::new();
    let base = vec![human("hand off"), g.write_ho("w1", None), tool_result("w1"), g.say("written")];
    assert_allow(&g.stop_run(&base, json!({"stop_hook_active": true})));
    assert_allow(&g.stop_run(&[human("x"), g.say("no hand-off here")], json!({})));
    // written in an earlier turn: the user already got it
    assert_allow(&g.stop_run(&cat(&[&base, &[human("thanks"), g.say("ok")]]), json!({})));
    // a Write elsewhere is not a hand-off
    assert_allow(&g.stop_run(&[human("x"), g.write_ho("w1", Some("/repo/notes.md")), g.say("ok")], json!({})));
}

#[test]
fn test_g6_archive_same_turn_denied() {
    let g = G::new();
    let base = vec![human("hand off and archive"), g.write_ho("w1", None), tool_result("w1"), g.good_reply()];
    assert_deny(&g.archive(&base, "self"), "Do not archive yet");
    assert_allow(&g.archive(&cat(&[&base, &[human("ok, archive")]]), "self"));
    assert_allow(&g.archive(&base, "other-session"));
    assert_allow(&g.archive(&[human("archive this"), g.say("done")], "self"));
}

#[test]
fn test_scratch_allowed_paths() {
    let g = G::new();
    let own = g.join("s.py");
    for p in [
        "/tmp/x.py", "/private/tmp/a/b.sh", "/var/folders/ab/cd/T/x.js", "/private/var/folders/ab/cd/T/x.js",
        own.as_str(), "/Users/someone/.claude/tmp/scratchpad/s.py", "/private/tmp/claude-501/p/abc/scratchpad/s.py",
        "/Users/someone/proj/.superpowers/sdd/task-9-report.md", "/Users/someone/proj/.agents/notes.md",
    ] {
        assert_allow(&g.scratch(p, "file_path", json!({})));
    }
    assert_allow(&g.scratch(&g.join("nb.ipynb"), "notebook_path", json!({})));
}

#[test]
fn test_scratch_denied_paths() {
    let g = G::new();
    for p in [
        "/etc/hosts", "~/dotfiles/x.py", "/tmpfoo/x", "/Users/someone/proj/src/app.ts", "/tmp/../etc/passwd",
        "relative.py", "/Users/someone/proj/.superpowersx/r.md", "/Users/someone/proj/README.md",
    ] {
        assert_deny(
            &g.scratch(p, "file_path", json!({"cwd": "/Users/someone/proj"})),
            "guard: this agent writes only scratch files; write under the scratchpad, $TMPDIR, .agents/ or .superpowers/.",
        );
    }
    assert_deny(&g.scratch("/etc/x.ipynb", "notebook_path", json!({})), "scratch files");
}

#[cfg(unix)]
fn symlink(target: &str, link: &str) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(not(unix))]
fn symlink(_target: &str, _link: &str) {
    panic!("these tests need symlinks (unix)");
}

#[test]
fn test_scratch_tilde_symlink_and_relative() {
    let g = G::new();
    assert_deny(&g.scratch("~/x.py", "file_path", json!({})), "scratch files");
    // a symlink inside the temp dir that points outside is denied
    let link = g.join("link");
    symlink("/etc", &link);
    assert_deny(&g.scratch(&format!("{link}/hosts"), "file_path", json!({})), "scratch files");
    // a relative path resolves against cwd
    assert_allow(&g.scratch("s.py", "file_path", json!({"cwd": g.d()})));
    assert_deny(&g.scratch("s.py", "file_path", json!({"cwd": "/Users/someone/proj"})), "scratch files");
}

#[test]
fn test_scratch_fail_open() {
    let g = G::new();
    assert_allow(&g.run_raw("scratch", "not json"));
    assert_allow(&g.run("scratch", &json!({"tool_name": "Write", "tool_input": {}})));
    assert_allow(&g.run("scratch", &json!({"tool_name": "Write", "tool_input": "oops"})));
    assert_allow(&g.run("scratch", &json!({"tool_name": "Write", "tool_input": {"file_path": 5}})));
}

// --- transcript reading ---
// test_last_assistant_usage_used_and_large_file is a unit test in src/bin/guard.rs.

// --- concurrency cap ---
#[test]
fn test_cap_three_at_once_then_stop_frees_a_slot() {
    let g = G::new();
    for _ in 0..3 {
        assert_allow(&g.agent("c1"));
    }
    let res = g.agent("c1");
    assert_deny(&res, "3 helpers");
    for text in ["cap is 3", "Wait for one to finish, or do the step yourself"] {
        assert!(res.2.contains(text), "{res:?}");
    }
    // another session has its own count
    assert_allow(&g.agent("c2"));
    assert_allow(&g.subagent_stop("c1"));
    assert_allow(&g.agent("c1"));
    assert_deny(&g.agent("c1"), "cap is 3");
}

#[test]
fn test_cap_failed_spawn_frees_its_own_slot() {
    let g = G::new();
    for tid in ["t1", "t2", "t3"] {
        assert_allow(&g.spawn_with_id("f1", tid));
    }
    assert_deny(&g.spawn_with_id("f1", "t4"), "cap is 3");
    // the failure of t2 frees t2's slot, not the oldest
    assert_allow(&g.failure("f1", json!("t2")));
    assert_allow(&g.spawn_with_id("f1", "t5"));
    assert_deny(&g.spawn_with_id("f1", "t6"), "cap is 3");
    // a failure of an unknown id, a missing id or another session frees nothing
    assert_allow(&g.failure("f1", json!("nope")));
    assert_allow(&g.failure("f1", Value::Null));
    assert_allow(&g.failure("other", json!("t1")));
    assert_deny(&g.spawn_with_id("f1", "t7"), "cap is 3");
    // freeing the same id twice frees one slot
    assert_allow(&g.failure("f1", json!("t1")));
    assert_allow(&g.failure("f1", json!("t1")));
    assert_allow(&g.spawn_with_id("f1", "t8"));
    assert_deny(&g.spawn_with_id("f1", "t9"), "cap is 3");
}

#[test]
fn test_cap_failure_ignores_other_tools_and_bad_input() {
    let g = G::new();
    assert_allow(&g.spawn_with_id("f2", "a"));
    assert_allow(&g.run("posttoolfailure", &g.failure_data("f2", json!("a"), json!({"tool_name": "Bash"}))));
    assert_allow(&g.run_raw("posttoolfailure", "not json"));
    for _ in 0..2 {
        assert_allow(&g.spawn_with_id("f2", "b"));
    }
    assert_deny(&g.spawn_with_id("f2", "c"), "cap is 3");
}

#[test]
fn test_cap_old_format_slots_still_load() {
    let g = G::new();
    let now = now_secs();
    // plain times from the first version
    g.write(&g.state_dir().join("f3.json").to_string_lossy(), &json!({"spawns": [now - 30.0, now - 20.0]}).to_string());
    assert_allow(&g.spawn_with_id("f3", "n1"));
    assert_deny(&g.spawn_with_id("f3", "n2"), "cap is 3");
    assert_allow(&g.subagent_stop("f3")); // pops the oldest (plain) one
    assert_allow(&g.spawn_with_id("f3", "n3"));
    assert_allow(&g.failure("f3", json!("n3")));
    assert_allow(&g.spawn_with_id("f3", "n4"));
}

#[test]
fn test_cap_denied_call_is_not_counted_and_stop_never_goes_negative() {
    let g = G::new();
    for _ in 0..3 {
        g.agent("c3");
    }
    for _ in 0..3 {
        assert_deny(&g.agent("c3"), "");
    }
    for _ in 0..5 {
        // more stops than spawns
        assert_allow(&g.subagent_stop("c3"));
    }
    for _ in 0..3 {
        assert_allow(&g.agent("c3"));
    }
    assert_deny(&g.agent("c3"), "");
}

#[test]
fn test_cap_other_denies_do_not_take_a_slot() {
    let g = G::new();
    for _ in 0..5 {
        // no deadline: denied by G1 before the cap
        assert_deny(&g.run("pretool", &g.agent_data("c4", json!({"prompt": "build"}))), "a deadline");
    }
    for _ in 0..3 {
        assert_allow(&g.agent("c4"));
    }
}

#[test]
fn test_cap_stop_with_denied_handback_keeps_the_slot() {
    let g = G::new();
    for _ in 0..3 {
        g.agent("c5");
    }
    let big = g.stop_data("c5", json!({"last_assistant_message": "x".repeat(5000)}));
    assert_deny(&g.run("subagentstop", &big), "");
    assert_deny(&g.agent("c5"), "cap is 3");
}

#[test]
fn test_cap_entries_older_than_90_minutes_are_dropped() {
    let g = G::new();
    let old = now_secs() - 91.0 * 60.0;
    g.write(&g.state_dir().join("c6.json").to_string_lossy(), &json!({"spawns": [old, old, old]}).to_string());
    for _ in 0..3 {
        assert_allow(&g.agent("c6"));
    }
    assert_deny(&g.agent("c6"), "");
}

#[test]
fn test_cap_by_pace() {
    let g = G::new();
    let ahead = Some(fake_pace("ahead", 60.0, 50.0));
    assert_allow(&g.inproc("pretool", &g.agent_data("p1", json!({})), ahead));
    let res = g.inproc("pretool", &g.agent_data("p1", json!({})), ahead);
    assert_deny(&res, "1 helper");
    for text in ["cap is 1", "60%", "50%"] {
        assert!(res.2.contains(text), "{res:?}"); // the pace numbers
    }
    let crit = Some(fake_pace("critical", 90.0, 100.0));
    let res = g.inproc("pretool", &g.agent_data("p2", json!({})), crit);
    assert_deny(&res, "cap is 0");
    assert!(res.2.contains("90%"), "{res:?}");
    for state in ["ok", "unknown"] {
        let sid = format!("p3{state}");
        for _ in 0..3 {
            assert_allow(&g.inproc("pretool", &g.agent_data(&sid, json!({})), Some(pace_default(state))));
        }
        assert_deny(&g.inproc("pretool", &g.agent_data(&sid, json!({})), Some(pace_default(state))), "cap is 3");
    }
}

#[test]
fn test_cap_without_pace_module_is_unknown() {
    let g = G::new();
    for _ in 0..3 {
        assert_allow(&g.inproc("pretool", &g.agent_data("p4", json!({})), None));
    }
    assert_deny(&g.inproc("pretool", &g.agent_data("p4", json!({})), None), "cap is 3");
}

#[test]
fn test_cap_pace_crash_is_unknown() {
    let g = G::new();
    // a pace read that fails: here an unreadable statusline cache
    g.write(&g.join("cache/claude-pace.json"), "{not json");
    for _ in 0..3 {
        assert_allow(&g.agent("p5"));
    }
    assert_deny(&g.agent("p5"), "cap is 3");
}

#[test]
fn test_cap_parallel_calls_race_safely() {
    let g = G::new();
    let results: Vec<R> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..6).map(|_| s.spawn(|| g.agent("par"))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut codes: Vec<i32> = results.iter().map(|r| r.0).collect();
    codes.sort();
    assert_eq!(codes, vec![0, 0, 0, 2, 2, 2]);
}

#[test]
fn test_cap_state_dir_unwritable_fails_open() {
    let g = G::new();
    let bad = g.join("not-a-dir");
    g.write(&bad, "x");
    let data = g.agent_data("c9", json!({}));
    for _ in 0..5 {
        let res = g.run_with("pretool", &data, &[("TMPDIR", &bad)]);
        assert_eq!(res, (0, String::new(), String::new()));
    }
}

// --- plan gate ---
struct S {
    mode: Option<&'static str>,
    pace: Option<Pace>,
    model: Option<&'static str>,
    entry: Option<&'static str>,
    extra: Value,
}

impl S {
    fn new() -> S {
        S { mode: Some("default"), pace: None, model: Some("claude-sonnet-5-5"), entry: Some("cli"), extra: json!({}) }
    }
}

impl G {
    /// A main transcript: the entrypoint on the first record, the model on the newest assistant one.
    fn plan_transcript(&self, model: Option<&str>, entry: Option<&str>) -> String {
        let mut first = human("hi");
        if let Some(e) = entry {
            first["entrypoint"] = json!(e);
        }
        let mut rec = assistant(1000);
        if let Some(m) = model {
            rec["message"]["model"] = json!(m);
        }
        let mut recs = vec![first, rec];
        if model.is_some() {
            let mut synth = assistant(1000);
            synth["message"]["model"] = json!("<synthetic>");
            recs.push(human("go"));
            recs.push(synth);
        }
        self.transcript_named(&recs, "plan.jsonl")
    }

    fn skill(&self, name: &str, s: S) -> R {
        let mut data = json!({"hook_event_name": "PreToolUse", "session_id": "g1", "tool_name": "Skill",
            "tool_input": {"skill": name}});
        if let Some(m) = s.mode {
            data["permission_mode"] = json!(m);
        }
        if s.model.is_some() || s.entry.is_some() {
            data["transcript_path"] = json!(self.plan_transcript(s.model, s.entry));
        }
        let data = merge(data, s.extra);
        if s.pace.is_some() { self.inproc("pretool", &data, s.pace) } else { self.run("pretool", &data) }
    }
}

const PLAN_SKILLS: [&str; 2] = ["superpowers:brainstorming", "superpowers:writing-plans"];

#[test]
fn test_plan_gate_denies_planning_skills_outside_plan_mode() {
    let g = G::new();
    // CLI + Sonnet or Haiku: opusplan applies, so plans go through plan mode
    for name in PLAN_SKILLS {
        for model in ["claude-sonnet-5-5", "claude-haiku-4-5"] {
            for pace in [None, Some(pace_default("ok")), Some(pace_default("unknown"))] {
                let res = g.skill(name, S { pace, model: Some(model), ..S::new() });
                assert_deny(&res, "EnterPlanMode");
                assert!(res.2.contains("plan mode"), "{res:?}");
                assert!(res.2.contains("(autopilot: send a `Plan` helper instead)"), "{res:?}");
            }
        }
        assert_deny(&g.skill(name, S { mode: None, ..S::new() }), "EnterPlanMode");
        assert_deny(&g.skill(name, S { mode: Some("acceptEdits"), ..S::new() }), "EnterPlanMode");
    }
}

#[test]
fn test_plan_gate_desktop_sonnet_points_to_plan_helper_or_opus() {
    let g = G::new();
    for name in PLAN_SKILLS {
        let res = g.skill(name, S { entry: Some("claude-desktop"), ..S::new() });
        assert_deny(&res, "send a `Plan` helper (Opus by the router), or pick Opus in the model menu");
        assert!(!res.2.contains("EnterPlanMode"), "{res:?}");
        let res = g.skill(name, S { entry: Some("claude-desktop"), pace: Some(pace_default("ok")), ..S::new() });
        assert_deny(&res, "pick Opus");
        assert_allow(&g.skill(name, S { entry: Some("claude-desktop"), mode: Some("plan"), ..S::new() }));
    }
}

#[test]
fn test_plan_gate_allows_opus_and_fable_mains() {
    let g = G::new();
    for model in ["claude-opus-5-5", "claude-fable-5-1", "opus"] {
        for entry in ["cli", "claude-desktop"] {
            assert_allow(&g.skill("superpowers:writing-plans", S { model: Some(model), entry: Some(entry), ..S::new() }));
            assert_allow(&g.skill("superpowers:brainstorming",
                S { model: Some(model), entry: Some(entry), pace: Some(pace_default("ok")), ..S::new() }));
        }
    }
}

#[test]
fn test_plan_gate_unknown_model_or_entrypoint_allows() {
    let g = G::new();
    let name = "superpowers:writing-plans";
    assert_allow(&g.skill(name, S { model: None, entry: None, ..S::new() })); // no transcript at all
    assert_allow(&g.skill(name, S { model: None, entry: Some("cli"), ..S::new() })); // no assistant record
    assert_allow(&g.skill(name, S { entry: None, ..S::new() })); // no entrypoint
    assert_allow(&g.skill(name, S { entry: Some("sdk-cli"), ..S::new() })); // not cli or desktop
    assert_allow(&g.skill(name, S { model: Some("mystery-model"), ..S::new() }));
    assert_allow(&g.skill(name, S { extra: json!({"transcript_path": "/no/such/file.jsonl"}), ..S::new() }));
}

#[test]
fn test_plan_gate_uses_the_newest_assistant_model() {
    let g = G::new();
    let with_model = |m: &str| {
        let mut r = assistant(1000);
        r["message"]["model"] = json!(m);
        r
    };
    let path = g.transcript(&[merge(human("hi"), json!({"entrypoint": "cli"})), with_model("claude-sonnet-5-5"),
        with_model("claude-opus-5-5")]);
    let data = json!({"hook_event_name": "PreToolUse", "session_id": "g1", "tool_name": "Skill",
        "permission_mode": "default", "tool_input": {"skill": "superpowers:writing-plans"}, "transcript_path": path});
    assert_allow(&g.run("pretool", &data)); // switched to Opus mid-session
}

#[test]
fn test_plan_gate_allows() {
    let g = G::new();
    assert_allow(&g.skill("superpowers:writing-plans", S { mode: Some("plan"), ..S::new() }));
    assert_allow(&g.skill("superpowers:brainstorming", S { mode: Some("plan"), ..S::new() }));
    assert_allow(&g.skill("superpowers:test-driven-development", S::new()));
    assert_allow(&g.skill("ps:handoff", S::new()));
    assert_allow(&g.skill("superpowers:writing-plans", S { extra: json!({"agent_id": "a1"}), ..S::new() })); // a subagent
    assert_allow(&g.skill("superpowers:writing-plans",
        S { extra: json!({"transcript_path": "/x/subagents/agent-1.jsonl"}), ..S::new() }));
    for state in ["ahead", "critical"] {
        // Sonnet plans without plan mode
        assert_allow(&g.skill("superpowers:writing-plans", S { pace: Some(fake_pace(state, 90.0, 50.0)), ..S::new() }));
        assert_allow(&g.skill("superpowers:brainstorming", S { pace: Some(fake_pace(state, 90.0, 50.0)), ..S::new() }));
    }
}

#[test]
fn test_enter_plan_mode_denied_when_ahead() {
    let g = G::new();
    let enter = |pace: Option<Pace>| {
        let data = json!({"hook_event_name": "PreToolUse", "session_id": "g2", "tool_name": "EnterPlanMode", "tool_input": {}});
        if pace.is_some() { g.inproc("pretool", &data, pace) } else { g.run("pretool", &data) }
    };
    for state in ["ahead", "critical"] {
        let res = enter(Some(fake_pace(state, 90.0, 50.0)));
        assert_deny(&res, "Plan without plan mode");
        assert!(res.2.contains("90%"), "{res:?}");
    }
    assert_allow(&enter(Some(pace_default("ok"))));
    assert_allow(&enter(Some(pace_default("unknown"))));
    assert_allow(&enter(None));
}

// --- pace notes (UserPromptSubmit) ---
impl G {
    fn prompt_pace(&self, pace: Option<Pace>, sid: &str, ctx: i64) -> Option<String> {
        let path = self.transcript_named(&[human("hi"), assistant(ctx)], &format!("{sid}.jsonl"));
        let data = json!({"hook_event_name": "UserPromptSubmit", "session_id": sid, "transcript_path": path, "prompt": "go"});
        let (rc, out, err) = self.inproc("prompt", &data, pace);
        assert_eq!((rc, err.as_str()), (0, ""));
        if out.is_empty() {
            None
        } else {
            let v: Value = serde_json::from_str(&out).unwrap();
            Some(v["hookSpecificOutput"]["additionalContext"].as_str().unwrap().to_string())
        }
    }
}

#[test]
fn test_pace_note_once_when_ahead() {
    let g = G::new();
    let ahead = Some(fake_pace("ahead", 62.0, 50.0));
    let note = g.prompt_pace(ahead, "n1", 1000).expect("a note");
    for text in ["62%", "50%", "Sonnet", "max 1 at once", "without plan mode", "pick Sonnet"] {
        assert!(note.contains(text), "{note:?} lacks {text:?}");
    }
    assert!(g.prompt_pace(ahead, "n1", 1000).is_none()); // only once
    // The Python test also counts pace.refresh(budget_s=5) calls; a binary hides them (see PORTED.md).
    assert!(g.prompt_pace(Some(pace_default("unknown")), "n1", 1000).is_none());
    assert!(g.prompt_pace(ahead, "n1", 1000).is_none()); // unknown does not reset the memory
}

#[test]
fn test_pace_note_critical_and_return_to_ok() {
    let g = G::new();
    let p = |state, used, budget, ctx| g.prompt_pace(Some(fake_pace(state, used, budget)), "n1", ctx);
    assert!(g.prompt_pace(Some(pace_default("ok")), "n1", 1000).is_none());
    assert!(p("ahead", 60.0, 50.0, 1000).unwrap().contains("Sonnet"));
    let note = p("critical", 90.0, 100.0, 1000).unwrap();
    for text in ["90%", "Sonnet only", "no helpers", "Tell the user"] {
        assert!(note.contains(text), "{note:?} lacks {text:?}");
    }
    // critical repeats on every prompt, ahead does not
    for used in [91.0, 92.0] {
        assert!(p("critical", used, 100.0, 1000).unwrap().contains("Sonnet only"));
    }
    assert!(g.prompt_pace(Some(pace_default("ok")), "n1", 1000).is_none());
    assert!(p("ahead", 60.0, 50.0, 1000).unwrap().contains("Sonnet")); // ahead again after ok
}

#[test]
fn test_pace_note_sessions_independent_and_joined_with_nudge() {
    let g = G::new();
    let ahead = Some(fake_pace("ahead", 60.0, 50.0));
    assert!(g.prompt_pace(ahead, "n2", 1000).is_some());
    assert!(g.prompt_pace(ahead, "n3", 1000).is_some());
    let both = g.prompt_pace(Some(fake_pace("critical", 90.0, 100.0)), "n4", 350_000).unwrap();
    assert!(both.contains("Context is 350K"), "{both}");
    assert!(both.contains("Sonnet only"), "{both}");
}

#[test]
fn test_pace_note_fail_open() {
    let g = G::new();
    // a pace read that fails: an unreadable statusline cache
    g.write(&g.join("cache/claude-pace.json"), "{not json");
    let path = g.transcript_named(&[human("hi"), assistant(1000)], "n5.jsonl");
    let data = json!({"hook_event_name": "UserPromptSubmit", "session_id": "n5", "transcript_path": path, "prompt": "go"});
    assert_allow(&g.run("prompt", &data));
    assert!(g.prompt_pace(None, "n6", 1000).is_none());
    // a refresh that fails does not stop the status read (the refresh itself is not observable here)
    let note = g.prompt_pace(Some(fake_pace("ahead", 60.0, 50.0)), "n7", 1000).unwrap();
    assert!(note.contains("Sonnet"), "{note}");
}

// --- off switch ---
#[test]
fn test_check_off() {
    // check_off is private to the binary: drive it through G14 (macOS sed -i) with the list forms.
    let g = G::new();
    let data = json!({"tool_name": "Bash", "tool_input": {"command": "sed -i s/a/b/ f"}});
    let run = |off: &str| g.run_with("pretool", &data, &[("GUARD_PLATFORM", "darwin"), ("GUARD_OFF", off)]);
    assert_deny(&run(""), "G14");
    assert_allow(&run("g14, G13")); // lower case, comma and space
    assert_allow(&run("G13 G14"));
    assert_deny(&run("g10, G13"), "G14");
    assert_deny(&run("G1"), "G14"); // a prefix is not the id
}

// --- G10 to G12 Stop checks and the G6 fix ---
fn stamp(ago_s: i64) -> String {
    common::iso_utc(now_secs() as i64 - ago_s)
}

fn human_at(ago_s: i64, text: &str) -> Value {
    merge(human(text), json!({"timestamp": stamp(ago_s)}))
}

fn tu(name: &str, tool_input: Value, tid: &str) -> Value {
    assistant_b(1000, json!([{"type": "tool_use", "id": tid, "name": name, "input": tool_input}]))
}

// G6 fix: a hand-off that is gone from disk is not owed to the user
#[test]
fn test_g6_deleted_handoff_is_skipped() {
    let g = G::new();
    let gone = g.join("handoffs/gone.md");
    let base = vec![human("hand off"), g.write_ho("w1", Some(&gone)), tool_result("w1"), g.say("written")];
    assert_allow(&g.stop_run(&base, json!({})));
    assert_allow(&g.archive(&base, "self"));
    g.write(&gone, "# Task");
    assert_deny(&g.stop_run(&base, json!({})), &format!("Read {gone}"));
    assert_deny(&g.archive(&base, "self"), "Do not archive yet");
}

// G10 commit check
impl G {
    fn git(&self, repo: &str, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-C", repo])
            .args(args)
            .env("HOME", &self.dir)
            .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("run git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn real_dir(&self) -> PathBuf {
        std::fs::canonicalize(&self.dir).unwrap()
    }

    fn make_repo(&self, upstream: bool) -> String {
        let work = self.real_dir().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let work = work.to_string_lossy().into_owned();
        self.git(&work, &["init", "-q"]);
        self.git(&work, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        self.write_file(&work, "base.txt", "base");
        self.git(&work, &["add", "-A"]);
        self.git(&work, &["commit", "-q", "-m", "base"]);
        if upstream {
            let bare = self.real_dir().join("origin.git");
            std::fs::create_dir_all(&bare).unwrap();
            let bare = bare.to_string_lossy().into_owned();
            self.git(&bare, &["init", "-q", "--bare"]);
            self.git(&work, &["remote", "add", "origin", &bare]);
            self.git(&work, &["push", "-q", "-u", "origin", "main"]);
        }
        work
    }

    fn write_file(&self, repo: &str, name: &str, text: &str) -> String {
        let p = format!("{repo}/{name}");
        std::fs::write(&p, text).unwrap();
        p
    }

    fn g10(&self, recs: &[Value], repo: &str, extra: Value) -> R {
        self.stop_msg(recs, merge(json!({"cwd": repo}), extra), &[])
    }
}

#[test]
fn test_g10_dirty_file_touched_this_turn() {
    let g = G::new();
    let repo = g.make_repo(true);
    let p = g.write_file(&repo, "new.txt", "x");
    let recs = [human("build"), tu("Write", json!({"file_path": p}), "u1"), tool_result("u1"), g.say("done")];
    let res = g.g10(&recs, &repo, json!({}));
    assert_deny(&res, "G10: this turn left work in work: 1 uncommitted file(s), 0 unpushed commit(s).");
    assert!(res.2.contains("Rule 13: commit and push now, unless the user said not to."), "{res:?}");
    // Edit and NotebookEdit count as well
    let q = g.write_file(&repo, "base.txt", "changed");
    assert_deny(&g.g10(&[human("x"), tu("Edit", json!({"file_path": q}), "u1")], &repo, json!({})), "1 uncommitted");
    let n = g.write_file(&repo, "nb.ipynb", "{}");
    assert_deny(&g.g10(&[human("x"), tu("NotebookEdit", json!({"notebook_path": n}), "u1")], &repo, json!({})), "1 uncommitted");
}

#[test]
fn test_g10_untouched_files_stay_quiet() {
    let g = G::new();
    let repo = g.make_repo(true);
    g.write_file(&repo, "other.txt", "x"); // dirty, but the session never wrote it
    let p = g.write_file(&repo, "mine.txt", "x");
    g.git(&repo, &["add", "mine.txt"]);
    g.git(&repo, &["commit", "-q", "-m", "mine"]);
    g.git(&repo, &["push", "-q"]);
    assert_allow(&g.g10(&[human("x"), tu("Write", json!({"file_path": p}), "u1"), g.say("ok")], &repo, json!({})));
    // touched in an earlier turn only
    let d = g.write_file(&repo, "old.txt", "x");
    let recs = [human("a"), tu("Write", json!({"file_path": d}), "u1"), human("b"), g.say("chat")];
    assert_allow(&g.g10(&recs, &repo, json!({})));
    // a file outside the repo is not in its status
    let outside = g.write_file(&g.d(), "elsewhere.txt", "x");
    assert_allow(&g.g10(&[human("x"), tu("Write", json!({"file_path": outside}), "u1")], &repo, json!({})));
}

#[test]
fn test_g10_unpushed_needs_a_commit_this_turn() {
    let g = G::new();
    let repo = g.make_repo(true);
    let p = g.write_file(&repo, "a.txt", "x");
    g.git(&repo, &["add", "-A"]);
    g.git(&repo, &["commit", "-q", "-m", "a"]);
    let commit = tu("Bash", json!({"command": "git commit -m \"a\""}), "u1");
    let res = g.g10(&[human("x"), tu("Write", json!({"file_path": p}), "u1"), commit.clone(), g.say("ok")], &repo, json!({}));
    assert_deny(&res, "0 uncommitted file(s), 1 unpushed commit(s).");
    // an unpushed commit from an earlier turn is not this turn's work
    assert_allow(&g.g10(&[human("x"), commit, human("y"), g.say("chat")], &repo, json!({})));
    assert_allow(&g.g10(&[human("x"), g.say("chat")], &repo, json!({})));
}

#[test]
fn test_g10_user_said_not_to_push() {
    let g = G::new();
    let repo = g.make_repo(true);
    let p = g.write_file(&repo, "new.txt", "x");
    let work = vec![tu("Write", json!({"file_path": p}), "u1"), g.say("done")];
    for phrase in ["do not push", "Don't push please", "NO PUSH", "keep it local only", "don't commit", "Do Not Commit"] {
        let recs = cat(&[&[human(&format!("first. {phrase}")), human("more")], &work]);
        assert_allow(&g.g10(&recs, &repo, json!({})));
    }
    let list = json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "local only"}]}});
    assert_allow(&g.g10(&cat(&[&[list, human("go")], &work]), &repo, json!({})));
    assert_deny(&g.g10(&cat(&[&[human("please push when done")], &work]), &repo, json!({})), "G10");
}

#[test]
fn test_g10_no_upstream_and_git_failures_are_quiet() {
    let g = G::new();
    let repo = g.make_repo(false);
    let p = g.write_file(&repo, "new.txt", "x");
    let recs = [human("x"), tu("Write", json!({"file_path": p}), "u1"), g.say("done")];
    // no upstream: unpushed counts as 0, but a dirty file of this turn still fires
    assert_deny(&g.g10(&recs, &repo, json!({})),
        "G10: this turn left work in work: 1 uncommitted file(s), 0 unpushed commit(s).");
    g.git(&repo, &["add", "-A"]);
    g.git(&repo, &["commit", "-q", "-m", "new"]);
    assert_allow(&g.g10(&recs, &repo, json!({}))); // clean, and nothing to compare with
    let commit = tu("Bash", json!({"command": "git commit -m \"new\""}), "u1");
    assert_allow(&g.g10(&[human("x"), commit, g.say("done")], &repo, json!({})));
    let plain = g.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_allow(&g.g10(&recs, &plain, json!({}))); // not a repo
    assert_allow(&g.g10(&recs, &g.join("missing"), json!({})));
    assert_allow(&g.stop_run(&recs, json!({}))); // no cwd at all
}

#[test]
fn test_g10_git_timeout_or_crash_fails_open() {
    let g = G::new();
    let repo = g.make_repo(true);
    let p = g.write_file(&repo, "new.txt", "x");
    let path = g.transcript(&[human("x"), tu("Write", json!({"file_path": p}), "u1"), g.say("done")]);
    let data = json!({"transcript_path": path, "cwd": repo});
    assert!(!g.run("stop", &data).2.is_empty()); // sanity: fires with a working git
    // a git that cannot start: an empty PATH (the Python test mocked subprocess.run)
    let empty = g.join("nopath");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(g.run_with("stop", &data, &[("PATH", &empty)]), (0, String::new(), String::new()));
}

#[test]
fn test_g10_switches() {
    let g = G::new();
    let repo = g.make_repo(true);
    let p = g.write_file(&repo, "new.txt", "x");
    let recs = [human("x"), tu("Write", json!({"file_path": p}), "u1"), g.say("done")];
    assert_deny(&g.g10(&recs, &repo, json!({})), "G10");
    assert_allow(&g.g10(&recs, &repo, json!({"stop_hook_active": true})));
    assert_allow(&g.stop_msg(&recs, json!({"cwd": repo}), &[("GUARD_OFF", "G10")]));
    assert_allow(&g.g10(&recs, &repo, json!({"agent_id": "a1"})));
    assert_allow(&g.run("stop", &json!({"hook_event_name": "Stop", "cwd": repo,
        "transcript_path": g.join("missing.jsonl")})));
}

// G11 clock check
#[test]
fn test_g11_long_turn_without_a_clock() {
    let g = G::new();
    let recs = [human_at(12 * 60 + 10, "go"), g.say("All done.")];
    assert_deny(&g.stop_run(&recs, json!({})),
        "G11: this turn ran 12 min. Rule 12: give the clock as a Vienna time (HH:MM) and one estimate with a range.");
    assert_allow(&g.stop_run(&[human_at(5 * 60, "go"), g.say("All done.")], json!({})));
    assert_allow(&g.stop_run(&[human("no stamp"), g.say("All done.")], json!({})));
}

#[test]
fn test_g11_a_clock_or_duration_in_the_reply_is_enough() {
    let g = G::new();
    let base = vec![human_at(20 * 60, "go")];
    for reply in ["Done at 14:05.", "That took 20 min.", "About 2 hours left.", "Roughly 1 h more.", "Estimate: 15-30 minutes."] {
        assert_allow(&g.stop_run(&cat(&[&base, &[g.say(reply)]]), json!({})), );
        assert_allow(&g.stop_run(&base, json!({"last_assistant_message": reply})));
    }
    assert_deny(&g.stop_run(&cat(&[&base, &[g.say("Done at 14:05.")]]), json!({"last_assistant_message": "Done."})), "G11");
    assert_deny(&g.stop_run(&cat(&[&base, &[g.say("Version 3 is out, 5 files changed.")]]), json!({})), "G11");
}

#[test]
fn test_g11_switches_and_missing_transcript() {
    let g = G::new();
    let recs = [human_at(15 * 60, "go"), g.say("Done.")];
    assert_allow(&g.stop_run(&recs, json!({"stop_hook_active": true})));
    assert_allow(&g.stop_msg(&recs, json!({}), &[("GUARD_OFF", "G11")]));
    assert_allow(&g.stop_run(&recs, json!({"agent_id": "a1"})));
    assert_allow(&g.run("stop", &json!({"hook_event_name": "Stop", "transcript_path": g.join("missing.jsonl")})));
    // the turn start is the LAST human prompt, not an earlier one
    assert_allow(&g.stop_run(&[human_at(60 * 60, "go"), g.say("a"), human_at(60, "go"), g.say("b")], json!({})));
    // a tool_result is not a prompt
    assert_deny(&g.stop_run(&[human_at(15 * 60, "go"), tool_result("t1"), g.say("b")], json!({})), "G11");
}

// G12 open steps check
fn plan(kw: Value) -> Value {
    let n = kw.as_object().map_or(0, |o| o.len());
    tu("mcp__ccd_turn__update_plan", kw, &format!("p{n}"))
}

fn items(specs: &[(&str, &str)]) -> Value {
    Value::Array(specs.iter().map(|(c, s)| json!({"content": c, "status": s})).collect())
}

#[test]
fn test_g12_open_steps_fire() {
    let g = G::new();
    let recs = [human("go"),
        plan(json!({"items": items(&[("Read brief", "in_progress"), ("Build", "pending"), ("Test", "pending"), ("Ship", "pending")])})),
        g.say("Working on it.")];
    assert_deny(&g.stop_run(&recs, json!({})), "G12: the task list still has open steps (Read brief, Build, Test). \
Finish them now, or end with what is done, what is left, and one question.");
}

#[test]
fn test_g12_replays_advance_done_and_start() {
    let g = G::new();
    let base = vec![human("go"), plan(json!({"items": items(&[("A", "in_progress"), ("B", "pending"), ("C", "pending")])}))];
    let run = |tail: Vec<Value>| g.stop_run(&cat(&[&base, &tail]), json!({}));
    let x = || g.say("x");
    assert_deny(&run(vec![plan(json!({"advance": true})), x()]), "steps (B, C)");
    assert_allow(&run(vec![plan(json!({"advance": true})), plan(json!({"advance": true})), plan(json!({"advance": true})), x()]));
    assert_deny(&run(vec![plan(json!({"done": [1]})), x()]), "steps (B, C)");
    assert_deny(&run(vec![plan(json!({"done": [1, 3]})), x()]), "steps (B)");
    assert_allow(&run(vec![plan(json!({"done": [1, 2, 3]})), x()]));
    assert_deny(&run(vec![plan(json!({"done": [1], "start": 3})), x()]), "steps (B, C)");
    assert_deny(&run(vec![plan(json!({"done": [1, 2], "start": 3})), x()]), "steps (C)");
    // an items call replaces the list
    assert_deny(&run(vec![plan(json!({"advance": true})), plan(json!({"items": items(&[("Z", "pending")])})), x()]), "steps (Z)");
    assert_allow(&run(vec![plan(json!({"items": items(&[("Z", "completed"), ("Y", "done")])})), x()]));
}

#[test]
fn test_g12_list_from_an_earlier_turn_still_counts() {
    let g = G::new();
    let recs = [human("go"), plan(json!({"items": items(&[("A", "in_progress"), ("B", "pending")])})), g.say("part"),
        human("continue"), g.say("hm")];
    assert_deny(&g.stop_run(&recs, json!({})), "steps (A, B)");
}

#[test]
fn test_g12_stays_quiet_when_asked_wrapped_or_questioned() {
    let g = G::new();
    let lst = || plan(json!({"items": items(&[("A", "in_progress"), ("B", "pending")])}));
    assert_allow(&g.stop_run(&[human("go"), lst(), tu("AskUserQuestion", json!({"questions": ["q"]}), "q1"), g.say("Waiting.")], json!({})));
    assert_allow(&g.stop_run(&[human("go"), lst(), tu("mcp__ccd_turn__wrap_up", json!({"recap": "r"}), "w1"), g.say("Done.")], json!({})));
    assert_allow(&g.stop_run(&[human("go"), lst(), g.say("Done A. B is left. Shall I go on?")], json!({})));
    assert_allow(&g.stop_run(&[human("go"), lst(), g.say("x")], json!({"last_assistant_message": "Shall I go on?  "})));
    // a wrap_up after the last update_plan closes every step, also from an earlier turn
    let wrapped = vec![human("go"), lst(), tu("mcp__ccd_turn__wrap_up", json!({"recap": "r"}), "w1"), g.say("ok"),
        human("thanks"), g.say("welcome")];
    assert_allow(&g.stop_run(&wrapped, json!({})));
    // but a later update_plan reopens the list
    let reopen = cat(&[&wrapped, &[plan(json!({"items": items(&[("C", "pending")])})), g.say("x")]]);
    assert_deny(&g.stop_run(&reopen, json!({})), "steps (C)");
    // an AskUserQuestion from an earlier turn does not count
    let old = [human("go"), tu("AskUserQuestion", json!({"questions": ["q"]}), "q1"), lst(), human("ok"), g.say("x")];
    assert_deny(&g.stop_run(&old, json!({})), "G12");
    assert_allow(&g.stop_run(&[human("go"), g.say("no plan at all")], json!({})));
}

#[test]
fn test_g12_quiet_while_a_helper_slot_is_live() {
    let g = G::new();
    let recs = [human("go"), plan(json!({"items": items(&[("A", "pending")])})), g.say("x")];
    assert_deny(&g.stop_run(&recs, json!({})), "G12"); // no slot: reminder
    let state = g.state_dir().join("s.json").to_string_lossy().into_owned();
    g.write(&state, &json!({"spawns": [{"id": "a1", "t": now_secs() - 30.0}]}).to_string());
    assert_allow(&g.stop_run(&recs, json!({}))); // live slot: a helper still runs
    g.write(&state, &json!({"spawns": [{"id": "a1", "t": now_secs() - 10.0 * 5400.0}]}).to_string());
    assert_deny(&g.stop_run(&recs, json!({})), "G12"); // expired slot does not count
    g.write(&state, "not json {{{");
    assert_deny(&g.stop_run(&recs, json!({})), "G12"); // broken state file = no slot
}

#[test]
fn test_g12_switches_and_missing_transcript() {
    let g = G::new();
    let recs = [human("go"), plan(json!({"items": items(&[("A", "pending")])})), g.say("x")];
    assert_deny(&g.stop_run(&recs, json!({})), "G12");
    assert_allow(&g.stop_run(&recs, json!({"stop_hook_active": true})));
    assert_allow(&g.stop_msg(&recs, json!({}), &[("GUARD_OFF", "G12")]));
    assert_allow(&g.stop_run(&recs, json!({"agent_id": "a1"})));
    assert_allow(&g.run("stop", &json!({"hook_event_name": "Stop", "transcript_path": g.join("missing.jsonl")})));
}

#[test]
fn test_stop_checks_run_in_order_and_only_one_fires() {
    let g = G::new();
    let repo = g.make_repo(true);
    let p = g.write_file(&repo, "new.txt", "x");
    let mut recs = vec![human_at(15 * 60, "go"), plan(json!({"items": items(&[("A", "pending")])})),
        tu("Write", json!({"file_path": p}), "u1"), g.say("x")];
    assert_deny(&g.g10(&recs, &repo, json!({})), "G10"); // G10 first
    assert_deny(&g.stop_run(&recs, json!({})), "G11"); // then G11
    recs[0] = human("go");
    assert_deny(&g.stop_run(&recs, json!({})), "G12"); // then G12
}

// --- fail-open ---
#[test]
fn test_fail_open() {
    let g = G::new();
    assert_allow(&g.run_raw("pretool", "not json {{{"));
    assert_allow(&g.run_raw("pretool", ""));
    assert_allow(&g.run_raw("pretool", "[1,2]"));
    assert_allow(&g.run_raw("prompt", "garbage"));
    assert_allow(&g.run("bogus", &json!({"tool_name": "Agent", "tool_input": {}})));
    assert_allow(&g.run("", &json!({})));
    let bad = json!({"tool_name": "AskUserQuestion", "transcript_path": g.join("missing.jsonl")});
    assert_allow(&g.run("pretool", &bad));
    assert_allow(&g.run("pretool", &json!({"tool_name": "Agent", "tool_input": "oops-string"})));
    assert_allow(&g.run("pretool", &json!({"tool_name": "Read", "tool_input": {}})));
}

#[test]
fn test_missing_transcript_means_zero_context() {
    let g = G::new();
    let d = json!({"tool_name": "Agent", "tool_input": {"model": "sonnet", "effort": "high", "prompt": BRIEF,
        "subagent_type": "worker"}, "transcript_path": "/no/such"});
    assert_allow(&g.run("pretool", &d));
    let d = json!({"tool_name": "ScheduleWakeup", "tool_input": {"delaySeconds": 60}, "transcript_path": "/no/such"});
    assert_allow(&g.run("pretool", &d));
}

// --- G13 to G15 tool checks and the G3 text ---
// G3 text
#[test]
fn test_g3_text_no_handoff_push() {
    let g = G::new();
    let (_, out, _) = g.prompt_run(350_000, "g3t");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["additionalContext"], NUDGE_350);
    let res = g.pre_ctx("Agent", ok_agent(), 600_000);
    assert_deny(&res, "context is 600K (> 350K)");
    assert!(!res.2.contains("handoff"), "{res:?}");
    assert!(!res.2.contains("fresh session"), "{res:?}");
}

// G14 sed -i
const G14_MSG: &str = "G14: BSD sed -i on macOS differs from GNU sed and breaks files. Use the Edit tool, or python3 for bulk edits.";

impl G {
    fn g14_data(&self, data: &Value, platform: &str, env: &[(&str, &str)]) -> Option<String> {
        let mut e = vec![("GUARD_PLATFORM", platform)];
        e.extend_from_slice(env);
        let r = self.run_with("pretool", data, &e);
        (r.0 == 2 && r.2.starts_with("G14")).then(|| r.2.trim().to_string())
    }

    fn g14(&self, cmd: &str, platform: &str, env: &[(&str, &str)]) -> Option<String> {
        self.g14_data(&json!({"tool_name": "Bash", "tool_input": {"command": cmd}}), platform, env)
    }
}

#[test]
fn test_g14_denies_in_place_forms() {
    let g = G::new();
    for cmd in [
        "sed -i 's/a/b/' f", "sed -i '' 's/a/b/' f", "sed -i.bak s/a/b/ f", "sed -ni 's/a/b/p' f",
        "sed -Ei 's/a/b/' f", "sed -i -E 's/a/b/' f", "sed --in-place s/a/b/ f", "sed --in-place=.bak s/a/b/ f",
        "sed -n -i p f", "sed -e s/a/b/ -i f", "sed s/a/b/ -i f", "/usr/bin/sed -i s/a/b/ f", "FOO=1 sed -i s/a/b/ f",
    ] {
        assert_eq!(g.g14(cmd, "darwin", &[]).as_deref(), Some(G14_MSG), "{cmd}");
    }
}

#[test]
fn test_g14_denies_after_separators_and_wrappers() {
    let g = G::new();
    for cmd in [
        "cd x && sed -i s/a/b/ f", "echo a; sed -i s/a/b/ f", "false || sed -i s/a/b/ f", "ls | sed -i s/a/b/",
        "echo f | xargs sed -i s/a/b/", "echo f | xargs -I {} sed -i s/a/b/ {}",
        "find . -name '*.md' -exec sed -i '' s/a/b/ {} \\;", "find . -name '*.md' -exec sed -i '' s/a/b/ {} +",
        "sudo sed -i s/a/b/ f", "sudo -u root sed -i s/a/b/ f", "env FOO=1 sed -i s/a/b/ f", "env -i sed -i s/a/b/ f",
        "bash -c \"sed -i s/a/b/ f\"", "echo a\nsed -i s/a/b/ f", "(cd x; sed -i s/a/b/ f)",
    ] {
        assert_eq!(g.g14(cmd, "darwin", &[]).as_deref(), Some(G14_MSG), "{cmd}");
    }
}

#[test]
fn test_g14_allows() {
    let g = G::new();
    for cmd in [
        "sed -n '1,5p' f", "sed -E 's/a/b/' f > g", "sed -En 's/a/b/p' f", "sed -e 's/-i/x/' f", "sed -s -n p f",
        "sed 's/a/b/' f", "sed -n -e 'p' f", "gsed -i s/a/b/ f", "echo \"sed -i\"", "rg 'sed -i' f", "grep sed -i f",
        "ls -i", "git commit -m \"no more sed -i here\"", "cat <<'EOF'\nsed -i foo\nEOF", "echo x | tee out.txt", "",
    ] {
        assert_eq!(g.g14(cmd, "darwin", &[]), None, "{cmd}");
    }
}

#[test]
fn test_g14_macos_only_off_switch_and_fail_open() {
    let g = G::new();
    assert_eq!(g.g14("sed -i s/a/b/ f", "linux", &[]), None);
    assert_eq!(g.g14("sed -i s/a/b/ f", "darwin", &[("GUARD_OFF", "G14")]), None);
    assert_eq!(g.g14("sed -i s/a/b/ f", "darwin", &[("GUARD_OFF", "G13")]).as_deref(), Some(G14_MSG));
    for data in [
        json!({"tool_name": "Bash"}),
        json!({"tool_name": "Bash", "tool_input": "oops"}),
        json!({"tool_name": "Bash", "tool_input": {"command": null}}),
        json!({"tool_name": "Bash", "tool_input": {"command": ["sed", "-i"]}}),
        json!({"tool_name": "Bash", "tool_input": {"command": "echo 'unbalanced sed -i"}}),
        json!({"tool_name": "Edit", "tool_input": {"command": "sed -i x"}}),
    ] {
        assert_eq!(g.g14_data(&data, "darwin", &[]), None, "{data}");
    }
}

#[test]
fn test_g14_through_pretool() {
    let g = G::new();
    // the Python test ran only on macOS; GUARD_PLATFORM makes the macOS rules reachable everywhere
    let run = |cmd: &str| {
        let data = json!({"hook_event_name": "PreToolUse", "session_id": format!("s{}", next_sid()),
            "tool_name": "Bash", "tool_input": {"command": cmd}});
        g.run_with("pretool", &data, &[("GUARD_PLATFORM", "darwin")])
    };
    assert_deny(&run("sed -i '' s/a/b/ f"), "G14: BSD sed -i");
    assert_allow(&run("sed -n 1p f"));
}

// G15 secret reads
const G15_FILE: &str = "G15: never read local.zsh: it holds API keys. Use `test -f` or `git check-ignore` for facts about it, or ask the user.";
const G15_SEARCH: &str = "G15: this search can reach local.zsh (API keys). Exclude it: `rg -g '!local.zsh' ...` or `grep --exclude=local.zsh ...`.";
const G15_PS: &str = "G15: this prints full command lines, which can hold API keys. Use `pgrep -f <name>` for pids, or `ps -o pid=,etime=,ucomm= -p <pid>`.";

impl G {
    fn g15x(&self, tool: &str, ti: Value, env: &[(&str, &str)], platform: Option<&str>, extra: Value) -> Option<String> {
        let data = merge(json!({"tool_name": tool, "tool_input": ti}), extra);
        let mut e: Vec<(&str, &str)> = Vec::new();
        if let Some(p) = platform {
            e.push(("GUARD_PLATFORM", p));
        }
        e.extend_from_slice(env);
        let r = self.run_with("pretool", &data, &e);
        (r.0 == 2 && r.2.starts_with("G15")).then(|| r.2.trim().to_string())
    }

    fn g15(&self, tool: &str, ti: Value) -> Option<String> {
        self.g15x(tool, ti, &[], None, json!({}))
    }

    fn g15bash(&self, cmd: &str) -> Option<String> {
        self.g15("Bash", json!({"command": cmd}))
    }

    fn g15bash_on(&self, cmd: &str, platform: &str) -> Option<String> {
        self.g15x("Bash", json!({"command": cmd}), &[], Some(platform), json!({}))
    }

    fn g15bash_in(&self, cmd: &str, cwd: &str) -> Option<String> {
        self.g15x("Bash", json!({"command": cmd}), &[], None, json!({"cwd": cwd}))
    }

    /// A temp HOME whose ~/.config/zsh links into a dotfiles repo, like the real machine. Returns the repo.
    fn secret_tree(&self) -> String {
        let repo = self.real_dir().join("dotfiles");
        let zsh = repo.join("home/.config/zsh");
        std::fs::create_dir_all(&zsh).unwrap();
        for name in ["local.zsh", "aliases.zsh"] {
            std::fs::write(zsh.join(name), "export KEY=1\n").unwrap();
        }
        std::fs::create_dir_all(self.dir.join(".config")).unwrap();
        symlink(&zsh.to_string_lossy(), &self.join(".config/zsh"));
        repo.to_string_lossy().into_owned()
    }
}

#[test]
fn test_g15_read_tool() {
    let g = G::new();
    let zsh = g.join(".config/zsh");
    std::fs::create_dir_all(&zsh).unwrap();
    let target = format!("{zsh}/local.zsh");
    g.write(&target, "export KEY=1\n");
    let link = g.join("link.zsh");
    symlink(&target, &link);
    for path in ["~/.config/zsh/local.zsh", target.as_str(), link.as_str(), &format!("{zsh}/../zsh/local.zsh")] {
        assert_eq!(g.g15("Read", json!({"file_path": path})).as_deref(), Some(G15_FILE), "{path}");
    }
    for path in ["~/.config/zsh/aliases.zsh", &g.join("README.md"), "/etc/hosts", ""] {
        assert_eq!(g.g15("Read", json!({"file_path": path})), None, "{path}");
    }
    assert_eq!(g.g15x("Read", json!({"file_path": "local.zsh"}), &[], None, json!({"cwd": zsh})).as_deref(), Some(G15_FILE));
    assert_eq!(g.g15x("Read", json!({"file_path": "local.zsh"}), &[], None, json!({"cwd": g.d()})), None);
}

#[test]
fn test_g15_bash_readers_denied() {
    let g = G::new();
    for cmd in [
        "cat ~/.config/zsh/local.zsh", "cat \"$HOME/.config/zsh/local.zsh\"", "cat ${HOME}/.config/zsh/local.zsh",
        "cat ~/.config/zsh/../zsh/local.zsh", "bat ~/.config/zsh/local.zsh", "less ~/.config/zsh/local.zsh",
        "head -5 ~/.config/zsh/local.zsh", "tail -n 3 ~/.config/zsh/local.zsh", "rg KEY ~/.config/zsh/local.zsh",
        "grep -n KEY ~/.config/zsh/local.zsh", "sed -n 1p ~/.config/zsh/local.zsh",
        "awk '{print}' ~/.config/zsh/local.zsh", "cut -d= -f2 ~/.config/zsh/local.zsh",
        "strings ~/.config/zsh/local.zsh", "xxd ~/.config/zsh/local.zsh", "cp ~/.config/zsh/local.zsh /tmp/x",
        "open ~/.config/zsh/local.zsh", "cd ~ && cat .config/zsh/local.zsh", "cd ~/.config/zsh && cat local.zsh",
        "cat ~/.config/zsh/local.z*", "echo x | cat - ~/.config/zsh/local.zsh",
        "xargs cat < ~/.config/zsh/local.zsh", "while read l; do echo $l; done < ~/.config/zsh/local.zsh",
        "find ~/.config/zsh -name local.zsh -exec cat {} \\;", "sudo cat ~/.config/zsh/local.zsh",
    ] {
        assert_eq!(g.g15bash(cmd).as_deref(), Some(G15_FILE), "{cmd}");
    }
    assert_eq!(g.g15bash("rg KEY ~/.config/zsh").as_deref(), Some(G15_SEARCH));
}

#[test]
fn test_g15_bash_allowed() {
    let g = G::new();
    for cmd in [
        "ls ~/.config/zsh/local.zsh", "ls -la ~/.config/zsh", "test -f ~/.config/zsh/local.zsh",
        "[ -f ~/.config/zsh/local.zsh ] && echo yes", "stat ~/.config/zsh/local.zsh", "cat ~/.config/zsh/aliases.zsh",
        "cat README.md", "rg KEY ~/dotfiles", "head -5 /tmp/local.txt", "echo \"cat ~/.config/zsh/local.zsh\"",
        "rg 'local.zsh' ~/dotfiles/docs", "git add home/.config/zsh/aliases.zsh", "echo hi >> ~/.config/zsh/local.zsh", "",
    ] {
        assert_eq!(g.g15bash(cmd), None, "{cmd}");
    }
}

#[test]
fn test_g15_process_command_lines_denied() {
    let g = G::new();
    for cmd in [
        "ps aux", "ps auxww", "ps aux | grep node", "ps -ef", "ps -ef | grep x", "ps ax", "ps -e", "ps -A",
        "ps -f -p 12", "ps -o args", "ps -o command -p 1", "ps -p 1 -o pid,command", "ps -p 1 -o pid,args=",
        "ps -p 1 -opid,args", "ps -axo pid,command", "ps -eo pid,args", "ps --format pid,args", "ps -ww",
        "ps -p 1 -ww", "ps eww", "ps eww -p 1", "/bin/ps aux", "sudo ps aux", "pgrep -fl node", "pgrep -af node",
        "pgrep -lf node", "pgrep -f -l node", "pgrep -f -a node", "pgrep --list-full node", "procs", "procs --tree",
        "echo x; procs",
    ] {
        assert_eq!(g.g15bash(cmd).as_deref(), Some(G15_PS), "{cmd}");
    }
}

#[test]
fn test_g15_process_checks_allowed() {
    let g = G::new();
    for cmd in [
        "pgrep -f node", "pgrep node", "pgrep -l node", "pgrep -u me -f node", "pkill -f node",
        "ps -p 123 -o pid,etime,stat,comm", "ps -o pid,etime,comm -p 123", "ps -p 123 -o etime=", "ps -axo pid,comm",
        "ps -e -o pid,comm", "ps -p 123", "ps -u me -o pid,comm", "ps --help", "echo \"ps aux\"", "rg 'ps aux' docs",
        "kill -0 123", "lsof -p 123",
    ] {
        assert_eq!(g.g15bash_on(cmd, "linux"), None, "{cmd}"); // Linux behaviour is unchanged
    }
}

#[test]
fn test_g15_recursive_searches_denied() {
    let g = G::new();
    let repo = g.secret_tree();
    for cmd in [
        "grep -rn KEY .", "grep -R KEY", "grep --recursive KEY .", "grep -d recurse KEY .", "grep -drecurse KEY .",
        "grep --directories=recurse KEY .", "grep -r -e KEY .", "grep -rn KEY home", "egrep -rn KEY .",
        "fgrep -r KEY ~", "grep -rn KEY ~/dotfiles", "grep -rl KEY ..", "grep -r --exclude=other.txt KEY .",
        "grep -r --exclude-dir=docs KEY .", "ack KEY", "ack KEY .", "rg -u KEY .", "rg -uu KEY", "rg -uuu KEY .",
        "rg -nu KEY .", "rg --no-ignore KEY .", "rg --no-ignore-vcs KEY", "rg --hidden KEY .", "rg -. KEY .",
        "rg -n --hidden KEY home", "rg -u -g '!other' KEY .", "ag -u KEY", "ag --unrestricted KEY", "ag -U KEY .",
        "ag --skip-vcs-ignores KEY .", "ag --hidden KEY .", "sudo rg -u KEY .", "cd .. && grep -rn KEY dotfiles",
    ] {
        assert_eq!(g.g15bash_in(cmd, &repo).as_deref(), Some(G15_SEARCH), "{cmd}");
    }
}

#[test]
fn test_g15_searches_allowed() {
    let g = G::new();
    let repo = g.secret_tree();
    for cmd in [
        "rg KEY .", "rg -n x claude/", "rg KEY ~", "rg -u KEY claude/", "rg --hidden -n x claude/", "ag KEY .",
        "grep -n x file", "grep KEY .", "grep -r KEY claude/", "grep -rn x claude/hooks",
        "grep -r --include='*.py' KEY .", "sed -n 1,5p f", "ps -o pid=,ucomm= -p 1", "test -f ~/.config/zsh/local.zsh",
        "rg -u -g '!local.zsh' KEY .", "rg -u -g '!**/local.zsh' KEY", "rg -u --glob=!**/local.zsh KEY .",
        "rg --hidden --glob '!*.zsh' KEY .", "rg -u -g '!zsh/' KEY .", "ag -u -g '!local.zsh' KEY .",
        "ag -u --ignore local.zsh KEY .", "grep -rn --exclude=local.zsh KEY .", "grep -rn --exclude local.zsh KEY .",
        "grep -r --exclude='*.zsh' KEY .", "grep -rn --exclude-dir=zsh KEY .", "ack --ignore-dir=zsh KEY",
    ] {
        assert_eq!(g.g15bash_in(cmd, &repo), None, "{cmd}");
    }
}

#[test]
fn test_g15_ps_on_macos() {
    let g = G::new();
    for cmd in ["ps -o pid=,ucomm= -p 1", "ps -c -p 1", "ps -ax -o pid,comm", "ps -o pid,etime,comm -p 123",
        "ps -p 1 -o etime=", "ps --help"]
    {
        assert_eq!(g.g15bash_on(cmd, "darwin"), None, "{cmd}");
    }
    for cmd in ["ps", "ps -p 1", "ps -U me", "ps -ux", "ps -j", "ps -v", "ps -O ppid -p 1", "ps -c -o args -p 1"] {
        assert_eq!(g.g15bash_on(cmd, "darwin").as_deref(), Some(G15_PS), "{cmd}");
    }
    for cmd in ["ps -p 1", "ps -U me", "ps -O ppid -p 1"] {
        assert_eq!(g.g15bash_on(cmd, "linux"), None, "{cmd}");
    }
}

#[test]
fn test_g15_grep_tool() {
    let g = G::new();
    let repo = g.secret_tree();
    let zsh = format!("{repo}/home/.config/zsh");
    let in_repo = |ti: Value| g.g15x("Grep", ti, &[], None, json!({"cwd": repo}));
    for path in ["~/.config/zsh", "~/.config/zsh/local.zsh", zsh.as_str(), &format!("{zsh}/local.zsh"), &g.join(".config/zsh")] {
        assert_eq!(g.g15("Grep", json!({"pattern": "KEY", "path": path})).as_deref(), Some(G15_FILE), "{path}");
    }
    assert_eq!(g.g15x("Grep", json!({"pattern": "KEY", "path": "local.zsh"}), &[], None, json!({"cwd": zsh})).as_deref(),
        Some(G15_FILE));
    let others = [json!(repo), json!("~"), json!(g.d()), json!("~/.config"), json!(format!("{zsh}/aliases.zsh")),
        json!(""), Value::Null, json!(5)];
    for path in others {
        assert_eq!(in_repo(json!({"pattern": "KEY", "path": path})), None, "{path}");
    }
    assert_eq!(in_repo(json!({"pattern": "KEY"})), None);
    assert_eq!(g.g15x("Grep", json!({"pattern": "KEY", "path": "~/.config/zsh"}), &[("GUARD_OFF", "G15")], None, json!({})), None);
}

#[test]
fn test_g15_grep_tool_is_in_the_hook_matcher() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../claude/settings.json");
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(root).unwrap()).unwrap();
    let hooks = settings["hooks"]["PreToolUse"].as_array().unwrap();
    let matchers: Vec<&str> = hooks
        .iter()
        .filter(|h| h.to_string().contains("guard pretool"))
        .map(|h| h["matcher"].as_str().unwrap_or(""))
        .collect();
    assert!(!matchers.is_empty() && matchers.iter().all(|m| m.contains("|Read|Grep")), "{matchers:?}");
}

#[test]
fn test_g15_letter_case_on_macos() {
    let g = G::new();
    g.secret_tree();
    // The deny needs a case-insensitive file system (macOS default): on a case-sensitive
    // one (Linux) ~/.config/ZSH is another folder and the check has nothing to resolve.
    if g.dir.join(".CONFIG").exists() {
        for path in ["~/.config/ZSH/LOCAL.zsh", "~/.config/zsh/Local.ZSH"] {
            assert_eq!(g.g15x("Read", json!({"file_path": path}), &[], Some("darwin"), json!({})).as_deref(), Some(G15_FILE), "{path}");
            assert_eq!(g.g15bash_on(&format!("cat {path}"), "darwin").as_deref(), Some(G15_FILE), "{path}");
        }
        assert_eq!(g.g15bash_on("cat ~/.config/zsh/LOCAL.z*", "darwin").as_deref(), Some(G15_FILE));
        assert_eq!(g.g15x("Grep", json!({"path": "~/.config/ZSH"}), &[], Some("darwin"), json!({})).as_deref(), Some(G15_FILE));
    }
    assert_eq!(g.g15x("Read", json!({"file_path": "~/.config/ZSH/LOCAL.zsh"}), &[], Some("linux"), json!({})), None);
    assert_eq!(g.g15bash_on("cat ~/.config/ZSH/LOCAL.zsh", "linux"), None);
    assert_eq!(g.g15x("Read", json!({"file_path": "~/.config/ZSH/ALIASES.zsh"}), &[], Some("darwin"), json!({})), None);
}

#[test]
fn test_g15_ln_install_and_dd() {
    let g = G::new();
    g.secret_tree();
    for cmd in [
        "ln -s ~/.config/zsh/local.zsh /tmp/x", "ln ~/.config/zsh/local.zsh /tmp/x",
        "install ~/.config/zsh/local.zsh /tmp/x", "dd if=~/.config/zsh/local.zsh",
        "dd if=$HOME/.config/zsh/local.zsh of=/dev/null", "dd of=/tmp/x if=~/.config/zsh/local.zsh bs=1",
    ] {
        assert_eq!(g.g15bash(cmd).as_deref(), Some(G15_FILE), "{cmd}");
    }
    for cmd in ["dd if=/dev/zero of=/tmp/x count=1", "dd of=~/.config/zsh/local.zsh if=/dev/zero count=1",
        "ln -s ~/.config/zsh/aliases.zsh /tmp/x"]
    {
        assert_eq!(g.g15bash(cmd), None, "{cmd}");
    }
}

#[test]
fn test_g15_off_switch_and_fail_open() {
    let g = G::new();
    let bash = |cmd: &str, off: &str| g.g15x("Bash", json!({"command": cmd}), &[("GUARD_OFF", off)], None, json!({}));
    assert_eq!(bash("ps aux", "G15"), None);
    assert_eq!(bash("cat ~/.config/zsh/local.zsh", "G14 G15"), None);
    assert_eq!(g.g15x("Read", json!({"file_path": "~/.config/zsh/local.zsh"}), &[("GUARD_OFF", "g15")], None, json!({})), None);
    assert_eq!(bash("ps aux", "G14").as_deref(), Some(G15_PS));
    for (tool, ti) in [
        ("Bash", Value::Null),
        ("Bash", json!("oops")),
        ("Bash", json!({})),
        ("Bash", json!({"command": 5})),
        ("Bash", json!({"command": "echo 'unbalanced ps aux"})),
        ("Read", json!({})),
        ("Read", json!({"file_path": 7})),
        ("Read", json!("x")),
        ("Write", json!({"file_path": "~/.config/zsh/local.zsh"})),
    ] {
        assert_eq!(g.g15(tool, ti.clone()), None, "{tool} {ti}");
    }
    // The Python test also made os.path.realpath raise; the binary has no such seam (see PORTED.md).
}

#[test]
fn test_g15_through_pretool() {
    let g = G::new();
    assert_deny(&g.pre("Read", json!({"file_path": "~/.config/zsh/local.zsh"})), "G15: never read local.zsh");
    assert_deny(&g.pre("Bash", json!({"command": "ps aux"})), "G15:");
    assert_allow(&g.pre("Read", json!({"file_path": "/etc/hosts"})));
    assert_allow(&g.pre("Bash", json!({"command": "pgrep -f node"})));
    let data = json!({"tool_name": "Bash", "tool_input": {"command": "ps aux"}});
    let res = g.run_with("pretool", &data, &[("GUARD_OFF", "G15")]);
    assert_eq!(res, (0, String::new(), String::new()));
}

// G13 silence note
fn note13() -> Value {
    json!({"hookSpecificOutput": {"hookEventName": "PostToolUse",
        "additionalContext": "G13: 8 tool calls with no text. Write one short status line for the user now, then go on."}})
}

fn tu13(i: usize) -> Value {
    json!({"type": "assistant", "message": {"role": "assistant",
        "content": [{"type": "tool_use", "id": format!("u{i}"), "name": "Bash", "input": {}}]}})
}

fn say13(text: &str) -> Value {
    json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}})
}

fn calls(start: usize, end: usize) -> Vec<Value> {
    (start..=end).flat_map(|i| [tu13(i), tool_result(&format!("u{i}"))]).collect()
}

impl G {
    fn post_env(&self, records: &[Value], last: Option<usize>, extra: Value, env: &[(&str, &str)]) -> R {
        let path = self.transcript(records);
        let mut data = json!({"hook_event_name": "PostToolUse", "session_id": "p1", "tool_name": "Bash",
            "transcript_path": path});
        if let Some(l) = last {
            data["tool_use_id"] = json!(format!("u{l}"));
        }
        self.run_with("posttool", &merge(data, extra), env)
    }

    fn post(&self, records: &[Value], last: Option<usize>) -> R {
        self.post_env(records, last, json!({}), &[])
    }
}

#[test]
fn test_g13_fires_at_exactly_eight() {
    let g = G::new();
    for (n, fires) in [(7, false), (8, true), (9, false), (20, false)] {
        let res = g.post(&cat(&[&[human("hi")], &calls(1, n)]), Some(n));
        if fires {
            assert_eq!((res.0, res.2.as_str()), (0, ""));
            assert_eq!(json_out(&res), note13());
            assert_eq!(res.1.matches('\n').count(), 1);
        } else {
            assert_allow(&res);
        }
    }
}

#[test]
fn test_g13_off_in_the_desktop_app() {
    let g = G::new();
    let recs = cat(&[&[human("hi")], &calls(1, 8)]);
    let res = g.post(&recs, Some(8)); // no entrypoint env: the note fires
    assert_eq!(json_out(&res), note13());
    for (entry, fires) in [("claude-desktop", false), ("cli", true), ("", true)] {
        let res = g.post_env(&recs, Some(8), json!({}), &[("CLAUDE_CODE_ENTRYPOINT", entry)]);
        if fires {
            assert_eq!(json_out(&res), note13(), "{entry}");
        } else {
            assert_allow(&res);
        }
    }
}

#[test]
fn test_g13_without_tool_use_id_counts_all() {
    let g = G::new();
    let res = g.post(&cat(&[&[human("hi")], &calls(1, 8)]), None);
    assert_eq!(json_out(&res), note13());
    assert_allow(&g.post(&cat(&[&[human("hi")], &calls(1, 9)]), None));
}

#[test]
fn test_g13_parallel_calls_each_see_their_own_count() {
    let g = G::new();
    let recs = cat(&[&[human("hi")], &calls(1, 10)]);
    let fired: Vec<usize> = (1..=10).filter(|i| !g.post(&recs, Some(*i)).1.is_empty()).collect();
    assert_eq!(fired, vec![8]);
    // a text written after the current call does not reset its count
    let recs = cat(&[&[human("hi")], &calls(1, 8), &[say13("Now checking the results")]]);
    assert_eq!(json_out(&g.post(&recs, Some(8))), note13());
}

#[test]
fn test_g13_text_resets_and_new_streak_starts() {
    let g = G::new();
    let talk = [say13("Working on the parser now")];
    let recs = cat(&[&[human("hi")], &calls(1, 5), &talk, &calls(6, 10)]);
    assert_allow(&g.post(&recs, Some(10))); // 5 since the text
    let recs = cat(&[&[human("hi")], &calls(1, 5), &talk, &calls(6, 13)]);
    assert_eq!(json_out(&g.post(&recs, Some(13))), note13()); // 8 since the text
    // text in the same assistant message as a tool call counts as text
    let mixed = json!({"type": "assistant", "message": {"role": "assistant", "content": [
        {"type": "text", "text": "Reading the config files"},
        {"type": "tool_use", "id": "u6", "name": "Bash", "input": {}}]}});
    let recs = cat(&[&[human("hi")], &calls(1, 5), &[mixed, tool_result("u6")], &calls(7, 11)]);
    assert_allow(&g.post(&recs, Some(11))); // u6 shares a message with the text: 6 calls since it
}

#[test]
fn test_g13_empty_text_does_not_reset() {
    let g = G::new();
    for filler in [" ", "\n", "...", ""] {
        let recs = cat(&[&[human("hi")], &calls(1, 4), &[say13(filler)], &calls(5, 8)]);
        assert_eq!(json_out(&g.post(&recs, Some(8))), note13(), "{filler:?}");
    }
    let thinking = json!({"type": "assistant", "message": {"role": "assistant", "content": [
        {"type": "thinking", "thinking": "Let me think about this a lot"}]}});
    let recs = cat(&[&[human("hi")], &calls(1, 4), &[thinking], &calls(5, 8)]);
    assert_eq!(json_out(&g.post(&recs, Some(8))), note13());
}

#[test]
fn test_g13_stops_at_last_human_prompt() {
    let g = G::new();
    let recs = cat(&[&calls(1, 6), &[human("go on")], &calls(7, 10)]);
    assert_allow(&g.post(&recs, Some(10))); // only 4 since the prompt
    let recs = cat(&[&calls(1, 6), &[human("go on")], &calls(7, 14)]);
    assert_eq!(json_out(&g.post(&recs, Some(14))), note13());
}

#[test]
fn test_g13_main_thread_only_and_off_switch() {
    let g = G::new();
    let recs = cat(&[&[human("hi")], &calls(1, 8)]);
    assert_allow(&g.post_env(&recs, Some(8), json!({"agent_id": "a1"}), &[]));
    let path = g.join("x/subagents/agent-a1.jsonl");
    g.lines(&path, &recs);
    let data = json!({"tool_name": "Bash", "tool_use_id": "u8", "transcript_path": path});
    assert_allow(&g.run("posttool", &data));
    let res = g.post_env(&recs, Some(8), json!({}), &[("GUARD_OFF", "G13")]);
    assert_eq!(res, (0, String::new(), String::new()));
}

#[test]
fn test_g13_fail_open() {
    let g = G::new();
    assert_allow(&g.run_raw("posttool", "not json {{{"));
    assert_allow(&g.run_raw("posttool", ""));
    assert_allow(&g.run_raw("posttool", "[1]"));
    assert_allow(&g.run("posttool", &json!({"tool_name": "Bash"})));
    assert_allow(&g.run("posttool", &json!({"tool_name": "Bash", "transcript_path": "/no/such"})));
    let path = g.join("bad.jsonl");
    g.write(&path, &"garbage\n{not json\n".repeat(5));
    assert_allow(&g.run("posttool", &json!({"tool_name": "Bash", "transcript_path": path})));
    assert_allow(&g.post_env(&cat(&[&[human("hi")], &calls(1, 8)]), Some(8), json!({"transcript_path": 5}), &[]));
    // The Python test also called guard.posttool directly and mocked reverse_lines; here the same
    // inputs go through the binary (a missing transcript above) and the mock has no counterpart.
    assert_allow(&g.run("posttool", &json!({"transcript_path": "/no/such", "tool_use_id": "u8"})));
}

#[test]
fn ported_map_complete() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let map = fs::read_to_string(root.join("tests/PORTED.md")).expect("tests/PORTED.md");
    let repo = root.join("../..");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut problems: Vec<String> = Vec::new();
    for line in map.lines().filter(|l| l.starts_with('|')) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 || !cells[0].contains("::") {
            continue; // the header and the separator
        }
        let (py_file, py_name) = cells[0].split_once("::").unwrap();
        *counts.entry(py_file.to_string()).or_default() += 1;
        if !seen.insert(cells[0].to_string()) {
            problems.push(format!("{}: listed twice", cells[0]));
        }
        // The Python test must exist while its file does (the Python files are deleted later).
        if let Ok(py_src) = fs::read_to_string(repo.join(py_file)) {
            let def = py_name.rsplit("::").next().unwrap();
            if !py_src.contains(&format!("def {def}(")) {
                problems.push(format!("{}: no `def {}` in {}", cells[0], def, py_file));
            }
        }
        if cells[1] == "-" {
            if cells[2].is_empty() {
                problems.push(format!("{}: a `-` row needs a note", cells[0]));
            }
            continue;
        }
        let Some((rs_file, rs_name)) = cells[1].rsplit_once("::") else {
            problems.push(format!("{}: Rust cell `{}` is not file::name", cells[0], cells[1]));
            continue;
        };
        // Only the file the row names counts: a function of the same name elsewhere is not a twin.
        let Ok(source) = fs::read_to_string(root.join(rs_file)) else {
            problems.push(format!("{}: {} does not exist", cells[0], rs_file));
            continue;
        };
        let re = regex::Regex::new(&format!(r"(?m)^\s*(?:pub\s+)?fn {}\b", regex::escape(rs_name))).unwrap();
        let Some(m) = re.find(&source) else {
            problems.push(format!("{}: no `fn {}` in {}", py_name, rs_name, rs_file));
            continue;
        };
        // It must be a real test: `#[test]` among the attributes right above it, and no `#[ignore]`.
        let attrs: Vec<&str> = source[..m.start()]
            .lines()
            .rev()
            .map(str::trim)
            .take_while(|l| l.starts_with("#["))
            .collect();
        if !attrs.iter().any(|a| *a == "#[test]") {
            problems.push(format!("{}: `fn {}` in {} has no #[test]", cells[0], rs_name, rs_file));
        }
        if attrs.iter().any(|a| a.starts_with("#[ignore")) {
            problems.push(format!("{}: `fn {}` in {} is #[ignore]d", cells[0], rs_name, rs_file));
        }
    }
    for (file, want) in REQUIRED {
        let got = counts.get(file).copied().unwrap_or(0);
        if got != want {
            problems.push(format!("{file}: {got} rows, expected {want}"));
        }
    }
    assert!(problems.is_empty(), "PORTED.md problems:\n{}", problems.join("\n"));
}
