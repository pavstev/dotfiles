//! whatsleft-scan: collect proof of what is still open in a Claude Code session.
//!
//! Reads the session transcript (JSONL) and the git state of every repo the session touched.
//! Prints tagged evidence lines so the agent can build the "me / you" table.
//! Read-only, except `git fetch` (skip with --no-fetch).

use agent_config::guard_checks::expanduser;
use agent_config::pycompat::{
    basename, civil_from_days, dirname, fname, json_str, list_dir, mtime, parse_ts, py_float, py_splitlines, py_str,
    truthy,
};
use clap::Parser;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DOC: &str = "whatsleft-scan: collect proof of what is still open in a Claude Code session.

Reads the session transcript (JSONL) and the git state of every repo the
session touched. Prints tagged evidence lines so the agent can build the
\"me / you\" table. Read-only, except `git fetch` (skip with --no-fetch).

Tags:
  ASK    a user request, the agent must check it is done, with proof
  ME     open work Claude can do
  YOU    needs the user (a decision, an approval, a login)
  CHECK  maybe open, the agent decides
  LATER  optional, parked in the leftovers file
  OK     nothing to do
  TASK   a running background task or subagent, with a STOP or KEEP advice
  SPEED  a slowness finding from hurry-scan (only the worst 3)

Last line: VERDICT: ...";

macro_rules! rx {
    ($name:ident, $re:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($re).unwrap());
    };
}

rx!(
    CHECK_CMD,
    r"\b(pytest|nextest|vitest|jest|clippy|ruff|biome|golangci-lint|tsc|mypy|shellcheck|pre-commit|ty check|go (?:test|vet|build)|cargo (?:test|check|build|clippy)|hook run|(?:npm|pnpm|yarn|bun) (?:run )?(?:test|lint|check|typecheck|build)|(?:make|just|mise run) (?:test|lint|check|verify))\b"
);
// Python's pattern also has \blater\b(?=\s*[.:!]|\s*$); the regex crate has no lookahead, see has_open_words.
rx!(
    OPEN_WORDS,
    r"(?i)\bTODO:|\bFIXME\b|\bfollow[- ]?ups?\b|\bnext steps?\b|\bnot (?:\w+ )?yet\b|\bskipped\b|\bstill (?:need|needs|have|has) to\b|\byou(?:'ll| will)? (?:need|have|must|should) to\b|\bleft to do\b|\b(?:I|we) (?:could not|couldn't|can't|cannot|did not|didn't)\b|\bblocked (?:by|on)\b|\bwaiting (?:for|on)\b|\b(?:is|are|was|were|still) (?:failing|broken|pending|missing)\b"
);
rx!(LATER, r"(?i)\blater\b");
rx!(MARKER, r"(?:#|//|/\*|<!--|--|;|^\s*\*)\s*(?:TODO|FIXME|XXX|HACK)\b|\b(?:TODO|FIXME|XXX|HACK)\b(?:\([^)]*\))?:");
rx!(DENIED, r"(?i)doesn't want to proceed|was denied|permission .{0,40}denied|rejected by the user");
rx!(
    BG_RESULT,
    r"^\s*(?:Command running in background with ID: (\w+)|Command did not complete[^\n]{0,80}moved to the background \(ID: (\w+)\))"
);
rx!(
    NOTICE,
    r"(?s)<task-id>(?P<task>[^<]*)</task-id>\s*<tool-use-id>(?P<tool>[^<]*)</tool-use-id>.*?<status>(?P<status>[^<]*)</status>(?:\s*<summary>(?P<summary>[^<]*)</summary>)?"
);
// A Bash edit of a file: sed -i, perl -pi / -i (not grep -i, not perl -MFoo). Python ends each branch
// with a lookahead (?=\s|$); the regex crate has none, and a match that takes the space is as good here.
rx!(
    IN_PLACE_EDIT,
    r"\bsed\s+(?:[^\s|;&]+\s+)*?(?:-[A-Za-z]*i[A-Za-z.]*|--in-place\S*)(?:\s|$)|\bperl\s+(?:[^\s|;&]+\s+)*?-[lpna0-9w]*i(?:\.\S*)?(?:\s|$)"
);
rx!(EXIT_CODE, r"exit code:?\s*(-?\d+)");
rx!(PIPEFAIL, r"\bpipefail\b");
rx!(REDIRECT, r"\d*>&\d+|&>");
rx!(PIPE_WORD, r"^(tail|tee|head)\b");
rx!(TRUE_WORD, r"^true\b");
rx!(ECHO_WORD, r"^(echo|true)\b");
rx!(HEREDOC_HEAD, r#"<<-?\s*['"]?(\w+)['"]?[^\n]*\n"#);
rx!(OUTFILE, r"(?:Output is being written to:|output_file:)\s*(\S+?\.output)");
rx!(AGENT_ID, r"agentId:\s*(\w+)");
rx!(ANY_ID, r"(?i)\b(?:task[_ ]?id|id)[:=\s]+([A-Za-z0-9_-]{6,})");
rx!(DEADLINE, r"(?i)(?:deadline|done in|within)[^\d\n]{0,20}(\d+)\s*(h(?:ours?|rs?)?|m(?:in(?:ute)?s?)?)\b");
rx!(
    ENDLESS,
    r"next dev|vite|webpack|wrangler dev|(?:npm|pnpm|yarn|bun) (?:run )?dev\b|--watch|tail -f|http\.server|\bserve\b"
);
rx!(BIG_AGENT, r"(?i)research|refactor|thorough|migrate|audit");
rx!(ARTIFACT_URL, r"https://claude\.ai/(?:code/)?artifact/[\w-]+");
rx!(WS, r"\s+");
rx!(NON_ALNUM, r"[^A-Za-z0-9]");
rx!(SYS_REMINDER, r"(?s)<system-reminder>.*?</system-reminder>");
rx!(PASTED, r"(?s)<pasted_content[^>]*>.*?</pasted_content>");
rx!(CMD_NAME, r"<command-name>([^<]*)</command-name>");
rx!(CMD_ARGS, r"<command-args>([^<]*)</command-args>");
rx!(ASK_SKIP, r"^\s*(<(task-notification|local-command|bash-|command-message)|\[Request interrupted)");
rx!(FENCE, r"(?s)```.*?```");
rx!(TASK_NUM, r"#?(\d+)");
rx!(CRON_ID, r"\b(?:id|ID|job)[:\s]+([\w-]+)");
rx!(GIT_DIR_REF, r#"(?:git -C|cd)\s+("[^"]+"|'[^']+'|[^\s;&|]+)"#);

static NULL: Value = Value::Null;
static QUIET: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct State {
    me: usize,
    you: usize,
    check: usize,
    later: usize,
    rows: Vec<(String, String)>,
}
static STATE: Mutex<State> = Mutex::new(State { me: 0, you: 0, check: 0, later: 0, rows: Vec::new() });

fn emit(s: &str) {
    if !QUIET.load(Ordering::Relaxed) {
        let _ = writeln!(std::io::stdout(), "{}", s);
    }
}

macro_rules! pl {
    () => { emit("") };
    ($($a:tt)*) => { emit(&format!($($a)*)) };
}

fn line(tag: &str, text: &str) {
    {
        let mut st = STATE.lock().unwrap();
        let counted = match tag {
            "ME" => {
                st.me += 1;
                true
            }
            "YOU" => {
                st.you += 1;
                true
            }
            "CHECK" => {
                st.check += 1;
                true
            }
            "LATER" => {
                st.later += 1;
                true
            }
            _ => false,
        };
        if counted {
            st.rows.push((tag.to_string(), text.to_string()));
        }
    }
    pl!("  {:<6}{}", tag, text);
}

// ---------------------------------------------------------------------------
// Python-compat helpers
// ---------------------------------------------------------------------------

fn home() -> String {
    for k in ["HOME", "USERPROFILE"] {
        if let Ok(h) = std::env::var(k)
            && !h.is_empty()
        {
            return h;
        }
    }
    dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "~".into())
}

fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&NULL)
}

