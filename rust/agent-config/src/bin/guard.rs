//! Claude Code guard hook (`PreToolUse`, `UserPromptSubmit`, `SubagentStop`, `Stop`), a port of
//! `claude/hooks/guard.py`. Deny = message on stderr + exit 2. Allow = exit 0 silently.
//! Warn = exit 0 with hookSpecificOutput.additionalContext JSON on stdout.
//! Fail-open: any bad input, panic or unknown mode exits 0 with no output.
//!
//! The hook input is read once into [`HookInput`]; a field of another JSON type reads as absent.
//! Lengths are counted in chars, as the Python guard did. A check that cannot read what it needs
//! (a missing transcript) lets the call through.
//!
//! `GUARD_OFF` (env, else the setting; ids split by comma or space, any case) turns checks off. The
//! ids, one list in `guard_checks::REGISTRY`: G1 to G7 and G12, G13, G18, G19, G20, H1, H2 and W5 (preset workflow), G8 and
//! PACE (pace), G9 and ROUTER (models), G10 (ship-to-main), G11 (clock), G17 (no-ai-attribution),
//! G14, G15 and G16 (core). G14 and G15 fix instead of denying where they can: a bare `sed -i` on macOS
//! becomes `sed -i ''`, and a `ps` or `pgrep` that prints command lines loses them (`permissionDecision:
//! allow` + `updatedInput` + a note, the shape the router uses). They deny only where the rewrite cannot
//! reach the call (`bash -c`, `find -exec`, `--in-place`, a here-document, `procs`, a `ps` user filter),
//! and G15 always denies a read of `local.zsh`. H1 is a hint, not a deny: a Bash `sed -n` or `cat` of a code file adds
//! the context "Tip: ~/.claude/hooks/rust-bin ctx outline ...", at most 3 times per session. H2 is a hint on
//! `UserPromptSubmit`: a phrase of the user from `guard_checks::H2_PHRASES` (one set per skill, first set wins)
//! adds `The user said '<phrase>': run /ps:<name> first.` Never for a prompt that starts with `/` or
//! `<command-message>`, never when the skill ran in the last 3 assistant turns, one hint per prompt. English
//! only: Serbian words such as "arhiviraj" are out of scope. W5
//! (preset workflow) is a warning too: a plain check command (`pnpm check`, `cargo test`, ...) that
//! already passed in this session, with the git work tree unchanged since, gets a note to skip it. G18
//! (workflow) denies the third try of a Bash command or an edit that failed twice. G19 (workflow) holds an
//! autopilot session or a helper back once at stop when it edited a project and no check passed since. G20
//! (workflow) denies a `Read` of a file over 40,000 bytes with no `offset` or `limit`, once per file per
//! session; the same call again goes through. Images, PDFs and notebooks are exempt.

use agent_config::guard_checks::{
    self as gc, Check, FireKind, Verdict, basename, check_enabled, expanduser, is_macos, path_join,
    realpath,
};
use agent_config::paths;
use agent_config::proj;
use agent_config::pycompat::{self, truthy};
use agent_config::transcript;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};
use std::time::{Duration, Instant};

use agent_config::pace_core::{self as pace, Model, PaceState};

const POLL_CONTEXT: i64 = 100_000;
const NUDGE_CONTEXT: i64 = 400_000;
// below the 500K autoCompactWindow, so a session can still spawn until auto-compact
const SPAWN_CONTEXT: i64 = 475_000;
const MIN_INTERVAL_MIN: i64 = 30;
const NUDGE_EVERY: i64 = 5;
const SPAWN_TTL: f64 = 5400.0;
/// `SPAWN_TTL` in whole seconds, for the integer clock.
const SPAWN_TTL_SECS: i64 = 5400;
/// The 7-day review of 2026-11-02 found the median denied hand-back at 1.24 times the cap and 338
/// denies in a week, so the slack went up to 2x the same day.
const HANDBACK_SLACK: f64 = 2.0;
const HANDBACK_MAX_DENIES: i64 = 2;
const HANDBACK_DEFAULT_CAP: i64 = 4000;
const PROMPT_SCAN_LINES: usize = 200;
const SCRATCH_ROOTS: &[&str] = &[
    "/tmp",
    "/private/tmp",
    "/var/folders",
    "/private/var/folders",
];
const SCRATCH_DIRS: &[&str] = &[".agents", ".superlite"];
const ARCHIVE_TOOL: &str = "mcp__ccd_session_mgmt__archive_session";
const HEAVY_AGENTS: &[&str] = &["general-purpose", "claude"];
const LOOKUP_AGENTS: &[&str] = &["Explore", "claude-code-guide", "statusline-setup"];
const PLAN_SKILLS: &[&str] = &["superlite:brainstorming", "superlite:writing-plans"];
/// superlite's own agents: its `sl` tool writes their briefs and its agent files pin model and effort.
const SUPERLITE: &str = "superlite:";
/// superlite agents pinned to opus; the router lowers them to sonnet only when the week is tight.
const SUPERLITE_OPUS: &[&str] = &["superlite:final-reviewer", "superlite:fixer-opus"];
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
/// G20: a whole-file Read over this many bytes is denied once per file.
const BIG_READ_BYTES: u64 = 40_000;
const BIG_READ_TABLE_MAX: usize = 200;
/// G20 exempts the files the Read tool shows as images, pages or cells.
const BIG_READ_SKIP_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "pdf", "ipynb"];

const MSG_SCRATCH: &str = "guard: this agent writes only scratch files; write under the scratchpad, $TMPDIR, .agents/ or .superlite/.";
const MSG_POLL: &str = "guard: no polling loops at big context; use Monitor / SendMessage notify_when_idle, or one final report.";
const MSG_ASK: &str = "guard: an earlier AskUserQuestion in this turn has no answer yet. Wait for it, then ask again. Put all questions of one round in one call.";
const MSG_HEAVY_TAIL: &str = "subagent loads every tool and skill (about 60K tokens; `worker` starts near 15-20K). Use `worker` (edits, commands), `researcher` (web), `Explore` (search), `task-implementer` (one plan task) or `Plan` (planning).";
const MSG_PLAN_GATE: &str = "guard: plans run on Opus only in plan mode (opusplan). Call EnterPlanMode first, then save the plan file after you approve it (autopilot: send a `Plan` helper instead).";
const MSG_PLAN_GATE_APP: &str = "guard: in the desktop app plan mode does not switch a Sonnet session to Opus. For this plan, send a `Plan` helper (Opus by the router), or pick Opus in the model menu.";
const MSG_G18_WARN: &str = "G18: this step failed twice. Rule: after 2 failed tries on one step, stop and report. Change the approach now, or stop and report to the user.";
const MSG_G18_DENY: &str = "G18: stop and report: 2 failed tries on this step. Say what failed and what you tried, then wait (autopilot: log the blocker and take the next task). A new user message clears this.";
const MSG_G13: &str =
    "G13: 8 tool calls with no text. Write one short status line for the user now, then go on.";

/// Compile a regex literal of this file. Every call site passes a constant pattern, so it cannot fail.
fn lit_re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("regex literal in guard.rs is valid")
}

static DEADLINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    lit_re(r"(?i)deadline|time ?(?:box|limit)|stop at|\d+ ?-?(?:m|mins?|minutes?|h|hrs?|hours?)\b")
});
static CAP_RE: LazyLock<Regex> = LazyLock::new(|| {
    lit_re(
        r"(?i)hand[ -]?back|at most \d[\d,]*|\d[\d,]* ?(?:chars|characters|words|lines)\b|under \d[\d,]*|max(?:imum)? \d[\d,]*|one line|≤ ?\d[\d,]*",
    )
});
static CAP_NUM: LazyLock<Regex> =
    LazyLock::new(|| lit_re(r"(?i)(\d[\d,]*) ?(?:chars|characters)\b"));
static CAP_HANDBACK: LazyLock<Regex> = LazyLock::new(|| lit_re(r"(?i)hand[ -]?backs?\b"));
static CAP_STRONG: LazyLock<Regex> =
    LazyLock::new(|| lit_re(r"(?i)final message|reply|response|respond|output"));
static CAP_WEAK: LazyLock<Regex> = LazyLock::new(|| lit_re(r"(?i)answer|return|report|summary"));
static CAP_BOUND: LazyLock<Regex> = LazyLock::new(|| {
    lit_re(r"(?i)(?:≤|<=|at most|under|max(?:imum)?|stay|keep|limit|no more than|within)\W*$")
});
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| lit_re(r"\.\s|\n"));
static PANEL_ANGLE: LazyLock<Regex> =
    LazyLock::new(|| lit_re(r"(?i)\b(?:panel|angle|re-?review)\b"));
static FINAL_REVIEW: LazyLock<Regex> = LazyLock::new(|| {
    lit_re(r"(?i)\bfinal\b.{0,30}\breview|\bwhole[- ]branch\b|\bbranch[- ]review\b")
});
static HANDOFF_PATH: LazyLock<Regex> = LazyLock::new(|| lit_re(r"/handoffs/[^/]+\.md\n?\z"));
static LINK_TARGET: LazyLock<Regex> = LazyLock::new(|| lit_re(r"\]\(([^)\s]+)\)"));
static TASK_ID: LazyLock<Regex> = LazyLock::new(|| lit_re(r"<task-id>([^<]+)</task-id>"));
static TASK_STATUS: LazyLock<Regex> = LazyLock::new(|| lit_re(r"<status>([a-z_]+)</status>"));
static STAR_STEP: LazyLock<Regex> = LazyLock::new(|| lit_re(r"\A\*/([0-9]+)\z"));
static DIGITS: LazyLock<Regex> = LazyLock::new(|| lit_re(r"\A[0-9]+\z"));
static MINUTE_PART: LazyLock<Regex> =
    LazyLock::new(|| lit_re(r"\A([0-9]+)(?:-([0-9]+))?(?:/([0-9]+))?\z"));
static NO_PUSH: LazyLock<Regex> = LazyLock::new(|| {
    lit_re(
        r"(?i)do not push|don['’]t push|no push|local only|don['’]t commit|do not commit|\bnot push\b|\bnot to push\b|\bwithout pushing\b|\blet me review (?:it|this|first)\b|\bwait for (?:my )?review\b|\bask before (?:you )?push|\bdo not merge|\bdon['’]t merge",
    )
});
static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    lit_re(r"(?i)\b\d{1,2}:\d{2}\b|\b\d+(?:\.\d+)?\s?(?:min(?:ute)?s?|h|hrs?|hours?)\b")
});
static GIT_COMMIT: LazyLock<Regex> = LazyLock::new(|| lit_re(r"\bgit\b[^;&|\n]*?\bcommit\b"));
static WORDS: LazyLock<Regex> = LazyLock::new(|| lit_re(r"\w{2,}"));
static ENTRYPOINT: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(r#""entrypoint"\s*:\s*"([^"]*)""#)
        .expect("regex literal in guard.rs is valid")
});

