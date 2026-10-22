//! Claude Code guard hook (PreToolUse, UserPromptSubmit, SubagentStop, Stop), a port of
//! `claude/hooks/guard.py`. Deny = message on stderr + exit 2. Allow = exit 0 silently.
//! Warn = exit 0 with hookSpecificOutput.additionalContext JSON on stdout.
//! Fail-open: any bad input, panic or unknown mode exits 0 with no output.
//!
//! The hook input is read once into [`HookInput`]; a field of another JSON type reads as absent.
//! Lengths are counted in chars, as the Python guard did. A check that cannot read what it needs
//! (a missing transcript) lets the call through.
//!
//! `GUARD_OFF` (env, else the setting; ids split by comma or space, any case) turns checks off. The
//! ids, one list in `guard_checks::REGISTRY`: G1 to G7 and G12, G13, G18, G19, H1 and W5 (preset workflow), G8 and
//! PACE (pace), G9 and ROUTER (models), G10 (ship-to-main), G11 (clock), G17 (no-ai-attribution),
//! G14, G15 and G16 (core). H1 is a hint, not a deny: a Bash `sed -n` or `cat` of a code file adds
//! the context "Tip: ~/.claude/hooks/rust-bin ctx outline ...", at most 3 times per session. W5
//! (preset workflow) is a warning too: a plain check command (`pnpm check`, `cargo test`, ...) that
//! already passed in this session, with the git work tree unchanged since, gets a note to skip it. G18
//! (workflow) denies the third try of a Bash command or an edit that failed twice. G19 (workflow) holds an
//! autopilot session or a helper back once at stop when it edited a project and no check passed since.

use agent_config::guard_checks::{self as gc, Check, FireKind, Verdict, expanduser, is_macos, realpath};
use agent_config::paths;
use agent_config::proj;
use agent_config::pycompat::{days_from_civil, truthy};
use agent_config::transcript;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agent_config::pace_core::{self as pace, Model, PaceState};

const POLL_CONTEXT: i64 = 100_000;
const NUDGE_CONTEXT: i64 = 300_000;
const SPAWN_CONTEXT: i64 = 350_000;
const MIN_INTERVAL_MIN: i64 = 30;
const NUDGE_EVERY: i64 = 5;
const SPAWN_TTL: f64 = 5400.0;
const HANDBACK_SLACK: f64 = 1.10;
const HANDBACK_MAX_DENIES: i64 = 2;
const HANDBACK_DEFAULT_CAP: i64 = 4000;
const PROMPT_SCAN_LINES: usize = 200;
const SCRATCH_ROOTS: &[&str] = &["/tmp", "/private/tmp", "/var/folders", "/private/var/folders"];
const SCRATCH_DIRS: &[&str] = &[".agents", ".superpowers"];
const ARCHIVE_TOOL: &str = "mcp__ccd_session_mgmt__archive_session";
const HEAVY_AGENTS: &[&str] = &["general-purpose", "claude"];
const LOOKUP_AGENTS: &[&str] = &["Explore", "claude-code-guide", "statusline-setup"];
const PLAN_SKILLS: &[&str] = &["superpowers:brainstorming", "superpowers:writing-plans"];
const BROWSER_PREFIXES: &[&str] = &["mcp__Claude_Browser__", "mcp__claude-in-chrome__"];
const BROWSER_WARN_AT: usize = 5;
const BROWSER_WARN_EVERY: usize = 20;
const SPAWN_CAP: usize = 15;
const TOKEN_CAP: i64 = 1_500_000;
const BUDGET_EVERY: usize = 5;
const GIT_TIMEOUT: f64 = 2.0;
const CLOCK_TURN_MIN: i64 = 10;
const G13_AFTER: usize = 8;
const H1_MAX: i64 = 3;
const PLAN_TOOL: &str = "mcp__ccd_turn__update_plan";
const WRAP_TOOL: &str = "mcp__ccd_turn__wrap_up";
const FAIL_LIMIT: i64 = 2;
const STEP_TABLE_MAX: usize = 40;

const MSG_SCRATCH: &str =
    "guard: this agent writes only scratch files; write under the scratchpad, $TMPDIR, .agents/ or .superpowers/.";
const MSG_POLL: &str =
    "guard: no polling loops at big context; use Monitor / SendMessage notify_when_idle, or one final report.";
const MSG_ASK: &str = "guard: an earlier AskUserQuestion in this turn has no answer yet. Wait for it, then ask again. Put all questions of one round in one call.";
const MSG_HEAVY_TAIL: &str = "subagent loads every tool and skill (about 60K tokens; `worker` starts near 15-20K). Use `worker` (edits, commands), `researcher` (web), `Explore` (search), `task-implementer` (one plan task) or `Plan` (planning).";
const MSG_PLAN_GATE: &str = "guard: plans run on Opus only in plan mode (opusplan). Call EnterPlanMode first, then save the plan file after you approve it (autopilot: send a `Plan` helper instead).";
const MSG_PLAN_GATE_APP: &str = "guard: in the desktop app plan mode does not switch a Sonnet session to Opus. For this plan, send a `Plan` helper (Opus by the router), or pick Opus in the model menu.";
const MSG_G18_WARN: &str = "G18: this step failed twice. Rule: after 2 failed tries on one step, stop and report. Change the approach now, or stop and report to the user.";
const MSG_G18_DENY: &str = "G18: stop and report: 2 failed tries on this step. Say what failed and what you tried, then wait (autopilot: log the blocker and take the next task). A new user message clears this.";
const MSG_G13: &str = "G13: 8 tool calls with no text. Write one short status line for the user now, then go on.";

static DEADLINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)deadline|time ?(?:box|limit)|stop at|\d+ ?-?(?:m|mins?|minutes?|h|hrs?|hours?)\b").unwrap()
});
static CAP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)hand[ -]?back|at most \d[\d,]*|\d[\d,]* ?(?:chars|characters|words|lines)\b|under \d[\d,]*|max(?:imum)? \d[\d,]*|one line|≤ ?\d[\d,]*",
    )
    .unwrap()
});
static CAP_NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(\d[\d,]*) ?(?:chars|characters)\b").unwrap());
static CAP_HANDBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)hand[ -]?backs?\b").unwrap());
static CAP_STRONG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)final message|reply|response|respond|output").unwrap());
static CAP_WEAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)answer|return|report|summary").unwrap());
static CAP_BOUND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:≤|<=|at most|under|max(?:imum)?|stay|keep|limit|no more than|within)\W*$").unwrap()
});
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.\s|\n").unwrap());
static PANEL_ANGLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:panel|angle|re-?review)\b").unwrap());
static FINAL_REVIEW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bfinal\b.{0,30}\breview|\bwhole[- ]branch\b|\bbranch[- ]review\b").unwrap());
static HANDOFF_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/handoffs/[^/]+\.md\n?\z").unwrap());
static LINK_TARGET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\(([^)\s]+)\)").unwrap());
static TASK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<task-id>([^<]+)</task-id>").unwrap());
static STAR_STEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A\*/([0-9]+)\z").unwrap());
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A[0-9]+\z").unwrap());
static MINUTE_PART: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A([0-9]+)(?:-([0-9]+))?(?:/([0-9]+))?\z").unwrap());
static NO_PUSH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)do not push|don['’]t push|no push|local only|don['’]t commit|do not commit").unwrap()
});
static CLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b\d{1,2}:\d{2}\b|\b\d+(?:\.\d+)?\s?(?:min(?:ute)?s?|h|hrs?|hours?)\b").unwrap());
static ISO_TS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)(?:\.\d+)?(Z|[+-]\d\d:?\d\d)?\z").unwrap()
});
static GIT_COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bgit\b[^;&|\n]*?\bcommit\b").unwrap());
static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w{2,}").unwrap());
static ENTRYPOINT: LazyLock<regex::bytes::Regex> =
    LazyLock::new(|| regex::bytes::Regex::new(r#""entrypoint"\s*:\s*"([^"]*)""#).unwrap());

static NULL: Value = Value::Null;

/// A record or block of a transcript: `v.get(k)`, Null when missing or not an object.
fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&NULL)
}

/// A JSON number of a transcript field, else 0.
fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// An integer counter of the state file, else 0.
fn counter(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

/// What every hook event carries, read once. A field of another JSON type reads as absent, and an
/// id, name or path that is an empty string reads as absent too.
struct HookInput<'a> {
    session_id: Option<&'a str>,
    transcript_path: Option<&'a str>,
    cwd: Option<&'a str>,
    tool_name: Option<&'a str>,
    tool_input: &'a Value,
    tool_use_id: Option<&'a str>,
    agent_id: Option<&'a str>,
    agent_type: Option<&'a str>,
    agent_transcript_path: Option<&'a str>,
    permission_mode: Option<&'a str>,
    /// Unlike the fields above, an empty reply is still a reply.
    last_assistant_message: Option<&'a str>,
    stop_hook_active: bool,
    /// `tool_response.interrupted`
    interrupted: bool,
    /// `is_interrupt` of a PostToolUseFailure: the user stopped the call, it did not fail by itself.
    is_interrupt: bool,
}

impl<'a> HookInput<'a> {
    fn new(data: &'a Value) -> Self {
        let text = |key: &str| data.get(key).and_then(Value::as_str);
        let named = |key: &str| text(key).filter(|s| !s.is_empty());
        HookInput {
            session_id: named("session_id"),
            transcript_path: named("transcript_path"),
            cwd: named("cwd"),
            tool_name: text("tool_name"),
            tool_input: get(data, "tool_input"),
            tool_use_id: named("tool_use_id"),
            agent_id: named("agent_id"),
            agent_type: text("agent_type"),
            agent_transcript_path: named("agent_transcript_path"),
            permission_mode: text("permission_mode"),
            last_assistant_message: text("last_assistant_message"),
            stop_hook_active: truthy(get(data, "stop_hook_active")),
            interrupted: get(get(data, "tool_response"), "interrupted").as_bool() == Some(true),
            is_interrupt: get(data, "is_interrupt").as_bool() == Some(true),
        }
    }

    /// A string field of the call's input.
    fn input_str(&self, key: &str) -> Option<&'a str> {
        self.tool_input.get(key).and_then(Value::as_str)
    }

    /// The command of a Bash call, when it is not empty.
    fn bash_command(&self) -> Option<&'a str> {
        if self.tool_name != Some("Bash") {
            return None;
        }
        self.input_str("command").filter(|s| !s.is_empty())
    }

    fn is_main_thread(&self) -> bool {
        self.agent_id.is_none() && !self.transcript_path.is_some_and(|p| p.contains("/subagents/"))
    }

    /// The working folder of the call, else the process folder.
    fn call_cwd(&self) -> PathBuf {
        self.cwd.map_or_else(|| env::current_dir().unwrap_or_default(), PathBuf::from)
    }
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(hay, needle).is_some()
}

fn loads(line: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(line).ok().filter(Value::is_object)
}

/// The `{"hookSpecificOutput": {...}}` line for an event with a context note.
fn hook_output(event: &str, context: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": event, "additionalContext": context}}).to_string()
}

fn sanitize_key(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' }).collect()
}

fn state_dir() -> PathBuf {
    let base = match env::var("TMPDIR").ok().filter(|s| !s.is_empty()) {
        Some(b) => PathBuf::from(b),
        None if cfg!(windows) => env::temp_dir(),
        None => PathBuf::from("/tmp"),
    };
    base.join("claude-guard")
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

// The check list (`gc::REGISTRY`) and the gate (`gc::Gate`) live in `guard_checks`: one copy for the
// guard and the Cursor adapter.

/// What a hook run reads from the settings: read once per process.
struct Cfg {
    gate: gc::Gate,
    zone: Option<String>,
}

/// An IANA-looking name that this machine knows (shape only where there is no zoneinfo folder).
fn valid_zone(z: &str) -> bool {
    let shape = !z.is_empty()
        && z.len() <= 64
        && !z.contains("..")
        && !z.starts_with('/')
        && z.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'));
    let root = Path::new("/usr/share/zoneinfo");
    shape && (!root.is_dir() || root.join(z).is_file())
}

fn load_cfg() -> Cfg {
    let all_on = || Cfg { gate: gc::Gate::all_on(), zone: None };
    let home = paths::home();
    std::panic::catch_unwind(|| {
        let zone = Some(agent_config::setting(&home, "TIMEZONE").trim().to_string()).filter(|z| valid_zone(z));
        Cfg { gate: gc::Gate::load(&home), zone }
    })
    .unwrap_or_else(|_| all_on())
}

fn cfg() -> &'static Cfg {
    static CFG: OnceLock<Cfg> = OnceLock::new();
    CFG.get_or_init(load_cfg)
}

