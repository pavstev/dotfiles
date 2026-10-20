//! statusline: the Claude Code status line, copper themed. Reads the session JSON on stdin.
//! Replaces claude/statusline.sh and needs no `jq`.
//!
//! Prints:   <dir>   <git branch>   <model>   ctx <K> <%>   5h <%> · 7d <%>
//! Context turns copper at 300 K: time to /ps:handoff. Rate limits are absent until the first API
//! reply; then nothing is printed for them. Writes the pace cache `claude-pace.json` (read by the
//! pace hook) when the weekly limit is present. `NO_COLOR` set (non-empty) or `THEME=plain` in the
//! dotfiles settings prints the same text without escape codes.

use serde_json::{Map, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const COPPER: &str = "\x1b[38;2;224;130;95m"; // #E0825F
const SAGE: &str = "\x1b[38;2;135;176;131m"; // #87B083
const STONE: &str = "\x1b[38;2;168;162;158m"; // #A8A29E
const RST: &str = "\x1b[0m";
/// Context size (in K) from which the segment turns copper.
const CTX_WARN_K: i64 = 300;

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `jq -r "<path> // empty"`: nothing for null, false or a missing key; strings as they are;
/// anything else as JSON text.
fn jq_str(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |cur, k| cur.get(*k))
}

/// A number the way `printf '%.0f'` rounds it (half to even).
fn int_text(v: Option<&Value>) -> Option<String> {
    let n = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    Some(format!("{n:.0}"))
}

/// The last call's input plus cache tokens (`total_input_tokens` is a session running total).
fn ctx_tokens(input: &Value) -> Option<f64> {
    let from_usage = at(input, &["context_window", "current_usage"]).and_then(|u| {
        let u = u.as_object()?;
        let mut sum = 0.0;
        let mut any = false;
        for key in ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"] {
            match u.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::Number(n)) => {
                    sum += n.as_f64()?;
                    any = true;
                }
                Some(_) => return None, // jq would fail on the addition
            }
        }
        any.then_some(sum)
    });
    match from_usage {
        Some(t) if t != 0.0 => Some(t),
        _ => match at(input, &["context_window", "total_input_tokens"]) {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(s)) => s.trim().parse().ok(),
            _ => None,
        },
    }
}

fn basename(path: &str) -> String {
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let trimmed = path.trim_end_matches(is_sep);
    if trimmed.is_empty() {
        return if path.is_empty() { String::new() } else { "/".to_string() };
    }
    trimmed.rsplit(is_sep).next().unwrap_or(trimmed).to_string()
}

fn git_branch(cwd: &str) -> String {
    Command::new("git")
        .args(["-C", cwd, "branch", "--show-current"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end_matches(['\n', '\r']).to_string())
        .unwrap_or_default()
}

/// Build the line. `branch` is looked up by the caller so tests can pass it in.
fn render(input: &Value, pwd: &str, branch_of: &dyn Fn(&str) -> String, color: bool) -> String {
    let cwd = jq_str(at(input, &["workspace", "current_dir"]))
        .filter(|s| !s.is_empty())
        .or_else(|| jq_str(input.get("cwd")).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| pwd.to_string());
    let model = jq_str(at(input, &["model", "display_name"])).unwrap_or_default();
    let dir = basename(&cwd);
    let branch = branch_of(&cwd);
    let paint = |code: &str, text: &str| if color { format!("{code}{text}{RST}") } else { text.to_string() };

    let mut line = paint(COPPER, &format!(" {dir}"));
    if !branch.is_empty() {
        line.push_str(&format!("  {}", paint(SAGE, &format!(" {branch}"))));
    }
    if !model.is_empty() {
        line.push_str(&format!("  {}", paint(STONE, &model)));
    }

    if let Some(tok) = ctx_tokens(input) {
        let k = (format!("{tok:.0}").parse::<f64>().unwrap_or(0.0) as i64) / 1000;
        let code = if k >= CTX_WARN_K { COPPER } else { STONE };
        let mut ctx = format!("ctx {k}K");
        if let Some(p) = int_text(at(input, &["context_window", "used_percentage"])) {
            ctx.push_str(&format!(" {p}%"));
        }
        line.push_str(&format!("  {}", paint(code, &ctx)));
    }

    let mut caps = String::new();
    if let Some(p) = int_text(at(input, &["rate_limits", "five_hour", "used_percentage"])) {
        caps = format!("5h {p}%");
    }
    if let Some(p) = int_text(at(input, &["rate_limits", "seven_day", "used_percentage"])) {
        if !caps.is_empty() {
            caps.push_str(" · ");
        }
        caps.push_str(&format!("7d {p}%"));
    }
    if !caps.is_empty() {
        line.push_str(&format!("  {}", paint(STONE, &caps)));
    }
    line
}

/// The pace cache JSON, field order as in the shell: `updated`, `five_hour`, `seven_day`.
fn pace_json(input: &Value, now: u64) -> Option<String> {
    jq_str(at(input, &["rate_limits", "seven_day", "used_percentage"]))?;
    let pick = |v: &Value| {
        let mut m = Map::new();
        for k in ["used_percentage", "resets_at"] {
            m.insert(k.to_string(), v.get(k).cloned().unwrap_or(Value::Null));
        }
        Value::Object(m)
    };
    let mut out = Map::new();
    out.insert("updated".into(), Value::from(now));
    let five = match at(input, &["rate_limits", "five_hour"]) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Value::Null,
        Some(v) => pick(v),
    };
    out.insert("five_hour".into(), five);
    out.insert("seven_day".into(), pick(at(input, &["rate_limits", "seven_day"])?));
    serde_json::to_string(&Value::Object(out)).ok()
}

/// Write `text` to `file` through a temp file and a rename. Errors are ignored by the caller.
fn write_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = PathBuf::from(format!("{}.{}", file.display(), std::process::id()));
    let res = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, file));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".cache"))
}

fn main() {
    let mut raw = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut raw);
    let input: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Some(json) = pace_json(&input, now) {
        let _ = write_atomic(&cache_dir().join("claude-pace.json"), &format!("{json}\n"));
    }

    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let plain = agent_config::setting(&home_dir(), "THEME") == "plain";
    let pwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let line = render(&input, &pwd, &git_branch, !(no_color || plain));
    let mut out = std::io::stdout();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}