/// json.dumps(v) with Python defaults.
fn py_dumps(v: &Value) -> String {
    let mut s = String::new();
    dump(v, &mut s);
    s
}

fn dump(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                out.push_str(&n.to_string());
            } else {
                out.push_str(&py_float(n.as_f64().unwrap_or(0.0)));
            }
        }
        Value::String(s) => json_str(out, s),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                dump(x, out);
            }
            out.push(']');
        }
        Value::Object(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                json_str(out, k);
                out.push_str(": ");
                dump(x, out);
            }
            out.push('}');
        }
    }
}

/// str(v or "")
fn or_blank(v: &Value) -> String {
    if truthy(v) { py_str(v) } else { String::new() }
}

/// First truthy value among the keys, as in `a.get(k1) or a.get(k2) or ...`.
fn first_truthy<'a>(inp: &'a Value, keys: &[&str]) -> &'a Value {
    for k in keys {
        let v = get(inp, k);
        if truthy(v) {
            return v;
        }
    }
    &NULL
}

fn pychars(s: &str, a: usize, b: usize) -> String {
    s.chars().skip(a).take(b.saturating_sub(a)).collect()
}

fn clen(s: &str) -> usize {
    s.chars().count()
}

/// short(): collapse whitespace, trim, cut at n characters with an ellipsis.
fn short(s: &str, n: usize) -> String {
    let t = WS.replace_all(s, " ");
    let t = t.trim();
    if clen(t) <= n { t.to_string() } else { format!("{}…", pychars(t, 0, n - 1)) }
}

fn vshort(v: &Value, n: usize) -> String {
    short(&or_blank(v), n)
}

fn tilde(p: &str) -> String {
    let h = home();
    if p.starts_with(&h) { p.replacen(&h, "~", 1) } else { p.to_string() }
}

/// open(path, errors="replace") with universal newlines.
fn read_text(p: &Path) -> Option<String> {
    fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).replace("\r\n", "\n").replace('\r', "\n"))
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Local calendar fields for a unix time: (year, month, day, hour, minute, second).
#[cfg(all(unix, target_pointer_width = "64"))]
fn local_fields(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const u8,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: both pointers are valid for the call; localtime_r only writes into `tm`.
    let r = unsafe { localtime_r(&secs, &mut tm) };
    if r.is_null() {
        return utc_fields(secs);
    }
    (
        tm.tm_year as i64 + 1900,
        tm.tm_mon as i64 + 1,
        tm.tm_mday as i64,
        tm.tm_hour as i64,
        tm.tm_min as i64,
        tm.tm_sec as i64,
    )
}

#[cfg(not(all(unix, target_pointer_width = "64")))]
fn local_fields(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    // No portable local-time call without a new crate: other platforms show UTC.
    utc_fields(secs)
}

fn utc_fields(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, sod / 3600, sod % 3600 / 60, sod % 60)
}

/// datetime.fromtimestamp(ts).isoformat()
fn local_iso(ts: f64) -> String {
    let t = ts.trunc();
    let mut us = ((ts - t) * 1e6).round_ties_even() as i64;
    let mut secs = t as i64;
    if us >= 1_000_000 {
        secs += 1;
        us -= 1_000_000;
    } else if us < 0 {
        secs -= 1;
        us += 1_000_000;
    }
    let (y, mo, d, h, mi, s) = local_fields(secs);
    let mut o = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, d, h, mi, s);
    if us != 0 {
        o.push_str(&format!(".{:06}", us));
    }
    // The UTC fallback must say so: git reads a zone-less --since time as local time.
    #[cfg(not(all(unix, target_pointer_width = "64")))]
    o.push('Z');
    o
}

/// datetime.fromtimestamp(ts).strftime("%H:%M")
fn local_hm(ts: f64) -> String {
    let (_, _, _, h, mi, _) = local_fields(ts.floor() as i64);
    format!("{:02}:{:02}", h, mi)
}

// ---------------------------------------------------------------------------
// subprocess helper
// ---------------------------------------------------------------------------

fn run(args: &[&str], cwd: Option<&str>, timeout: u64) -> (i32, String) {
    let mut cmd = Command::new(args[0]);
    cmd.args(&args[1..])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    let Ok(mut child) = cmd.spawn() else { return (1, String::new()) };
    let mut so = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut buf = vec![];
        let _ = so.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(3));
            }
            Err(_) => break None,
        }
    };
    let buf = reader.join().unwrap_or_default();
    let Some(st) = status else { return (1, String::new()) };
    match String::from_utf8(buf) {
        Ok(s) => (st.code().unwrap_or(-1), s.replace("\r\n", "\n").replace('\r', "\n").trim_end().to_string()),
        Err(_) => (1, String::new()),
    }
}

fn git(root: &str, args: &[&str], t: u64) -> (i32, String) {
    let mut v = vec!["git", "-C", root];
    v.extend_from_slice(args);
    run(&v, None, t)
}

// ---------------------------------------------------------------------------
// 1. Find and read the session
// ---------------------------------------------------------------------------

fn projects_dir() -> PathBuf {
    Path::new(&home()).join(".claude").join("projects")
}

fn session_hits(prefix: &str, exact: bool) -> Vec<PathBuf> {
    let mut hits = vec![];
    for d in list_dir(&projects_dir()) {
        for f in list_dir(&d) {
            let n = fname(&f);
            let ok =
                if exact { n == format!("{}.jsonl", prefix) } else { n.starts_with(prefix) && n.ends_with(".jsonl") };
            if ok {
                hits.push(f);
            }
        }
    }
    hits
}