/// True when check `id` runs: it is not in `GUARD_OFF` (env, else the setting; a comma or space
/// list) and its preset is on. `core` checks ignore the presets.
fn check_enabled(check: Check) -> bool {
    cfg().gate.enabled(check)
}

/// Locked read-modify-write of the per-session state file (the keys count, pace, spawns). None
/// when the state folder or file cannot be used.
fn with_state<T>(data: &HookInput, f: impl FnOnce(&mut Map<String, Value>) -> T) -> Option<T> {
    let sid = sanitize_key(data.session_id.unwrap_or("unknown"));
    let dir = state_dir();
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{sid}.json"));
    let lock = fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{sid}.json.lock"))).ok()?;
    let _ = lock.lock(); // closing the file releases the lock; no lock support is not fatal
    let mut state = match fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    // Compare the text, not the maps: Map equality ignores key order, and W5 keeps recency in it.
    let before = Value::Object(state.clone()).to_string();
    let out = f(&mut state);
    let text = Value::Object(state).to_string();
    // a read-only closure leaves the file alone (a missing file still gets written once)
    if text != before || !path.exists() {
        let tmp = dir.join(format!("{}.json.{}.tmp", sid, std::process::id()));
        fs::write(&tmp, text).ok()?;
        fs::rename(&tmp, &path).ok()?;
    }
    Some(out)
}

/// One running helper: the tool_use_id of its Agent call and when it started.
#[derive(Clone)]
struct Slot {
    id: Option<String>,
    t: f64,
}

/// Running slots, oldest first.
fn live_spawns(state: &Map<String, Value>, now: f64) -> Vec<Slot> {
    let mut slots = Vec::new();
    if let Some(Value::Array(a)) = state.get("spawns") {
        for s in a {
            if let Value::Object(o) = s
                && let Some(t) = o.get("t").and_then(Value::as_f64)
                && now - t < SPAWN_TTL
            {
                slots.push(Slot { id: o.get("id").and_then(Value::as_str).map(String::from), t });
            }
        }
    }
    slots.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    slots
}

fn slots_value(slots: &[Slot]) -> Value {
    Value::Array(
        slots
            .iter()
            .map(|s| {
                let mut m = Map::new();
                m.insert("id".into(), s.id.as_deref().map_or(Value::Null, Value::from));
                m.insert("t".into(), serde_json::Number::from_f64(s.t).map_or(Value::Null, Value::Number));
                Value::Object(m)
            })
            .collect(),
    )
}

fn read_counter(file: &Path, key: &str) -> i64 {
    let Ok(s) = fs::read_to_string(file) else { return 0 };
    let Ok(v) = serde_json::from_str::<Value>(&s) else { return 0 };
    counter(v.get(key))
}

// --- transcript reading -------------------------------------------------------

fn rev_lines(path: &Path) -> Option<transcript::RevLines> {
    transcript::rev_lines(path).ok()
}

fn context_size(path: Option<&str>) -> i64 {
    path.map_or(0, |p| transcript::context_size(Path::new(p)))
}

fn is_human_prompt(rec: &Value) -> bool {
    if get(rec, "type").as_str() != Some("user")
        || truthy(get(rec, "isMeta"))
        || truthy(get(rec, "isCompactSummary"))
        || truthy(get(rec, "isSidechain"))
    {
        return false;
    }
    let origin = get(rec, "origin");
    if origin.is_object() {
        let kind = get(origin, "kind");
        if !kind.is_null() && kind.as_str() != Some("human") {
            return false;
        }
    }
    let content = get(get(rec, "message"), "content");
    if content.is_string() {
        return true;
    }
    if let Some(arr) = content.as_array() {
        return !arr.iter().any(|b| get(b, "type").as_str() == Some("tool_result"));
    }
    false
}

fn message_content(rec: &Value) -> &Value {
    get(get(rec, "message"), "content")
}

/// Earlier AskUserQuestion calls of this turn that still have no tool_result (G4). A call with a
/// result, answered or refused, is done and does not count.
fn asks_since_last_prompt(path: &Path, tool_use_id: Option<&str>, tool_input: &Value) -> Option<usize> {
    let mut found: Vec<(Option<String>, Value)> = Vec::new(); // newest first
    let mut answered: Vec<Option<String>> = Vec::new(); // ids with a tool_result
    let id_of = |b: &Value, key: &str| get(b, key).as_str().map(String::from);
    for line in rev_lines(path)? {
        if !has(&line, b"\"user\"") && !has(&line, b"AskUserQuestion") && !has(&line, b"askme") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if is_human_prompt(&rec) {
            if has(&line, b"ps:askme") {
                return Some(0);
            }
            break;
        }
        let rtype = get(&rec, "type").as_str();
        if rtype == Some("user") {
            if let Some(arr) = message_content(&rec).as_array() {
                for b in arr {
                    if get(b, "type").as_str() == Some("tool_result") {
                        answered.push(id_of(b, "tool_use_id"));
                    }
                }
            }
            continue;
        }
        if rtype != Some("assistant") {
            continue;
        }
        let Some(content) = message_content(&rec).as_array() else { continue };
        for b in content.iter().rev() {
            if !b.is_object() {
                continue;
            }
            if get(b, "name").as_str() == Some("Skill")
                && get(get(b, "input"), "skill").as_str().is_some_and(|s| s.contains("askme"))
            {
                return Some(0);
            }
            if get(b, "type").as_str() == Some("tool_use") && get(b, "name").as_str() == Some("AskUserQuestion") {
                let id = id_of(b, "id");
                if !answered.contains(&id) {
                    found.push((id, get(b, "input").clone()));
                }
            }
        }
    }
    // Only asks made BEFORE the current call count.
    for (idx, (id, input)) in found.iter().enumerate() {
        let is_current = match tool_use_id {
            Some(current) => id.as_deref() == Some(current),
            None => input == tool_input,
        };
        if is_current {
            return Some(found.len() - idx - 1);
        }
    }
    Some(found.len())
}

// --- interval parsing (G2) ----------------------------------------------------

fn big(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(i64::MAX / 4).min(i64::MAX / 4)
}

fn minute_set(field: &str) -> Option<Vec<i64>> {
    let mut mins: Vec<i64> = Vec::new();
    for part in field.split(',') {
        let m = MINUTE_PART.captures(part)?;
        let lo = big(&m[1]);
        let hi = m.get(2).map_or(lo, |x| big(x.as_str()));
        let step = m.get(3).map_or(1, |x| big(x.as_str()));
        if step < 1 || lo > hi || hi > 59 {
            return None;
        }
        let mut v = lo;
        while v <= hi {
            mins.push(v);
            v += step;
        }
    }
    mins.sort_unstable();
    mins.dedup();
    Some(mins)
}

/// Smallest interval in minutes of a 5-field cron, or None if unknown.
fn cron_interval_minutes(cron: &str) -> Option<i64> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return None;
    }
    let (mut minute, hour) = (fields[0].to_string(), fields[1]);
    if minute == "*" {
        return Some(1);
    }
    if let Some(m) = STAR_STEP.captures(&minute) {
        let n = big(&m[1]);
        if n < 1 {
            return None;
        }
        minute = format!("0-59/{n}"); // */45 fires at :00 and :45, so the gap is 15
    }
    if hour == "*" || !DIGITS.is_match(&minute) {
        let ordered = minute_set(&minute)?;
        if ordered.is_empty() {
            return None;
        }
        if ordered.len() == 1 {
            return Some(60);
        }
        let mut gaps: Vec<i64> = ordered.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.push(ordered[0] + 60 - ordered[ordered.len() - 1]);
        return gaps.into_iter().min();
    }
    if let Some(h) = STAR_STEP.captures(hour)
        && DIGITS.is_match(&minute)
    {
        let n = big(&h[1]);
        return if n >= 1 { Some(n.saturating_mul(60)) } else { None };
    }
    None
}

/// Parse `(\d+(?:h|min|m|s))+` at byte `at`, followed by whitespace or end. Returns seconds.
fn loop_chain(s: &str, at: usize) -> Option<i64> {
    let b = s.as_bytes();
    let mut i = at;
    let mut total: i64 = 0;
    let mut reps = 0;
    loop {
        let ds = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == ds {
            break;
        }
        let n = big(&s[ds..i]);
        let rest = &s[i..];
        let (unit, mult) = if rest.starts_with('h') {
            (1, 3600)
        } else if rest.starts_with("min") {
            (3, 60)
        } else if rest.starts_with('m') {
            (1, 60)
        } else if rest.starts_with('s') {
            (1, 1)
        } else {
            // digits without a unit: this repetition fails; the chain so far stands
            i = ds;
            break;
        };
        total = total.saturating_add(n.saturating_mul(mult));
        i += unit;
        reps += 1;
    }
    if reps == 0 {
        return None;
    }
    match s[i..].chars().next() {
        None => Some(total),
        Some(c) if c.is_whitespace() => Some(total),
        _ => None,
    }
}

/// Seconds of a leading interval token (30s, 5m, 10min, 1h, 2h30m) or of `every <token>`.
fn loop_interval_seconds(args: &str) -> Option<i64> {
    let s = args.trim();
    if let Some(v) = loop_chain(s, 0) {
        return Some(v);
    }
    let mut from = 0;
    while let Some(rel) = s[from..].find("every ") {
        let idx = from + rel;
        let boundary = s[..idx].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if boundary && let Some(v) = loop_chain(s, idx + 6) {
            return Some(v);
        }
        from = idx + 1;
    }
    None
}

// --- guards -------------------------------------------------------------------

/// pace.status() as the guard uses it: state is one of the CAPS keys, any failure is "unknown".
#[derive(Clone)]
struct PaceSt {
    state: PaceState,
    used: Option<f64>,
    budget: Option<f64>,
}

impl PaceSt {
    const UNKNOWN: Self = Self { state: PaceState::Unknown, used: None, budget: None };
}

fn pace_status() -> PaceSt {
    if !check_enabled(Check::Pace) {
        return PaceSt::UNKNOWN; // no pace: caps stay at the base
    }
    match std::panic::catch_unwind(|| pace::status(None)) {
        Ok(s) => PaceSt { state: s.state, used: s.used, budget: Some(s.budget) },
        Err(_) => PaceSt::UNKNOWN,
    }
}

fn cap_for(state: PaceState) -> usize {
    match state {
        PaceState::Ahead => 1,
        PaceState::Critical => 0,
        PaceState::Ok | PaceState::Unknown => 3,
    }
}

/// `"%.0f%%" % v`, or "?".
fn pct(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.0}%"),
        None => "?".to_string(),
    }
}

/// What a run computes at most once: the context size and the pace.
struct Ctx<'a> {
    data: &'a HookInput<'a>,
    size: Cell<Option<i64>>,
    pace: RefCell<Option<PaceSt>>,
}

impl<'a> Ctx<'a> {
    fn new(data: &'a HookInput<'a>) -> Self {
        Ctx { data, size: Cell::new(None), pace: RefCell::new(None) }
    }
    fn size(&self) -> i64 {
        if let Some(s) = self.size.get() {
            return s;
        }
        let s = context_size(self.data.transcript_path);
        self.size.set(Some(s));
        s
    }
    fn pace(&self) -> PaceSt {
        if let Some(p) = self.pace.borrow().as_ref() {
            return p.clone();
        }
        let p = pace_status();
        *self.pace.borrow_mut() = Some(p.clone());
        p
    }
    fn reset_pace(&self) {
        *self.pace.borrow_mut() = None;
    }
}

