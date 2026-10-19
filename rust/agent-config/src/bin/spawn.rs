//! spawn: start a new Claude Code session with Remote Control on, in a detached tmux.
//!
//! Port of claude/skills/ps_spawn/scripts/spawn.py. Same checks in the same
//! order, same lines on stdout, same exit codes (0 started, 2 bad args, 3 a check
//! failed, 4 the session stopped, 5 no link in time), and the same argparse
//! errors on stderr.
//!
//! Where Python would crash with a traceback (a prompt file that is not UTF-8, a
//! prompt file that cannot be written) this prints a FAIL line instead.
//! Known gaps: `~user` is not expanded, Unicode digits are not read by `--wait`,
//! and the help text is the Python 3.9 one at 80 columns.

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SOCKET: &str = "claude-spawn"; // own tmux server, started from a clean env
const MODES: &[&str] = &["bypassPermissions", "acceptEdits", "auto", "default", "dontAsk", "plan"];
const EXIT_MARK: &str = "CLAUDE_EXITED";
const URL_PATTERN: &str = r"https://claude\.ai/code/session_[A-Za-z0-9]+";

type EnvMap = HashMap<String, String>;

const PROG: &str = "spawn.py";
const USAGE: &str = "usage: spawn.py [-h] (--prompt-file PROMPT_FILE | --prompt PROMPT) [--dir DIR]
                [--name NAME]
                [--mode {bypassPermissions,acceptEdits,auto,default,dontAsk,plan}]
                [--wait WAIT] [--dry-run]
";
const HELP_BODY: &str = "
spawn.py: start a new Claude Code session with Remote Control on, in a
detached tmux.

optional arguments:
  -h, --help            show this help message and exit
  --prompt-file PROMPT_FILE
                        file with the prompt for the new session
  --prompt PROMPT       the prompt itself (prefer --prompt-file)
  --dir DIR             folder for the new session (default: current)
  --name NAME           session name on the phone and in tmux (default: from
                        the prompt)
  --mode {bypassPermissions,acceptEdits,auto,default,dontAsk,plan}
                        permission mode
  --wait WAIT           seconds to wait for the Remote Control link
  --dry-run             run the checks, print the command, start nothing
";

// ---------------------------------------------------------------------------
// Python string helpers.
// ---------------------------------------------------------------------------

/// Python's str.isspace for one char (Unicode White_Space plus \x1c-\x1f).
fn py_is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

fn py_strip(s: &str) -> &str {
    s.trim_matches(py_is_space)
}

/// " ".join(s.split())
fn py_flat(s: &str) -> String {
    s.split(py_is_space).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// str.splitlines(): splits on more than \n.
fn py_splitlines(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            '\n' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}' => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn is_printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    if c.is_control() || c.is_whitespace() {
        return false;
    }
    let u = c as u32;
    !(matches!(u, 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E)
        || (0x200B..=0x200F).contains(&u)
        || (0x202A..=0x202E).contains(&u)
        || (0x2060..=0x206F).contains(&u)
        || u == 0xFEFF
        || (0xFFF9..=0xFFFB).contains(&u)
        || (0xE000..=0xF8FF).contains(&u)
        || u >= 0xF0000
        || (u & 0xFFFE) == 0xFFFE
        || (0xFDD0..=0xFDEF).contains(&u))
}

/// Python's repr() of a str.
fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if is_printable(c) => out.push(c),
            c => {
                let u = c as u32;
                if u < 0x100 {
                    out.push_str(&format!("\\x{:02x}", u));
                } else if u < 0x10000 {
                    out.push_str(&format!("\\u{:04x}", u));
                } else {
                    out.push_str(&format!("\\U{:08x}", u));
                }
            }
        }
    }
    out.push(quote);
    out
}

/// Python's OSError text: `[Errno 2] No such file or directory: 'path'`.
fn os_error_text(e: &std::io::Error, path: &str) -> String {
    let text = e.to_string();
    let msg = match text.rfind(" (os error") {
        Some(i) => &text[..i],
        None => &text,
    };
    format!("[Errno {}] {}: {}", e.raw_os_error().unwrap_or(0), msg, repr_str(path))
}

/// shlex.quote
fn shlex_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

// ---------------------------------------------------------------------------
// Paths the way Python's os.path sees them.
// ---------------------------------------------------------------------------

fn py_join(a: &str, b: &str) -> String {
    if b.starts_with('/') || a.is_empty() {
        b.to_string()
    } else if a.ends_with('/') {
        format!("{}{}", a, b)
    } else {
        format!("{}/{}", a, b)
    }
}

fn py_dirname(p: &str) -> String {
    let head = match p.rfind('/') {
        Some(i) => &p[..=i],
        None => "",
    };
    if !head.is_empty() && head.chars().any(|c| c != '/') {
        head.trim_end_matches('/').to_string()
    } else {
        head.to_string()
    }
}

