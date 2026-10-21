//! harness_stats: measure Claude Code sessions from local transcripts.

use agent_config::guard_checks::{Fire, Gate, REGISTRY, expanduser, fire_log_path, read_fire_log};
use agent_config::paths;
use agent_config::pycompat::{fname, list_dir, parse_ts_micros, py_float, truthy};
use agent_config::scan::{Counter, py_dumps};
use agent_config::stores::claude::{desktop_dir, desktop_meta, project_name, text_of};
use clap::{Args, Parser, Subcommand};
use regex::Regex;
use serde_json::{Map, Number, Value};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

const DOC: &str = "harness_stats: measure Claude Code sessions from local transcripts.

  harness_stats collect [--since 2026-10-01] [--out FILE] [--exclude SID ...]
  harness_stats cuts    [--repo ~/dotfiles | --gh OWNER/REPO] [--since DATE] [--path claude]
  harness_stats report  [--in FILE] [--cut ISO ...] [--by period|project]
  harness_stats errors  [--in FILE] [--project NAME] [--top 12] [--daily]
  harness_stats skills  [--in FILE]
  harness_stats inventory [--in FILE] [--top 8]
  harness_stats agents  [--in FILE] [--cut ISO ...]
  harness_stats guard   [--days 30] [--log FILE]

