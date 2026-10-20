//! Weekly usage pace, ported from the old `pace.py`. The work days a week (`WORK_DAYS`) and the
//! default weekly weight (`PLAN`) come from `~/.config/dotfiles/settings`; a calibration from the
//! status line always wins over the plan. With no plan, no calibration and no fresh status line
//! the state is `unknown`.
//!
//! Included by `src/bin/pace.rs` and the guard with `#[path] mod`. The cache files keep the
//! Python formats (compact JSON), so old and new caches read alike.
//! Sources: the statusline cache (`claude-pace.json`) when fresh, else an estimate from the
//! transcripts under `$CLAUDE_CONFIG_DIR/projects`. Caches live in `${XDG_CACHE_HOME:-~/.cache}`.
#![allow(dead_code)] // the guard uses `status` only; the CLI uses the rest

use agent_config::pycompat::{civil_from_days, days_from_civil, days_in_month, json_string as json_str, py_float, truthy};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const WEEK: i64 = 7 * 86400;
const FORMAT: f64 = 2.0; // cache format: older caches are rebuilt or ignored
const SCAN_FORMAT: f64 = 3.0; // scan cache: 3 added the weekly-limit hits per file
const MIN_HIT_COST: f64 = 1000.0; // a limit-hit calibration below this cost comes from a partial scan
/// The weekly weight of the Pro plan, the base for the other plans.
pub const DEFAULT_WEEK_WEIGHT: f64 = 5860.0;
const FRESH_S: i64 = 6 * 3600; // statusline cache is fresh for 6 hours
const RESULT_TTL_S: i64 = 120; // cached estimate is current for 2 minutes
const CALIBRATE_MIN: f64 = 5.0; // needs this much reported use to calibrate
const CRITICAL: f64 = 85.0;
const CHECK_EVERY: u64 = 200; // lines between clock checks

/// The weekly pace: how much is used against how much the week allows so far.
pub struct Status {
    /// "ok" | "ahead" | "critical" | "unknown"
    pub state: String,
    /// Percent of the week used, if known.
    pub used: Option<f64>,
    /// Percent of the week that should be used by today.
    pub budget: f64,
    /// Where `used` came from: "statusline" or "estimate".
    pub source: Option<String>,
    /// When the week resets (epoch seconds), if known.
    pub resets_at: Option<i64>,
}

/// The outcome of a transcript scan.
pub struct RefreshResult {
    /// True if every transcript was scanned within the time budget.
    pub complete: bool,
    /// The summed weight of this week's usage.
    pub weight: f64,
    /// How many transcript files were counted.
    pub files: usize,
}

// --- paths and JSON files --------------------------------------------------------------------

/// Where the caches and the transcripts live, and the two settings that shape the numbers: the plan
/// (`pro`, `max5`, ...; `unknown` when unset) and the work days a week. Tests build one over a temp dir.
#[derive(Clone)]
pub struct Paths {
    /// The cache folder.
    pub cache: PathBuf,
    /// The folder of the transcripts.
    pub projects: PathBuf,
    /// The plan name, lowercase.
    pub plan: String,
    /// Work days a week.
    pub days: i64,
}

fn env_nonempty(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn home() -> PathBuf {
    let from_env = env_nonempty("HOME").or_else(|| if cfg!(windows) { env_nonempty("USERPROFILE") } else { None });
    from_env.map(PathBuf::from).or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("~"))
}

impl Paths {
    /// The paths and settings of this machine.
    pub fn from_env() -> Paths {
        let cache = env_nonempty("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache"));
        let base = env_nonempty("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude"));
        // the settings file is read once per process
        static SETTINGS: OnceLock<(String, i64)> = OnceLock::new();
        let (plan, days) = SETTINGS.get_or_init(|| {
            let h = home();
            let plan = agent_config::setting(&h, "PLAN").trim().to_lowercase();
            (if plan.is_empty() { "unknown".to_string() } else { plan }, work_days(&agent_config::setting(&h, "WORK_DAYS")))
        });
        Paths { cache, projects: base.join("projects"), plan: plan.clone(), days: *days }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.cache.join(name)
    }
}

fn read_json(path: &PathBuf) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Atomic write; errors are swallowed like the Python `_write_json`.
fn write_text(path: &PathBuf, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = PathBuf::from(format!("{}.{}.tmp", path.display(), std::process::id()));
    if fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, path)).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// --- Python-compatible number and JSON text --------------------------------------------------

/// round(x, 1) of Python: the exact decimal value, rounded half to even.
fn round1(x: f64) -> f64 {
    format!("{:.1}", x).parse().unwrap_or(x)
}

/// float(s) of Python for the forms that matter here.
fn py_parse_float(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let cleaned: String;
    let t = if t.contains('_') {
        let b: Vec<char> = t.chars().collect();
        for (i, c) in b.iter().enumerate() {
            if *c == '_' && !(i > 0 && b[i - 1].is_ascii_digit() && b.get(i + 1).is_some_and(|n| n.is_ascii_digit())) {
                return None;
            }
        }
        cleaned = t.replace('_', "");
        cleaned.as_str()
    } else {
        t
    };
    t.parse::<f64>().ok()
}

/// isinstance(v, (int, float)): bools count as numbers in Python.
fn pynum(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// isinstance(v, int)
fn pyint(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}

/// `v == n` for a JSON number (a bool never equals the format numbers 2 and 3).
fn num_eq(v: Option<&Value>, n: f64) -> bool {
    matches!(v, Some(Value::Number(x)) if x.as_f64() == Some(n))
}

/// Python's sum() of floats (3.12+ compensated summation).
fn py_sum(xs: impl Iterator<Item = f64>) -> f64 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for x in xs {
        let t = s + x;
        if s.abs() >= x.abs() {
            c += (s - t) + x;
        } else {
            c += (x - t) + s;
        }
        s = t;
    }
    if c != 0.0 && c.is_finite() { s + c } else { s }
}

// --- time ------------------------------------------------------------------------------------

/// Local (year, month, day) of a unix time.
#[cfg(all(unix, target_pointer_width = "64"))]
fn local_ymd(secs: i64) -> (i64, i64, i64) {
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
        return civil_from_days(secs.div_euclid(86400));
    }
    (tm.tm_year as i64 + 1900, tm.tm_mon as i64 + 1, tm.tm_mday as i64)
}

#[cfg(not(all(unix, target_pointer_width = "64")))]
fn local_ymd(secs: i64) -> (i64, i64, i64) {
    // No portable local-time call without a new crate: other platforms count days in UTC.
    civil_from_days(secs.div_euclid(86400))
}

fn local_day(t: i64) -> i64 {
    let (y, m, d) = local_ymd(t);
    days_from_civil(y, m, d)
}

fn strptime_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^([0-9]{4})-(1[0-2]|0[1-9]|[1-9])-(3[0-1]|[1-2][0-9]|0[1-9]|[1-9]| [1-9])T(2[0-3]|[0-1][0-9]|[0-9]):([0-5][0-9]|[0-9]):(6[0-1]|[0-5][0-9]|[0-9])$",
        )
        .expect("static regex")
    })
}

/// calendar.timegm(time.strptime(s, "%Y-%m-%dT%H:%M:%S")); None when strptime would fail.
fn strptime_utc(s: &str) -> Option<i64> {
    let c = strptime_re().captures(s)?;
    let n = |i: usize| -> Option<i64> { c.get(i)?.as_str().trim().parse().ok() };
    let (y, mo, d, h, mi, se) = (n(1)?, n(2)?, n(3)?, n(4)?, n(5)?, n(6)?);
    if y < 1 || d > days_in_month(y, mo) {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se)
}

fn epoch_f(v: f64) -> Option<i64> {
    if !v.is_finite() {
        return None;
    }
    Some(if v > 1e11 { (v / 1000.0) as i64 } else { v as i64 })
}

/// Seconds since the epoch from a number, a numeric string or an ISO UTC string; else None.
fn epoch(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => match n.as_i64() {
            Some(i) => Some(if i > 100_000_000_000 { (i as f64 / 1000.0) as i64 } else { i }),
            None => epoch_f(n.as_f64()?),
        },
        Value::String(s) => match py_parse_float(s) {
            Some(f) => epoch_f(f),
            None => strptime_utc(&s.chars().take(19).collect::<String>()),
        },
        _ => None,
    }
}

/// `_num`: a finite JSON number, else 0.
fn num(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().filter(|f| f.is_finite()).unwrap_or(0.0),
        _ => 0.0,
    }
}

/// `_pct`: a finite number clamped to 0..100.
fn pct(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => {
            let f = n.as_f64().filter(|f| f.is_finite())?;
            let lo = if f > 0.0 { f } else { 0.0 };
            Some(if lo < 100.0 { lo } else { 100.0 })
        }
        _ => None,
    }
}

// --- week and budget -------------------------------------------------------------------------

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object()?.get(k)
}

/// The seven_day object of the statusline cache, if it has one.
fn c1_seven_day(p: &Paths) -> Option<(Value, Value)> {
    let c1 = read_json(&p.file("claude-pace.json"))?;
    let sd = get(&c1, "seven_day").filter(|v| v.is_object())?.clone();
    Some((c1, sd))
}

/// The known weekly reset in the future (rolled forward by whole weeks), or None.
fn c1_reset(p: &Paths, now: i64) -> Option<i64> {
    let (_, sd) = c1_seven_day(p)?;
    let mut e = epoch(get(&sd, "resets_at"))?;
    if e <= now {
        e += ((now - e).div_euclid(WEEK) + 1) * WEEK;
    }
    Some(e)
}

fn fallback_week_start(now: i64) -> i64 {
    let days = now.div_euclid(86400);
    let weekday = (days + 3).rem_euclid(7); // 1970-01-01 was a Thursday; Monday is 0
    let start = days * 86400 + 6 * 3600 - weekday * 86400;
    if start <= now { start } else { start - WEEK }
}

fn week_start(p: &Paths, now: i64) -> i64 {
    c1_reset(p, now).map_or_else(|| fallback_week_start(now), |e| e - WEEK)
}

/// Work days a week from the `WORK_DAYS` setting: `mon-fri` 5, `mon-sat` 6, `mon-sun` 7, anything else 5.
pub fn work_days(spec: &str) -> i64 {
    match spec.trim().to_lowercase().as_str() {
        "mon-sat" => 6,
        "mon-sun" => 7,
        _ => 5,
    }
}

/// The share of the week that should be used by today, in percent: one `days`-th per work day.
fn budget_for(now: i64, ws: i64, days: i64) -> f64 {
    let days = days.max(1);
    let idx = (local_day(now) - local_day(ws)).max(0);
    (idx + 1).min(days) as f64 / days as f64 * 100.0
}

