//! Claude Code guard hook (PreToolUse, UserPromptSubmit, SubagentStop, Stop), a port of
//! `claude/hooks/guard.py`. Deny = message on stderr + exit 2. Allow = exit 0 silently.
//! Warn = exit 0 with hookSpecificOutput.additionalContext JSON on stdout.
//! Fail-open: any bad input, panic or unknown mode exits 0 with no output.
//!
//! Python semantics are mirrored on purpose (truthiness, `str()`, char-based lengths). Where the
//! Python code would raise and so fail open, the Rust code returns `Err(Fail)`.

use regex::Regex;
use serde_json::{Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "../pace_core.rs"]
mod pace;

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
const MODEL_RANK: &[(&str, i32)] = &[("haiku", 0), ("sonnet", 1), ("opus", 2), ("fable", 3)];
const BROWSER_PREFIXES: &[&str] = &["mcp__Claude_Browser__", "mcp__claude-in-chrome__"];
const BROWSER_WARN_AT: usize = 5;
const BROWSER_WARN_EVERY: usize = 20;
const SPAWN_CAP: usize = 15;
const TOKEN_CAP: i64 = 1_500_000;
const BUDGET_EVERY: usize = 5;
const GIT_TIMEOUT: f64 = 2.0;
const CLOCK_TURN_MIN: i64 = 10;
const G13_AFTER: usize = 8;
const PLAN_TOOL: &str = "mcp__ccd_turn__update_plan";
const WRAP_TOOL: &str = "mcp__ccd_turn__wrap_up";
const SECRET_FILE: &str = "~/.config/zsh/local.zsh";

const MSG_SCRATCH: &str = "guard: this agent writes only scratch files; write under the scratchpad, $TMPDIR, .agents/ or .superpowers/.";
const MSG_POLL: &str = "guard: no polling loops at big context; use Monitor / SendMessage notify_when_idle, or one final report.";
const MSG_ASK: &str = "guard: one AskUserQuestion per turn; put all questions in one call.";
const MSG_HEAVY_TAIL: &str = "subagent loads every tool and skill (about 60K tokens; `worker` starts near 15-20K). Use `worker` (edits, commands), `researcher` (web), `Explore` (search), `task-implementer` (one plan task) or `Plan` (planning).";
const MSG_PLAN_GATE: &str = "guard: plans run on Opus only in plan mode (opusplan). Call EnterPlanMode first, then save the plan file after you approve it (autopilot: send a `Plan` helper instead).";
const MSG_PLAN_GATE_APP: &str = "guard: in the desktop app plan mode does not switch a Sonnet session to Opus. For this plan, send a `Plan` helper (Opus by the router), or pick Opus in the model menu.";
const MSG_G13: &str = "G13: 8 tool calls with no text. Write one short status line for the user now, then go on.";
const MSG_G14: &str = "G14: BSD sed -i on macOS differs from GNU sed and breaks files. Use the Edit tool, or python3 for bulk edits.";
const MSG_G15_FILE: &str = "G15: never read local.zsh: it holds API keys. Use `test -f` or `git check-ignore` for facts about it, or ask the user.";
const MSG_G15_SEARCH: &str = "G15: this search can reach local.zsh (API keys). Exclude it: `rg -g '!local.zsh' ...` or `grep --exclude=local.zsh ...`.";
const MSG_G15_PS: &str = "G15: this prints full command lines, which can hold API keys. Use `pgrep -f <name>` for pids, or `ps -o pid=,etime=,ucomm= -p <pid>`.";

static DEADLINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)deadline|time ?(?:box|limit)|stop at|\d+ ?-?(?:m|mins?|minutes?|h|hrs?|hours?)\b").unwrap()
});
static CAP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)hand[ -]?back|at most \d[\d,]*|\d[\d,]* ?(?:chars|characters|words|lines)\b|under \d[\d,]*|max(?:imum)? \d[\d,]*|one line|≤ ?\d[\d,]*",
    )
    .unwrap()
});
static CAP_NUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d[\d,]*) ?(?:chars|characters)\b").unwrap());
static CAP_HANDBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)hand[ -]?backs?\b").unwrap());
static CAP_STRONG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)final message|reply|response|respond|output").unwrap());
static CAP_WEAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)answer|return|report|summary").unwrap());
static CAP_BOUND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:≤|<=|at most|under|max(?:imum)?|stay|keep|limit|no more than|within)\W*$").unwrap()
});
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.\s|\n").unwrap());
static PANEL_ANGLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:panel|angle|re-?review)\b").unwrap());
static FINAL_REVIEW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bfinal\b.{0,30}\breview|\bwhole[- ]branch\b|\bbranch[- ]review\b").unwrap()
});
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
static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b\d{1,2}:\d{2}\b|\b\d+(?:\.\d+)?\s?(?:min(?:ute)?s?|h|hrs?|hours?)\b").unwrap()
});
static ISO_TS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)(?:\.\d+)?(Z|[+-]\d\d:?\d\d)?\z").unwrap()
});
static GIT_COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bgit\b[^;&|\n]*?\bcommit\b").unwrap());
static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w{2,}").unwrap());
static ENTRYPOINT: LazyLock<regex::bytes::Regex> =
    LazyLock::new(|| regex::bytes::Regex::new(r#""entrypoint"\s*:\s*"([^"]*)""#).unwrap());
static GLOB_CHARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*?\[]").unwrap());

/// Python would have raised here: the whole hook fails open.
struct Fail;
type Res = Result<Option<String>, Fail>;

static NULL: Value = Value::Null;

// --- Python-flavoured helpers -------------------------------------------------

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `v.get(k)` on a dict, None (Null) when missing or not a dict.
fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&NULL)
}

/// `x or {}` followed by `.get(...)`: falsy gives an empty dict, a truthy non-dict raises.
fn obj_or_empty(v: &Value) -> Result<&Value, Fail> {
    if !truthy(v) {
        Ok(&NULL)
    } else if v.is_object() {
        Ok(v)
    } else {
        Err(Fail)
    }
}

/// Python `str(v)` (approximate for containers and floats).
fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `str(v or "")`.
fn py_str_or_empty(v: &Value) -> String {
    if truthy(v) { py_str(v) } else { String::new() }
}

/// Python `int(v)` for the shapes JSON can hold; None where Python raises.
fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(*b as i64),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// `_num`: int/float that is not a bool, else 0.
fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn is_blank(b: &[u8]) -> bool {
    b.iter().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

fn loads(line: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(line).ok().filter(|v| v.is_object())
}

/// `json.dumps(str)` with ensure_ascii.
fn py_json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `{"hookSpecificOutput": {...}}` line exactly as Python's json.dumps prints it.
fn hook_output(event: &str, context: &str) -> String {
    format!(
        "{{\"hookSpecificOutput\": {{\"hookEventName\": {}, \"additionalContext\": {}}}}}",
        py_json_str(event),
        py_json_str(context)
    )
}

fn sanitize_key(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' })
        .collect()
}

/// `json.dumps(v)` with ensure_ascii and the default ", " and ": " separators.
fn py_dumps(v: &Value) -> String {
    match v {
        Value::String(s) => py_json_str(s),
        Value::Array(a) => format!("[{}]", a.iter().map(py_dumps).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter().map(|(k, x)| format!("{}: {}", py_json_str(k), py_dumps(x))).collect::<Vec<_>>().join(", ")
        ),
        other => other.to_string(),
    }
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
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// macOS rules (BSD sed, case-folded paths, ps). GUARD_PLATFORM=darwin|linux|win32 overrides the build
/// target, so a Linux run can test the macOS branches (the Python hook tests set it from sys.platform).
fn is_macos() -> bool {
    match env::var("GUARD_PLATFORM") {
        Ok(p) if !p.is_empty() => p == "darwin",
        _ => cfg!(target_os = "macos"),
    }
}

/// GUARD_OFF="G10,G13" (env, comma or space list) turns those checks off.
fn check_off(gid: &str) -> bool {
    let off = env::var("GUARD_OFF").unwrap_or_default().to_uppercase();
    off.split(|c: char| c.is_whitespace() || c == ',').any(|p| p == gid.to_uppercase())
}

/// Locked read-modify-write of the per-session state file (the keys count, pace, spawns).
fn with_state<T>(data: &Value, f: impl FnOnce(&mut Map<String, Value>) -> T) -> Result<T, Fail> {
    let sid_v = get(data, "session_id");
    let sid = sanitize_key(&if truthy(sid_v) { py_str(sid_v) } else { "unknown".to_string() });
    let dir = state_dir();
    fs::create_dir_all(&dir).map_err(|_| Fail)?;
    let path = dir.join(format!("{}.json", sid));
    let lock = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{}.json.lock", sid)))
        .map_err(|_| Fail)?;
    let _ = lock.lock(); // closing the file releases the lock; no lock support is not fatal
    let mut state = match fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let out = f(&mut state);
    let tmp = dir.join(format!("{}.json.{}.tmp", sid, std::process::id()));
    fs::write(&tmp, py_dumps(&Value::Object(state))).map_err(|_| Fail)?;
    fs::rename(&tmp, &path).map_err(|_| Fail)?;
    Ok(out)
}

#[derive(Clone)]
struct Slot {
    id: Value,
    t: f64,
}

/// Running slots, oldest first; a slot written as a plain time loads without an id.
fn live_spawns(state: &Map<String, Value>, now: f64) -> Vec<Slot> {
    let mut slots = Vec::new();
    if let Some(Value::Array(a)) = state.get("spawns") {
        for s in a {
            let (t, id) = match s {
                Value::Object(o) => (o.get("t").unwrap_or(&NULL), o.get("id").unwrap_or(&NULL).clone()),
                other => (other, Value::Null),
            };
            if let Value::Number(n) = t
                && let Some(t) = n.as_f64()
                    && now - t < SPAWN_TTL {
                        slots.push(Slot { id, t });
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
                m.insert("id".into(), s.id.clone());
                m.insert("t".into(), serde_json::Number::from_f64(s.t).map(Value::Number).unwrap_or(Value::Null));
                Value::Object(m)
            })
            .collect(),
    )
}

fn read_counter(file: &Path, key: &str) -> i64 {
    let Ok(s) = fs::read_to_string(file) else { return 0 };
    let Ok(v) = serde_json::from_str::<Value>(&s) else { return 0 };
    if !v.is_object() {
        return 0;
    }
    match v.get(key) {
        None => 0,
        Some(x) => py_int(x).unwrap_or(0),
    }
}

// --- transcript reading -------------------------------------------------------

struct RevLines {
    f: fs::File,
    pos: u64,
    chunk: u64,
    buf: Vec<u8>,
    pending: Vec<Vec<u8>>, // forward order; pop() yields the newest
    finished: bool,
}

fn rev_lines(path: &Path) -> Result<RevLines, Fail> {
    let mut f = fs::File::open(path).map_err(|_| Fail)?;
    let len = f.seek(SeekFrom::End(0)).map_err(|_| Fail)?;
    Ok(RevLines { f, pos: len, chunk: 1 << 16, buf: Vec::new(), pending: Vec::new(), finished: false })
}

impl Iterator for RevLines {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some(l) = self.pending.pop() {
                return Some(l);
            }
            if self.pos == 0 {
                if self.finished {
                    return None;
                }
                self.finished = true;
                let b = std::mem::take(&mut self.buf);
                return if is_blank(&b) { None } else { Some(b) };
            }
            let n = self.chunk.min(self.pos);
            self.pos -= n;
            let mut data = vec![0u8; n as usize];
            if self.f.seek(SeekFrom::Start(self.pos)).is_err() || self.f.read_exact(&mut data).is_err() {
                self.pos = 0;
                self.buf.clear();
                continue;
            }
            data.extend_from_slice(&self.buf);
            let parts: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
            if parts.len() == 1 {
                self.chunk *= 2; // one giant line: read bigger pieces
            }
            let first = parts[0].to_vec();
            let pend: Vec<Vec<u8>> = parts[1..].iter().filter(|l| !is_blank(l)).map(|l| l.to_vec()).collect();
            self.buf = first;
            self.pending = pend;
        }
    }
}

