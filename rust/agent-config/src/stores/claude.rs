//! Claude Code store: `~/.claude/projects/*/*.jsonl` and `*/<session>/subagents/*.jsonl`,
//! plus the desktop app's session records for titles.
//!
//! Files are streamed one line at a time. A line that does not parse (a transcript still being
//! written ends in a half line) is skipped. Tool output is never indexed; tool error text is.

use super::{Doc, Skip};
use crate::pycompat::{fname, list_dir, parse_ts_micros, truthy};
use crate::redact::redact;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Longest text kept per document, in chars.
const MAX_TEXT: usize = 20_000;
/// Longest tool error text read before redaction, in chars.
const MAX_ERROR: usize = 1_000;
const INTERRUPT: &str = "[Request interrupted by user";

static PROJ_WT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"--claude-worktrees-.*").unwrap());
static WORK_PFX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^work-").unwrap());

/// `~/.claude/projects`.
pub fn claude_root(home: &Path) -> PathBuf {
    home.join(".claude").join("projects")
}

/// The desktop app's session records (title, archived, effort).
pub fn desktop_dir(home: &Path) -> PathBuf {
    home.join("Library").join("Application Support").join("Claude").join("claude-code-sessions")
}

/// What the desktop app knows about one session.
pub struct DesktopMeta {
    pub title: Value,
    pub archived: bool,
    pub effort: Value,
}

fn val<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}

fn key_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Session id (`cliSessionId`) to its desktop record. Reads `local_*.json` two levels under `desktop`.
pub fn desktop_meta(desktop: &Path) -> HashMap<String, DesktopMeta> {
    let mut meta = HashMap::new();
    for d1 in list_dir(desktop) {
        for d2 in list_dir(&d1) {
            for f in list_dir(&d2) {
                let n = fname(&f);
                if !(n.starts_with("local_") && n.ends_with(".json")) {
                    continue;
                }
                let Ok(txt) = fs::read(&f) else { continue };
                let Ok(d) = serde_json::from_slice::<Value>(&txt) else { continue };
                let sid = val(&d, "cliSessionId");
                if truthy(sid) {
                    meta.insert(
                        key_str(sid),
                        DesktopMeta {
                            title: val(&d, "title").clone(),
                            archived: truthy(val(&d, "isArchived")),
                            effort: val(&d, "effort").clone(),
                        },
                    );
                }
            }
        }
    }
    meta
}

/// Short project name from a `~/.claude/projects` folder name.
pub fn project_name(home: &str, folder: &str) -> String {
    let p = PROJ_WT.replace(folder, "").into_owned();
    let hm = format!("-{}", home.trim_matches('/').replace('/', "-"));
    if p == hm {
        return "~".into();
    }
    if p.contains("scratch-workspaces") {
        return "scratch".into();
    }
    if p.starts_with("-private-") {
        return "tmp".into();
    }
    let p = if let Some(r) = p.strip_prefix(&format!("{hm}-")) { r.to_string() } else { p.trim_start_matches('-').to_string() };
    let r = WORK_PFX.replace(&p, "").into_owned();
    if r.is_empty() { "?".into() } else { r }
}

struct FileCtx<'a> {
    file: &'a str,
    session: &'a str,
    project: &'a str,
    title: &'a str,
    is_sub: bool,
}