/// The default weekly weight of a plan (pro and team 5860, max5 five times that, max20 twenty times),
/// or None when the plan is `api`, `unknown` or not a plan: then only a calibration gives a weight.
pub fn plan_weight(plan: &str) -> Option<f64> {
    match plan.trim().to_lowercase().as_str() {
        "pro" | "team" => Some(DEFAULT_WEEK_WEIGHT),
        "max5" => Some(DEFAULT_WEEK_WEIGHT * 5.0),
        "max20" => Some(DEFAULT_WEEK_WEIGHT * 20.0),
        _ => None,
    }
}

// --- calibration -----------------------------------------------------------------------------

fn calib(p: &Paths) -> Option<Value> {
    read_json(&p.file("claude-pace-calib.json"))
}

/// The weekly weight: a valid calibration, else the plan's default, else None (no way to tell).
fn week_weight_in(p: &Paths) -> Option<f64> {
    let c = calib(p);
    let w = c.as_ref().filter(|c| num_eq(get(c, "v"), FORMAT)).and_then(|c| get(c, "week_weight"));
    let calibrated = match w {
        Some(Value::Number(n)) => n.as_f64().filter(|f| f.is_finite() && *f > 0.0),
        _ => None,
    };
    calibrated.or_else(|| plan_weight(&p.plan))
}

/// When the stored calibration was made (epoch), or 0 when there is none.
fn calib_updated(p: &Paths) -> f64 {
    let c = calib(p);
    c.as_ref()
        .filter(|c| num_eq(get(c, "v"), FORMAT))
        .and_then(|c| get(c, "updated"))
        .and_then(pynum)
        .unwrap_or(0.0)
}

/// Store the calibration. `updated` orders them: the newest calibration wins.
pub fn seed_weight(p: &Paths, week_weight: f64, now: Option<i64>, source: &str, week_start: Option<i64>) {
    let now = now.unwrap_or_else(unix_now);
    let ws = week_start.map_or("null".to_string(), |w| w.to_string());
    let text = format!(
        "{{\"v\":2,\"week_weight\":{},\"updated\":{},\"source\":{},\"week_start\":{}}}",
        py_float(week_weight),
        now,
        json_str(source),
        ws
    );
    write_text(&p.file("claude-pace-calib.json"), &text);
}

// --- statusline cache ------------------------------------------------------------------------

/// (seven_day used %, resets_at) when the statusline cache is fresh, else None.
fn fresh_c1(p: &Paths, now: i64) -> Option<(f64, i64)> {
    let (c1, sd) = c1_seven_day(p)?;
    let updated = epoch(get(&c1, "updated"))?;
    let resets = epoch(get(&sd, "resets_at"))?;
    let used = pct(get(&sd, "used_percentage"))?;
    if !(-3600..=FRESH_S).contains(&(now - updated)) || resets <= now {
        return None;
    }
    Some((used, resets))
}

// --- transcript scan -------------------------------------------------------------------------

/// (input, output) price per million tokens for a Claude model, else None (free or foreign).
fn prices(model: &str) -> Option<(f64, f64)> {
    let m = model.to_lowercase();
    if m.contains(":free") {
        return None;
    }
    [("opus", (5.0, 25.0)), ("sonnet", (3.0, 15.0)), ("haiku", (1.0, 5.0)), ("fable", (15.0, 75.0))]
        .iter()
        .find(|(f, _)| m.contains(f))
        .map(|(_, p)| *p)
}

/// Cost units of one usage block: cache writes cost 1.25x (5m) or 2x (1h), reads 0.1x.
fn cost(u: &serde_json::Map<String, Value>, price: (f64, f64)) -> f64 {
    let cc = num(u.get("cache_creation_input_tokens"));
    let (w5, w1) = match u.get("cache_creation") {
        Some(Value::Object(d)) if d.contains_key("ephemeral_5m_input_tokens") || d.contains_key("ephemeral_1h_input_tokens") => {
            (num(d.get("ephemeral_5m_input_tokens")), num(d.get("ephemeral_1h_input_tokens")))
        }
        _ => (0.0, cc),
    };
    let cin = num(u.get("input_tokens")) + 1.25 * w5 + 2.0 * w1 + 0.1 * num(u.get("cache_read_input_tokens"));
    (price.0 * cin + price.1 * num(u.get("output_tokens"))) / 1e6
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    let Some(&first) = needle.first() else { return true };
    let mut i = 0;
    while let Some(off) = hay[i..].iter().position(|&b| b == first) {
        let s = i + off;
        if hay.len() - s < needle.len() {
            return false;
        }
        if &hay[s..s + needle.len()] == needle {
            return true;
        }
        i = s + 1;
    }
    false
}

/// (message key, timestamp, weight) for a counted assistant record, else None.
fn parse_line(line: &[u8], pos: i64, ws: i64) -> Option<(String, i64, f64)> {
    if !find(line, b"\"usage\"") {
        return None;
    }
    let d: Value = serde_json::from_slice(line).ok()?;
    let msg = get(&d, "message")?.as_object()?;
    let usage = msg.get("usage")?.as_object()?;
    let price = prices(msg.get("model")?.as_str()?)?;
    let ts = epoch(get(&d, "timestamp"))?;
    if ts < ws {
        return None;
    }
    let weight = cost(usage, price);
    let key = match msg.get("id") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => format!("L{}", pos),
    };
    Some((key, ts, weight))
}

/// Timestamp of the app's "You've hit your weekly limit" record, else None. The record is an API
/// error message (synthetic assistant) or a system record; a real reply that quotes it does not count.
fn hit_ts(line: &[u8]) -> Option<i64> {
    let d: Value = serde_json::from_slice(line).ok()?;
    let o = d.as_object()?;
    let msg = o.get("message").and_then(Value::as_object);
    let mut texts: Vec<&str> = Vec::new();
    match o.get("type").and_then(Value::as_str) {
        Some("assistant") if msg.is_some() => {
            let m = msg?;
            let api_err = o.get("isApiErrorMessage").is_some_and(truthy);
            if !(api_err || m.get("model").and_then(Value::as_str) == Some("<synthetic>")) {
                return None;
            }
            match m.get("content") {
                Some(Value::String(s)) => texts.push(s),
                Some(Value::Array(a)) => {
                    texts.extend(a.iter().filter_map(|b| b.as_object()?.get("text")?.as_str()));
                }
                _ => {}
            }
        }
        Some("system") => {
            texts.extend(o.get("content").and_then(Value::as_str));
            texts.extend(msg.and_then(|m| m.get("content")).and_then(Value::as_str));
        }
        _ => {}
    }
    if !texts.iter().any(|t| t.to_lowercase().contains("hit your weekly limit")) {
        return None;
    }
    epoch(o.get("timestamp"))
}

/// Insertion-ordered message map, like a Python dict: a repeated key keeps its place.
#[derive(Default)]
struct Msgs {
    items: Vec<(String, i64, f64)>,
    idx: HashMap<String, usize>,
}

impl Msgs {
    fn set(&mut self, k: String, ts: i64, w: f64) {
        match self.idx.get(&k) {
            Some(&i) => self.items[i] = (k, ts, w),
            None => {
                self.idx.insert(k.clone(), self.items.len());
                self.items.push((k, ts, w));
            }
        }
    }

    fn keep_from(&mut self, lo: i64) {
        self.items.retain(|(_, ts, _)| *ts >= lo);
        self.idx = self.items.iter().enumerate().map(|(i, (k, _, _))| (k.clone(), i)).collect();
    }
}

struct Ent {
    size: Option<i64>,
    offset: i64,
    mtime: Value,
    msgs: Msgs,
    hits: Vec<i64>,
}

impl Ent {
    fn new() -> Ent {
        Ent { size: Some(0), offset: 0, mtime: Value::from(0), msgs: Msgs::default(), hits: Vec::new() }
    }

    fn from_json(e: &Value) -> Option<Ent> {
        let o = e.as_object()?;
        let offset = match o.get("offset")? {
            Value::Number(n) => n.as_i64()?,
            _ => return None,
        };
        let mut msgs = Msgs::default();
        for (k, v) in o.get("msgs")?.as_object()? {
            let a = v.as_array()?;
            msgs.set(k.clone(), epoch_int(a.first()?)?, a.get(1).and_then(pynum)?);
        }
        let hits = o.get("hits").and_then(Value::as_array).map(|a| a.iter().filter_map(epoch_int).collect()).unwrap_or_default();
        let size = o.get("size").and_then(|v| v.as_f64()).filter(|f| f.fract() == 0.0).map(|f| f as i64);
        let mtime = o.get("mtime").cloned().unwrap_or(Value::from(0));
        Some(Ent { size, offset, mtime, msgs, hits })
    }

    fn write(&self, out: &mut String) {
        out.push('{');
        if let Some(s) = self.size {
            out.push_str(&format!("\"size\":{},", s));
        }
        out.push_str(&format!("\"offset\":{},\"mtime\":", self.offset));
        match self.mtime.as_f64() {
            Some(f) if self.mtime.is_f64() => out.push_str(&py_float(f)),
            _ => out.push_str(&self.mtime.to_string()),
        }
        out.push_str(",\"msgs\":{");
        for (i, (k, ts, w)) in self.msgs.items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!("{}:[{},{}]", json_str(k), ts, py_float(*w)));
        }
        out.push_str("},\"hits\":[");
        out.push_str(&self.hits.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(","));
        out.push_str("]}");
    }
}

fn epoch_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        _ => None,
    }
}

/// (cost up to the first weekly-limit hit of week wk, hit time) or None. Needs a complete scan.
fn limit_hit_calibration(files: &[(String, Ent)], wk: i64) -> Option<(f64, i64)> {
    let t0 = files.iter().flat_map(|(_, e)| e.hits.iter().copied()).filter(|t| (wk..wk + WEEK).contains(t)).min()?;
    let c = py_sum(
        files.iter().flat_map(|(_, e)| e.msgs.items.iter()).filter(|(_, ts, _)| wk <= *ts && *ts <= t0).map(|(_, _, w)| *w),
    );
    (c > MIN_HIT_COST).then_some((c, t0))
}

