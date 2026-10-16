//! Claude Code guard hook (PreToolUse, UserPromptSubmit, SubagentStop, Stop), a port of
//! `claude/hooks/guard.py`. Deny = message on stderr + exit 2. Allow = exit 0 silently.
//! Warn = exit 0 with hookSpecificOutput.additionalContext JSON on stdout.
//! Fail-open: any bad input, panic or unknown mode exits 0 with no output.
//!
//! Python semantics are mirrored on purpose (truthiness, `str()`, char-based lengths). Where the
//! Python code would raise and so fail open, the Rust code returns `Err(Fail)`.

use regex::Regex;
use serde_json::Value;
use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const POLL_CONTEXT: i64 = 100_000;
const NUDGE_CONTEXT: i64 = 300_000;
const SPAWN_CONTEXT: i64 = 500_000;
const MIN_INTERVAL_MIN: i64 = 30;
const NUDGE_EVERY: i64 = 5;
const HANDBACK_SLACK: f64 = 1.10;
const HANDBACK_MAX_DENIES: i64 = 2;
const HANDBACK_DEFAULT_CAP: i64 = 4000;
const PROMPT_SCAN_LINES: usize = 200;
const SCRATCH_ROOTS: &[&str] = &["/tmp", "/private/tmp", "/var/folders", "/private/var/folders"];
const SCRATCH_DIRS: &[&str] = &[".agents", ".superpowers"];
const ARCHIVE_TOOL: &str = "mcp__ccd_session_mgmt__archive_session";
const HEAVY_AGENTS: &[&str] = &["general-purpose", "claude"];
const REVIEW_AGENTS: &[&str] = &["code-reviewer", "task-reviewer"];
const BROWSER_PREFIXES: &[&str] = &["mcp__Claude_Browser__", "mcp__claude-in-chrome__"];
const BROWSER_WARN_AT: usize = 5;
const BROWSER_WARN_EVERY: usize = 20;
const SPAWN_CAP: usize = 15;
const TOKEN_CAP: i64 = 1_500_000;
const BUDGET_EVERY: usize = 5;
const STRONG_MODELS: &[&str] = &["opus", "fable"];

const MSG_AGENT: &str = "guard: Agent call needs \"model\" and \"effort\" (sonnet + high for build/review/research; opus or fable for plan and final review).";
const MSG_MAX: &str = "guard: effort \"max\" is only for plan / synthesis / architecture / final review; use \"high\", or say which of those this is in the description.";
const MSG_SCRATCH: &str = "guard: this agent writes only scratch files; write under the scratchpad, $TMPDIR, .agents/ or .superpowers/.";
const MSG_POLL: &str = "guard: no polling loops at big context; use Monitor / SendMessage notify_when_idle, or one final report.";
const MSG_ASK: &str = "guard: one AskUserQuestion per turn; put all questions in one call.";

static MAX_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:plan|synthesis|architecture|final\b.{0,30}\breview)\b").unwrap());
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

fn state_dir() -> PathBuf {
    let base = env::var("TMPDIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/tmp".to_string());
    PathBuf::from(base).join("claude-guard")
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
        if has(&line, b"compact_boundary") || has(&line, b"isCompactSummary") {
            if let Some(rec) = loads(&line) {
                if get(&rec, "subtype").as_str() == Some("compact_boundary") || truthy(get(&rec, "isCompactSummary")) {
                    return 0;
                }
            }
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
    if let Some(h) = STAR_STEP.captures(hour) {
        if DIGITS.is_match(&minute) {
            let n = big(&h[1]);
            return if n >= 1 { Some(n.saturating_mul(60)) } else { None };
        }
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
        if boundary {
            if let Some(v) = loop_chain(s, idx + 6) {
                return Some(v);
            }
        }
        from = idx + 1;
    }
    None
}

// --- guards -------------------------------------------------------------------

struct Ctx<'a> {
    data: &'a Value,
    size: Cell<Option<i64>>,
}