fn g3_pretool(data: &HookInput, ctx: &Ctx) -> Verdict {
    if data.tool_name == Some("Agent") && ctx.size() > SPAWN_CONTEXT {
        return Verdict::Deny(format!(
            "guard: context is {}K (> {}K); do this step yourself instead of spawning a helper. Keep PROGRESS.md current; auto-compact carries the run.",
            ctx.size() / 1000,
            SPAWN_CONTEXT / 1000
        ));
    }
    Verdict::Allow
}

/// G1: the brief names a deadline and a hand-back cap (the router fills model and effort).
fn g1_agent(data: &HookInput, _ctx: &Ctx) -> Verdict {
    if data.tool_name != Some("Agent") || !data.tool_input.is_object() {
        return Verdict::Allow;
    }
    let prompt_text = data.input_str("prompt").unwrap_or("");
    let mut missing = Vec::new();
    if !DEADLINE_RE.is_match(prompt_text) {
        missing.push("a deadline");
    }
    if !CAP_RE.is_match(prompt_text) {
        missing.push("a hand-back cap");
    }
    if missing.is_empty() {
        return Verdict::Allow;
    }
    Verdict::Deny(format!(
        "guard: Agent prompt needs {}. Add `Deadline: 25 min.` and `Hand back at most 4,000 chars; write the rest to a file.`",
        missing.join(" and ")
    ))
}

/// G7: refuse general-purpose, claude and an untyped Agent call.
fn g7_heavy(data: &HookInput, _ctx: &Ctx) -> Verdict {
    if data.tool_name != Some("Agent") || !data.tool_input.is_object() {
        return Verdict::Allow;
    }
    let kind = match data.tool_input.get("subagent_type") {
        None | Some(Value::Null) => "general-purpose",
        Some(Value::String(s)) if s.is_empty() => "general-purpose",
        Some(Value::String(s)) => s,
        Some(_) => return Verdict::Allow,
    };
    if HEAVY_AGENTS.contains(&kind) {
        return Verdict::Deny(format!("guard: a {kind} {MSG_HEAVY_TAIL}"));
    }
    Verdict::Allow
}

fn g2_polling(data: &HookInput, ctx: &Ctx) -> Verdict {
    let Some(tool @ ("CronCreate" | "ScheduleWakeup" | "Skill")) = data.tool_name else { return Verdict::Allow };
    let ti = data.tool_input;
    if tool == "Skill" && data.input_str("skill") != Some("loop") {
        return Verdict::Allow;
    }
    if tool == "CronCreate" && ti.get("recurring") == Some(&Value::Bool(false)) {
        return Verdict::Allow;
    }
    if ctx.size() <= POLL_CONTEXT {
        return Verdict::Allow;
    }
    let bad = match tool {
        "CronCreate" => data.input_str("cron").and_then(cron_interval_minutes).is_some_and(|iv| iv < MIN_INTERVAL_MIN),
        "ScheduleWakeup" => match ti.get("delaySeconds") {
            Some(Value::Bool(b)) => i64::from(*b) < MIN_INTERVAL_MIN * 60,
            Some(Value::Number(n)) => n.as_f64().is_some_and(|d| d < (MIN_INTERVAL_MIN * 60) as f64),
            _ => false,
        },
        _ => loop_interval_seconds(data.input_str("args").unwrap_or("")).is_some_and(|s| s < MIN_INTERVAL_MIN * 60),
    };
    Verdict::from(bad.then(|| MSG_POLL.to_string()))
}

fn g4_ask(data: &HookInput, _ctx: &Ctx) -> Verdict {
    if data.tool_name != Some("AskUserQuestion") {
        return Verdict::Allow;
    }
    let Some(path) = data.transcript_path else { return Verdict::Allow };
    let open = asks_since_last_prompt(Path::new(path), data.tool_use_id, data.tool_input).unwrap_or(0);
    Verdict::from((open >= 1).then(|| MSG_ASK.to_string()))
}

// Differs from the other `text_of` copies on purpose (newline join, None when empty); see stores/claude.rs.
fn text_of(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => {
            let parts: Vec<&str> = a
                .iter()
                .filter(|b| get(b, "type").as_str() == Some("text"))
                .filter_map(|b| get(b, "text").as_str())
                .collect();
            if parts.is_empty() { None } else { Some(parts.join("\n")) }
        }
        _ => None,
    }
}

/// Text of the first user record in a subagent transcript (read from the start, 200 lines max).
fn subagent_prompt(path: Option<&str>) -> Option<String> {
    let f = fs::File::open(path?).ok()?;
    let mut rd = io::BufReader::new(f);
    for _ in 0..PROMPT_SCAN_LINES {
        let mut line = Vec::new();
        match rd.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if !has(&line, b"\"user\"") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if get(&rec, "type").as_str() != Some("user") {
            continue;
        }
        if let Some(text) = text_of(message_content(&rec))
            && !text.is_empty()
        {
            return Some(text);
        }
    }
    None
}

fn subagent_transcript(data: &HookInput) -> Option<String> {
    if let Some(atp) = data.agent_transcript_path {
        return Some(atp.to_string());
    }
    let tp = data.transcript_path?;
    if tp.contains("/subagents/") {
        return Some(tp.to_string());
    }
    let aid = data.agent_id?;
    let base = tp.strip_suffix(".jsonl").unwrap_or(tp);
    Some(format!("{base}/subagents/agent-{aid}.jsonl"))
}

fn last_sentence(s: &str) -> &str {
    match SENTENCE_END.find_iter(s).last() {
        Some(m) => &s[m.end()..],
        None => s,
    }
}

fn first_sentence(s: &str) -> &str {
    match SENTENCE_END.find(s) {
        Some(m) => &s[..m.start()],
        None => s,
    }
}

/// Smallest N (>= 200) of "N chars" that the prompt ties to the reply, or None.
///
/// The tie word must sit in the same sentence: up to 80 chars before the number or 40 after.
/// A "hand back" tie wins, then strong ties, then weak ones, then a bare bound before the number.
fn prompt_cap(text: &str) -> Option<i64> {
    let chars: Vec<char> = text.chars().collect();
    let (mut handback, mut strong, mut weak, mut bound) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for m in CAP_NUM.captures_iter(text) {
        let whole = m.get(0).unwrap();
        let digits: String = m[1].chars().filter(|c| *c != ',').collect();
        let n = match digits.parse::<i64>() {
            Ok(n) => n,
            Err(_) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => i64::MAX,
            Err(_) => continue,
        };
        if n < 200 {
            continue;
        }
        let start = text[..whole.start()].chars().count();
        let end = start + text[whole.start()..whole.end()].chars().count();
        let before_full: String = chars[start.saturating_sub(80)..start].iter().collect();
        let before = last_sentence(&before_full);
        let after_raw: String = chars[end..(end + 40).min(chars.len())].iter().collect();
        let after = first_sentence(&after_raw);
        let window = format!("{before} {after}");
        if CAP_HANDBACK.is_match(&window) {
            handback.push(n);
        } else if CAP_STRONG.is_match(&window) {
            strong.push(n);
        } else if CAP_WEAK.is_match(&window) {
            weak.push(n);
        } else {
            let bc: Vec<char> = before.chars().collect();
            let tail: String = bc[bc.len().saturating_sub(25)..].iter().collect();
            if CAP_BOUND.is_match(&tail) {
                bound.push(n); // "Stay ≤ 4,000 chars." with no reply word
            }
        }
    }
    [handback, strong, weak, bound].into_iter().find(|v| !v.is_empty()).and_then(|v| v.into_iter().min())
}

fn default_cap(agent_type: Option<&str>) -> i64 {
    match agent_type {
        Some("task-implementer" | "task-reviewer" | "researcher" | "ui-verifier" | "worker") => 3000,
        Some("code-reviewer") => 4000,
        Some("repo-scanner") => 2000,
        Some("session-finder") => 1500,
        _ => HANDBACK_DEFAULT_CAP,
    }
}

fn handback_cap(data: &HookInput) -> i64 {
    let tr = subagent_transcript(data);
    let prompt = subagent_prompt(tr.as_deref());
    match prompt.as_deref().and_then(prompt_cap) {
        Some(cap) if cap != 0 => cap,
        _ => default_cap(data.agent_type),
    }
}

/// G5: deny a hand-back over cap x 1.10, at most twice per agent; None = allow.
fn check_handback(data: &HookInput, text: Option<&str>) -> Option<String> {
    let text = text.filter(|s| !s.is_empty())?;
    let cap = handback_cap(data);
    let n = text.chars().count();
    if (n as f64) <= (cap as f64) * HANDBACK_SLACK {
        return None;
    }
    let ident = data.agent_id.or(data.session_id).unwrap_or("unknown");
    let dir = state_dir();
    let state_file = dir.join(format!("handback-{}.json", sanitize_key(ident)));
    let denies = read_counter(&state_file, "denies");
    if denies >= HANDBACK_MAX_DENIES {
        return None;
    }
    if fs::create_dir_all(&dir).is_err() || fs::write(&state_file, format!("{{\"denies\": {}}}", denies + 1)).is_err() {
        return None; // no counter, no deny: a deny without one could repeat forever
    }
    Some(format!(
        "guard: hand-back is {n} chars; the cap is {cap}. Write the full text to a file (the path the caller named, or the scratchpad), then hand back at most {cap} chars: status on line 1, key results, the file path. No Write tool: cut to the top findings."
    ))
}

