use clap::Parser;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const NUDGE_CONTEXT: i64 = 300_000;
const SPAWN_CONTEXT: i64 = 500_000;
const NUDGE_EVERY: i64 = 5;
const MAX_STATE_FILES: usize = 200;
const MAX_STATE_AGE: u64 = 7 * 24 * 3600;
const SUBAGENT_TOOLS: &[&str] = &["Task", "Subagent", "mcp_task"];
const DIRECT_KEYS: &[&str] = &["context_tokens", "contextTokens"];
const USAGE_KEYS: &[&str] = &["usage", "token_usage", "tokenUsage", "tokens"];

#[derive(Parser, Debug)]
#[command(name = "guard_cursor", about = "Cursor port of guard G3 (context ceiling)")]
struct Cli {
    mode: Option<String>,
}

fn state_dir_from(tmpdir: Option<String>) -> PathBuf {
    let tmp = tmpdir.filter(|s| !s.is_empty()).unwrap_or_else(|| "/tmp".to_string());
    PathBuf::from(tmp).join("cursor-guard")
}

fn state_dir() -> PathBuf {
    state_dir_from(env::var("TMPDIR").ok())
}

/// A non-empty string field; Python treats "" like a missing value.
fn str_field<'a>(data: &'a Value, name: &str) -> Option<&'a str> {
    data.get(name).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

fn conv_key(data: &Value, mode: &str) -> String {
    let raw = if mode == "subagent" {
        str_field(data, "parent_conversation_id").or_else(|| str_field(data, "conversation_id"))
    } else {
        str_field(data, "conversation_id")
    };
    let raw_str = raw.unwrap_or("unknown");
    let re = Regex::new(r"[^A-Za-z0-9_.-]").unwrap();
    let replaced = re.replace_all(raw_str, "_");
    let key: String = replaced.chars().take(128).collect();
    if key.trim_matches('.').is_empty() {
        "unknown".to_string()
    } else {
        key
    }
}

fn read_state(key: &str) -> BTreeMap<String, Value> {
    let path = state_dir().join(format!("{}.json", key));
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => BTreeMap::new(),
    }
}

fn prune_state(dir: &Path) {
    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => return,
    };
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    let mut files = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let mt = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now.saturating_sub(mt) > MAX_STATE_AGE {
            let _ = fs::remove_file(&path);
        } else {
            files.push((mt, path));
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    if files.len() > MAX_STATE_FILES {
        for (_, path) in files.iter().skip(MAX_STATE_FILES) {
            let _ = fs::remove_file(path);
        }
    }
}

fn write_state(key: &str, mut st: BTreeMap<String, Value>) {
    let d = state_dir();
    let _ = fs::create_dir_all(&d);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    st.insert("updated".to_string(), Value::from(now));
    let tmp = d.join(format!(".{}.{}.tmp", key, std::process::id()));
    let target = d.join(format!("{}.json", key));
    if let Ok(bytes) = serde_json::to_vec(&st)
        && fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(&tmp, &target);
        }
    prune_state(&d);
}

fn to_int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                if i > 0 { i } else { 0 }
            } else if let Some(f) = n.as_f64() {
                if f > 0.0 { f as i64 } else { 0 }
            } else {
                0
            }
        }
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.chars().all(|c| c.is_ascii_digit()) {
                s.parse::<i64>().unwrap_or(0)
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn tokens_from(obj: Option<&Value>) -> i64 {
    if let Some(map) = obj.and_then(|v| v.as_object()) {
        for k in DIRECT_KEYS {
            let n = to_int(map.get(*k));
            if n > 0 {
                return n;
            }
        }
    }
    0
}

fn response_tokens(data: &Value) -> i64 {
    for k in USAGE_KEYS {
        let n = tokens_from(data.get(*k));
        if n > 0 {
            return n;
        }
    }
    tokens_from(Some(data))
}

fn default_app_db() -> PathBuf {
    if cfg!(target_os = "macos") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join("Library/Application Support/Cursor/User/globalStorage/state.vscdb")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".config/Cursor/User/globalStorage/state.vscdb")
    }
}