fn cwd_string() -> String {
    std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn find_session(arg: Option<&str>) -> Option<PathBuf> {
    if let Some(a) = arg.filter(|a| !a.is_empty()) {
        if Path::new(a).is_file() {
            return Some(PathBuf::from(a));
        }
        return session_hits(a, false).into_iter().next();
    }
    if let Ok(sid) = std::env::var("CLAUDE_CODE_SESSION_ID")
        && !sid.is_empty()
        && let Some(h) = session_hits(&sid, true).into_iter().next()
    {
        return Some(h);
    }
    let enc = NON_ALNUM.replace_all(&cwd_string(), "-").into_owned();
    let mine: Vec<PathBuf> =
        list_dir(&projects_dir().join(enc)).into_iter().filter(|p| fname(p).ends_with(".jsonl")).collect();
    let mut best: Option<(f64, PathBuf)> = None;
    for p in mine {
        let m = mtime(&p).unwrap_or(0.0);
        if best.as_ref().map(|(bm, _)| m > *bm).unwrap_or(true) {
            best = Some((m, p));
        }
    }
    best.map(|(_, p)| p)
}

// Differs from the other `text_of` copies on purpose (newline join, empty string); see stores/claude.rs.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter(|b| b.is_object() && get(b, "type").as_str() == Some("text"))
            .map(|b| get(b, "text").as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn clean_ask(t: &str) -> String {
    let t = SYS_REMINDER.replace_all(t, " ");
    let t = PASTED.replace_all(&t, " [pasted text] ").into_owned();
    if let Some(m) = CMD_NAME.captures(&t) {
        let args = CMD_ARGS.captures(&t).map(|a| a[1].to_string()).unwrap_or_default();
        return format!("{} {}", &m[1], args).trim().to_string();
    }
    if ASK_SKIP.is_match(&t) {
        return String::new();
    }
    short(&t, 150)
}

fn is_human(obj: &Value, content: &Value) -> bool {
    if truthy(get(obj, "isMeta")) || truthy(get(obj, "isSidechain")) {
        return false;
    }
    let origin = get(obj, "origin");
    if origin.is_object() {
        let k = get(origin, "kind");
        if !(k.is_null() || k.as_str() == Some("human")) {
            return false;
        }
    }
    if let Some(a) = content.as_array()
        && a.iter().any(|b| b.is_object() && get(b, "type").as_str() == Some("tool_result"))
    {
        return false;
    }
    !text_of(content).trim().is_empty()
}

fn strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Object(o) => {
            for x in o.values() {
                strings(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                strings(x, out);
            }
        }
        _ => {}
    }
}

struct Use {
    name: String,
    inp: Value,
    ts: Option<f64>,
    cwd: Option<String>,
    idx: usize,
}

#[derive(Default)]
struct Session {
    asks: Vec<(Option<f64>, String, usize)>,
    uses: HashMap<String, Use>,
    uses_keys: Vec<String>,
    results: HashMap<String, (bool, String)>,
    results_keys: Vec<String>,
    texts: Vec<String>,
    notices: HashMap<String, (String, String)>,
    cwds: HashSet<String>,
    order: Vec<String>,
    start: Option<f64>,
    end: Option<f64>,
    todos: Option<Vec<Value>>,
    tasks: Vec<(String, String, String)>,
    stopped: HashSet<String>,
    crons: Vec<(String, String)>,
}

impl Session {
    fn result(&self, uid: &str) -> (bool, &str) {
        match self.results.get(uid) {
            Some((e, b)) => (*e, b.as_str()),
            None => (false, ""),
        }
    }
}

fn read_session(path: &Path) -> Session {
    let mut s = Session::default();
    let text = String::from_utf8_lossy(&fs::read(path).unwrap_or_default()).into_owned();
    for raw in text.split('\n') {
        let Ok(obj) = serde_json::from_str::<Value>(raw) else { continue };
        if !obj.is_object() {
            continue;
        }
        if raw.contains("<task-notification>") {
            let mut strs = vec![];
            strings(&obj, &mut strs);
            for txt in strs {
                for n in NOTICE.captures_iter(txt) {
                    s.notices.insert(
                        n["tool"].to_string(),
                        (
                            n["status"].to_string(),
                            n.name("summary").map(|m| m.as_str().to_string()).unwrap_or_default(),
                        ),
                    );
                }
            }
        }
        let ts = get(&obj, "timestamp").as_str().and_then(parse_ts);
        if let Some(t) = ts
            && t != 0.0
        {
            if !s.start.map(|x| x != 0.0).unwrap_or(false) {
                s.start = Some(t);
            }
            s.end = Some(t);
        }
        let sidechain = truthy(get(&obj, "isSidechain"));
        if let Some(c) = get(&obj, "cwd").as_str().filter(|c| !c.is_empty())
            && !sidechain
        {
            s.cwds.insert(c.to_string());
        }
        let empty = Value::Object(Default::default());
        let m = get(&obj, "message");
        let msg = if truthy(m) && m.is_object() { m } else { &empty };
        let content = get(msg, "content");
        let typ = get(&obj, "type").as_str().unwrap_or("");
        if typ == "user" && !sidechain {
            if is_human(&obj, content) {
                let ask = clean_ask(&text_of(content));
                if !ask.is_empty() {
                    s.asks.push((ts, ask, s.order.len()));
                }
            }
            if let Some(arr) = content.as_array() {
                for b in arr {
                    if b.is_object() && get(b, "type").as_str() == Some("tool_result") {
                        let c = get(b, "content");
                        let body = match c {
                            Value::String(x) => x.clone(),
                            other => {
                                let t = text_of(other);
                                if !t.is_empty() { t } else { pychars(&py_dumps(other), 0, 2000) }
                            }
                        };
                        let id = py_str(get(b, "tool_use_id"));
                        if !s.results.contains_key(&id) {
                            s.results_keys.push(id.clone());
                        }
                        s.results.insert(id, (truthy(get(b, "is_error")), pychars(&body, 0, 2000)));
                    }
                }
            }
        } else if typ == "assistant" && !sidechain && content.is_array() {
            for b in content.as_array().unwrap() {
                if !b.is_object() {
                    continue;
                }
                let bt = get(b, "type").as_str().unwrap_or("");
                if bt == "text" && get(b, "text").as_str().map(|t| !t.trim().is_empty()).unwrap_or(false) {
                    s.texts.push(get(b, "text").as_str().unwrap().to_string());
                } else if bt == "tool_use" {
                    let inp0 = get(b, "input");
                    let inp = if truthy(inp0) { inp0.clone() } else { Value::Object(Default::default()) };
                    let id = py_str(get(b, "id"));
                    let name = get(b, "name").as_str().unwrap_or("").to_string();
                    if !s.uses.contains_key(&id) {
                        s.uses_keys.push(id.clone());
                    }
                    s.uses.insert(
                        id.clone(),
                        Use {
                            name: name.clone(),
                            inp: inp.clone(),
                            ts,
                            cwd: get(&obj, "cwd").as_str().map(|c| c.to_string()),
                            idx: s.order.len(),
                        },
                    );
                    s.order.push(id);
                    if name == "TodoWrite" {
                        if let Some(t) = get(&inp, "todos").as_array() {
                            s.todos = Some(t.clone());
                        }
                    } else if name == "TaskStop" {
                        let v = first_truthy(&inp, &["task_id", "shell_id"]);
                        s.stopped.insert(or_blank(v));
                    }
                    // CronDelete pops from s.crons, which is still empty during the read (it is filled below).
                }
            }
        }
    }
    // tasks need results, so link them after the read
    let order = s.order.clone();
    for uid in &order {
        let (name, inp) = {
            let u = &s.uses[uid];
            (u.name.clone(), u.inp.clone())
        };
        let res = s.result(uid).1.to_string();
        if name == "TaskCreate" {
            let key = TASK_NUM.captures(&res).map(|m| m[1].to_string()).unwrap_or_else(|| uid.clone());
            let subj = {
                let v = first_truthy(&inp, &["subject", "title", "description"]);
                if truthy(v) { py_str(v) } else { "?".to_string() }
            };
            match s.tasks.iter_mut().find(|t| t.0 == key) {
                Some(t) => {
                    t.1 = subj;
                    t.2 = "pending".into();
                }
                None => s.tasks.push((key, subj, "pending".into())),
            }
        } else if name == "TaskUpdate" {
            let key = or_blank(first_truthy(&inp, &["taskId", "task_id", "id"]));
            if truthy(get(&inp, "status"))
                && let Some(t) = s.tasks.iter_mut().find(|t| t.0 == key)
            {
                t.2 = py_str(get(&inp, "status"));
            }
        } else if name == "CronCreate" {
            let key = CRON_ID.captures(&res).map(|m| m[1].to_string()).unwrap_or_else(|| uid.clone());
            let val = {
                let v = first_truthy(&inp, &["prompt", "cron"]);
                if truthy(v) { py_str(v) } else { "?".to_string() }
            };
            match s.crons.iter_mut().find(|c| c.0 == key) {
                Some(c) => c.1 = val,
                None => s.crons.push((key, val)),
            }
        }
    }
    s
}

fn describe(name: &str, inp: &Value) -> String {
    let d = first_truthy(inp, &["description", "command", "prompt", "file_path"]);
    format!("{} \"{}\"", name, vshort(d, 80))
}

// ---------------------------------------------------------------------------
// 2. Session sections
// ---------------------------------------------------------------------------

/// Python `lst[-limit:]` start index.
fn tail_start(len: usize, limit: i64) -> usize {
    let start = -limit;
    if start < 0 { (len as i64 + start).max(0) as usize } else { (start as usize).min(len) }
}

fn section_asks(s: &Session, limit: i64) {
    let asks = &s.asks;
    pl!("== ASKS ({}) — check each one: done, with proof from this session? ==", asks.len());
    if asks.len() as i64 > limit {
        pl!("  …     {} older ask(s) not shown (--asks N)", asks.len() as i64 - limit);
    }
    let from = tail_start(asks.len(), limit);
    let first_no = (asks.len() as i64 - limit + 1).max(1);
    for (i, (ts, ask, _)) in asks[from..].iter().enumerate() {
        let when = match ts {
            Some(t) if *t != 0.0 => local_hm(*t),
            _ => "--:--".to_string(),
        };
        pl!("  ASK   #{} {}  {}", first_no + i as i64, when, ask);
    }
    if asks.is_empty() {
        pl!("  ASK   none found — ask the user what they wanted");
    }
    pl!();
}

fn section_todos(s: &Session) {
    let mut items: Vec<(String, String)> = vec![];
    if let Some(todos) = &s.todos
        && !todos.is_empty()
    {
        for t in todos {
            let c = first_truthy(t, &["content", "activeForm"]);
            let c = if truthy(c) { py_str(c) } else { "?".to_string() };
            let st = match get(t, "status") {
                Value::Null => {
                    if t.get("status").is_some() {
                        "None".to_string()
                    } else {
                        "?".to_string()
                    }
                }
                v => py_str(v),
            };
            items.push((c, st));
        }
    }
    for (_, subj, st) in &s.tasks {
        items.push((subj.clone(), st.clone()));
    }
    if items.is_empty() {
        return;
    }
    pl!("== TODO LIST ==");
    let open: Vec<&(String, String)> =
        items.iter().filter(|(_, st)| !["completed", "deleted", "cancelled"].contains(&st.as_str())).collect();
    for (c, st) in &open {
        line("ME", &format!("{}: {}", st, short(c, 120)));
    }
    if open.is_empty() {
        line("OK", &format!("all {} todo item(s) completed", items.len()));
    }
    pl!();
}

#[derive(Clone)]
struct Bg {
    id: String,
    name: String,
    inp: Value,
    ts: Option<f64>,
    status: String,
    summary: String,
    out: String,
    deadline: i64,
    // set by advise()
    run_min: f64,
    out_age: Option<f64>,
    endless: bool,
}

fn background_tasks(s: &Session) -> Vec<Bg> {
    let mut rows = vec![];
    for uid in &s.order {
        let u = &s.uses[uid];
        let res = s.result(uid).1;
        let m = BG_RESULT.captures(res);
        let mut task_id = m
            .as_ref()
            .map(|m| m.get(1).or_else(|| m.get(2)).map(|x| x.as_str().to_string()).unwrap_or_default())
            .unwrap_or_default();
        let is_bg = m.is_some()
            || res.trim_start().starts_with("Async agent launched")
            || (get(&u.inp, "run_in_background") == &Value::Bool(true)
                && ["Bash", "Agent", "Monitor"].contains(&u.name.as_str()));
        if !is_bg {
            continue;
        }
        if task_id.is_empty() {
            let a = AGENT_ID.captures(res).or_else(|| ANY_ID.captures(res));
            task_id = a.map(|a| a[1].to_string()).unwrap_or_else(|| uid.clone());
        }
        let (mut status, summary) = s.notices.get(uid).cloned().unwrap_or_default();
        if s.stopped.contains(&task_id) && status.is_empty() {
            status = "stopped".into();
        }
        let o = OUTFILE.captures(res);
        let prompt = or_blank(get(&u.inp, "prompt"));
        let d = DEADLINE.captures(&prompt);
        let deadline = d
            .map(|d| {
                let n: i64 = d[1].parse().unwrap_or(0);
                n.saturating_mul(if d[2].starts_with(['h', 'H']) { 60 } else { 1 })
            })
            .unwrap_or(0);
        rows.push(Bg {
            id: task_id,
            name: u.name.clone(),
            inp: u.inp.clone(),
            ts: u.ts,
            status,
            summary,
            out: o.map(|o| o[1].to_string()).unwrap_or_default(),
            deadline,
            run_min: 0.0,
            out_age: None,
            endless: false,
        });
    }
    rows
}

/// STOP or KEEP for one running task, with the reason. Sets run_min, out_age and endless.
fn advise(t: &mut Bg) -> (bool, String) {
    let n = now();
    t.run_min = match t.ts {
        Some(ts) if ts != 0.0 => (n - ts) / 60.0,
        _ => 0.0,
    };
    let mt = if !t.out.is_empty() && Path::new(&t.out).exists() { mtime(Path::new(&t.out)) } else { None };
    t.out_age = mt.map(|m| (n - m) / 60.0);
    let cmd = or_blank(get(&t.inp, "command"));
    t.endless = ENDLESS.is_match(&cmd);
    let (run, quiet) = (t.run_min, t.out_age);
    if !t.out.is_empty() && mt.is_none() && run > 5.0 {
        return (true, "output file is gone, the task is dead".into());
    }
    if t.deadline != 0 && run > 1.5 * t.deadline as f64 {
        return (true, format!("past its {}m deadline", t.deadline));
    }
    if t.endless {
        if let Some(q) = quiet.filter(|q| *q >= 30.0) {
            return (false, format!("idle server ({}m quiet), stop it when work ends", q as i64));
        }
        return (false, "server, stop it when work ends".into());
    }
    if let Some(q) = quiet
        && q >= 20.0
        && run >= 20.0
    {
        return (false, format!("quiet for {}m, read its output before stopping", q as i64));
    }
    if run < 5.0 {
        return (false, "just started".into());
    }
    (false, if quiet.map(|q| q < 20.0).unwrap_or(false) { "output is recent".into() } else { "inside its time".into() })
}

fn section_background(s: &Session) {
    let mut rows = background_tasks(s);
    if rows.is_empty() && s.crons.is_empty() {
        return;
    }
    pl!("== BACKGROUND WORK ==");
    let mut stops = 0;
    for t in rows.iter_mut().filter(|t| t.status.is_empty()) {
        let (stop, why) = advise(t);
        if stop {
            stops += 1;
        }
        let kind = match t.name.as_str() {
            "Agent" => "agent".to_string(),
            "Bash" => "shell".to_string(),
            "Monitor" => "monitor".to_string(),
            other => other.to_lowercase(),
        };
        let what = vshort(first_truthy(&t.inp, &["description", "command", "prompt"]), 50);
        let last = match t.out_age {
            None => "no file".to_string(),
            Some(a) => format!("{}m ago", a as i64),
        };
        pl!(
            "  TASK  {} | {} | {} | {}m | {} | {}: {}",
            t.id,
            kind,
            what,
            t.run_min as i64,
            last,
            if stop { "STOP" } else { "KEEP" },
            why
        );
    }
    if stops > 0 {
        line("ME", &format!("stop {} dead task(s) marked STOP above (TaskStop)", stops));
    }
    for t in &rows {
        if t.status == "failed" {
            line(
                "CHECK",
                &format!("failed: {} — {} — still needed?", describe(&t.name, &t.inp), short(&t.summary, 90)),
            );
        }
    }
    for (_, what) in &s.crons {
        line("CHECK", &format!("scheduled job still set: {} — keep it?", short(what, 90)));
    }
    let done = rows.iter().filter(|t| ["completed", "killed", "stopped"].contains(&t.status.as_str())).count();
    if done > 0 {
        line("OK", &format!("{} background task(s) finished or stopped", done));
    }
    pl!();
}

fn hurry_binary() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "hurry_scan.exe" } else { "hurry_scan" };
    if let Ok(me) = std::env::current_exe()
        && let Some(dir) = me.parent()
    {
        let p = dir.join(exe);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn section_speed(path: &Path) {
    let bin = hurry_binary().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "hurry_scan".to_string());
    let (rc, out) = run(&[&bin, "--brief", "--session", &path.to_string_lossy()], None, 30);
    if rc == 0 && !out.trim().is_empty() {
        pl!("== SPEED (slow session? [ME] rows are mine, [YOU] rows are yours) ==");
        pl!("{}", out);
        pl!();
    }
}