fn g5_handback(data: &HookInput, _ctx: &Ctx) -> Verdict {
    if data.tool_name != Some("SubagentHandback") {
        return Verdict::Allow;
    }
    Verdict::from(check_handback(data, data.input_str("message")))
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// (hand-off path, assistant text written after it) for the newest hand-off Write since the last
/// human prompt, or (None, "").
fn handoff_this_turn(path: &Path) -> Option<(Option<String>, String)> {
    let mut texts: Vec<String> = Vec::new(); // newest first
    for line in rev_lines(path)? {
        let Some(rec) = loads(&line) else { continue };
        if is_human_prompt(&rec) {
            break;
        }
        if get(&rec, "type").as_str() != Some("assistant") {
            continue;
        }
        let Some(content) = message_content(&rec).as_array() else { continue };
        for b in content.iter().rev() {
            if !b.is_object() {
                continue;
            }
            let btype = get(b, "type").as_str();
            if btype == Some("text") {
                if let Some(t) = get(b, "text").as_str() {
                    texts.push(t.to_string());
                }
            } else if btype == Some("tool_use")
                && get(b, "name").as_str() == Some("Write")
                && let Some(fp) = get(get(b, "input"), "file_path").as_str()
                && HANDOFF_PATH.is_match(fp)
            {
                texts.reverse();
                return Some((Some(fp.to_string()), texts.join("\n")));
            }
        }
    }
    Some((None, String::new()))
}

/// The reply holds the paste line and a markdown link to the file.
fn handoff_given(fp: &str, text: &str) -> bool {
    let name = basename(fp);
    if !text.contains(&format!("Read {fp} and do it.")) {
        return false;
    }
    LINK_TARGET.captures_iter(text).any(|c| c[1].ends_with(name))
}

fn g6_archive(data: &HookInput, _ctx: &Ctx) -> Verdict {
    if data.tool_name != Some(ARCHIVE_TOOL) {
        return Verdict::Allow;
    }
    let Some(path) = data.transcript_path else { return Verdict::Allow };
    let own_session = match data.tool_input.get("session_id") {
        None | Some(Value::Null) => true,
        Some(sid) => sid.as_str() == Some("self"),
    };
    if !own_session {
        return Verdict::Allow;
    }
    let Some((fp, _)) = handoff_this_turn(Path::new(path)) else { return Verdict::Allow };
    let fp = fp.filter(|f| Path::new(f).exists()); // a hand-off deleted since is not owed to the user
    Verdict::from(fp.map(|fp| {
        let name = basename(&fp);
        format!(
            "guard: you wrote the hand-off {fp} this turn. Do not archive yet: give the link [{name}]({fp}) and the block `Read {fp} and do it.`, end the turn, and archive after the user answers."
        )
    }))
}

// --- Stop checks G6, G10 to G12 -------------------------------------------------

/// Epoch seconds of an ISO-8601 timestamp string, or None.
fn timestamp(v: &Value) -> Option<i64> {
    let m = ISO_TS.captures(v.as_str()?)?;
    let n = |i: usize| m[i].parse::<i64>().unwrap();
    let (y, mo, d, h, mi, sec) = (n(1), n(2), n(3), n(4), n(5), n(6));
    // days from civil (proleptic Gregorian), the same as calendar.timegm
    let days = days_from_civil(y, mo, d);
    let mut secs = days * 86400 + h * 3600 + mi * 60 + sec;
    if let Some(tz) = m.get(7) {
        let tz = tz.as_str();
        if tz != "Z" {
            let digits: Vec<char> = tz.chars().collect();
            let hh: i64 = digits[1..3].iter().collect::<String>().parse().unwrap_or(0);
            let mm: i64 = digits[digits.len() - 2..].iter().collect::<String>().parse().unwrap_or(0);
            let off = hh * 3600 + mm * 60;
            secs -= if digits[0] == '+' { off } else { -off };
        }
    }
    Some(secs)
}

#[derive(Default)]
struct Turn {
    start: Option<i64>,
    paths: Vec<String>,
    committed: bool,
    asked: bool,
    wrapped: bool,
    last_text: Option<String>,
    plan: Vec<(bool, Value)>, // (is wrap_up, input), newest first
    plan_done: bool,
}

/// Fold one assistant record (newest block first) into the scan result.
fn scan_record(rec: &Value, in_turn: bool, t: &mut Turn) {
    let Some(content) = message_content(rec).as_array() else { return };
    for b in content.iter().rev() {
        if !b.is_object() {
            continue;
        }
        let btype = get(b, "type").as_str();
        if btype == Some("text") {
            if let Some(text) = get(b, "text").as_str()
                && in_turn
                && t.last_text.is_none()
            {
                t.last_text = Some(text.to_string());
            }
            continue;
        }
        if btype != Some("tool_use") {
            continue;
        }
        let name = get(b, "name").as_str();
        let inp = if get(b, "input").is_object() { get(b, "input") } else { &NULL };
        if name == Some(WRAP_TOOL) {
            t.wrapped = t.wrapped || in_turn;
            if !t.plan_done {
                t.plan.push((true, inp.clone()));
                t.plan_done = true;
            }
        } else if name == Some(PLAN_TOOL) {
            if !t.plan_done {
                t.plan.push((false, inp.clone()));
                t.plan_done = get(inp, "items").is_array();
            }
        } else if !in_turn {
            // before the turn starts: nothing to count
        } else if matches!(name, Some("Edit" | "Write" | "NotebookEdit")) {
            let named = |key: &str| get(inp, key).as_str().filter(|s| !s.is_empty());
            if let Some(p) = named("file_path").or_else(|| named("notebook_path")) {
                t.paths.push(p.to_string());
            }
        } else if name == Some("Bash") {
            if GIT_COMMIT.is_match(get(inp, "command").as_str().unwrap_or("")) {
                t.committed = true;
            }
        } else if name == Some("AskUserQuestion") {
            t.asked = true;
        }
    }
}

/// One pass from the end of the transcript: this turn's facts and the update_plan calls.
fn scan_turn(path: &Path) -> Option<Turn> {
    let mut t = Turn::default();
    let mut in_turn = true;
    for line in rev_lines(path)? {
        if !in_turn {
            if t.plan_done {
                break; // the list is rebuilt: nothing older matters
            }
            if !has(&line, b"mcp__ccd_turn__") {
                continue;
            }
        }
        let Some(rec) = loads(&line) else { continue };
        if truthy(get(&rec, "isSidechain")) {
            continue;
        }
        if in_turn && is_human_prompt(&rec) {
            t.start = timestamp(get(&rec, "timestamp"));
            in_turn = false;
            continue;
        }
        if get(&rec, "type").as_str() == Some("assistant") {
            scan_record(&rec, in_turn, &mut t);
        }
    }
    Some(t)
}

fn step_name(item: &Value) -> String {
    match item {
        Value::String(s) => s.clone(),
        Value::Object(_) => ["content", "step", "title", "name", "text", "label", "description"]
            .into_iter()
            .find_map(|k| get(item, k).as_str().filter(|s| !s.is_empty()))
            .map_or_else(|| item.to_string(), String::from),
        other => other.to_string(),
    }
}

fn step_status(item: &Value) -> String {
    if !item.is_object() {
        return "pending".to_string();
    }
    let status = get(item, "status").as_str().filter(|s| !s.is_empty()).unwrap_or("pending").to_lowercase();
    if matches!(status.as_str(), "in_progress" | "in-progress" | "active") { "in_progress".to_string() } else { status }
}

fn apply_plan(steps: &mut Vec<(String, String)>, inp: &Value) {
    if let Some(items) = get(inp, "items").as_array() {
        *steps = items.iter().map(|i| (step_name(i), step_status(i))).collect();
    }
    fn mark(steps: &mut [(String, String)], n: &Value, status: &str) {
        if let Value::Number(num) = n
            && let Some(i) = num.as_i64()
            && i >= 1
            && (i as usize) <= steps.len()
        {
            steps[i as usize - 1].1 = status.to_string();
        }
    }
    let done = get(inp, "done");
    match done.as_array() {
        Some(list) => list.iter().for_each(|n| mark(steps, n, "completed")),
        None => mark(steps, done, "completed"),
    }
    if get(inp, "advance") == &Value::Bool(true) {
        if let Some(s) = steps.iter_mut().find(|s| s.1 == "in_progress") {
            s.1 = "completed".to_string();
        }
        if let Some(s) = steps.iter_mut().find(|s| s.1 == "pending") {
            s.1 = "in_progress".to_string();
        }
    }
    mark(steps, get(inp, "start"), "in_progress");
}

/// Names of the steps still open after replaying the plan events (newest first).
fn open_steps(events: &[(bool, Value)]) -> Vec<String> {
    let mut steps: Vec<(String, String)> = Vec::new();
    for (wrap, inp) in events.iter().rev() {
        if *wrap {
            steps.clear();
        } else {
            apply_plan(&mut steps, inp);
        }
    }
    steps
        .into_iter()
        .filter(|s| matches!(s.1.as_str(), "pending" | "in_progress" | "in-progress" | "active" | "todo"))
        .map(|s| s.0)
        .collect()
}

/// What one git call came to.
enum Git {
    Out(String),
    /// git ran and exited with an error (no upstream, not a repo)
    Failed,
    /// git did not start or ran past the deadline
    TimedOut,
}

fn git(args: &[&str], cwd: &str, deadline: Instant) -> Git {
    let Some(left) = deadline.checked_duration_since(Instant::now()) else { return Git::TimedOut };
    let mut cmd = std::process::Command::new("git");
    cmd.args(args).current_dir(cwd).env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0");
    match gc::run_limited(&mut cmd, left) {
        Some(done) if done.success => Git::Out(String::from_utf8_lossy(&done.stdout).into_owned()),
        Some(_) => Git::Failed,
        None => Git::TimedOut,
    }
}

/// True when any human prompt of the session says not to push or commit; None when the transcript
/// cannot be read.
fn said_no_push(path: &Path) -> Option<bool> {
    for line in rev_lines(path)? {
        if !has(&line, b"\"user\"") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if !is_human_prompt(&rec) {
            continue;
        }
        if NO_PUSH.is_match(&text_of(message_content(&rec)).unwrap_or_default()) {
            return Some(true);
        }
    }
    Some(false)
}

fn path_join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_string()
    } else if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// G10: this turn's work is uncommitted or unpushed in the session's repo. It fires once per repo
