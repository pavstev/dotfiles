//! hurry-scan: find out why a Claude Code session is slow, and what to cut.
//! Read-only: it never changes anything.

use agent_config::pycompat::{basename, dirname, fname, json_str, list_dir, mtime, parse_ts, py_float, py_str, truthy};
use clap::Parser;
use regex::Regex;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use walkdir::WalkDir;

const DOC: &str = "hurry-scan: find out why a Claude Code session is slow, and what to cut.

Reads the session transcript (JSONL) plus the static weight Claude loads on
every turn (CLAUDE.md, skills, agents, MCP, hooks, plugins). Prints a short,
ranked report. Read-only: it never changes anything.

--brief prints at most 3 SPEED lines (score 60+) for whatsleft-scan and nothing else.";

static NULL: Value = Value::Null;

macro_rules! out {
    () => { let _ = writeln!(std::io::stdout()); };
    ($($a:tt)*) => { let _ = writeln!(std::io::stdout(), $($a)*); };
}

static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static NON_ALNUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9]").unwrap());
static MODEL5: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"claude-(opus|sonnet|fable)-5").unwrap());
static MCP_SERVER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^mcp__(.+?)__").unwrap());

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

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn pyround1(x: f64) -> f64 {
    format!("{:.1}", x).parse().unwrap_or(x)
}

/// json.dumps(v, indent=2) with ensure_ascii.
fn dumps_indent(v: &Value, level: usize, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
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
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad(level + 1));
                dumps_indent(x, level + 1, out);
            }
            out.push('\n');
            out.push_str(&pad(level));
            out.push(']');
        }
        Value::Object(o) => {
            if o.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad(level + 1));
                json_str(out, k);
                out.push_str(": ");
                dumps_indent(x, level + 1, out);
            }
            out.push('\n');
            out.push_str(&pad(level));
            out.push('}');
        }
    }
}

fn clen(s: &str) -> usize {
    s.chars().count()
}

/// open(path, errors="replace") with universal newlines.
fn read_text(p: &Path) -> String {
    match fs::read(p) {
        Ok(b) => String::from_utf8_lossy(&b).replace("\r\n", "\n").replace('\r', "\n"),
        Err(_) => String::new(),
    }
}

fn load_json(p: &Path) -> Value {
    match fs::read(p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        Some(v) if v.is_object() => v,
        _ => Value::Object(Map::new()),
    }
}

fn tok(chars: i64) -> i64 {
    chars / 4
}

fn human(n: f64) -> String {
    let mut n = n;
    for unit in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 || unit == "GB" {
            return if unit == "B" { format!("{:.0}{}", n, unit) } else { format!("{:.1}{}", n, unit) };
        }
        n /= 1024.0;
    }
    unreachable!()
}

fn ktok(n: i64) -> String {
    if n >= 1000 { format!("{:.0}k", n as f64 / 1000.0) } else { n.to_string() }
}

/// Recursive `**` glob: every non-hidden entry under `root` (symlinks followed) for which `pick` is true.
fn rglob(root: &Path, pick: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut v = vec![];
    for e in WalkDir::new(root)
        .follow_links(true)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| e.ok())
    {
        if pick(&e.file_name().to_string_lossy()) {
            v.push(e.path().to_path_buf());
        }
    }
    v
}

// ---------------------------------------------------------------------------
// 1. Find the session transcript
// ---------------------------------------------------------------------------

fn projects_dir() -> PathBuf {
    Path::new(&home()).join(".claude").join("projects")
}

fn session_hits(prefix: &str) -> Vec<PathBuf> {
    let mut hits = vec![];
    for d in list_dir(&projects_dir()) {
        for f in list_dir(&d) {
            let n = fname(&f);
            if n.starts_with(prefix) && n.ends_with(".jsonl") {
                hits.push(f);
            }
        }
    }
    hits
}

fn newest(v: Vec<PathBuf>) -> Option<PathBuf> {
    let mut best: Option<(f64, PathBuf)> = None;
    for p in v {
        let m = mtime(&p).unwrap_or(0.0);
        if best.as_ref().map(|(bm, _)| m > *bm).unwrap_or(true) {
            best = Some((m, p));
        }
    }
    best.map(|(_, p)| p)
}