fn home_from(env: &EnvMap) -> String {
    match env.get("HOME") {
        Some(h) => h.clone(),
        None => dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

/// os.path.expanduser for `~` and `~/...`.
fn expanduser(p: &str, env: &EnvMap) -> String {
    if !p.starts_with('~') {
        return p.to_string();
    }
    let i = p.find('/').unwrap_or(p.len());
    if i != 1 {
        return p.to_string(); // ~user: left alone
    }
    let home = home_from(env);
    let home = home.trim_end_matches('/');
    let out = format!("{}{}", home, &p[i..]);
    if out.is_empty() { "/".to_string() } else { out }
}

/// os.path.realpath: absolute, symlinks resolved as far as they exist.
fn realpath(p: &str, cwd: &str) -> String {
    let abs = if p.starts_with('/') { p.to_string() } else { py_join(cwd, p) };
    let mut result: Vec<String> = Vec::new();
    let mut todo: std::collections::VecDeque<String> = abs.split('/').map(String::from).collect();
    let mut hops = 0;
    while let Some(c) = todo.pop_front() {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            result.pop();
            continue;
        }
        result.push(c);
        let cur = format!("/{}", result.join("/"));
        if let Ok(md) = std::fs::symlink_metadata(&cur)
            && md.file_type().is_symlink()
            && hops < 64
            && let Ok(target) = std::fs::read_link(&cur)
        {
            hops += 1;
            result.pop();
            let t = target.to_string_lossy().into_owned();
            if t.starts_with('/') {
                result.clear();
            }
            for part in t.split('/').rev() {
                todo.push_front(part.to_string());
            }
        }
    }
    format!("/{}", result.join("/"))
}

#[cfg(unix)]
fn is_executable(md: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_md: &std::fs::Metadata) -> bool {
    true
}

/// shutil.which
fn which_in(name: &str, env: &EnvMap) -> Option<String> {
    let path = env
        .get("PATH")
        .cloned()
        .unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".to_string());
    if path.is_empty() {
        return None;
    }
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut seen: Vec<&str> = Vec::new();
    for dir in path.split(sep) {
        if seen.contains(&dir) {
            continue;
        }
        seen.push(dir);
        let candidate = py_join(dir, name);
        if let Ok(md) = std::fs::metadata(&candidate)
            && !md.is_dir()
            && is_executable(&md)
        {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The checks.
// ---------------------------------------------------------------------------

fn config_paths(env: &EnvMap) -> (String, String) {
    match env.get("CLAUDE_CONFIG_DIR").filter(|s| !s.is_empty()) {
        Some(cfg) => (cfg.clone(), py_join(cfg, ".claude.json")),
        None => {
            let cfg = py_join(&home_from(env), ".claude");
            let global = py_join(&py_dirname(&cfg), ".claude.json");
            (cfg, global)
        }
    }
}

fn load_json(path: &str) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

/// Python truthiness of a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_none_or(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Env of the parent session that must not leak into the child.
fn clean_env(env: &EnvMap) -> EnvMap {
    env.iter()
        .filter(|(k, _)| {
            let drop = (k.starts_with("CLAUDE") && k.as_str() != "CLAUDE_CONFIG_DIR")
                || k.starts_with("ANTHROPIC_")
                || k.starts_with("MCP_");
            !drop
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn make_name(prompt: &str, stamp: &str) -> String {
    let re = Regex::new(r"[A-Za-z0-9]+").unwrap();
    let lower = prompt.to_lowercase();
    let mut words: Vec<&str> = re.find_iter(&lower).map(|m| m.as_str()).take(3).collect();
    if words.is_empty() {
        words.push("session");
    }
    words.push(stamp);
    words.join("-").chars().take(40).collect()
}

fn now_stamp() -> String {
    Command::new("date")
        .arg("+%H%M")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0000".to_string())
}

fn safe_name(name: &str) -> Result<String, String> {
    let re = Regex::new(r"[^A-Za-z0-9_-]+").unwrap();
    let out = re.replace_all(name, "-");
    let out: String = out.trim_matches('-').chars().take(40).collect();
    if out.is_empty() {
        return Err("name is empty after cleaning: use letters, digits, - or _".to_string());
    }
    Ok(out)
}

/// True when the folder or a parent has hasTrustDialogAccepted.
fn trusted(folder: &str, projects: &Value, cwd: &str) -> bool {
    let mut p = realpath(folder, cwd);
    loop {
        if truthy(projects.get(&p).and_then(|v| v.get("hasTrustDialogAccepted"))) {
            return true;
        }
        let parent = py_dirname(&p);
        if parent == p {
            return false;
        }
        p = parent;
    }
}

/// Returns (claude_path, tmux_path), or the message that says what to fix.
fn preflight(
    folder: &str,
    mode: &str,
    name: &str,
    env: &EnvMap,
    which: &dyn Fn(&str) -> Option<String>,
    tmux_has: &dyn Fn(&str, &str) -> bool,
) -> Result<(String, String), String> {
    let tmux = which("tmux").ok_or("tmux missing: run `brew install tmux` (or `dots`)")?;
    let claude = which("claude").ok_or("claude missing on PATH: install Claude Code")?;
    if !Path::new(folder).is_dir() {
        return Err(format!("folder does not exist: {}", folder));
    }
    let (cfg, global_json) = config_paths(env);
    let g = load_json(&global_json);
    if !truthy(g.get("oauthAccount")) {
        return Err("not logged in to claude.ai (Remote Control needs it): run `claude` and /login".into());
    }
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let empty = Value::Null;
    let projects = g.get("projects").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    if !trusted(folder, projects, &cwd) {
        return Err(format!("folder not trusted: run `claude` once in {} and accept", folder));
    }
    if mode == "bypassPermissions" {
        let settings = load_json(&py_join(&cfg, "settings.json"));
        if !(truthy(g.get("bypassPermissionsModeAccepted"))
            || truthy(settings.get("skipDangerousModePermissionPrompt")))
        {
            return Err("bypass warning never accepted on this machine: run \
`claude --permission-mode bypassPermissions` once in a terminal, accept, \
then /exit. Or pass --mode acceptEdits."
                .into());
        }
    }
    if tmux_has(&tmux, name) {
        return Err(format!("tmux session {} already exists: pick another --name", name));
    }
    Ok((claude, tmux))
}

/// The command tmux runs. The prompt is read from a file, so no quoting issues.
fn shell_command(claude: &str, name: &str, mode: &str, prompt_file: &str) -> String {
    format!(
        "{} --remote-control {} --name {} --permission-mode {} \"$(cat {})\"; echo {} $?; exec sleep 86400",
        shlex_quote(claude),
        shlex_quote(name),
        shlex_quote(name),
        shlex_quote(mode),
        shlex_quote(prompt_file),
        EXIT_MARK
    )
}

/// Reads the screen: ("ok", url) | ("fail", why) | ("wait", "").
fn classify(pane: &str, prompt: &str, folder: &str) -> (&'static str, String) {
    if pane.contains(EXIT_MARK) {
        return ("fail", "claude exited at start".to_string());
    }
    let url_re = Regex::new(URL_PATTERN).unwrap();
    let flat = py_flat(pane);
    let first: String = py_flat(prompt).chars().take(20).collect();
    let sent = !first.is_empty() && flat.contains(&first);
    // dialogs show before the prompt is sent; after that, the text may be the prompt's
    if !sent && !url_re.is_match(pane) {
        let dialogs: [(&str, String); 3] = [
            (
                r"(?i)trust (the files|this folder)",
                format!("folder not trusted: run `claude` once in {} and accept", folder),
            ),
            (
                r"(?i)Bypass Permissions mode",
                "bypass warning not accepted: run `claude --permission-mode bypassPermissions` once in a terminal and accept"
                    .to_string(),
            ),
            (
                r"(?i)Select login method",
                "not logged in to claude.ai: run `claude` and /login".to_string(),
            ),
        ];
        for (rx, why) in dialogs {
            if Regex::new(rx).unwrap().is_match(pane) {
                return ("fail", why);
            }
        }
    }
    // the link shows only once the session runs; a long prompt may show as "[Pasted text]"
    if let Some(m) = url_re.find(pane) {
        return ("ok", m.as_str().to_string());
    }
    ("wait", String::new())
}

// ---------------------------------------------------------------------------
// argparse, for this one parser: usage errors read like Python's.
// ---------------------------------------------------------------------------

enum ArgExit {
    Help,
    Error(String),
}

#[derive(Debug)]
struct Args {
    prompt_file: Option<String>,
    prompt: Option<String>,
    dir: Option<String>,
    name: Option<String>,
    mode: String,
    wait: i64,
    dry_run: bool,
}

// Registration order matters for "ambiguous option" messages.
const OPTS: &[(&str, usize)] = &[
    ("-h", 0),
    ("--help", 0),
    ("--prompt-file", 1),
    ("--prompt", 2),
    ("--dir", 3),
    ("--name", 4),
    ("--mode", 5),
    ("--wait", 6),
    ("--dry-run", 7),
];
const ACTION_NAMES: [&str; 8] =
    ["-h/--help", "--prompt-file", "--prompt", "--dir", "--name", "--mode", "--wait", "--dry-run"];

fn negative_number_like(a: &str) -> bool {
    // ^-\d+$|^-\d*\.\d+$
    let Some(r) = a.strip_prefix('-') else { return false };
    if !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    match r.split_once('.') {
        Some((a, b)) => {
            a.chars().all(|c| c.is_ascii_digit()) && !b.is_empty() && b.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// int(v) as Python reads it (ASCII digits). Saturates at the i64 range.
fn py_int(v: &str) -> Option<i64> {
    let t = py_strip(v);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    if !digits.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return None;
    }
    let clean = digits.replace('_', "");
    let n = clean.parse::<i64>().unwrap_or(i64::MAX);
    Some(if neg { -n } else { n })
}

enum Item {
    Pos,
    Dash,
    /// action index (None: unknown option), explicit argument
    Opt(Option<usize>, Option<String>),
}

fn classify_arg(a: &str) -> Result<Item, String> {
    let exact = |s: &str| OPTS.iter().find(|(n, _)| *n == s).map(|(_, i)| *i);
    if a.is_empty() || !a.starts_with('-') {
        return Ok(Item::Pos);
    }
    if let Some(i) = exact(a) {
        return Ok(Item::Opt(Some(i), None));
    }
    if a.chars().count() == 1 {
        return Ok(Item::Pos);
    }
    if let Some((o, v)) = a.split_once('=')
        && let Some(i) = exact(o)
    {
        return Ok(Item::Opt(Some(i), Some(v.to_string())));
    }
    let mut hits: Vec<(&str, usize, Option<String>)> = Vec::new();
    let second = a.chars().nth(1).unwrap();
    if second == '-' {
        let (prefix, explicit) = match a.split_once('=') {
            Some((p, v)) => (p, Some(v.to_string())),
            None => (a, None),
        };
        for (n, i) in OPTS {
            if n.starts_with(prefix) {
                hits.push((n, *i, explicit.clone()));
            }
        }
    } else {
        let short: String = a.chars().take(2).collect();
        let rest: String = a.chars().skip(2).collect();
        for (n, i) in OPTS {
            if *n == short {
                hits.push((n, *i, Some(rest.clone())));
            } else if n.starts_with(a) {
                hits.push((n, *i, None));
            }
        }
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|h| h.0).collect();
        return Err(format!("ambiguous option: {} could match {}", a, names.join(", ")));
    }
    if let Some((_, i, explicit)) = hits.pop() {
        return Ok(Item::Opt(Some(i), explicit));
    }
    if negative_number_like(a) || a.contains(' ') {
        return Ok(Item::Pos);
    }
    Ok(Item::Opt(None, None))
}

fn parse_args(argv: &[String]) -> Result<Args, ArgExit> {
    let mut items: Vec<Item> = Vec::new();
    let mut after_dd = false;
    for a in argv {
        if after_dd {
            items.push(Item::Pos);
        } else if a == "--" {
            items.push(Item::Dash);
            after_dd = true;
        } else {
            items.push(classify_arg(a).map_err(ArgExit::Error)?);
        }
    }
    let mut args = Args {
        prompt_file: None,
        prompt: None,
        dir: None,
        name: None,
        mode: "bypassPermissions".into(),
        wait: 60,
        dry_run: false,
    };
    let mut extras: Vec<String> = Vec::new();
    let n = argv.len();
    let mut i = 0;
    while i < n {
        match &items[i] {
            // no positionals here: every plain argument is an extra
            Item::Pos | Item::Dash => {
                extras.push(argv[i].clone());
                i += 1;
            }
            Item::Opt(None, _) => {
                extras.push(argv[i].clone());
                i += 1;
            }
            Item::Opt(Some(idx), explicit) => {
                let idx = *idx;
                let mut explicit = explicit.clone();
                let single_dash = !argv[i].starts_with("--");
                i += 1;
                let err = |m: String| ArgExit::Error(format!("argument {}: {}", ACTION_NAMES[idx], m));
                if idx == 0 || idx == 7 {
                    // flags: help exits now; explicit arguments are rejected
                    if let Some(mut ex) = explicit.take() {
                        if !(single_dash && idx == 0) {
                            return Err(err(format!("ignored explicit argument {}", repr_str(&ex))));
                        }
                        // -hX: X may be more short flags; -h is the only one
                        while !ex.is_empty() {
                            if !ex.starts_with('h') {
                                return Err(err(format!("ignored explicit argument {}", repr_str(&ex))));
                            }
                            ex = ex.chars().skip(1).collect();
                        }
                    }
                    if idx == 0 {
                        return Err(ArgExit::Help);
                    }
                    args.dry_run = true;
                    continue;
                }
                // value options: explicit, or the very next plain argument
                let value = if let Some(v) = explicit.take() {
                    v
                } else if i < n && matches!(items[i], Item::Pos) {
                    i += 1;
                    argv[i - 1].clone()
                } else {
                    return Err(err("expected one argument".into()));
                };
                match idx {
                    1 | 2 => {
                        // the two prompt sources exclude each other
                        let other = if idx == 1 { &args.prompt } else { &args.prompt_file };
                        if other.is_some() {
                            return Err(err(format!(
                                "not allowed with argument {}",
                                ACTION_NAMES[if idx == 1 { 2 } else { 1 }]
                            )));
                        }
                        if idx == 1 {
                            args.prompt_file = Some(value);
                        } else {
                            args.prompt = Some(value);
                        }
                    }
                    3 => args.dir = Some(value),
                    4 => args.name = Some(value),
                    5 => {
                        if !MODES.contains(&value.as_str()) {
                            let choices: Vec<String> = MODES.iter().map(|m| repr_str(m)).collect();
                            return Err(err(format!(
                                "invalid choice: {} (choose from {})",
                                repr_str(&value),
                                choices.join(", ")
                            )));
                        }
                        args.mode = value;
                    }
                    _ => {
                        args.wait = py_int(&value)
                            .ok_or_else(|| err(format!("invalid int value: {}", repr_str(&value))))?;
                    }
                }
            }
        }
    }
    if args.prompt_file.is_none() && args.prompt.is_none() {
        return Err(ArgExit::Error("one of the arguments --prompt-file --prompt is required".into()));
    }
    if !extras.is_empty() {
        return Err(ArgExit::Error(format!("unrecognized arguments: {}", extras.join(" "))));
    }
    Ok(args)
}

// ---------------------------------------------------------------------------
// The run.
// ---------------------------------------------------------------------------

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Outcome {
    fn new() -> Self {
        Outcome { code: 0, stdout: String::new(), stderr: String::new() }
    }
    fn say(&mut self, line: impl AsRef<str>) {
        self.stdout.push_str(line.as_ref());
        self.stdout.push('\n');
    }
    fn end(mut self, code: i32) -> Self {
        self.code = code;
        self
    }
}

fn tmux_command(tmux: &str, env: &EnvMap, args: &[&str]) -> Command {
    let mut c = Command::new(tmux);
    c.args(args).env_clear().envs(env);
    c
}

fn run(argv: &[String], env: &EnvMap, cwd: &str) -> Outcome {
    let mut o = Outcome::new();
    let a = match parse_args(argv) {
        Ok(a) => a,
        Err(ArgExit::Help) => {
            o.stdout = format!("{}{}", USAGE, HELP_BODY);
            return o;
        }
        Err(ArgExit::Error(m)) => {
            o.stderr = format!("{}{}: error: {}\n", USAGE, PROG, m);
            return o.end(2);
        }
    };

    let prompt = if let Some(pf) = &a.prompt_file {
        let path = expanduser(pf, env);
        match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                // a Python text file reads \r\n and \r as \n
                Ok(s) => s.replace("\r\n", "\n").replace('\r', "\n"),
                Err(e) => {
                    o.say(format!("spawn: cannot read the prompt file: {}", e.utf8_error()));
                    return o.end(2);
                }
            },
            Err(e) => {
                o.say(format!("spawn: cannot read the prompt file: {}", os_error_text(&e, &path)));
                return o.end(2);
            }
        }
    } else {
        a.prompt.clone().unwrap_or_default()
    };
    if py_strip(&prompt).is_empty() {
        o.say("spawn: the prompt is empty");
        return o.end(2);
    }
    let dir = a.dir.clone().unwrap_or_else(|| cwd.to_string());
    let folder = realpath(&expanduser(&dir, env), cwd);

    let which = |n: &str| which_in(n, env);
    let tmux_has = |tmux: &str, name: &str| {
        tmux_command(tmux, env, &["-L", SOCKET, "has-session", "-t", &format!("={}", name)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    let checked = (|| {
        let base = match a.name.as_deref() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => make_name(&prompt, &now_stamp()),
        };
        let name = safe_name(&base)?;
        let (claude, tmux) = preflight(&folder, &a.mode, &name, env, &which, &tmux_has)?;
        Ok::<_, String>((name, claude, tmux))
    })();
    let (name, claude, tmux) = match checked {
        Ok(t) => t,
        Err(e) => {
            o.say(format!("FAIL  {}", e));
            o.say(format!("FIX   {}", e.split_once(": ").map(|(_, f)| f).unwrap_or(&e)));
            return o.end(3);
        }
    };

    let (cfg, _) = config_paths(env);
    let keep = py_join(&cfg, "spawn");
    let _ = std::fs::create_dir_all(&keep);
    let prompt_file = py_join(&keep, &format!("{}.md", name));
    let cmd = shell_command(&claude, &name, &a.mode, &prompt_file);
    let tmux_args = ["-L", SOCKET, "new-session", "-d", "-s", &name, "-c", &folder, "-x", "200", "-y", "50", &cmd];
    if a.dry_run {
        o.say("CHECKS ok");
        let quoted: Vec<String> = std::iter::once(tmux.as_str())
            .chain(tmux_args.iter().copied())
            .map(shlex_quote)
            .collect();
        o.say(format!("CMD   {}", quoted.join(" ")));
        return o;
    }
    if let Err(e) = std::fs::write(&prompt_file, &prompt) {
        o.say(format!("FAIL  cannot write the prompt file: {}", os_error_text(&e, &prompt_file)));
        return o.end(4);
    }

    let child_env = clean_env(env);
    let started = tmux_command(&tmux, &child_env, &tmux_args).stdin(Stdio::inherit()).output();
    match started {
        Ok(r) if r.status.success() => {}
        Ok(r) => {
            let err = String::from_utf8_lossy(&r.stderr).trim().to_string();
            let why = if err.is_empty() { r.status.code().unwrap_or(-1).to_string() } else { err };
            o.say(format!("FAIL  tmux did not start: {}", why));
            return o.end(4);
        }
        Err(e) => {
            o.say(format!("FAIL  tmux did not start: {}", os_error_text(&e, &tmux)));
            return o.end(4);
        }
    }
    let wait = a.wait;
    let deadline = if wait > 0 {
        Instant::now().checked_add(Duration::from_secs(wait as u64)).unwrap_or_else(|| Instant::now() + Duration::from_secs(3_000_000_000))
    } else {
        Instant::now()
    };
    let (mut state, mut info, mut pane) = ("wait", String::new(), String::new());
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(1));
        pane = tmux_command(&tmux, &child_env, &["-L", SOCKET, "capture-pane", "-p", "-t", &name])
            .output()
            .map(|r| String::from_utf8_lossy(&r.stdout).into_owned())
            .unwrap_or_default();
        (state, info) = classify(&pane, &prompt, &folder);
        if state != "wait" {
            break;
        }
    }
    let watch = format!("tmux -L {} attach -t {}", SOCKET, name);
    let stop = format!("tmux -L {} kill-session -t {}", SOCKET, name);
    if state == "ok" {
        o.say(format!("NAME  {}", name));
        o.say(format!("MODE  {} in {}", a.mode, folder));
        o.say(format!("URL   {}", info));
        o.say(format!("WATCH phone: Claude app > Code > {} · Terminal: {}", name, watch));
        o.say(format!("STOP  {}", stop));
        return o;
    }
    let lines: Vec<String> = py_splitlines(&pane).into_iter().filter(|l| !py_strip(l).is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(12)..].join("\n");
    if state == "fail" {
        let _ = tmux_command(&tmux, &child_env, &["-L", SOCKET, "kill-session", "-t", &name]).output();
        o.say(format!("FAIL  {} (session stopped)", info));
        o.say(tail);
        return o.end(4);
    }
    o.say(format!("FAIL  no Remote Control link after {}s. The session still runs.", wait));
    o.say(format!("WATCH {}", watch));
    o.say(format!("STOP  {}", stop));
    o.say(tail);
    o.end(5)
}

fn main() {
    let argv: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let env: EnvMap = std::env::vars_os()
        .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
        .collect();
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| ".".into());
    let o = run(&argv, &env, &cwd);
    let _ = std::io::stdout().write_all(o.stdout.as_bytes());
    let _ = std::io::stderr().write_all(o.stderr.as_bytes());
    std::process::exit(o.code);
}

// ---------------------------------------------------------------------------
// Tests: one counterpart per case in test_spawn.py, then the argparse and
// full-run paths against a fake tmux.
// ---------------------------------------------------------------------------

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const OK_PANE: &str = " Claude Code v2.1.296
 ~/proj · /rc active
> Fix the login bug in app/auth.ts
  /remote-control is active · Continue here, on your phone, or at https://claude.ai/code/session_01XLjrBUwh9QVMBxLZ1bcqE4
";

    /// A temp HOME with ~/.claude.json and ~/.claude/settings.json.
    struct Home {
        _dir: tempfile::TempDir,
        root: String,
        proj: String,
        env: EnvMap,
    }

    impl Home {
        fn new(trusted: bool, bypass: bool, logged_in: bool, skip_setting: bool) -> Home {
            let dir = tempfile::tempdir().unwrap();
            let root = realpath(dir.path().to_str().unwrap(), "/");
            let proj = format!("{}/proj", root);
            std::fs::create_dir_all(format!("{}/sub", proj)).unwrap();
            std::fs::create_dir_all(format!("{}/.claude", root)).unwrap();
            let mut g = serde_json::json!({ "projects": { proj.clone(): { "hasTrustDialogAccepted": trusted } } });
            if logged_in {
                g["oauthAccount"] = serde_json::json!({ "emailAddress": "x" });
            }
            if bypass {
                g["bypassPermissionsModeAccepted"] = Value::Bool(true);
            }
            std::fs::write(format!("{}/.claude.json", root), g.to_string()).unwrap();
            let settings = if skip_setting {
                serde_json::json!({ "skipDangerousModePermissionPrompt": true })
            } else {
                serde_json::json!({})
            };
            std::fs::write(format!("{}/.claude/settings.json", root), settings.to_string()).unwrap();
            let mut env = EnvMap::new();
            env.insert("HOME".into(), root.clone());
            Home { _dir: dir, root, proj, env }
        }
    }

    fn fake_which(name: &str) -> Option<String> {
        Some(format!("/usr/bin/{}", name))
    }

    fn check(
        h: &Home,
        mode: &str,
        folder: Option<&str>,
        has: bool,
        which: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(String, String), String> {
        preflight(folder.unwrap_or(&h.proj), mode, "n1", &h.env, which, &|_, _| has)
    }

    #[test]
    fn test_all_good() {
        let h = Home::new(true, true, true, false);
        assert_eq!(
            check(&h, "bypassPermissions", None, false, &fake_which),
            Ok(("/usr/bin/claude".to_string(), "/usr/bin/tmux".to_string()))
        );
        let sub = format!("{}/sub", h.proj);
        assert_eq!(check(&h, "bypassPermissions", Some(&sub), false, &fake_which).unwrap().0, "/usr/bin/claude");
    }

    #[test]
    fn test_skip_setting_counts_as_accepted() {
        let h = Home::new(true, false, true, true);
        check(&h, "bypassPermissions", None, false, &fake_which).unwrap();
    }

    #[test]
    fn test_failures_name_the_fix() {
        let none = |_: &str| -> Option<String> { None };
        // (trusted, bypass, logged_in), has, which, folder, want
        let cases: Vec<(bool, bool, bool, bool, bool, Option<&str>, &str)> = vec![
            (true, false, true, false, true, None, "bypass warning"),
            (false, true, true, false, true, None, "not trusted"),
            (true, true, false, false, true, None, "not logged in"),
            (true, true, true, true, true, None, "already exists"),
            (true, true, true, false, false, None, "tmux missing"),
            (true, true, true, false, true, Some("/nope/nowhere"), "does not exist"),
        ];
        for (trusted, bypass, logged_in, has, with_which, folder, want) in cases {
            let h = Home::new(trusted, bypass, logged_in, false);
            let which: &dyn Fn(&str) -> Option<String> = if with_which { &fake_which } else { &none };
            let err = check(&h, "bypassPermissions", folder, has, which).unwrap_err();
            assert!(err.contains(want), "{:?} not in {:?}", want, err);
        }
    }

    #[test]
    fn test_other_modes_skip_bypass_check() {
        let h = Home::new(true, false, true, false);
        check(&h, "acceptEdits", None, false, &fake_which).unwrap();
    }

    #[test]
    fn test_clean_env() {
        let env: EnvMap = [
            ("PATH", "/bin"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_SESSION_ID", "x"),
            ("ANTHROPIC_API_KEY", "k"),
            ("MCP_X", "1"),
            ("CLAUDE_CONFIG_DIR", "/c"),
            ("HOME", "/h"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let want: EnvMap = [("PATH", "/bin"), ("CLAUDE_CONFIG_DIR", "/c"), ("HOME", "/h")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(clean_env(&env), want);
    }

    #[test]
    fn test_names() {
        assert_eq!(make_name("Fix the login bug!", "0905"), "fix-the-login-0905");
        assert_eq!(make_name("!!!", "0905"), "session-0905");
        assert_eq!(safe_name("a.b:c d").unwrap(), "a-b-c-d");
        assert!(safe_name("...").is_err());
        // lower() first, as Python does: the Kelvin sign becomes a plain k
        assert_eq!(make_name("\u{212A}ey", "0905"), "key-0905");
        // the 40 char cut, then the trailing dash goes
        assert_eq!(make_name(&"a".repeat(39), "0905"), "a".repeat(39) + "-");
        assert_eq!(safe_name(&make_name(&"a".repeat(39), "0905")).unwrap(), "a".repeat(39));
    }

    #[test]
    fn test_command_quotes_everything() {
        let cmd = shell_command("/x/claude", "n1", "bypassPermissions", "/tmp/it's here.md");
        assert!(cmd.contains("--remote-control n1 --name n1 --permission-mode bypassPermissions"));
        assert!(cmd.contains("\"$(cat '/tmp/it'\"'\"'s here.md')\""));
        assert!(cmd.contains(EXIT_MARK));
        assert_eq!(shlex_quote(""), "''");
        assert_eq!(shlex_quote("a-b/c.d:e=f,g@h%i+j_k"), "a-b/c.d:e=f,g@h%i+j_k");
        assert_eq!(shlex_quote("a b"), "'a b'");
    }

    const P: &str = "Fix the login bug in app/auth.ts";

    #[test]
    fn test_classify_ok() {
        let (state, url) = classify(OK_PANE, P, "/p");
        assert_eq!(state, "ok");
        assert!(url.starts_with("https://claude.ai/code/session_01XL"));
    }

    #[test]
    fn test_classify_wait() {
        assert_eq!(classify("Claude Code v2\n> Fix the login", P, "/p").0, "wait");
        assert_eq!(classify("", P, "/p").0, "wait");
    }

    #[test]
    fn test_classify_dialogs() {
        let exited = format!("error\n{} 1", EXIT_MARK);
        let cases = [
            ("Do you trust the files in this folder?\n1. Yes", "not trusted"),
            ("WARNING: Claude Code running in Bypass Permissions mode\n1. No, exit", "bypass"),
            ("Select login method:\n1. Claude account", "not logged in"),
            (exited.as_str(), "exited"),
        ];
        for (pane, want) in cases {
            let (state, why) = classify(pane, P, "/p");
            assert_eq!(state, "fail", "{}", pane);
            assert!(why.contains(want), "{}", why);
        }
        assert_eq!(
            classify("Do you trust the files in this folder?", P, "/p").1,
            "folder not trusted: run `claude` once in /p and accept"
        );
    }

    #[test]
    fn test_classify_prompt_words_are_not_a_dialog() {
        let p = "Check why Bypass Permissions mode and trust the files in this folder fail";
        let pane = format!("> {}\n  working...", p);
        assert_eq!(classify(&pane, p, "/p").0, "wait");
    }

    // ---- beyond the Python tests: the parts the port had to get right ----

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn usage_error(args: &[&str]) -> String {
        match parse_args(&s(args)) {
            Err(ArgExit::Error(m)) => m,
            Err(ArgExit::Help) => "HELP".into(),
            Ok(a) => format!("OK {:?}", a),
        }
    }

    #[test]
    fn test_argparse_messages() {
        assert_eq!(usage_error(&[]), "one of the arguments --prompt-file --prompt is required");
        assert_eq!(usage_error(&["--prompt", "a", "--prompt-file", "b"]), "argument --prompt-file: not allowed with argument --prompt");
        assert_eq!(usage_error(&["--prompt-file", "b", "--prompt", "a"]), "argument --prompt: not allowed with argument --prompt-file");
        assert_eq!(
            usage_error(&["--prompt", "x", "--mode", "nope"]),
            "argument --mode: invalid choice: 'nope' (choose from 'bypassPermissions', 'acceptEdits', 'auto', 'default', 'dontAsk', 'plan')"
        );
        assert_eq!(usage_error(&["--prompt", "x", "--wait", "z"]), "argument --wait: invalid int value: 'z'");
        assert_eq!(usage_error(&["--prompt", "x", "--dir"]), "argument --dir: expected one argument");
        assert_eq!(usage_error(&["--prompt", "-x"]), "argument --prompt: expected one argument");
        assert_eq!(usage_error(&["--prompt", "x", "extra"]), "unrecognized arguments: extra");
        assert_eq!(usage_error(&["--pro", "x"]), "ambiguous option: --pro could match --prompt-file, --prompt");
        assert_eq!(usage_error(&["--prompt", "x", "--dry-run=1"]), "argument --dry-run: ignored explicit argument '1'");
        assert_eq!(usage_error(&["-h", "--bogus"]), "HELP");
        assert!(usage_error(&["--prompt", "x", "--mo", "plan", "--wait", "-5", "--dry-run"]).contains("mode: \"plan\""));
        assert!(usage_error(&["--prompt=- a b"]).contains("prompt: Some(\"- a b\")"));
    }

    struct Fake {
        _dir: tempfile::TempDir,
        bin: String,
        log: String,
        pane: String,
    }

    /// A PATH dir with a fake `tmux` (logs its arguments, shows a pane file) and `claude`.
    fn fake_bin(has_session: bool, pane: &str) -> Fake {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = realpath(dir.path().to_str().unwrap(), "/");
        let bin = format!("{}/bin", root);
        std::fs::create_dir_all(&bin).unwrap();
        let log = format!("{}/tmux.log", root);
        let pane_file = format!("{}/pane.txt", root);
        std::fs::write(&pane_file, pane).unwrap();
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{log}'\ncase \"$3\" in\n  has-session) exit {has} ;;\n  capture-pane) cat '{pane_file}' ;;\n  *) exit 0 ;;\nesac\n",
            log = log,
            has = if has_session { 0 } else { 1 },
            pane_file = pane_file
        );
        for (name, body) in [("tmux", script.as_str()), ("claude", "#!/bin/sh\nexit 0\n")] {
            let p = format!("{}/{}", bin, name);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Fake { _dir: dir, bin, log, pane: pane_file }
    }

    fn env_with(h: &Home, f: &Fake) -> EnvMap {
        let mut env = h.env.clone();
        env.insert("PATH".into(), format!("{}:/usr/bin:/bin", f.bin));
        env.insert("CLAUDECODE".into(), "1".into());
        env
    }

    #[test]
    fn test_dry_run_prints_checks_and_command() {
        let h = Home::new(true, true, true, false);
        let f = fake_bin(false, "");
        let env = env_with(&h, &f);
        let o = run(&s(&["--prompt", "Fix the login bug", "--dir", &h.proj, "--name", "fix login", "--dry-run"]), &env, &h.root);
        assert_eq!(o.code, 0, "{}", o.stdout);
        let want = format!(
            "CHECKS ok\nCMD   {bin}/tmux -L claude-spawn new-session -d -s fix-login -c {proj} -x 200 -y 50 '{bin}/claude --remote-control fix-login --name fix-login --permission-mode bypassPermissions \"$(cat {root}/.claude/spawn/fix-login.md)\"; echo CLAUDE_EXITED $?; exec sleep 86400'\n",
            bin = f.bin,
            proj = h.proj,
            root = h.root
        );
        assert_eq!(o.stdout, want);
        assert!(!Path::new(&format!("{}/.claude/spawn/fix-login.md", h.root)).exists());
    }

    #[test]
    fn test_check_failure_prints_fail_and_fix() {
        let h = Home::new(true, false, true, false);
        let f = fake_bin(false, "");
        let env = env_with(&h, &f);
        let o = run(&s(&["--prompt", "x", "--dir", &h.proj, "--name", "n1"]), &env, &h.root);
        assert_eq!(o.code, 3);
        assert!(o.stdout.starts_with("FAIL  bypass warning never accepted on this machine: run "));
        assert!(o.stdout.contains("\nFIX   run `claude --permission-mode bypassPermissions` once"));
        // a name that cleans to nothing
        let o = run(&s(&["--prompt", "x", "--name", "..."]), &env, &h.root);
        assert_eq!(o.code, 3);
        assert_eq!(
            o.stdout,
            "FAIL  name is empty after cleaning: use letters, digits, - or _\nFIX   use letters, digits, - or _\n"
        );
        // empty prompt, missing prompt file
        assert_eq!(run(&s(&["--prompt", "  \n"]), &env, &h.root), run_stdout("spawn: the prompt is empty\n", 2));
        let o = run(&s(&["--prompt-file", "/no/such/p.md"]), &env, &h.root);
        assert_eq!(o.stdout, "spawn: cannot read the prompt file: [Errno 2] No such file or directory: '/no/such/p.md'\n");
        assert_eq!(o.code, 2);
    }

    fn run_stdout(out: &str, code: i32) -> Outcome {
        Outcome { code, stdout: out.into(), stderr: String::new() }
    }

    impl PartialEq for Outcome {
        fn eq(&self, other: &Self) -> bool {
            self.code == other.code && self.stdout == other.stdout && self.stderr == other.stderr
        }
    }
    impl std::fmt::Debug for Outcome {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Outcome({}, {:?}, {:?})", self.code, self.stdout, self.stderr)
        }
    }

    #[test]
    fn test_full_run_reads_the_link_and_cleans_the_env() {
        let h = Home::new(true, true, true, false);
        let f = fake_bin(false, OK_PANE);
        let env = env_with(&h, &f);
        let pf = format!("{}/p.md", h.root);
        std::fs::write(&pf, "Fix the login bug in app/auth.ts\r\nmore\r\n").unwrap();
        let o = run(&s(&["--prompt-file", &pf, "--dir", &h.proj, "--name", "n1", "--wait", "5"]), &env, &h.root);
        assert_eq!(o.code, 0, "{}", o.stdout);
        let lines: Vec<&str> = o.stdout.lines().collect();
        assert_eq!(lines[0], "NAME  n1");
        assert_eq!(lines[1], format!("MODE  bypassPermissions in {}", h.proj));
        assert_eq!(lines[2], "URL   https://claude.ai/code/session_01XLjrBUwh9QVMBxLZ1bcqE4");
        assert_eq!(lines[3], "WATCH phone: Claude app > Code > n1 · Terminal: tmux -L claude-spawn attach -t n1");
        assert_eq!(lines[4], "STOP  tmux -L claude-spawn kill-session -t n1");
        // line ends are read like a Python text file reads them
        assert_eq!(std::fs::read_to_string(format!("{}/.claude/spawn/n1.md", h.root)).unwrap(), "Fix the login bug in app/auth.ts\nmore\n");
        let log = std::fs::read_to_string(&f.log).unwrap();
        assert!(log.starts_with("-L claude-spawn has-session -t =n1\n-L claude-spawn new-session -d -s n1 -c "));
        assert!(log.contains("-L claude-spawn capture-pane -p -t n1\n"));
    }

    #[test]
    fn test_full_run_failures_and_timeout() {
        let h = Home::new(true, true, true, false);
        let f = fake_bin(false, "Select login method:\n1. Claude account\n");
        let env = env_with(&h, &f);
        let o = run(&s(&["--prompt", "hello there", "--dir", &h.proj, "--name", "n2", "--wait", "3"]), &env, &h.root);
        assert_eq!(o.code, 4);
        assert_eq!(
            o.stdout,
            "FAIL  not logged in to claude.ai: run `claude` and /login (session stopped)\nSelect login method:\n1. Claude account\n"
        );
        assert!(std::fs::read_to_string(&f.log).unwrap().contains("kill-session -t n2"));

        std::fs::write(&f.pane, "Claude Code v2\n\n> hello there\n").unwrap();
        let o = run(&s(&["--prompt", "hello there", "--dir", &h.proj, "--name", "n3", "--wait", "1"]), &env, &h.root);
        assert_eq!(o.code, 5);
        assert_eq!(
            o.stdout,
            "FAIL  no Remote Control link after 1s. The session still runs.\nWATCH tmux -L claude-spawn attach -t n3\nSTOP  tmux -L claude-spawn kill-session -t n3\nClaude Code v2\n> hello there\n"
        );

        // a session of that name already exists
        let f = fake_bin(true, "");
        let env = env_with(&h, &f);
        let o = run(&s(&["--prompt", "x", "--dir", &h.proj, "--name", "n4"]), &env, &h.root);
        assert_eq!(o.code, 3);
        assert!(o.stdout.starts_with("FAIL  tmux session n4 already exists: pick another --name\nFIX   "));
    }

    #[test]
    fn test_paths_like_python() {
        assert_eq!(py_dirname("/h/.claude"), "/h");
        assert_eq!(py_dirname(".claude"), "");
        assert_eq!(py_dirname("/.claude"), "/");
        assert_eq!(py_join("", ".claude"), ".claude");
        assert_eq!(py_join("/h/", ".claude"), "/h/.claude");
        let mut env = EnvMap::new();
        env.insert("HOME".into(), "/home/x/".into());
        assert_eq!(expanduser("~/proj", &env), "/home/x/proj");
        assert_eq!(expanduser("~", &env), "/home/x");
        assert_eq!(config_paths(&env), ("/home/x/.claude".to_string(), "/home/x/.claude.json".to_string()));
        env.insert("CLAUDE_CONFIG_DIR".into(), "/c".into());
        assert_eq!(config_paths(&env), ("/c".to_string(), "/c/.claude.json".to_string()));
        env.insert("CLAUDE_CONFIG_DIR".into(), String::new());
        assert_eq!(config_paths(&env).0, "/home/x/.claude");
        assert_eq!(realpath("a/../b/./c", "/w"), "/w/b/c");
        assert_eq!(realpath("", "/w"), "/w");
        assert_eq!(py_splitlines("a\r\nb\rc\x0cd\n"), vec!["a", "b", "c", "d"]);
        assert_eq!(py_flat(" a\x1c b\n"), "a b");
    }
}