/// state: the session state keeps the last (repo, uncommitted, unpushed) it reported, and the same
/// tuple at the next Stop stays quiet. A changed tuple, or a clean repo in between, fires again.
fn g10_commit(data: &HookInput, t: &Turn) -> Option<String> {
    let cwd = data.cwd?;
    if t.paths.is_empty() && !t.committed {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs_f64(GIT_TIMEOUT);
    let Git::Out(top) = git(&["rev-parse", "--show-toplevel"], cwd, deadline) else { return None };
    let count = match git(&["rev-list", "--count", "@{u}..HEAD"], cwd, deadline) {
        Git::Out(c) => Some(c),
        Git::Failed => None, // no upstream
        Git::TimedOut => return None,
    };
    let top = realpath(top.trim());
    let unpushed: i64 = match (&count, t.committed) {
        (Some(c), true) => c.trim().parse().ok()?,
        _ => 0,
    };
    let mut dirty = 0;
    if !t.paths.is_empty() {
        let Git::Out(status) = git(&["status", "--porcelain", "-z", "-uall"], cwd, deadline) else { return None };
        let mine: HashSet<String> = t.paths.iter().map(|p| realpath(&path_join(cwd, p))).collect();
        let parts: Vec<&str> = status.split('\0').collect();
        let mut i = 0;
        while i < parts.len() {
            let entry = parts[i];
            i += 1;
            let chars: Vec<char> = entry.chars().collect();
            if chars.len() < 4 {
                continue;
            }
            if chars[0] == 'R' || chars[0] == 'C' {
                i += 1; // a rename or copy lists the old path next
            }
            let rel: String = chars[3..].iter().collect();
            if mine.contains(&realpath(&path_join(&top, &rel))) {
                dirty += 1;
            }
        }
    }
    if dirty == 0 && unpushed == 0 {
        let _ = with_state(data, |st| st.remove("g10_last")); // clean again: the next dirty state may fire
        return None;
    }
    if said_no_push(Path::new(data.transcript_path?))? {
        return None;
    }
    // Once per repo state: the same (repo, uncommitted, unpushed) as the last time stays quiet.
    let tuple = serde_json::json!([top, dirty, unpushed]);
    let seen = with_state(data, |st| {
        let same = st.get("g10_last") == Some(&tuple);
        st.insert("g10_last".into(), tuple.clone());
        same
    });
    if seen == Some(true) {
        return None;
    }
    Some(format!(
        "G10: this turn left work in {}: {} uncommitted file(s), {} unpushed commit(s). The ship-to-main preset asks: commit and push now, unless the user said not to. If the work is not finished, say what is left.",
        basename(&top),
        dirty,
        unpushed
    ))
}

fn final_reply(data: &HookInput, t: &Turn) -> String {
    data.last_assistant_message.map_or_else(|| t.last_text.clone().unwrap_or_default(), String::from)
}

/// G11: a turn of 10 min or more ends without a clock time or a duration.
fn g11_clock(data: &HookInput, t: &Turn) -> Option<String> {
    let start = t.start?;
    let minutes = ((now_secs() - start as f64) / 60.0).floor() as i64;
    if minutes >= CLOCK_TURN_MIN && !CLOCK.is_match(&final_reply(data, t)) {
        let zone = cfg().zone.as_ref().map(|z| format!(" in {z}")).unwrap_or_default();
        return Some(format!(
            "G11: this turn ran {minutes} min. The clock preset asks: give the clock as a local time (HH:MM){zone} and one estimate with a range."
        ));
    }
    None
}

/// G12: task-list steps are still open and the turn asks nothing (quiet while a helper still runs).
fn g12_open_steps(data: &HookInput, t: &Turn) -> Option<String> {
    if t.asked || t.wrapped || final_reply(data, t).trim_end().ends_with('?') {
        return None;
    }
    if with_state(data, |st| !live_spawns(st, now_secs()).is_empty()) == Some(true) {
        return None; // a background helper runs; the app wakes the session when it ends
    }
    let names = open_steps(&t.plan);
    if names.is_empty() {
        return None;
    }
    let shown: Vec<String> = names.into_iter().take(3).collect();
    Some(format!(
        "G12: the task list still has open steps ({}). Finish them now, or end with what is done, what is left, and one question.",
        shown.join(", ")
    ))
}

fn g6_stop(data: &HookInput) -> Option<String> {
    let (fp, mut text) = handoff_this_turn(Path::new(data.transcript_path?))?;
    if let Some(last) = data.last_assistant_message
        && !text.contains(last)
    {
        text = format!("{text}\n{last}");
    }
    let fp = fp.filter(|fp| Path::new(fp).exists() && !handoff_given(fp, &text))?;
    let name = basename(&fp);
    Some(format!(
        "guard: you wrote the hand-off {fp} this turn, but your reply does not give it to the user. End with the file link [{name}]({fp}) and a fenced block that holds only: Read {fp} and do it."
    ))
}

/// Keep the "what is left" file of the session in step with the open task steps: `<n>\t<first
/// open step>`, written only when the text changed, removed when nothing is open. Every IO error
/// is ignored: the statusline is a courtesy and must never fail the hook.
fn write_status(data: &HookInput, t: &Turn) {
    let Some(sid) = data.session_id else { return };
    let path = agent_config::status_path(sid);
    let names = open_steps(&t.plan);
    let Some(first) = names.first() else {
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        return;
    };
    let one_line: String = first.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let body = format!("{}\t{}\n", names.len(), one_line.trim());
    if fs::read_to_string(&path).is_ok_and(|old| old == body) {
        return;
    }
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(".status.{}.tmp", std::process::id()));
    if fs::write(&tmp, &body).and_then(|()| fs::rename(&tmp, &path)).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// Stop checks, first message wins: G6, then G19 and G10 to G12 on the main thread. One reminder per turn.
/// The status file for the statusline is updated first on the main thread, also on the final Stop
/// after a forced continuation (`stop_hook_active`), which runs no check.
fn stop(data: &HookInput) -> Option<String> {
    let path = data.transcript_path?;
    let again = data.stop_hook_active;
    if !again
        && check_enabled(Check::G6)
        && let Some(m) = g6_stop(data)
    {
        gc::fire_log(Check::G6, FireKind::Stop);
        return Some(m);
    }
    if !data.is_main_thread() {
        return None;
    }
    type TurnCheck = fn(&HookInput, &Turn) -> Option<String>;
    let checks: [(Check, TurnCheck); 4] =
        [(Check::G19, g19_stop), (Check::G10, g10_commit), (Check::G11, g11_clock), (Check::G12, g12_open_steps)];
    let active: Vec<_> = checks.into_iter().filter(|(check, _)| check_enabled(*check)).collect();
    let t = scan_turn(Path::new(path))?;
    write_status(data, &t);
    if again {
        return None;
    }
    active.into_iter().find_map(|(check, f)| {
        let m = f(data, &t)?;
        gc::fire_log(check, FireKind::Stop);
        Some(m)
    })
}

/// SubagentStop: the oldest running spawn of this session is done.
fn spawn_done(data: &HookInput) {
    let _ = with_state(data, |st| {
        let live = live_spawns(st, now_secs());
        let rest: Vec<Slot> = live.into_iter().skip(1).collect();
        st.insert("spawns".into(), slots_value(&rest));
    });
}

/// PostToolUseFailure on Agent: free the slot of that tool_use_id (a failed spawn never stops).
fn spawn_failed(data: &HookInput) {
    let Some(tid) = data.tool_use_id else { return };
    if !matches!(data.tool_name, None | Some("Agent")) {
        return;
    }
    let _ = with_state(data, |st| {
        let mut live = live_spawns(st, now_secs());
        if let Some(i) = live.iter().position(|s| s.id.as_deref() == Some(tid)) {
            live.remove(i);
        }
        st.insert("spawns".into(), slots_value(&live));
    });
}

fn subagentstop(data: &HookInput) -> Option<String> {
    if data.stop_hook_active {
        spawn_done(data);
        return None;
    }
    let mut msg = if check_enabled(Check::G5) { check_handback(data, data.last_assistant_message) } else { None };
    if msg.is_some() {
        gc::fire_log(Check::G5, FireKind::Deny);
    } else if check_enabled(Check::G19) && data.agent_id.is_some() {
        msg = g19_gate(data); // a helper always runs unattended
        if msg.is_some() {
            gc::fire_log(Check::G19, FireKind::Stop);
        }
    }
    if msg.is_none() {
        spawn_done(data); // the helper is really done; a denied hand-back keeps its slot
    }
    msg
}

// --- scratch mode -------------------------------------------------------------

/// Allow a write only under a temp or scratchpad folder; None = allow.
fn scratch(data: &HookInput) -> Option<String> {
    let named = |key: &str| data.input_str(key).filter(|s| !s.is_empty());
    let path = named("file_path").or_else(|| named("notebook_path"))?;
    let mut path = expanduser(path);
    if !path.starts_with('/')
        && let Some(cwd) = data.cwd
    {
        path = if cwd.ends_with('/') { format!("{cwd}{path}") } else { format!("{cwd}/{path}") };
    }
    let real = realpath(&path);
    let mut roots: Vec<String> = SCRATCH_ROOTS.iter().map(ToString::to_string).collect();
    if let Ok(t) = env::var("TMPDIR")
        && !t.is_empty()
    {
        roots.push(t);
    }
    for root in &roots {
        let trimmed = root.trim_end_matches('/');
        let mut cands = vec![if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() }];
        cands.push(realpath(root));
        for r in cands {
            if real == r || real.starts_with(&format!("{}/", r.trim_end_matches('/'))) {
                return None;
            }
        }
    }
    if real.contains("/scratchpad/") || real.split('/').any(|c| SCRATCH_DIRS.contains(&c)) {
        return None;
    }
    Some(MSG_SCRATCH.to_string())
}

// --- warnings -----------------------------------------------------------------

fn is_browser(name: &str) -> bool {
    BROWSER_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Browser tool_use blocks since the last human prompt, not counting the current call.
fn browser_calls_this_turn(path: &Path, tool_use_id: Option<&str>) -> Option<usize> {
    let mut n = 0;
    for line in rev_lines(path)? {
        if !has(&line, b"\"user\"") && !has(&line, b"mcp__C") && !has(&line, b"mcp__c") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if is_human_prompt(&rec) {
            break;
        }
        if get(&rec, "type").as_str() != Some("assistant") {
            continue;
        }
        let Some(content) = message_content(&rec).as_array() else { continue };
        for b in content {
            if b.is_object()
                && get(b, "type").as_str() == Some("tool_use")
                && is_browser(get(b, "name").as_str().unwrap_or(""))
                && get(b, "id").as_str() != tool_use_id
            {
                n += 1;
            }
        }
    }
    Some(n)
}

/// W3: a warning after the 5th browser call of a turn in the main window, then every 20th.
fn w3_browser(data: &HookInput, _ctx: &Ctx) -> Option<String> {
    if !is_browser(data.tool_name.unwrap_or("")) || data.agent_id.is_some() {
        return None;
    }
    let path = data.transcript_path?;
    if path.contains("/subagents/") {
        return None; // a subagent (ui-verifier) is where browser work belongs
    }
    let n = browser_calls_this_turn(Path::new(path), data.tool_use_id)?;
    if n == BROWSER_WARN_AT || (n > BROWSER_WARN_AT && (n - BROWSER_WARN_AT).is_multiple_of(BROWSER_WARN_EVERY)) {
        return Some(format!(
            "guard (warn): {} browser calls in this window this turn. Page dumps and screenshots fill the main context. For a check (layout, console, a click-through flow, a live URL), dispatch `ui-verifier`; it hands back pass or fail with evidence. Keep going here only when the user asked you to drive the browser for them.",
            n + 1
        ));
    }
    None
}

/// (Agent tool_use blocks so far, sum of totalTokens the finished Agent results report).
fn session_spawns(path: &Path) -> Option<(usize, i64)> {
    let data = fs::read(path).ok()?;
    let (mut spawns, mut tokens) = (0usize, 0i64);
    let mut tasks: HashSet<String> = HashSet::new();
    for line in data.split(|&b| b == b'\n') {
        if has(line, b"\"Agent\"")
            && let Some(rec) = loads(line)
            && let Some(content) = message_content(&rec).as_array()
        {
            for b in content {
                if b.is_object()
                    && get(b, "type").as_str() == Some("tool_use")
                    && get(b, "name").as_str() == Some("Agent")
                {
                    spawns += 1;
                }
            }
        }
        if has(line, b"\"totalTokens\"")
            && has(line, b"\"toolUseResult\"")
            && let Some(rec) = loads(line)
        {
            let tur = get(&rec, "toolUseResult");
            if tur.is_object() {
                tokens += num(get(tur, "totalTokens")) as i64;
            }
        }
        if has(line, b"\"totalTokens\"")
            && has(line, b"task-notification")
            && let Some(rec) = loads(line)
        {
            let att = get(&rec, "attachment");
            let usage = get(att, "usage");
            if att.is_object() && usage.is_object() && num(get(usage, "totalTokens")) != 0.0 {
                let prompt = get(att, "prompt").as_str().unwrap_or("");
                let delivery = get(att, "delivery_id");
                let key = if let Some(m) = TASK_ID.captures(prompt) {
                    format!("s{}", &m[1])
                } else if let Some(s) = delivery.as_str().filter(|s| !s.is_empty()) {
                    format!("s{s}")
                } else if truthy(delivery) {
                    format!("n{delivery}")
                } else {
                    format!("n{}", tasks.len())
                };
                if tasks.insert(key) {
                    tokens += num(get(usage, "totalTokens")) as i64;
                }
            }
        }
    }
    Some((spawns, tokens))
}

/// The current call is already in the transcript (written before the hook runs).
fn has_tool_use(path: &Path, tool_use_id: &str) -> Option<bool> {
    let n1 = format!("\"id\":\"{tool_use_id}\"");
    let n2 = format!("\"id\": \"{tool_use_id}\"");
    for (i, line) in rev_lines(path)?.enumerate() {
        if has(&line, n1.as_bytes()) || has(&line, n2.as_bytes()) {
            return Some(true);
        }
        if i > 50 {
            return Some(false);
        }
    }
    Some(false)
}

/// W4: a warning when the session spawned many helpers or used many subagent tokens.
fn w4_budget(data: &HookInput, _ctx: &Ctx) -> Option<String> {
    if data.tool_name != Some("Agent") {
        return None;
    }
    let path = Path::new(data.transcript_path?);
    let (spawns, tokens) = session_spawns(path)?;
    let mut prior = spawns;
    if let Some(tid) = data.tool_use_id {
        prior = if has_tool_use(path, tid)? { spawns.saturating_sub(1) } else { spawns };
    }
    let over = prior >= SPAWN_CAP || tokens >= TOKEN_CAP;
    if over && prior % BUDGET_EVERY == 0 {
        return Some(format!(
            "guard (warn): budget. This session has spawned {} agents ({}K subagent tokens; caps {} agents, {}K). Before more: call mcp__ccd_session_mgmt__get_usage, run the fan-out gate (parallel parts? pause? recovery? record? all no means no fan-out), and batch small tasks.",
            prior,
            tokens / 1000,
            SPAWN_CAP,
            TOKEN_CAP / 1000
        ));
    }
    None
}

// --- G14 to G17 live in the library (`agent_config::guard_checks`) ---------------

/// G14: deny a bare BSD `sed -i` on macOS (it eats the script as the backup suffix). A suffix
/// argument (`-i ''`, `-i.bak`, `-i '.bak'`) passes. Quoted text, `gsed` and `sed -n` pass.
fn g14_sed(data: &HookInput, _ctx: &Ctx) -> Verdict {
    let Some(cmd) = data.bash_command() else { return Verdict::Allow };
    if !check_enabled(Check::G14) {
        return Verdict::Allow;
    }
    gc::g14_sed(cmd, is_macos())
}

/// G17 (preset no-ai-attribution): deny an assistant co-author, noreply or "generated with" line in
/// a commit, tag, merge, revert, note or gh PR, issue or release text. In shadow mode (`GUARD_SHADOW`)
/// it logs what it would deny and lets the call through.
fn g17_attr(data: &HookInput, _ctx: &Ctx) -> Verdict {
    let Some(cmd) = data.bash_command() else { return Verdict::Allow };
    let cwd = data.call_cwd();
    let Ok(verdict) = std::panic::catch_unwind(|| gc::g17_attribution(cmd, &cwd)) else { return Verdict::Allow };
    let Verdict::Deny(msg) = &verdict else { return Verdict::Allow };
    if gc::shadow(Check::G17) {
        gc::shadow_log(Check::G17, &verdict, msg);
        return Verdict::Allow;
    }
    verdict
}

/// G15: deny reading local.zsh (Read, Grep or Bash) and printing full process command lines. The
/// message names the safe command for the matched case (`pgrep -f`, `ps -o ... -p`, the exclude flag).
fn g15_secrets(data: &HookInput, _ctx: &Ctx) -> Verdict {
    let Some(tool @ ("Read" | "Grep" | "Bash")) = data.tool_name else { return Verdict::Allow };
    if !check_enabled(Check::G15) {
        return Verdict::Allow;
    }
    gc::g15_secrets_at(tool, data.tool_input, &data.call_cwd().to_string_lossy())
}

// --- G13 silence note ------------------------------------------------------------

/// Tool calls since the last assistant text with real words (or the last human prompt). With a
/// tool_use_id the count ends at that call, so parallel calls each see their own number.
fn g13_streak(path: &Path, tool_use_id: Option<&str>) -> Option<usize> {
    let mut n = 0usize;
    let mut active = tool_use_id.is_none();
    for line in rev_lines(path)? {
        if has(&line, b"\"tool_result\"") {
            continue; // a tool result is never a prompt or an assistant text
        }
        if !has(&line, b"\"assistant\"") && !has(&line, b"\"user\"") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if truthy(get(&rec, "isSidechain")) {
            continue;
        }
        let rtype = get(&rec, "type").as_str();
        if rtype == Some("user") {
            if is_human_prompt(&rec) {
                break;
            }
            continue;
        }
        if rtype != Some("assistant") {
            continue;
        }
        if let Some(content) = message_content(&rec).as_array() {
            for b in content.iter().rev() {
                if !b.is_object() {
                    continue;
                }
                let btype = get(b, "type").as_str();
                if btype == Some("tool_use") {
                    if !active {
                        active = get(b, "id").as_str() == tool_use_id;
                    }
                    if active {
                        n += 1;
                    }
                } else if btype == Some("text")
                    && active
                    && let Some(text) = get(b, "text").as_str()
                    && WORDS.is_match(text)
                {
                    return Some(n);
                }
            }
        }
        if n > G13_AFTER {
            break;
        }
    }
    Some(n)
}

/// G13 (PostToolUse, main thread): one JSON line after the 8th tool call with no text, else None.
fn posttool(data: &HookInput) -> Option<String> {
    w5_record(data);
    g18_passed(data);
    g19_record(data);
    if !check_enabled(Check::G13) || env::var("CLAUDE_CODE_ENTRYPOINT").as_deref() == Ok("claude-desktop") {
        return None; // the desktop app has its own nudge and its transcript can miss text blocks
    }
    if !data.is_main_thread() {
        return None;
    }
    if g13_streak(Path::new(data.transcript_path?), data.tool_use_id)? != G13_AFTER {
        return None;
    }
    gc::fire_log(Check::G13, FireKind::Warn);
    Some(hook_output("PostToolUse", MSG_G13))
}

// --- W5 check once ------------------------------------------------------------------

/// The `checks` entry of one command: where it ran and the command in one line.
fn w5_key(data: &HookInput, cmd: &str) -> Option<String> {
    let plain = proj::plain_check(cmd)?;
    Some(format!("{}|{}", data.call_cwd().display(), plain))
}

/// W5, record side (PostToolUse runs only when the tool succeeded; a failed Bash goes to
/// PostToolUseFailure): a plain check command that passed is stored with the fingerprint of the work
/// tree after the run. The table keeps the 40 latest.
fn w5_record(data: &HookInput) {
    if !check_enabled(Check::W5) || data.interrupted {
        return;
    }
    let Some(key) = data.bash_command().and_then(|c| w5_key(data, c)) else { return };
    let Some(fp) = proj::tree_fingerprint(&data.call_cwd()) else { return };
    let _ = with_state(data, |state| {
        let table = state.entry("checks").or_insert_with(|| Value::Object(Map::new()));
        if let Some(m) = table.as_object_mut() {
            // shift_remove keeps insertion order (plain remove is swap_remove under
            // preserve_order), so the first key stays the oldest one.
            m.shift_remove(&key);
            if m.len() >= 40
                && let Some(first) = m.keys().next().cloned()
            {
                m.shift_remove(&first);
            }
            m.insert(key, Value::String(fp));
        }
    });
}

/// W5, warn side: the same plain check command passed in this session and the work tree is the same
/// as right after that run. Any error says nothing.
fn w5_repeat(data: &HookInput) -> Option<String> {
    let key = w5_key(data, data.bash_command()?)?;
    let fp = proj::tree_fingerprint(&data.call_cwd())?;
    let same = with_state(data, |state| {
        state.get("checks").and_then(|t| t.get(&key)).and_then(Value::as_str) == Some(fp.as_str())
    })?;
    same.then(|| {
        "W5: this check already passed in this session and nothing changed since (same commit, no edit, no new file). Skip it, or edit something first.".to_string()
    })
}

// --- G18 failed tries on one step -------------------------------------------------------

/// The key of the step a Bash, Edit, Write or MultiEdit call works on: the same command in the same
/// folder, or the same file and edit target. It starts with the agent, so a helper's tries are its
/// own. A hash keeps the state file small.
fn step_key(data: &HookInput) -> Option<String> {
    let who = data.agent_id.unwrap_or("main");
    let what = match data.tool_name? {
        "Bash" => {
            let cmd = data.bash_command()?.split_whitespace().collect::<Vec<_>>().join(" ");
            format!("bash|{}|{cmd}", data.call_cwd().display())
        }
        "Edit" => format!("edit|{}|{}", data.input_str("file_path")?, data.input_str("old_string").unwrap_or("")),
        "Write" => format!("write|{}", data.input_str("file_path")?),
        "MultiEdit" => {
            let olds: Vec<&str> = data
                .tool_input
                .get("edits")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|e| e.get("old_string").and_then(Value::as_str)).collect())
                .unwrap_or_default();
            format!("multiedit|{}|{}", data.input_str("file_path")?, olds.join("\u{1f}"))
        }
        _ => return None,
    };
    let digest = Sha256::digest(format!("{who}|{what}").as_bytes());
    Some(digest.iter().take(8).fold(String::new(), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    }))
}