fn section_deliverable(s: &Session, roots: &[String]) {
    let joined = s.results_keys.iter().map(|k| s.results[k].1.as_str()).collect::<Vec<_>>().join("\n");
    let urls: Vec<&str> = ARTIFACT_URL.find_iter(&joined).map(|m| m.as_str()).collect();
    let what: String;
    if let Some(u) = urls.last() {
        what = u.to_string();
    } else {
        let mut w = String::new();
        for r in roots {
            let since = match s.start {
                Some(st) if st != 0.0 => local_iso(st),
                _ => "1 day ago".to_string(),
            };
            let (_, sha) = git(r, &["log", &format!("--since={}", since), "--format=%h", "-1"], 10);
            if !sha.is_empty() {
                w = format!("{} ({})", sha, basename(r));
                break;
            }
        }
        if w.is_empty() {
            let files: Vec<String> = s
                .uses_keys
                .iter()
                .map(|k| &s.uses[k])
                .filter(|u| u.name == "Write" || u.name == "NotebookEdit")
                .filter_map(|u| {
                    let f = get(&u.inp, "file_path");
                    if truthy(f) { Some(py_str(f)) } else { None }
                })
                .collect();
            if let Some(f) = files.last() {
                w = tilde(f);
            }
        }
        what = w;
    }
    pl!("DELIVERABLE  {}\n", if what.is_empty() { "none yet" } else { &what });
}