static NULL: Value = Value::Null;

/// A record or block of a transcript: `v.get(k)`, Null when missing or not an object.
fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&NULL)
}

/// A JSON number of a transcript field, else 0.
fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// A JSON number of a transcript field cut to a whole number, else 0.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a float to int cast saturates and never panics; a token count is far below the limit"
)]
fn num_i64(v: &Value) -> i64 {
    num(v) as i64
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
    /// `is_interrupt` of a `PostToolUseFailure`: the user stopped the call, it did not fail by itself.
    is_interrupt: bool,
    /// The text of a `UserPromptSubmit`.
    prompt: Option<&'a str>,
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
            prompt: text("prompt"),
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
        self.agent_id.is_none()
            && !self
                .transcript_path
                .is_some_and(|p| p.contains("/subagents/"))
    }

    /// The working folder of the call, else the process folder.
    fn call_cwd(&self) -> PathBuf {
        self.cwd
            .map_or_else(|| env::current_dir().unwrap_or_default(), PathBuf::from)
    }
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(hay, needle).is_some()
}

fn loads(line: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(line)
        .ok()
        .filter(Value::is_object)
}

/// The `{"hookSpecificOutput": {...}}` line for an event with a context note.
fn hook_output(event: &str, context: &str) -> String {
    hook_json(event, vec![("additionalContext", json!(context))])
}

/// The `{"hookSpecificOutput": {...}}` line for `event` with these fields next to `hookEventName`.
fn hook_json(event: &str, fields: Vec<(&str, Value)>) -> String {
    let mut hso = json!({"hookEventName": event});
    for (k, v) in fields {
        hso[k] = v;
    }
    json!({"hookSpecificOutput": hso}).to_string()
}

fn sanitize_key(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
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
    paths::unix_now_f64()
}

// The check list (`gc::REGISTRY`) and the gate (`gc::check_enabled`) live in `guard_checks`: one copy
// for the guard and the Cursor adapter.

/// What a hook run reads from the settings besides the gate: read once per process.
struct Cfg {
    zone: Option<String>,
}

/// An IANA-looking name that this machine knows (shape only where there is no zoneinfo folder).
fn valid_zone(z: &str) -> bool {
    let shape = !z.is_empty()
        && z.len() <= 64
        && !z.contains("..")
        && !z.starts_with('/')
        && z.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'));
    let root = Path::new("/usr/share/zoneinfo");
    shape && (!root.is_dir() || root.join(z).is_file())
}

fn load_cfg() -> Cfg {
    let home = paths::home();
    std::panic::catch_unwind(|| {
        let zone = Some(agent_config::setting(&home, "TIMEZONE").trim().to_string())
            .filter(|z| valid_zone(z));
        Cfg { zone }
    })
    .unwrap_or(Cfg { zone: None })
}

fn cfg() -> &'static Cfg {
    static CFG: OnceLock<Cfg> = OnceLock::new();
    CFG.get_or_init(load_cfg)
}

/// Locked read-modify-write of the per-session state file (the keys count, pace, spawns). None
/// when the state folder or file cannot be used.
fn with_state<T>(data: &HookInput<'_>, f: impl FnOnce(&mut Map<String, Value>) -> T) -> Option<T> {
    let sid = sanitize_key(data.session_id.unwrap_or("unknown"));
    let dir = state_dir();
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{sid}.json"));
    let lock = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{sid}.json.lock")))
        .ok()?;
    let _ = lock.lock(); // closing the file releases the lock; no lock support is not fatal
    let mut state = match fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
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

/// One running helper: the `tool_use_id` of its Agent call and when it started.
#[derive(Clone)]
struct Slot {
    id: Option<String>,
    t: f64,
}

/// The state key of the slots a spawn of this agent type takes: superlite runs have their own pool.
fn slot_key(kind: Option<&str>) -> &'static str {
    if kind.is_some_and(|k| k.starts_with(SUPERLITE)) {
        "sl_spawns"
    } else {
        "spawns"
    }
}

/// Running slots under `key`, oldest first.
fn live_spawns(state: &Map<String, Value>, key: &str, now: f64) -> Vec<Slot> {
    let mut slots = Vec::new();
    if let Some(Value::Array(a)) = state.get(key) {
        for s in a {
            if let Value::Object(o) = s
                && let Some(t) = o.get("t").and_then(Value::as_f64)
                && now - t < SPAWN_TTL
            {
                slots.push(Slot {
                    id: o.get("id").and_then(Value::as_str).map(String::from),
                    t,
                });
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
                m.insert(
                    "id".into(),
                    s.id.as_deref().map_or(Value::Null, Value::from),
                );
                m.insert(
                    "t".into(),
                    serde_json::Number::from_f64(s.t).map_or(Value::Null, Value::Number),
                );
                Value::Object(m)
            })
            .collect(),
    )
}

fn read_counter(file: &Path, key: &str) -> i64 {
    let Ok(s) = fs::read_to_string(file) else {
        return 0;
    };
    let Ok(v) = serde_json::from_str::<Value>(&s) else {
        return 0;
    };
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
        return !arr
            .iter()
            .any(|b| get(b, "type").as_str() == Some("tool_result"));
    }
    false
}

fn message_content(rec: &Value) -> &Value {
    get(get(rec, "message"), "content")
}

/// The blocks of a record's `message.content` that are objects, newest first. Empty when the content
/// is not a list.
fn blocks_newest_first(rec: &Value) -> impl Iterator<Item = &Value> {
    message_content(rec)
        .as_array()
        .into_iter()
        .flat_map(|a| a.iter().rev())
        .filter(|b| b.is_object())
}