/// The work tree fingerprint that tells "nothing changed since the last try" for a Bash step. An
/// edit between two tries makes the second a new try; an Edit step has no such fingerprint.
fn step_fingerprint(data: &HookInput) -> Option<String> {
    if data.tool_name == Some("Bash") { proj::tree_fingerprint(&data.call_cwd()) } else { None }
}

/// G18, record side (PostToolUseFailure): count a failed try of this step. The count restarts when the
/// work tree changed since the last failure of the same command. Returns the warning on the second
/// failure. A call the user interrupted is no failed try.
fn g18_failed(data: &HookInput) -> Option<String> {
    if !check_enabled(Check::G18) || data.is_interrupt {
        return None;
    }
    let key = step_key(data)?;
    let fp = step_fingerprint(data);
    let n = with_state(data, |state| {
        let table = state.entry("fails").or_insert_with(|| Value::Object(Map::new()));
        let m = table.as_object_mut()?;
        let old = m.shift_remove(&key);
        let same = old.as_ref().is_some_and(|e| e.get("fp").and_then(Value::as_str) == fp.as_deref());
        let n = if same { counter(old.as_ref().and_then(|e| e.get("n"))) + 1 } else { 1 };
        if m.len() >= STEP_TABLE_MAX
            && let Some(first) = m.keys().next().cloned()
        {
            m.shift_remove(&first);
        }
        m.insert(key, json!({"n": n, "fp": fp}));
        Some(n)
    })
    .flatten()?;
    (n == FAIL_LIMIT).then(|| {
        gc::fire_log(Check::G18, FireKind::Warn);
        hook_output("PostToolUseFailure", MSG_G18_WARN)
    })
}

/// G18, clear side (PostToolUse runs only on success): the step worked, so its count goes.
fn g18_passed(data: &HookInput) {
    if !check_enabled(Check::G18) {
        return;
    }
    let Some(key) = step_key(data) else { return };
    let _ = with_state(data, |state| {
        if let Some(m) = state.get_mut("fails").and_then(Value::as_object_mut) {
            m.shift_remove(&key);
        }
    });
}

/// G18, deny side (PreToolUse): the third try of a step that failed twice. A Bash step counts only
/// while the work tree is the same as at its last failure. A new user message clears the table.
fn g18_stuck(data: &HookInput, _ctx: &Ctx) -> Verdict {
    let Some(key) = step_key(data) else { return Verdict::Allow };
    let seen = with_state(data, |state| {
        let e = state.get("fails")?.get(&key)?;
        Some((counter(e.get("n")), e.get("fp").and_then(Value::as_str).map(String::from)))
    })
    .flatten();
    let Some((n, fp)) = seen else { return Verdict::Allow };
    if n < FAIL_LIMIT {
        return Verdict::Allow;
    }
    if fp.is_some() && step_fingerprint(data) != fp {
        return Verdict::Allow;
    }
    Verdict::Deny(MSG_G18_DENY.to_string())
}

// --- G19 fast check before an unattended run stops -----------------------------------------

/// The `edits` entry of the calling agent: `n` project edits so far, `ok` the value of `n` at the last
/// passing check command, `blocked` the value of `n` at the last stop that was held back, `dir` the
/// folder of the last edit.
fn agent_edits<'a>(state: &'a mut Map<String, Value>, data: &HookInput) -> Option<&'a mut Map<String, Value>> {
    let table = state.entry("edits").or_insert_with(|| Value::Object(Map::new())).as_object_mut()?;
    table.entry(data.agent_id.unwrap_or("main")).or_insert_with(|| Value::Object(Map::new())).as_object_mut()
}

/// G19, record side (PostToolUse): an Edit, Write or MultiEdit inside a project counts as an edit, and a
/// plain check command that passed counts as the proof for every edit so far.
fn g19_record(data: &HookInput) {
    if !check_enabled(Check::G19) || data.interrupted {
        return;
    }
    let edited = matches!(data.tool_name, Some("Edit" | "Write" | "MultiEdit"))
        .then(|| data.input_str("file_path").map(PathBuf::from))
        .flatten()
        .and_then(|f| f.parent().filter(|d| proj::find_root(d).is_some()).map(Path::to_path_buf));
    let passed = data.bash_command().is_some_and(|c| proj::plain_check(c).is_some());
    if edited.is_none() && !passed {
        return;
    }
    let _ = with_state(data, |state| {
        let Some(e) = agent_edits(state, data) else { return };
        if let Some(dir) = edited {
            e.insert("n".into(), Value::from(counter(e.get("n")) + 1));
            e.insert("dir".into(), Value::String(dir.to_string_lossy().into_owned()));
        } else {
            e.insert("ok".into(), Value::from(counter(e.get("n"))));
        }
    });
}

/// True when the transcript shows `ps:autopilot` was started: a Skill call or the typed command.
fn autopilot_run(path: &Path) -> bool {
    static MARK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#""skill"\s*:\s*"ps:autopilot"|<command-name>/?ps:autopilot</command-name>"#).unwrap()
    });
    let Ok(f) = fs::File::open(path) else { return false };
    io::BufReader::new(f)
        .split(b'\n')
        .map_while(Result::ok)
        .any(|line| has(&line, b"ps:autopilot") && std::str::from_utf8(&line).is_ok_and(|l| MARK.is_match(l)))
}