fn up5(x: f64) -> i64 {
    ((x / 5.0).ceil() * 5.0) as i64
}

/// One ETA estimate: (mid, low, high) in minutes, from the counted rows and the running tasks.
fn estimate(s: &Session) -> (i64, i64, i64) {
    let mut serial = 0i64;
    let rows = STATE.lock().unwrap().rows.clone();
    for (tag, text) in &rows {
        let t = text.to_lowercase();
        if tag == "YOU" || t.starts_with("stop ") {
            continue;
        }
        if t.contains("ci run") || t.contains("ci failed") {
            serial += 10;
        } else if t.contains("check") || t.contains("lint") {
            serial += 2;
        } else if t.contains("pushed") || t.contains("committed") || t.contains("pulled") {
            serial += 1;
        } else if t.contains("half done") || t.contains("conflict") {
            serial += 5;
        } else if tag == "CHECK" {
            serial += 1;
        } else {
            let head = t.split(':').next().unwrap_or("");
            serial += if head == "pending" || head == "in_progress" { 3 } else { 2 };
        }
    }
    let mut parallel = 0.0f64;
    for mut t in background_tasks(s) {
        if !t.status.is_empty() || advise(&mut t).0 || t.endless {
            continue;
        }
        if t.name == "Agent" {
            let left = if t.deadline != 0 {
                t.deadline as f64 - t.run_min
            } else if BIG_AGENT.is_match(&or_blank(get(&t.inp, "prompt"))) {
                10.0
            } else {
                5.0
            };
            parallel = parallel.max(left.max(2.0));
        } else {
            parallel = parallel.max(3.0);
        }
    }
    let mid = serial as f64 + parallel;
    let low = if mid != 0.0 { 5.max(up5(mid * 0.7)) } else { 0 };
    (up5(mid), low, up5(mid * 2.0))
}

fn eta_line(s: &Session) -> String {
    let (mid, low, high) = estimate(s);
    if mid == 0 { "ETA ~0 min (nothing open)".to_string() } else { format!("ETA ~{} min ({}-{})", mid, low, high) }
}

fn section_recent(s: &Session) {
    let asks = &s.asks;
    let start = if asks.len() >= 2 { asks[asks.len() - 2].2 } else { 0 };
    let mut errs: Vec<(String, String)> = vec![];
    let mut denials: Vec<(String, String)> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    for uid in s.order.iter().skip(start) {
        let u = &s.uses[uid];
        let (is_err, res) = s.result(uid);
        if !is_err {
            continue;
        }
        let key = short(res, 60);
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        let item = (describe(&u.name, &u.inp), short(res, 100));
        if DENIED.is_match(res) { denials.push(item) } else { errs.push(item) }
    }
    if errs.is_empty() && denials.is_empty() {
        return;
    }
    pl!("== RECENT ERRORS (last work turn) ==");
    for (desc, _) in &denials[denials.len().saturating_sub(5)..] {
        line("YOU", &format!("you said no to: {} — do it yourself, or tell me another way", desc));
    }
    for (desc, res) in &errs[errs.len().saturating_sub(5)..] {
        line("CHECK", &format!("error: {} → {} — fixed later?", desc, res));
    }
    pl!();
}

/// OPEN_WORDS.search(ln), including the `\blater\b` case that needs a lookahead in Python.
fn has_open_words(ln: &str) -> bool {
    if OPEN_WORDS.is_match(ln) {
        return true;
    }
    for m in LATER.find_iter(ln) {
        let rest = ln[m.end()..].trim_start();
        if rest.is_empty() || rest.starts_with(['.', ':', '!']) {
            return true;
        }
    }
    false
}

fn section_open_words(s: &Session) {
    let mut hits: Vec<String> = vec![];
    let from = s.texts.len().saturating_sub(4);
    for t in &s.texts[from..] {
        let t = FENCE.replace_all(t, " ");
        for ln in py_splitlines(&t) {
            if has_open_words(&ln) && clen(ln.trim()) > 12 {
                hits.push(short(ln.trim_matches(|c: char| " -*#>|".contains(c)), 140));
            }
        }
    }
    let mut uniq: Vec<String> = vec![];
    for h in hits {
        if !uniq.contains(&h) {
            uniq.push(h);
        }
    }
    let from = uniq.len().saturating_sub(8);
    let hits = &uniq[from..];
    if hits.is_empty() {
        return;
    }
    pl!("== OPEN WORDS (from Claude's last replies) ==");
    for h in hits {
        line("CHECK", &format!("\"{}\"", h));
    }
    pl!();
}