fn app_context(data: &Value) -> i64 {
    let conv = match str_field(data, "parent_conversation_id").or_else(|| str_field(data, "conversation_id")) {
        Some(s) => s,
        None => return 0,
    };
    let path = match env::var("CURSOR_GUARD_APP_DB") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => default_app_db(),
    };
    if !path.is_file() {
        return 0;
    }
    // A path starting with "-" must not be read as an option.
    let path = if path.is_relative() {
        PathBuf::from(".").join(&path)
    } else {
        path
    };
    let escaped_conv = conv.replace('\'', "''").replace('\0', "");
    let query = format!(
        "SELECT value FROM cursorDiskKV WHERE key = 'composerData:{}' LIMIT 1;",
        escaped_conv
    );
    // Read-only, no rc file (-init /dev/null), fixed output mode, 1 s busy timeout.
    let child = Command::new("sqlite3")
        .args(["-readonly", "-batch", "-noheader", "-list", "-nullvalue", ""])
        .args(["-init", "/dev/null", "-cmd", ".timeout 1000"])
        .arg(&path)
        .arg(&query)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => return 0,
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    if !ok {
        return 0;
    }
    let val_str = String::from_utf8_lossy(&bytes).to_string();
    let trimmed = val_str.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let meta: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let mut n = to_int(meta.get("contextTokensUsed"));
    if n == 0
        && let Some(breakdown) = meta.get("promptTokenBreakdown") {
            n = to_int(breakdown.get("totalUsedTokens"));
        }
    n
}

/// Lazy newest-first line iterator: reads the file backwards in chunks, so a
/// caller that stops early never reads the rest of the file.
struct RevLines {
    f: fs::File,
    pos: u64,
    chunk: u64,
    buf: Vec<u8>,
    pending: Vec<String>, // forward order; pop() yields the newest
    finished: bool,
}

fn reverse_lines(path: &Path) -> RevLines {
    let mut f = fs::File::open(path).ok();
    let len = f.as_mut().and_then(|f| f.seek(SeekFrom::End(0)).ok());
    match (f, len) {
        (Some(f), Some(len)) => RevLines { f, pos: len, chunk: 1 << 16, buf: Vec::new(), pending: Vec::new(), finished: false },
        _ => {
            // Unreadable: an empty iterator.
            let f = fs::File::open("/dev/null").expect("/dev/null");
            RevLines { f, pos: 0, chunk: 1, buf: Vec::new(), pending: Vec::new(), finished: true }
        }
    }
}