collect reads ~/.claude/projects/*/*.jsonl (main sessions), their subagents/*.jsonl and
.meta.json, and the desktop app's session records (title, archived). It writes one JSON
row per main session. Everything else reads that file. Default file:
$TMPDIR/harness_stats.json. guard reads the guard's fire log instead
(${XDG_CACHE_HOME:-~/.cache}/dots-guard-fires.log).";

const INTERRUPT: &str = "[Request interrupted by user";
const POLL_TOOLS: [&str; 3] = ["ScheduleWakeup", "CronCreate", "Monitor"];

static CORRECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(why did you|that'?s wrong|is wrong|again\?|you didn'?t|not what i (asked|wanted|said)|ugh|wtf|still (not|broken|wrong|failing)|as i said|i said|i told you|i already (said|told))\b",
    )
    .unwrap()
});
static HANDOFF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Read \S+\.md and do it").unwrap());
static CD_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:cd [^&;]+(?:&&|;)\s*)+").unwrap());
static PATHS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/private/tmp/\S+|/Users/[^\s/]+/\S*").unwrap());
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());
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

fn default_out() -> String {
    let tmp = std::env::var("TMPDIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if cfg!(windows) { std::env::temp_dir().to_string_lossy().into_owned() } else { "/tmp".to_string() }
    });
    Path::new(&tmp).join("harness_stats.json").to_string_lossy().into_owned()
}

fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&NULL)
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

/// Python round(x, nd) for nd >= 0 (correctly rounded, ties to even on the exact value).
fn pyround(x: f64, nd: usize) -> f64 {
    format!("{x:.nd$}").parse().unwrap_or(x)
}

/// Python round(x) -> int.
fn pyround0(x: f64) -> i64 {
    x.round_ties_even() as i64
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

/// An aware datetime: UTC microseconds plus the original UTC offset in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dt {
    micros: i64,
    off: i64,
}

impl Dt {
    fn isoformat(&self) -> String {
        let local = self.micros + self.off * 1_000_000;
        let us = local.rem_euclid(1_000_000);
        let wall = jiff::Timestamp::from_microsecond(local).unwrap_or_default().to_zoned(jiff::tz::TimeZone::UTC);
        let mut s = wall.datetime().strftime("%Y-%m-%dT%H:%M:%S").to_string();
        if us != 0 {
            let _ = write!(s, ".{us:06}");
        }
        let sign = if self.off < 0 { '-' } else { '+' };
        let a = self.off.abs();
        let _ = write!(s, "{}{:02}:{:02}", sign, a / 3600, a % 3600 / 60);
        if a % 60 != 0 {
            let _ = write!(s, ":{:02}", a % 60);
        }
        s
    }
    fn timestamp(&self) -> f64 {
        self.micros as f64 / 1e6
    }
}

/// datetime.fromisoformat(s.replace("Z", "+00:00")). A missing offset counts as UTC.
fn parse_ts(s: &str) -> Option<Dt> {
    parse_ts_micros(s).map(|(micros, off)| Dt { micros, off })
}

// ---------------------------------------------------------------------------
// Counters as JSON
// ---------------------------------------------------------------------------

/// The counter as a JSON object; `only` picks the items and their order, else all in insertion order.
fn counter_to_json(c: &Counter<String>, only: Option<&[(String, i64)]>) -> Value {
    let mut m = Map::new();
    match only {
        Some(items) => {
            for (k, v) in items {
                m.insert(k.clone(), Value::from(*v));
            }
        }
        None => {
            for (k, v) in c.iter() {
                m.insert(k.clone(), Value::from(v));
            }
        }
    }
    Value::Object(m)
}

fn counter_from_json(v: &Value) -> Counter<String> {
    let mut c = Counter::new();
    if let Some(o) = v.as_object() {
        for (k, x) in o {
            c.add(k.clone(), as_i(x));
        }
    }
    c
}

fn fnum(x: f64) -> Value {
    Number::from_f64(x).map_or(Value::Null, Value::Number)
}

// ---------------------------------------------------------------------------
// Names and transcripts
// ---------------------------------------------------------------------------

fn is_dir(p: &Path) -> bool {
    p.is_dir()
}

fn lexists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
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
    let (mut tools, mut skills, mut bash, mut errors) =
        (Counter::new(), Counter::new(), Counter::new(), Counter::new());
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
        if truthy(get(&o, "timestamp"))
            && let Some(s) = get(&o, "timestamp").as_str()
            && let Some(d) = parse_ts(s)
        {
            times.push(d);
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
                                "git", "npm", "pnpm", "bun", "go", "uv", "npx", "gh", "wrangler", "python3", "node",
                                "make", "cargo",
                            ];
                            if !cmd.is_empty() {
                                let k = if TWO.contains(&cmd[0]) {
                                    cmd.iter().take(2).copied().collect::<Vec<_>>().join(" ")
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
                if let Some(arr) = content.as_array()
                    && arr.iter().any(|x| x.is_object() && get(x, "type").as_str() == Some("tool_result"))
                {
                    for x in arr {
                        if x.is_object() && get(x, "type").as_str() == Some("tool_result") && truthy(get(x, "is_error"))
                        {
                            let nm =
                                names.get(&get(x, "tool_use_id").to_string()).cloned().unwrap_or_else(|| "?".into());
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
                    r.set("from_handoff".into(), 1);
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
    Scan {
        r,
        tools,
        skills,
        bash,
        errors,
        agents,
        times,
        first_ctx: first_ctx.unwrap_or(0),
        peak,
        models,
        active_s: active,
        version,
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn cmd_collect(a: &CollectArgs) {
    let since =
        if a.since.contains('T') { parse_ts(&a.since) } else { parse_ts(&format!("{}T00:00:00+00:00", a.since)) }
            .unwrap_or_else(|| fail(&format!("ValueError: Invalid isoformat string: '{}'", a.since)));
    let h = paths::home();
    let projects = h.join(".claude").join("projects");
    let meta = desktop_meta(&desktop_dir(Path::new(&h)));
    let mut rows: Vec<Value> = vec![];
    for pdir in list_dir(&projects) {
        for f in list_dir(&pdir) {
            let n = fname(&f);
            if f.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let sid = n[..n.len() - 6].to_string();
            let mtime = fs::metadata(&f).and_then(|m| m.modified()).ok().map(|t| match t.duration_since(UNIX_EPOCH) {
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
                if sf.extension().is_none_or(|x| x != "jsonl") {
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
                if let Ok(b) = fs::read(&mp)
                    && let Ok(v) = serde_json::from_slice::<Value>(&b)
                {
                    let m = get(&v, "model");
                    smodels.add(if truthy(m) { key_str(m) } else { "inherit".into() }, 1);
                    let e = get(&v, "effort");
                    seffort.add(if truthy(e) { key_str(e) } else { "unset".into() }, 1);
                    md = v;
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
            sub.insert("models".into(), counter_to_json(&smodels, None));
            sub.insert("effort".into(), counter_to_json(&seffort, None));
            sub.insert("spawns".into(), Value::Array(spawns));
            let first = s.times[0];
            let last = *s.times.last().unwrap();
            let mut row = Map::new();
            row.insert("sid".into(), Value::from(sid.clone()));
            row.insert("project".into(), Value::from(project_name(&paths::home().to_string_lossy(), &folder)));
            row.insert("folder".into(), Value::from(folder.clone()));
            row.insert("start".into(), Value::from(first.isoformat()));
            row.insert("end".into(), Value::from(last.isoformat()));
            row.insert("title".into(), m.map_or(Value::Null, |m| m.title.clone()));
            row.insert("archived".into(), m.map_or(Value::Null, |m| Value::Bool(m.archived)));
            row.insert("effort".into(), m.map_or(Value::Null, |m| m.effort.clone()));
            row.insert("cli".into(), s.version.clone());
            row.insert(
                "model".into(),
                s.models.most_common(Some(1)).first().and_then(|(k, _)| k.clone()).map_or(Value::Null, Value::from),
            );
            row.insert("first_ctx".into(), Value::from(s.first_ctx));
            row.insert("peak_ctx".into(), Value::from(s.peak));
            row.insert("active_min".into(), fnum(pyround(s.active_s / 60.0, 1)));
            row.insert("wall_min".into(), fnum(pyround((last.micros - first.micros) as f64 / 1e6 / 60.0, 1)));
            row.insert("stats".into(), counter_to_json(&s.r, None));
            row.insert("tools".into(), counter_to_json(&s.tools, None));
            row.insert("skills".into(), counter_to_json(&s.skills, None));
            row.insert("bash".into(), counter_to_json(&s.bash, Some(&s.bash.most_common(Some(15)))));
            row.insert("errors".into(), counter_to_json(&errors, Some(&errors.most_common(Some(20)))));
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
    a.iter().map(ToString::to_string).collect()
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
        let root = first.last().map_or(root0, ToString::to_string);
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
            fail(&format!("ValueError: Invalid isoformat string: '{first}'"));
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
    let b = fs::read(inp).unwrap_or_else(|e| fail(&format!("FileNotFoundError: {inp}: {e}")));
    match serde_json::from_slice::<Value>(&b) {
        Ok(Value::Array(a)) => a,
        Ok(_) => fail("TypeError: harness_stats file is not a list of rows"),
        Err(e) => fail(&format!("json.decoder.JSONDecodeError: {e}")),
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
            Self::I(i) => *i as f64,
            Self::F(f) => *f,
        }
    }
    fn show(&self) -> String {
        match self {
            Self::I(i) => i.to_string(),
            Self::F(f) => py_float(*f),
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
    if n % 2 == 1 { v[n / 2] } else { Num::F(f64::midpoint(v[n / 2 - 1].f(), v[n / 2].f())) }
}

fn nums<'a, F: Fn(&'a Value) -> &'a Value>(rows: &[&'a Value], f: F) -> Vec<Num> {
    rows.iter().filter_map(|r| num_of(f(r))).collect()
}

fn stat_i(r: &Value, k: &str) -> i64 {
    as_i(get(get(r, "stats"), k))
}

fn obj_sum(v: &Value) -> i64 {
    v.as_object().map_or(0, |o| o.values().map(as_i).sum())
}

fn summarize(rows: &[&Value]) -> Vec<(&'static str, String)> {
    let n = if rows.is_empty() { 1 } else { rows.len() } as f64;
    let g = |k: &str| -> i64 { rows.iter().map(|r| stat_i(r, k)).sum() };
    let prompts = {
        let p = g("prompts");
        if p == 0 { 1 } else { p }
    } as f64;
    let tools = {
        let t: i64 = rows.iter().map(|r| obj_sum(get(r, "tools"))).sum();
        if t == 0 { 1 } else { t }
    } as f64;
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
    let vs: Vec<String> =
        rows.iter().filter_map(|r| get(r, "cli").as_str().filter(|s| !s.is_empty()).map(ToString::to_string)).collect();
    let versions = if vs.is_empty() {
        "?".to_string()
    } else {
        format!("{}..{}", vs.iter().min().unwrap(), vs.iter().max().unwrap())
    };
    let first_ctx = nums(rows, |r| get(r, "first_ctx"));
    let peak_ctx = nums(rows, |r| get(r, "peak_ctx"));
    let prompts_per: Vec<Num> =
        rows.iter().map(|r| num_of(get(get(r, "stats"), "prompts")).unwrap_or(Num::I(0))).collect();
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
            rows.iter()
                .map(|r| POLL_TOOLS.iter().map(|t| as_i(get(get(r, "tools"), t))).sum::<i64>())
                .sum::<i64>()
                .to_string(),
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
        .map(|c| parse_ts(c).unwrap_or_else(|| fail(&format!("ValueError: Invalid isoformat string: '{c}'"))))
        .collect();
    cuts.sort_by_key(|d| d.micros);
    let mut groups: Groups = (0..=cuts.len()).map(|i| (format!("P{i}"), vec![])).collect();
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
    let groups: Groups = if a.by == "project" {
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
        g
    } else {
        let (cuts, g) = periods(&rows, &a.cut);
        out!("{}", periods_line(&cuts));
        g
    };
    table(&groups);
    out!("\nHigher is better: cache hit %, agent calls w/ effort %, from hand-off. Lower is better for the rest.");
    let small: Vec<&str> = groups.iter().filter(|(_, v)| v.len() < 20).map(|(k, _)| k.as_str()).collect();
    if !small.is_empty() {
        out!("\nEarly (under 20 sessions): {}. Read their numbers as a hint, not a result.", small.join(", "));
    }
}

/// Context sizes of the sub-agent spawns, grouped by agent type.
type SpawnCtxByType = Vec<(String, Vec<Num>)>;

fn cmd_agents(a: &AgentsArgs) {
    let rows = load(&a.inp);
    let (cuts, groups) = periods(&rows, &a.cut);
    let mut data: Vec<(String, SpawnCtxByType)> = vec![];
    for (k, rs) in &groups {
        let mut by: SpawnCtxByType = vec![];
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
        data.iter().map(|(k, _)| format!("{k} spawns (share) | {k} start K")).collect::<Vec<_>>().join(" | ")
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
    out!("\nSessions: {}", groups.iter().map(|(k, v)| format!("{} {}", k, v.len())).collect::<Vec<_>>().join(", "));
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
        .filter(|r| a.project.as_deref().is_none_or(|p| get(r, "project").as_str() == Some(p)))
        .collect();
    if a.daily {
        let mut days: Vec<(String, Counter<String>)> = vec![];
        let mut total: Counter<String> = Counter::new();
        for r in &rows {
            let d: String = pychars(get(r, "start").as_str().unwrap_or(""), 10);
            let e = counter_from_json(get(r, "errors"));
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
        let e = counter_from_json(get(r, "errors"));
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
    let h = paths::home();
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
        used.update(&counter_from_json(get(&r, "skills")));
    }
    let installed = installed_skills();
    out!("Used (Skill tool calls):");
    for (s, n) in used.most_common(None) {
        out!("{:>5}  {}", n, s);
    }
    out!("\nNames above that no longer exist were renamed or removed; check the skills repo history.");
    out!("Plugin skills and skillOverrides are not covered.");
    out!("\nInstalled in ~/.claude/skills, ~/*/.claude/skills or ~/work/*/.claude/skills, never called:");
    let mut left: Vec<&String> = installed.iter().filter(|s| !used.contains(*s)).collect();
    left.sort();
    for s in left {
        out!("       {}", s);
    }
}

fn day(ts: u64) -> String {
    i64::try_from(ts)
        .ok()
        .and_then(|s| jiff::Timestamp::from_second(s).ok())
        .map_or_else(|| "?".into(), |t| t.strftime("%F").to_string())
}

/// The fire table: per check its preset, whether it is on, its fires in the last `days` days by
/// kind, and the last fire; then the checks that are on and never fired in that window. A check
/// that keeps not firing is a shim the models outgrew: a candidate to remove.
fn guard_report(fires: &[Fire], now: u64, days: u64, gate: &Gate) -> String {
    let from = now.saturating_sub(days * 86_400);
    let mut s = String::new();
    let first = fires.iter().map(|f| f.at).min();
    match first {
        None => {
            let _ = writeln!(s, "The fire log is empty: no check fired yet, or the guard is older than the log.");
        }
        Some(t) if t > from => {
            let _ = writeln!(
                s,
                "The log starts {}, less than {days} days ago: a quiet check may not have had its chance yet.",
                day(t)
            );
        }
        Some(_) => {}
    }
    let _ = writeln!(s, "| check | preset | on | fires ({days} d) | kinds | last fire |");
    let _ = writeln!(s, "|---|---|---|---|---|---|");
    let mut quiet: Vec<&str> = vec![];
    for (check, preset) in REGISTRY {
        let id = check.id();
        let mine: Vec<_> = fires.iter().filter(|f| f.check == *check).collect();
        let recent: Vec<_> = mine.iter().filter(|f| f.at >= from).collect();
        let mut kinds: Counter<&str> = Counter::new();
        for f in &recent {
            kinds.add(f.kind.as_str(), 1);
        }
        let kinds = kinds.most_common(None).into_iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ");
        let last = mine.iter().map(|f| f.at).max().map_or_else(|| "never".into(), day);
        let on = gate.enabled(*check);
        let _ = writeln!(
            s,
            "| {id} | {preset} | {} | {} | {} | {last} |",
            if on { "yes" } else { "no" },
            recent.len(),
            if kinds.is_empty() { "-" } else { &kinds }
        );
        if on && recent.is_empty() {
            quiet.push(id);
        }
    }
    if first.is_none() {
        return s; // no log yet: every check would look quiet
    }
    let _ = writeln!(
        s,
        "\nOn and never fired in {days} days: {}",
        if quiet.is_empty() { "none".into() } else { quiet.join(", ") }
    );
    let _ =
        writeln!(s, "Read each one's doc comment in guard.rs before you cut it: some guard a rare, costly mistake.");
    s
}

fn cmd_guard(a: &GuardArgs) {
    let path = a.log.as_deref().map_or_else(fire_log_path, |p| PathBuf::from(expanduser(p)));
    let now = u64::try_from(jiff::Timestamp::now().as_second()).unwrap_or(0);
    out!("{}", guard_report(&read_fire_log(&path), now, a.days, &Gate::load(&paths::home())).trim_end());
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
    let h = paths::home();
    let n = count.most_common(None);
    let n = &n[..slice_end(n.len(), a.top)];
    for (p, c) in n {
        if p == "tmp" || p == "scratch" {
            continue;
        }
        let cands: Vec<PathBuf> = if p == "~" { vec![h.clone()] } else { vec![h.join("work").join(p), h.join(p)] };
        let Some(root) = cands.into_iter().find(|c| is_dir(c)) else {
            out!("| {} | {} | (folder gone) | | | | |", p, c);
            continue;
        };
        let md = if p == "~" { h.join(".claude").join("CLAUDE.md") } else { root.join("CLAUDE.md") };
        let lines = if md.is_file() { count_lines(&md) } else { 0 };
        let rules = if p == "~" { vec![] } else { glob_ext(&root.join(".claude").join("rules"), ".md") };
        let scoped = rules
            .iter()
            .filter(|f| {
                let t: String = read_text_ignore(f).unwrap_or_default().chars().take(600).collect();
                PATHS_KEY.is_match(&t)
            })
            .count();
        let agents = if p == "~" { 0 } else { glob_ext(&root.join(".claude").join("agents"), ".md").len() };
        let skills = if p == "~" {
            0
        } else {
            list_dir(&root.join(".claude").join("skills")).iter().filter(|d| lexists(&d.join("SKILL.md"))).count()
        };
        let mut hooks = String::new();
        let sj = root.join(".claude").join("settings.json");
        if p != "~" && sj.is_file() {
            hooks = match fs::read(&sj).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                Some(v) => {
                    let mut ks: Vec<String> =
                        get(&v, "hooks").as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
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
    Guard(GuardArgs),
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
struct GuardArgs {
    /// the window: fires in the last N days
    #[arg(long, default_value_t = 30)]
    days: u64,
    /// the fire log (default: the guard's own)
    #[arg(long)]
    log: Option<String>,
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
    let two_weeks = jiff::SignedDuration::from_hours(14 * 24);
    let then = jiff::Timestamp::now().checked_sub(two_weeks).unwrap_or_default();
    then.to_zoned(jiff::tz::TimeZone::UTC).strftime("%F").to_string()
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
        Cmd::Guard(a) => cmd_guard(a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_config::guard_checks::{Check, read_fires};

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

    #[test]
    fn guard_report_counts_fires_and_lists_quiet_checks() {
        let now = 1_800_000_000;
        let old = now - 40 * 86_400;
        let log = format!("{old} G2 deny\n{} G12 stop\n{} G12 stop\n{} H1 hint\n", now - 60, now - 50, now - 40);
        let gate = Gate {
            off: vec![Check::G11],
            presets: agent_config::presets::ALL.iter().map(ToString::to_string).collect(),
        };
        let r = guard_report(&read_fires(&log), now, 30, &gate);
        assert!(r.contains("| G12 | workflow | yes | 2 | stop 2 |"), "{r}");
        assert!(r.contains("| G2 | workflow | yes | 0 | - |"), "an old fire is outside the window: {r}");
        assert!(r.contains("| G11 | clock | no | 0 | - | never |"), "{r}");
        let quiet = r.lines().find(|l| l.starts_with("On and never fired")).unwrap();
        assert!(quiet.contains("G2") && !quiet.contains("G12") && !quiet.contains("H1"), "{quiet}");
        assert!(!quiet.contains("G11"), "a check that is off is not listed: {quiet}");
        assert!(!r.contains("The log starts"), "the log is older than the window: {r}");
        let empty = guard_report(&[], now, 30, &gate);
        assert!(empty.starts_with("The fire log is empty") && !empty.contains("never fired"), "{empty}");
        assert!(
            guard_report(&read_fires(&format!("{} G1 deny\n", now - 10)), now, 30, &gate).starts_with("The log starts")
        );
    }
}