// ---------------------------------------------------------------------------
// 3. Repos the session touched
// ---------------------------------------------------------------------------

fn touched_repos(s: &Session) -> Vec<String> {
    let mut dirs: BTreeSet<String> = s.cwds.iter().cloned().collect();
    dirs.insert(cwd_string());
    for k in &s.uses_keys {
        let u = &s.uses[k];
        let p = first_truthy(&u.inp, &["file_path", "notebook_path"]);
        if ["Edit", "Write", "MultiEdit", "NotebookEdit"].contains(&u.name.as_str()) && truthy(p) {
            dirs.insert(dirname(&py_str(p)));
        }
        let cmd = get(&u.inp, "command").as_str().unwrap_or("");
        for m in GIT_DIR_REF.captures_iter(cmd) {
            dirs.insert(expanduser(m[1].trim_matches(|c| c == '\'' || c == '"')));
        }
    }
    let mut roots: BTreeSet<String> = BTreeSet::new();
    for d0 in dirs {
        let mut d = d0;
        while !d.is_empty() && !Path::new(&d).is_dir() {
            d = dirname(&d);
        }
        if d.is_empty() {
            continue;
        }
        let (rc, top) = git(&d, &["rev-parse", "--show-toplevel"], 5);
        if rc == 0 && !top.is_empty() {
            roots.insert(top);
        }
    }
    roots.into_iter().collect()
}

fn in_repo(path: &str, root: &str) -> bool {
    !path.is_empty() && (path == root || path.starts_with(&format!("{}{}", root, std::path::MAIN_SEPARATOR)))
}