/// G19, gate side: the caller edited a project that has a fast check and no check passed since. Holds the
/// stop back once per edit count: the folder of the last edit is marked, so a second stop goes through
/// even when the stop hook flag is missing. Returns the message.
fn g19_gate(data: &HookInput) -> Option<String> {
    let dir = with_state(data, |state| {
        let e = agent_edits(state, data)?;
        let (n, ok, blocked) = (counter(e.get("n")), counter(e.get("ok")), counter(e.get("blocked")));
        if n <= ok || n <= blocked {
            return None;
        }
        e.insert("blocked".into(), Value::from(n));
        e.get("dir").and_then(Value::as_str).map(PathBuf::from)
    })
    .flatten()?;
    let fast = proj::detect(&dir)?.check_fast();
    let line = fast.iter().map(proj::Cmd::line).collect::<Vec<_>>().join(" && ");
    (!line.is_empty())
        .then(|| format!("G19: you edited files and no check passed since; run the fast check before you stop: {line}"))
}

/// G19 on Stop: only in an autopilot session. A normal chat is never held back.
fn g19_stop(data: &HookInput, _t: &Turn) -> Option<String> {
    let pending = with_state(data, |state| {
        let e = agent_edits(state, data)?;
        Some(counter(e.get("n")) > counter(e.get("ok")) && counter(e.get("n")) > counter(e.get("blocked")))
    })
    .flatten()
    .unwrap_or(false);
    if !pending || !autopilot_run(Path::new(data.transcript_path?)) {
        return None;
    }
    g19_gate(data)
}

// --- G8 helper cap, G9 plan gate, router -----------------------------------------

/// G8: helpers running at once, per session. An allowed call is recorded here, so keep this last.
fn g8_cap(data: &HookInput, ctx: &Ctx) -> Verdict {
    if data.tool_name != Some("Agent") {
        return Verdict::Allow;
    }
    let now = now_secs();
    let st = ctx.pace(); // read before the lock: pace may take a moment
    let r = with_state(data, |state| {
        let mut spawns = live_spawns(state, now);
        let cap = cap_for(st.state);
        if spawns.len() >= cap {
            let msg = format!(
                "guard: {} {} running, the cap is {} (week {} used, budget {}, pace {}). Wait for one to finish, or do the step yourself.",
                spawns.len(),
                if spawns.len() == 1 { "helper" } else { "helpers" },
                cap,
                pct(st.used),
                pct(st.budget),
                st.state
            );
            state.insert("spawns".into(), slots_value(&spawns));
            return Some(msg);
        }
        spawns.push(Slot { id: data.tool_use_id.map(String::from), t: now });
        state.insert("spawns".into(), slots_value(&spawns));
        None
    });
    Verdict::from(r.flatten()) // no state, no cap: fail open
}

/// `message.model` of the newest assistant record of a transcript (lowercase), or None.
fn session_model(path: Option<&str>) -> Option<String> {
    for line in rev_lines(Path::new(path?))? {
        if !has(&line, b"\"assistant\"") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if get(&rec, "type").as_str() != Some("assistant") {
            continue;
        }
        if let Some(m) = get(get(&rec, "message"), "model").as_str()
            && !m.is_empty()
            && !m.starts_with('<')
        {
            return Some(m.to_lowercase()); // skip "<synthetic>"
        }
    }
    None
}

/// The `entrypoint` field of the first transcript records ("cli", "claude-desktop", ...), or None.
fn session_entrypoint(path: Option<&str>) -> Option<String> {
    let f = fs::File::open(path?).ok()?;
    let mut rd = io::BufReader::new(f);
    for _ in 0..200 {
        let mut line = Vec::new();
        match rd.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if let Some(m) = ENTRYPOINT.captures(&line) {
            return Some(String::from_utf8_lossy(&m[1]).into_owned());
        }
    }
    None
}

/// G9: planning skills only in plan mode; no plan mode when the week runs ahead.
fn g9_plan_gate(data: &HookInput, ctx: &Ctx) -> Verdict {
    if data.tool_name == Some("Skill") {
        let skill = data.input_str("skill");
        if !PLAN_SKILLS.iter().any(|s| skill == Some(s)) || !data.is_main_thread() {
            return Verdict::Allow;
        }
        let st = ctx.pace();
        if data.permission_mode == Some("plan") || st.state.is_tight() {
            return Verdict::Allow;
        }
        // The main model decides: Opus and Fable already plan on Opus. Only Sonnet or Haiku is gated,
        // and only where the entrypoint is known (the CLI has opusplan; the desktop app has none).
        let path = data.transcript_path;
        match session_model(path).and_then(|m| Model::of(&m)) {
            Some(m) if m < Model::Opus => {}
            _ => return Verdict::Allow,
        }
        return Verdict::from(match session_entrypoint(path).as_deref() {
            Some("cli") => Some(MSG_PLAN_GATE.to_string()),
            Some("claude-desktop") => Some(MSG_PLAN_GATE_APP.to_string()),
            _ => None,
        });
    }
    if data.tool_name == Some("EnterPlanMode") {
        let st = ctx.pace();
        if st.state.is_tight() {
            return Verdict::Deny(format!(
                "guard: the week is {} (used {}, budget {}). Plan without plan mode so Sonnet plans.",
                st.state,
                pct(st.used),
                pct(st.budget)
            ));
        }
    }
    Verdict::Allow
}

/// Router: (new tool_input or None, note or None) for a main-thread Agent call.
///
/// Plan and a final code-reviewer: opus + high (sonnet + high when the week is ahead). Every other
/// row never raises cost: the lower of the caller's model and the table's (Explore,
/// claude-code-guide, statusline-setup: haiku; the rest: sonnet), effort high unless the caller
/// asked for low or medium, none for haiku. xhigh and max never pass. `fork` is left alone.
fn route_agent(data: &HookInput, ctx: &Ctx) -> (Option<Value>, Option<String>) {
    if data.tool_name != Some("Agent") || !data.is_main_thread() {
        return (None, None);
    }
    let Some(ti) = data.tool_input.as_object() else { return (None, None) };
    let text = |key: &str| ti.get(key).and_then(Value::as_str);
    let kind = text("subagent_type");
    if kind == Some("fork") {
        return (None, None);
    }
    let desc = text("description").unwrap_or("");
    let head: String = text("prompt").unwrap_or("").chars().take(400).collect();
    let final_ = kind == Some("code-reviewer") && (FINAL_REVIEW.is_match(desc) || FINAL_REVIEW.is_match(&head));
    let opus_row = kind == Some("Plan") || (final_ && !PANEL_ANGLE.is_match(desc));
    let mut note: Option<String> = None;
    let model: String;
    let effort: &str;
    if opus_row {
        let st = ctx.pace();
        let mut m = "opus";
        if st.state.is_tight() {
            m = "sonnet";
            note = Some(format!("the week is {}, so opus became sonnet", st.state));
        }
        model = m.to_string();
        effort = "high";
    } else {
        let table = if kind.is_some_and(|k| LOOKUP_AGENTS.contains(&k)) { Model::Haiku } else { Model::Sonnet };
        // the caller's own model name stays when it is not dearer than the table's
        model = match text("model") {
            Some(given) if Model::of(given).is_some_and(|g| g <= table) => given.to_string(),
            _ => table.name().to_string(),
        };
        effort = match text("effort") {
            Some(e @ ("low" | "medium")) => e,
            _ => "high",
        };
    }
    let mut new = ti.clone();
    new.insert("model".into(), Value::String(model.clone()));
    let shown = if Model::of(&model) == Some(Model::Haiku) {
        new.shift_remove("effort"); // haiku takes no effort
        String::new()
    } else {
        new.insert("effort".into(), Value::String(effort.to_string()));
        format!(" ({effort})")
    };
    if &new == ti {
        return (None, None);
    }
    let mut msg = format!("guard router: helper runs on {model}{shown}");
    if let Some(n) = note {
        msg.push_str(" - ");
        msg.push_str(&n);
    }
    (Some(Value::Object(new)), Some(msg))
}

// --- modes --------------------------------------------------------------------

type GuardFn = fn(&HookInput, &Ctx) -> Verdict;
/// A warning check: a note for the model, or nothing.
type WarnFn = fn(&HookInput, &Ctx) -> Option<String>;

fn pretool(data: &HookInput, ctx: &Ctx) -> Verdict {
    // g8_cap records the spawn it allows, so it runs after every other deny
    // each check runs only when its id is enabled (G14 and G15 also test their own id)
    let guards: [(Check, GuardFn); 13] = [
        (Check::G3, g3_pretool),
        (Check::G7, g7_heavy),
        (Check::G1, g1_agent),
        (Check::G2, g2_polling),
        (Check::G4, g4_ask),
        (Check::G5, g5_handback),
        (Check::G6, g6_archive),
        (Check::G9, g9_plan_gate),
        (Check::G14, g14_sed),
        (Check::G15, g15_secrets),
        (Check::G17, g17_attr),
        (Check::G18, g18_stuck),
        (Check::G8, g8_cap),
    ];
    for (check, g) in guards {
        if !check_enabled(check) {
            continue;
        }
        let verdict = g(data, ctx);
        let kind = match verdict {
            Verdict::Allow => continue,
            Verdict::Ask(_) => FireKind::Ask,
            Verdict::Deny(_) => FireKind::Deny,
        };
        gc::fire_log(check, kind);
        return verdict;
    }
    Verdict::Allow
}

/// G16 (core): a Bash or MCP call that sends text off the machine and holds a private word or a
/// secret shape, or a body the guard cannot read. Returns the JSON line of an `ask`, or None. In
/// shadow mode (`GUARD_SHADOW`) it logs what it would ask and lets the call through.
fn outbound_ask(data: &HookInput) -> Option<String> {
    if !check_enabled(Check::G16) {
        return None;
    }
    let tool = data.tool_name?;
    let input = data.tool_input;
    let verdict = std::panic::catch_unwind(|| {
        if !gc::g16_applies(tool, input) {
            return Verdict::Allow;
        }
        let cwd = data.call_cwd();
        let words = gc::private_words_at(&paths::home(), Some(&cwd));
        gc::g16_outbound(tool, input, &cwd, &words)
    })
    .ok()?;
    let Verdict::Ask(msg) = &verdict else { return None };
    if gc::shadow(Check::G16) {
        gc::shadow_log(Check::G16, &verdict, msg);
        return None;
    }
    gc::fire_log(Check::G16, FireKind::Ask);
    Some(
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "ask",
            "permissionDecisionReason": msg,
        }})
        .to_string(),
    )
}

/// H1 (preset workflow): a Bash `sed -n` or `cat` of a code file `ctx` can outline gets a one-line
/// tip, at most `H1_MAX` times per session (the count lives in the per-session state, and only a
/// shown tip counts). A call that reads nothing like that costs no file access.
fn h1_hint(data: &HookInput) -> Option<String> {
    let file = gc::h1_read_target(data.bash_command()?)?;
    let show = with_state(data, |state| {
        let n = counter(state.get("ctx_hints"));
        if n < H1_MAX {
            state.insert("ctx_hints".into(), Value::from(n + 1));
        }
        n < H1_MAX
    })?;
    show.then(|| {
        format!(
            "Tip: ~/.claude/hooks/rust-bin ctx outline {file}, then ~/.claude/hooks/rust-bin ctx read {file} <symbol>"
        )
    })
}

/// Router and warnings for an allowed call: a JSON line for stdout, or None. The line holds
/// additionalContext, plus permissionDecision allow and updatedInput when the router changed the call.
fn pretool_warn(data: &HookInput, ctx: &Ctx) -> Option<String> {
    let mut notes: Vec<String> = Vec::new();
    let mut updated: Option<Value> = None;
    if check_enabled(Check::Router)
        && let Ok((u, note)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| route_agent(data, ctx)))
    {
        if u.is_some() || note.is_some() {
            gc::fire_log(Check::Router, FireKind::Route);
        }
        updated = u;
        notes.extend(note);
    }
    // each note is logged under the check that wrote it
    let mut add = |check: Check, kind: FireKind, note: Option<String>| {
        if let Some(n) = note {
            gc::fire_log(check, kind);
            notes.push(n);
        }
    };
    if check_enabled(Check::G3) {
        for w in [w3_browser as WarnFn, w4_budget] {
            add(Check::G3, FireKind::Warn, w(data, ctx));
        }
    }
    if check_enabled(Check::H1) {
        add(Check::H1, FireKind::Hint, h1_hint(data));
    }
    if check_enabled(Check::W5) {
        add(
            Check::W5,
            FireKind::Warn,
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w5_repeat(data))).ok().flatten(),
        );
    }
    if notes.is_empty() && updated.is_none() {
        return None;
    }
    let mut hso = json!({"hookEventName": "PreToolUse"});
    if let Some(u) = updated {
        hso["permissionDecision"] = json!("allow");
        hso["updatedInput"] = u;
    }
    hso["additionalContext"] = json!(notes.join("\n"));
    Some(json!({ "hookSpecificOutput": hso }).to_string())
}