fn context_size(path: &Value) -> i64 {
    let Some(p) = path.as_str().filter(|s| !s.is_empty()) else { return 0 };
    let Ok(lines) = rev_lines(Path::new(p)) else { return 0 };
    for line in lines {
        if (has(&line, b"compact_boundary") || has(&line, b"isCompactSummary"))
            && let Some(rec) = loads(&line)
                && (get(&rec, "subtype").as_str() == Some("compact_boundary") || truthy(get(&rec, "isCompactSummary"))) {
                    return 0;
                }
        if !has(&line, b"\"assistant\"") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if get(&rec, "type").as_str() != Some("assistant") {
            continue;
        }
        let usage = get(get(&rec, "message"), "usage");
        if !usage.is_object() {
            continue;
        }
        let total = num(get(usage, "input_tokens"))
            + num(get(usage, "cache_creation_input_tokens"))
            + num(get(usage, "cache_read_input_tokens"));
        if total > 0.0 {
            return total as i64;
        }
    }
    0
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

fn asks_since_last_prompt(path: &Path, tool_use_id: &Value, tool_input: &Value) -> Result<usize, Fail> {
    let mut found: Vec<(Value, Value)> = Vec::new(); // newest first
    let mut denied: Vec<Value> = Vec::new();
    for line in rev_lines(path)? {
        if !has(&line, b"\"user\"") && !has(&line, b"AskUserQuestion") && !has(&line, b"askme") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if is_human_prompt(&rec) {
            if has(&line, b"ps:askme") {
                return Ok(0);
            }
            break;
        }
        let rtype = get(&rec, "type").as_str();
        if rtype == Some("user") {
            if let Some(arr) = message_content(&rec).as_array() {
                for b in arr {
                    if get(b, "type").as_str() == Some("tool_result") && truthy(get(b, "is_error")) {
                        denied.push(get(b, "tool_use_id").clone());
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
            if get(b, "name").as_str() == Some("Skill") {
                let input = obj_or_empty(get(b, "input"))?;
                if py_str(get(input, "skill")).contains("askme") {
                    return Ok(0);
                }
            }
            if get(b, "type").as_str() == Some("tool_use") && get(b, "name").as_str() == Some("AskUserQuestion") {
                let id = get(b, "id");
                if !denied.contains(id) {
                    found.push((id.clone(), get(b, "input").clone()));
                }
            }
        }
    }
    // Only asks made BEFORE the current call count.
    for (idx, (i, inp)) in found.iter().enumerate() {
        let is_current = if truthy(tool_use_id) { i == tool_use_id } else { inp == tool_input };
        if is_current {
            return Ok(found.len() - idx - 1);
        }
    }
    Ok(found.len())
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
        let hi = m.get(2).map(|x| big(x.as_str())).unwrap_or(lo);
        let step = m.get(3).map(|x| big(x.as_str())).unwrap_or(1);
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
        minute = format!("0-59/{}", n); // */45 fires at :00 and :45, so the gap is 15
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
        && DIGITS.is_match(&minute) {
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

/// Seconds of a leading interval token (30s, 5m, 10min, 1h, 2h30m) or of "every <token>".
fn loop_interval_seconds(args: &str) -> Option<i64> {
    let s = args.trim();
    if let Some(v) = loop_chain(s, 0) {
        return Some(v);
    }
    let mut from = 0;
    while let Some(rel) = s[from..].find("every ") {
        let idx = from + rel;
        let boundary = s[..idx].chars().next_back().map(|c| !(c.is_alphanumeric() || c == '_')).unwrap_or(true);
        if boundary
            && let Some(v) = loop_chain(s, idx + 6) {
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
    state: String,
    used: Option<f64>,
    budget: Option<f64>,
}

fn pace_status() -> PaceSt {
    let st = std::panic::catch_unwind(|| pace::status(None)).ok();
    match st {
        Some(s) if matches!(s.state.as_str(), "ok" | "unknown" | "ahead" | "critical") => {
            PaceSt { state: s.state, used: s.used, budget: Some(s.budget) }
        }
        _ => PaceSt { state: "unknown".into(), used: None, budget: None },
    }
}

fn cap_for(state: &str) -> usize {
    match state {
        "ahead" => 1,
        "critical" => 0,
        _ => 3,
    }
}

/// `"%.0f%%" % v`, or "?".
fn pct(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{:.0}%", x),
        None => "?".to_string(),
    }
}

struct Ctx<'a> {
    data: &'a Value,
    size: Cell<Option<i64>>,
    pace: RefCell<Option<PaceSt>>,
}

impl<'a> Ctx<'a> {
    fn new(data: &'a Value) -> Self {
        Ctx { data, size: Cell::new(None), pace: RefCell::new(None) }
    }
    fn path_val(&self) -> &'a Value {
        get(self.data, "transcript_path")
    }
    fn size(&self) -> i64 {
        if let Some(s) = self.size.get() {
            return s;
        }
        let s = context_size(self.path_val());
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
    /// `ctx.path`: Ok(None) when falsy; a truthy non-string fails open when used.
    fn path(&self) -> Result<Option<&'a str>, Fail> {
        let v = self.path_val();
        if !truthy(v) {
            return Ok(None);
        }
        v.as_str().map(Some).ok_or(Fail)
    }
}

fn tool_name(data: &Value) -> Option<&str> {
    get(data, "tool_name").as_str()
}

fn is_main_thread(data: &Value) -> bool {
    !truthy(get(data, "agent_id")) && !py_str_or_empty(get(data, "transcript_path")).contains("/subagents/")
}

fn g3_pretool(data: &Value, ctx: &Ctx) -> Res {
    if tool_name(data) == Some("Agent") && ctx.size() > SPAWN_CONTEXT {
        return Ok(Some(format!(
            "guard: context is {}K (> {}K); do this step yourself instead of spawning a helper. Keep PROGRESS.md current; auto-compact carries the run.",
            ctx.size() / 1000,
            SPAWN_CONTEXT / 1000
        )));
    }
    Ok(None)
}

/// G1: the brief names a deadline and a hand-back cap (the router fills model and effort).
fn g1_agent(data: &Value, _ctx: &Ctx) -> Res {
    if tool_name(data) != Some("Agent") || !get(data, "tool_input").is_object() {
        return Ok(None);
    }
    let ti = get(data, "tool_input");
    let prompt_text = get(ti, "prompt").as_str().unwrap_or("");
    let mut missing = Vec::new();
    if !DEADLINE_RE.is_match(prompt_text) {
        missing.push("a deadline");
    }
    if !CAP_RE.is_match(prompt_text) {
        missing.push("a hand-back cap");
    }
    if !missing.is_empty() {
        return Ok(Some(format!(
            "guard: Agent prompt needs {}. Add `Deadline: 25 min.` and `Hand back at most 4,000 chars; write the rest to a file.`",
            missing.join(" and ")
        )));
    }
    Ok(None)
}

/// G7: refuse general-purpose, claude and an untyped Agent call.
fn g7_heavy(data: &Value, _ctx: &Ctx) -> Res {
    if tool_name(data) != Some("Agent") || !get(data, "tool_input").is_object() {
        return Ok(None);
    }
    let kind_v = get(get(data, "tool_input"), "subagent_type");
    if kind_v.is_array() || kind_v.is_object() {
        return Err(Fail); // unhashable in Python
    }
    let kind = if truthy(kind_v) { py_str(kind_v) } else { "general-purpose".to_string() };
    if HEAVY_AGENTS.contains(&kind.as_str()) {
        return Ok(Some(format!("guard: a {} {}", kind, MSG_HEAVY_TAIL)));
    }
    Ok(None)
}

fn g2_polling(data: &Value, ctx: &Ctx) -> Res {
    let tool = match tool_name(data) {
        Some(t @ ("CronCreate" | "ScheduleWakeup" | "Skill")) => t,
        _ => return Ok(None),
    };
    let ti = obj_or_empty(get(data, "tool_input"))?;
    if tool == "Skill" && get(ti, "skill").as_str() != Some("loop") {
        return Ok(None);
    }
    if tool == "CronCreate" && get(ti, "recurring") == &Value::Bool(false) {
        return Ok(None);
    }
    if ctx.size() <= POLL_CONTEXT {
        return Ok(None);
    }
    let bad = if tool == "CronCreate" {
        cron_interval_minutes(&py_str(get(ti, "cron"))).map(|iv| iv < MIN_INTERVAL_MIN).unwrap_or(false)
    } else if tool == "ScheduleWakeup" {
        match get(ti, "delaySeconds") {
            Value::Bool(b) => ((*b) as i64) < MIN_INTERVAL_MIN * 60,
            Value::Number(n) => n.as_f64().map(|d| d < (MIN_INTERVAL_MIN * 60) as f64).unwrap_or(false),
            _ => false,
        }
    } else {
        loop_interval_seconds(&py_str_or_empty(get(ti, "args")))
            .map(|s| s < MIN_INTERVAL_MIN * 60)
            .unwrap_or(false)
    };
    Ok(if bad { Some(MSG_POLL.to_string()) } else { None })
}

fn g4_ask(data: &Value, ctx: &Ctx) -> Res {
    if tool_name(data) != Some("AskUserQuestion") {
        return Ok(None);
    }
    let Some(path) = ctx.path()? else { return Ok(None) };
    let n = asks_since_last_prompt(Path::new(path), get(data, "tool_use_id"), get(data, "tool_input"))?;
    Ok(if n >= 1 { Some(MSG_ASK.to_string()) } else { None })
}

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
            && !text.is_empty() {
                return Some(text);
            }
    }
    None
}

fn subagent_transcript(data: &Value) -> Option<String> {
    if let Some(atp) = get(data, "agent_transcript_path").as_str()
        && !atp.is_empty() {
            return Some(atp.to_string());
        }
    let tp = get(data, "transcript_path").as_str().filter(|s| !s.is_empty())?;
    if tp.contains("/subagents/") {
        return Some(tp.to_string());
    }
    let aid = get(data, "agent_id");
    if !truthy(aid) {
        return None;
    }
    let base = tp.strip_suffix(".jsonl").unwrap_or(tp);
    Some(format!("{}/subagents/agent-{}.jsonl", base, py_str(aid)))
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
        let window = format!("{} {}", before, after);
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

fn default_cap(agent_type: &Value) -> i64 {
    match agent_type.as_str() {
        Some("task-implementer" | "task-reviewer" | "researcher" | "ui-verifier" | "worker") => 3000,
        Some("code-reviewer") => 4000,
        Some("repo-scanner") => 2000,
        _ => HANDBACK_DEFAULT_CAP,
    }
}

fn handback_cap(data: &Value) -> i64 {
    let tr = subagent_transcript(data);
    let prompt = subagent_prompt(tr.as_deref());
    match prompt.as_deref().and_then(prompt_cap) {
        Some(cap) if cap != 0 => cap,
        _ => default_cap(get(data, "agent_type")),
    }
}

/// G5: deny a hand-back over cap x 1.10, at most twice per agent; None = allow.
fn check_handback(data: &Value, text: &Value) -> Option<String> {
    let text = text.as_str().filter(|s| !s.is_empty())?;
    let cap = handback_cap(data);
    let n = text.chars().count();
    if (n as f64) <= (cap as f64) * HANDBACK_SLACK {
        return None;
    }
    let ident = [get(data, "agent_id"), get(data, "session_id")]
        .into_iter()
        .find(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "unknown".to_string());
    let dir = state_dir();
    let state_file = dir.join(format!("handback-{}.json", sanitize_key(&ident)));
    let denies = read_counter(&state_file, "denies");
    if denies >= HANDBACK_MAX_DENIES {
        return None;
    }
    if fs::create_dir_all(&dir).is_err() || fs::write(&state_file, format!("{{\"denies\": {}}}", denies + 1)).is_err() {
        return None; // no counter, no deny: a deny without one could repeat forever
    }
    Some(format!(
        "guard: hand-back is {} chars; the cap is {}. Write the full text to a file (the path the caller named, or the scratchpad), then hand back at most {} chars: status on line 1, key results, the file path. No Write tool: cut to the top findings.",
        n, cap, cap
    ))
}

fn g5_handback(data: &Value, _ctx: &Ctx) -> Res {
    if tool_name(data) != Some("SubagentHandback") {
        return Ok(None);
    }
    let ti = get(data, "tool_input");
    let msg = if ti.is_object() { get(ti, "message") } else { &NULL };
    Ok(check_handback(data, msg))
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// (hand-off path, assistant text written after it) for the newest hand-off Write since the last
/// human prompt, or (None, "").
fn handoff_this_turn(path: &Path) -> Result<(Option<String>, String), Fail> {
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
            } else if btype == Some("tool_use") && get(b, "name").as_str() == Some("Write") {
                let input = obj_or_empty(get(b, "input"))?;
                if let Some(fp) = get(input, "file_path").as_str()
                    && HANDOFF_PATH.is_match(fp) {
                        texts.reverse();
                        return Ok((Some(fp.to_string()), texts.join("\n")));
                    }
            }
        }
    }
    Ok((None, String::new()))
}

/// The reply holds the paste line and a markdown link to the file.
fn handoff_given(fp: &str, text: &str) -> bool {
    let name = basename(fp);
    if !text.contains(&format!("Read {} and do it.", fp)) {
        return false;
    }
    LINK_TARGET.captures_iter(text).any(|c| c[1].ends_with(name))
}

fn g6_archive(data: &Value, ctx: &Ctx) -> Res {
    if tool_name(data) != Some(ARCHIVE_TOOL) {
        return Ok(None);
    }
    let Some(path) = ctx.path()? else { return Ok(None) };
    let ti = get(data, "tool_input");
    let sid = get(ti, "session_id");
    if !(sid.is_null() || sid.as_str() == Some("self")) {
        return Ok(None);
    }
    let (fp, _) = handoff_this_turn(Path::new(path))?;
    let fp = fp.filter(|f| Path::new(f).exists()); // a hand-off deleted since is not owed to the user
    Ok(fp.map(|fp| {
        let name = basename(&fp).to_string();
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
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
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
                && in_turn && t.last_text.is_none() {
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
            continue;
        } else if matches!(name, Some("Edit" | "Write" | "NotebookEdit")) {
            let fp = get(inp, "file_path");
            let p = if truthy(fp) { fp } else { get(inp, "notebook_path") };
            if let Some(p) = p.as_str().filter(|s| !s.is_empty()) {
                t.paths.push(p.to_string());
            }
        } else if name == Some("Bash") {
            if GIT_COMMIT.is_match(&py_str_or_empty(get(inp, "command"))) {
                t.committed = true;
            }
        } else if name == Some("AskUserQuestion") {
            t.asked = true;
        }
    }
}

/// One pass from the end of the transcript: this turn's facts and the update_plan calls.
fn scan_turn(path: &Path) -> Result<Turn, Fail> {
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
    Ok(t)
}

fn step_name(item: &Value) -> String {
    if item.is_object() {
        for k in ["content", "step", "title", "name", "text", "label", "description"] {
            if let Some(s) = get(item, k).as_str().filter(|s| !s.is_empty()) {
                return s.to_string();
            }
        }
    }
    py_str(item)
}

fn step_status(item: &Value) -> String {
    if !item.is_object() {
        return "pending".to_string();
    }
    let st = get(item, "status");
    let s = if truthy(st) { py_str(st) } else { "pending".to_string() }.to_lowercase();
    if matches!(s.as_str(), "in_progress" | "in-progress" | "active") { "in_progress".to_string() } else { s }
}

fn apply_plan(steps: &mut Vec<(String, String)>, inp: &Value) {
    if let Some(items) = get(inp, "items").as_array() {
        *steps = items.iter().map(|i| (step_name(i), step_status(i))).collect();
    }
    fn mark(steps: &mut [(String, String)], n: &Value, status: &str) {
        if let Value::Number(num) = n
            && let Some(i) = num.as_i64()
                && i >= 1 && (i as usize) <= steps.len() {
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

/// Run git; Ok(None) when it failed, Err when it timed out or cannot start.
fn git(args: &[&str], cwd: &str, deadline: Instant) -> Result<Option<String>, Fail> {
    let left = deadline.checked_duration_since(Instant::now()).ok_or(Fail)?;
    let mut child = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| Fail)?;
    let mut out = child.stdout.take().ok_or(Fail)?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(left) {
        Ok(buf) => {
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            Ok(if ok { Some(String::from_utf8_lossy(&buf).into_owned()) } else { None })
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(Fail)
        }
    }
}

/// True when any human prompt of the session says not to push or commit.
fn said_no_push(path: &Path) -> Result<bool, Fail> {
    for line in rev_lines(path)? {
        if !has(&line, b"\"user\"") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if !is_human_prompt(&rec) {
            continue;
        }
        if NO_PUSH.is_match(&text_of(message_content(&rec)).unwrap_or_default()) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn path_join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_string()
    } else if a.ends_with('/') {
        format!("{}{}", a, b)
    } else {
        format!("{}/{}", a, b)
    }
}

/// G10: this turn's work is uncommitted or unpushed in the session's repo.
fn g10_commit(data: &Value, t: &Turn) -> Res {
    let Some(cwd) = get(data, "cwd").as_str().filter(|s| !s.is_empty()) else { return Ok(None) };
    if t.paths.is_empty() && !t.committed {
        return Ok(None);
    }
    let deadline = Instant::now() + Duration::from_secs_f64(GIT_TIMEOUT);
    let Some(top) = git(&["rev-parse", "--show-toplevel"], cwd, deadline)? else { return Ok(None) };
    let count = git(&["rev-list", "--count", "@{u}..HEAD"], cwd, deadline)?; // fails without an upstream
    let top = realpath(top.trim());
    let unpushed: i64 = match (&count, t.committed) {
        (Some(c), true) => c.trim().parse().map_err(|_| Fail)?,
        _ => 0,
    };
    let mut dirty = 0;
    if !t.paths.is_empty() {
        let Some(status) = git(&["status", "--porcelain", "-z", "-uall"], cwd, deadline)? else { return Ok(None) };
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
        return Ok(None);
    }
    let tp = get(data, "transcript_path").as_str().ok_or(Fail)?;
    if said_no_push(Path::new(tp))? {
        return Ok(None);
    }
    Ok(Some(format!(
        "G10: this turn left work in {}: {} uncommitted file(s), {} unpushed commit(s). Rule 13: commit and push now, unless the user said not to. If the work is not finished, say what is left.",
        basename(&top),
        dirty,
        unpushed
    )))
}

fn final_reply(data: &Value, t: &Turn) -> String {
    match get(data, "last_assistant_message").as_str() {
        Some(s) => s.to_string(),
        None => t.last_text.clone().unwrap_or_default(),
    }
}

/// G11: a turn of 10 min or more ends without a clock time or a duration.
fn g11_clock(data: &Value, t: &Turn) -> Res {
    let Some(start) = t.start else { return Ok(None) };
    let minutes = ((now_secs() - start as f64) / 60.0).floor() as i64;
    if minutes >= CLOCK_TURN_MIN && !CLOCK.is_match(&final_reply(data, t)) {
        return Ok(Some(format!(
            "G11: this turn ran {} min. Rule 12: give the clock as a Vienna time (HH:MM) and one estimate with a range.",
            minutes
        )));
    }
    Ok(None)
}

/// G12: task-list steps are still open and the turn asks nothing (quiet while a helper still runs).
fn g12_open_steps(data: &Value, t: &Turn) -> Res {
    if t.asked || t.wrapped || final_reply(data, t).trim_end().ends_with('?') {
        return Ok(None);
    }
    if let Ok(true) = with_state(data, |st| !live_spawns(st, now_secs()).is_empty()) {
        return Ok(None); // a background helper runs; the app wakes the session when it ends
    }
    let names = open_steps(&t.plan);
    if names.is_empty() {
        return Ok(None);
    }
    let shown: Vec<String> = names.into_iter().take(3).collect();
    Ok(Some(format!(
        "G12: the task list still has open steps ({}). Finish them now, or end with what is done, what is left, and one question.",
        shown.join(", ")
    )))
}

fn g6_stop(data: &Value) -> Res {
    let path = get(data, "transcript_path").as_str().ok_or(Fail)?;
    let (fp, mut text) = handoff_this_turn(Path::new(path))?;
    if let Some(last) = get(data, "last_assistant_message").as_str()
        && !text.contains(last) {
            text = format!("{}\n{}", text, last);
        }
    if let Some(fp) = fp
        && Path::new(&fp).exists() && !handoff_given(&fp, &text) {
            let name = basename(&fp).to_string();
            return Ok(Some(format!(
                "guard: you wrote the hand-off {fp} this turn, but your reply does not give it to the user. End with the file link [{name}]({fp}) and a fenced block that holds only: Read {fp} and do it."
            )));
        }
    Ok(None)
}

/// Stop checks, first message wins: G6, then G10 to G12 on the main thread. One reminder per turn.
fn stop(data: &Value) -> Res {
    if truthy(get(data, "stop_hook_active")) || !truthy(get(data, "transcript_path")) {
        return Ok(None);
    }
    if let Ok(Some(m)) = g6_stop(data) {
        return Ok(Some(m));
    }
    if !is_main_thread(data) {
        return Ok(None);
    }
    type TurnCheck = fn(&Value, &Turn) -> Res;
    let checks: [(&str, TurnCheck); 3] = [("G10", g10_commit), ("G11", g11_clock), ("G12", g12_open_steps)];
    let active: Vec<_> = checks.into_iter().filter(|(gid, _)| !check_off(gid)).collect();
    if active.is_empty() {
        return Ok(None);
    }
    let Some(path) = get(data, "transcript_path").as_str() else { return Ok(None) };
    let Ok(t) = scan_turn(Path::new(path)) else { return Ok(None) };
    for (_, f) in active {
        if let Ok(Some(m)) = f(data, &t) {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// SubagentStop: the oldest running spawn of this session is done.
fn spawn_done(data: &Value) {
    let _ = with_state(data, |st| {
        let live = live_spawns(st, now_secs());
        let rest: Vec<Slot> = live.into_iter().skip(1).collect();
        st.insert("spawns".into(), slots_value(&rest));
    });
}

/// PostToolUseFailure on Agent: free the slot of that tool_use_id (a failed spawn never stops).
fn spawn_failed(data: &Value) {
    let tid = get(data, "tool_use_id");
    if !truthy(tid) || !(get(data, "tool_name").is_null() || tool_name(data) == Some("Agent")) {
        return;
    }
    let _ = with_state(data, |st| {
        let mut live = live_spawns(st, now_secs());
        if let Some(i) = live.iter().position(|s| &s.id == tid) {
            live.remove(i);
        }
        st.insert("spawns".into(), slots_value(&live));
    });
}

fn subagentstop(data: &Value) -> Res {
    if truthy(get(data, "stop_hook_active")) {
        spawn_done(data);
        return Ok(None);
    }
    let msg = check_handback(data, get(data, "last_assistant_message"));
    if msg.is_none() {
        spawn_done(data); // the helper is really done; a denied hand-back keeps its slot
    }
    Ok(msg)
}

// --- scratch mode -------------------------------------------------------------

fn expanduser(p: &str) -> String {
    if !p.starts_with('~') {
        return p.to_string();
    }
    let i = p.find('/').unwrap_or(p.len());
    if i != 1 {
        return p.to_string(); // ~user form: left as is
    }
    let home = env::var("HOME")
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.to_string_lossy().into_owned()));
    let Some(home) = home else { return p.to_string() };
    let home = home.trim_end_matches('/');
    let out = format!("{}{}", home, &p[i..]);
    if out.is_empty() { "/".to_string() } else { out }
}

/// Like Python's non-strict os.path.realpath: resolve symlinks of the existing prefix, normalise
/// `..` as it goes, append the rest.
fn realpath(p: &str) -> String {
    let abs = if p.starts_with('/') {
        p.to_string()
    } else {
        let cwd = env::current_dir().map(|c| c.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".to_string());
        format!("{}/{}", cwd, p)
    };
    let mut resolved = String::from("/");
    let mut todo: VecDeque<String> = abs.split('/').map(String::from).collect();
    let mut links = 0;
    while let Some(c) = todo.pop_front() {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            if let Some(idx) = resolved.rfind('/') {
                resolved.truncate(if idx == 0 { 1 } else { idx });
            }
            continue;
        }
        let cand = if resolved == "/" { format!("/{}", c) } else { format!("{}/{}", resolved, c) };
        let is_link = fs::symlink_metadata(&cand).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if is_link && links < 40 {
            links += 1;
            match fs::read_link(&cand) {
                Ok(t) => {
                    let t = t.to_string_lossy().into_owned();
                    if t.starts_with('/') {
                        resolved = String::from("/");
                    }
                    for comp in t.split('/').rev() {
                        todo.push_front(comp.to_string());
                    }
                }
                Err(_) => resolved = cand,
            }
        } else {
            resolved = cand;
        }
    }
    resolved
}

/// Allow a write only under a temp or scratchpad folder; None = allow.
fn scratch(data: &Value) -> Option<String> {
    let ti = get(data, "tool_input");
    let fp = get(ti, "file_path");
    let path_v = if truthy(fp) { fp } else { get(ti, "notebook_path") };
    let path = path_v.as_str().filter(|s| !s.is_empty())?;
    let mut path = expanduser(path);
    if !path.starts_with('/')
        && let Some(cwd) = get(data, "cwd").as_str().filter(|s| !s.is_empty()) {
            path = if cwd.ends_with('/') { format!("{}{}", cwd, path) } else { format!("{}/{}", cwd, path) };
        }
    let real = realpath(&path);
    let mut roots: Vec<String> = SCRATCH_ROOTS.iter().map(|s| s.to_string()).collect();
    if let Ok(t) = env::var("TMPDIR")
        && !t.is_empty() {
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
fn browser_calls_this_turn(path: &Path, tool_use_id: &Value) -> Result<usize, Fail> {
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
                && is_browser(&py_str_or_empty(get(b, "name")))
                && get(b, "id") != tool_use_id
            {
                n += 1;
            }
        }
    }
    Ok(n)
}

fn w3_browser(data: &Value, ctx: &Ctx) -> Res {
    if !is_browser(&py_str_or_empty(get(data, "tool_name"))) {
        return Ok(None);
    }
    if truthy(get(data, "agent_id")) {
        return Ok(None);
    }
    let Some(path) = ctx.path()? else { return Ok(None) };
    if path.contains("/subagents/") {
        return Ok(None); // a subagent (ui-verifier) is where browser work belongs
    }
    let n = browser_calls_this_turn(Path::new(path), get(data, "tool_use_id"))?;
    if n == BROWSER_WARN_AT || (n > BROWSER_WARN_AT && (n - BROWSER_WARN_AT).is_multiple_of(BROWSER_WARN_EVERY)) {
        return Ok(Some(format!(
            "guard (warn): {} browser calls in this window this turn. Page dumps and screenshots fill the main context. For a check (layout, console, a click-through flow, a live URL), dispatch `ui-verifier`; it hands back pass or fail with evidence. Keep going here only when the user asked you to drive the browser for them.",
            n + 1
        )));
    }
    Ok(None)
}

/// (Agent tool_use blocks so far, sum of totalTokens the finished Agent results report).
fn session_spawns(path: &Path) -> Result<(usize, i64), Fail> {
    let data = fs::read(path).map_err(|_| Fail)?;
    let (mut spawns, mut tokens) = (0usize, 0i64);
    let mut tasks: HashSet<String> = HashSet::new();
    for line in data.split(|&b| b == b'\n') {
        if has(line, b"\"Agent\"")
            && let Some(rec) = loads(line)
                && let Some(content) = message_content(&rec).as_array() {
                    for b in content {
                        if b.is_object()
                            && get(b, "type").as_str() == Some("tool_use")
                            && get(b, "name").as_str() == Some("Agent")
                        {
                            spawns += 1;
                        }
                    }
                }
        if has(line, b"\"totalTokens\"") && has(line, b"\"toolUseResult\"")
            && let Some(rec) = loads(line) {
                let tur = get(&rec, "toolUseResult");
                if tur.is_object() && num(get(tur, "totalTokens")) != 0.0 {
                    tokens += num(get(tur, "totalTokens")) as i64;
                }
            }
        if has(line, b"\"totalTokens\"") && has(line, b"task-notification")
            && let Some(rec) = loads(line) {
                let att = get(&rec, "attachment");
                let usage = get(att, "usage");
                if att.is_object() && usage.is_object() && num(get(usage, "totalTokens")) != 0.0 {
                    let prompt = py_str_or_empty(get(att, "prompt"));
                    let key = if let Some(m) = TASK_ID.captures(&prompt) {
                        format!("s{}", &m[1])
                    } else if truthy(get(att, "delivery_id")) {
                        let d = get(att, "delivery_id");
                        match d.as_str() {
                            Some(s) => format!("s{}", s),
                            None => format!("n{}", py_str(d)),
                        }
                    } else {
                        format!("n{}", tasks.len())
                    };
                    if tasks.insert(key) {
                        tokens += num(get(usage, "totalTokens")) as i64;
                    }
                }
            }
    }
    Ok((spawns, tokens))
}

/// The current call is already in the transcript (written before the hook runs).
fn has_tool_use(path: &Path, tool_use_id: &str) -> Result<bool, Fail> {
    let n1 = format!("\"id\":\"{}\"", tool_use_id);
    let n2 = format!("\"id\": \"{}\"", tool_use_id);
    for (i, line) in rev_lines(path)?.enumerate() {
        if has(&line, n1.as_bytes()) || has(&line, n2.as_bytes()) {
            return Ok(true);
        }
        if i > 50 {
            return Ok(false);
        }
    }
    Ok(false)
}

fn w4_budget(data: &Value, ctx: &Ctx) -> Res {
    if tool_name(data) != Some("Agent") {
        return Ok(None);
    }
    let Some(path) = ctx.path()? else { return Ok(None) };
    let path = Path::new(path);
    let (spawns, tokens) = session_spawns(path)?;
    let mut prior = spawns;
    let tid = get(data, "tool_use_id");
    if truthy(tid) {
        prior = if has_tool_use(path, &py_str(tid))? { spawns.saturating_sub(1) } else { spawns };
    }
    let over = prior >= SPAWN_CAP || tokens >= TOKEN_CAP;
    if over && prior % BUDGET_EVERY == 0 {
        return Ok(Some(format!(
            "guard (warn): budget. This session has spawned {} agents ({}K subagent tokens; caps {} agents, {}K). Before more: call mcp__ccd_session_mgmt__get_usage, run the fan-out gate (parallel parts? pause? recovery? record? all no means no fan-out), and batch small tasks.",
            prior,
            tokens / 1000,
            SPAWN_CAP,
            TOKEN_CAP / 1000
        )));
    }
    Ok(None)
}

// --- helpers shared by G14 and G15: shell splitting and Python path functions ----

/// `os.path.normpath` for posix paths.
fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let initial = if p.starts_with("//") && !p.starts_with("///") { 2 } else if p.starts_with('/') { 1 } else { 0 };
    let mut out: Vec<&str> = Vec::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (initial == 0 && out.is_empty()) || out.last() == Some(&"..") {
            out.push(c);
        } else if !out.is_empty() {
            out.pop();
        }
    }
    let body = out.join("/");
    let s = format!("{}{}", "/".repeat(initial), body);
    if s.is_empty() { ".".to_string() } else { s }
}

/// `os.path.split`.
fn path_split(p: &str) -> (String, String) {
    let i = p.rfind('/').map(|x| x + 1).unwrap_or(0);
    let (mut head, tail) = (p[..i].to_string(), p[i..].to_string());
    if !head.is_empty() && head.chars().any(|c| c != '/') {
        head = head.trim_end_matches('/').to_string();
    }
    (head, tail)
}

fn dirname(p: &str) -> String {
    path_split(p).0
}

/// `os.path.expandvars`: $name and ${name}; unknown names stay as they are.
fn expandvars(p: &str) -> String {
    if !p.contains('$') {
        return p.to_string();
    }
    let cs: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '$' && i + 1 < cs.len() {
            let (name, end) = if cs[i + 1] == '{' {
                match cs[i + 2..].iter().position(|&c| c == '}') {
                    Some(k) => (Some(cs[i + 2..i + 2 + k].iter().collect::<String>()), i + 3 + k),
                    None => (None, i),
                }
            } else {
                let mut j = i + 1;
                while j < cs.len() && (cs[j].is_ascii_alphanumeric() || cs[j] == '_') {
                    j += 1;
                }
                if j > i + 1 { (Some(cs[i + 1..j].iter().collect::<String>()), j) } else { (None, i) }
            };
            if let Some(n) = name {
                if let Ok(v) = env::var(&n) {
                    out.push_str(&v);
                    i = end;
                    continue;
                }
                out.extend(&cs[i..end]);
                i = end;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// Python `fnmatch.fnmatchcase`: `*` matches anything (also `/`), `?` one char, `[seq]`, `[!seq]`.
fn fnmatch(name: &str, pat: &str) -> bool {
    type Class = (usize, bool, Vec<(char, char)>);
    fn class(pat: &[char], i: usize) -> Option<Class> {
        // pat[i] == '['; returns (index after ']', negated, ranges)
        let n = pat.len();
        let mut j = i + 1;
        if j < n && pat[j] == '!' {
            j += 1;
        }
        if j < n && pat[j] == ']' {
            j += 1;
        }
        while j < n && pat[j] != ']' {
            j += 1;
        }
        if j >= n {
            return None;
        }
        let mut k = i + 1;
        let neg = pat[k] == '!';
        if neg {
            k += 1;
        }
        let mut ranges = Vec::new();
        while k < j {
            if k + 2 < j && pat[k + 1] == '-' {
                ranges.push((pat[k], pat[k + 2]));
                k += 3;
            } else {
                ranges.push((pat[k], pat[k]));
                k += 1;
            }
        }
        Some((j + 1, neg, ranges))
    }
    fn go(n: &[char], p: &[char]) -> bool {
        if p.is_empty() {
            return n.is_empty();
        }
        match p[0] {
            '*' => {
                let mut rest = p;
                while !rest.is_empty() && rest[0] == '*' {
                    rest = &rest[1..];
                }
                (0..=n.len()).any(|k| go(&n[k..], rest))
            }
            '?' => !n.is_empty() && go(&n[1..], &p[1..]),
            '[' => match class(p, 0) {
                Some((end, neg, ranges)) => {
                    !n.is_empty()
                        && (ranges.iter().any(|&(a, b)| a <= n[0] && n[0] <= b) != neg)
                        && go(&n[1..], &p[end..])
                }
                None => !n.is_empty() && n[0] == '[' && go(&n[1..], &p[1..]),
            },
            c => !n.is_empty() && n[0] == c && go(&n[1..], &p[1..]),
        }
    }
    let n: Vec<char> = name.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    go(&n, &p)
}

fn case_fold(s: &str) -> String {
    if is_macos() { s.to_lowercase() } else { s.to_string() }
}

/// `shlex.split(s)` (posix, no comments); None where Python raises ValueError.
fn shlex_split(s: &str) -> Option<Vec<String>> {
    let cs: Vec<char> = s.chars().collect();
    let ws = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        // one token
        let mut piece = String::new();
        let mut quoted = false;
        let mut state = ' ';
        let mut escaped = ' ';
        let mut got = false;
        while i <= cs.len() {
            let c = cs.get(i).copied();
            i += 1;
            match state {
                ' ' => match c {
                    None => break,
                    Some(c) if ws(c) => {
                        if !piece.is_empty() || quoted {
                            got = true;
                            break;
                        }
                    }
                    Some('\\') => {
                        escaped = 'a';
                        state = '\\';
                    }
                    Some(c @ ('\'' | '"')) => state = c,
                    Some(c) => {
                        piece.push(c);
                        state = 'a';
                    }
                },
                '\'' | '"' => {
                    quoted = true;
                    let c = c?; // No closing quotation
                    if c == state {
                        state = 'a';
                    } else if state == '"' && c == '\\' {
                        escaped = state;
                        state = '\\';
                    } else {
                        piece.push(c);
                    }
                }
                '\\' => {
                    let c = c?; // No escaped character
                    if (escaped == '\'' || escaped == '"') && c != state && c != escaped {
                        piece.push(state);
                    }
                    piece.push(c);
                    state = escaped;
                }
                _ => match c {
                    None => break,
                    Some(c) if ws(c) => {
                        state = ' ';
                        if !piece.is_empty() || quoted {
                            got = true;
                            break;
                        }
                    }
                    Some(c @ ('\'' | '"')) => state = c,
                    Some('\\') => {
                        escaped = 'a';
                        state = '\\';
                    }
                    Some(c) => piece.push(c),
                },
            }
        }
        if got || !piece.is_empty() || quoted {
            out.push(piece);
        }
        if i > cs.len() {
            break;
        }
    }
    Some(out)
}

/// Delimiters of the here-documents started on one line: `<<WORD`, `<<-'WORD'`, not `<<<`.
fn heredoc_delims(line: &str) -> Vec<String> {
    let cs: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < cs.len() {
        if cs[i] == '<' && cs[i + 1] == '<' && (i == 0 || cs[i - 1] != '<') {
            let mut j = i + 2;
            if j < cs.len() && cs[j] == '-' {
                j += 1;
            }
            while j < cs.len() && cs[j].is_whitespace() {
                j += 1;
            }
            let quote = if j < cs.len() && (cs[j] == '\'' || cs[j] == '"') { Some(cs[j]) } else { None };
            let mut k = if quote.is_some() { j + 1 } else { j };
            if k < cs.len() && (cs[k].is_ascii_alphabetic() || cs[k] == '_') {
                let st = k;
                while k < cs.len() && (cs[k].is_ascii_alphanumeric() || cs[k] == '_') {
                    k += 1;
                }
                let word: String = cs[st..k].iter().collect();
                if let Some(q) = quote {
                    if k < cs.len() && cs[k] == q {
                        out.push(word);
                        i = k + 1;
                        continue;
                    }
                } else {
                    out.push(word);
                    i = k;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn strip_heredocs(cmd: &str) -> String {
    let (mut out, mut pending): (Vec<&str>, Vec<String>) = (Vec::new(), Vec::new());
    for line in cmd.split('\n') {
        if !pending.is_empty() {
            if line.trim() == pending[0] {
                pending.remove(0);
            }
            continue;
        }
        out.push(line);
        pending.extend(heredoc_delims(line));
    }
    out.join("\n")
}

/// Split a command line at ; | & newline ( ) ` and $( outside quotes. Quotes stay in the text.
fn segments(cmd: &str) -> Vec<String> {
    let cs: Vec<char> = cmd.chars().collect();
    let (mut segs, mut cur, mut quote, mut i) = (Vec::new(), String::new(), None::<char>, 0);
    while i < cs.len() {
        let c = cs[i];
        if let Some(q) = quote {
            cur.push(c);
            if q == '"' && c == '\\' && i + 1 < cs.len() {
                cur.push(cs[i + 1]);
                i += 1;
            } else if c == q {
                quote = None;
            }
        } else if c == '\\' && i + 1 < cs.len() {
            cur.push(c);
            cur.push(cs[i + 1]);
            i += 1;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            cur.push(c);
        } else if ";|&\n()`".contains(c) || (c == '$' && cs.get(i + 1) == Some(&'(')) {
            segs.push(std::mem::take(&mut cur));
            if c == '$' {
                i += 1;
            }
        } else {
            cur.push(c);
        }
        i += 1;
    }
    segs.push(cur);
    segs
}

fn is_assign(t: &str) -> bool {
    let mut it = t.chars();
    match it.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    for c in it {
        if c == '=' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    false
}

fn wrapper_opts(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "sudo" => &["-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U"],
        "doas" => &["-u", "-C"],
        "env" => &["-u", "-C", "-S"],
        "xargs" => &["-I", "-n", "-P", "-L", "-s", "-d", "-E", "-a", "-J"],
        "nice" => &["-n"],
        "timeout" => &["-s", "-k"],
        "exec" => &["-a"],
        "command" | "builtin" | "nohup" | "time" => &[],
        _ => return None,
    })
}

/// Drop env assignments, reserved words and wrappers (sudo, env, xargs ...) from the front.
fn peel(mut tokens: Vec<String>) -> Vec<String> {
    const RESERVED: &[&str] = &["if", "then", "else", "elif", "do", "while", "until", "{", "}", "!"];
    for _ in 0..8 {
        let mut i = 0;
        while i < tokens.len() && (RESERVED.contains(&tokens[i].as_str()) || is_assign(&tokens[i])) {
            i += 1;
        }
        tokens.drain(..i);
        if tokens.is_empty() {
            return tokens;
        }
        let base = basename(&tokens[0]).to_string();
        let Some(optval) = wrapper_opts(&base) else { return tokens };
        let mut i = 1;
        while i < tokens.len() && tokens[i].starts_with('-') && tokens[i] != "-" {
            if tokens[i] == "--" {
                i += 1;
                break;
            }
            i += if optval.contains(&tokens[i].as_str()) { 2 } else { 1 };
        }
        if base == "timeout" {
            i += 1; // the duration
        }
        tokens.drain(..i.min(tokens.len()));
    }
    tokens
}

fn abspath(p: &str, cwd: Option<&str>) -> String {
    let mut p = expanduser(&expandvars(p));
    if !p.starts_with('/')
        && let Some(c) = cwd.filter(|c| !c.is_empty()) {
            p = path_join(c, &p);
        }
    realpath(&p)
}

struct Cmd {
    toks: Vec<String>,
    extra: Vec<String>,
    cwd: Option<String>,
}

/// Every simple command of a Bash command line, wrappers peeled; commands inside `bash -c`, `eval`
/// and `find -exec` too (extra = the find words before -exec). cwd follows `cd`.
fn walk(cmd: &str, cwd: Option<String>, depth: usize) -> Vec<Cmd> {
    let mut out = Vec::new();
    let mut cwd = cwd;
    for seg in segments(&strip_heredocs(cmd)) {
        let Some(split) = shlex_split(&seg) else { continue }; // unbalanced quotes: not a command
        let toks = peel(split);
        if toks.is_empty() {
            continue;
        }
        let name = basename(&toks[0]).to_string();
        if (name == "cd" || name == "pushd")
            && let Some(d) = toks[1..].iter().find(|t| !t.starts_with('-')) {
                cwd = Some(abspath(d, cwd.as_deref()));
            }
        out.push(Cmd { toks: toks.clone(), extra: Vec::new(), cwd: cwd.clone() });
        if depth >= 3 {
            continue;
        }
        if matches!(name.as_str(), "bash" | "sh" | "zsh" | "dash" | "ksh") {
            for (k, t) in toks.iter().enumerate().skip(1) {
                let is_c = t.len() >= 2 && t.starts_with('-') && t.ends_with('c') && t[1..].chars().all(|c| c.is_ascii_alphabetic());
                if is_c && k + 1 < toks.len() {
                    out.extend(walk(&toks[k + 1], cwd.clone(), depth + 1));
                    break;
                }
            }
        } else if name == "eval" {
            out.extend(walk(&toks[1..].join(" "), cwd.clone(), depth + 1));
        } else if name == "find" {
            for (j, t) in toks.iter().enumerate() {
                if matches!(t.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                    let sub: Vec<String> =
                        toks[j + 1..].iter().take_while(|u| u.as_str() != ";" && u.as_str() != "+").cloned().collect();
                    let sub = peel(sub);
                    if !sub.is_empty() {
                        out.push(Cmd { toks: sub, extra: toks[1..j].to_vec(), cwd: cwd.clone() });
                    }
                }
            }
        }
    }
    out
}

fn bash_command(data: &Value) -> Option<String> {
    if tool_name(data) != Some("Bash") {
        return None;
    }
    let ti = get(data, "tool_input");
    get(ti, "command").as_str().filter(|s| !s.is_empty()).map(String::from)
}

// --- G14 sed -i ----------------------------------------------------------------

/// True when sed's arguments hold -i, -i.bak, -ni, -Ei, --in-place[=SUFFIX].
fn sed_in_place(args: &[String]) -> bool {
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a == "--" {
            break;
        } else if a.starts_with("--") {
            if a == "--in-place" || a.starts_with("--in-place=") {
                return true;
            }
            skip = matches!(a.as_str(), "--expression" | "--file" | "--line-length");
        } else if a.starts_with('-') && a.chars().count() > 1 {
            let cs: Vec<char> = a.chars().collect();
            for k in 1..cs.len() {
                if cs[k] == 'i' {
                    return true;
                }
                if "efl".contains(cs[k]) {
                    skip = k == cs.len() - 1;
                    break;
                }
            }
        }
    }
    false
}

/// G14: deny BSD `sed -i` on macOS. Quoted text, `gsed` and `sed -n` pass.
fn g14_sed(data: &Value, _ctx: &Ctx) -> Res {
    let Some(cmd) = bash_command(data) else { return Ok(None) };
    if !is_macos() || !cmd.contains("sed") || check_off("G14") {
        return Ok(None);
    }
    let cwd = get(data, "cwd").as_str().map(String::from);
    let r = std::panic::catch_unwind(|| {
        walk(&cmd, cwd, 0).iter().any(|c| basename(&c.toks[0]) == "sed" && sed_in_place(&c.toks[1..]))
    });
    Ok(if r.unwrap_or(false) { Some(MSG_G14.to_string()) } else { None })
}

// --- G15 secret reads ----------------------------------------------------------

const SECRET_READERS: &[&str] = &[
    "cat", "bat", "batcat", "less", "more", "most", "head", "tail", "rg", "grep", "egrep", "fgrep", "ag", "ack",
    "sed", "gsed", "awk", "gawk", "mawk", "cut", "strings", "xxd", "hexdump", "od", "nl", "tac", "rev", "sort",
    "uniq", "paste", "fold", "column", "cp", "mv", "scp", "rsync", "open", "code", "cursor", "vim", "vi", "nvim",
    "nano", "view", "pbcopy", "base64", "jq", "yq", "diff", "cmp", "ln", "install",
];
const SECRET_SEARCHERS: &[&str] = &["rg", "grep", "egrep", "fgrep", "ag", "ack"];
const ARGS_FIELDS: &[&str] = &["args", "command", "cmd", "command_line", "cmdline"];
const PS_VALUE_LONG: &[&str] = &[
    "--format", "--pid", "--ppid", "--sid", "--tty", "--user", "--group", "--sort", "--cols", "--columns", "--width",
    "--lines", "--rows",
];

type Target = (String, String);

fn secret_target() -> Target {
    let raw = normpath(&expanduser(SECRET_FILE));
    let real = realpath(&raw);
    (raw, real)
}

/// True when `path` (word as typed) is the secret file, a glob that matches it, or (dir_ok) its folder.
fn is_secret(path: &str, cwd: Option<&str>, target: &Target, dir_ok: bool) -> bool {
    let (raw, real) = (case_fold(&target.0), case_fold(&target.1));
    let mut p = expanduser(&expandvars(path));
    if p.is_empty() || p.starts_with('-') {
        return false;
    }
    if !p.starts_with('/')
        && let Some(c) = cwd.filter(|c| !c.is_empty()) {
            p = path_join(c, &p);
        }
    if GLOB_CHARS.is_match(&p) {
        let norm = normpath(&p);
        let (d, base) = path_split(&norm);
        return fnmatch(&raw, &case_fold(&norm)) || fnmatch(&real, &case_fold(&path_join(&realpath(&d), &base)));
    }
    let got = case_fold(&realpath(&p));
    got == real || (dir_ok && got == dirname(&real))
}

/// True for a ps call that lists processes with their command lines (aux, -ef, -o args, ww, eww).
fn ps_leaks(args: &[String]) -> bool {
    use std::collections::HashMap;
    let mut flags: HashMap<char, i32> = HashMap::new();
    let mut bare: HashSet<char> = HashSet::new();
    let mut fmt: Vec<String> = Vec::new();
    let (mut i, n) = (0, args.len());
    let mut has_o = false;
    while i < n {
        let a = &args[i];
        i += 1;
        if a.starts_with("--") {
            if let Some(v) = a.strip_prefix("--format=") {
                fmt.push(v.to_string());
                has_o = true;
            } else if PS_VALUE_LONG.contains(&a.as_str()) && i < n {
                if a == "--format" {
                    fmt.push(args[i].clone());
                    has_o = true;
                }
                i += 1;
            }
        } else if a.starts_with('-') && a.chars().count() > 1 {
            let cs: Vec<char> = a.chars().collect();
            for k in 1..cs.len() {
                let c = cs[k];
                if "oOpCtuUgGsD".contains(c) {
                    let mut val: String = cs[k + 1..].iter().collect();
                    if val.is_empty() && i < n {
                        val = args[i].clone();
                        i += 1;
                    }
                    if c == 'o' || c == 'O' {
                        fmt.push(val);
                        has_o = has_o || c == 'o';
                    }
                    break;
                }
                *flags.entry(c).or_insert(0) += 1;
            }
        } else if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphabetic()) {
            for c in a.chars() {
                *flags.entry(c).or_insert(0) += 1;
                bare.insert(c);
            }
        }
    }
    for val in &fmt {
        let hit = val
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|f| !f.is_empty())
            .any(|f| ARGS_FIELDS.contains(&f.split('=').next().unwrap_or("").to_lowercase().as_str()));
        if hit {
            return true;
        }
    }
    if flags.get(&'w').copied().unwrap_or(0) >= 2 || flags.contains_key(&'f') || bare.contains(&'e') || bare.contains(&'u') {
        return true;
    }
    if fmt.is_empty() && "eAaxd".chars().any(|c| flags.contains_key(&c)) {
        return true;
    }
    if is_macos() {
        return !has_o && !flags.contains_key(&'c') && !args.iter().any(|a| a == "--help" || a == "--version");
    }
    false
}

/// True for pgrep -fl / -af / -lf / --list-full (they print full command lines).
fn pgrep_leaks(args: &[String]) -> bool {
    let mut flags: HashSet<char> = HashSet::new();
    let (mut i, n) = (0, args.len());
    while i < n {
        let a = &args[i];
        i += 1;
        if a == "--list-full" {
            return true;
        }
        if a.starts_with("--") || !a.starts_with('-') || a.chars().count() < 2 {
            continue;
        }
        let cs: Vec<char> = a.chars().collect();
        for k in 1..cs.len() {
            if "dFgGLPrstuU".contains(cs[k]) {
                if k == cs.len() - 1 {
                    i += 1;
                }
                break;
            }
            flags.insert(cs[k]);
        }
    }
    flags.contains(&'f') && (flags.contains(&'l') || flags.contains(&'a'))
}

fn search_val(fam: &str) -> (&'static str, &'static [&'static str]) {
    match fam {
        "grep" => (
            "efmABCdD",
            &[
                "--regexp", "--file", "--max-count", "--after-context", "--before-context", "--context",
                "--directories", "--devices", "--include", "--exclude", "--exclude-dir", "--exclude-from",
                "--label", "--binary-files", "--group-separator",
            ],
        ),
        "rg" => (
            "efgmABCdjMrtTE",
            &[
                "--regexp", "--file", "--glob", "--iglob", "--max-count", "--after-context", "--before-context",
                "--context", "--max-depth", "--maxdepth", "--threads", "--max-columns", "--replace", "--type",
                "--type-not", "--type-add", "--type-clear", "--ignore-file", "--encoding", "--colors", "--color",
                "--sort", "--sortr", "--path-separator", "--pre", "--pre-glob", "--engine", "--dfa-size-limit",
                "--regex-size-limit", "--max-filesize", "--context-separator", "--field-context-separator",
                "--field-match-separator", "--hostname-bin", "--hyperlink-format",
            ],
        ),
        "ag" => (
            "ABCGgmpW",
            &[
                "--ignore", "--ignore-dir", "--depth", "--file-search-regex", "--path-to-ignore", "--pager",
                "--after", "--before", "--context", "--max-count", "--width", "--workers",
            ],
        ),
        _ => (
            "ABCmgt",
            &[
                "--ignore-dir", "--noignore-dir", "--ignore-file", "--type", "--after-context", "--before-context",
                "--context", "--max-count", "--output", "--pager",
            ],
        ),
    }
}

#[derive(Default)]
struct SearchOpts {
    pos: Vec<String>,
    pat: bool,
    rec: bool,
    unres: bool,
    globs: Vec<String>,
    inc: Vec<String>,
    xfile: Vec<String>,
    xdir: Vec<String>,
}

/// Fold one short option (val is None for a flag) into the search options.
fn search_short(fam: &str, c: char, val: Option<&str>, o: &mut SearchOpts) {
    if (fam == "grep" || fam == "rg") && (c == 'e' || c == 'f') && val.is_some() {
        o.pat = true;
    } else if fam == "grep" {
        if ((c == 'r' || c == 'R') && val.is_none()) || (c == 'd' && val == Some("recurse")) {
            o.rec = true;
        }
    } else if fam == "rg" || fam == "ag" {
        if let (true, Some(v)) = (c == 'g', val) {
            o.globs.push(v.to_string());
            o.pat = o.pat || fam == "ag"; // ag -g PATTERN replaces the content pattern
        } else if val.is_none() && (c == 'u' || (c == '.' && fam == "rg") || (c == 'U' && fam == "ag")) {
            o.unres = true;
        }
    } else if fam == "ack" && (c == 'g' || c == 'f') {
        o.pat = true;
    }
}

/// Fold one long option (value already split off) into the search options.
fn search_long(fam: &str, key: &str, val: &str, o: &mut SearchOpts) {
    if (fam == "grep" || fam == "rg") && (key == "--regexp" || key == "--file") {
        o.pat = true;
    } else if fam == "grep" {
        if key == "--recursive" || key == "--dereference-recursive" || (key == "--directories" && val == "recurse") {
            o.rec = true;
        } else if key == "--include" {
            o.inc.push(val.to_string());
        } else if key == "--exclude" {
            o.xfile.push(val.to_string());
        } else if key == "--exclude-dir" {
            o.xdir.push(val.to_string());
        }
    } else if fam == "rg" {
        if key == "--files" {
            o.pat = true;
        } else if key == "--glob" || key == "--iglob" {
            o.globs.push(val.to_string());
        } else if key == "--hidden" || key == "--unrestricted" || (key.starts_with("--no-ignore") && key != "--no-ignore-messages") {
            o.unres = true;
        }
    } else if fam == "ag" {
        if key == "--ignore" {
            o.globs.push(format!("!{}", val));
        } else if key == "--ignore-dir" {
            o.xdir.push(val.to_string());
        } else if key == "--unrestricted" || key == "--skip-vcs-ignores" || key == "--hidden" {
            o.unres = true;
        } else if key == "--file-search-regex" {
            o.pat = true;
        }
    } else if fam == "ack" && key == "--ignore-dir" {
        o.xdir.push(val.to_string());
    }
}

/// Parse the arguments of grep, rg, ag or ack: path operands, recursion, unrestricted flags, excludes.
fn search_opts(fam: &str, args: &[String]) -> SearchOpts {
    let (short_val, long_val) = search_val(fam);
    let mut o = SearchOpts { rec: fam == "ack", ..Default::default() };
    let (mut i, n) = (0, args.len());
    while i < n {
        let a = &args[i];
        i += 1;
        if a == "--" {
            o.pos.extend(args[i..].iter().cloned());
            break;
        }
        if a.starts_with("--") {
            let (key, eq, mut val) = match a.find('=') {
                Some(x) => (&a[..x], true, a[x + 1..].to_string()),
                None => (a.as_str(), false, String::new()),
            };
            if !eq && long_val.contains(&key) && i < n {
                val = args[i].clone();
                i += 1;
            }
            search_long(fam, key, &val, &mut o);
        } else if a.starts_with('-') && a.chars().count() > 1 {
            let cs: Vec<char> = a.chars().collect();
            for k in 1..cs.len() {
                let c = cs[k];
                if short_val.contains(c) {
                    let mut val: String = cs[k + 1..].iter().collect();
                    if val.is_empty() && i < n {
                        val = args[i].clone();
                        i += 1;
                    }
                    search_short(fam, c, Some(&val), &mut o);
                    break;
                }
                search_short(fam, c, None, &mut o);
            }
        } else {
            o.pos.push(a.clone());
        }
    }
    o
}

/// True when an ignore-style glob (rg -g '!GLOB') matches the file, one of its folders, or its path.
fn glob_hits(g: &str, dirs: &[String], fname: &str, path: &str) -> bool {
    let g = case_fold(g);
    let g = g.trim_start_matches('/');
    let only_dir = g.ends_with('/');
    let g = g.trim_end_matches('/');
    if g.is_empty() {
        return false;
    }
    let mut names: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
    if !only_dir {
        names.push(fname);
    }
    if names.iter().any(|c| fnmatch(c, g)) {
        return true;
    }
    [g.to_string(), format!("*/{}", g), format!("*/{}/*", g)].iter().any(|p| fnmatch(path, p))
}

/// True when the search options leave `path` out (--exclude, --exclude-dir, --include, rg/ag globs).
fn search_excluded(o: &SearchOpts, path: &str, dirs: &[String], fname: &str) -> bool {
    let path = case_fold(path);
    if o.xfile.iter().any(|g| fnmatch(fname, &case_fold(g)) || fnmatch(&path, &case_fold(g))) {
        return true;
    }
    if o.xdir.iter().any(|g| dirs.iter().any(|d| fnmatch(d, case_fold(g).trim_end_matches('/')))) {
        return true;
    }
    if !o.inc.is_empty() && !o.inc.iter().any(|g| fnmatch(fname, &case_fold(g))) {
        return true;
    }
    let keep: Vec<&String> = o.globs.iter().filter(|g| !g.starts_with('!')).collect();
    if o.globs.iter().filter(|g| g.starts_with('!')).any(|g| glob_hits(&g[1..], dirs, fname, &path)) {
        return true;
    }
    if !keep.is_empty() {
        return !keep.iter().any(|g| {
            let g = case_fold(g);
            fnmatch(fname, &g) || fnmatch(&path, &g) || fnmatch(&path, &format!("*/{}", g))
        });
    }
    false
}

/// True when the search root covers `cand` (the secret file) and no option leaves it out.
fn search_reaches(o: &SearchOpts, root: &str, cand: &str) -> bool {
    let root = normpath(root);
    let cand = case_fold(cand);
    let mut base = root.clone();
    if GLOB_CHARS.is_match(&root) {
        // a glob operand: the part before the first glob word is the base folder
        let mut keep: Vec<&str> = Vec::new();
        for part in root.split('/') {
            if GLOB_CHARS.is_match(part) {
                break;
            }
            keep.push(part);
        }
        base = keep.join("/");
        if base.is_empty() {
            base = "/".to_string();
        }
        let pat = case_fold(&root);
        if !(fnmatch(&cand, &pat) || fnmatch(&cand, &format!("{}/*", pat.trim_end_matches('/')))) {
            return false;
        }
    } else {
        let r = case_fold(&root);
        if !(r == cand || cand.starts_with(&format!("{}/", r.trim_end_matches('/')))) {
            return false;
        }
    }
    let base = case_fold(&base);
    let prefix = format!("{}/", base.trim_end_matches('/'));
    if !cand.starts_with(&prefix) {
        return true;
    }
    let parts: Vec<String> = cand[prefix.len()..].split('/').map(String::from).collect();
    let (fname, dirs) = parts.split_last().unwrap();
    !search_excluded(o, &cand, dirs, fname)
}

/// True for a recursive search (grep -r, ack, and rg/ag with ignore rules off) that reaches the secret file.
fn search_leaks(name: &str, args: &[String], cwd: Option<&str>, target: &Target) -> bool {
    let fam = if matches!(name, "grep" | "egrep" | "fgrep") { "grep" } else { name };
    let o = search_opts(fam, args);
    if (fam == "grep" && !o.rec) || ((fam == "rg" || fam == "ag") && !o.unres) {
        return false; // rg and ag skip the hidden, git-ignored file unless told not to
    }
    let operands: Vec<&String> = if o.pat { o.pos.iter().collect() } else { o.pos.iter().skip(1).collect() };
    let mut paths: Vec<String> = operands.into_iter().filter(|p| p.as_str() != "-").cloned().collect();
    let cwd_s = cwd.map(String::from).unwrap_or_else(|| {
        env::current_dir().map(|c| c.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".to_string())
    });
    if paths.is_empty() {
        paths.push(cwd_s.clone());
    }
    for p in &paths {
        let mut q = expanduser(&expandvars(p));
        if !q.starts_with('/') {
            q = path_join(&cwd_s, &q);
        }
        for root in [normpath(&q), realpath(&q)] {
            if [&target.0, &target.1].iter().any(|cand| search_reaches(&o, &root, cand)) {
                return true;
            }
        }
    }
    false
}

/// The G15 message for a Bash command that reads local.zsh or prints command lines, else None.
fn bash_leaks(cmd: &str, cwd: &str) -> Option<&'static str> {
    let target = secret_target();
    for c in walk(cmd, Some(cwd.to_string()), 0) {
        let name = basename(&c.toks[0]).to_string();
        let args = &c.toks[1..];
        let ccwd = c.cwd.as_deref();
        let head = &args[..args.len().min(200)];
        if name == "procs" || (name == "ps" && ps_leaks(args)) || (name == "pgrep" && pgrep_leaks(args)) {
            return Some(MSG_G15_PS);
        }
        for (k, t) in c.toks.iter().enumerate() {
            // input redirect: cmd < file
            if t == "<" && k + 1 < c.toks.len() && is_secret(&c.toks[k + 1], ccwd, &target, false) {
                return Some(MSG_G15_FILE);
            }
            if t.starts_with('<')
                && !t.starts_with("<<")
                && !t.starts_with("<(")
                && t.chars().count() > 1
                && is_secret(&t[1..], ccwd, &target, false)
            {
                return Some(MSG_G15_FILE);
            }
        }
        if name == "dd" && head.iter().any(|a| a.starts_with("if=") && is_secret(&a[3..], ccwd, &target, false)) {
            return Some(MSG_G15_FILE);
        }
        if SECRET_READERS.contains(&name.as_str()) {
            if head.iter().any(|a| is_secret(a, ccwd, &target, false)) {
                return Some(MSG_G15_FILE);
            }
            if SECRET_SEARCHERS.contains(&name.as_str())
                && (head.iter().any(|a| is_secret(a, ccwd, &target, true)) || search_leaks(&name, head, ccwd, &target))
            {
                return Some(MSG_G15_SEARCH);
            }
            if c.extra.iter().take(200).any(|a| is_secret(a, ccwd, &target, true)) {
                return Some(MSG_G15_FILE);
            }
        }
    }
    None
}

/// G15: deny reading local.zsh (Read, Grep or Bash) and printing full process command lines.
fn g15_secrets(data: &Value, _ctx: &Ctx) -> Res {
    let tool = tool_name(data);
    if !matches!(tool, Some("Read" | "Grep" | "Bash")) || check_off("G15") {
        return Ok(None);
    }
    let r = std::panic::catch_unwind(|| {
        let cwd = match get(data, "cwd").as_str().filter(|s| !s.is_empty()) {
            Some(c) => c.to_string(),
            None => env::current_dir().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        if tool == Some("Bash") {
            return bash_command(data).and_then(|cmd| bash_leaks(&cmd, &cwd));
        }
        let key = if tool == Some("Read") { "file_path" } else { "path" };
        let ti = get(data, "tool_input");
        let path = get(if ti.is_object() { ti } else { &NULL }, key);
        match path.as_str() {
            Some(p) if !p.is_empty() && is_secret(p, Some(&cwd), &secret_target(), tool == Some("Grep")) => {
                Some(MSG_G15_FILE)
            }
            _ => None,
        }
    });
    Ok(r.ok().flatten().map(String::from))
}

// --- G13 silence note ------------------------------------------------------------

/// Tool calls since the last assistant text with real words (or the last human prompt). With a
/// tool_use_id the count ends at that call, so parallel calls each see their own number.
fn g13_streak(path: &Path, tool_use_id: Option<&str>) -> Result<usize, Fail> {
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
                } else if btype == Some("text") && active
                    && let Some(text) = get(b, "text").as_str()
                        && WORDS.is_match(text) {
                            return Ok(n);
                        }
            }
        }
        if n > G13_AFTER {
            break;
        }
    }
    Ok(n)
}

/// G13 (PostToolUse, main thread): one JSON line after the 8th tool call with no text, else None.
fn posttool(data: &Value) -> Option<String> {
    if check_off("G13") || env::var("CLAUDE_CODE_ENTRYPOINT").as_deref() == Ok("claude-desktop") {
        return None; // the desktop app has its own nudge and its transcript can miss text blocks
    }
    if !is_main_thread(data) {
        return None;
    }
    let path = get(data, "transcript_path").as_str().filter(|s| !s.is_empty())?;
    let tid = get(data, "tool_use_id").as_str().filter(|s| !s.is_empty());
    match g13_streak(Path::new(path), tid) {
        Ok(n) if n == G13_AFTER => Some(hook_output("PostToolUse", MSG_G13)),
        _ => None,
    }
}

// --- G8 helper cap, G9 plan gate, router -----------------------------------------

/// G8: helpers running at once, per session. An allowed call is recorded here, so keep this last.
fn g8_cap(data: &Value, ctx: &Ctx) -> Res {
    if tool_name(data) != Some("Agent") {
        return Ok(None);
    }
    let now = now_secs();
    let st = ctx.pace(); // read before the lock: pace may take a moment
    let r = with_state(data, |state| {
        let mut spawns = live_spawns(state, now);
        let cap = cap_for(&st.state);
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
        spawns.push(Slot { id: get(data, "tool_use_id").clone(), t: now });
        state.insert("spawns".into(), slots_value(&spawns));
        None
    });
    Ok(r.unwrap_or(None)) // no state, no cap: fail open
}

/// `message.model` of the newest assistant record of a transcript (lowercase), or None.
fn session_model(path: Option<&str>) -> Option<String> {
    let lines = rev_lines(Path::new(path?)).ok()?;
    for line in lines {
        if !has(&line, b"\"assistant\"") {
            continue;
        }
        let Some(rec) = loads(&line) else { continue };
        if get(&rec, "type").as_str() != Some("assistant") {
            continue;
        }
        if let Some(m) = get(get(&rec, "message"), "model").as_str()
            && !m.is_empty() && !m.starts_with('<') {
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

/// Cost rank of a model name or id (haiku < sonnet < opus < fable); None when unknown.
fn model_rank(model: &str) -> Option<i32> {
    let name = model.to_lowercase();
    MODEL_RANK.iter().find(|(w, _)| name.contains(w)).map(|(_, r)| *r)
}

/// G9: planning skills only in plan mode; no plan mode when the week runs ahead.
fn g9_plan_gate(data: &Value, ctx: &Ctx) -> Res {
    let tool = tool_name(data);
    if tool == Some("Skill") {
        let ti = get(data, "tool_input");
        let skill = get(if ti.is_object() { ti } else { &NULL }, "skill");
        if !PLAN_SKILLS.iter().any(|s| skill.as_str() == Some(s)) || !is_main_thread(data) {
            return Ok(None);
        }
        let st = ctx.pace();
        if get(data, "permission_mode").as_str() == Some("plan") || matches!(st.state.as_str(), "ahead" | "critical") {
            return Ok(None);
        }
        // The main model decides: Opus and Fable already plan on Opus. Only Sonnet or Haiku is gated,
        // and only where the entrypoint is known (the CLI has opusplan; the desktop app has none).
        let path = ctx.path_val().as_str().filter(|s| !s.is_empty());
        let rank = session_model(path).and_then(|m| model_rank(&m));
        match rank {
            None => return Ok(None),
            Some(r) if r >= 2 => return Ok(None),
            _ => {}
        }
        return Ok(match session_entrypoint(path).as_deref() {
            Some("cli") => Some(MSG_PLAN_GATE.to_string()),
            Some("claude-desktop") => Some(MSG_PLAN_GATE_APP.to_string()),
            _ => None,
        });
    }
    if tool == Some("EnterPlanMode") {
        let st = ctx.pace();
        if matches!(st.state.as_str(), "ahead" | "critical") {
            return Ok(Some(format!(
                "guard: the week is {} (used {}, budget {}). Plan without plan mode so Sonnet plans.",
                st.state,
                pct(st.used),
                pct(st.budget)
            )));
        }
    }
    Ok(None)
}

/// Router: (new tool_input or None, note or None) for a main-thread Agent call.
///
/// Plan and a final code-reviewer: opus + high (sonnet + high when the week is ahead). Every other
/// row never raises cost: the lower of the caller's model and the table's (Explore,
/// claude-code-guide, statusline-setup: haiku; the rest: sonnet), effort high unless the caller
/// asked for low or medium, none for haiku. xhigh and max never pass. `fork` is left alone.
fn route_agent(data: &Value, ctx: &Ctx) -> (Option<Value>, Option<String>) {
    if tool_name(data) != Some("Agent") || !is_main_thread(data) {
        return (None, None);
    }
    let Some(ti) = get(data, "tool_input").as_object() else { return (None, None) };
    let kind_v = ti.get("subagent_type").unwrap_or(&NULL);
    let kind = kind_v.as_str();
    if kind == Some("fork") {
        return (None, None);
    }
    let desc = py_str_or_empty(ti.get("description").unwrap_or(&NULL));
    let head: String = py_str_or_empty(ti.get("prompt").unwrap_or(&NULL)).chars().take(400).collect();
    let final_ = kind == Some("code-reviewer") && (FINAL_REVIEW.is_match(&desc) || FINAL_REVIEW.is_match(&head));
    let opus_row = kind == Some("Plan") || (final_ && !PANEL_ANGLE.is_match(&desc));
    let mut note: Option<String> = None;
    let model: Value;
    let effort: String;
    if opus_row {
        let st = ctx.pace();
        let mut m = "opus";
        if matches!(st.state.as_str(), "ahead" | "critical") {
            m = "sonnet";
            note = Some(format!("the week is {}, so opus became sonnet", st.state));
        }
        model = Value::String(m.to_string());
        effort = "high".to_string();
    } else {
        let table = if kind.is_some_and(|k| LOOKUP_AGENTS.contains(&k)) { "haiku" } else { "sonnet" };
        let mut m = Value::String(table.to_string());
        let given = ti.get("model").unwrap_or(&NULL);
        if let Some(gr) = model_rank(&py_str_or_empty(given))
            && gr <= model_rank(table).unwrap() {
                m = given.clone();
            }
        model = m;
        effort = match ti.get("effort").and_then(|e| e.as_str()) {
            Some(e @ ("low" | "medium")) => e.to_string(),
            _ => "high".to_string(),
        };
    }
    let mut new = ti.clone();
    new.insert("model".into(), model.clone());
    if model_rank(&py_str(&model)) == Some(0) {
        new.shift_remove("effort"); // haiku takes no effort
    } else {
        new.insert("effort".into(), Value::String(effort));
    }
    if &new == ti {
        return (None, None);
    }
    let shown = new.get("effort").filter(|e| truthy(e)).map(|e| format!(" ({})", py_str(e))).unwrap_or_default();
    let mut msg = format!("guard router: helper runs on {}{}", py_str(&model), shown);
    if let Some(n) = note {
        msg.push_str(" - ");
        msg.push_str(&n);
    }
    (Some(Value::Object(new)), Some(msg))
}

// --- modes --------------------------------------------------------------------

type GuardFn = fn(&Value, &Ctx) -> Res;

fn pretool(data: &Value, ctx: &Ctx) -> Res {
    // g8_cap records the spawn it allows, so it runs after every other deny
    let guards: [GuardFn; 11] = [
        g3_pretool, g7_heavy, g1_agent, g2_polling, g4_ask, g5_handback, g6_archive, g9_plan_gate, g14_sed,
        g15_secrets, g8_cap,
    ];
    for g in guards {
        if let Some(msg) = g(data, ctx)? {
            return Ok(Some(msg));
        }
    }
    Ok(None)
}

/// Router and warnings for an allowed call: a JSON line for stdout, or None. The line holds
/// additionalContext, plus permissionDecision allow and updatedInput when the router changed the call.
fn pretool_warn(data: &Value, ctx: &Ctx) -> Option<String> {
    let mut notes: Vec<String> = Vec::new();
    let mut updated: Option<Value> = None;
    if let Ok((u, note)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| route_agent(data, ctx))) {
        updated = u;
        notes.extend(note);
    }
    let warners: [GuardFn; 2] = [w3_browser, w4_budget];
    notes.extend(warners.iter().filter_map(|w| w(data, ctx).ok().flatten()));
    if notes.is_empty() && updated.is_none() {
        return None;
    }
    let mut hso = Map::new();
    hso.insert("hookEventName".into(), Value::String("PreToolUse".into()));
    if let Some(u) = updated {
        hso.insert("permissionDecision".into(), Value::String("allow".into()));
        hso.insert("updatedInput".into(), u);
    }
    hso.insert("additionalContext".into(), Value::String(notes.join("\n")));
    let mut top = Map::new();
    top.insert("hookSpecificOutput".into(), Value::Object(hso));
    Some(py_dumps(&Value::Object(top)))
}

/// The note for a pace state that was not announced yet (`last` = the state announced before).
fn pace_note(st: &PaceSt, last: &Value) -> Option<String> {
    let (used, budget) = (pct(st.used), pct(st.budget));
    match st.state.as_str() {
        "critical" => Some(format!(
            "guard: the week is critical ({} used, budget {} today). Sonnet only, no helpers. Tell the user once.",
            used, budget
        )),
        "ahead" if last.as_str() != Some("ahead") => Some(format!(
            "guard: the week is ahead of pace ({} used, budget {} today). Helpers run on Sonnet, max 1 at once, and plans go without plan mode. If the main model is Opus, tell the user once to pick Sonnet.",
            used, budget
        )),
        _ => None,
    }
}

/// G3 nudge and pace notes: JSON for stdout (or None); counters live in the per-session state file.
fn prompt(data: &Value, ctx: &Ctx) -> Option<String> {
    let _ = std::panic::catch_unwind(|| pace::refresh(5.0, None));
    ctx.reset_pace();
    let st = ctx.pace();
    let over = ctx.size() >= NUDGE_CONTEXT;
    let mut note: Option<String> = None;
    let count = match with_state(data, |state| {
        let prev = state.get("count").map(|v| py_int(v).unwrap_or(0)).unwrap_or(0);
        let count = if over { prev + 1 } else { 0 };
        state.insert("count".into(), Value::from(count));
        if st.state != "unknown" {
            // a gap in the data does not reset what was announced
            note = pace_note(&st, state.get("pace").unwrap_or(&NULL));
            state.insert("pace".into(), Value::String(st.state.clone()));
        }
        count
    }) {
        Ok(c) => c,
        Err(_) => i64::from(over),
    };
    let mut notes: Vec<String> = Vec::new();
    if count >= 1 && (count - 1) % NUDGE_EVERY == 0 {
        notes.push(format!(
            "Context is {}K. Keep PROGRESS.md (or the run's ledger) current; auto-compact carries the run. Hand off only when the user asks.",
            ctx.size() / 1000
        ));
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
    let ctx = Ctx::new(&data);
    let mut out: Option<String> = None;
    let msg = match mode.as_str() {
        "posttoolfailure" => {
            spawn_failed(&data);
            None
        }
        "posttool" => {
            out = posttool(&data);
            None
        }
        "pretool" => match pretool(&data, &ctx) {
            Ok(m) => {
                if m.is_none() {
                    out = pretool_warn(&data, &ctx);
                }
                m
            }
            Err(_) => return 0,
        },
        "subagentstop" => match subagentstop(&data) {
            Ok(m) => m,
            Err(_) => return 0,
        },
        "stop" => match stop(&data) {
            Ok(m) => m,
            Err(_) => return 0,
        },
        "scratch" => scratch(&data),
        _ => {
            out = prompt(&data, &ctx);
            None
        }
    };
    if let Some(m) = msg {
        let _ = writeln!(io::stderr(), "{}", m);
        return 2;
    }
    if let Some(o) = out {
        let _ = writeln!(io::stdout(), "{}", o);
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
    fn prompt_cap_phrasings() {
        let cases: &[(&str, Option<i64>)] = &[
            ("Keep your response under 6,000 characters.", Some(6000)),
            ("Limit the output to 5,000 chars.", Some(5000)),
            ("Make truncate() return at most 500 characters. Hand back at most 4,000 chars.", Some(4000)),
            ("Hand back at most 4,000 chars. The summary field holds 300 chars.", Some(4000)),
            ("Deadline: answer within 8 minutes. Stay \u{2264} 4,000 chars.", Some(4000)),
            ("Keep it under 2,500 characters. Pad the file to 900 chars.", Some(2500)),
            (
                "Reply in 2,000 chars. Final message under 1500 characters. Then hand back 800 chars.",
                Some(800),
            ),
            ("Hand back 50 chars.", None),
            ("Write a description of at most 160 characters for the page.", None),
        ];
        for (p, want) in cases {
            assert_eq!(prompt_cap(p), *want, "{}", p);
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

    #[test]
    fn cron_intervals() {
        assert_eq!(cron_interval_minutes("* * * * *"), Some(1));
        assert_eq!(cron_interval_minutes("*/15 * * * *"), Some(15));
        assert_eq!(cron_interval_minutes("5 * * * *"), Some(60));
        assert_eq!(cron_interval_minutes("0-30/10 * * * *"), Some(10));
        assert_eq!(cron_interval_minutes("0 */3 * * *"), Some(180));
        assert_eq!(cron_interval_minutes("0,50 * * * *"), Some(10));
        assert_eq!(cron_interval_minutes("0 9 * * *"), None);
        assert_eq!(cron_interval_minutes("nope"), None);
    }

    #[test]
    fn loop_intervals() {
        assert_eq!(loop_interval_seconds("5m check deploy"), Some(300));
        assert_eq!(loop_interval_seconds("30s /foo"), Some(30));
        assert_eq!(loop_interval_seconds("10min x"), Some(600));
        assert_eq!(loop_interval_seconds("2h30m x"), Some(9000));
        assert_eq!(loop_interval_seconds("check deploy every 5m"), Some(300));
        assert_eq!(loop_interval_seconds("check the deploy"), None);
        assert_eq!(loop_interval_seconds("5mx"), None);
    }

    #[test]
    fn py_json_matches_json_dumps() {
        let want = "{\"hookSpecificOutput\": {\"hookEventName\": \"E\", \"additionalContext\": \"a\\\"b\\n\\u00e9\"}}";
        assert_eq!(hook_output("E", "a\"b\n\u{e9}"), want);
    }

    #[test]
    fn handoff_link_and_block() {
        let fp = "/repo/.agents/handoffs/x.md";
        let good = format!("[x.md](.agents/handoffs/x.md)\n```\nRead {} and do it.\n```", fp);
        assert!(handoff_given(fp, &good));
        assert!(!handoff_given(fp, &format!("Read {} and do it.", fp)));
        assert!(HANDOFF_PATH.is_match(fp));
        assert!(!HANDOFF_PATH.is_match("/repo/notes.md"));
    }

    #[test]
    fn truthiness_and_brief_rules() {
        assert!(!truthy(&json!({})));
        assert!(!truthy(&json!(0)));
        assert!(truthy(&json!("x")));
        let data = json!({"tool_name": "Agent", "tool_input": {"prompt": "do work", "description": "build it"}});
        let ctx = Ctx::new(&data);
        let msg = g1_agent(&data, &ctx).ok().flatten().unwrap();
        assert!(msg.contains("a deadline and a hand-back cap"));
    }

    #[test]
    fn shlex_matches_python() {
        let split = |s: &str| shlex_split(s);
        assert_eq!(split("a 'b c' \"d e\" f\\ g"), Some(vec!["a".into(), "b c".into(), "d e".into(), "f g".into()]));
        assert_eq!(split("echo \"a\\\"b\" 'c\\d'"), Some(vec!["echo".into(), "a\"b".into(), "c\\d".into()]));
        assert_eq!(split("x '' y"), Some(vec!["x".into(), "".into(), "y".into()]));
        assert_eq!(split("unbalanced 'quote"), None);
        assert_eq!(split("trailing \\"), None);
        assert_eq!(split("   "), Some(vec![]));
    }

    #[test]
    fn fnmatch_matches_python() {
        assert!(fnmatch("/a/b/local.zsh", "*/local.zsh"));
        assert!(fnmatch("local.zsh", "local.z?h"));
        assert!(fnmatch("a1", "a[0-9]"));
        assert!(!fnmatch("a1", "a[!0-9]"));
        assert!(fnmatch("a[", "a["));
        assert!(!fnmatch("abc", "ab"));
    }

    #[test]
    fn normpath_matches_python() {
        assert_eq!(normpath("/a//b/./c/../d"), "/a/b/d");
        assert_eq!(normpath("//a"), "//a");
        assert_eq!(normpath("///a"), "/a");
        assert_eq!(normpath(""), ".");
        assert_eq!(normpath("../x"), "../x");
        assert_eq!(normpath("/.."), "/");
    }

    #[test]
    fn heredoc_delims_match_python() {
        assert_eq!(heredoc_delims("cat <<EOF"), vec!["EOF"]);
        assert_eq!(heredoc_delims("cat <<-'EOF' > x"), vec!["EOF"]);
        assert!(heredoc_delims("cat <<< word").is_empty());
        assert_eq!(strip_heredocs("cat <<E\nsed -i x\nE\nls"), "cat <<E\nls");
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(&json!("1970-01-02T00:00:00Z")), Some(86400));
        assert_eq!(timestamp(&json!("2026-10-19T12:00:00.123+02:00")), Some(1792404000));
        assert_eq!(timestamp(&json!("nope")), None);
    }
}