impl<'a> Ctx<'a> {
    fn new(data: &'a Value) -> Self {
        Ctx { data, size: Cell::new(None) }
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

fn g3_pretool(data: &Value, ctx: &Ctx) -> Res {
    if tool_name(data) == Some("Agent") && ctx.size() > SPAWN_CONTEXT {
        return Ok(Some(format!(
            "guard: context is {}K (> 500K); run /ps:handoff and continue in a fresh session instead of spawning.",
            ctx.size() / 1000
        )));
    }
    Ok(None)
}

fn g1_agent(data: &Value, _ctx: &Ctx) -> Res {
    if tool_name(data) != Some("Agent") {
        return Ok(None);
    }
    let ti = obj_or_empty(get(data, "tool_input"))?;
    if !truthy(get(ti, "effort")) || !truthy(get(ti, "model")) {
        return Ok(Some(MSG_AGENT.to_string()));
    }
    if get(ti, "effort").as_str() == Some("max") {
        let text = format!(
            "{} {}",
            py_str_or_empty(get(ti, "description")),
            py_str_or_empty(get(ti, "prompt"))
        )
        .to_lowercase();
        if !MAX_WORD.is_match(&text) {
            return Ok(Some(MSG_MAX.to_string()));
        }
    }
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
        if let Some(text) = text_of(message_content(&rec)) {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn subagent_transcript(data: &Value) -> Option<String> {
    if let Some(atp) = get(data, "agent_transcript_path").as_str() {
        if !atp.is_empty() {
            return Some(atp.to_string());
        }
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
                if let Some(fp) = get(input, "file_path").as_str() {
                    if HANDOFF_PATH.is_match(fp) {
                        texts.reverse();
                        return Ok((Some(fp.to_string()), texts.join("\n")));
                    }
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
    Ok(fp.map(|fp| {
        let name = basename(&fp).to_string();
        format!(
            "guard: you wrote the hand-off {fp} this turn. Do not archive yet: give the link [{name}]({fp}) and the block `Read {fp} and do it.`, end the turn, and archive after the user answers."
        )
    }))
}

/// G6 at Stop: a hand-off written this turn must be in the final reply. One deny only.
fn stop(data: &Value) -> Res {
    if truthy(get(data, "stop_hook_active")) || !truthy(get(data, "transcript_path")) {
        return Ok(None);
    }
    let path = get(data, "transcript_path").as_str().ok_or(Fail)?;
    let (fp, mut text) = handoff_this_turn(Path::new(path))?;
    if let Some(last) = get(data, "last_assistant_message").as_str() {
        if !text.contains(last) {
            text = format!("{}\n{}", text, last);
        }
    }
    if let Some(fp) = fp {
        if !handoff_given(&fp, &text) {
            let name = basename(&fp).to_string();
            return Ok(Some(format!(
                "guard: you wrote the hand-off {fp} this turn, but your reply does not give it to the user. End with the file link [{name}]({fp}) and a fenced block that holds only: Read {fp} and do it."
            )));
        }
    }
    Ok(None)
}

fn subagentstop(data: &Value) -> Res {
    if truthy(get(data, "stop_hook_active")) {
        return Ok(None);
    }
    Ok(check_handback(data, get(data, "last_assistant_message")))
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
    if !path.starts_with('/') {
        if let Some(cwd) = get(data, "cwd").as_str().filter(|s| !s.is_empty()) {
            path = if cwd.ends_with('/') { format!("{}{}", cwd, path) } else { format!("{}/{}", cwd, path) };
        }
    }
    let real = realpath(&path);
    let mut roots: Vec<String> = SCRATCH_ROOTS.iter().map(|s| s.to_string()).collect();
    if let Ok(t) = env::var("TMPDIR") {
        if !t.is_empty() {
            roots.push(t);
        }
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

fn w1_general(data: &Value, _ctx: &Ctx) -> Res {
    if tool_name(data) != Some("Agent") {
        return Ok(None);
    }
    let ti = obj_or_empty(get(data, "tool_input"))?;
    let kind_v = get(ti, "subagent_type");
    let kind = if truthy(kind_v) { py_str(kind_v) } else { "general-purpose".to_string() };
    if kind_v.is_array() || kind_v.is_object() {
        return Err(Fail); // unhashable in Python
    }
    if HEAVY_AGENTS.contains(&kind.as_str()) {
        return Ok(Some(format!(
            "guard (warn): a {kind} subagent starts with about 60K tokens of context; `worker` starts near 15-20K. Next time use `worker` for an edit or command job, `Explore` for a read-only search, or the named agent for the job (task-implementer, task-reviewer, code-reviewer, researcher, repo-scanner, ui-verifier). This spawn is allowed."
        )));
    }
    Ok(None)
}

fn w2_review_model(data: &Value, _ctx: &Ctx) -> Res {
    if tool_name(data) != Some("Agent") {
        return Ok(None);
    }
    let ti = obj_or_empty(get(data, "tool_input"))?;
    let kind_v = get(ti, "subagent_type");
    if kind_v.is_array() || kind_v.is_object() {
        return Err(Fail);
    }
    let kind = py_str_or_empty(kind_v);
    let model = py_str_or_empty(get(ti, "model"));
    if model.is_empty() {
        return Ok(None); // G1 denies first
    }
    let desc = py_str_or_empty(get(ti, "description"));
    let head: String = py_str_or_empty(get(ti, "prompt")).chars().take(400).collect();
    let final_ = FINAL_REVIEW.is_match(&desc) || (REVIEW_AGENTS.contains(&kind.as_str()) && FINAL_REVIEW.is_match(&head));
    let lower = model.to_lowercase();
    let strong = STRONG_MODELS.iter().any(|m| lower.contains(m));
    if final_ && !strong && !PANEL_ANGLE.is_match(&desc) {
        return Ok(Some(format!(
            "guard (warn): this looks like a final or whole-branch review on {model}. The owner's rule: final reviews run on opus (effort max); a cheap judge gives false findings and extra fix rounds."
        )));
    }
    if kind == "task-reviewer" && strong && !final_ {
        return Ok(Some(format!(
            "guard (warn): a per-task review on {model}. The owner's rule: per-task reviews stay on sonnet; opus is for the final and whole-branch review only."
        )));
    }
    Ok(None)
}

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
    if n == BROWSER_WARN_AT || (n > BROWSER_WARN_AT && (n - BROWSER_WARN_AT) % BROWSER_WARN_EVERY == 0) {
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
        if has(line, b"\"Agent\"") {
            if let Some(rec) = loads(line) {
                if let Some(content) = message_content(&rec).as_array() {
                    for b in content {
                        if b.is_object()
                            && get(b, "type").as_str() == Some("tool_use")
                            && get(b, "name").as_str() == Some("Agent")
                        {
                            spawns += 1;
                        }
                    }
                }
            }
        }
        if has(line, b"\"totalTokens\"") && has(line, b"\"toolUseResult\"") {
            if let Some(rec) = loads(line) {
                let tur = get(&rec, "toolUseResult");
                if tur.is_object() && num(get(tur, "totalTokens")) != 0.0 {
                    tokens += num(get(tur, "totalTokens")) as i64;
                }
            }
        }
        if has(line, b"\"totalTokens\"") && has(line, b"task-notification") {
            if let Some(rec) = loads(line) {
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

// --- modes --------------------------------------------------------------------

type GuardFn = fn(&Value, &Ctx) -> Res;

fn pretool(data: &Value) -> Res {
    let ctx = Ctx::new(data);
    let guards: [GuardFn; 6] = [g3_pretool, g1_agent, g2_polling, g4_ask, g5_handback, g6_archive];
    for g in guards {
        if let Some(msg) = g(data, &ctx)? {
            return Ok(Some(msg));
        }
    }
    Ok(None)
}

/// Warnings for an allowed call: a JSON line for stdout (additionalContext), or None.
fn pretool_warn(data: &Value) -> Option<String> {
    let ctx = Ctx::new(data);
    let warners: [GuardFn; 4] = [w1_general, w2_review_model, w3_browser, w4_budget];
    let notes: Vec<String> = warners.iter().filter_map(|w| w(data, &ctx).ok().flatten()).collect();
    if notes.is_empty() {
        None
    } else {
        Some(hook_output("PreToolUse", &notes.join("\n")))
    }
}

/// G3 nudge: JSON for stdout (or None); counter state lives in TMPDIR.
fn prompt(data: &Value) -> Option<String> {
    let ctx = Ctx::new(data);
    let sid_v = get(data, "session_id");
    let sid = sanitize_key(&if truthy(sid_v) { py_str(sid_v) } else { "unknown".to_string() });
    let dir = state_dir();
    let state_file = dir.join(format!("{}.json", sid));
    let mut count = read_counter(&state_file, "count");
    if ctx.size() >= NUDGE_CONTEXT {
        count += 1;
    } else {
        count = 0;
    }
    if fs::create_dir_all(&dir).is_ok() {
        let _ = fs::write(&state_file, format!("{{\"count\": {}}}", count));
    }
    if count >= 1 && (count - 1) % NUDGE_EVERY == 0 {
        return Some(hook_output(
            "UserPromptSubmit",
            &format!("Context is {}K. Run /ps:handoff and continue in a fresh session.", ctx.size() / 1000),
        ));
    }
    None
}

fn run() -> i32 {
    let mode = env::args().nth(1).unwrap_or_default();
    if !matches!(mode.as_str(), "pretool" | "prompt" | "subagentstop" | "scratch" | "stop") {
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
    let mut out: Option<String> = None;
    let msg = match mode.as_str() {
        "pretool" => match pretool(&data) {
            Ok(m) => {
                if m.is_none() {
                    out = pretool_warn(&data);
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
            out = prompt(&data);
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
    fn max_effort_words_are_whole_words() {
        assert!(MAX_WORD.is_match("final whole-branch review"));
        assert!(MAX_WORD.is_match("plan the refactor"));
        assert!(!MAX_WORD.is_match("explain the planet's orbit airplane"));
        assert!(!MAX_WORD.is_match("read docs/plans/x.md"));
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
    fn truthiness_and_effort_rules() {
        assert!(!truthy(&json!({})));
        assert!(!truthy(&json!(0)));
        assert!(truthy(&json!("x")));
        let data = json!({"tool_name": "Agent", "tool_input": {"model": "opus", "effort": "max", "prompt": "do work", "description": "build it"}});
        let ctx = Ctx::new(&data);
        assert_eq!(g1_agent(&data, &ctx).ok().flatten().as_deref(), Some(MSG_MAX));
    }
}