/// The note for a pace state that was not announced yet (`last` = the state announced before).
fn pace_note(st: &PaceSt, last: Option<&str>) -> Option<String> {
    let (used, budget) = (pct(st.used), pct(st.budget));
    match st.state {
        PaceState::Critical => Some(format!(
            "guard: the week is critical ({used} used, budget {budget} today). Sonnet only, no helpers. Tell the user once."
        )),
        PaceState::Ahead if last != Some("ahead") => Some(format!(
            "guard: the week is ahead of pace ({used} used, budget {budget} today). Helpers run on Sonnet, max 1 at once, and plans go without plan mode. If the main model is Opus, tell the user once to pick Sonnet."
        )),
        _ => None,
    }
}

/// G3 nudge and pace notes: JSON for stdout (or None); counters live in the per-session state file.
fn prompt(data: &HookInput, ctx: &Ctx) -> Option<String> {
    // Rescan the transcripts only when the last scan is partial, older than 2 min or from another
    // week. A fresh status line does not skip it: the scan also saves the calibration.
    if check_enabled(Check::Pace) {
        let _ = std::panic::catch_unwind(|| {
            let now = now_secs() as i64;
            if pace::prompt_needs_refresh(now) {
                pace::refresh(5.0, Some(now));
            }
        });
    }
    ctx.reset_pace();
    let st = ctx.pace();
    let over = ctx.size() >= NUDGE_CONTEXT;
    let mut note: Option<String> = None;
    let count = with_state(data, |state| {
        state.remove("fails"); // the user spoke: the tries start again
        let prev = counter(state.get("count"));
        let count = if over { prev + 1 } else { 0 };
        state.insert("count".into(), Value::from(count));
        if st.state != PaceState::Unknown && check_enabled(Check::Pace) {
            // a gap in the data does not reset what was announced
            note = pace_note(&st, state.get("pace").and_then(Value::as_str));
            state.insert("pace".into(), Value::String(st.state.as_str().to_string()));
        }
        count
    })
    .unwrap_or_else(|| i64::from(over));
    let mut notes: Vec<String> = Vec::new();
    if count >= 1 && (count - 1) % NUDGE_EVERY == 0 && check_enabled(Check::G3) {
        notes.push(format!(
            "Context is {}K. Keep PROGRESS.md (or the run's ledger) current; auto-compact carries the run. Hand off only when the user asks.",
            ctx.size() / 1000
        ));
        gc::fire_log(Check::G3, FireKind::Note);
    }
    if note.is_some() {
        gc::fire_log(Check::Pace, FireKind::Note);
    }
    notes.extend(note);
    if notes.is_empty() { None } else { Some(hook_output("UserPromptSubmit", &notes.join("\n"))) }
}

fn run() -> i32 {
    let mode = env::args().nth(1).unwrap_or_default();
    if !matches!(
        mode.as_str(),
        "pretool" | "prompt" | "subagentstop" | "posttoolfailure" | "posttool" | "scratch" | "stop"
    ) {
        return 0;
    }
    let mut raw = Vec::new();
    if io::stdin().read_to_end(&mut raw).is_err() {
        return 0;
    }
    let Ok(s) = String::from_utf8(raw) else { return 0 };
    let Ok(data) = serde_json::from_str::<Value>(&s) else { return 0 };
    if !data.is_object() {
        return 0;
    }
    let input = HookInput::new(&data);
    let ctx = Ctx::new(&input);
    let mut out: Option<String> = None;
    let msg = match mode.as_str() {
        "posttoolfailure" => {
            spawn_failed(&input);
            out = g18_failed(&input);
            None
        }
        "posttool" => {
            out = posttool(&input);
            None
        }
        "pretool" => {
            if let Verdict::Deny(m) = pretool(&input, &ctx) {
                Some(m)
            } else {
                out = outbound_ask(&input).or_else(|| pretool_warn(&input, &ctx));
                None
            }
        }
        "subagentstop" => subagentstop(&input),
        "stop" => stop(&input),
        "scratch" => scratch(&input),
        _ => {
            out = prompt(&input, &ctx);
            None
        }
    };
    if let Some(m) = msg {
        let _ = writeln!(io::stderr(), "{m}");
        return 2;
    }
    if let Some(o) = out {
        let _ = writeln!(io::stdout(), "{o}");
        let _ = io::stdout().flush();
    }
    0
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let code = std::panic::catch_unwind(run).unwrap_or(0);
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hook_output_is_valid_json() {
        let out = hook_output("PostToolUse", "a\u{7f}\"b\n");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v,
            json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "a\u{7f}\"b\n"}})
        );
    }

    #[test]
    fn prompt_cap_phrasings() {
        let cases: &[(&str, Option<i64>)] = &[
            ("Keep your response under 6,000 characters.", Some(6000)),
            ("Limit the output to 5,000 chars.", Some(5000)),
            ("6,000 chars max for the hand-back.", Some(6000)),
            ("Make truncate() return at most 500 characters. Hand back at most 4,000 chars.", Some(4000)),
            ("The API summary field holds 300 chars. Hand back at most 2,000 chars.", Some(2000)),
            (
                "Hand back \u{2264} 3,000 chars. In researcher.md set a hard cap of 2,500 chars on the final message.",
                Some(3000),
            ),
            ("Hand back at most 4,000 chars. The summary field holds 300 chars.", Some(4000)),
            ("Deadline: answer within 8 minutes. Stay \u{2264} 4,000 chars.", Some(4000)),
            ("Keep it under 2,500 characters. Pad the file to 900 chars.", Some(2500)),
            ("Reply in 2,000 chars. Final message under 1500 characters. Then hand back 800 chars.", Some(800)),
            ("Hand back 50 chars.", None),
            ("Write a description of at most 160 characters for the page.", None),
        ];
        for (p, want) in cases {
            assert_eq!(prompt_cap(p), *want, "{p}");
        }
    }

    #[test]
    fn prompt_cap_far_keyword_is_no_tie() {
        let far = format!("Reply first.{}Pad the file to 500 chars.", " ".repeat(90));
        assert_eq!(prompt_cap(&far), None);
    }

    #[test]
    fn brief_regexes() {
        assert!(DEADLINE_RE.is_match("a 25-minute budget"));
        assert!(DEADLINE_RE.is_match("DEADLINE 20 MIN"));
        assert!(!DEADLINE_RE.is_match("build it"));
        assert!(CAP_RE.is_match("reply \u{2264}4000"));
        assert!(CAP_RE.is_match("Hand-back: 3000 chars"));
        assert!(!CAP_RE.is_match("build it"));
    }

    /// `(preset, ids)` from the `guard:` line of each `claude/presets/*/rules.md`.
    fn preset_guard_lists() -> Vec<(String, Vec<String>)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../claude/presets");
        let mut out = Vec::new();
        for e in fs::read_dir(&dir).unwrap().flatten() {
            let Ok(text) = fs::read_to_string(e.path().join("rules.md")) else { continue };
            let line = text.lines().find_map(|l| l.strip_prefix("guard:")).unwrap_or("[]");
            let ids = line
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            out.push((e.file_name().to_string_lossy().into_owned(), ids));
        }
        out
    }

    #[test]
    fn registry_matches_the_preset_front_matter() {
        let mut from_files: Vec<(String, String)> = Vec::new();
        for (preset, ids) in preset_guard_lists() {
            for id in ids {
                from_files.push((id, preset.clone()));
            }
        }
        // core checks have no preset folder
        let mut from_registry: Vec<(String, String)> = gc::REGISTRY
            .iter()
            .filter(|(_, p)| *p != "core")
            .map(|(c, p)| (c.id().to_string(), p.to_string()))
            .collect();
        from_files.sort();
        from_registry.sort();
        assert_eq!(from_files, from_registry);
        let core: Vec<&str> = gc::REGISTRY.iter().filter(|(_, p)| *p == "core").map(|(c, _)| c.id()).collect();
        assert_eq!(core, ["G14", "G15", "G16"]);
    }

    #[test]
    fn zone_names() {
        assert!(valid_zone("UTC"));
        assert!(valid_zone("Asia/Tokyo"));
        assert!(!valid_zone(""));
        assert!(!valid_zone("../etc/passwd"));
        assert!(!valid_zone("/etc/passwd"));
        assert!(!valid_zone("a b"));
    }

    #[test]
    fn cron_intervals() {
        assert_eq!(cron_interval_minutes("* * * * *"), Some(1));
        assert_eq!(cron_interval_minutes("*/15 * * * *"), Some(15));
        assert_eq!(cron_interval_minutes("5 * * * *"), Some(60));
        assert_eq!(cron_interval_minutes("0-30/10 * * * *"), Some(10));
        assert_eq!(cron_interval_minutes("0 */3 * * *"), Some(180));
        assert_eq!(cron_interval_minutes("0,50 * * * *"), Some(10)); // the gap wraps past the hour
        assert_eq!(cron_interval_minutes("0,10,20 * * * *"), Some(10));
        assert_eq!(cron_interval_minutes("0,30 * * * *"), Some(30));
        assert_eq!(cron_interval_minutes("0 */2 * * *"), Some(120));
        assert_eq!(cron_interval_minutes("0 9 * * *"), None);
        assert_eq!(cron_interval_minutes("nope"), None);
    }

    #[test]
    fn test_last_assistant_usage_used_and_large_file() {
        // usage of the newest assistant record counts; a last record without usage is skipped
        let assistant = |total: i64| {
            json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "ok"}],
                "usage": {"input_tokens": 10, "cache_creation_input_tokens": total - 10 - 500,
                    "cache_read_input_tokens": 500, "output_tokens": 5}}})
        };
        let result = || {
            json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "x"}]}})
        };
        let mut recs = vec![json!({"type": "user", "message": {"role": "user", "content": "hi"}})];
        for _ in 0..3000 {
            recs.push(assistant(120_000));
            recs.push(result());
        }
        recs.push(assistant(250_000));
        recs.push(result()); // last record has no usage
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let text = recs.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
        fs::write(&path, text).unwrap();
        assert_eq!(context_size(Some(&path.to_string_lossy())), 250_000);
        assert_eq!(context_size(Some(&dir.path().join("nope.jsonl").to_string_lossy())), 0);
    }

    #[test]
    fn loop_intervals() {
        assert_eq!(loop_interval_seconds("5m check deploy"), Some(300));
        assert_eq!(loop_interval_seconds("30s /foo"), Some(30));
        assert_eq!(loop_interval_seconds("10min x"), Some(600));
        assert_eq!(loop_interval_seconds("2h30m x"), Some(9000));
        assert_eq!(loop_interval_seconds("45m check deploy"), Some(2700));
        assert_eq!(loop_interval_seconds("1h x"), Some(3600));
        assert_eq!(loop_interval_seconds("check deploy every 5m"), Some(300));
        assert_eq!(loop_interval_seconds("check deploy every 45m"), Some(2700));
        assert_eq!(loop_interval_seconds("check the deploy"), None);
        assert_eq!(loop_interval_seconds("5mx"), None);
    }

    #[test]
    fn truthiness() {
        assert!(!truthy(&json!({})));
        assert!(!truthy(&json!(0)));
        assert!(truthy(&json!("x")));
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(&json!("1970-01-02T00:00:00Z")), Some(86400));
        assert_eq!(timestamp(&json!("2026-10-19T12:00:00.123+02:00")), Some(1792404000));
        assert_eq!(timestamp(&json!("nope")), None);
    }
}