/// Earlier `AskUserQuestion` calls of this turn that still have no `tool_result` (G4). A call with a
/// result, answered or refused, is done and does not count.
fn asks_since_last_prompt(
    path: &Path,
    tool_use_id: Option<&str>,
    tool_input: &Value,
) -> Option<usize> {
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
        for b in blocks_newest_first(&rec) {
            if get(b, "name").as_str() == Some("Skill")
                && get(get(b, "input"), "skill")
                    .as_str()
                    .is_some_and(|s| s.contains("askme"))
            {
                return Some(0);
            }
            if get(b, "type").as_str() == Some("tool_use")
                && get(b, "name").as_str() == Some("AskUserQuestion")
            {
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
        return if n >= 1 {
            Some(n.saturating_mul(60))
        } else {
            None
        };
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
        let boundary = s[..idx]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if boundary && let Some(v) = loop_chain(s, idx + 6) {
            return Some(v);
        }
        from = idx + 1;
    }
    None
}

// --- guards -------------------------------------------------------------------

/// `pace.status()` as the guard uses it: state is one of the CAPS keys, any failure is "unknown".
#[derive(Clone)]
struct PaceSt {
    state: PaceState,
    used: Option<f64>,
    budget: Option<f64>,
}

impl PaceSt {
    const UNKNOWN: Self = Self {
        state: PaceState::Unknown,
        used: None,
        budget: None,
    };
}

fn pace_status() -> PaceSt {
    if !check_enabled(Check::Pace) {
        return PaceSt::UNKNOWN; // no pace: caps stay at the base
    }
    match std::panic::catch_unwind(|| pace::status(None)) {
        Ok(s) => PaceSt {
            state: s.state,
            used: s.used,
            budget: Some(s.budget),
        },
        Err(_) => PaceSt::UNKNOWN,
    }
}

/// G8 cap for helpers. An ahead week keeps the full cap: speed won over pace on
/// 2026-11-02, so only a critical week stops spawns.
fn cap_for(state: PaceState) -> usize {
    match state {
        PaceState::Critical => 0,
        PaceState::Ok | PaceState::Ahead | PaceState::Unknown => 3,
    }
}

/// G8 cap for superlite's own agents, a pool apart from the other helpers.
fn superlite_cap_for(state: PaceState) -> usize {
    match state {
        PaceState::Critical => 0,
        PaceState::Ok | PaceState::Ahead | PaceState::Unknown => 10,
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
        Ctx {
            data,
            size: Cell::new(None),
            pace: RefCell::new(None),
        }
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

fn g3_pretool(data: &HookInput<'_>, ctx: &Ctx<'_>) -> Verdict {
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
fn g1_agent(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    if data.tool_name != Some("Agent") || !data.tool_input.is_object() {
        return Verdict::Allow;
    }
    if data
        .input_str("subagent_type")
        .is_some_and(|k| k.starts_with(SUPERLITE))
    {
        return Verdict::Allow; // superlite's `sl` writes these briefs
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
fn g7_heavy(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
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

fn g2_polling(data: &HookInput<'_>, ctx: &Ctx<'_>) -> Verdict {
    let Some(tool @ ("CronCreate" | "ScheduleWakeup" | "Skill")) = data.tool_name else {
        return Verdict::Allow;
    };
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
        "CronCreate" => data
            .input_str("cron")
            .and_then(cron_interval_minutes)
            .is_some_and(|iv| iv < MIN_INTERVAL_MIN),
        "ScheduleWakeup" => match ti.get("delaySeconds") {
            Some(Value::Bool(b)) => i64::from(*b) < MIN_INTERVAL_MIN * 60,
            Some(Value::Number(n)) => n
                .as_f64()
                .is_some_and(|d| d < (MIN_INTERVAL_MIN * 60) as f64),
            _ => false,
        },
        _ => loop_interval_seconds(data.input_str("args").unwrap_or(""))
            .is_some_and(|s| s < MIN_INTERVAL_MIN * 60),
    };
    Verdict::from(bad.then(|| MSG_POLL.to_string()))
}

fn g4_ask(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    if data.tool_name != Some("AskUserQuestion") {
        return Verdict::Allow;
    }
    let Some(path) = data.transcript_path else {
        return Verdict::Allow;
    };
    let open =
        asks_since_last_prompt(Path::new(path), data.tool_use_id, data.tool_input).unwrap_or(0);
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
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
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

fn subagent_transcript(data: &HookInput<'_>) -> Option<String> {
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
    let (mut handback, mut strong, mut weak, mut bound) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for m in CAP_NUM.captures_iter(text) {
        let Some(whole) = m.get(0) else { continue };
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
    [handback, strong, weak, bound]
        .into_iter()
        .find(|v| !v.is_empty())
        .and_then(|v| v.into_iter().min())
}

fn default_cap(agent_type: Option<&str>) -> i64 {
    match agent_type {
        Some("task-implementer" | "task-reviewer" | "researcher" | "ui-verifier" | "worker") => {
            3000
        }
        Some("code-reviewer") => 4000,
        Some("repo-scanner") => 2000,
        Some("session-finder") => 1500,
        _ => HANDBACK_DEFAULT_CAP,
    }
}

fn handback_cap(data: &HookInput<'_>) -> i64 {
    let tr = subagent_transcript(data);
    let prompt = subagent_prompt(tr.as_deref());
    match prompt.as_deref().and_then(prompt_cap) {
        Some(cap) if cap != 0 => cap,
        _ => default_cap(data.agent_type),
    }
}

/// G5: deny a hand-back over cap x 2, at most twice per agent; None = allow.
fn check_handback(data: &HookInput<'_>, text: Option<&str>) -> Option<String> {
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
    if fs::create_dir_all(&dir).is_err()
        || fs::write(&state_file, format!("{{\"denies\": {}}}", denies + 1)).is_err()
    {
        return None; // no counter, no deny: a deny without one could repeat forever
    }
    Some(format!(
        "guard: hand-back is {n} chars; the cap is {cap}. Write the full text to a file (the path the caller named, or the scratchpad), then hand back at most {cap} chars: status on line 1, key results, the file path. No Write tool: cut to the top findings."
    ))
}

fn g5_handback(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    if data.tool_name != Some("SubagentHandback") {
        return Verdict::Allow;
    }
    Verdict::from(check_handback(data, data.input_str("message")))
}

/// A file in a `handoffs/` folder is a hand-off prompt for the user when its name has none of
/// `report`, `brief` or `review` (those are findings or briefs the parent reads), or when its body
/// says `Read <path> and do it`.
fn is_handoff_prompt(fp: &str, body: &str) -> bool {
    let name = basename(fp).to_lowercase();
    let says_read = body.lines().any(|l| {
        let l = l.trim().trim_end_matches('.');
        l.starts_with("Read ") && l.ends_with(" and do it")
    });
    !["report", "brief", "review"]
        .iter()
        .any(|w| name.contains(w))
        || says_read
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
        for b in blocks_newest_first(&rec) {
            let btype = get(b, "type").as_str();
            if btype == Some("text") {
                if let Some(t) = get(b, "text").as_str() {
                    texts.push(t.to_string());
                }
            } else if btype == Some("tool_use")
                && get(b, "name").as_str() == Some("Write")
                && let Some(fp) = get(get(b, "input"), "file_path").as_str()
                && HANDOFF_PATH.is_match(fp)
                && is_handoff_prompt(fp, get(get(b, "input"), "content").as_str().unwrap_or(""))
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
    LINK_TARGET
        .captures_iter(text)
        .any(|c| c[1].ends_with(name))
}

fn g6_archive(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    if data.tool_name != Some(ARCHIVE_TOOL) {
        return Verdict::Allow;
    }
    let Some(path) = data.transcript_path else {
        return Verdict::Allow;
    };
    let own_session = match data.tool_input.get("session_id") {
        None | Some(Value::Null) => true,
        Some(sid) => sid.as_str() == Some("self"),
    };
    if !own_session {
        return Verdict::Allow;
    }
    let Some((fp, _)) = handoff_this_turn(Path::new(path)) else {
        return Verdict::Allow;
    };
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
    pycompat::parse_ts_micros(v.as_str()?).map(|(micros, _)| micros.div_euclid(1_000_000))
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
    bg_started: HashSet<String>, // background Bash, Monitor and async Agent ids: this turn, or an earlier one within the spawn TTL
    bg_ended: HashSet<String>,   // ids that a task notification or TaskStop ended
    bg_expires: HashMap<String, i64>, // id -> when it ends on its own (epoch seconds): a Monitor's timeout, else the spawn TTL
    failed: HashSet<String>,          // tool_use ids whose result is an error
    wakeup: bool,                     // a ScheduleWakeup this turn: the app wakes the session later
}

impl Turn {
    /// Background work still runs (started this turn or an earlier one), or a wakeup is set. A
    /// Monitor past its timeout, or a Bash or Agent task past the spawn TTL, has ended even when no
    /// notification says so: a task that never reports must not mute the checks for good.
    fn bg_open(&self, now: i64) -> bool {
        self.wakeup
            || self.bg_started.iter().any(|id| {
                !self.bg_ended.contains(id) && self.bg_expires.get(id).is_none_or(|&end| now < end)
            })
    }
}

/// Background work in a non-assistant record of the turn: a Bash or Monitor launch (its
/// `toolUseResult` names the task id), a failed tool result, or a task notification whose status
/// is no longer `running`. The app writes a notification as a user record when the session is
/// idle, and as a `queued_command` attachment when it arrives mid-turn.
fn scan_bg(rec: &Value, t: &mut Turn) {
    let res = get(rec, "toolUseResult");
    if let Some(id) = get(res, "backgroundTaskId").as_str() {
        t.bg_started.insert(id.to_string());
        if let Some(at) = timestamp(get(rec, "timestamp")) {
            t.bg_expires.insert(id.to_string(), at + SPAWN_TTL_SECS); // a dev server never reports
        }
    } else if let (Some(id), Some(ms)) =
        (get(res, "taskId").as_str(), get(res, "timeoutMs").as_i64())
    {
        t.bg_started.insert(id.to_string()); // a Monitor
        if let Some(at) = timestamp(get(rec, "timestamp")) {
            t.bg_expires.insert(id.to_string(), at + ms / 1000);
        }
    } else if get(res, "status").as_str() == Some("async_launched")
        && let Some(id) = get(res, "agentId").as_str()
    {
        t.bg_started.insert(id.to_string()); // a helper started in the background; its notification names the same id
        if let Some(at) = timestamp(get(rec, "timestamp")) {
            // like a spawn slot, it expires: a notice that never comes must not mute the checks for good
            t.bg_expires.insert(id.to_string(), at + SPAWN_TTL_SECS);
        }
    }
    for b in message_content(rec).as_array().into_iter().flatten() {
        if get(b, "type").as_str() == Some("tool_result")
            && truthy(get(b, "is_error"))
            && let Some(id) = get(b, "tool_use_id").as_str()
        {
            t.failed.insert(id.to_string());
        }
    }
    let att = get(rec, "attachment");
    let text = if get(rec, "type").as_str() == Some("attachment")
        && get(att, "type").as_str() == Some("queued_command")
    {
        text_of(get(att, "prompt"))
    } else if get(get(rec, "origin"), "kind").as_str() == Some("task-notification") {
        text_of(message_content(rec))
    } else {
        return;
    };
    let text = text.unwrap_or_default();
    for c in TASK_ID.captures_iter(&text) {
        let (Some(all), Some(id)) = (c.get(0), c.get(1)) else {
            continue;
        };
        let rest = text[all.end()..].split("<task-id>").next().unwrap_or("");
        if TASK_STATUS
            .captures(rest)
            .and_then(|s| s.get(1))
            .is_some_and(|s| s.as_str() != "running")
        {
            t.bg_ended.insert(id.as_str().trim().to_string());
        }
    }
}

/// Fold one assistant record (newest block first) into the scan result.
fn scan_record(rec: &Value, in_turn: bool, t: &mut Turn) {
    for b in blocks_newest_first(rec) {
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
        let inp = if get(b, "input").is_object() {
            get(b, "input")
        } else {
            &NULL
        };
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
        } else if name == Some("TaskStop") {
            // in this turn or an earlier one: the task it names is over
            if let Some(id) = get(inp, "task_id")
                .as_str()
                .or_else(|| get(inp, "shell_id").as_str())
            {
                t.bg_ended.insert(id.to_string());
            }
        } else if !in_turn {
            // before the turn starts: nothing else to count
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
        } else if name == Some("ScheduleWakeup") {
            // a refused or failed call sets no wakeup; its result came first in this scan
            t.wakeup = t.wakeup
                || !get(b, "id")
                    .as_str()
                    .is_some_and(|id| t.failed.contains(id));
        }
    }
}

/// True when the record on `line` carries a `timestamp` before `horizon` (both ISO UTC, compared
/// as bytes on the `YYYY-MM-DDTHH:MM:SS` prefix): no JSON parse.
fn older_than(line: &[u8], horizon: &str) -> bool {
    const KEY: &[u8] = b"\"timestamp\":\"";
    memchr::memmem::find(line, KEY).is_some_and(|i| {
        let at = &line[i + KEY.len()..];
        at.len() >= horizon.len() && at[..horizon.len()] < *horizon.as_bytes()
    })
}

/// A line that may start or end background work: a Bash, Monitor or async Agent launch, or a task
/// notification.
fn bg_line(line: &[u8]) -> bool {
    has(line, b"backgroundTaskId")
        || has(line, b"async_launched")
        || has(line, b"\"taskId\"")
        || has(line, b"task-notification")
}

/// One pass from the end of the transcript: this turn's facts and the `update_plan` calls. Background
/// work is read further back, to the spawn TTL: a task that an earlier turn started and nothing
/// ended keeps the Stop checks quiet too. Older launches have expired, so once the plan is known
/// the scan stops there.
fn scan_turn(path: &Path) -> Option<Turn> {
    let mut t = Turn::default();
    let mut in_turn = true;
    let mut bg_done = false; // the records are older than the spawn TTL: no live background work there
    let horizon = jiff::Timestamp::from_second(paths::unix_now() - SPAWN_TTL_SECS)
        .unwrap_or_default()
        .strftime("%Y-%m-%dT%H:%M:%S")
        .to_string();
    for line in rev_lines(path)? {
        if !in_turn {
            bg_done = bg_done || older_than(&line, &horizon);
            if t.plan_done && bg_done {
                break; // the list is rebuilt and the background work is settled: nothing older matters
            }
            let bg = !bg_done && (bg_line(&line) || has(&line, b"TaskStop"));
            if !bg && !has(&line, b"mcp__ccd_turn__") {
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
        let is_bg = if in_turn {
            bg_line(&line) || has(&line, b"\"is_error\":true")
        } else {
            !bg_done && bg_line(&line)
        };
        if get(&rec, "type").as_str() == Some("assistant") {
            scan_record(&rec, in_turn, &mut t);
        } else if is_bg {
            scan_bg(&rec, &mut t);
        }
    }
    Some(t)
}

fn step_name(item: &Value) -> String {
    match item {
        Value::String(s) => s.clone(),
        Value::Object(_) => [
            "content",
            "step",
            "title",
            "name",
            "text",
            "label",
            "description",
        ]
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
    let status = get(item, "status")
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("pending")
        .to_lowercase();
    if matches!(status.as_str(), "in_progress" | "in-progress" | "active") {
        "in_progress".to_string()
    } else {
        status
    }
}

/// One task-list step: name, status, and whether it was handed to another session (`session`).
type Step = (String, String, bool);

/// Set the status of step `n` (1-based) when `n` is a JSON number inside the list.
fn mark(steps: &mut [Step], n: &Value, status: &str) {
    if let Value::Number(num) = n
        && let Some(i) = num.as_i64()
        && let Ok(i) = usize::try_from(i)
        && let Some(step) = i.checked_sub(1).and_then(|at| steps.get_mut(at))
    {
        step.1 = status.to_string();
    }
}

fn apply_plan(steps: &mut Vec<Step>, inp: &Value) {
    if let Some(items) = get(inp, "items").as_array() {
        let session = |i: &Value| {
            get(i, "session")
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
        };
        *steps = items
            .iter()
            .map(|i| (step_name(i), step_status(i), session(i)))
            .collect();
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

/// The steps still open after replaying the plan events (newest first).
fn open_plan(events: &[(bool, Value)]) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    for (wrap, inp) in events.iter().rev() {
        if *wrap {
            steps.clear();
        } else {
            apply_plan(&mut steps, inp);
        }
    }
    steps.retain(|s| {
        matches!(
            s.1.as_str(),
            "pending" | "in_progress" | "in-progress" | "active" | "todo"
        )
    });
    steps
}

/// Names of the steps still open.
fn open_steps(events: &[(bool, Value)]) -> Vec<String> {
    open_plan(events).into_iter().map(|s| s.0).collect()
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
    let Some(left) = deadline.checked_duration_since(Instant::now()) else {
        return Git::TimedOut;
    };
    let mut cmd = std::process::Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    match gc::run_limited(&mut cmd, left) {
        Some(done) if done.success => Git::Out(String::from_utf8_lossy(&done.stdout).into_owned()),
        Some(_) => Git::Failed,
        None => Git::TimedOut,
    }
}

/// True when any human prompt of the session says not to push, commit or merge, or asks to review
/// first (the `NO_PUSH` phrases); None when the transcript cannot be read.
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

/// G10: this turn's work is uncommitted or unpushed in the session's repo. It fires once per repo
/// state: the session state keeps the last (repo, uncommitted, unpushed) it reported, and the same
/// tuple at the next Stop stays quiet. A changed tuple, or a clean repo in between, fires again.
/// It is also quiet while a helper of this session runs (the work is not finished), and `stop`
/// lets only one of G10 to G12 fire per human turn.
fn g10_commit(data: &HookInput<'_>, t: &Turn) -> Option<String> {
    let cwd = data.cwd?;
    if t.paths.is_empty() && !t.committed {
        return None;
    }
    if busy(data, t) {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs_f64(GIT_TIMEOUT);
    let Git::Out(top) = git(&["rev-parse", "--show-toplevel"], cwd, deadline) else {
        return None;
    };
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
        let Git::Out(status) = git(&["status", "--porcelain", "-z", "-uall"], cwd, deadline) else {
            return None;
        };
        let mine: HashSet<String> = t
            .paths
            .iter()
            .map(|p| realpath(&path_join(cwd, p)))
            .collect();
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
    let advice = if ships_to_main(cwd, deadline) {
        "The ship-to-main preset asks: commit and push now, unless the user said not to or the project's CLAUDE.md says otherwise. Fetch and rebase on origin/main first, run the fast check again, then push."
    } else {
        "The ship-to-main preset pushes to main only in repos of the SHIP_OWNERS setting, and this repo is not one: commit on a branch and open a PR, do not push to main."
    };
    Some(format!(
        "G10: this turn left work in {}: {} uncommitted file(s), {} unpushed commit(s). {advice} If the work is not finished, say what is left.",
        basename(&top),
        dirty,
        unpushed
    ))
}

/// The GitHub owner in a remote URL (`git@github.com:o/r.git`, `https://[user@]github.com/o/r`,
/// `ssh://git@github.com/o/r`); None for any other host or shape.
fn github_owner(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("ssh://"))?;
        let (host, path) = rest.split_once('/')?;
        if host.rsplit('@').next() != Some("github.com") {
            return None;
        }
        path
    };
    let owner = path
        .split('/')
        .next()
        .filter(|o| !o.is_empty() && path.contains('/'))?;
    Some(owner.to_ascii_lowercase())
}

/// True when the repo's origin is a GitHub repo whose owner is in `SHIP_OWNERS` (env, even an
/// empty one, else the setting; a space or comma list, any case). An empty list, another host or
/// no origin is false: those repos get a branch and a PR.
fn ships_to_main(cwd: &str, deadline: Instant) -> bool {
    let raw = env::var("SHIP_OWNERS")
        .unwrap_or_else(|_| agent_config::setting(&paths::home(), "SHIP_OWNERS"));
    let owners: Vec<String> = raw
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|o| !o.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if owners.is_empty() {
        return false;
    }
    let Git::Out(url) = git(&["remote", "get-url", "origin"], cwd, deadline) else {
        return false;
    };
    github_owner(&url).is_some_and(|o| owners.contains(&o))
}

fn final_reply(data: &HookInput<'_>, t: &Turn) -> String {
    data.last_assistant_message
        .map_or_else(|| t.last_text.clone().unwrap_or_default(), String::from)
}

/// True while a helper of this session runs (a live slot in `spawns` or `sl_spawns`). The app wakes
/// the session when it ends, so the Stop reminders wait.
fn helpers_running(data: &HookInput<'_>) -> bool {
    let running = |st: &mut Map<String, Value>| {
        ["spawns", "sl_spawns"]
            .iter()
            .any(|k| !live_spawns(st, k, now_secs()).is_empty())
    };
    with_state(data, running) == Some(true)
}

/// True while the session waits on work that wakes it later: background Bash or Monitor tasks
/// of this turn that no notification or `TaskStop` ended, a `ScheduleWakeup`, or a live helper (a helper
/// started in the background counts through its launch record too, until it ends or the spawn TTL
/// passes). The reply's wording ("waiting for the notification") says nothing by itself and is not
/// read. G19 and G10 to G12 stay quiet then; the app starts a new segment of the turn when the work
/// ends.
fn busy(data: &HookInput<'_>, t: &Turn) -> bool {
    t.bg_open(paths::unix_now()) || helpers_running(data)
}

/// The local time as `HH:MM`, and the zone's name: the `TIMEZONE` setting when it names a zone, else
/// the system zone (its IANA name, or None when the system gives none).
fn local_clock(secs: i64, setting: Option<&str>) -> (String, Option<String>) {
    let tz = setting
        .and_then(|z| jiff::tz::TimeZone::get(z).ok())
        .unwrap_or_else(jiff::tz::TimeZone::system);
    let name = tz.iana_name().map(String::from);
    let at = jiff::Timestamp::from_second(secs)
        .unwrap_or_default()
        .to_zoned(tz);
    (at.strftime("%H:%M").to_string(), name)
}

/// G11: a turn of 10 min or more ends without a clock time or a duration. Quiet while a helper
/// of this session runs; `stop` lets only one of G10 to G12 fire per human turn.
fn g11_clock(data: &HookInput<'_>, t: &Turn) -> Option<String> {
    let start = t.start?;
    let minutes = (paths::unix_now() - start).div_euclid(60);
    if minutes >= CLOCK_TURN_MIN && !CLOCK.is_match(&final_reply(data, t)) {
        if busy(data, t) {
            return None;
        }
        let (now, name) = local_clock(paths::unix_now(), cfg().zone.as_deref());
        let is_at = match name {
            Some(z) => format!("It is {now} in {z}."),
            None => format!("It is {now} on this machine's clock."),
        };
        // the zone is asked for only when the user set one
        let zone = cfg()
            .zone
            .as_ref()
            .map(|z| format!(" in {z}"))
            .unwrap_or_default();
        return Some(format!(
            "G11: this turn ran {minutes} min. {is_at} The clock preset asks: give the clock as a local time (HH:MM){zone} and one estimate with a range. No `date` call is needed."
        ));
    }
    None
}

/// G12: task-list steps are still open and the turn asks nothing (quiet while a helper still runs).
/// `stop` lets only one of G10 to G12 fire per human turn.
fn g12_open_steps(data: &HookInput<'_>, t: &Turn) -> Option<String> {
    if t.asked || t.wrapped || final_reply(data, t).trim_end().ends_with('?') {
        return None;
    }
    if busy(data, t) {
        return None; // a background helper runs; the app wakes the session when it ends
    }
    let open = open_plan(&t.plan);
    if open.is_empty() || open.iter().any(|s| s.2) {
        return None; // a step runs in another session; the app wakes this one when it ends
    }
    let shown: Vec<String> = open.into_iter().take(3).map(|s| s.0).collect();
    Some(format!(
        "G12: the task list still has open steps ({}). Finish them now, or end with what is done, what is left, and one question.",
        shown.join(", ")
    ))
}

fn g6_stop(data: &HookInput<'_>) -> Option<String> {
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
fn write_status(data: &HookInput<'_>, t: &Turn) {
    let Some(sid) = data.session_id else { return };
    let path = agent_config::status_path(sid);
    let names = open_steps(&t.plan);
    let Some(first) = names.first() else {
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        return;
    };
    let one_line: String = first
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let body = format!("{}\t{}\n", names.len(), one_line.trim());
    if fs::read_to_string(&path).is_ok_and(|old| old == body) {
        return;
    }
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(".status.{}.tmp", std::process::id()));
    if fs::write(&tmp, &body)
        .and_then(|()| fs::rename(&tmp, &path))
        .is_err()
    {
        let _ = fs::remove_file(&tmp);
    }
}

/// The state key that holds the start (`Turn.start`) of the human turn in which G10, G11 or G12 last fired.
const STOP_TURN_KEY: &str = "stop_turn";

/// True when G10 to G12 already fired in the human turn that began at `start`. A task notification
/// starts a new segment of the same turn, so the Stop checks would fire again at each segment end.
fn reminded_this_turn(data: &HookInput<'_>, start: Option<i64>) -> bool {
    let Some(start) = start else { return false };
    with_state(data, |st| {
        st.get(STOP_TURN_KEY).and_then(Value::as_i64) == Some(start)
    }) == Some(true)
}

/// The state key that maps a check id to the turn start of its last shadow log. Apart from
/// `STOP_TURN_KEY`, so a shadowed check never hides a real hold of another check in the same turn.
const SHADOW_TURN_KEY: &str = "shadow_turn";

/// True when `check` already logged a shadow hold in the turn that began at `start`; else marks it.
/// A turn with no timestamp is never deduped.
fn shadow_logged_this_turn(data: &HookInput<'_>, check: Check, start: Option<i64>) -> bool {
    let Some(start) = start else { return false };
    with_state(data, |st| {
        let mut seen = st
            .get(SHADOW_TURN_KEY)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if seen.get(check.id()).and_then(Value::as_i64) == Some(start) {
            return true;
        }
        seen.insert(check.id().into(), Value::from(start));
        st.insert(SHADOW_TURN_KEY.into(), Value::Object(seen));
        false
    }) == Some(true)
}

/// A Stop check that reads the scanned turn.
type TurnCheck = fn(&HookInput<'_>, &Turn) -> Option<String>;

/// Stop checks, first message wins: G6, then G19 and G10 to G12 on the main thread. G10 to G12 give
/// one reminder per human turn together (keyed on the turn start in the session state; a turn with
/// no timestamp is never deduped); G6 and G19 are not limited. The status file for the statusline
/// is updated first on the main thread, also on the final Stop after a forced continuation
/// (`stop_hook_active`), which runs no check.
fn stop(data: &HookInput<'_>) -> Option<String> {
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
    let checks: [(Check, TurnCheck); 4] = [
        (Check::G19, g19_stop),
        (Check::G10, g10_commit),
        (Check::G11, g11_clock),
        (Check::G12, g12_open_steps),
    ];
    let active: Vec<_> = checks
        .into_iter()
        .filter(|(check, _)| check_enabled(*check))
        .collect();
    let t = scan_turn(Path::new(path))?;
    write_status(data, &t);
    if again {
        return None;
    }
    let mut reminded: Option<bool> = None; // looked up once, and only when G10 to G12 are about to run
    active.into_iter().find_map(|(check, f)| {
        let once = matches!(check, Check::G10 | Check::G11 | Check::G12);
        if once && *reminded.get_or_insert_with(|| reminded_this_turn(data, t.start)) {
            return None;
        }
        let m = f(data, &t)?;
        if gc::shadow(check) {
            // logged once per turn and check; the stop goes through
            if !shadow_logged_this_turn(data, check, t.start) {
                gc::shadow_log(check, &Verdict::Deny(m.clone()), &m);
            }
            return None;
        }
        if let (true, Some(start)) = (once, t.start) {
            let _ = with_state(data, |st| {
                st.insert(STOP_TURN_KEY.into(), Value::from(start))
            });
        }
        gc::fire_log(check, FireKind::Stop);
        Some(m)
    })
}

/// `SubagentStop`: the oldest running spawn of this session, in the pool of its agent type, is done.
fn spawn_done(data: &HookInput<'_>) {
    let key = slot_key(data.agent_type);
    let _ = with_state(data, |st| {
        let live = live_spawns(st, key, now_secs());
        let rest: Vec<Slot> = live.into_iter().skip(1).collect();
        st.insert(key.into(), slots_value(&rest));
    });
}

/// `PostToolUseFailure` on Agent: free the slot of that `tool_use_id` (a failed spawn never stops).
fn spawn_failed(data: &HookInput<'_>) {
    let Some(tid) = data.tool_use_id else { return };
    if !matches!(data.tool_name, None | Some("Agent")) {
        return;
    }
    let _ = with_state(data, |st| {
        for key in ["spawns", "sl_spawns"] {
            let mut live = live_spawns(st, key, now_secs());
            if let Some(i) = live.iter().position(|s| s.id.as_deref() == Some(tid)) {
                live.remove(i);
                st.insert(key.into(), slots_value(&live));
            }
        }
    });
}

fn subagentstop(data: &HookInput<'_>) -> Option<String> {
    if data.stop_hook_active {
        spawn_done(data);
        return None;
    }
    let mut msg = if check_enabled(Check::G5) {
        check_handback(data, data.last_assistant_message)
    } else {
        None
    };
    if msg.is_some() {
        gc::fire_log(Check::G5, FireKind::Deny);
    } else if check_enabled(Check::G19)
        && data.agent_id.is_some()
        && !data.agent_type.is_some_and(|k| k.starts_with(SUPERLITE))
    // superlite gates its own tests
    {
        msg = g19_gate(data); // a helper always runs unattended
        // g19_gate has already marked this edit count as held, so turning shadow off later does
        // not hold it once more; harmless.
        if let Some(m) = msg.take_if(|_| gc::shadow(Check::G19)) {
            gc::shadow_log(Check::G19, &Verdict::Deny(m.clone()), &m);
        }
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
fn scratch(data: &HookInput<'_>) -> Option<String> {
    let named = |key: &str| data.input_str(key).filter(|s| !s.is_empty());
    let path = named("file_path").or_else(|| named("notebook_path"))?;
    let mut path = expanduser(path);
    if let Some(cwd) = data.cwd {
        path = path_join(cwd, &path);
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
        let mut cands = vec![if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        }];
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

/// Browser `tool_use` blocks since the last human prompt, not counting the current call.
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
        n += blocks_newest_first(&rec)
            .filter(|b| {
                get(b, "type").as_str() == Some("tool_use")
                    && is_browser(get(b, "name").as_str().unwrap_or(""))
                    && get(b, "id").as_str() != tool_use_id
            })
            .count();
    }
    Some(n)
}

/// W3: a warning after the 5th browser call of a turn in the main window, then every 20th.
fn w3_browser(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Option<String> {
    if !is_browser(data.tool_name.unwrap_or("")) || data.agent_id.is_some() {
        return None;
    }
    let path = data.transcript_path?;
    if path.contains("/subagents/") {
        return None; // a subagent (ui-verifier) is where browser work belongs
    }
    let n = browser_calls_this_turn(Path::new(path), data.tool_use_id)?;
    if n == BROWSER_WARN_AT
        || (n > BROWSER_WARN_AT && (n - BROWSER_WARN_AT).is_multiple_of(BROWSER_WARN_EVERY))
    {
        return Some(format!(
            "guard (warn): {} browser calls in this window this turn. Page dumps and screenshots fill the main context. For a check (layout, console, a click-through flow, a live URL), dispatch `ui-verifier`; it hands back pass or fail with evidence. Keep going here only when the user asked you to drive the browser for them.",
            n + 1
        ));
    }
    None
}

/// (Agent `tool_use` blocks so far, sum of totalTokens the finished Agent results report).
fn session_spawns(path: &Path) -> Option<(usize, i64)> {
    let data = fs::read(path).ok()?;
    let (mut spawns, mut tokens) = (0usize, 0i64);
    let mut tasks: HashSet<String> = HashSet::new();
    // a transcript can repeat one Agent call in several records: count each call id once
    let mut calls: HashSet<String> = HashSet::new();
    for line in data.split(|&b| b == b'\n') {
        if has(line, b"\"Agent\"")
            && let Some(rec) = loads(line)
            && let Some(content) = message_content(&rec).as_array()
        {
            for b in content {
                if b.is_object()
                    && get(b, "type").as_str() == Some("tool_use")
                    && get(b, "name").as_str() == Some("Agent")
                    && get(b, "id")
                        .as_str()
                        .is_none_or(|id| calls.insert(id.to_string()))
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
                tokens += num_i64(get(tur, "totalTokens"));
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
                    tokens += num_i64(get(usage, "totalTokens"));
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
fn w4_budget(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Option<String> {
    if data.tool_name != Some("Agent") {
        return None;
    }
    let path = Path::new(data.transcript_path?);
    let (spawns, tokens) = session_spawns(path)?;
    let mut prior = spawns;
    if let Some(tid) = data.tool_use_id {
        prior = if has_tool_use(path, tid)? {
            spawns.saturating_sub(1)
        } else {
            spawns
        };
    }
    let over = prior >= SPAWN_CAP || tokens >= TOKEN_CAP;
    if over && prior % BUDGET_EVERY == 0 {
        return Some(format!(
            "guard (warn): budget. This session has spawned {} agents ({}K subagent tokens; caps {} agents, {}K). Before more: call mcp__ccd_session_mgmt__get_usage, send only independent parts in parallel, and batch small tasks.",
            prior,
            tokens / 1000,
            SPAWN_CAP,
            TOKEN_CAP / 1000
        ));
    }
    None
}

// --- G14 to G17 live in the library (`agent_config::guard_checks`) ---------------

/// G14: a bare BSD `sed -i` on macOS (it eats the script as the backup suffix) is rewritten to
/// `sed -i ''` by [`bash_rewrite`] and let through. It is denied only where the rewrite cannot
/// reach it. A suffix argument (`-i ''`, `-i.bak`, `-i '.bak'`) passes. Quoted text, `gsed` and
/// `sed -n` pass.
fn g14_sed(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    let Some(cmd) = data.bash_command() else {
        return Verdict::Allow;
    };
    if !check_enabled(Check::G14) {
        return Verdict::Allow;
    }
    let verdict = gc::g14_sed(cmd, is_macos());
    if matches!(verdict, Verdict::Deny(_)) && gc::g14_fix(cmd, is_macos()).is_some() {
        return Verdict::Allow;
    }
    verdict
}

/// G17 (preset no-ai-attribution): deny an assistant co-author, noreply or "generated with" line in
/// a commit, tag, merge, revert, note or gh PR, issue or release text. In shadow mode (`GUARD_SHADOW`)
/// it logs what it would deny and lets the call through.
fn g17_attr(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    let Some(cmd) = data.bash_command() else {
        return Verdict::Allow;
    };
    let cwd = data.call_cwd();
    let verdict =
        std::panic::catch_unwind(|| gc::g17_attribution(cmd, &cwd)).unwrap_or(Verdict::Allow);
    gc::shadowed(Check::G17, verdict)
}

/// G15: deny reading local.zsh (Read, Grep or Bash). A `ps` or `pgrep` that prints full process
/// command lines is rewritten to the safe form by [`bash_rewrite`] and let through; it is denied
/// only where the rewrite cannot reach it (`procs`, a user filter, a `ps` in `bash -c`). The
/// message names the safe command for the matched case (`pgrep -f`, `ps -o ... -p`, the exclude flag).
fn g15_secrets(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    let Some(tool @ ("Read" | "Grep" | "Bash")) = data.tool_name else {
        return Verdict::Allow;
    };
    if !check_enabled(Check::G15) {
        return Verdict::Allow;
    }
    let cwd = data.call_cwd().to_string_lossy().into_owned();
    let verdict = gc::g15_secrets_at(tool, data.tool_input, &cwd);
    if matches!(verdict, Verdict::Deny(_))
        && let Some(cmd) = data.bash_command()
        && gc::g15_fix(cmd, &cwd).is_some()
    {
        return Verdict::Allow;
    }
    verdict
}

const NOTE_G14_REWRITE: &str =
    "G14: rewrote `sed -i` to `sed -i ''` (BSD sed on macOS needs the suffix word).";
const NOTE_G15_REWRITE: &str = "G15: rewrote ps or pgrep to print no command lines (ps: `-o pid=,etime=,ucomm=`; pgrep: no -l or -a).";

/// The G14 and G15 fixes of a Bash call: the new command, and the checks that changed it. None when
/// neither check has anything to fix. The result passes both checks.
fn bash_rewrite(data: &HookInput<'_>) -> Option<(String, Vec<Check>)> {
    let mut cmd = data.bash_command()?.to_string();
    let mut done = Vec::new();
    if check_enabled(Check::G14)
        && let Some(fixed) = gc::g14_fix(&cmd, is_macos())
    {
        cmd = fixed;
        done.push(Check::G14);
    }
    if check_enabled(Check::G15)
        && let Some(fixed) = gc::g15_fix(&cmd, &data.call_cwd().to_string_lossy())
    {
        cmd = fixed;
        done.push(Check::G15);
    }
    (!done.is_empty()).then_some((cmd, done))
}

// --- G13 silence note ------------------------------------------------------------

/// Tool calls since the last assistant text with real words (or the last human prompt). With a
/// `tool_use_id` the count ends at that call, so parallel calls each see their own number.
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
        for b in blocks_newest_first(&rec) {
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
        if n > G13_AFTER {
            break;
        }
    }
    Some(n)
}

/// G13 (`PostToolUse`, main thread): one JSON line after the 8th tool call with no text, else None.
fn posttool(data: &HookInput<'_>) -> Option<String> {
    w5_record(data);
    g18_passed(data);
    g19_record(data);
    if !check_enabled(Check::G13)
        || env::var("CLAUDE_CODE_ENTRYPOINT").as_deref() == Ok("claude-desktop")
    {
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

// --- bounded tables in the session state ----------------------------------------------------

/// The object under `key` in `m`, made when missing. None when the entry holds something else.
fn state_table<'a>(m: &'a mut Map<String, Value>, key: &str) -> Option<&'a mut Map<String, Value>> {
    m.entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
}

/// Set `key` as the newest entry of the table; at `max` entries the oldest one goes first.
fn put_bounded(m: &mut Map<String, Value>, key: String, v: Value, max: usize) {
    // shift_remove keeps insertion order (plain remove is swap_remove under preserve_order), so the
    // first key stays the oldest one.
    m.shift_remove(&key);
    if m.len() >= max
        && let Some(first) = m.keys().next().cloned()
    {
        m.shift_remove(&first);
    }
    m.insert(key, v);
}

// --- W5 check once ------------------------------------------------------------------

/// The `checks` entry of one command: where it ran and the command in one line.
fn w5_key(data: &HookInput<'_>, cmd: &str) -> Option<String> {
    let plain = proj::plain_check(cmd)?;
    Some(format!("{}|{}", data.call_cwd().display(), plain))
}

/// W5, record side (`PostToolUse` runs only when the tool succeeded; a failed Bash goes to
/// PostToolUseFailure): a plain check command that passed is stored with the fingerprint of the work
/// tree after the run. The table keeps the `STEP_TABLE_MAX` latest.
fn w5_record(data: &HookInput<'_>) {
    if !check_enabled(Check::W5) || data.interrupted {
        return;
    }
    let Some(key) = data.bash_command().and_then(|c| w5_key(data, c)) else {
        return;
    };
    let Some(fp) = proj::tree_fingerprint(&data.call_cwd()) else {
        return;
    };
    let _ = with_state(data, |state| {
        if let Some(m) = state_table(state, "checks") {
            put_bounded(m, key, Value::String(fp), STEP_TABLE_MAX);
        }
    });
}

/// W5, warn side: the same plain check command passed in this session and the work tree is the same
/// as right after that run. Any error says nothing.
fn w5_repeat(data: &HookInput<'_>) -> Option<String> {
    let key = w5_key(data, data.bash_command()?)?;
    let fp = proj::tree_fingerprint(&data.call_cwd())?;
    let same = with_state(data, |state| {
        state
            .get("checks")
            .and_then(|t| t.get(&key))
            .and_then(Value::as_str)
            == Some(fp.as_str())
    })?;
    same.then(|| {
        "W5: this check already passed in this session and nothing changed since (same commit, no edit, no new file). Skip it, or edit something first.".to_string()
    })
}

// --- G18 failed tries on one step -------------------------------------------------------

/// The key of the step a Bash, Edit, Write or `MultiEdit` call works on: the same command in the same
/// folder, or the same file and edit target. It starts with the agent, so a helper's tries are its
/// own. A hash keeps the state file small.
fn step_key(data: &HookInput<'_>) -> Option<String> {
    let who = data.agent_id.unwrap_or("main");
    let what = match data.tool_name? {
        "Bash" => {
            let cmd = data
                .bash_command()?
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            format!("bash|{}|{cmd}", data.call_cwd().display())
        }
        "Edit" => format!(
            "edit|{}|{}",
            data.input_str("file_path")?,
            data.input_str("old_string").unwrap_or("")
        ),
        "Write" => format!("write|{}", data.input_str("file_path")?),
        "MultiEdit" => {
            let olds: Vec<&str> = data
                .tool_input
                .get("edits")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.get("old_string").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            format!(
                "multiedit|{}|{}",
                data.input_str("file_path")?,
                olds.join("\u{1f}")
            )
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
fn step_fingerprint(data: &HookInput<'_>) -> Option<String> {
    if data.tool_name == Some("Bash") {
        proj::tree_fingerprint(&data.call_cwd())
    } else {
        None
    }
}

/// G18, record side (PostToolUseFailure): count a failed try of this step. The count restarts when the
/// work tree changed since the last failure of the same command. Returns the warning on the second
/// failure. A call the user interrupted is no failed try.
fn g18_failed(data: &HookInput<'_>) -> Option<String> {
    if !check_enabled(Check::G18) || data.is_interrupt {
        return None;
    }
    let key = step_key(data)?;
    let fp = step_fingerprint(data);
    let n = with_state(data, |state| {
        let m = state_table(state, "fails")?;
        let old = m.shift_remove(&key);
        let same = old
            .as_ref()
            .is_some_and(|e| e.get("fp").and_then(Value::as_str) == fp.as_deref());
        let n = if same {
            counter(old.as_ref().and_then(|e| e.get("n"))) + 1
        } else {
            1
        };
        put_bounded(m, key, json!({"n": n, "fp": fp}), STEP_TABLE_MAX);
        Some(n)
    })
    .flatten()?;
    (n == FAIL_LIMIT).then(|| {
        gc::fire_log(Check::G18, FireKind::Warn);
        hook_output("PostToolUseFailure", MSG_G18_WARN)
    })
}

/// G18, clear side (`PostToolUse` runs only on success): the step worked, so its count goes.
fn g18_passed(data: &HookInput<'_>) {
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
fn g18_stuck(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    let Some(key) = step_key(data) else {
        return Verdict::Allow;
    };
    let seen = with_state(data, |state| {
        let e = state.get("fails")?.get(&key)?;
        Some((
            counter(e.get("n")),
            e.get("fp").and_then(Value::as_str).map(String::from),
        ))
    })
    .flatten();
    let Some((n, fp)) = seen else {
        return Verdict::Allow;
    };
    if n < FAIL_LIMIT {
        return Verdict::Allow;
    }
    if fp.is_some() && step_fingerprint(data) != fp {
        return Verdict::Allow;
    }
    Verdict::Deny(MSG_G18_DENY.to_string())
}

// --- G20 whole-file Read of a big file ------------------------------------------------------

/// G20 (preset workflow), `PreToolUse` on `Read`, main thread and helpers: deny a Read with no `offset`
/// and no `limit` of an existing file over `BIG_READ_BYTES`, once per file per session. The deny
/// records the real path in the session state, so the same call again goes through. Images, PDFs and
/// notebooks are exempt. Any error reading the size or the state lets the call through.
fn g20_big_read(data: &HookInput<'_>, _ctx: &Ctx<'_>) -> Verdict {
    if data.tool_name != Some("Read") {
        return Verdict::Allow;
    }
    let ti = data.tool_input;
    if ["offset", "limit"]
        .iter()
        .any(|k| ti.get(*k).is_some_and(|v| !v.is_null()))
    {
        return Verdict::Allow;
    }
    let Some(raw) = data.input_str("file_path").filter(|s| !s.is_empty()) else {
        return Verdict::Allow;
    };
    let ext = Path::new(raw)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if BIG_READ_SKIP_EXT.contains(&ext.as_str()) {
        return Verdict::Allow;
    }
    let path = expanduser(raw);
    let real = realpath(&path_join(&data.call_cwd().to_string_lossy(), &path));
    let Ok(meta) = fs::metadata(&real) else {
        return Verdict::Allow;
    };
    if !meta.is_file() || meta.len() <= BIG_READ_BYTES {
        return Verdict::Allow;
    }
    let first = with_state(data, |state| {
        let list = state
            .entry("big_reads")
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(a) = list.as_array_mut() else {
            return false;
        };
        if a.iter().any(|v| v.as_str() == Some(real.as_str())) {
            return false;
        }
        if a.len() >= BIG_READ_TABLE_MAX {
            a.remove(0);
        }
        a.push(Value::String(real.clone()));
        true
    });
    if first != Some(true) {
        return Verdict::Allow;
    }
    let name = Path::new(&real)
        .file_name()
        .map_or_else(|| real.clone(), |n| n.to_string_lossy().into_owned());
    Verdict::Deny(format!(
        "G20: {name} is {}K chars. Read only the part you need with offset and limit, or run ~/.claude/hooks/rust-bin ctx outline {raw} first. Repeat this exact call to read the whole file.",
        meta.len() / 1000
    ))
}

// --- G19 fast check before an unattended run stops -----------------------------------------

/// The `edits` entry of the calling agent: `n` project edits so far, `ok` the value of `n` at the last
/// passing check command, `blocked` the value of `n` at the last stop that was held back, `dir` the
/// folder of the last edit.
fn agent_edits<'a>(
    state: &'a mut Map<String, Value>,
    data: &HookInput<'_>,
) -> Option<&'a mut Map<String, Value>> {
    state_table(
        state_table(state, "edits")?,
        data.agent_id.unwrap_or("main"),
    )
}

/// G19, record side (PostToolUse): an Edit, Write or `MultiEdit` inside a project counts as an edit, and a
/// plain check command that passed counts as the proof for every edit so far.
fn g19_record(data: &HookInput<'_>) {
    if !check_enabled(Check::G19) || data.interrupted {
        return;
    }
    let edited = matches!(data.tool_name, Some("Edit" | "Write" | "MultiEdit"))
        .then(|| data.input_str("file_path").map(PathBuf::from))
        .flatten()
        .and_then(|f| {
            let f = if f.is_absolute() {
                f
            } else {
                data.call_cwd().join(f)
            };
            let dir = f.parent()?.to_path_buf();
            let root = proj::find_root(&dir)?;
            // a file in another repo than the one the caller works in (a scratch repo, a sibling
            // checkout) is not this caller's project work
            let own = proj::find_root(&data.call_cwd());
            let same = own.is_none_or(|o| {
                realpath(&o.to_string_lossy()) == realpath(&root.to_string_lossy())
            });
            same.then_some(dir)
        });
    let passed = data
        .bash_command()
        .is_some_and(|c| proj::plain_check(c).is_some());
    if edited.is_none() && !passed {
        return;
    }
    let _ = with_state(data, |state| {
        let Some(e) = agent_edits(state, data) else {
            return;
        };
        if let Some(dir) = edited {
            e.insert("n".into(), Value::from(counter(e.get("n")) + 1));
            e.insert(
                "dir".into(),
                Value::String(dir.to_string_lossy().into_owned()),
            );
        } else {
            e.insert("ok".into(), Value::from(counter(e.get("n"))));
        }
    });
}

/// True when the transcript shows `ps:autopilot` was started: a Skill call or the typed command.
fn autopilot_run(path: &Path) -> bool {
    static MARK: LazyLock<Regex> = LazyLock::new(|| {
        lit_re(r#""skill"\s*:\s*"ps:autopilot"|<command-name>/?ps:autopilot</command-name>"#)
    });
    let Ok(f) = fs::File::open(path) else {
        return false;
    };
    io::BufReader::new(f)
        .split(b'\n')
        .map_while(Result::ok)
        .any(|line| {
            has(&line, b"ps:autopilot")
                && std::str::from_utf8(&line).is_ok_and(|l| MARK.is_match(l))
        })
}

/// G19, gate side: the caller edited a project that has a fast check and no check passed since. Holds the
/// stop back once per edit count: the folder of the last edit is marked, so a second stop goes through
/// even when the stop hook flag is missing. Returns the message.
fn g19_gate(data: &HookInput<'_>) -> Option<String> {
    let dir = with_state(data, |state| {
        let e = agent_edits(state, data)?;
        let (n, ok, blocked) = (
            counter(e.get("n")),
            counter(e.get("ok")),
            counter(e.get("blocked")),
        );
        if n <= ok || n <= blocked {
            return None;
        }
        e.insert("blocked".into(), Value::from(n));
        e.get("dir").and_then(Value::as_str).map(PathBuf::from)
    })
    .flatten()?;
    let project = proj::detect(&dir)?;
    // the project's own fast check; a bare linter line is no stand-in for it, so a line made of
    // biome calls is dropped and the full check named instead
    let line_of = |cmds: Vec<proj::Cmd>| {
        cmds.iter()
            .filter(|c| c.argv.first().is_some_and(|a| basename(a) != "biome"))
            .map(proj::Cmd::line)
            .collect::<Vec<_>>()
            .join(" && ")
    };
    let mut line = line_of(project.check_fast());
    if line.is_empty() {
        line = line_of(project.check_full());
    }
    (!line.is_empty())
        .then(|| format!("G19: you edited files and no check passed since; run the fast check before you stop: {line}"))
}

/// G19 on Stop: only in an autopilot session. A normal chat is never held back.
fn g19_stop(data: &HookInput<'_>, t: &Turn) -> Option<String> {
    if busy(data, t) {
        return None; // work still runs (a helper, a background check) or the reply waits for its notice
    }
    let pending = with_state(data, |state| {
        let e = agent_edits(state, data)?;
        Some(
            counter(e.get("n")) > counter(e.get("ok"))
                && counter(e.get("n")) > counter(e.get("blocked")),
        )
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
/// superlite's own agents have a pool of their own (10, none when the week is critical).
fn g8_cap(data: &HookInput<'_>, ctx: &Ctx<'_>) -> Verdict {
    if data.tool_name != Some("Agent") {
        return Verdict::Allow;
    }
    let now = now_secs();
    let st = ctx.pace(); // read before the lock: pace may take a moment
    let kind = data.input_str("subagent_type");
    let key = slot_key(kind);
    let r = with_state(data, |state| {
        let mut spawns = live_spawns(state, key, now);
        let cap = if key == "spawns" {
            cap_for(st.state)
        } else {
            superlite_cap_for(st.state)
        };
        if spawns.len() >= cap {
            let msg = format!(
                "guard: {} {} running, the cap is {} (week {} used, budget {}, pace {}). The cap is the pace preset's setting ({} at once). Wait for a running helper to finish, then send the rest. There is no switch to raise it in a session.",
                spawns.len(),
                if spawns.len() == 1 {
                    "helper"
                } else {
                    "helpers"
                },
                cap,
                pct(st.used),
                pct(st.budget),
                st.state,
                cap
            );
            state.insert(key.into(), slots_value(&spawns));
            return Some(msg);
        }
        spawns.push(Slot {
            id: data.tool_use_id.map(String::from),
            t: now,
        });
        state.insert(key.into(), slots_value(&spawns));
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

/// G9: planning skills only in plan mode; no plan mode when the week is critical.
fn g9_plan_gate(data: &HookInput<'_>, ctx: &Ctx<'_>) -> Verdict {
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

/// Router: (new `tool_input` or None, note or None) for a main-thread Agent call.
///
/// Plan and a final code-reviewer: opus + high (sonnet + high when the week is critical). Every other
/// row never raises cost: the caller's model, else the agent file's `model:` pin, else the table's
/// (Explore, claude-code-guide, statusline-setup: haiku; the rest: sonnet), each only when not dearer
/// than the table's. Effort is the caller's low or medium, else the file's low or medium, else high;
/// none for haiku. xhigh and max never pass. `fork` is left alone.
fn route_agent(data: &HookInput<'_>, ctx: &Ctx<'_>) -> (Option<Value>, Option<String>) {
    if data.tool_name != Some("Agent") || !data.is_main_thread() {
        return (None, None);
    }
    let Some(ti) = data.tool_input.as_object() else {
        return (None, None);
    };
    let text = |key: &str| ti.get(key).and_then(Value::as_str);
    let kind = text("subagent_type");
    if kind == Some("fork") {
        return (None, None);
    }
    if let Some(k) = kind.filter(|k| k.starts_with(SUPERLITE)) {
        return route_superlite(k, ti, ctx); // the agent file pins model and effort
    }
    let desc = text("description").unwrap_or("");
    let head: String = text("prompt").unwrap_or("").chars().take(400).collect();
    let final_ = kind == Some("code-reviewer")
        && (FINAL_REVIEW.is_match(desc) || FINAL_REVIEW.is_match(&head));
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
        let table = if kind.is_some_and(|k| LOOKUP_AGENTS.contains(&k)) {
            Model::Haiku
        } else {
            Model::Sonnet
        };
        // the caller's own model name stays when it is not dearer than the table's
        // else the agent file's own pin, when it is not dearer; else the table's
        let pin = kind.and_then(|k| agent_file_meta(k, data.cwd));
        let pinned = |key: &str| {
            pin.as_ref()
                .and_then(|m| m.get(key))
                .map(|v| v.trim().to_ascii_lowercase())
        };
        model = match text("model") {
            Some(given) if Model::of(given).is_some_and(|g| g <= table) => given.to_string(),
            _ => pinned("model")
                .and_then(|m| Model::of(&m))
                .filter(|m| *m <= table)
                .unwrap_or(table)
                .name()
                .to_string(),
        };
        effort = match (text("effort"), pinned("effort").as_deref()) {
            (Some(e @ ("low" | "medium")), _) => e,
            (_, Some("low")) => "low",
            (_, Some("medium")) => "medium",
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

/// The front matter of the agent file whose `name:` is `kind`. The project folders come first
/// (from `cwd` up to the git root, nearest first; never `$HOME` itself), then `$HOME/.claude/agents`.
/// None on any error: the router then uses its table.
fn agent_file_meta(
    kind: &str,
    cwd: Option<&str>,
) -> Option<std::collections::BTreeMap<String, String>> {
    let home = paths::home();
    let mut left = 400; // files to read in all
    for dir in Path::new(cwd?).ancestors().take(24) {
        if dir == home || dir.parent().is_none() {
            break;
        }
        if let Some(m) = scan_agents(&dir.join(".claude/agents"), kind, 4, &mut left) {
            return Some(m);
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    scan_agents(&home.join(".claude/agents"), kind, 4, &mut left)
}

fn scan_agents(
    dir: &Path,
    kind: &str,
    depth: u32,
    left: &mut u32,
) -> Option<std::collections::BTreeMap<String, String>> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| Some(e.ok()?.path()))
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if depth > 0
                && let Some(m) = scan_agents(&p, kind, depth - 1, left)
            {
                return Some(m);
            }
        } else if p.extension().is_some_and(|x| x == "md") && p.is_file() {
            *left = left.checked_sub(1)?;
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let (meta, _) = agent_config::split_frontmatter(&text);
            if meta.get("name").is_some_and(|n| n.trim() == kind) {
                return Some(meta);
            }
        }
    }
    None
}

/// Router row for a superlite agent: left alone, except an opus-pinned one runs on sonnet when the
/// week is tight (superlite says a `model` overrides the agent file, so only then is one set).
fn route_superlite(
    kind: &str,
    ti: &Map<String, Value>,
    ctx: &Ctx<'_>,
) -> (Option<Value>, Option<String>) {
    if !SUPERLITE_OPUS.contains(&kind) {
        return (None, None);
    }
    let st = ctx.pace();
    if !st.state.is_tight() {
        return (None, None);
    }
    let mut new = ti.clone();
    new.insert("model".into(), Value::String("sonnet".into()));
    let msg = format!(
        "guard router: helper runs on sonnet - the week is {}, so opus became sonnet",
        st.state
    );
    (Some(Value::Object(new)), Some(msg))
}

// --- modes --------------------------------------------------------------------

type GuardFn = fn(&HookInput<'_>, &Ctx<'_>) -> Verdict;
/// A warning check: a note for the model, or nothing.
type WarnFn = fn(&HookInput<'_>, &Ctx<'_>) -> Option<String>;

fn pretool(data: &HookInput<'_>, ctx: &Ctx<'_>) -> Verdict {
    // g8_cap records the spawn it allows, so it runs after every other deny
    // each check runs only when its id is enabled (G14 and G15 also test their own id)
    let guards: [(Check, GuardFn); 14] = [
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
        (Check::G20, g20_big_read),
        (Check::G8, g8_cap),
    ];
    for (check, g) in guards {
        if !check_enabled(check) {
            continue;
        }
        let verdict = g(data, ctx);
        if gc::log_verdict(check, &verdict) {
            return verdict;
        }
    }
    Verdict::Allow
}

/// G16 (core): a Bash or MCP call that sends text off the machine and holds a private word or a
/// secret shape, or a body the guard cannot read. Returns the JSON line of an `ask`, or None. In
/// shadow mode (`GUARD_SHADOW`) it logs what it would ask and lets the call through.
fn outbound_ask(data: &HookInput<'_>) -> Option<String> {
    let tool = data.tool_name?;
    let Verdict::Ask(msg) = gc::run_gated(Check::G16, || {
        gc::g16_run(tool, data.tool_input, &data.call_cwd())
    }) else {
        return None;
    };
    Some(hook_json(
        "PreToolUse",
        vec![
            ("permissionDecision", json!("ask")),
            ("permissionDecisionReason", json!(msg)),
        ],
    ))
}

/// H1 (preset workflow): a Bash `sed -n` or `cat` of a code file `ctx` can outline gets a one-line
/// tip (none for text and data files such as md, json, yaml or toml), at most `H1_MAX` times per
/// session (the count lives in the per-session state, and only a shown tip counts). A call that reads nothing like that costs no file access.
fn h1_hint(data: &HookInput<'_>) -> Option<String> {
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
fn pretool_warn(data: &HookInput<'_>, ctx: &Ctx<'_>) -> Option<String> {
    let mut notes: Vec<String> = Vec::new();
    let mut updated: Option<Value> = None;
    if check_enabled(Check::Router)
        && let Some((u, note)) = gc::guarded(|| route_agent(data, ctx))
    {
        if u.is_some() || note.is_some() {
            gc::fire_log(Check::Router, FireKind::Route);
        }
        updated = u;
        notes.extend(note);
    }
    // G14 and G15 fix a Bash call instead of denying it: the same input with the new command
    if updated.is_none()
        && let Some(Some((cmd, done))) = gc::guarded(|| bash_rewrite(data))
    {
        let mut input = data.tool_input.clone();
        input["command"] = json!(cmd);
        updated = Some(input);
        for check in done {
            gc::fire_log(check, FireKind::Rewrite);
            notes.push(
                (if check == Check::G14 {
                    NOTE_G14_REWRITE
                } else {
                    NOTE_G15_REWRITE
                })
                .to_string(),
            );
        }
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
            gc::guarded(|| w5_repeat(data)).flatten(),
        );
    }
    if notes.is_empty() && updated.is_none() {
        return None;
    }
    let mut fields = Vec::new();
    if let Some(u) = updated {
        fields.push(("permissionDecision", json!("allow")));
        fields.push(("updatedInput", u));
    }
    fields.push(("additionalContext", json!(notes.join("\n"))));
    Some(hook_json("PreToolUse", fields))
}

/// The note for a pace state that was not announced yet (`last` = the state announced before).
fn pace_note(st: &PaceSt, last: Option<&str>) -> Option<String> {
    let (used, budget) = (pct(st.used), pct(st.budget));
    match st.state {
        PaceState::Critical => Some(format!(
            "guard: the week is critical ({used} used, budget {budget} today). Sonnet only, no helpers. Tell the user once."
        )),
        PaceState::Ahead if last != Some("ahead") => Some(format!(
            "guard: the week is ahead of pace ({used} used, budget {budget} today). Nothing is limited: helpers, models and plan mode stay as usual until the week is critical. Tell the user once."
        )),
        _ => None,
    }
}

/// G3 nudge and pace notes: JSON for stdout (or None); counters live in the per-session state file.
/// How many assistant turns back H2 looks for a call of the skill it would name.
const H2_TURNS: usize = 3;

/// True when `/ps:<skill>` was typed or the Skill tool called with it in the last `H2_TURNS`
/// assistant turns of the transcript (a turn runs from one human prompt to the next).
fn skill_called_recently(path: &Path, skill: &str) -> bool {
    let full = format!("ps:{skill}");
    let typed = format!("{full}</command-name>");
    let Some(lines) = rev_lines(path) else {
        return false;
    };
    let (mut turns, mut seen_assistant) = (0, false);
    for line in lines {
        let Some(rec) = loads(&line) else { continue };
        if get(&rec, "type").as_str() == Some("assistant") {
            seen_assistant = true;
            for b in blocks_newest_first(&rec) {
                if get(b, "type").as_str() == Some("tool_use")
                    && get(b, "name").as_str() == Some("Skill")
                    && get(get(b, "input"), "skill").as_str() == Some(full.as_str())
                {
                    return true;
                }
            }
        } else if is_human_prompt(&rec) {
            if has(&line, typed.as_bytes()) {
                return true;
            }
            if seen_assistant {
                turns += 1;
                seen_assistant = false;
                if turns >= H2_TURNS {
                    return false;
                }
            }
        }
    }
    false
}

/// H2 (preset workflow, UserPromptSubmit): a phrase of the user that names a skill adds one line,
/// `The user said '<phrase>': run /ps:<name> first.` One hint per prompt, none when the skill was
/// called in the last `H2_TURNS` assistant turns. Arhiviraj and other Serbian phrases are out of scope.
fn h2_hint(data: &HookInput<'_>) -> Option<String> {
    let (skill, phrase) = gc::h2_match(data.prompt?)?;
    if data
        .transcript_path
        .is_some_and(|p| skill_called_recently(Path::new(p), skill))
    {
        return None;
    }
    Some(format!("The user said '{phrase}': run /ps:{skill} first."))
}

fn prompt(data: &HookInput<'_>, ctx: &Ctx<'_>) -> Option<String> {
    // Rescan the transcripts only when the last scan is partial, older than 2 min or from another
    // week. A fresh status line does not skip it: the scan also saves the calibration.
    if check_enabled(Check::Pace) {
        let _ = std::panic::catch_unwind(|| {
            let now = paths::unix_now();
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
    if check_enabled(Check::H2)
        && let Some(Some(hint)) = gc::guarded(|| h2_hint(data))
    {
        gc::fire_log(Check::H2, FireKind::Hint);
        notes.push(hint);
    }
    if notes.is_empty() {
        None
    } else {
        Some(hook_output("UserPromptSubmit", &notes.join("\n")))
    }
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
    let Ok(s) = String::from_utf8(raw) else {
        return 0;
    };
    let Ok(data) = serde_json::from_str::<Value>(&s) else {
        return 0;
    };
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
                out = outbound_ask(&input);
                // a command G14 or G15 can fix never goes to the user's approval unfixed: deny with
                // the fixed command, and G16 asks on that clean call
                let fixed = out
                    .as_ref()
                    .and_then(|_| gc::guarded(|| bash_rewrite(&input)))
                    .flatten();
                if let Some((cmd, _)) = fixed {
                    out = None;
                    Some(format!(
                        "guard: this command needs a fix (G14 or G15) and also sends text off the machine (G16 would ask). Send this fixed command instead: {cmd}"
                    ))
                } else {
                    if out.is_none() {
                        out = pretool_warn(&input, &ctx);
                    }
                    None
                }
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
            (
                "Make truncate() return at most 500 characters. Hand back at most 4,000 chars.",
                Some(4000),
            ),
            (
                "The API summary field holds 300 chars. Hand back at most 2,000 chars.",
                Some(2000),
            ),
            (
                "Hand back \u{2264} 3,000 chars. In researcher.md set a hard cap of 2,500 chars on the final message.",
                Some(3000),
            ),
            (
                "Hand back at most 4,000 chars. The summary field holds 300 chars.",
                Some(4000),
            ),
            (
                "Deadline: answer within 8 minutes. Stay \u{2264} 4,000 chars.",
                Some(4000),
            ),
            (
                "Keep it under 2,500 characters. Pad the file to 900 chars.",
                Some(2500),
            ),
            (
                "Reply in 2,000 chars. Final message under 1500 characters. Then hand back 800 chars.",
                Some(800),
            ),
            ("Hand back 50 chars.", None),
            (
                "Write a description of at most 160 characters for the page.",
                None,
            ),
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
            let Ok(text) = fs::read_to_string(e.path().join("rules.md")) else {
                continue;
            };
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix("guard:"))
                .unwrap_or("[]");
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
        let core: Vec<&str> = gc::REGISTRY
            .iter()
            .filter(|(_, p)| *p == "core")
            .map(|(c, _)| c.id())
            .collect();
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
        let text = recs
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&path, text).unwrap();
        assert_eq!(context_size(Some(&path.to_string_lossy())), 250_000);
        assert_eq!(
            context_size(Some(&dir.path().join("nope.jsonl").to_string_lossy())),
            0
        );
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
        assert_eq!(
            timestamp(&json!("2026-10-19T12:00:00.123+02:00")),
            Some(1_792_404_000)
        );
        assert_eq!(timestamp(&json!("nope")), None);
        assert_eq!(timestamp(&json!("2026-13-45T00:00:00Z")), None);
        assert_eq!(timestamp(&json!(5)), None);
    }
}