/// Jsonl files modified since min_mtime, newest first, and the count of all jsonl files.
fn find_files(root: &PathBuf, min_mtime: f64) -> (Vec<(f64, i64, String)>, usize) {
    let mut found = Vec::new();
    let mut total = 0;
    let walk = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !(e.depth() > 0 && e.file_type().is_dir() && e.file_name() == "tool-results"));
    for ent in walk.flatten() {
        if ent.file_type().is_dir() || !ent.file_name().to_string_lossy().ends_with(".jsonl") {
            continue;
        }
        let Ok(md) = fs::metadata(ent.path()) else { continue };
        if md.is_dir() {
            continue;
        }
        total += 1;
        let mtime = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs() as f64 + d.subsec_nanos() as f64 * 1e-9);
        if mtime >= min_mtime {
            found.push((mtime, md.len() as i64, ent.path().to_string_lossy().into_owned()));
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then_with(|| b.2.cmp(&a.2)));
    (found, total)
}

/// Read the new complete lines of one file into ent. False when the time budget ended.
fn scan_file(path: &str, ent: &mut Ent, ws: i64, deadline: Instant) -> bool {
    let mut pos = ent.offset;
    let mut n = 0u64;
    if let Ok(f) = fs::File::open(path) {
        let mut r = BufReader::new(f);
        if pos >= 0 && r.seek(SeekFrom::Start(pos as u64)).is_ok() {
            let mut line = Vec::new();
            loop {
                line.clear();
                if !matches!(r.read_until(b'\n', &mut line), Ok(k) if k > 0) || line.last() != Some(&b'\n') {
                    break; // end, an error, or a partial last line: wait for the writer
                }
                let got = parse_line(&line, pos, ws);
                if line.len() < 8000
                    && find(&line.to_ascii_lowercase(), b"hit your weekly limit")
                    && let Some(t) = hit_ts(&line).filter(|t| *t >= ws)
                {
                    ent.hits.push(t);
                }
                pos += line.len() as i64;
                if let Some((k, ts, w)) = got {
                    ent.msgs.set(k, ts, w); // last record of a message wins
                }
                n += 1;
                if n.is_multiple_of(CHECK_EVERY) && Instant::now() >= deadline {
                    ent.offset = pos;
                    return false;
                }
            }
        }
    }
    ent.offset = pos;
    true
}

fn sum_week(files: &[(String, Ent)], ws: i64) -> f64 {
    let mut total = 0.0;
    for (_, e) in files {
        for (_, ts, w) in &e.msgs.items {
            if ws <= *ts && *ts < ws + WEEK {
                total += *w;
            }
        }
    }
    total
}

fn valid_files(scan: Option<&Value>) -> HashMap<String, Ent> {
    let mut ok = HashMap::new();
    let Some(s) = scan.filter(|s| num_eq(get(s, "v"), SCAN_FORMAT)) else { return ok };
    if let Some(Value::Object(files)) = get(s, "files") {
        for (p, e) in files {
            if let Some(ent) = Ent::from_json(e) {
                ok.insert(p.clone(), ent);
            }
        }
    }
    ok
}

/// The cached estimate when it is a well-formed result record.
fn load_result(p: &Paths) -> Option<Value> {
    let r = read_json(&p.file("claude-pace-result.json"))?;
    if !num_eq(get(&r, "v"), FORMAT) || get(&r, "weight").and_then(pynum).is_none() {
        return None;
    }
    if get(&r, "at").and_then(pynum).is_none() || !get(&r, "ws").is_some_and(|v| pyint(v).is_some()) {
        return None;
    }
    Some(r)
}

/// Scan new transcript bytes for at most budget_s seconds (one chunk minimum per call).
pub fn refresh_in(p: &Paths, budget_s: f64, now: Option<i64>) -> RefreshResult {
    let now = now.unwrap_or_else(unix_now);
    let wait = if budget_s.is_finite() { budget_s.clamp(0.0, 1e7) } else { 0.0 };
    let deadline = Instant::now() + Duration::from_secs_f64(wait);
    let ws = week_start(p, now);
    let scan = read_json(&p.file("claude-pace-scan.json"));
    let mut old = valid_files(scan.as_ref());
    let had_valid = !old.is_empty();
    let meta = scan.as_ref().filter(|s| num_eq(get(s, "v"), SCAN_FORMAT));
    let mut backfilled = meta.and_then(|m| get(m, "backfilled")).is_some_and(truthy);
    // A cold calibration looks at the previous week too, once, to find its limit hit.
    let back = calib_updated(p) == 0.0 && !backfilled;
    let lo = if back { ws - WEEK } else { ws };
    let meta_lo = meta.and_then(|m| get(m, "lo")).and_then(|v| v.as_i64());
    let mut dirty = false;
    if meta_lo.is_none_or(|l| l > lo) {
        old.clear(); // the cache covers less than this scan needs: rebuild
        dirty = had_valid;
    }
    let old_len = old.len();
    let mut files: Vec<(String, Ent)> = Vec::new();
    let mut complete = true;
    let mut started = false;
    let (found, total) = find_files(&p.projects, (lo - 86400) as f64);
    for (mtime, size, path) in found {
        let mut ent = match old.remove(&path) {
            Some(e) if size >= e.offset => e,
            _ => {
                dirty = true;
                Ent::new()
            }
        };
        if size == ent.offset && ent.size == Some(size) {
            files.push((path, ent));
            continue;
        }
        if started && Instant::now() >= deadline {
            complete = false;
            files.push((path, ent));
            continue;
        }
        started = true;
        if !scan_file(&path, &mut ent, lo, deadline) {
            complete = false;
        }
        ent.size = Some(size);
        ent.mtime = serde_json::Number::from_f64(mtime).map_or(Value::from(0), Value::Number);
        dirty = true;
        files.push((path, ent));
    }
    if files.len() != old_len {
        dirty = true;
    }
    if complete {
        if back {
            backfilled = true;
            dirty = true;
        }
        // Weeks to look at: the previous one only on the backfill pass, then this one (newer wins).
        let weeks = if back { vec![ws - WEEK, ws] } else { vec![ws] };
        for wk in weeks {
            if let Some((c, t0)) = limit_hit_calibration(&files, wk)
                && t0 as f64 > calib_updated(p)
            {
                seed_weight(p, c, Some(t0), "limit-hit", Some(wk));
            }
        }
    }
    if dirty || meta_lo != Some(lo) {
        let mut out = format!("{{\"v\":3,\"lo\":{},\"backfilled\":{},\"files\":{{", lo, backfilled);
        for (i, (path, ent)) in files.iter_mut().enumerate() {
            ent.msgs.keep_from(lo);
            ent.hits.retain(|t| *t >= lo);
            if i > 0 {
                out.push(',');
            }
            out.push_str(&json_str(path));
            out.push(':');
            ent.write(&mut out);
        }
        out.push_str("}}");
        write_text(&p.file("claude-pace-scan.json"), &out);
    }
    let weight = sum_week(&files, ws);
    let res = format!(
        "{{\"v\":2,\"at\":{},\"ws\":{},\"weight\":{},\"files\":{},\"complete\":{}}}",
        now,
        ws,
        py_float(weight),
        total,
        complete
    );
    write_text(&p.file("claude-pace-result.json"), &res);
    if complete
        && weight > 0.0
        && let Some((used, _)) = fresh_c1(p, now).filter(|c| c.0 >= CALIBRATE_MIN)
    {
        seed_weight(p, weight / (used / 100.0), Some(now), "statusline", Some(ws));
    }
    RefreshResult { complete, weight, files: total }
}

/// `refresh_in` over this machine's paths.
pub fn refresh(budget_s: f64, now: Option<i64>) -> RefreshResult {
    refresh_in(&Paths::from_env(), budget_s, now)
}

/// Calibrate the weekly weight from a percent the app reports. True when stored.
pub fn seed_in(p: &Paths, used: f64, now: Option<i64>) -> bool {
    if !(used > 0.0 && used <= 100.0) {
        return false;
    }
    let now = now.unwrap_or_else(unix_now);
    let res = refresh_in(p, 60.0, Some(now));
    if !res.complete || res.weight <= 0.0 {
        return false; // a partial scan would store a too-small weight
    }
    seed_weight(p, res.weight / (used / 100.0), Some(now), "seed", None);
    true
}

/// `seed_in` over this machine's paths.
pub fn seed(used: f64, now: Option<i64>) -> bool {
    seed_in(&Paths::from_env(), used, now)
}

// --- status ----------------------------------------------------------------------------------

fn state_of(used: Option<f64>, budget: f64) -> &'static str {
    match used {
        None => "unknown",
        Some(u) if u >= CRITICAL => "critical",
        Some(u) if u > budget => "ahead",
        Some(_) => "ok",
    }
}

fn out(state: &str, used: Option<f64>, budget: f64, source: Option<&str>, resets_at: Option<i64>) -> Status {
    Status {
        state: state.to_string(),
        used: used.map(round1),
        budget: round1(budget),
        source: source.map(str::to_string),
        resets_at,
    }
}

fn status_inner(p: &Paths, now: i64) -> Result<Status, ()> {
    let ws = week_start(p, now);
    let budget = budget_for(now, ws, p.days);
    if let Some((used, resets)) = fresh_c1(p, now) {
        return Ok(out(state_of(Some(used), budget), Some(used), budget, Some("statusline"), Some(resets)));
    }
    if let Some(r) = load_result(p)
        && get(&r, "ws").and_then(pyint) == Some(ws)
    {
        let positive = match get(&r, "files") {
            None => false,
            Some(v) => pynum(v).ok_or(())? > 0.0,
        };
        if positive {
            let weight = get(&r, "weight").and_then(pynum).ok_or(())?;
            // no calibration and no plan weight (api, unknown): nothing to divide by, so no estimate
            if let Some(total) = week_weight_in(p) {
                let raw = weight / total * 100.0;
                let used = if raw < 100.0 { raw } else { 100.0 };
                return Ok(out(state_of(Some(used), budget), Some(used), budget, Some("estimate"), c1_reset(p, now)));
            }
        }
    }
    Ok(out("unknown", None, budget, None, None))
}

/// Weekly pace from the caches only. Never panics; any error gives state "unknown".
pub fn status_in(p: &Paths, now: Option<i64>) -> Status {
    let now = now.unwrap_or_else(unix_now);
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| status_inner(p, now)));
    if let Ok(Ok(s)) = attempt {
        return s;
    }
    let budget = std::panic::catch_unwind(|| budget_for(now, fallback_week_start(now), p.days)).unwrap_or(0.0);
    out("unknown", None, budget, None, None)
}

/// `status_in` over this machine's paths.
pub fn status(now: Option<i64>) -> Status {
    match std::panic::catch_unwind(Paths::from_env) {
        Ok(p) => status_in(&p, now),
        Err(_) => out("unknown", None, 0.0, None, None),
    }
}

/// The weekly weight on this machine, or None when there is no calibration and no plan.
pub fn week_weight() -> Option<f64> {
    week_weight_in(&Paths::from_env())
}

