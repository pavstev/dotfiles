//! Start-up latency of the hook bins that run on every Claude Code tool call.
//!
//! Each case spawns one bin with a stdin JSON payload, discards its output, waits, and times the
//! whole spawn. Cases: `guard pretool|posttool|prompt`, `statusline`, `pace status`,
//! `guard_cursor tool` and `agent-config --help`. The launcher script (`claude/hooks/rust-bin`)
//! looks for `target/release`, not the bench profile, so its overhead is measured by hand
//! (see BENCHMARK.md).
//!
//! Run: `cargo bench --bench hooks`
//!
//! Knobs (env): `BENCH_RUNS=<n>` timed runs per case (default 50, after 5 warmup runs),
//! `BENCH_FILTER=<text>` runs only the cases whose name contains the text.
//!
//! Hermetic: HOME, TMPDIR, XDG_CACHE_HOME and CLAUDE_CONFIG_DIR all point into a temp folder
//! that holds about 200 small transcripts and one ~5 MB transcript. Nothing in the real `$HOME`
//! is read or written. The pace cache is warm after the warmup runs, as in a live session.

use serde_json::json;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const WARMUP: usize = 5;
const DEFAULT_RUNS: usize = 50;
const SMALL_FILES: usize = 200;
const BIG_BYTES: usize = 5 * 1024 * 1024;

struct Case {
    name: &'static str,
    bin: &'static str,
    args: &'static [&'static str],
    stdin: String,
}

/// ISO UTC time (`2026-10-20T12:00:00.000Z`) from epoch seconds.
fn iso(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// One synthetic record pair (a user line and an assistant line), like the ones Claude Code writes.
fn push_pair(out: &mut String, i: usize, t: i64) {
    let user = json!({
        "type": "user", "timestamp": iso(t), "uuid": format!("u-{i}"), "sessionId": "bench",
        "message": {"role": "user", "content": format!("please look at file number {i} and tell me what the function does, then fix the failing test in it")},
    });
    let asst = json!({
        "type": "assistant", "timestamp": iso(t + 1), "uuid": format!("a-{i}"), "sessionId": "bench",
        "message": {
            "id": format!("msg_{i}"), "role": "assistant", "model": "claude-opus-5-5",
            "content": [{"type": "text", "text": format!("The function in file {i} reads the config, merges the defaults and returns the result. The test fails because the default is applied twice.")}],
            "usage": {"input_tokens": 10, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 5000, "output_tokens": 50},
        },
    });
    let _ = writeln!(out, "{user}");
    let _ = writeln!(out, "{asst}");
}

fn write_transcripts(config: &Path, now: i64) -> std::io::Result<std::path::PathBuf> {
    let dir = config.join("projects").join("bench-proj");
    fs::create_dir_all(&dir)?;
    for f in 0..SMALL_FILES {
        let mut s = String::new();
        for i in 0..5 {
            push_pair(&mut s, i, now - 600 + i as i64 * 2);
        }
        fs::write(dir.join(format!("small-{f:03}.jsonl")), s)?;
    }
    let mut big = String::new();
    let mut i = 0;
    while big.len() < BIG_BYTES {
        push_pair(&mut big, i, now - 3600 + i as i64 % 3000);
        i += 1;
    }
    let path = dir.join("big.jsonl");
    fs::write(&path, big)?;
    Ok(path)
}

fn cases(tmp: &Path, transcript: &Path) -> Vec<Case> {
    let tp = transcript.to_string_lossy().into_owned();
    let cwd = tmp.to_string_lossy().into_owned();
    let hook = |event: &str, tool: &str, input: serde_json::Value| {
        json!({"session_id": "bench", "transcript_path": tp, "cwd": cwd, "hook_event_name": event, "tool_name": tool, "tool_input": input})
    };
    let bash = json!({"command": "git status --short && cargo test 2>&1 | tail -5"});
    let agent_prompt = "Why: the build is slow.\nWhat: measure the start-up of the hook bins.\nHow: run the bench and read the table.\n\
        Deadline: done in 20 min. Hand back at most 4,000 chars; write the rest to a file and cite it.";
    let agent = json!({"description": "bench", "subagent_type": "worker", "prompt": agent_prompt});
    let mut post = hook("PostToolUse", "Bash", bash.clone());
    post["tool_response"] = json!({"stdout": "ok\n", "stderr": "", "interrupted": false});
    let mut prompt = hook("UserPromptSubmit", "", json!({}));
    prompt["prompt"] = json!("fix the failing test in the parser");
    let status = json!({
        "session_id": "bench", "transcript_path": tp, "cwd": cwd,
        "workspace": {"current_dir": cwd},
        "model": {"id": "claude-opus-5-5", "display_name": "Opus 5.5"},
        "context_window": {
            "total_input_tokens": 120_000, "used_percentage": 12,
            "current_usage": {"input_tokens": 10, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 60_000, "output_tokens": 50},
        },
        "rate_limits": {
            "five_hour": {"used_percentage": 23.5, "resets_at": 1_900_000_000},
            "seven_day": {"used_percentage": 41.2, "resets_at": 1_900_400_000},
        },
    });
    let cursor = json!({"conversation_id": "bench", "hook_event_name": "preToolUse", "tool_name": "Shell", "tool_input": {"command": "git status"}});
    let guard = env!("CARGO_BIN_EXE_guard");
    vec![
        Case { name: "guard pretool Bash", bin: guard, args: &["pretool"], stdin: hook("PreToolUse", "Bash", bash).to_string() },
        Case { name: "guard pretool Agent", bin: guard, args: &["pretool"], stdin: hook("PreToolUse", "Agent", agent).to_string() },
        Case { name: "guard posttool", bin: guard, args: &["posttool"], stdin: post.to_string() },
        Case { name: "guard prompt", bin: guard, args: &["prompt"], stdin: prompt.to_string() },
        Case { name: "statusline", bin: env!("CARGO_BIN_EXE_statusline"), args: &[], stdin: status.to_string() },
        Case { name: "pace status", bin: env!("CARGO_BIN_EXE_pace"), args: &["status"], stdin: String::new() },
        Case { name: "guard_cursor tool", bin: env!("CARGO_BIN_EXE_guard_cursor"), args: &["tool"], stdin: cursor.to_string() },
        Case { name: "agent-config --help", bin: env!("CARGO_BIN_EXE_agent-config"), args: &["--help"], stdin: String::new() },
    ]
}

/// One timed spawn in ms, or None when the spawn itself failed. The exit code is ignored
/// (the guard exits 2 on a block).
fn run_once(case: &Case, tmp: &Path, config: &Path, cache: &Path) -> Option<f64> {
    let t = Instant::now();
    let mut child = Command::new(case.bin)
        .args(case.args)
        .current_dir(tmp)
        .env("HOME", tmp)
        .env("USERPROFILE", tmp)
        .env("TMPDIR", tmp)
        .env("TMP", tmp)
        .env("TEMP", tmp)
        .env("XDG_CACHE_HOME", cache)
        .env("CLAUDE_CONFIG_DIR", config)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(case.stdin.as_bytes()); // a bin may exit before it reads
    }
    child.wait().ok()?;
    Some(t.elapsed().as_secs_f64() * 1000.0)
}