impl Iterator for RevLines {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        loop {
            if let Some(l) = self.pending.pop() {
                return Some(l);
            }
            if self.pos == 0 {
                if self.finished {
                    return None;
                }
                self.finished = true;
                let s = String::from_utf8_lossy(&std::mem::take(&mut self.buf)).trim().to_string();
                return if s.is_empty() { None } else { Some(s) };
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
            let pend: Vec<String> = parts[1..]
                .iter()
                .map(|l| String::from_utf8_lossy(l).trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            self.buf = first;
            self.pending = pend;
        }
    }
}

fn transcript_context_size(path: &Path) -> i64 {
    if !path.is_file() {
        return 0;
    }
    for line in reverse_lines(path) {
        if (line.contains("compact_boundary") || line.contains("isCompactSummary"))
            && let Ok(rec) = serde_json::from_str::<Value>(&line)
                && (rec.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary")
                    || rec.get("isCompactSummary").and_then(|v| v.as_bool()) == Some(true))
                {
                    return 0;
                }
        if !line.contains("\"assistant\"") {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<Value>(&line) {
            if rec.get("type").and_then(|v| v.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(msg) = rec.get("message")
                && let Some(usage) = msg.get("usage") {
                    let total = to_int(usage.get("input_tokens"))
                        + to_int(usage.get("cache_creation_input_tokens"))
                        + to_int(usage.get("cache_read_input_tokens"));
                    if total > 0 {
                        return total;
                    }
                }
        }
    }
    0
}

fn estimate(data: &Value, st: &BTreeMap<String, Value>) -> i64 {
    if let Some(p) = data.get("transcript_path").and_then(|v| v.as_str()) {
        let path = PathBuf::from(p);
        if path.is_file() {
            let n = transcript_context_size(&path);
            if n > 0 {
                return n;
            }
        }
    }
    let app = app_context(data);
    if app > 0 {
        return app;
    }
    to_int(st.get("ctx"))
}

fn mode_prompt(data: &Value, key: &str) {
    let mut st = read_state(key);
    let size = estimate(data, &st);
    let count = if size >= NUDGE_CONTEXT {
        to_int(st.get("prompts")) + 1
    } else {
        0
    };
    st.insert("prompts".to_string(), Value::from(count));
    write_state(key, st);

    let mut out = serde_json::json!({"continue": true});
    if count >= 1 && (count - 1) % NUDGE_EVERY == 0 {
        let msg = format!(
            "Context is about {}K. Run /ps:handoff and continue in a fresh session.",
            size / 1000
        );
        out["user_message"] = Value::String(msg.clone());
        out["additional_context"] = Value::String(format!(
            "Start your reply by telling the user, in one line: {}",
            msg
        ));
    }
    emit_out(&serde_json::to_string(&out).unwrap_or_default());
}

fn emit_out(s: &str) {
    let mut o = std::io::stdout();
    let _ = writeln!(o, "{}", s);
    let _ = o.flush();
}

fn mode_subagent(data: &Value, key: &str) -> i32 {
    if let Some(tool) = data.get("tool_name").and_then(|v| v.as_str())
        && !SUBAGENT_TOOLS.contains(&tool) {
            return 0;
        }
    let st = read_state(key);
    let size = estimate(data, &st);
    if size > SPAWN_CONTEXT {
        let msg = format!(
            "guard: context is {}K (> {}K); run /ps:handoff and continue in a fresh session instead of spawning.",
            size / 1000,
            SPAWN_CONTEXT / 1000
        );
        let out = serde_json::json!({
            "permission": "deny",
            "user_message": msg,
            "agent_message": msg
        });
        emit_out(&serde_json::to_string(&out).unwrap_or_default());
        let _ = writeln!(std::io::stderr(), "{}", msg);
        return 2;
    }
    0
}

fn mode_compact(key: &str) {
    let mut st = read_state(key);
    if st.contains_key("ctx") || st.contains_key("prompts") {
        st.insert("ctx".to_string(), Value::from(0));
        st.insert("prompts".to_string(), Value::from(0));
        write_state(key, st);
    }
}

fn mode_response(data: &Value, key: &str) {
    let n = response_tokens(data);
    if n > 0 {
        let mut st = read_state(key);
        st.insert("ctx".to_string(), Value::from(n));
        write_state(key, st);
    }
}

fn main() {
    // Fail open: any panic (bad input, closed pipe) exits 0 silently.
    std::panic::set_hook(Box::new(|_| {}));
    let code = std::panic::catch_unwind(run).unwrap_or(0);
    std::process::exit(code);
}

fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mode = match cli.mode.as_deref() {
        Some(m) if ["prompt", "subagent", "compact", "response"].contains(&m) => m,
        _ => return 0,
    };

    let mut stdin_buf = String::new();
    if std::io::stdin().read_to_string(&mut stdin_buf).is_err() {
        return 0;
    }
    let data: Value = match serde_json::from_str::<Value>(&stdin_buf) {
        Ok(v) if v.is_object() => v,
        _ => return 0,
    };

    let key = conv_key(&data, mode);

    match mode {
        "prompt" => {
            mode_prompt(&data, &key);
            0
        }
        "subagent" => mode_subagent(&data, &key),
        "compact" => {
            mode_compact(&key);
            0
        }
        "response" => {
            mode_response(&data, &key);
            0
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_parent_does_not_win() {
        let d = json!({"parent_conversation_id": "", "conversation_id": "abc"});
        assert_eq!(conv_key(&d, "subagent"), "abc");
        assert_eq!(conv_key(&d, "prompt"), "abc");
        let d = json!({"parent_conversation_id": "par", "conversation_id": "abc"});
        assert_eq!(conv_key(&d, "subagent"), "par");
        assert_eq!(conv_key(&d, "prompt"), "abc");
        let d = json!({"parent_conversation_id": "", "conversation_id": ""});
        assert_eq!(conv_key(&d, "subagent"), "unknown");
    }

    #[test]
    fn empty_conv_ids_skip_app_db() {
        let d = json!({"parent_conversation_id": "", "conversation_id": ""});
        assert_eq!(app_context(&d), 0);
    }

    #[test]
    fn empty_tmpdir_falls_back_to_tmp() {
        assert_eq!(state_dir_from(Some(String::new())), PathBuf::from("/tmp/cursor-guard"));
        assert_eq!(state_dir_from(None), PathBuf::from("/tmp/cursor-guard"));
        assert_eq!(state_dir_from(Some("/x".into())), PathBuf::from("/x/cursor-guard"));
    }

    #[test]
    fn reverse_lines_is_newest_first_and_lazy() {
        let p = env::temp_dir().join(format!("gc-rev-{}.txt", std::process::id()));
        let mut body = String::new();
        for i in 0..20000 {
            body.push_str(&format!("line{}\n\n", i));
        }
        fs::write(&p, body).unwrap();
        let mut it = reverse_lines(&p);
        assert_eq!(it.next().as_deref(), Some("line19999"));
        assert_eq!(it.next().as_deref(), Some("line19998"));
        assert_eq!(it.count(), 19998);
        let _ = fs::remove_file(&p);
    }
}