/// True if a scan result is older than RESULT_TTL_S or from another week.
fn result_stale(p: &Paths, r: &Value, now: i64) -> bool {
    let at = get(r, "at").and_then(pynum).unwrap_or(0.0);
    (now as f64 - at).abs() >= RESULT_TTL_S as f64 || get(r, "ws").and_then(pyint) != Some(week_start(p, now))
}

/// True if the cached estimate is stale and a scan should run.
pub fn needs_refresh_in(p: &Paths, now: i64) -> bool {
    fresh_c1(p, now).is_none() && load_result(p).is_none_or(|r| result_stale(p, &r, now))
}

/// `needs_refresh_in` over this machine's paths.
pub fn needs_refresh(now: i64) -> bool {
    needs_refresh_in(&Paths::from_env(), now)
}

/// True if the prompt hook should scan: the last scan is missing, partial or stale. Unlike
/// `needs_refresh_in` it ignores a fresh status line, because the scan also saves the
/// status line calibration and the limit-hit one.
pub fn prompt_needs_refresh_in(p: &Paths, now: i64) -> bool {
    load_result(p).is_none_or(|r| get(&r, "complete").and_then(Value::as_bool) != Some(true) || result_stale(p, &r, now))
}

/// `prompt_needs_refresh_in` over this machine's paths.
pub fn prompt_needs_refresh(now: i64) -> bool {
    prompt_needs_refresh_in(&Paths::from_env(), now)
}

// --- CLI text (json.dumps defaults) ----------------------------------------------------------

/// The status as the JSON text the CLI prints.
pub fn status_json(s: &Status) -> String {
    let used = s.used.map_or("null".to_string(), py_float);
    let source = s.source.as_deref().map_or("null".to_string(), json_str);
    let resets = s.resets_at.map_or("null".to_string(), |r| r.to_string());
    format!(
        "{{\"state\": {}, \"used\": {}, \"budget\": {}, \"source\": {}, \"resets_at\": {}}}",
        json_str(&s.state),
        used,
        py_float(s.budget),
        source,
        resets
    )
}

/// The scan result as the JSON text the CLI prints.
pub fn refresh_json(r: &RefreshResult) -> String {
    format!("{{\"complete\": {}, \"weight\": {}, \"files\": {}}}", r.complete, py_float(r.weight), r.files)
}

/// The new weekly weight as the JSON text the CLI prints.
pub fn seed_json(week_weight: f64) -> String {
    format!("{{\"week_weight\": {}}}", py_float(week_weight))
}