fn main() {
    let runs = std::env::var("BENCH_RUNS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0).unwrap_or(DEFAULT_RUNS);
    let filter = std::env::var("BENCH_FILTER").unwrap_or_default();

    let world = tempfile::tempdir().expect("temp folder");
    let tmp = world.path().join("tmp");
    let config = world.path().join("claude");
    let cache = world.path().join("cache");
    for d in [&tmp, &config, &cache] {
        fs::create_dir_all(d).expect("create folder");
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let big = write_transcripts(&config, now).expect("write transcripts");

    println!("hook latency, {runs} runs per case after {WARMUP} warmup runs\n");
    println!("| case | mean ms | median ms | min ms | p95 ms |");
    println!("|---|---:|---:|---:|---:|");
    let mut failures = 0;
    for case in cases(&tmp, &big).iter().filter(|c| c.name.contains(&filter)) {
        for _ in 0..WARMUP {
            let _ = run_once(case, &tmp, &config, &cache);
        }
        let mut ms: Vec<f64> = Vec::with_capacity(runs);
        for _ in 0..runs {
            match run_once(case, &tmp, &config, &cache) {
                Some(v) => ms.push(v),
                None => failures += 1,
            }
        }
        if ms.is_empty() {
            println!("| {} | failed | failed | failed | failed |", case.name);
            continue;
        }
        ms.sort_by(|a, b| a.total_cmp(b));
        let n = ms.len();
        let mean = ms.iter().sum::<f64>() / n as f64;
        let median = if n % 2 == 1 { ms[n / 2] } else { (ms[n / 2 - 1] + ms[n / 2]) / 2.0 };
        let p95 = ms[(n * 95).div_ceil(100).max(1) - 1];
        println!("| {} | {mean:.2} | {median:.2} | {:.2} | {p95:.2} |", case.name, ms[0]);
    }

    let kib = |p: &str| fs::metadata(p).map_or(0, |m| m.len() / 1024);
    println!(
        "\nbin sizes KiB: guard {}, statusline {}, pace {}, guard_cursor {}, agent-config {}",
        kib(env!("CARGO_BIN_EXE_guard")),
        kib(env!("CARGO_BIN_EXE_statusline")),
        kib(env!("CARGO_BIN_EXE_pace")),
        kib(env!("CARGO_BIN_EXE_guard_cursor")),
        kib(env!("CARGO_BIN_EXE_agent-config")),
    );
    if failures > 0 {
        println!("spawn failures: {failures}");
    }
}