pub fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter(|x| x.is_object() && val(x, "type").as_str() == Some("text"))
            .map(|x| val(x, "text").as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn doc(c: &FileCtx, time: i64, role: &str, text: &str) -> Option<Doc> {
    if text.trim().is_empty() {
        return None;
    }
    let text = clip(&redact(text), MAX_TEXT);
    Some(Doc {
        source: "claude",
        file: c.file.to_string(),
        session: c.session.to_string(),
        project: c.project.to_string(),
        title: redact(c.title).into_owned(),
        time,
        role: role.to_string(),
        text,
    })
}

/// Turn one transcript line into documents. `names` maps tool_use ids to tool names.
fn line_docs(o: &Value, time: i64, c: &FileCtx, names: &mut HashMap<String, String>, out: &mut dyn FnMut(Doc)) {
    let msg = val(o, "message");
    let content = val(msg, "content");
    match val(o, "type").as_str().unwrap_or("") {
        "assistant" => {
            let Some(blocks) = content.as_array() else { return };
            for b in blocks {
                match val(b, "type").as_str() {
                    Some("text") => {
                        if let Some(d) = doc(c, time, "assistant", val(b, "text").as_str().unwrap_or("")) {
                            out(d);
                        }
                    }
                    Some("tool_use") => {
                        let name = val(b, "name").as_str().unwrap_or("?");
                        names.insert(key_str(val(b, "id")), name.to_string());
                        let input = val(b, "input");
                        let arg = ["command", "file_path"].iter().find_map(|k| val(input, k).as_str()).unwrap_or("");
                        if let Some(d) = doc(c, time, "tool", &format!("{name} {arg}")) {
                            out(d);
                        }
                    }
                    _ => {}
                }
            }
        }
        "user" => {
            let is_result = |x: &Value| x.is_object() && val(x, "type").as_str() == Some("tool_result");
            if let Some(arr) = content.as_array()
                && arr.iter().any(is_result)
            {
                for x in arr.iter().filter(|x| is_result(x)) {
                    if !truthy(val(x, "is_error")) {
                        continue;
                    }
                    let name = names.get(&key_str(val(x, "tool_use_id"))).map(String::as_str).unwrap_or("?");
                    // Redact before the cut: a cut can split a token below the size the redactor needs.
                    let body = clip(&redact(&text_of(val(x, "content"))), MAX_ERROR);
                    if let Some(d) = doc(c, time, "tool_error", &format!("{name}: {body}")) {
                        out(d);
                    }
                }
                return;
            }
            if truthy(val(o, "isMeta")) || truthy(val(o, "isCompactSummary")) || (truthy(val(o, "isSidechain")) && !c.is_sub) {
                return;
            }
            let t = text_of(content);
            if t.starts_with(INTERRUPT) || (t.trim_start().starts_with('<') && !t.contains("command-name")) {
                return;
            }
            if let Some(d) = doc(c, time, "user", &t) {
                out(d);
            }
        }
        _ => {}
    }
}

/// Stream one transcript. Err means the file could not be read to the end.
fn read_file(path: &Path, c: &FileCtx, out: &mut dyn FnMut(Doc)) -> std::io::Result<()> {
    let mut r = BufReader::new(File::open(path)?);
    let mut buf: Vec<u8> = Vec::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut time = 0i64;
    loop {
        buf.clear();
        if r.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        let line = String::from_utf8_lossy(&buf);
        let Ok(o) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if !o.is_object() {
            continue;
        }
        if let Some(s) = val(&o, "timestamp").as_str()
            && let Some((us, _)) = parse_ts_micros(s)
        {
            time = us.div_euclid(1_000_000);
        }
        line_docs(&o, time, c, &mut names, out);
    }
}

fn stamp(path: &Path) -> Option<(u64, i64)> {
    let m = fs::metadata(path).ok()?;
    let mtime = match m.modified().ok()?.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };
    Some((m.len(), mtime))
}

fn is_jsonl(p: &Path) -> bool {
    p.extension().is_some_and(|x| x == "jsonl")
}

/// Read every Claude Code transcript whose (size, mtime) differs from `since`, in path order.
/// Returns the (size, mtime) of every transcript found, changed or not. A file that cannot be
/// read to the end is left out of the result, so the next run reads it again.
pub fn read_claude(
    home: &Path,
    since: &BTreeMap<PathBuf, (u64, i64)>,
    out: &mut dyn FnMut(Doc),
) -> Result<BTreeMap<PathBuf, (u64, i64)>, Skip> {
    let root = claude_root(home);
    if !root.is_dir() {
        return Err(Skip::Missing);
    }
    let titles = desktop_meta(&desktop_dir(home));
    let home_s = home.to_string_lossy().into_owned();
    let mut state = BTreeMap::new();
    let mut folders = list_dir(&root);
    folders.sort();
    for pdir in folders.into_iter().filter(|p| p.is_dir()) {
        let project = project_name(&home_s, &fname(&pdir));
        let mut files: Vec<(PathBuf, String, bool)> = Vec::new();
        for f in list_dir(&pdir) {
            if f.is_file() && is_jsonl(&f) {
                let sid = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                files.push((f, sid, false));
            } else if f.is_dir() {
                let sid = fname(&f);
                for s in list_dir(&f.join("subagents")) {
                    if is_jsonl(&s) {
                        files.push((s, sid.clone(), true));
                    }
                }
            }
        }
        files.sort();
        for (path, sid, is_sub) in files {
            let Some(st) = stamp(&path) else { continue };
            if since.get(&path) == Some(&st) {
                state.insert(path, st);
                continue;
            }
            let title = titles.get(&sid).and_then(|m| m.title.as_str()).unwrap_or("");
            let file = path.to_string_lossy();
            let ctx = FileCtx { file: &file, session: &sid, project: &project, title, is_sub };
            if read_file(&path, &ctx, out).is_ok() {
                state.insert(path, st);
            }
        }
    }
    Ok(state)
}