/// `float(arg)` of the CLI: None when Python would raise ValueError.
pub fn parse_percent(arg: &str) -> Option<f64> {
    py_parse_float(arg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DAY: i64 = 86400;
    fn mon() -> i64 {
        days_from_civil(2026, 10, 19) * 86400 + 6 * 3600
    }

    fn iso(t: i64) -> String {
        let (y, m, d) = civil_from_days(t.div_euclid(86400));
        let s = t.rem_euclid(86400);
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z", y, m, d, s / 3600, s % 3600 / 60, s % 60)
    }

    fn record(t: i64, id: &str, model: &str, inp: i64, out: i64, cc: i64, cr: i64) -> String {
        json!({"type": "assistant", "timestamp": iso(t), "message": {"id": id, "model": model, "usage": {
            "input_tokens": inp, "output_tokens": out, "cache_creation_input_tokens": cc, "cache_read_input_tokens": cr}}})
        .to_string()
    }

    fn sonnet(t: i64, id: &str) -> String {
        record(t, id, "claude-sonnet-5-5", 1000, 500, 2000, 10000)
    }

    const WEIGHT: f64 = 0.0255; // sonnet: 3 x (1000 + 2 x 2000 + 0.1 x 10000) + 15 x 500, per million

    struct Env {
        _dir: tempfile::TempDir,
        p: Paths,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths {
            cache: dir.path().join("cache"),
            projects: dir.path().join("projects"),
            plan: "pro".into(),
            days: 6,
        };
        fs::create_dir_all(p.projects.join("proj")).unwrap();
        Env { _dir: dir, p }
    }

    impl Env {
        fn write(&self, name: &str, lines: &[String]) {
            let path = self.p.projects.join("proj").join(name);
            let mut text = lines.join("\n");
            text.push('\n');
            fs::write(&path, text).unwrap();
            let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs((mon() + DAY) as u64)).unwrap();
        }

        fn c1(&self, updated: i64, used: f64, resets: i64) {
            let v = json!({"updated": updated, "seven_day": {"used_percentage": used, "resets_at": resets}});
            write_text(&self.p.file("claude-pace.json"), &v.to_string());
        }

        fn weight(&self) -> f64 {
            get(&load_result(&self.p).unwrap(), "weight").and_then(pynum).unwrap()
        }
    }

    #[test]
    fn python_float_text_and_rounding() {
        assert_eq!(py_float(40.0), "40.0");
        assert_eq!(py_float(12.3), "12.3");
        assert_eq!(py_float(0.0255), "0.0255");
        assert_eq!(py_float(1e16), "1e+16");
        assert_eq!(py_float(1.5e-7), "1.5e-07");
        assert_eq!(round1(0.25), 0.2);
        assert_eq!(round1(41.23456), 41.2);
        assert_eq!(round1(16.666666666666664), 16.7);
        assert_eq!(json_str("a\u{e9}\"b"), "\"a\\u00e9\\\"b\"");
    }

    #[test]
    fn epoch_forms() {
        assert_eq!(epoch(Some(&json!(1760860800))), Some(1760860800));
        assert_eq!(epoch(Some(&json!(1760860800000i64))), Some(1760860800));
        assert_eq!(epoch(Some(&json!("1760860800"))), Some(1760860800));
        assert_eq!(epoch(Some(&json!("2025-10-19T08:00:00Z"))), Some(1760860800));
        assert_eq!(epoch(Some(&json!("2025-10-19T08:00:00.123Z"))), Some(1760860800));
        assert_eq!(epoch(Some(&json!("2025-02-30T08:00:00Z"))), None);
        assert_eq!(epoch(Some(&json!(null))), None);
        assert_eq!(epoch(Some(&json!("junk"))), None);
        assert_eq!(epoch(Some(&json!(true))), None);
    }

    #[test]
    fn fallback_week_starts_monday_six_utc() {
        assert_eq!(fallback_week_start(mon() + 60), mon());
        assert_eq!(fallback_week_start(mon() - 60), mon() - WEEK);
        assert_eq!(fallback_week_start(mon() + 6 * DAY + 3600), mon());
    }

    #[test]
    fn fresh_statusline_wins_and_states() {
        let e = env();
        let now = mon() + 2 * DAY;
        for (used, want) in [(10.0, "ok"), (84.0, "ahead"), (85.0, "critical"), (100.0, "critical")] {
            e.c1(now - 10, used, mon() + WEEK);
            let st = status_in(&e.p, Some(now));
            assert_eq!(st.source.as_deref(), Some("statusline"));
            assert_eq!(st.resets_at, Some(mon() + WEEK));
            assert_eq!(st.state, want, "used {used}");
        }
        e.c1(now - 7 * 3600, 41.2, mon() + WEEK); // stale
        assert_eq!(status_in(&e.p, Some(now)).state, "unknown");
    }

    #[test]
    fn unknown_without_data_and_json_text() {
        let e = env();
        let st = status_in(&e.p, Some(mon() + 3600));
        assert_eq!((st.state.as_str(), st.used, st.source.clone()), ("unknown", None, None));
        let text = status_json(&st);
        assert!(text.starts_with("{\"state\": \"unknown\", \"used\": null, \"budget\": "), "{text}");
        assert!(text.ends_with(", \"source\": null, \"resets_at\": null}"), "{text}");
    }

    #[test]
    fn estimate_from_transcripts_and_dedupe() {
        let e = env();
        e.write("a.jsonl", &[sonnet(mon() + 60, "m1"), sonnet(mon() + 61, "m1"), sonnet(mon() - 60, "old")]);
        let now = mon() + DAY;
        seed_weight(&e.p, WEIGHT * 100.0, Some(now), "seed", None);
        let r = refresh_in(&e.p, 5.0, Some(now));
        assert!(r.complete);
        assert_eq!(r.files, 1);
        assert!((e.weight() - WEIGHT).abs() < 1e-12, "{}", e.weight());
        let st = status_in(&e.p, Some(now));
        assert_eq!(st.source.as_deref(), Some("estimate"));
        assert_eq!(st.used, Some(1.0));
    }

    #[test]
    fn model_families_and_cache_write_split() {
        let e = env();
        let recs = [
            record(mon() + 60, "a", "claude-opus-4-8", 1000, 500, 2000, 10000),
            record(mon() + 61, "b", "openai/gpt-oss-120b:free", 1000, 500, 2000, 10000),
            record(mon() + 62, "c", "<synthetic>", 1000, 500, 2000, 10000),
        ];
        e.write("a.jsonl", &recs);
        refresh_in(&e.p, 5.0, Some(mon() + DAY));
        assert!((e.weight() - (5.0 * 6000.0 + 25.0 * 500.0) / 1e6).abs() < 1e-12);
        let split = json!({"type": "assistant", "timestamp": iso(mon() + 60), "message": {"id": "s", "model": "claude-opus-4-8",
            "usage": {"cache_creation_input_tokens": 3000, "cache_creation": {"ephemeral_5m_input_tokens": 1000, "ephemeral_1h_input_tokens": 2000}}}});
        e.write("a.jsonl", &[split.to_string()]);
        fs::remove_file(e.p.file("claude-pace-scan.json")).unwrap();
        refresh_in(&e.p, 5.0, Some(mon() + DAY));
        assert!((e.weight() - 5.0 * (1.25 * 1000.0 + 2.0 * 2000.0) / 1e6).abs() < 1e-12);
    }

    #[test]
    fn partial_last_line_waits_for_the_writer() {
        let e = env();
        e.write("a.jsonl", &[sonnet(mon() + 60, "m1")]);
        let path = e.p.projects.join("proj").join("a.jsonl");
        let next = sonnet(mon() + 90, "m2");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(&next[..40]);
        fs::write(&path, &text).unwrap();
        let now = mon() + DAY;
        refresh_in(&e.p, 5.0, Some(now));
        assert!((e.weight() - WEIGHT).abs() < 1e-12);
        text.push_str(&next[40..]);
        text.push('\n');
        fs::write(&path, text).unwrap();
        refresh_in(&e.p, 5.0, Some(now));
        assert!((e.weight() - 2.0 * WEIGHT).abs() < 1e-12);
    }

    #[test]
    fn time_budget_stops_and_next_call_continues() {
        let e = env();
        let recs: Vec<String> = (0..1500).map(|i| sonnet(mon() + i, &format!("m{i}"))).collect();
        e.write("a.jsonl", &recs);
        let now = mon() + DAY;
        let r = refresh_in(&e.p, 0.0, Some(now));
        assert!(!r.complete);
        assert!(e.weight() < 1500.0 * WEIGHT);
        let mut done = false;
        for _ in 0..20 {
            done = refresh_in(&e.p, 0.0, Some(now)).complete;
            if done {
                break;
            }
        }
        assert!(done);
        assert!((e.weight() - 1500.0 * WEIGHT).abs() < 1e-9);
    }

    #[test]
    fn limit_hit_sets_calibration() {
        let e = env();
        let big = |t, id: &str| record(t, id, "claude-opus-4-8", 400_000_000, 0, 0, 0); // 2000 cost units
        let t0 = mon() + 2 * DAY;
        let hit = json!({"type": "assistant", "timestamp": iso(t0), "isApiErrorMessage": true, "message": {"model": "<synthetic>",
            "content": [{"type": "text", "text": "You've hit your weekly limit"}]}});
        e.write("a.jsonl", &[big(mon() + 3600, "b1"), hit.to_string(), big(t0 + 60, "after")]);
        refresh_in(&e.p, 5.0, Some(mon() + 3 * DAY));
        let c = calib(&e.p).unwrap();
        assert_eq!(get(&c, "source").and_then(Value::as_str), Some("limit-hit"));
        assert_eq!(get(&c, "week_weight").and_then(Value::as_f64), Some(2000.0));
        assert_eq!(get(&c, "updated").and_then(Value::as_i64), Some(t0));
        assert_eq!(week_weight_in(&e.p), Some(2000.0));
    }

    #[test]
    fn statusline_and_seed_calibrate_and_bad_percents_fail() {
        let e = env();
        let now = mon() + DAY;
        let recs: Vec<String> = (0..10).map(|i| sonnet(mon() + 60, &format!("m{i}"))).collect();
        e.write("a.jsonl", &recs);
        for bad in [0.0, -3.0, 101.0, f64::NAN] {
            assert!(!seed_in(&e.p, bad, Some(now)));
        }
        assert!(!e.p.file("claude-pace-calib.json").exists());
        assert!(seed_in(&e.p, 25.0, Some(now)));
        assert!((week_weight_in(&e.p).unwrap() - 10.0 * WEIGHT / 0.25).abs() < 1e-9);
        assert!((status_in(&e.p, Some(now)).used.unwrap() - 25.0).abs() < 0.05);
        e.c1(now - 10, 10.0, mon() + WEEK);
        refresh_in(&e.p, 5.0, Some(now));
        assert!((week_weight_in(&e.p).unwrap() - 10.0 * WEIGHT / 0.10).abs() < 1e-9);
    }

    #[test]
    fn prompt_scans_past_a_fresh_statusline_and_calibrates() {
        let e = env();
        let now = mon() + DAY;
        let recs: Vec<String> = (0..10).map(|i| sonnet(mon() + 60, &format!("m{i}"))).collect();
        e.write("a.jsonl", &recs);
        e.c1(now - 10, 10.0, mon() + WEEK);
        // no scan yet: `pace status` trusts the fresh status line, the prompt scans
        assert!(!needs_refresh_in(&e.p, now));
        assert!(prompt_needs_refresh_in(&e.p, now));
        refresh_in(&e.p, 5.0, Some(now));
        assert!((week_weight_in(&e.p).unwrap() - 10.0 * WEIGHT / 0.10).abs() < 1e-9);
        // a complete scan holds for RESULT_TTL_S, then the prompt scans again
        assert!(!prompt_needs_refresh_in(&e.p, now + RESULT_TTL_S - 1));
        assert!(prompt_needs_refresh_in(&e.p, now + RESULT_TTL_S));
        // a partial scan is continued on the next prompt
        write_text(&e.p.file("claude-pace-result.json"), &json!({"at": now, "ws": mon(), "weight": 1.0, "files": 1, "complete": false}).to_string());
        assert!(prompt_needs_refresh_in(&e.p, now));
        assert!(!needs_refresh_in(&e.p, now));
    }

    #[test]
    fn old_cache_formats_are_ignored() {
        let e = env();
        e.write("a.jsonl", &[sonnet(mon() + 60, "m1")]);
        write_text(&e.p.file("claude-pace-scan.json"), r#"{"v":1,"ws":1,"files":{}}"#);
        write_text(&e.p.file("claude-pace-result.json"), r#"{"at":1,"ws":1,"weight":1e9,"files":5,"complete":true}"#);
        write_text(&e.p.file("claude-pace-calib.json"), r#"{"week_weight":1.3e9,"updated":1}"#);
        let now = mon() + DAY;
        assert_eq!(status_in(&e.p, Some(now)).state, "unknown");
        assert_eq!(week_weight_in(&e.p), Some(DEFAULT_WEEK_WEIGHT));
        refresh_in(&e.p, 5.0, Some(now));
        assert!((e.weight() - WEIGHT).abs() < 1e-12);
    }

    // --- port of claude/hooks/test_pace.py (everything but its CLI class, which is tests/pace.rs) ---

    const WEEK_S: i64 = 7 * DAY;

    fn utc(y: i64, m: i64, d: i64, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60
    }

    /// Python's `assertAlmostEqual(a, b, places=n)`: round(a - b, n) == 0.
    fn close(a: f64, b: f64, places: i32) -> bool {
        (a - b).abs() < 0.5 * 10f64.powi(-places)
    }

    fn rv(t: i64, id: &str, model: &str, inp: i64, out: i64, cc: i64, cr: i64) -> Value {
        json!({"type": "assistant", "timestamp": iso(t), "message": {"id": id, "model": model, "role": "assistant",
            "usage": {"input_tokens": inp, "output_tokens": out, "cache_creation_input_tokens": cc,
                "cache_read_input_tokens": cr}}})
    }

    fn rec(t: i64, id: &str) -> Value {
        rv(t, id, "claude-sonnet-5-5", 1000, 500, 2000, 10000)
    }

    /// The app's limit message: a synthetic assistant record flagged as an API error.
    fn hit(t: i64) -> Value {
        json!({"type": "assistant", "timestamp": iso(t), "isApiErrorMessage": true, "message": {
            "id": format!("hit{t}"), "model": "<synthetic>", "role": "assistant",
            "content": [{"type": "text", "text": "You've hit your weekly limit \u{b7} resets Oct 19 at 8am (Europe/Berlin)"}],
            "usage": {"input_tokens": 0, "output_tokens": 0}}})
    }

    /// One opus record worth `tokens` x 5 / 1e6 cost units (2000 by default).
    fn big(t: i64, id: &str, tokens: i64) -> Value {
        rv(t, id, "claude-opus-4-8", tokens, 0, 0, 0)
    }

    fn big2000(t: i64, id: &str) -> Value {
        big(t, id, 400_000_000)
    }

    struct Pc {
        _dir: tempfile::TempDir,
        p: Paths,
    }

    fn pc() -> Pc {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths {
            cache: dir.path().join("cache"),
            projects: dir.path().join(".claude/projects"),
            plan: "pro".into(),
            days: 6,
        };
        fs::create_dir_all(&p.cache).unwrap();
        fs::create_dir_all(p.projects.join("proj")).unwrap();
        Pc { _dir: dir, p }
    }

    impl Pc {
        fn write_in<T: ToString>(&self, name: &str, sub: Option<&str>, recs: &[T], mtime: i64) -> PathBuf {
            let mut d = self.p.projects.join("proj");
            if let Some(s) = sub {
                d = d.join(s).join("subagents");
            }
            fs::create_dir_all(&d).unwrap();
            let path = d.join(name);
            let mut text = String::new();
            for r in recs {
                text.push_str(&r.to_string());
                text.push('\n');
            }
            fs::write(&path, text).unwrap();
            let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs(mtime as u64)).unwrap(); // the real clock must not matter
            path
        }

        fn lines<T: ToString>(&self, name: &str, recs: &[T]) -> PathBuf {
            self.write_in(name, None, recs, mon() + DAY)
        }

        fn c1(&self, updated: i64, used: f64, resets_at: i64) {
            let v = json!({"updated": updated,
                "five_hour": {"used_percentage": 1.0, "resets_at": resets_at - 5 * DAY},
                "seven_day": {"used_percentage": used, "resets_at": resets_at}});
            write_text(&self.p.file("claude-pace.json"), &v.to_string());
        }

        fn calib_json(&self) -> Value {
            read_json(&self.p.file("claude-pace-calib.json")).expect("a calibration file")
        }

        fn has_calib(&self) -> bool {
            self.p.file("claude-pace-calib.json").exists()
        }

        fn weight(&self) -> f64 {
            get(&load_result(&self.p).expect("a result"), "weight").and_then(pynum).unwrap()
        }

        fn refresh(&self, budget: f64, now: i64) -> RefreshResult {
            refresh_in(&self.p, budget, Some(now))
        }

        fn scan(&self) -> Value {
            read_json(&self.p.file("claude-pace-scan.json")).expect("a scan file")
        }

        /// The Python `estimate`: optional seeded week weight, one refresh, the status.
        fn estimate(&self, now: i64, week_weight: Option<f64>) -> Status {
            if let Some(w) = week_weight {
                seed_weight(&self.p, w, None, "seed", None);
            }
            self.refresh(5.0, now);
            status_in(&self.p, Some(now))
        }

        fn budget_at(&self, now: i64) -> Status {
            self.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
            self.refresh(5.0, now);
            status_in(&self.p, Some(now))
        }

        fn cost_of(&self, model: &str, inp: i64, out: i64, cc: i64, cr: i64) -> f64 {
            self.lines("a.jsonl", &[rv(mon() + 60, "m1", model, inp, out, cc, cr)]);
            self.refresh(5.0, mon() + DAY);
            let weight = self.weight();
            fs::remove_file(self.p.projects.join("proj/a.jsonl")).unwrap(); // transcripts only grow: start clean per call
            self.refresh(5.0, mon() + DAY);
            weight
        }
    }

    // TestBudget
    #[test]
    fn test_budget_per_weekday() {
        for (i, want) in [16.7, 33.3, 50.0, 66.7, 83.3, 100.0, 100.0].into_iter().enumerate() {
            let c = pc();
            let st = c.budget_at(mon() + i as i64 * DAY + 6 * 3600);
            assert!(close(st.budget, want, 1), "day {i}: {} vs {want}", st.budget);
        }
    }

    #[test]
    fn test_sunday_stays_at_100() {
        let c = pc();
        assert_eq!(c.budget_at(utc(2026, 10, 25, 23, 59)).budget, 100.0);
    }

    #[test]
    fn test_week_rollover() {
        // Monday after 06:00 UTC starts a new week: old messages no longer count.
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        let now = mon() + WEEK_S + 3600;
        c.refresh(5.0, now);
        let st = status_in(&c.p, Some(now));
        assert!(close(st.budget, 16.7, 1), "{}", st.budget);
        assert_eq!(st.used, Some(0.0));
        assert_eq!(st.state, "ok");
    }

    #[test]
    fn test_monday_before_six_belongs_to_previous_week() {
        let c = pc();
        let now = mon() + WEEK_S - 3 * 3600; // next Monday 03:00 UTC
        assert_eq!(c.budget_at(now).budget, 100.0);
    }

    #[test]
    fn test_week_start_from_resets_at() {
        // Reset on Thursday 12:00: Thursday is day 0 of this week.
        let c = pc();
        let thu = utc(2026, 10, 22, 12, 0);
        let now = thu + 2 * DAY + 3600; // Saturday
        c.c1(now - 60, 10.0, thu + WEEK_S);
        let st = status_in(&c.p, Some(now));
        assert_eq!(st.source.as_deref(), Some("statusline"));
        assert!(close(st.budget, 50.0, 1), "{}", st.budget); // day index 2 -> 3/6
        assert_eq!(st.resets_at, Some(thu + WEEK_S));
    }

    // TestSources
    #[test]
    fn test_unknown_without_data() {
        let c = pc();
        let st = status_in(&c.p, Some(mon() + 3600));
        assert_eq!(st.state, "unknown");
        assert_eq!(st.used, None);
        assert_eq!(st.source, None);
    }

    #[test]
    fn test_fresh_c1_wins() {
        let c = pc();
        let now = mon() + 2 * DAY;
        c.c1(now - 60, 41.2, mon() + WEEK_S);
        let st = status_in(&c.p, Some(now));
        assert_eq!(st.source.as_deref(), Some("statusline"));
        assert_eq!(st.used, Some(41.2));
        assert_eq!(st.state, "ok"); // budget 50
        assert_eq!(st.resets_at, Some(mon() + WEEK_S));
    }

    #[test]
    fn test_stale_c1_falls_back_to_estimate() {
        let c = pc();
        let now = mon() + 2 * DAY;
        c.c1(now - 7 * 3600, 41.2, mon() + WEEK_S);
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        c.refresh(5.0, now);
        let st = status_in(&c.p, Some(now));
        assert_eq!(st.source.as_deref(), Some("estimate"));
        assert_ne!(st.used, Some(41.2));
    }

    #[test]
    fn test_c1_with_past_reset_is_not_fresh() {
        let c = pc();
        let now = mon() + 8 * DAY;
        c.c1(now - 60, 41.2, mon() + WEEK_S);
        assert_ne!(status_in(&c.p, Some(now)).source.as_deref(), Some("statusline"));
    }

    #[test]
    fn test_corrupt_c1_is_ignored() {
        let c = pc();
        write_text(&c.p.file("claude-pace.json"), "{not json");
        assert_eq!(status_in(&c.p, Some(mon())).state, "unknown");
    }

    #[test]
    fn test_c1_without_seven_day_is_ignored() {
        let c = pc();
        let v = json!({"updated": mon(), "five_hour": {"used_percentage": 3, "resets_at": mon() + 100}});
        write_text(&c.p.file("claude-pace.json"), &v.to_string());
        assert_eq!(status_in(&c.p, Some(mon() + 10)).state, "unknown");
    }

    #[test]
    fn test_states() {
        let c = pc();
        let now = mon() + 2 * DAY; // Wednesday, budget 50
        for (used, want) in [(10.0, "ok"), (50.0, "ok"), (50.5, "ahead"), (84.9, "ahead"), (85.0, "critical"), (100.0, "critical")] {
            c.c1(now - 10, used, mon() + WEEK_S);
            assert_eq!(status_in(&c.p, Some(now)).state, want, "used {used}");
        }
    }

    #[test]
    fn test_critical_even_on_saturday() {
        let c = pc();
        let now = mon() + 5 * DAY;
        c.c1(now - 10, 90.0, mon() + WEEK_S);
        assert_eq!(status_in(&c.p, Some(now)).state, "critical");
    }

    #[test]
    fn test_error_gives_unknown() {
        // The Python test made `_read_json` raise. Here a cached result whose `files` is not a
        // number fails inside the status read; the answer is still the "unknown" status.
        let c = pc();
        let body = json!({"v": 2, "at": mon(), "ws": fallback_week_start(mon()), "weight": 1.0, "files": "x", "complete": true});
        write_text(&c.p.file("claude-pace-result.json"), &body.to_string());
        let st = status_in(&c.p, Some(mon()));
        assert_eq!((st.state.as_str(), st.used, st.source, st.resets_at), ("unknown", None, None, None));
        assert!(status_json(&status_in(&c.p, Some(mon()))).contains("\"resets_at\": null"));
    }

    // TestEstimate
    #[test]
    fn test_weight_formula_and_percent() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        let st = c.estimate(mon() + DAY, Some(WEIGHT * 100.0));
        assert_eq!(st.source.as_deref(), Some("estimate"));
        assert!(close(st.used.unwrap(), 1.0, 1), "{:?}", st.used);
    }

    #[test]
    fn test_default_week_weight() {
        assert_eq!(DEFAULT_WEEK_WEIGHT, 5860.0);
    }

    // Work days and plan come from the settings (the fixtures above are pro and mon-sat)
    #[test]
    fn work_days_names() {
        assert_eq!(work_days("mon-fri"), 5);
        assert_eq!(work_days("mon-sat"), 6);
        assert_eq!(work_days("mon-sun"), 7);
        assert_eq!(work_days(" MON-SAT "), 6);
        assert_eq!(work_days(""), 5);
        assert_eq!(work_days("tue-sat"), 5);
    }

    #[test]
    fn budget_mon_fri_day_one_is_20() {
        assert_eq!(budget_for(mon(), mon(), 5), 20.0);
        assert_eq!(budget_for(mon() + 2 * DAY, mon(), 5), 60.0);
    }

    #[test]
    fn budget_caps_at_100_on_day_six_of_five() {
        assert_eq!(budget_for(mon() + 4 * DAY, mon(), 5), 100.0);
        assert_eq!(budget_for(mon() + 5 * DAY, mon(), 5), 100.0);
        assert_eq!(budget_for(mon() + 6 * DAY, mon(), 5), 100.0);
        assert_eq!(budget_for(mon() + 6 * DAY, mon(), 7), 100.0);
        assert!(close(budget_for(mon(), mon(), 7), 14.3, 1));
    }

    #[test]
    fn mon_sat_matches_old_budget() {
        let mut x: u64 = 0x2545F4914F6CDD1D; // a fixed xorshift: 300 values, the same on every run
        for _ in 0..300 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let ws = mon() + (x % 3) as i64 * WEEK_S;
            let now = ws - DAY + (x >> 8) as i64 % (10 * DAY);
            let old = {
                let idx = (local_day(now) - local_day(ws)).max(0);
                (idx + 1).min(6) as f64 / 6.0 * 100.0
            };
            assert_eq!(budget_for(now, ws, work_days("mon-sat")), old, "now {now} ws {ws}");
        }
    }

    #[test]
    fn plan_weights() {
        assert_eq!(plan_weight("pro"), Some(5860.0));
        assert_eq!(plan_weight("team"), Some(5860.0));
        assert_eq!(plan_weight("max5"), Some(29300.0));
        assert_eq!(plan_weight("max20"), Some(117200.0));
        assert_eq!(plan_weight("MAX5"), Some(29300.0));
        for none in ["api", "unknown", "", "max100"] {
            assert_eq!(plan_weight(none), None, "{none}");
        }
    }

    #[test]
    fn plan_max5_weight() {
        let e = env();
        let p = Paths { plan: "max5".into(), ..e.p.clone() };
        assert_eq!(week_weight_in(&p), Some(29300.0));
    }

    #[test]
    fn calibration_beats_plan() {
        let e = env();
        let p = Paths { plan: "max5".into(), ..e.p.clone() };
        seed_weight(&p, 2000.0, Some(mon()), "seed", None);
        assert_eq!(week_weight_in(&p), Some(2000.0));
        let api = Paths { plan: "api".into(), ..e.p.clone() };
        assert_eq!(week_weight_in(&api), Some(2000.0)); // a calibration also rescues an unknown plan
    }

    #[test]
    fn plan_api_is_unknown_without_cache() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        let api = Paths { plan: "unknown".into(), ..c.p.clone() };
        refresh_in(&api, 5.0, Some(mon() + DAY));
        let st = status_in(&api, Some(mon() + DAY));
        assert_eq!((st.state.as_str(), st.used, st.source), ("unknown", None, None));
        // the same transcripts on a known plan give an estimate
        let pro = Paths { plan: "pro".into(), ..c.p.clone() };
        assert_eq!(status_in(&pro, Some(mon() + DAY)).source.as_deref(), Some("estimate"));
        // a fresh statusline cache needs no plan
        c.c1(mon() + DAY - 60, 41.2, mon() + WEEK_S);
        let st = status_in(&api, Some(mon() + DAY));
        assert_eq!((st.source.as_deref(), st.used), (Some("statusline"), Some(41.2)));
    }

    #[test]
    fn budget_follows_the_work_days_of_the_paths() {
        let c = pc();
        let five = Paths { days: 5, ..c.p.clone() };
        assert_eq!(status_in(&five, Some(mon())).budget, 20.0);
        assert_eq!(status_in(&c.p, Some(mon())).budget, 16.7); // the fixture is mon-sat
    }

    #[test]
    fn test_cost_per_model_family() {
        // input 1000, output 500, cache write 2000 (1h), cache read 10000.
        for (model, (pi, po)) in [("claude-opus-4-8", (5.0, 25.0)), ("claude-sonnet-5-5", (3.0, 15.0)),
            ("claude-haiku-4-5", (1.0, 5.0)), ("fable-1", (15.0, 75.0))]
        {
            let c = pc();
            let want = (pi * (1000.0 + 2.0 * 2000.0 + 0.1 * 10000.0) + po * 500.0) / 1e6;
            let got = c.cost_of(model, 1000, 500, 2000, 10000);
            assert!(close(got, want, 7), "{model}: {got} vs {want}");
        }
    }

    #[test]
    fn test_cache_write_split_5m_and_1h() {
        let c = pc();
        let mut r = rv(mon() + 60, "m1", "claude-opus-4-8", 0, 0, 3000, 0);
        r["message"]["usage"]["cache_creation"] = json!({"ephemeral_5m_input_tokens": 1000, "ephemeral_1h_input_tokens": 2000});
        c.lines("a.jsonl", &[r]);
        c.refresh(5.0, mon() + DAY);
        assert!(close(c.weight(), 5.0 * (1.25 * 1000.0 + 2.0 * 2000.0) / 1e6, 7), "{}", c.weight());
    }

    #[test]
    fn test_cache_write_without_split_counts_as_1h() {
        let c = pc();
        assert!(close(c.cost_of("claude-opus-4-8", 0, 0, 3000, 0), 5.0 * 2.0 * 3000.0 / 1e6, 7));
    }

    #[test]
    fn test_old_cache_formats_are_rebuilt_or_ignored() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        write_text(&c.p.file("claude-pace-scan.json"), &json!({"v": 1, "ws": mon(), "files": {}}).to_string());
        let old = json!({"at": mon() + DAY, "ws": mon(), "weight": 1e9, "files": 5, "complete": true});
        write_text(&c.p.file("claude-pace-result.json"), &old.to_string());
        write_text(&c.p.file("claude-pace-calib.json"), &json!({"week_weight": 1.3e9, "updated": mon()}).to_string());
        assert_eq!(status_in(&c.p, Some(mon() + DAY)).state, "unknown"); // old result ignored
        assert_eq!(week_weight_in(&c.p), Some(DEFAULT_WEEK_WEIGHT)); // old calibration ignored
        c.refresh(5.0, mon() + DAY);
        assert!(close(c.weight(), WEIGHT, 7));
        assert_eq!(c.scan()["v"].as_f64(), Some(SCAN_FORMAT));
    }

    #[test]
    fn test_dedupe_streamed_records_last_wins() {
        let c = pc();
        let mut recs = vec![];
        for (dt, out) in [(60, 10), (61, 200), (62, 500)] {
            recs.push(rv(mon() + dt, "m1", "claude-sonnet-5-5", 1000, out, 2000, 10000));
        }
        c.lines("a.jsonl", &recs);
        c.estimate(mon() + DAY, Some(WEIGHT * 100.0));
        assert!(close(c.weight(), WEIGHT, 7), "{}", c.weight());
    }

    #[test]
    fn test_same_id_in_two_files_counts_per_file() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        c.lines("b.jsonl", &[rec(mon() + 60, "m1")]);
        c.estimate(mon() + DAY, None);
        assert!(close(c.weight(), 2.0 * WEIGHT, 7));
    }

    #[test]
    fn test_free_and_unknown_models_are_skipped() {
        let c = pc();
        let recs = [
            rv(mon() + 60, "m1", "openai/gpt-oss-120b:free", 1000, 500, 2000, 10000),
            rv(mon() + 61, "m2", "<synthetic>", 1000, 500, 2000, 10000),
            rv(mon() + 62, "m3", "claude-opus-4-8", 1000, 500, 2000, 10000),
            rv(mon() + 63, "m4", "claude-haiku-4-5", 1000, 500, 2000, 10000),
            rv(mon() + 64, "m5", "fable-1", 1000, 500, 2000, 10000),
        ];
        c.lines("a.jsonl", &recs);
        c.estimate(mon() + DAY, None);
        let per = |pi: f64, po: f64| (pi * (1000.0 + 2.0 * 2000.0 + 0.1 * 10000.0) + po * 500.0) / 1e6;
        assert!(close(c.weight(), per(5.0, 25.0) + per(1.0, 5.0) + per(15.0, 75.0), 7));
    }

    #[test]
    fn test_old_messages_are_skipped() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() - 60, "old"), rec(mon() + 60, "new")]);
        c.estimate(mon() + DAY, None);
        assert!(close(c.weight(), WEIGHT, 7));
    }

    #[test]
    fn test_corrupt_and_odd_lines_are_skipped() {
        let c = pc();
        let recs = [
            "{broken".to_string(),
            String::new(),
            r#"{"type":"assistant","message":"str"}"#.to_string(),
            r#"{"type":"assistant","timestamp":"nope","message":{"id":"x","model":"claude-sonnet-5-5","usage":{"output_tokens":5}}}"#.to_string(),
            r#"{"type":"assistant","timestamp":null,"message":{"usage":null}}"#.to_string(),
            r#"{"usage": 1}"#.to_string(),
            rec(mon() + 60, "m1").to_string(),
        ];
        c.lines("a.jsonl", &recs);
        c.estimate(mon() + DAY, None);
        assert!(close(c.weight(), WEIGHT, 7));
    }

    #[test]
    fn test_null_token_fields() {
        let c = pc();
        let mut r = rec(mon() + 60, "m1");
        r["message"]["usage"]["cache_read_input_tokens"] = Value::Null;
        c.lines("a.jsonl", &[r]);
        c.estimate(mon() + DAY, None);
        assert!(close(c.weight(), (3.0 * (1000.0 + 2.0 * 2000.0) + 15.0 * 500.0) / 1e6, 7));
    }

    #[test]
    fn test_subagent_files_are_counted() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        c.write_in("agent-1.jsonl", Some("sess1"), &[rec(mon() + 70, "s1")], mon() + DAY);
        c.estimate(mon() + DAY, None);
        assert!(close(c.weight(), 2.0 * WEIGHT, 7));
    }

    #[test]
    fn test_incremental_scan_reads_only_new_bytes() {
        let c = pc();
        let path = c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        c.estimate(mon() + DAY, None);
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(&format!("{}\n", rec(mon() + 90, "m2")));
        fs::write(&path, text).unwrap();
        c.refresh(5.0, mon() + DAY);
        assert!(close(c.weight(), 2.0 * WEIGHT, 7));
        let scan = c.scan();
        let key = path.to_string_lossy().into_owned();
        assert_eq!(scan["files"][key.as_str()]["offset"].as_u64(), Some(fs::metadata(&path).unwrap().len()));
    }

    #[test]
    fn test_partial_last_line_is_left_for_next_time() {
        let c = pc();
        let path = c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        let next = rec(mon() + 90, "m2").to_string();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(&next[..40]); // no newline yet
        fs::write(&path, &text).unwrap();
        c.estimate(mon() + DAY, None);
        assert!(close(c.weight(), WEIGHT, 7));
        text.push_str(&next[40..]);
        text.push('\n');
        fs::write(&path, text).unwrap();
        c.refresh(5.0, mon() + DAY);
        assert!(close(c.weight(), 2.0 * WEIGHT, 7));
    }

    #[test]
    fn test_truncated_file_is_rescanned() {
        let c = pc();
        let path = c.lines("a.jsonl", &[rec(mon() + 60, "m1"), rec(mon() + 61, "m2")]);
        c.estimate(mon() + DAY, None);
        c.lines("a.jsonl", &[rec(mon() + 62, "m3")]);
        let key = path.to_string_lossy().into_owned();
        let offset = c.scan()["files"][key.as_str()]["offset"].as_u64().unwrap();
        assert!(fs::metadata(&path).unwrap().len() < offset);
        c.refresh(5.0, mon() + DAY);
        assert!(close(c.weight(), WEIGHT, 7));
    }

    #[test]
    fn test_status_uses_cached_result_under_120s() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        let now = mon() + DAY;
        c.estimate(now, Some(WEIGHT * 10.0));
        let first = status_in(&c.p, Some(now)).used.unwrap();
        c.lines("b.jsonl", &[rec(mon() + 70, "m2")]);
        assert_eq!(status_in(&c.p, Some(now + 30)).used, Some(first)); // no scan in status()
        c.refresh(5.0, now + 40);
        assert!(status_in(&c.p, Some(now + 41)).used.unwrap() > first);
    }

    #[test]
    fn test_status_ignores_result_from_another_week() {
        let c = pc();
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        c.refresh(5.0, mon() + DAY);
        assert_eq!(status_in(&c.p, Some(mon() + 8 * DAY)).state, "unknown");
    }

    #[test]
    fn test_no_transcripts_is_unknown() {
        let c = pc();
        c.refresh(5.0, mon() + DAY);
        assert_eq!(status_in(&c.p, Some(mon() + DAY)).state, "unknown");
    }

    // TestTimeBudget
    fn many(n: i64) -> Vec<Value> {
        (0..n).map(|i| rec(mon() + i, &format!("m{i}"))).collect()
    }

    #[test]
    fn test_budget_stops_and_next_call_continues() {
        let c = pc();
        c.lines("a.jsonl", &many(1500));
        let r = c.refresh(0.0, mon() + DAY);
        assert!(!r.complete);
        assert!(c.weight() < 1500.0 * WEIGHT);
        let mut r = r;
        for _ in 0..20 {
            r = c.refresh(0.0, mon() + DAY);
            if r.complete {
                break;
            }
        }
        assert!(r.complete);
        assert!(close(c.weight(), 1500.0 * WEIGHT, 7));
    }

    #[test]
    fn test_budget_makes_progress_every_call() {
        let c = pc();
        c.lines("a.jsonl", &many(1500));
        c.refresh(0.0, mon() + DAY);
        let w1 = c.weight();
        c.refresh(0.0, mon() + DAY);
        assert!(c.weight() > w1);
    }

    // TestCalibration
    fn ten() -> Vec<Value> {
        (0..10).map(|i| rec(mon() + 60, &format!("m{i}"))).collect()
    }

    #[test]
    fn test_fresh_c1_stores_week_weight() {
        let c = pc();
        let now = mon() + DAY;
        c.lines("a.jsonl", &ten());
        c.c1(now - 10, 10.0, mon() + WEEK_S);
        c.refresh(5.0, now);
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), 10.0 * WEIGHT / 0.10, 7));
    }

    #[test]
    fn test_low_usage_does_not_calibrate() {
        let c = pc();
        let now = mon() + DAY;
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        c.c1(now - 10, 4.9, mon() + WEEK_S);
        c.refresh(5.0, now);
        assert!(!c.has_calib());
    }

    #[test]
    fn test_stale_c1_does_not_calibrate() {
        let c = pc();
        let now = mon() + DAY;
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        c.c1(now - 7 * 3600, 20.0, mon() + WEEK_S);
        c.refresh(5.0, now);
        assert!(!c.has_calib());
    }

    #[test]
    fn test_incomplete_scan_does_not_calibrate() {
        let c = pc();
        let now = mon() + DAY;
        c.lines("a.jsonl", &many(1500));
        c.c1(now - 10, 20.0, mon() + WEEK_S);
        let r = c.refresh(0.0, now);
        assert!(!r.complete);
        assert!(!c.has_calib());
    }

    #[test]
    fn test_estimate_uses_calibration() {
        let c = pc();
        let now = mon() + DAY;
        c.lines("a.jsonl", &ten());
        c.c1(now - 10, 10.0, mon() + WEEK_S);
        c.refresh(5.0, now);
        fs::remove_file(c.p.file("claude-pace.json")).unwrap();
        c.refresh(5.0, now);
        let st = status_in(&c.p, Some(now));
        assert_eq!(st.source.as_deref(), Some("estimate"));
        assert!(close(st.used.unwrap(), 10.0, 1), "{:?}", st.used);
    }

    #[test]
    fn test_seed() {
        let c = pc();
        let now = mon() + DAY;
        c.lines("a.jsonl", &ten());
        assert!(seed_in(&c.p, 25.0, Some(now)));
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), 10.0 * WEIGHT / 0.25, 7));
        assert!(close(status_in(&c.p, Some(now)).used.unwrap(), 25.0, 1));
    }

    #[test]
    fn test_seed_needs_a_complete_refresh() {
        // The Python test mocked `refresh` to report an incomplete scan. Without that seam the
        // refusal is reached through the other half of the same guard: a week with no usage
        // (weight 0) stores nothing; the stored case is `test_seed`.
        let c = pc();
        assert!(!seed_in(&c.p, 25.0, Some(mon() + DAY)));
        assert!(!c.has_calib());
        c.lines("a.jsonl", &[rec(mon() + 60, "m1")]);
        assert!(seed_in(&c.p, 25.0, Some(mon() + DAY)));
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), WEIGHT / 0.25, 7));
    }

    #[test]
    fn test_seed_rejects_bad_values() {
        let c = pc();
        for bad in [0.0, -3.0, 101.0, f64::NAN] {
            assert!(!seed_in(&c.p, bad, Some(mon() + DAY)), "{bad}");
        }
        assert!(!c.has_calib());
    }

    #[test]
    fn test_corrupt_calibration_uses_default() {
        let c = pc();
        write_text(&c.p.file("claude-pace-calib.json"), "zzz");
        assert_eq!(week_weight_in(&c.p), Some(DEFAULT_WEEK_WEIGHT));
    }

    // TestLimitHit
    fn refresh_default(c: &Pc) -> RefreshResult {
        c.refresh(5.0, mon() + 3 * DAY)
    }

    #[test]
    fn test_hit_sets_calibration() {
        let c = pc();
        let t0 = mon() + 2 * DAY;
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), hit(t0), big(t0 + 60, "after", 100_000_000)]);
        refresh_default(&c);
        let cal = c.calib_json();
        assert!(close(cal["week_weight"].as_f64().unwrap(), 2000.0, 7)); // the record after the hit is not counted
        assert_eq!(
            (cal["source"].as_str(), cal["week_start"].as_i64(), cal["updated"].as_i64()),
            (Some("limit-hit"), Some(mon()), Some(t0))
        );
        assert_eq!(week_weight_in(&c.p), cal["week_weight"].as_f64());
        // 2500 used against a 2000 week: capped at 100 and critical
        let st = status_in(&c.p, Some(mon() + 3 * DAY));
        assert_eq!((st.used, st.state.as_str()), (Some(100.0), "critical"));
    }

    #[test]
    fn test_hit_in_a_second_file_and_subagent_counts() {
        let c = pc();
        let t0 = mon() + 2 * DAY;
        c.lines("a.jsonl", &[big(mon() + 3600, "b1", 200_000_000)]);
        c.write_in("agent-1.jsonl", Some("s1"), &[big(mon() + 7200, "b2", 200_000_000)], mon() + DAY);
        c.lines("b.jsonl", &[hit(t0)]);
        refresh_default(&c);
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), 2000.0, 7));
    }

    #[test]
    fn test_no_hit_keeps_default() {
        let c = pc();
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1")]);
        refresh_default(&c);
        assert!(!c.has_calib());
        assert_eq!(week_weight_in(&c.p), Some(DEFAULT_WEEK_WEIGHT));
    }

    #[test]
    fn test_a_reply_that_quotes_the_phrase_is_not_a_hit() {
        let c = pc();
        let mut quote = rv(mon() + 2 * DAY, "q1", "claude-opus-4-8", 1000, 500, 2000, 10000);
        quote["message"]["content"] = json!([{"type": "text", "text": "You've hit your weekly limit, they said"}]);
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), quote]);
        refresh_default(&c);
        assert!(!c.has_calib());
    }

    #[test]
    fn test_phrase_is_case_insensitive_and_system_records_count() {
        let c = pc();
        let sysrec = json!({"type": "system", "timestamp": iso(mon() + 2 * DAY), "content": "YOU'VE HIT YOUR WEEKLY LIMIT"});
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), sysrec]);
        refresh_default(&c);
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), 2000.0, 7));
    }

    #[test]
    fn test_first_hit_of_the_week_wins() {
        let c = pc();
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), hit(mon() + DAY), big2000(mon() + DAY + 60, "b2"), hit(mon() + 3 * DAY)]);
        refresh_default(&c);
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), 2000.0, 7));
    }

    #[test]
    fn test_cheap_hit_stores_nothing() {
        let c = pc();
        c.lines("a.jsonl", &[big(mon() + 3600, "b1", 100_000_000), hit(mon() + DAY)]); // cost 500
        refresh_default(&c);
        assert!(!c.has_calib());
    }

    #[test]
    fn test_partial_scan_stores_nothing_until_complete() {
        let c = pc();
        let mut recs = vec![big2000(mon() + 3600, "b1")];
        recs.extend((0..1500).map(|i| rec(mon() + 60 + i, &format!("f{i}"))));
        recs.push(hit(mon() + DAY));
        c.lines("a.jsonl", &recs);
        let r = c.refresh(0.0, mon() + 3 * DAY);
        assert!(!r.complete);
        assert!(!c.has_calib());
        for _ in 0..20 {
            if c.refresh(0.0, mon() + 3 * DAY).complete {
                break;
            }
        }
        assert!(c.calib_json()["week_weight"].as_f64().unwrap() > 2000.0);
    }

    #[test]
    fn test_newer_seed_wins() {
        let c = pc();
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), hit(mon() + 2 * DAY)]);
        seed_weight(&c.p, 3000.0, Some(mon() + 2 * DAY + 600), "seed", None); // after the hit
        refresh_default(&c);
        assert_eq!(c.calib_json()["week_weight"].as_f64(), Some(3000.0));
        assert_eq!(c.calib_json()["source"].as_str(), Some("seed"));
    }

    #[test]
    fn test_newer_hit_wins_over_older_seed() {
        let c = pc();
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), hit(mon() + 2 * DAY)]);
        seed_weight(&c.p, 3000.0, Some(mon() + DAY), "seed", None); // before the hit
        refresh_default(&c);
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), 2000.0, 7));
    }

    #[test]
    fn test_hit_is_stored_once() {
        let c = pc();
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), hit(mon() + 2 * DAY)]);
        refresh_default(&c);
        let path = c.p.file("claude-pace-calib.json");
        let stamp = UNIX_EPOCH + Duration::from_secs(1);
        fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(stamp).unwrap();
        refresh_default(&c);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), stamp); // not rewritten
    }

    #[test]
    fn test_no_hit_this_week_keeps_last_week_calibration() {
        let c = pc();
        c.lines("a.jsonl", &[big2000(mon() + 3600, "b1"), hit(mon() + 2 * DAY)]);
        refresh_default(&c);
        let nxt = mon() + 8 * DAY;
        c.write_in("b.jsonl", None, &[big(mon() + WEEK_S + 3600, "b9", 100_000_000)], nxt);
        c.refresh(5.0, nxt);
        assert!(close(c.calib_json()["week_weight"].as_f64().unwrap(), 2000.0, 7));
        assert!(close(status_in(&c.p, Some(nxt)).used.unwrap(), 500.0 / 2000.0 * 100.0, 7));
    }

    #[test]
    fn test_backfill_finds_the_previous_week_hit_once() {
        let c = pc();
        let prev = mon() - WEEK_S;
        c.lines("a.jsonl", &[big2000(prev + 3600, "b1"), hit(prev + 5 * DAY)]);
        c.lines("b.jsonl", &[big(mon() + 3600, "b2", 100_000_000)]);
        c.refresh(5.0, mon() + DAY);
        let cal = c.calib_json();
        assert!(close(cal["week_weight"].as_f64().unwrap(), 2000.0, 7));
        assert_eq!((cal["source"].as_str(), cal["week_start"].as_i64()), (Some("limit-hit"), Some(prev)));
        // this week's cost still counts only this week
        assert!(close(c.weight(), 500.0, 7));
    }

    #[test]
    fn test_backfill_runs_once_without_a_hit() {
        let c = pc();
        let prev = mon() - WEEK_S;
        c.lines("a.jsonl", &[big2000(prev + 3600, "b1")]);
        c.refresh(5.0, mon() + DAY);
        let scan = c.scan();
        assert_eq!((scan["lo"].as_i64(), scan["backfilled"].as_bool()), (Some(prev), Some(true)));
        c.refresh(5.0, mon() + DAY + 10);
        let scan = c.scan();
        assert_eq!((scan["lo"].as_i64(), scan["backfilled"].as_bool()), (Some(mon()), Some(true))); // previous week dropped again
    }

    #[test]
    fn test_no_backfill_with_a_warm_calibration() {
        let c = pc();
        let prev = mon() - WEEK_S;
        seed_weight(&c.p, 3000.0, Some(mon()), "seed", None);
        c.lines("a.jsonl", &[big2000(prev + 3600, "b1"), hit(prev + 5 * DAY)]);
        c.refresh(5.0, mon() + DAY);
        assert_eq!(c.calib_json()["week_weight"].as_f64(), Some(3000.0));
    }

    // TestEpoch
    #[test]
    fn test_epoch_forms() {
        assert_eq!(epoch(Some(&json!(1760860800))), Some(1760860800));
        assert_eq!(epoch(Some(&json!(1760860800000i64))), Some(1760860800)); // milliseconds
        assert_eq!(epoch(Some(&json!("1760860800"))), Some(1760860800));
        assert_eq!(epoch(Some(&json!("2025-10-19T08:00:00Z"))), Some(1760860800));
        assert_eq!(epoch(Some(&json!(null))), None);
        assert_eq!(epoch(Some(&json!("junk"))), None);
        assert_eq!(epoch(Some(&json!(true))), None);
    }
}