/// HEREDOC.sub(" ", cmd): the regex needs a backreference, so this is done by hand.
fn strip_heredocs(cmd: &str) -> String {
    let mut out = String::new();
    let mut pos = 0usize; // start of the text not yet copied
    let mut scan = 0usize; // where to look for the next head
    while scan <= cmd.len() {
        let Some(h) = HEREDOC_HEAD.captures_at(cmd, scan) else { break };
        let hm = h.get(0).unwrap();
        let tag = h.get(1).unwrap().as_str();
        match heredoc_end(cmd, hm.end(), tag) {
            Some(end) => {
                out.push_str(&cmd[pos..hm.start()]);
                out.push(' ');
                pos = end;
                scan = end;
            }
            None => {
                scan = hm.start() + cmd[hm.start()..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }
        }
    }
    out.push_str(&cmd[pos..]);
    out
}

/// After the head: `.*?\n\s*TAG[ \t]*(?=\n|$)` (dot matches newline). Returns the end of the match.
fn heredoc_end(cmd: &str, from: usize, tag: &str) -> Option<usize> {
    let bytes = cmd.as_bytes();
    let mut p = from;
    while p <= cmd.len() {
        // find the next "\n" at or after p
        let rel = cmd[p..].find('\n')?;
        let nl = p + rel;
        let rest = &cmd[nl + 1..];
        let after_ws = rest.trim_start();
        if let Some(tail) = after_ws.strip_prefix(tag) {
            let tail2 = tail.trim_start_matches([' ', '\t']);
            if tail2.is_empty() || tail2.starts_with('\n') {
                let consumed = rest.len() - tail2.len();
                return Some(nl + 1 + consumed);
            }
        }
        let _ = bytes;
        p = nl + 1;
    }
    None
}

/// The text that hides the check's exit code ('; echo', '|| true', '| tail'), or "".
/// Looks only at what follows the check command, up to the first `&&`.
fn masked_exit(bare: &str, check_end: usize) -> String {
    let pipe_ok = PIPEFAIL.is_match(bare);
    let tail = &bare[check_end..];
    let rest = tail.split("&&").next().unwrap_or("");
    let rest = REDIRECT.replace_all(rest, " ").into_owned(); // 2>&1 is a redirect, not a separator
    // ([^;|\n]*)(\|\||;|\n|\|)?\s*(.*) with re.S
    let Some(at) = rest.find([';', '|', '\n']) else { return String::new() };
    let after_sep = |n: usize| rest[at + n..].trim_start();
    let b = rest.as_bytes();
    if b[at] == b'|' && b.get(at + 1) == Some(&b'|') {
        return if TRUE_WORD.is_match(after_sep(2)) { "|| true".to_string() } else { String::new() };
    }
    if b[at] == b'|' {
        return match PIPE_WORD.captures(after_sep(1)) {
            Some(p) if !pipe_ok => format!("| {}", &p[1]),
            _ => String::new(),
        };
    }
    match ECHO_WORD.captures(after_sep(1)) {
        Some(p) => format!("; {}", &p[1]),
        None => String::new(),
    }
}

/// How the check at tool use `uid` ended: `failed`, `running`, `hidden: <mask>` or `passed`.
fn check_state(s: &Session, uid: &str, bare: &str, check_end: usize) -> String {
    let (is_err, res) = s.result(uid);
    if is_err {
        return "failed".into();
    }
    if get(&s.uses[uid].inp, "run_in_background") == &Value::Bool(true) || BG_RESULT.is_match(res) {
        let Some((status, summary)) = s.notices.get(uid) else { return "running".into() };
        if status.is_empty() {
            return "running".into();
        }
        let nonzero = EXIT_CODE.captures(summary).is_some_and(|c| &c[1] != "0");
        if status != "completed" || nonzero {
            return "failed".into();
        }
    }
    let mask = masked_exit(bare, check_end);
    if mask.is_empty() { "passed".into() } else { format!("hidden: {}", mask) }
}

fn check_freshness(s: &Session, root: &str) {
    let (mut last_edit, mut last_check): (Option<usize>, Option<usize>) = (None, None);
    let mut state = String::new();
    for uid in &s.order {
        let u = &s.uses[uid];
        let fp = first_truthy(&u.inp, &["file_path", "notebook_path"]);
        if ["Edit", "Write", "MultiEdit", "NotebookEdit"].contains(&u.name.as_str()) && in_repo(&or_blank(fp), root) {
            last_edit = Some(u.idx);
        }
        let cmd = or_blank(get(&u.inp, "command"));
        let bare = strip_heredocs(&cmd);
        let here = u.name == "Bash"
            && (in_repo(u.cwd.as_deref().unwrap_or(""), root) || expanduser_text(&bare).contains(root));
        if here && IN_PLACE_EDIT.is_match(&bare) {
            last_edit = Some(u.idx);
        }
        if here && let Some(m) = CHECK_CMD.find(&bare) {
            last_check = Some(u.idx);
            state = check_state(s, uid, &bare, m.end());
        }
    }
    let Some(le) = last_edit else { return };
    match last_check {
        None => line("ME", "files edited, but no test/lint/check ran in this repo this session"),
        Some(lc) if lc < le => line("ME", "files edited after the last check run — run the checks again"),
        _ if state == "failed" => line("ME", "the last check run failed — fix it, then run it again"),
        _ if state == "running" => {
            line("ME", "the last check is still running: wait for its notice, then read its exit line")
        }
        _ if state.starts_with("hidden: ") => line(
            "ME",
            &format!("the last check's exit code is hidden ({}): read its log or run it again without it", &state[8..]),
        ),
        _ => line("OK", "a check ran after the last edit, and it passed"),
    }
}

/// os.path.expanduser applied to a whole string (only a leading "~" counts).
fn expanduser_text(s: &str) -> String {
    if s == "~" || s.starts_with("~/") { expanduser(s) } else { s.to_string() }
}

fn new_markers(root: &str, since: Option<f64>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut cmds: Vec<Vec<String>> = vec![vec!["git".into(), "diff".into(), "HEAD".into(), "-U0".into()]];
    if let Some(st) = since.filter(|s| *s != 0.0) {
        cmds.push(
            ["git", "log", &format!("--since={}", local_iso(st)), "-p", "-U0", "--format="]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
    }
    for args in cmds {
        let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (_, diff) = run(&a, Some(root), 20);
        let mut f = "?".to_string();
        for ln in py_splitlines(&diff) {
            if ln.starts_with("+++ ") {
                f = if ln.starts_with("+++ b/") { pychars(&ln, 6, usize::MAX) } else { pychars(&ln, 4, usize::MAX) };
            } else if ln.starts_with('+') && MARKER.is_match(&ln) {
                out.push(format!("{}: {}", f, short(pychars(&ln, 1, usize::MAX).trim(), 90)));
            }
        }
    }
    let (_, untracked) = run(&["git", "ls-files", "--others", "--exclude-standard"], Some(root), 20);
    for f in py_splitlines(&untracked).into_iter().take(50) {
        let p = Path::new(root).join(&f);
        if let Some(text) = read_text(&p) {
            let head: String = text.chars().take(200_000).collect();
            for ln in py_splitlines(&head) {
                if MARKER.is_match(&ln) {
                    out.push(format!("{}: {}", f, short(ln.trim(), 90)));
                }
            }
        }
    }
    let mut uniq: Vec<String> = vec![];
    for o in out {
        if !uniq.contains(&o) {
            uniq.push(o);
        }
    }
    uniq
}

/// glob(".github/workflows/*.y*ml")
fn workflow_files(root: &str) -> bool {
    list_dir(&Path::new(root).join(".github").join("workflows")).iter().any(|p| {
        let n = fname(p);
        let b = n.as_bytes();
        if b.len() < 4 || !n.ends_with("ml") {
            return false;
        }
        (0..=b.len().saturating_sub(4)).any(|i| &b[i..i + 2] == b".y" && i + 2 <= b.len() - 2)
    })
}

fn section_repo(s: &Session, root: &str, fetch: bool, ci: bool) {
    let g = |args: &[&str], t: u64| git(root, args, t);
    pl!("== REPO {}  ({}) ==", basename(root), tilde(root));
    if fetch {
        let (rc, _) = g(&["fetch", "--quiet", "--prune"], 25);
        if rc != 0 {
            line("CHECK", "git fetch failed — remote state may be old");
        }
    }
    let (_, gd) = g(&["rev-parse", "--absolute-git-dir"], 20);
    let (_, branch) = g(&["symbolic-ref", "--quiet", "--short", "HEAD"], 20);
    let (_, main0) = g(&["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"], 20);
    let main = main0.replacen("origin/", "", 1);
    let main = if main.is_empty() { "main".to_string() } else { main };

    for (marker, what) in [
        ("MERGE_HEAD", "merge"),
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ] {
        if !gd.is_empty() && Path::new(&gd).join(marker).exists() {
            line("ME", &format!("a {} is half done — finish it", what));
            break;
        }
    }
    let (_, conflicts) = g(&["diff", "--name-only", "--diff-filter=U"], 20);
    if !conflicts.is_empty() {
        line("ME", &format!("merge conflicts in: {}", short(&py_splitlines(&conflicts).join(" "), 100)));
    }

    if branch.is_empty() {
        line("ME", "detached HEAD — put the work on a branch");
    } else if branch != main {
        line("YOU", &format!("on branch '{}', not '{}' — merge it, or open a PR? (your call)", branch, main));
    }

    let (_, status) = g(&["status", "--porcelain"], 20);
    if !status.is_empty() {
        let st = py_splitlines(&status);
        let new = st.iter().filter(|x| x.starts_with("??")).count();
        line("ME", &format!("not committed: {} changed, {} new file(s)", st.len() - new, new));
        for x in st.iter().take(6) {
            pl!("          {}", x);
        }
        if st.len() > 6 {
            pl!("          … {} more", st.len() - 6);
        }
    } else {
        line("OK", "working tree clean");
    }

    if !branch.is_empty() {
        let (rc, up) = g(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], 20);
        if rc != 0 || up.is_empty() {
            line("ME", &format!("branch '{}' was never pushed (no upstream)", branch));
        } else {
            let (_, lr) = g(&["rev-list", "--left-right", "--count", &format!("{}...HEAD", up)], 20);
            let parts: Vec<&str> = lr.split_whitespace().collect();
            let (behind, ahead): (i64, i64) = if parts.is_empty() {
                (0, 0)
            } else {
                (parts[0].parse().unwrap_or(0), parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(0))
            };
            if ahead != 0 {
                line("ME", &format!("{} commit(s) not pushed to {}", ahead, up));
            }
            if behind != 0 {
                line("ME", &format!("{} commit(s) on {} not pulled", behind, up));
            }
            if ahead == 0 && behind == 0 {
                let (_, sha) = g(&["rev-parse", "--short", "HEAD"], 20);
                line("OK", &format!("in sync with {} ({})", up, sha));
            }
        }
    }

    let (_, stash) = g(&["stash", "list"], 20);
    if !stash.is_empty() {
        let l = py_splitlines(&stash);
        line("CHECK", &format!("{} stash(es) — forgotten work? ({})", l.len(), short(&l[0], 70)));
    }

    check_freshness(s, root);

    if ci && !branch.is_empty() && workflow_files(root) {
        let (_, head) = g(&["rev-parse", "HEAD"], 20);
        let (rc, runs) = run(
            &[
                "gh",
                "run",
                "list",
                "--commit",
                &head,
                "--json",
                "name,status,conclusion",
                "-q",
                r#".[] | "\(.status)|\(.conclusion)|\(.name)""#,
            ],
            Some(root),
            20,
        );
        if rc != 0 {
            line("CHECK", "could not read CI (gh missing or not logged in)");
        } else if !runs.is_empty() {
            let rows: Vec<Vec<String>> =
                py_splitlines(&runs).iter().map(|r| r.splitn(3, '|').map(|x| x.to_string()).collect()).collect();
            let mut bad: Vec<String> = rows
                .iter()
                .filter(|r| {
                    r.len() >= 3
                        && r[0] == "completed"
                        && ["failure", "cancelled", "timed_out", "startup_failure"].contains(&r[1].as_str())
                })
                .map(|r| r[2].clone())
                .collect();
            bad.sort();
            bad.dedup();
            let pending = rows.iter().filter(|r| r[0] != "completed").count();
            if !bad.is_empty() {
                line("ME", &format!("CI failed on HEAD: {}", bad.join(", ")));
            }
            if pending > 0 {
                line("CHECK", &format!("{} CI run(s) still running on HEAD — wait, then look again", pending));
            }
            if bad.is_empty() && pending == 0 {
                line("OK", "CI green on HEAD");
            }
        }
    }

    for m in new_markers(root, s.start).into_iter().take(5) {
        line("CHECK", &format!("new TODO marker: {}", m));
    }
    pl!();
}

fn leftovers_path() -> String {
    std::env::var("MIC_LEFTOVERS")
        .unwrap_or_else(|_| Path::new(&home()).join(".claude").join("leftovers.md").to_string_lossy().into_owned())
}

fn section_leftovers(roots: &[String]) {
    let lp = leftovers_path();
    let Some(text) = read_text(Path::new(&lp)) else { return };
    let rows: Vec<String> =
        text.split_inclusive('\n').filter(|l| l.starts_with("- [ ]")).map(|l| l.trim().to_string()).collect();
    let names: HashSet<String> = roots.iter().map(|r| basename(r)).collect();
    let mine: Vec<&String> = rows.iter().filter(|r| names.iter().any(|n| r.contains(&format!("**{}**", n)))).collect();
    if mine.is_empty() {
        return;
    }
    pl!("== LEFTOVERS ({}) ==", tilde(&lp));
    for r in &mine[mine.len().saturating_sub(8)..] {
        line("LATER", &short(&pychars(r, 6, usize::MAX), 130));
    }
    pl!();
}

// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "whatsleft_scan", about = DOC, infer_long_args = true)]
struct Cli {
    /// session id (prefix) or .jsonl path; default: this session
    #[arg(long)]
    session: Option<String>,
    /// skip git fetch
    #[arg(long)]
    no_fetch: bool,
    /// skip the GitHub CI lookup
    #[arg(long)]
    no_ci: bool,
    /// max asks to print (default 25)
    #[arg(long, default_value_t = 25, allow_negative_numbers = true)]
    asks: i64,
    /// print only the ETA line (implies --no-fetch)
    #[arg(long)]
    eta: bool,
}

fn main() {
    let a = Cli::parse();
    let Some(path) = find_session(a.session.as_deref()) else {
        pl!("No session transcript found. Pass --session <id|path>.");
        pl!("VERDICT: NO SESSION — build the table from this conversation only");
        return;
    };
    let s = read_session(&path);
    if a.eta {
        QUIET.store(true, Ordering::Relaxed);
        for r in touched_repos(&s) {
            section_repo(&s, &r, false, !a.no_ci);
        }
        section_todos(&s);
        section_background(&s);
        QUIET.store(false, Ordering::Relaxed);
        pl!("{}", eta_line(&s));
        return;
    }
    let dur = ((s.end.unwrap_or(0.0) - s.start.unwrap_or(0.0)) / 60.0) as i64;
    let sid: String = fname(&path).chars().take(8).collect();
    pl!(
        "WHATSLEFT  session {} · {}h{:02}m · {} asks · {} tool calls\n",
        sid,
        dur.div_euclid(60),
        dur.rem_euclid(60),
        s.asks.len(),
        s.order.len()
    );

    let roots = touched_repos(&s);
    section_deliverable(&s, &roots);
    section_asks(&s, a.asks);
    section_todos(&s);
    section_background(&s);
    section_recent(&s);
    section_open_words(&s);
    section_speed(&path);
    for r in &roots {
        section_repo(&s, r, !a.no_fetch, !a.no_ci);
    }
    if roots.is_empty() {
        pl!("== REPOS ==\n  OK    no git repo touched\n");
    }
    section_leftovers(&roots);

    pl!("{}", eta_line(&s));
    let (me, you, check, later) = {
        let st = STATE.lock().unwrap();
        (st.me, st.you, st.check, st.later)
    };
    let total = me + you + check;
    let later_s = if later > 0 { format!(" · LATER {}", later) } else { String::new() };
    let head = if total > 0 { format!("{} open", total) } else { "NOTHING OPEN FOUND".to_string() };
    pl!(
        "VERDICT: {} — ME {} · YOU {} · CHECK {}{} (+ {} asks to cross-check)",
        head,
        me,
        you,
        check,
        later_s,
        s.asks.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_collapses_and_cuts() {
        assert_eq!(short("  a \n b\t c ", 150), "a b c");
        assert_eq!(short("abcdef", 4), "abc…");
        assert_eq!(short("abcd", 4), "abcd");
    }

    #[test]
    fn clean_ask_rules() {
        assert_eq!(clean_ask("<system-reminder>x</system-reminder>fix it"), "fix it");
        assert_eq!(clean_ask("<command-name>/foo</command-name><command-args>bar</command-args>"), "/foo bar");
        assert_eq!(clean_ask("<task-notification>x</task-notification>"), "");
        assert_eq!(clean_ask("[Request interrupted by user]"), "");
        assert_eq!(clean_ask("see <pasted_content a=\"1\">big</pasted_content> now"), "see [pasted text] now");
    }

    #[test]
    fn open_words_later_needs_a_stop() {
        assert!(has_open_words("I will do it later."));
        assert!(has_open_words("do it later"));
        assert!(has_open_words("do it later  "));
        assert!(!has_open_words("later today we ship"));
        assert!(has_open_words("TODO: wire it up"));
        assert!(has_open_words("this is still failing"));
        assert!(!has_open_words("it failed"));
    }

    #[test]
    fn heredoc_bodies_are_removed() {
        let cmd = "cat <<'EOF'\npytest is text\nEOF\nls";
        let bare = strip_heredocs(cmd);
        assert!(!CHECK_CMD.is_match(&bare), "{bare:?}");
        assert!(bare.ends_with("\nls"));
        let cmd = "pytest -q <<EOF\nx\nEOF";
        assert!(CHECK_CMD.is_match(&strip_heredocs(cmd)));
        // no terminator: left alone
        assert_eq!(strip_heredocs("cat <<EOF\nno end"), "cat <<EOF\nno end");
    }

    #[test]
    fn marker_needs_a_comment_sign_or_colon() {
        assert!(MARKER.is_match("+  // TODO fix"));
        assert!(MARKER.is_match("FIXME(bob): later"));
        assert!(!MARKER.is_match("a TODO list"));
    }

    #[test]
    fn workflow_files_finds_yaml() {
        let d = tempfile::tempdir().unwrap();
        let wf = d.path().join(".github").join("workflows");
        std::fs::create_dir_all(&wf).unwrap();
        let root = d.path().to_str().unwrap();
        std::fs::write(wf.join("ci.json"), "").unwrap();
        assert!(!workflow_files(root));
        std::fs::write(wf.join("ci.yml"), "").unwrap();
        assert!(workflow_files(root));
        std::fs::remove_file(wf.join("ci.yml")).unwrap();
        std::fs::write(wf.join("ci.yaml"), "").unwrap();
        assert!(workflow_files(root));
    }

    #[test]
    fn up5_and_tail() {
        assert_eq!(up5(0.0), 0);
        assert_eq!(up5(1.0), 5);
        assert_eq!(up5(5.0), 5);
        assert_eq!(up5(5.1), 10);
        assert_eq!(tail_start(30, 25), 5);
        assert_eq!(tail_start(3, 25), 0);
        assert_eq!(tail_start(3, 0), 0);
    }

    #[test]
    fn bg_result_only_at_start() {
        assert!(BG_RESULT.is_match("Command running in background with ID: abc123"));
        assert!(BG_RESULT.is_match("  Command running in background with ID: abc123"));
        assert!(!BG_RESULT.is_match("echo Command running in background with ID: abc123"));
    }

    #[test]
    fn iso_local_has_micros_only_when_needed() {
        let s = local_iso(1_700_000_000.0);
        assert_eq!(s.len(), 19);
        let s = local_iso(1_700_000_000.5);
        assert!(s.ends_with(".500000"));
    }
}
