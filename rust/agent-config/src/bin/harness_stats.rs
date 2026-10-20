//! harness_stats: measure Claude Code sessions from local transcripts.
//! Port of the old Python tool harness_stats.py (deleted; its tests are mapped in tests/PORTED.md).

use clap::{Args, Parser, Subcommand};
use regex::Regex;
use serde_json::{Map, Number, Value};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::Hash;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

const DOC: &str = "harness_stats: measure Claude Code sessions from local transcripts.

  harness_stats collect [--since 2026-10-01] [--out FILE] [--exclude SID ...]
  harness_stats cuts    [--repo ~/dotfiles | --gh OWNER/REPO] [--since DATE] [--path claude]
  harness_stats report  [--in FILE] [--cut ISO ...] [--by period|project]
  harness_stats errors  [--in FILE] [--project NAME] [--top 12] [--daily]
  harness_stats skills  [--in FILE]
  harness_stats inventory [--in FILE] [--top 8]
  harness_stats agents  [--in FILE] [--cut ISO ...]

collect reads ~/.claude/projects/*/*.jsonl (main sessions), their subagents/*.jsonl and
.meta.json, and the desktop app's session records (title, archived). It writes one JSON
row per main session. Everything else reads that file. Default file:
$TMPDIR/harness_stats.json.";

const INTERRUPT: &str = "[Request interrupted by user";
const POLL_TOOLS: [&str; 3] = ["ScheduleWakeup", "CronCreate", "Monitor"];

static CORRECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(why did you|that'?s wrong|is wrong|again\?|you didn'?t|not what i (asked|wanted|said)|ugh|wtf|still (not|broken|wrong|failing)|as i said|i said|i told you|i already (said|told))\b",
    )
    .unwrap()
});
static HANDOFF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Read \S+\.md and do it").unwrap());
static CD_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:cd [^&;]+(?:&&|;)\s*)+").unwrap());
static PATHS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/private/tmp/\S+|/Users/[^\s/]+/\S*").unwrap());
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());
static PROJ_WT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"--claude-worktrees-.*").unwrap());
static WORK_PFX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^work-").unwrap());
static PATHS_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^paths:").unwrap());

static NULL: Value = Value::Null;
static EMPTY: LazyLock<Value> = LazyLock::new(|| Value::Object(Map::new()));

macro_rules! out {
    () => { let _ = writeln!(std::io::stdout()); };
    ($($a:tt)*) => { let _ = writeln!(std::io::stdout(), $($a)*); };
}

// ---------------------------------------------------------------------------
// Python-compat helpers
// ---------------------------------------------------------------------------

fn home() -> String {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return h;
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return h;
        }
    }
    dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "~".into())
}

fn expanduser(p: &str) -> String {
    if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{}", home().trim_end_matches('/'), rest)
    } else {
        p.to_string()
    }
}

fn default_out() -> String {
    let tmp = std::env::var("TMPDIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if cfg!(windows) {
            std::env::temp_dir().to_string_lossy().into_owned()
        } else {
            "/tmp".to_string()
        }
    });
    Path::new(&tmp).join("harness_stats.json").to_string_lossy().into_owned()
}

fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&NULL)
}

/// Python truthiness of a JSON value.
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

/// `x or {}` for an object-ish value.
fn or_empty(v: &Value) -> &Value {
    if truthy(v) && v.is_object() { v } else { &EMPTY }
}

fn key_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn as_i(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        _ => 0,
    }
}

fn pychars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Python `s[:n]` for a signed n.
fn slice_end(len: usize, n: i64) -> usize {
    if n >= 0 { (n as usize).min(len) } else { (len as i64 + n).max(0) as usize }
}

/// Python repr(float).
fn py_float(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    if !f.is_finite() {
        return if f.is_nan() { "NaN".into() } else if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let e = format!("{:e}", f.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    let body = if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                format!("{}{}.0", digits, "0".repeat(int_len - digits.len()))
            } else {
                format!("{}.{}", &digits[..int_len], &digits[int_len..])
            }
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        }
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.clone() };
        format!("{}e{}{:02}", m, if exp < 0 { "-" } else { "+" }, exp.abs())
    };
    format!("{}{}", sign, body)
}

/// Python round(x, nd) for nd >= 0 (correctly rounded, ties to even on the exact value).
fn pyround(x: f64, nd: usize) -> f64 {
    format!("{:.*}", nd, x).parse().unwrap_or(x)
}

/// Python round(x) -> int.
fn pyround0(x: f64) -> i64 {
    x.round_ties_even() as i64
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// json.dumps(v) with Python defaults (ensure_ascii, ", " and ": ").
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

/// bytes -> str like open(errors="ignore"): invalid sequences are dropped.
fn decode_ignore(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                return out;
            }
            Err(e) => {
                let (ok, after) = rest.split_at(e.valid_up_to());
                out.push_str(std::str::from_utf8(ok).unwrap());
                let skip = e.error_len().unwrap_or(after.len());
                rest = &after[skip..];
            }
        }
    }
}

fn read_text_ignore(p: &Path) -> Option<String> {
    fs::read(p).ok().map(|b| decode_ignore(&b))
}

// Civil date helpers (proleptic Gregorian), Howard Hinnant's algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 }
        }
    }
}

/// An aware datetime: UTC microseconds plus the original UTC offset in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dt {
    micros: i64,
    off: i64,
}

impl Dt {
    fn isoformat(&self) -> String {
        let local = self.micros + self.off * 1_000_000;
        let secs = local.div_euclid(1_000_000);
        let us = local.rem_euclid(1_000_000);
        let days = secs.div_euclid(86400);
        let sod = secs.rem_euclid(86400);
        let (y, m, d) = civil_from_days(days);
        let mut s = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, m, d, sod / 3600, sod % 3600 / 60, sod % 60);
        if us != 0 {
            s.push_str(&format!(".{:06}", us));
        }
        let sign = if self.off < 0 { '-' } else { '+' };
        let a = self.off.abs();
        s.push_str(&format!("{}{:02}:{:02}", sign, a / 3600, a % 3600 / 60));
        if a % 60 != 0 {
            s.push_str(&format!(":{:02}", a % 60));
        }
        s
    }
    fn timestamp(&self) -> f64 {
        self.micros as f64 / 1e6
    }
}

/// datetime.fromisoformat(s.replace("Z", "+00:00")). A missing offset counts as UTC.
fn parse_ts(s: &str) -> Option<Dt> {
    let s = s.replace('Z', "+00:00");
    let b: Vec<char> = s.chars().collect();
    let num = |a: usize, n: usize| -> Option<i64> {
        let t: String = b.get(a..a + n)?.iter().collect();
        if t.chars().all(|c| c.is_ascii_digit()) { t.parse().ok() } else { None }
    };
    if b.len() < 10 || b[4] != '-' || b[7] != '-' {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if !(1..=12).contains(&mo) || y < 1 || d < 1 || d > days_in_month(y, mo) {
        return None;
    }
    let (mut hh, mut mm, mut ss, mut us, mut off) = (0, 0, 0, 0i64, 0i64);
    if b.len() > 10 {
        // separator at 10, then the time
        let rest: Vec<char> = b[11..].to_vec();
        let tz_at = rest.iter().position(|c| *c == '+' || *c == '-');
        let (time, tz) = match tz_at {
            Some(i) => (&rest[..i], Some(&rest[i..])),
            None => (&rest[..], None),
        };
        let ts: String = time.iter().collect();
        let (hms, frac) = match ts.split_once('.') {
            Some((a, f)) => (a.to_string(), Some(f.to_string())),
            None => (ts.clone(), None),
        };
        let parts: Vec<&str> = hms.split(':').collect();
        let pn = |x: &str, w: usize| -> Option<i64> {
            if x.len() == w && x.chars().all(|c| c.is_ascii_digit()) { x.parse().ok() } else { None }
        };
        hh = pn(parts.first()?, 2)?;
        if parts.len() > 1 {
            mm = pn(parts[1], 2)?;
        }
        if parts.len() > 2 {
            ss = pn(parts[2], 2)?;
        }
        if parts.len() > 3 {
            return None;
        }
        if let Some(f) = frac {
            if f.is_empty() || f.len() > 9 || !f.chars().all(|c| c.is_ascii_digit()) || parts.len() < 3 {
                return None;
            }
            let mut p6 = f.clone();
            p6.truncate(6);
            while p6.len() < 6 {
                p6.push('0');
            }
            us = p6.parse().ok()?;
        }
        if hh > 23 || mm > 59 || ss > 59 {
            return None;
        }
        if let Some(tz) = tz {
            let sign = if tz[0] == '-' { -1 } else { 1 };
            let t: String = tz[1..].iter().collect();
            let p: Vec<&str> = t.split(':').collect();
            if p.len() < 2 || p.len() > 3 {
                return None;
            }
            let (th, tm) = (pn(p[0], 2)?, pn(p[1], 2)?);
            let tsec = if p.len() == 3 { pn(p[2], 2)? } else { 0 };
            if th > 23 || tm > 59 || tsec > 59 {
                return None;
            }
            off = sign * (th * 3600 + tm * 60 + tsec);
        }
    }
    let days = days_from_civil(y, mo, d);
    let secs = days * 86400 + hh * 3600 + mm * 60 + ss - off;
    Some(Dt { micros: secs * 1_000_000 + us, off })
}

// ---------------------------------------------------------------------------
// Insertion-ordered counter (collections.Counter)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Counter<K: Clone + Eq + Hash> {
    keys: Vec<K>,
    idx: HashMap<K, usize>,
    vals: Vec<i64>,
}

impl<K: Clone + Eq + Hash> Counter<K> {
    fn new() -> Self {
        Counter { keys: vec![], idx: HashMap::new(), vals: vec![] }
    }
    fn add(&mut self, k: K, n: i64) {
        match self.idx.get(&k) {
            Some(&i) => self.vals[i] += n,
            None => {
                self.idx.insert(k.clone(), self.keys.len());
                self.keys.push(k);
                self.vals.push(n);
            }
        }
    }
    fn get(&self, k: &K) -> i64 {
        self.idx.get(k).map(|&i| self.vals[i]).unwrap_or(0)
    }
    fn items(&self) -> Vec<(K, i64)> {
        self.keys.iter().cloned().zip(self.vals.iter().cloned()).collect()
    }
    fn update(&mut self, other: &Counter<K>) {
        for (k, v) in other.items() {
            self.add(k, v);
        }
    }
    /// Descending by count, ties in insertion order; `None` = all.
    fn most_common(&self, n: Option<i64>) -> Vec<(K, i64)> {
        let mut it = self.items();
        it.sort_by_key(|(_, v)| Reverse(*v));
        match n {
            None => it,
            Some(n) if n <= 0 => vec![],
            Some(n) => {
                it.truncate(n as usize);
                it
            }
        }
    }
    fn total(&self) -> i64 {
        self.vals.iter().sum()
    }
}

impl Counter<String> {
    fn to_json(&self, only: Option<&[(String, i64)]>) -> Value {
        let mut m = Map::new();
        match only {
            Some(items) => {
                for (k, v) in items {
                    m.insert(k.clone(), Value::from(*v));
                }
            }
            None => {
                for (k, v) in self.keys.iter().zip(&self.vals) {
                    m.insert(k.clone(), Value::from(*v));
                }
            }
        }
        Value::Object(m)
    }
    fn from_json(v: &Value) -> Self {
        let mut c = Counter::new();
        if let Some(o) = v.as_object() {
            for (k, x) in o {
                c.add(k.clone(), as_i(x));
            }
        }
        c
    }
}

fn fnum(x: f64) -> Value {
    Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// Names and transcripts
// ---------------------------------------------------------------------------

fn project_name(folder: &str) -> String {
    let h = home();
    let p = PROJ_WT.replace(folder, "").into_owned();
    let hm = format!("-{}", h.trim_matches('/').replace('/', "-"));
    if p == hm {
        return "~".into();
    }
    if p.contains("scratch-workspaces") {
        return "scratch".into();
    }
    if p.starts_with("-private-") {
        return "tmp".into();
    }
    let p = if let Some(r) = p.strip_prefix(&format!("{}-", hm)) {
        r.to_string()
    } else {
        p.trim_start_matches('-').to_string()
    };
    let r = WORK_PFX.replace(&p, "").into_owned();
    if r.is_empty() { "?".into() } else { r }
}

/// Entries of `dir` whose name does not start with "." (glob "*"), in directory order.
fn list_dir(dir: &Path) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| e.path())
            .collect(),
        Err(_) => vec![],
    }
}

fn is_dir(p: &Path) -> bool {
    p.is_dir()
}

fn lexists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

fn fname(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

struct DesktopMeta {
    title: Value,
    archived: bool,
    effort: Value,
}

fn desktop_meta(desktop: &Path) -> HashMap<String, DesktopMeta> {
    let mut meta = HashMap::new();
    for d1 in list_dir(desktop) {
        for d2 in list_dir(&d1) {
            for f in list_dir(&d2) {
                let n = fname(&f);
                if !(n.starts_with("local_") && n.ends_with(".json")) {
                    continue;
                }
                let Some(txt) = fs::read(&f).ok() else { continue };
                let Ok(d) = serde_json::from_slice::<Value>(&txt) else { continue };
                let sid = get(&d, "cliSessionId");
                if truthy(sid) {
                    meta.insert(
                        key_str(sid),
                        DesktopMeta {
                            title: get(&d, "title").clone(),
                            archived: truthy(get(&d, "isArchived")),
                            effort: get(&d, "effort").clone(),
                        },
                    );
                }
            }
        }
    }
    meta
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter(|x| x.is_object() && get(x, "type").as_str() == Some("text"))
            .map(|x| get(x, "text").as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn norm_error(name: &str, content: &Value) -> String {
    let t = match content {
        Value::String(s) => s.clone(),
        other => py_dumps(other),
    };
    let t = PATHS.replace_all(&t, "<path>").into_owned();
    let t = DIGITS.replace_all(&t, "N").replace('\n', " ");
    format!("{}: {}", name, pychars(&t, 110))
}

struct Scan {
    r: Counter<String>,
    tools: Counter<String>,
    skills: Counter<String>,
    bash: Counter<String>,
    errors: Counter<String>,
    agents: Vec<Value>,
    times: Vec<Dt>,
    first_ctx: i64,
    peak: i64,
    models: Counter<Option<String>>,
    active_s: f64,
    version: Value,
}

fn scan(path: &Path, is_sub: bool) -> Scan {
    let mut r: Counter<String> = Counter::new();
    let (mut tools, mut skills, mut bash, mut errors) = (Counter::new(), Counter::new(), Counter::new(), Counter::new());
    let mut agents: Vec<Value> = vec![];
    let mut times: Vec<Dt> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let mut models: Counter<Option<String>> = Counter::new();
    let (mut first_ctx, mut peak): (Option<i64>, i64) = (None, 0);
    let mut names: HashMap<String, String> = HashMap::new();
    let mut version = Value::Null;
    let text = read_text_ignore(path).unwrap_or_default();
    for line in text.split('\n') {
        let Ok(o) = serde_json::from_str::<Value>(line) else { continue };
        if !o.is_object() {
            continue;
        }
        if truthy(get(&o, "timestamp")) {
            if let Some(s) = get(&o, "timestamp").as_str() {
                if let Some(d) = parse_ts(s) {
                    times.push(d);
                }
            }
        }
        let kind = get(&o, "type").as_str().unwrap_or("");
        let msg = or_empty(get(&o, "message"));
        if !truthy(&version) {
            version = get(&o, "version").clone();
        }
        match kind {
            "system" => match get(&o, "subtype").as_str() {
                Some("compact_boundary") => r.add("compactions".into(), 1),
                Some("api_error") => r.add("api_errors".into(), 1),
                _ => {}
            },
            "assistant" => {
                if let Some(arr) = get(msg, "content").as_array() {
                    for c in arr {
                        if !c.is_object() || get(c, "type").as_str() != Some("tool_use") {
                            continue;
                        }
                        let n = get(c, "name").as_str().unwrap_or("").to_string();
                        let inp = or_empty(get(c, "input"));
                        names.insert(get(c, "id").to_string(), n.clone());
                        tools.add(n.clone(), 1);
                        if n == "Skill" {
                            let sk = match get(inp, "skill") {
                                Value::String(s) => s.clone(),
                                Value::Null => "?".to_string(),
                                other => other.to_string(),
                            };
                            skills.add(sk, 1);
                        } else if n == "Agent" || n == "Task" {
                            let st = get(inp, "subagent_type");
                            let ty = if truthy(st) { st.clone() } else { Value::from("general-purpose") };
                            let plen = match get(inp, "prompt") {
                                Value::String(s) => s.chars().count(),
                                Value::Array(a) => a.len(),
                                _ => 0,
                            };
                            let mut m = Map::new();
                            m.insert("type".into(), ty);
                            m.insert("model".into(), get(inp, "model").clone());
                            m.insert("effort".into(), get(inp, "effort").clone());
                            m.insert("bg".into(), get(inp, "run_in_background").clone());
                            m.insert("plen".into(), Value::from(plen));
                            agents.push(Value::Object(m));
                        } else if n == "Bash" {
                            let cmd_s = get(inp, "command").as_str().unwrap_or("").trim().to_string();
                            let stripped = CD_PREFIX.replace(&cmd_s, "").into_owned();
                            let cmd: Vec<&str> = stripped.split_whitespace().collect();
                            const TWO: [&str; 13] = [
                                "git", "npm", "pnpm", "bun", "go", "uv", "npx", "gh", "wrangler", "python3", "node", "make", "cargo",
                            ];
                            if !cmd.is_empty() {
                                let k = if TWO.contains(&cmd[0]) {
                                    cmd.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
                                } else {
                                    pychars(cmd[0], 24)
                                };
                                bash.add(k, 1);
                            }
                        }
                    }
                }
                let mid = get(msg, "id").to_string();
                let u = or_empty(get(msg, "usage"));
                let model = get(msg, "model");
                if seen.contains(&mid) || !truthy(u) || model.as_str() == Some("<synthetic>") {
                    continue;
                }
                seen.insert(mid);
                let inp_t = as_i(get(u, "input_tokens"));
                let cr = as_i(get(u, "cache_read_input_tokens"));
                let cc = as_i(get(u, "cache_creation_input_tokens"));
                let ctx = inp_t + cr + cc;
                let mk = match model {
                    Value::Null => None,
                    Value::String(s) => Some(s.clone()),
                    other => Some(other.to_string()),
                };
                models.add(mk, 1);
                r.add("calls".into(), 1);
                r.add("out".into(), as_i(get(u, "output_tokens")));
                r.add("fresh_in".into(), inp_t + cc);
                r.add("cache_read".into(), cr);
                if first_ctx.is_none() && ctx != 0 {
                    first_ctx = Some(ctx);
                }
                peak = peak.max(ctx);
            }
            "user" => {
                let content = get(msg, "content");
                if let Some(arr) = content.as_array() {
                    if arr.iter().any(|x| x.is_object() && get(x, "type").as_str() == Some("tool_result")) {
                        for x in arr {
                            if x.is_object() && get(x, "type").as_str() == Some("tool_result") && truthy(get(x, "is_error")) {
                                let nm = names.get(&get(x, "tool_use_id").to_string()).cloned().unwrap_or_else(|| "?".into());
                                let e = norm_error(&nm, get(x, "content"));
                                if e.contains("hook error") {
                                    r.add("hook_denials".into(), 1);
                                } else {
                                    r.add("tool_errors".into(), 1);
                                }
                                errors.add(e, 1);
                            }
                        }
                        continue;
                    }
                }
                if truthy(get(&o, "isMeta"))
                    || truthy(get(&o, "isCompactSummary"))
                    || (truthy(get(&o, "isSidechain")) && !is_sub)
                {
                    continue;
                }
                let t = text_of(content);
                if t.starts_with(INTERRUPT) {
                    r.add("interrupts".into(), 1);
                    continue;
                }
                if t.trim().is_empty() || (t.trim_start().starts_with('<') && !t.contains("command-name")) {
                    continue;
                }
                r.add("prompts".into(), 1);
                if CORRECTION.is_match(&t) {
                    r.add("corrections".into(), 1);
                }
                if r.get(&"prompts".to_string()) == 1 && HANDOFF.is_match(&t) {
                    // r["from_handoff"] = 1
                    if r.idx.contains_key("from_handoff") {
                        let i = r.idx["from_handoff"];
                        r.vals[i] = 1;
                    } else {
                        r.add("from_handoff".into(), 1);
                    }
                }
            }
            _ => {}
        }
    }
    times.sort_by_key(|d| d.micros);
    let mut active = 0.0f64;
    for w in times.windows(2) {
        let g = (w[1].micros - w[0].micros) as f64 / 1e6;
        if g < 600.0 {
            active += g;
        }
    }
    Scan { r, tools, skills, bash, errors, agents, times, first_ctx: first_ctx.unwrap_or(0), peak, models, active_s: active, version }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn fail(msg: &str) -> ! {
    eprintln!("{}", msg);
    std::process::exit(1);
}

fn cmd_collect(a: &CollectArgs) {
    let since = if !a.since.contains('T') {
        parse_ts(&format!("{}T00:00:00+00:00", a.since))
    } else {
        parse_ts(&a.since)
    }
    .unwrap_or_else(|| fail(&format!("ValueError: Invalid isoformat string: '{}'", a.since)));
    let h = home();
    let projects = Path::new(&h).join(".claude").join("projects");
    let desktop = Path::new(&h).join("Library").join("Application Support").join("Claude").join("claude-code-sessions");
    let meta = desktop_meta(&desktop);
    let mut rows: Vec<Value> = vec![];
    for pdir in list_dir(&projects) {
        for f in list_dir(&pdir) {
            let n = fname(&f);
            if !n.ends_with(".jsonl") {
                continue;
            }
            let sid = n[..n.len() - 6].to_string();
            let mtime = fs::metadata(&f)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| match t.duration_since(UNIX_EPOCH) {
                    Ok(d) => d.as_secs_f64(),
                    Err(e) => -e.duration().as_secs_f64(),
                });
            let Some(mtime) = mtime else { continue };
            if a.exclude.contains(&sid) || mtime < since.timestamp() {
                continue;
            }
            let s = scan(&f, false);
            if s.times.is_empty() || s.times[0].micros < since.micros || s.r.get(&"calls".to_string()) == 0 {
                continue;
            }
            let folder = fname(&pdir);
            let (mut sn, mut scalls, mut sfresh, mut scache, mut sout, mut serr) = (0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
            let mut smodels: Counter<String> = Counter::new();
            let mut seffort: Counter<String> = Counter::new();
            let mut spawns: Vec<Value> = vec![];
            let mut errors = s.errors.clone();
            for sf in list_dir(&pdir.join(&sid).join("subagents")) {
                let sfn = fname(&sf);
                if !sfn.ends_with(".jsonl") {
                    continue;
                }
                let ss = scan(&sf, true);
                sn += 1;
                scalls += ss.r.get(&"calls".to_string());
                sfresh += ss.r.get(&"fresh_in".to_string());
                scache += ss.r.get(&"cache_read".to_string());
                sout += ss.r.get(&"out".to_string());
                serr += ss.r.get(&"tool_errors".to_string());
                errors.update(&ss.errors);
                let mut md = Value::Object(Map::new());
                let mp = format!("{}.meta.json", &sf.to_string_lossy()[..sf.to_string_lossy().len() - 6]);
                if let Some(b) = fs::read(&mp).ok() {
                    if let Ok(v) = serde_json::from_slice::<Value>(&b) {
                        let m = get(&v, "model");
                        smodels.add(if truthy(m) { key_str(m) } else { "inherit".into() }, 1);
                        let e = get(&v, "effort");
                        seffort.add(if truthy(e) { key_str(e) } else { "unset".into() }, 1);
                        md = v;
                    }
                }
                if ss.first_ctx != 0 {
                    let at = get(&md, "agentType");
                    let at = if truthy(at) { at.clone() } else { Value::from("?") };
                    spawns.push(Value::Array(vec![at, Value::from(ss.first_ctx)]));
                }
            }
            let m = meta.get(&sid);
            let mut sub = Map::new();
            sub.insert("n".into(), Value::from(sn));
            sub.insert("calls".into(), Value::from(scalls));
            sub.insert("fresh_in".into(), Value::from(sfresh));
            sub.insert("cache_read".into(), Value::from(scache));
            sub.insert("out".into(), Value::from(sout));
            sub.insert("tool_errors".into(), Value::from(serr));
            sub.insert("models".into(), smodels.to_json(None));
            sub.insert("effort".into(), seffort.to_json(None));
            sub.insert("spawns".into(), Value::Array(spawns));
            let first = s.times[0];
            let last = *s.times.last().unwrap();
            let mut row = Map::new();
            row.insert("sid".into(), Value::from(sid.clone()));
            row.insert("project".into(), Value::from(project_name(&folder)));
            row.insert("folder".into(), Value::from(folder.clone()));
            row.insert("start".into(), Value::from(first.isoformat()));
            row.insert("end".into(), Value::from(last.isoformat()));
            row.insert("title".into(), m.map(|m| m.title.clone()).unwrap_or(Value::Null));
            row.insert("archived".into(), m.map(|m| Value::Bool(m.archived)).unwrap_or(Value::Null));
            row.insert("effort".into(), m.map(|m| m.effort.clone()).unwrap_or(Value::Null));
            row.insert("cli".into(), s.version.clone());
            row.insert(
                "model".into(),
                s.models.most_common(Some(1)).first().and_then(|(k, _)| k.clone()).map(Value::from).unwrap_or(Value::Null),
            );
            row.insert("first_ctx".into(), Value::from(s.first_ctx));
            row.insert("peak_ctx".into(), Value::from(s.peak));
            row.insert("active_min".into(), fnum(pyround(s.active_s / 60.0, 1)));
            row.insert("wall_min".into(), fnum(pyround((last.micros - first.micros) as f64 / 1e6 / 60.0, 1)));
            row.insert("stats".into(), s.r.to_json(None));
            row.insert("tools".into(), s.tools.to_json(None));
            row.insert("skills".into(), s.skills.to_json(None));
            row.insert("bash".into(), s.bash.to_json(Some(&s.bash.most_common(Some(15)))));
            row.insert("errors".into(), errors.to_json(Some(&errors.most_common(Some(20)))));
            row.insert("agent_calls".into(), Value::Array(s.agents.clone()));
            row.insert("sub".into(), Value::Object(sub));
            rows.push(Value::Object(row));
        }
    }
    let body = py_dumps(&Value::Array(rows.clone()));
    if let Err(e) = fs::write(&a.out, body) {
        fail(&format!("OSError: {}: {}", a.out, e));
    }
    out!("{} sessions -> {}", rows.len(), a.out);
}

fn run_lines(cmd: &str, args: &[String]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => String::new(),
    }
}

fn sv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn cmd_cuts(a: &CutsArgs) {
    let outp: String;
    if let Some(gh) = &a.gh {
        let since = if a.since.contains('T') { a.since.clone() } else { format!("{}T00:00:00Z", a.since) };
        let args = vec![
            "api".to_string(),
            format!("repos/{}/commits?since={}&path={}&per_page=100", gh, since, a.path),
            "--paginate".into(),
            "--jq".into(),
            r#".[] | "\(.commit.committer.date) \(.sha[0:7]) \(.commit.message|split("\n")[0])""#.into(),
        ];
        outp = run_lines("gh", &args);
    } else {
        let repo = expanduser(&a.repo);
        outp = run_lines("git", &sv(&["-C", &repo, "log", "--since", &a.since, "--format=%cI %h %s", "--", &a.path]));
        let root0 = run_lines("git", &sv(&["-C", &repo, "log", "--reverse", "--format=%cI", "--max-count=1"]));
        let first_s = run_lines("git", &sv(&["-C", &repo, "rev-list", "--max-parents=0", "--format=%cI", "HEAD"]));
        let first: Vec<&str> = first_s.lines().collect();
        let root = first.last().map(|s| s.to_string()).unwrap_or(root0);
        out!("Repo history starts {}. If that is inside your window, the older pushes are in another repo:", root);
        out!("  gh repo list --limit 50   then   harness_stats cuts --gh OWNER/REPO --since {}\n", a.since);
    }
    let mut commits: Vec<(Dt, String)> = vec![];
    for l in outp.lines() {
        if l.is_empty() {
            continue;
        }
        let mut sp = l.splitn(2, char::is_whitespace);
        let first = sp.next().unwrap_or("");
        let rest = sp.next().unwrap_or("").trim_start();
        let Some(t) = parse_ts(first) else {
            fail(&format!("ValueError: Invalid isoformat string: '{}'", first));
        };
        commits.push((t, rest.to_string()));
    }
    commits.sort_by(|x, y| x.0.micros.cmp(&y.0.micros).then_with(|| x.1.cmp(&y.1)));
    let mut groups: Vec<Vec<(Dt, String)>> = vec![];
    for (t, s) in commits {
        match groups.last_mut() {
            Some(g) if t.micros - g.last().unwrap().0.micros < 3 * 3600 * 1_000_000 => g.push((t, s)),
            _ => groups.push(vec![(t, s)]),
        }
    }
    for g in &groups {
        out!("{}  {:>2} commits  {}", g[0].0.isoformat(), g.len(), pychars(&g[0].1, 70));
        for (_, s) in g.iter().skip(1).take(3) {
            out!("    {}", pychars(s, 76));
        }
    }
    out!("\nUse the first time of a group as --cut. Commits from an older history are missing here.");
}

fn load(inp: &str) -> Vec<Value> {
    let b = fs::read(inp).unwrap_or_else(|e| fail(&format!("FileNotFoundError: {}: {}", inp, e)));
    match serde_json::from_slice::<Value>(&b) {
        Ok(Value::Array(a)) => a,
        Ok(_) => fail("TypeError: harness_stats file is not a list of rows"),
        Err(e) => fail(&format!("json.decoder.JSONDecodeError: {}", e)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Num {
    I(i64),
    F(f64),
}

impl Num {
    fn f(&self) -> f64 {
        match self {
            Num::I(i) => *i as f64,
            Num::F(f) => *f,
        }
    }
    fn show(&self) -> String {
        match self {
            Num::I(i) => i.to_string(),
            Num::F(f) => py_float(*f),
        }
    }
}

fn num_of(v: &Value) -> Option<Num> {
    match v {
        Value::Number(n) => Some(if let Some(i) = n.as_i64() { Num::I(i) } else { Num::F(n.as_f64().unwrap_or(0.0)) }),
        _ => None,
    }
}

/// statistics.median over the non-null values; 0 for none.
fn med(xs: &[Num]) -> Num {
    if xs.is_empty() {
        return Num::I(0);
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.f().partial_cmp(&b.f()).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        Num::F((v[n / 2 - 1].f() + v[n / 2].f()) / 2.0)
    }
}

fn nums<'a, F: Fn(&'a Value) -> &'a Value>(rows: &[&'a Value], f: F) -> Vec<Num> {
    rows.iter().filter_map(|r| num_of(f(r))).collect()
}

fn stat_i(r: &Value, k: &str) -> i64 {
    as_i(get(get(r, "stats"), k))
}

fn obj_sum(v: &Value) -> i64 {
    v.as_object().map(|o| o.values().map(as_i).sum()).unwrap_or(0)
}

fn summarize(rows: &[&Value]) -> Vec<(&'static str, String)> {
    let n = if rows.is_empty() { 1 } else { rows.len() } as f64;
    let g = |k: &str| -> i64 { rows.iter().map(|r| stat_i(r, k)).sum() };
    let prompts = { let p = g("prompts"); if p == 0 { 1 } else { p } } as f64;
    let tools = { let t: i64 = rows.iter().map(|r| obj_sum(get(r, "tools"))).sum(); if t == 0 { 1 } else { t } } as f64;
    let agents: Vec<&Value> = rows
        .iter()
        .flat_map(|r| get(r, "agent_calls").as_array().map(|a| a.iter().collect::<Vec<_>>()).unwrap_or_default())
        .collect();
    let (fresh, cread) = (g("fresh_in"), g("cache_read"));
    let sub_calls: i64 = rows.iter().map(|r| as_i(get(get(r, "sub"), "calls"))).sum();
    let ask: i64 = rows.iter().map(|r| as_i(get(get(r, "tools"), "AskUserQuestion"))).sum();
    let non0 = |x: usize| if x == 0 { 1.0 } else { x as f64 };
    let non0i = |x: i64| if x == 0 { 1.0 } else { x as f64 };
    let r1 = |x: f64| Num::F(pyround(x, 1)).show();
    let r0 = |x: f64| pyround0(x).to_string();
    let with_effort = agents.iter().filter(|c| truthy(get(c, "effort"))).count();
    let typed = agents.iter().filter(|c| truthy(get(c, "type"))).count();
    let gp = agents.iter().filter(|c| get(c, "type").as_str() == Some("general-purpose")).count();
    let vs: Vec<String> = rows
        .iter()
        .filter_map(|r| get(r, "cli").as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()))
        .collect();
    let versions = if vs.is_empty() {
        "?".to_string()
    } else {
        format!("{}..{}", vs.iter().min().unwrap(), vs.iter().max().unwrap())
    };
    let first_ctx = nums(rows, |r| get(r, "first_ctx"));
    let peak_ctx = nums(rows, |r| get(r, "peak_ctx"));
    let prompts_per: Vec<Num> = rows.iter().map(|r| num_of(get(get(r, "stats"), "prompts")).unwrap_or(Num::I(0))).collect();
    vec![
        ("sessions", rows.len().to_string()),
        ("archived", rows.iter().filter(|r| truthy(get(r, "archived"))).count().to_string()),
        ("start ctx med K", r0(med(&first_ctx).f() / 1000.0)),
        ("peak ctx med K", r0(med(&peak_ctx).f() / 1000.0)),
        ("peak > 500K", rows.iter().filter(|r| as_i(get(r, "peak_ctx")) > 500000).count().to_string()),
        ("compactions", g("compactions").to_string()),
        ("prompts/session med", med(&prompts_per).show()),
        ("questions/session", Num::F(pyround(ask as f64 / n, 2)).show()),
        ("questions/100 prompts", r1((100 * ask) as f64 / prompts)),
        ("stops/100 prompts", r1((100 * g("interrupts")) as f64 / prompts)),
        ("corrections/100 prompts", r1((100 * g("corrections")) as f64 / prompts)),
        ("tool errors/100 calls", r1((100 * g("tool_errors")) as f64 / tools)),
        ("hook denials", g("hook_denials").to_string()),
        ("cache hit %", r0((100 * cread) as f64 / non0i(cread + fresh))),
        ("wall min med", r0(med(&nums(rows, |r| get(r, "wall_min"))).f())),
        ("active min med", r0(med(&nums(rows, |r| get(r, "active_min"))).f())),
        ("subagents/session", r1(rows.iter().map(|r| as_i(get(get(r, "sub"), "n"))).sum::<i64>() as f64 / n)),
        ("subagent share of calls %", r0((100 * sub_calls) as f64 / non0i(sub_calls + g("calls")))),
        ("agent calls w/ effort %", r0((100 * with_effort) as f64 / non0(agents.len()))),
        ("general-purpose spawns %", r0((100 * gp) as f64 / non0(typed))),
        (
            "wait calls (Monitor, Cron, Wakeup)",
            rows.iter().map(|r| POLL_TOOLS.iter().map(|t| as_i(get(get(r, "tools"), t))).sum::<i64>()).sum::<i64>().to_string(),
        ),
        ("from hand-off", g("from_handoff").to_string()),
        ("Claude Code versions", versions),
    ]
}

type Groups<'a> = Vec<(String, Vec<&'a Value>)>;

fn table(groups: &Groups) {
    let keys: Vec<&String> = groups.iter().map(|(k, _)| k).collect();
    let stats: Vec<Vec<(&'static str, String)>> = groups.iter().map(|(_, v)| summarize(v)).collect();
    out!("| metric | {} |", keys.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" | "));
    out!("|---|{}", "---|".repeat(keys.len()));
    if let Some(first) = stats.first() {
        for (i, (m, _)) in first.iter().enumerate() {
            let cells: Vec<&str> = stats.iter().map(|s| s[i].1.as_str()).collect();
            out!("| {} | {} |", m, cells.join(" | "));
        }
    }
}

fn periods<'a>(rows: &'a [Value], cut: &[String]) -> (Vec<Dt>, Groups<'a>) {
    let mut cuts: Vec<Dt> = cut
        .iter()
        .map(|c| parse_ts(c).unwrap_or_else(|| fail(&format!("ValueError: Invalid isoformat string: '{}'", c))))
        .collect();
    cuts.sort_by_key(|d| d.micros);
    let mut groups: Groups = (0..=cuts.len()).map(|i| (format!("P{}", i), vec![])).collect();
    for r in rows {
        let st = parse_ts(get(r, "start").as_str().unwrap_or(""))
            .unwrap_or_else(|| fail("ValueError: Invalid isoformat string in row start"));
        let i = cuts.iter().filter(|c| st.micros >= c.micros).count();
        groups[i].1.push(r);
    }
    (cuts, groups)
}

fn periods_line(cuts: &[Dt]) -> String {
    format!(
        "Periods: P0 < {}\n",
        cuts.iter().enumerate().map(|(i, c)| format!("P{} {}", i + 1, c.isoformat())).collect::<Vec<_>>().join(" <= ")
    )
}

fn cmd_report(a: &ReportArgs) {
    let rows = load(&a.inp);
    let groups: Groups;
    if a.by == "project" {
        let mut g: Groups = vec![];
        for r in &rows {
            let p = key_str(get(r, "project"));
            match g.iter_mut().find(|(k, _)| *k == p) {
                Some((_, v)) => v.push(r),
                None => g.push((p, vec![r])),
            }
        }
        g.sort_by_key(|(_, v)| Reverse(v.len()));
        g.truncate(slice_end(g.len(), a.top));
        groups = g;
    } else {
        let (cuts, g) = periods(&rows, &a.cut);
        out!("{}", periods_line(&cuts));
        groups = g;
    }
    table(&groups);
    out!("\nHigher is better: cache hit %, agent calls w/ effort %, from hand-off. Lower is better for the rest.");
    let small: Vec<&str> = groups.iter().filter(|(_, v)| v.len() < 20).map(|(k, _)| k.as_str()).collect();
    if !small.is_empty() {
        out!("\nEarly (under 20 sessions): {}. Read their numbers as a hint, not a result.", small.join(", "));
    }
}

fn cmd_agents(a: &AgentsArgs) {
    let rows = load(&a.inp);
    let (cuts, groups) = periods(&rows, &a.cut);
    let mut data: Vec<(String, Vec<(String, Vec<Num>)>)> = vec![];
    for (k, rs) in &groups {
        let mut by: Vec<(String, Vec<Num>)> = vec![];
        for r in rs {
            let spawns = get(or_empty(get(r, "sub")), "spawns").as_array().cloned().unwrap_or_default();
            for sp in spawns {
                let Some(pair) = sp.as_array() else { continue };
                if pair.len() < 2 {
                    continue;
                }
                let typ = key_str(&pair[0]);
                let ctx = num_of(&pair[1]).unwrap_or(Num::I(0));
                match by.iter_mut().find(|(t, _)| *t == typ) {
                    Some((_, v)) => v.push(ctx),
                    None => by.push((typ, vec![ctx])),
                }
            }
        }
        data.push((k.clone(), by));
    }
    let find = |by: &Vec<(String, Vec<Num>)>, t: &str| -> Vec<Num> {
        by.iter().find(|(k, _)| k == t).map(|(_, v)| v.clone()).unwrap_or_default()
    };
    let mut types: Vec<String> = vec![];
    for (_, by) in &data {
        for (t, _) in by {
            if !types.contains(t) {
                types.push(t.clone());
            }
        }
    }
    // Python sorts a set here, so ties are in hash order; we break ties by name.
    let count = |t: &String| -> usize { data.iter().map(|(_, by)| find(by, t).len()).sum() };
    types.sort();
    types.sort_by_key(|t| Reverse(count(t)));
    out!("{}", periods_line(&cuts));
    out!(
        "| agent type | {} |",
        data.iter().map(|(k, _)| format!("{} spawns (share) | {} start K", k, k)).collect::<Vec<_>>().join(" | ")
    );
    out!("|---|{}", "---|---|".repeat(data.len()));
    let n = slice_end(types.len(), a.top);
    for t in &types[..n] {
        let mut cells: Vec<String> = vec![];
        for (_, by) in &data {
            let total: usize = by.iter().map(|(_, v)| v.len()).sum();
            let total = if total == 0 { 1 } else { total };
            let xs = find(by, t);
            let share = pyround0(100.0 * xs.len() as f64 / total as f64);
            let k = if xs.is_empty() { "-".to_string() } else { pyround0(med(&xs).f() / 1000.0).to_string() };
            cells.push(format!("{} ({}%) | {}", xs.len(), share, k));
        }
        out!("| {} | {} |", t, cells.join(" | "));
    }
    out!(
        "\nSessions: {}",
        groups.iter().map(|(k, v)| format!("{} {}", k, v.len())).collect::<Vec<_>>().join(", ")
    );
    out!("Start K = median first-turn context of the spawn. Watch: general-purpose share should fall and");
    out!("task-implementer / worker should rise after a dispatch change. Agents with a tools: list start near 15K.");
    let small: Vec<&str> = groups.iter().filter(|(_, v)| v.len() < 20).map(|(k, _)| k.as_str()).collect();
    if !small.is_empty() {
        out!("Early (under 20 sessions): {}. Read their numbers as a hint, not a result.", small.join(", "));
    }
}

fn cmd_errors(a: &ErrorsArgs) {
    let rows: Vec<Value> = load(&a.inp)
        .into_iter()
        .filter(|r| a.project.as_deref().map(|p| get(r, "project").as_str() == Some(p)).unwrap_or(true))
        .collect();
    if a.daily {
        let mut days: Vec<(String, Counter<String>)> = vec![];
        let mut total: Counter<String> = Counter::new();
        for r in &rows {
            let d: String = pychars(get(r, "start").as_str().unwrap_or(""), 10);
            let e = Counter::from_json(get(r, "errors"));
            match days.iter_mut().find(|(k, _)| *k == d) {
                Some((_, c)) => c.update(&e),
                None => days.push((d, e.clone())),
            }
            total.update(&e);
        }
        let top: Vec<String> = total.most_common(Some(a.top)).into_iter().map(|(e, _)| e).collect();
        days.sort_by(|x, y| x.0.cmp(&y.0));
        let dates: Vec<String> = days.iter().map(|(d, _)| d.chars().skip(5).collect()).collect();
        out!("| cluster | {} |", dates.join(" | "));
        out!("|---|{}", "---|".repeat(days.len()));
        for e in top {
            let cells: Vec<String> = days.iter().map(|(_, c)| c.get(&e).to_string()).collect();
            out!("| {} | {} |", pychars(&e, 70).replace('|', "/"), cells.join(" | "));
        }
        return;
    }
    let mut by: Vec<(String, Counter<String>)> = vec![];
    for r in &rows {
        let p = key_str(get(r, "project"));
        let e = Counter::from_json(get(r, "errors"));
        match by.iter_mut().find(|(k, _)| *k == p) {
            Some((_, c)) => c.update(&e),
            None => by.push((p, e)),
        }
    }
    by.sort_by_key(|(_, c)| Reverse(c.total()));
    for (p, c) in &by {
        out!("\n## {} ({} errors)", p, c.total());
        for (e, n) in c.most_common(Some(a.top)) {
            out!("{:>5}  {}", n, e);
        }
    }
}

fn installed_skills() -> HashSet<String> {
    let h = PathBuf::from(home());
    let mut roots = vec![h.join(".claude").join("skills")];
    for d in list_dir(&h) {
        let p = d.join(".claude").join("skills");
        if lexists(&p) {
            roots.push(p);
        }
    }
    for d in list_dir(&h.join("work")) {
        let p = d.join(".claude").join("skills");
        if lexists(&p) {
            roots.push(p);
        }
    }
    let mut installed = HashSet::new();
    for d in roots {
        for s in list_dir(&d) {
            if lexists(&s.join("SKILL.md")) {
                installed.insert(fname(&s));
            }
        }
    }
    installed
}

fn cmd_skills(a: &SimpleArgs) {
    let mut used: Counter<String> = Counter::new();
    for r in load(&a.inp) {
        used.update(&Counter::from_json(get(&r, "skills")));
    }
    let installed = installed_skills();
    out!("Used (Skill tool calls):");
    for (s, n) in used.most_common(None) {
        out!("{:>5}  {}", n, s);
    }
    out!("\nNames above that no longer exist were renamed or removed; check the skills repo history.");
    out!("Plugin skills and skillOverrides are not covered.");
    out!("\nInstalled in ~/.claude/skills, ~/*/.claude/skills or ~/work/*/.claude/skills, never called:");
    let mut left: Vec<&String> = installed.iter().filter(|s| !used.idx.contains_key(*s)).collect();
    left.sort();
    for s in left {
        out!("       {}", s);
    }
}

fn count_lines(p: &Path) -> usize {
    let t = read_text_ignore(p).unwrap_or_default().replace("\r\n", "\n").replace('\r', "\n");
    let n = t.matches('\n').count();
    if !t.is_empty() && !t.ends_with('\n') { n + 1 } else { n }
}

fn glob_ext(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    list_dir(dir).into_iter().filter(|p| fname(p).ends_with(suffix)).collect()
}

fn cmd_inventory(a: &InvArgs) {
    let rows = load(&a.inp);
    let mut count: Counter<String> = Counter::new();
    for r in &rows {
        count.add(key_str(get(r, "project")), 1);
    }
    out!("| project | sessions | CLAUDE.md lines | rules (paths/all) | agents | skills | project hooks |");
    out!("|---|---|---|---|---|---|---|");
    let h = PathBuf::from(home());
    let n = count.most_common(None);
    let n = &n[..slice_end(n.len(), a.top)];
    for (p, c) in n {
        if p == "tmp" || p == "scratch" {
            continue;
        }
        let cands: Vec<PathBuf> =
            if p != "~" { vec![h.join("work").join(p), h.join(p)] } else { vec![h.clone()] };
        let Some(root) = cands.into_iter().find(|c| is_dir(c)) else {
            out!("| {} | {} | (folder gone) | | | | |", p, c);
            continue;
        };
        let md = if p != "~" { root.join("CLAUDE.md") } else { h.join(".claude").join("CLAUDE.md") };
        let lines = if md.is_file() { count_lines(&md) } else { 0 };
        let rules = if p != "~" { glob_ext(&root.join(".claude").join("rules"), ".md") } else { vec![] };
        let scoped = rules
            .iter()
            .filter(|f| {
                let t: String = read_text_ignore(f).unwrap_or_default().chars().take(600).collect();
                PATHS_KEY.is_match(&t)
            })
            .count();
        let agents = if p != "~" { glob_ext(&root.join(".claude").join("agents"), ".md").len() } else { 0 };
        let skills = if p != "~" {
            list_dir(&root.join(".claude").join("skills")).iter().filter(|d| lexists(&d.join("SKILL.md"))).count()
        } else {
            0
        };
        let mut hooks = String::new();
        let sj = root.join(".claude").join("settings.json");
        if p != "~" && sj.is_file() {
            hooks = match fs::read(&sj).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                Some(v) => {
                    let mut ks: Vec<String> = get(&v, "hooks").as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                    ks.sort();
                    ks.join(",")
                }
                None => "(bad json)".to_string(),
            };
        }
        let lines_s = if lines == 0 { "none".to_string() } else { lines.to_string() };
        out!(
            "| {} | {} | {} | {}/{} | {} | {} | {} |",
            p,
            c,
            lines_s,
            scoped,
            rules.len(),
            agents,
            skills,
            if hooks.is_empty() { "-" } else { &hooks }
        );
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "harness_stats", about = DOC, infer_long_args = true)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Collect(CollectArgs),
    Cuts(CutsArgs),
    Report(ReportArgs),
    Errors(ErrorsArgs),
    Skills(SimpleArgs),
    Inventory(InvArgs),
    Agents(AgentsArgs),
}

#[derive(Args, Debug)]
struct CollectArgs {
    #[arg(long, default_value_t = default_since())]
    since: String,
    #[arg(long, default_value_t = default_out())]
    out: String,
    #[arg(long, num_args = 0.., default_value = None)]
    exclude: Vec<String>,
}

#[derive(Args, Debug)]
struct CutsArgs {
    #[arg(long, default_value = "~/dotfiles")]
    repo: String,
    /// OWNER/REPO on GitHub, read with gh api (for an archived older history)
    #[arg(long)]
    gh: Option<String>,
    #[arg(long, default_value = "3 weeks ago")]
    since: String,
    #[arg(long, default_value = "claude")]
    path: String,
}

#[derive(Args, Debug)]
struct ReportArgs {
    #[arg(long = "in", default_value_t = default_out())]
    inp: String,
    #[arg(long, default_value_t = 12, allow_negative_numbers = true)]
    top: i64,
    #[arg(long, num_args = 0..)]
    cut: Vec<String>,
    #[arg(long, default_value = "period", value_parser = ["period", "project"])]
    by: String,
}

#[derive(Args, Debug)]
struct AgentsArgs {
    #[arg(long = "in", default_value_t = default_out())]
    inp: String,
    #[arg(long, default_value_t = 12, allow_negative_numbers = true)]
    top: i64,
    #[arg(long, num_args = 0..)]
    cut: Vec<String>,
}

#[derive(Args, Debug)]
struct ErrorsArgs {
    #[arg(long = "in", default_value_t = default_out())]
    inp: String,
    #[arg(long, default_value_t = 12, allow_negative_numbers = true)]
    top: i64,
    #[arg(long)]
    project: Option<String>,
    /// cluster x day table: when did it start or stop
    #[arg(long)]
    daily: bool,
}

#[derive(Args, Debug)]
struct SimpleArgs {
    #[arg(long = "in", default_value_t = default_out())]
    inp: String,
    #[arg(long, default_value_t = 12, allow_negative_numbers = true)]
    top: i64,
}

#[derive(Args, Debug)]
struct InvArgs {
    #[arg(long = "in", default_value_t = default_out())]
    inp: String,
    #[arg(long, default_value_t = 12, allow_negative_numbers = true)]
    top: i64,
}

fn default_since() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86400) - 14);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn main() {
    let cli = Cli::parse();
    match &cli.cmd {
        Cmd::Collect(a) => cmd_collect(a),
        Cmd::Cuts(a) => cmd_cuts(a),
        Cmd::Report(a) => cmd_report(a),
        Cmd::Errors(a) => cmd_errors(a),
        Cmd::Skills(a) => cmd_skills(a),
        Cmd::Inventory(a) => cmd_inventory(a),
        Cmd::Agents(a) => cmd_agents(a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn float_repr_matches_python() {
        assert_eq!(py_float(5.0), "5.0");
        assert_eq!(py_float(12.3), "12.3");
        assert_eq!(py_float(0.05), "0.05");
        assert_eq!(py_float(1e16), "1e+16");
        assert_eq!(py_float(0.00001), "1e-05");
        assert_eq!(py_float(123456.5), "123456.5");
    }

    #[test]
    fn round_is_bankers_on_exact_ties() {
        assert_eq!(pyround0(2.5), 2);
        assert_eq!(pyround0(3.5), 4);
        assert_eq!(pyround(0.25, 1), 0.2);
        assert_eq!(pyround(2.675, 2), 2.67);
    }

    #[test]
    fn dumps_is_python_default() {
        let v = json!({"a": [1, 2.0, "é\n"], "b": null, "c": "😀"});
        let b = '\\';
        let want = format!(r#"{{"a": [1, 2.0, "{b}u00e9{b}n"], "b": null, "c": "{b}ud83d{b}ude00"}}"#);
        assert_eq!(py_dumps(&v), want);
    }

    #[test]
    fn iso_roundtrip_keeps_offset() {
        let d = parse_ts("2026-10-12T08:13:21.123Z").unwrap();
        assert_eq!(d.isoformat(), "2026-10-12T08:13:21.123000+00:00");
        let d = parse_ts("2026-10-12T10:13:21+02:00").unwrap();
        assert_eq!(d.isoformat(), "2026-10-12T10:13:21+02:00");
        assert_eq!(d.micros, parse_ts("2026-10-12T08:13:21Z").unwrap().micros);
        assert!(parse_ts("nope").is_none());
    }

    #[test]
    fn counter_most_common_is_stable() {
        let mut c: Counter<String> = Counter::new();
        c.add("a".into(), 1);
        c.add("b".into(), 2);
        c.add("c".into(), 2);
        let m: Vec<String> = c.most_common(None).into_iter().map(|x| x.0).collect();
        assert_eq!(m, vec!["b", "c", "a"]);
    }

    #[test]
    fn norm_error_replaces_paths_and_digits() {
        let e = norm_error("Bash", &Value::from("fail /Users/bob/x.py line 12\nnext"));
        assert_eq!(e, "Bash: fail <path> line N next");
    }

    #[test]
    fn median_types() {
        assert_eq!(med(&[Num::I(1), Num::I(3), Num::I(2)]), Num::I(2));
        assert_eq!(med(&[Num::I(4), Num::I(6)]).show(), "5.0");
        assert_eq!(med(&[]).show(), "0");
    }
}