fn cwd_string() -> String {
    std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn find_session(arg: Option<&str>, use_all: bool) -> Option<PathBuf> {
    if let Some(a) = arg.filter(|a| !a.is_empty()) {
        if Path::new(a).is_file() {
            return Some(PathBuf::from(a));
        }
        return session_hits(a).into_iter().next();
    }
    if let Ok(sid) = std::env::var("CLAUDE_CODE_SESSION_ID")
        && !sid.is_empty()
    {
        let want = format!("{}.jsonl", sid);
        let hit = session_hits(&sid).into_iter().find(|p| fname(p) == want);
        if hit.is_some() {
            return hit;
        }
    }
    if !use_all {
        let enc = NON_ALNUM.replace_all(&cwd_string(), "-").into_owned();
        let mine: Vec<PathBuf> =
            list_dir(&projects_dir().join(enc)).into_iter().filter(|p| fname(p).ends_with(".jsonl")).collect();
        if !mine.is_empty() {
            return newest(mine);
        }
    }
    let all = session_hits("");
    newest(all)
}

// ---------------------------------------------------------------------------
// 2. Read the transcript
// ---------------------------------------------------------------------------

fn result_size(content: &Value) -> usize {
    match content {
        Value::String(s) => clen(s),
        Value::Array(a) => a
            .iter()
            .map(|b| if b.is_object() { clen(get(b, "text").as_str().unwrap_or("")) } else { clen(&py_str(b)) })
            .sum(),
        other => clen(&py_str(other)),
    }
}

fn short_input(inp: &Value) -> String {
    if !inp.is_object() {
        return String::new();
    }
    for key in ["file_path", "command", "pattern", "url", "query", "description", "prompt"] {
        if let Some(v) = inp.get(key) {
            let s = WS.replace_all(&py_str(v), " ").into_owned();
            return s.chars().take(70).collect();
        }
    }
    String::new()
}

/// Insertion-ordered counter.
#[derive(Default, Clone)]
struct Counter {
    keys: Vec<String>,
    idx: HashMap<String, usize>,
    vals: Vec<i64>,
}

impl Counter {
    fn add(&mut self, k: &str, n: i64) {
        match self.idx.get(k) {
            Some(&i) => self.vals[i] += n,
            None => {
                self.idx.insert(k.to_string(), self.keys.len());
                self.keys.push(k.to_string());
                self.vals.push(n);
            }
        }
    }
    fn get(&self, k: &str) -> i64 {
        self.idx.get(k).map(|&i| self.vals[i]).unwrap_or(0)
    }
    fn most_common(&self, n: usize) -> Vec<(String, i64)> {
        let mut it: Vec<(String, i64)> = self.keys.iter().cloned().zip(self.vals.iter().cloned()).collect();
        it.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        it.truncate(n);
        it
    }
    fn total(&self) -> i64 {
        self.vals.iter().sum()
    }
    fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

struct Transcript {
    path: String,
    bytes: u64,
    turns_assistant: usize,
    sidechain: usize,
    model: Option<String>,
    ctx_now: i64,
    ctx_max: i64,
    compactions: Vec<Option<f64>>,
    tool_calls: Counter,
    tool_bytes: Counter,
    results: Vec<(usize, String, String)>,
    slow: Vec<(f64, String, String)>,
    reads: Counter,
    think_ms: f64,
    errors: usize,
    first_ts: Option<f64>,
    last_ts: Option<f64>,
    cwd: Option<String>,
    attach_by_type: Counter,
    mcp_tools: HashSet<String>,
}

fn scan_transcript(path: &Path) -> Transcript {
    let mut s = Transcript {
        path: path.to_string_lossy().into_owned(),
        bytes: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        turns_assistant: 0,
        sidechain: 0,
        model: None,
        ctx_now: 0,
        ctx_max: 0,
        compactions: vec![],
        tool_calls: Counter::default(),
        tool_bytes: Counter::default(),
        results: vec![],
        slow: vec![],
        reads: Counter::default(),
        think_ms: 0.0,
        errors: 0,
        first_ts: None,
        last_ts: None,
        cwd: None,
        attach_by_type: Counter::default(),
        mcp_tools: HashSet::new(),
    };
    let mut pending: HashMap<String, (String, String, Option<f64>)> = HashMap::new();
    let text = read_text(path);
    for raw in text.split_inclusive('\n') {
        let Ok(o) = serde_json::from_str::<Value>(raw) else { continue };
        if !o.is_object() {
            continue;
        }
        let t = get(&o, "type").as_str().unwrap_or("");
        let ts = if truthy(get(&o, "timestamp")) { get(&o, "timestamp").as_str().and_then(parse_ts) } else { None };
        if let Some(ts) = ts {
            if !s.first_ts.map(|f| f != 0.0).unwrap_or(false) {
                s.first_ts = Some(ts);
            }
            s.last_ts = Some(ts);
        }
        if s.cwd.as_deref().map(|c| c.is_empty()).unwrap_or(true)
            && let Some(c) = get(&o, "cwd").as_str()
        {
            s.cwd = Some(c.to_string());
        }
        if truthy(get(&o, "isSidechain")) {
            s.sidechain += 1;
        }
        if t == "system" && get(&o, "subtype").as_str() == Some("compact_boundary") {
            let meta = get(&o, "compactMetadata");
            let pre = if truthy(get(meta, "preTokens")) { Some(num(get(meta, "preTokens"))) } else { None };
            s.compactions.push(pre);
        } else if t == "attachment" {
            let att = get(&o, "attachment");
            let ty = match get(att, "type") {
                Value::String(x) => x.clone(),
                Value::Null => "?".into(),
                other => other.to_string(),
            };
            s.attach_by_type.add(&ty, clen(raw) as i64);
            if let Some(names) = get(att, "addedNames").as_array() {
                for n in names {
                    if let Some(n) = n.as_str()
                        && n.starts_with("mcp__")
                    {
                        s.mcp_tools.insert(n.to_string());
                    }
                }
            }
        } else if t == "assistant" {
            let msg = get(&o, "message");
            s.turns_assistant += 1;
            s.think_ms += num(get(&o, "thinkingDurationMs"));
            if let Some(m) = get(msg, "model").as_str().filter(|m| !m.is_empty()) {
                s.model = Some(m.to_string());
            }
            let u = get(msg, "usage");
            let ctx = num(get(u, "input_tokens")) as i64
                + num(get(u, "cache_read_input_tokens")) as i64
                + num(get(u, "cache_creation_input_tokens")) as i64;
            if ctx != 0 && !truthy(get(&o, "isSidechain")) {
                s.ctx_now = ctx;
                s.ctx_max = s.ctx_max.max(ctx);
            }
            if let Some(arr) = get(msg, "content").as_array() {
                for b in arr {
                    if !b.is_object() || get(b, "type").as_str() != Some("tool_use") {
                        continue;
                    }
                    let name = match get(b, "name") {
                        Value::Null => "?".to_string(),
                        Value::String(x) => x.clone(),
                        other => other.to_string(),
                    };
                    let inp = get(b, "input");
                    s.tool_calls.add(&name, 1);
                    pending.insert(get(b, "id").to_string(), (name.clone(), short_input(inp), ts));
                    if name == "Read" && inp.is_object() && truthy(get(inp, "file_path")) {
                        s.reads.add(&py_str(get(inp, "file_path")), 1);
                    }
                }
            }
        } else if t == "user"
            && let Some(arr) = get(get(&o, "message"), "content").as_array()
        {
            for b in arr {
                if !b.is_object() {
                    continue;
                }
                if let Some("tool_result") = get(b, "type").as_str() {
                    let (name, summ, t0) =
                        pending.remove(&get(b, "tool_use_id").to_string()).unwrap_or(("?".into(), String::new(), None));
                    let size = result_size(get(b, "content"));
                    s.tool_bytes.add(&name, size as i64);
                    s.results.push((size, name.clone(), summ.clone()));
                    if truthy(get(b, "is_error")) {
                        s.errors += 1;
                    }
                    if let (Some(t0), Some(ts)) = (t0.filter(|x| *x != 0.0), ts.filter(|x| *x != 0.0))
                        && ts - t0 > 20.0
                    {
                        s.slow.push((ts - t0, name, summ));
                    }
                }
            }
        }
    }
    s.results.sort_by(|a, b| b.cmp(a));
    s.slow.sort_by(|a, b| {
        b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal).then_with(|| b.1.cmp(&a.1)).then_with(|| b.2.cmp(&a.2))
    });
    s
}

// ---------------------------------------------------------------------------
// 3. Static weight: what loads on every turn
// ---------------------------------------------------------------------------

fn claude_dir() -> PathBuf {
    Path::new(&home()).join(".claude")
}

fn realpath(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn claude_md_files(cwd: &str) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut found: Vec<PathBuf> = vec![];
    let g = claude_dir().join("CLAUDE.md");
    if g.exists() {
        found.push(g);
    }
    let mut d = if cwd.is_empty() { cwd_string() } else { cwd.to_string() };
    let mut seen: HashSet<String> = HashSet::new();
    let h = home();
    while !d.is_empty() && d != "/" && !seen.contains(&d) {
        seen.insert(d.clone());
        for name in ["CLAUDE.md", "AGENTS.md", ".claude/CLAUDE.md"] {
            let p = Path::new(&d).join(name);
            if p.exists() {
                let rp = realpath(&p);
                if !found.iter().any(|x| realpath(x) == rp) {
                    found.push(p);
                }
            }
        }
        if Path::new(&d).join(".git").is_dir() || d == h {
            break;
        }
        d = dirname(&d);
    }
    let rules = rglob(&claude_dir().join("rules"), |n| n.ends_with(".md"));
    (found, rules)
}

struct Weight {
    claude_md: Vec<(PathBuf, usize)>,
    rules: (usize, usize),
    hooks: Vec<(String, i64)>,
    plugins_on: Vec<String>,
    mcp: Vec<String>,
    output_style: Value,
    effort_env: Option<String>,
}

impl Weight {
    fn hook(&self, ev: &str) -> i64 {
        self.hooks.iter().find(|(k, _)| k == ev).map(|(_, v)| *v).unwrap_or(0)
    }
    fn hook_total(&self) -> i64 {
        self.hooks.iter().map(|(_, v)| *v).sum()
    }
}

fn static_weight(cwd: &Option<String>) -> Weight {
    let cwd_s = cwd.clone().unwrap_or_default();
    let (files, rules) = claude_md_files(&cwd_s);
    let claude_md: Vec<(PathBuf, usize)> = files.iter().map(|p| (p.clone(), clen(&read_text(p)))).collect();
    let rules_w = (rules.len(), rules.iter().map(|p| clen(&read_text(p))).sum());
    let base = if cwd_s.is_empty() { cwd_string() } else { cwd_s.clone() };
    let settings = load_json(&claude_dir().join("settings.json"));
    let local = load_json(&Path::new(&base).join(".claude").join("settings.json"));
    let mut hooks: Vec<(String, i64)> = vec![];
    for src in [&settings, &local] {
        if let Some(o) = get(src, "hooks").as_object() {
            for (ev, groups) in o {
                let mut n = 0i64;
                if let Some(arr) = groups.as_array() {
                    for g in arr {
                        if g.is_object() {
                            n += get(g, "hooks").as_array().map(|a| a.len() as i64).unwrap_or(0);
                        }
                    }
                }
                match hooks.iter_mut().find(|(k, _)| k == ev) {
                    Some((_, v)) => *v += n,
                    None => hooks.push((ev.clone(), n)),
                }
            }
        }
    }
    let mut plugins: Map<String, Value> = Map::new();
    for src in [&settings, &local] {
        if let Some(o) = get(src, "enabledPlugins").as_object() {
            for (k, v) in o {
                plugins.insert(k.clone(), v.clone());
            }
        }
    }
    let mut plugins_on: Vec<String> = plugins.iter().filter(|(_, v)| truthy(v)).map(|(k, _)| k.clone()).collect();
    plugins_on.sort();
    let cj = load_json(&Path::new(&home()).join(".claude.json"));
    let mut mcp: Vec<String> = vec![];
    let mut add_keys = |v: &Value| {
        if let Some(o) = v.as_object() {
            for k in o.keys() {
                if !mcp.contains(k) {
                    mcp.push(k.clone());
                }
            }
        }
    };
    add_keys(get(&cj, "mcpServers"));
    add_keys(get(get(get(&cj, "projects"), &base), "mcpServers"));
    add_keys(get(&load_json(&Path::new(&base).join(".mcp.json")), "mcpServers"));
    mcp.sort();
    Weight {
        claude_md,
        rules: rules_w,
        hooks,
        plugins_on,
        mcp,
        output_style: get(&settings, "outputStyle").clone(),
        effort_env: std::env::var("CLAUDE_EFFORT").ok().filter(|s| !s.is_empty()),
    }
}

/// Python `s.split(None, 2)` when it yields exactly three parts.
fn split_none2(s: &str) -> Option<(&str, &str, &str)> {
    let s = s.trim_start();
    let i = s.find(char::is_whitespace)?;
    let (a, rest) = s.split_at(i);
    let rest = rest.trim_start();
    let j = rest.find(char::is_whitespace)?;
    let (b, rest2) = rest.split_at(j);
    let c = rest2.trim_start();
    if c.is_empty() { None } else { Some((a, b, c)) }
}

struct Load {
    load: Option<f64>,
    cpus: usize,
    top: Vec<(f64, i64, String)>,
}

#[cfg(unix)]
unsafe extern "C" {
    fn getloadavg(loadavg: *mut f64, nelem: i32) -> i32;
}

fn machine_load() -> Load {
    let mut out =
        Load { load: None, cpus: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1), top: vec![] };
    #[cfg(unix)]
    {
        let mut l = [0f64; 3];
        // SAFETY: l holds 3 doubles, the call writes at most 1.
        let n = unsafe { getloadavg(l.as_mut_ptr(), 1) };
        if n >= 1 {
            out.load = Some(l[0]);
        }
    }
    if let Ok(o) = Command::new("ps").args(["-Ao", "pcpu,rss,comm"]).output() {
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        let mut rows: Vec<(f64, i64, String)> = vec![];
        let mut ok = true;
        for r in text.lines().skip(1) {
            if let Some((a, b, c)) = split_none2(r) {
                match (a.parse::<f64>(), b.parse::<i64>()) {
                    (Ok(cpu), Ok(rss)) => rows.push((cpu, rss.div_euclid(1024), basename(c))),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if ok {
            rows.sort_by(|x, y| {
                y.0.partial_cmp(&x.0).unwrap_or(Ordering::Equal).then_with(|| y.1.cmp(&x.1)).then_with(|| y.2.cmp(&x.2))
            });
            rows.truncate(3);
            out.top = rows;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 4. Judge: turn numbers into ranked findings
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Finding {
    score: i64,
    who: &'static str,
    title: String,
    why: String,
    fix: String,
}

fn guess_window(s: &Transcript) -> i64 {
    let model = s.model.clone().unwrap_or_default();
    if model.contains("[1m]") || MODEL5.is_match(&model) {
        return 1_000_000;
    }
    if s.ctx_max > 200_000 || s.compactions.iter().any(|c| c.unwrap_or(0.0) > 200_000.0) {
        return 1_000_000;
    }
    200_000
}

fn plural(n: usize) -> &'static str {
    if n != 1 { "s" } else { "" }
}

fn judge(s: &Transcript, w: &Weight, m: &Load, window: i64) -> Vec<Finding> {
    let mut f: Vec<Finding> = vec![];
    let mut add = |score: i64, who: &'static str, title: String, why: String, fix: &str| {
        f.push(Finding { score, who, title, why, fix: fix.to_string() });
    };
    let pct = if window != 0 { 100.0 * s.ctx_now as f64 / window as f64 } else { 0.0 };
    if pct >= 75.0 {
        add(
            100,
            "you",
            format!("Context is {:.0}% full", pct),
            "Every turn re-reads the whole history. Replies get slow and fuzzy.".into(),
            "Run /compact now. Or /clear and paste a short handoff.",
        );
    } else if pct >= 50.0 {
        add(
            70,
            "you",
            format!("Context is {:.0}% full", pct),
            "Each turn costs more as the history grows.".into(),
            "Run /compact at the next calm moment.",
        );
    }
    let big: Vec<&(usize, String, String)> = s.results.iter().filter(|r| r.0 >= 20000).collect();
    if !big.is_empty() {
        let total: usize = big.iter().map(|r| r.0).sum();
        let top = big[0];
        add(
            if total > 200000 { 80 } else { 55 },
            "me",
            format!("{} huge tool result{} (~{} tokens)", big.len(), plural(big.len()), ktok(tok(total as i64))),
            format!("Biggest: {} {} ({}). They stay in context forever.", top.1, human(top.0 as f64), top.2),
            "I will use offset/limit, rg, head, and subagents for big output.",
        );
    }
    let rereads: Vec<(String, i64)> =
        s.reads.keys.iter().zip(&s.reads.vals).filter(|(_, n)| **n >= 3).map(|(p, n)| (p.clone(), *n)).collect();
    if !rereads.is_empty() {
        // max(rereads, key=count): first of the maximal
        let mut best = &rereads[0];
        for r in &rereads {
            if r.1 > best.1 {
                best = r;
            }
        }
        add(
            50,
            "me",
            format!("{} file{} read 3+ times", rereads.len(), plural(rereads.len())),
            format!("Worst: {} read {} times.", basename(&best.0), best.1),
            "I will read once and keep notes. No more re-reads.",
        );
    }
    if !s.slow.is_empty() {
        let (d, name, summ) = &s.slow[0];
        add(
            if *d > 120.0 { 65 } else { 45 },
            "me",
            format!("{} slow tool call{} (over 20s)", s.slow.len(), plural(s.slow.len())),
            format!("Slowest: {} {:.0}s ({}).", name, d, summ),
            "I will run slow work in the background and add timeouts.",
        );
    }
    if s.compactions.len() >= 2 {
        add(
            60,
            "you",
            format!("Compacted {} times", s.compactions.len()),
            "Session is too long. Each compact loses detail.".into(),
            "Start a fresh session for the next task. Keep the handoff short.",
        );
    }
    if s.bytes > 15 * 1024 * 1024 {
        add(
            40,
            "you",
            format!("Transcript is {}", human(s.bytes as f64)),
            "Resume and startup get slow on huge sessions.".into(),
            "Finish this task, then start a new session.",
        );
    }
    let cmd_total: usize = w.claude_md.iter().map(|(_, n)| *n).sum::<usize>() + w.rules.1;
    if cmd_total > 24000 {
        add(
            if cmd_total > 48000 { 60 } else { 45 },
            "you",
            format!("CLAUDE.md + rules = ~{} tokens", ktok(tok(cmd_total as i64))),
            "Loaded on every turn, even when not needed.".into(),
            "Trim or move rarely used parts into skills.",
        );
    }
    let ab = &s.attach_by_type;
    let servers: HashSet<String> =
        s.mcp_tools.iter().filter_map(|n| MCP_SERVER.captures(n).map(|c| c[1].to_string())).collect();
    let mcp_bytes = ab.get("deferred_tools_delta") + ab.get("mcp_instructions_delta");
    if s.mcp_tools.len() >= 100 || mcp_bytes > 60000 {
        add(
            65,
            "you",
            format!(
                "{} MCP servers, {} tools (~{} tokens injected)",
                servers.len(),
                s.mcp_tools.len(),
                ktok(tok(mcp_bytes))
            ),
            "Tool names and server rules are sent on every session start and cache miss.".into(),
            "Disable MCP servers this project does not use.",
        );
    } else if w.mcp.len() >= 6 {
        add(
            55,
            "you",
            format!("{} MCP servers connected", w.mcp.len()),
            "Each server adds tool schemas to every turn.".into(),
            "Disable servers you do not use in this project.",
        );
    }
    if ab.get("skill_listing") > 40000 {
        add(
            60,
            "you",
            format!("Skill list injects ~{} tokens", ktok(tok(ab.get("skill_listing")))),
            "Every skill name and description is sent each session.".into(),
            "Disable plugins with many skills. Remove skills you never use.",
        );
    }
    if ab.get("hook_additional_context") > 8000 {
        add(
            45,
            "you",
            format!("Start hooks inject ~{} tokens", ktok(tok(ab.get("hook_additional_context")))),
            "A SessionStart hook pastes big text into every session.".into(),
            "Shorten what the hook prints.",
        );
    }
    if w.plugins_on.len() >= 8 {
        add(
            45,
            "you",
            format!("{} plugins enabled", w.plugins_on.len()),
            "Each plugin adds skills, agents, and hooks.".into(),
            "Turn off plugins you rarely use. It is reversible.",
        );
    }
    let pre = w.hook("PreToolUse") + w.hook("PostToolUse");
    if pre >= 5 {
        add(
            50,
            "you",
            format!("{} tool hooks run on every tool call", pre),
            "Each hook adds a process spawn before and after each tool.".into(),
            "Check they are fast. Drop the ones you do not need.",
        );
    }
    if let Some(e) = &w.effort_env
        && ["high", "xhigh", "max"].contains(&e.as_str())
        && s.tool_calls.get("Read") + s.tool_calls.get("Bash") > 0
    {
        add(
            35,
            "you",
            format!("Effort is {}", e),
            "High effort means long thinking before each step.".into(),
            "Use /effort medium for simple work. Try /fast for quick tasks.",
        );
    }
    if s.turns_assistant > 0 && s.think_ms / 1000.0 > 600.0 {
        add(
            40,
            "you",
            format!("{:.0} min spent thinking", s.think_ms / 60000.0),
            "Long thinking slows every step.".into(),
            "Lower effort for routine edits.",
        );
    }
    if s.errors >= 8 {
        add(
            40,
            "me",
            format!("{} tool errors", s.errors),
            "Retry loops waste turns.".into(),
            "I will stop, read the error once, and change the plan.",
        );
    }
    if let Some(load) = m.load.filter(|l| *l != 0.0)
        && load > m.cpus as f64 * 0.8
    {
        let top = m.top.iter().map(|(c, _, n)| format!("{} {:.0}%", n, c)).collect::<Vec<_>>().join(", ");
        add(
            50,
            "you",
            format!("This machine is busy (load {:.1} on {} cores)", load, m.cpus),
            format!("Top: {}.", top),
            "Quit the heavy app. Run ps:bag-mode to see more.",
        );
    }
    f.sort_by(|a, b| b.cmp(a));
    f
}

fn verdict(f: &[Finding]) -> &'static str {
    let top = f.first().map(|x| x.score).unwrap_or(0);
    if top >= 80 {
        "HIGH"
    } else if top >= 50 {
        "MEDIUM"
    } else {
        "LOW"
    }
}

// ---------------------------------------------------------------------------
// 5. Report
// ---------------------------------------------------------------------------

fn report(s: &Transcript, w: &Weight, m: &Load, window: i64, f: &[Finding], as_json: bool) {
    let _ = m;
    let pct = if window != 0 { 100.0 * s.ctx_now as f64 / window as f64 } else { 0.0 };
    if as_json {
        let mut o = Map::new();
        o.insert("session".into(), Value::from(s.path.clone()));
        o.insert("verdict".into(), Value::from(verdict(f)));
        o.insert("context_tokens".into(), Value::from(s.ctx_now));
        o.insert("window".into(), Value::from(window));
        o.insert(
            "context_pct".into(),
            serde_json::Number::from_f64(pyround1(pct)).map(Value::Number).unwrap_or(Value::Null),
        );
        let fs_: Vec<Value> = f
            .iter()
            .map(|x| {
                let mut e = Map::new();
                e.insert("score".into(), Value::from(x.score));
                e.insert("who".into(), Value::from(x.who));
                e.insert("title".into(), Value::from(x.title.clone()));
                e.insert("why".into(), Value::from(x.why.clone()));
                e.insert("fix".into(), Value::from(x.fix.clone()));
                Value::Object(e)
            })
            .collect();
        o.insert("findings".into(), Value::Array(fs_));
        let mut buf = String::new();
        dumps_indent(&Value::Object(o), 0, &mut buf);
        out!("{}", buf);
        return;
    }
    let hashes = (pct / 5.0) as i64;
    let bar = format!("{}{}", "#".repeat(hashes.max(0) as usize), ".".repeat((20 - hashes).max(0) as usize));
    let mut age = String::new();
    if let (Some(a), Some(b)) = (s.first_ts, s.last_ts)
        && a != 0.0
        && b != 0.0
    {
        age = format!(", {:.1}h old", (b - a) / 3600.0);
    }
    out!("SESSION  {}  ({}{})", basename(&s.path), human(s.bytes as f64), age);
    let style = match &w.output_style {
        v if truthy(v) => py_str(v),
        _ => "default".to_string(),
    };
    out!(
        "MODEL    {}   effort: {}   style: {}",
        s.model.clone().unwrap_or_else(|| "?".into()),
        w.effort_env.clone().unwrap_or_else(|| "?".into()),
        style
    );
    out!("CONTEXT  [{}] {:.0}%  {} of {} tokens", bar, pct, ktok(s.ctx_now), ktok(window));
    out!(
        "TURNS    {} replies, {} tool calls, {} compactions, {} subagent lines",
        s.turns_assistant,
        s.tool_calls.total(),
        s.compactions.len(),
        s.sidechain
    );
    if !s.tool_bytes.is_empty() {
        let parts = s
            .tool_bytes
            .most_common(4)
            .iter()
            .map(|(n, b)| format!("{} {}", n, human(*b as f64)))
            .collect::<Vec<_>>()
            .join(", ");
        out!("RESULTS  top tools by output: {}", parts);
    }
    if !s.attach_by_type.is_empty() {
        let top = s
            .attach_by_type
            .most_common(4)
            .iter()
            .map(|(k, v)| format!("{} ~{}t", k, ktok(tok(*v))))
            .collect::<Vec<_>>()
            .join(", ");
        out!("STARTUP  injected: {}", top);
    }
    let cmd_total: usize = w.claude_md.iter().map(|(_, n)| *n).sum::<usize>() + w.rules.1;
    out!(
        "STATIC   CLAUDE.md+rules ~{}t | {} plugins on | {} hooks | {} MCP tools",
        ktok(tok(cmd_total as i64)),
        w.plugins_on.len(),
        w.hook_total(),
        s.mcp_tools.len()
    );
    out!();
    out!("VERDICT  {}", verdict(f));
    if f.is_empty() {
        out!("Nothing slow found. The wait is the model, not the setup. Try /fast.");
        return;
    }
    out!();
    for (i, x) in f.iter().take(7).enumerate() {
        out!("{}. [{}] {}", i + 1, x.who.to_uppercase(), x.title);
        out!("   why: {}", x.why);
        out!("   fix: {}", x.fix);
    }
    out!();
    out!("[ME] = Claude can do it now. [YOU] = only you can: slash commands and settings.");
}

#[derive(Parser, Debug)]
#[command(name = "hurry_scan", about = DOC, infer_long_args = true)]
struct Cli {
    /// session id (or prefix) or path to a .jsonl file
    #[arg(long)]
    session: Option<String>,
    /// pick the newest session of any project
    #[arg(long)]
    all: bool,
    /// context window in tokens (default: guessed)
    #[arg(long, allow_negative_numbers = true)]
    window: Option<i64>,
    /// machine-readable output
    #[arg(long)]
    json: bool,
    /// 3 SPEED lines for whatsleft-scan, nothing else
    #[arg(long)]
    brief: bool,
}

fn main() {
    let a = Cli::parse();
    let Some(path) = find_session(a.session.as_deref(), a.all) else {
        eprintln!("No session transcript found under ~/.claude/projects.");
        std::process::exit(1);
    };
    let s = scan_transcript(&path);
    let window = match a.window {
        Some(w) if w != 0 => w,
        _ => guess_window(&s),
    };
    let w = static_weight(&s.cwd);
    let m = machine_load();
    let f = judge(&s, &w, &m, window);
    if a.brief {
        for x in f.iter().filter(|x| x.score >= 60).take(3) {
            out!("  SPEED [{}] {} — {}", x.who.to_uppercase(), x.title, x.fix);
        }
        return;
    }
    report(&s, &w, &m, window, &f, a.json);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_and_ktok() {
        assert_eq!(human(512.0), "512B");
        assert_eq!(human(2048.0), "2.0KB");
        assert_eq!(human(5.0 * 1024.0 * 1024.0), "5.0MB");
        assert_eq!(ktok(999), "999");
        assert_eq!(ktok(12345), "12k");
        assert_eq!(tok(4001), 1000);
    }

    #[test]
    fn timestamps_parse() {
        assert_eq!(parse_ts("1970-01-01T00:00:10Z"), Some(10.0));
        assert_eq!(parse_ts("1970-01-01T01:00:10+01:00"), Some(10.0));
        assert_eq!(parse_ts("1970-01-01T00:00:10.5Z"), Some(10.5));
        assert_eq!(parse_ts("garbage"), None);
    }

    #[test]
    fn short_input_collapses_space() {
        let v: Value = serde_json::from_str(r#"{"command": "ls   -la\n/tmp"}"#).unwrap();
        assert_eq!(short_input(&v), "ls -la /tmp");
    }

    #[test]
    fn findings_sort_descending_by_tuple() {
        let mk = |s, w, t: &str| Finding { score: s, who: w, title: t.into(), why: String::new(), fix: String::new() };
        let mut v = [mk(50, "me", "a"), mk(50, "you", "a"), mk(70, "me", "z")];
        v.sort_by(|a, b| b.cmp(a));
        assert_eq!(v[0].score, 70);
        assert_eq!(v[1].who, "you");
    }

    #[test]
    fn split_none2_like_python() {
        assert_eq!(split_none2("  12.5   345 /usr/bin/x y"), Some(("12.5", "345", "/usr/bin/x y")));
        assert_eq!(split_none2("1 2"), None);
    }
}
