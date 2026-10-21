//! What the session scanners (`hurry_scan`, `harness_stats`, `whatsleft_scan`) share: where Claude
//! Code keeps its transcripts, how to pick the session to read, an insertion-ordered counter and
//! the Python-style JSON text of a value.

use crate::paths;
use crate::pycompat::{fname, json_str, list_dir, mtime, py_float};
use regex::Regex;
use serde_json::Value;
use std::borrow::Borrow;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// A counter that remembers the order its keys first came in, like `collections.Counter`.
#[derive(Clone, Debug)]
pub struct Counter<K: Clone + Eq + Hash> {
    keys: Vec<K>,
    idx: HashMap<K, usize>,
    vals: Vec<i64>,
}

impl<K: Clone + Eq + Hash> Default for Counter<K> {
    fn default() -> Self {
        Self { keys: vec![], idx: HashMap::new(), vals: vec![] }
    }
}

impl<K: Clone + Eq + Hash> Counter<K> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, k: K, n: i64) {
        if let Some(&i) = self.idx.get(&k) {
            self.vals[i] += n;
        } else {
            self.idx.insert(k.clone(), self.keys.len());
            self.keys.push(k);
            self.vals.push(n);
        }
    }

    /// `c[k] = n`: a known key keeps its place.
    pub fn set(&mut self, k: K, n: i64) {
        if let Some(&i) = self.idx.get(&k) {
            self.vals[i] = n;
        } else {
            self.add(k, n);
        }
    }

    pub fn get<Q: Hash + Eq + ?Sized>(&self, k: &Q) -> i64
    where
        K: Borrow<Q>,
    {
        self.idx.get(k).map_or(0, |&i| self.vals[i])
    }

    pub fn contains<Q: Hash + Eq + ?Sized>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.idx.contains_key(k)
    }

    /// Keys with their counts, in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, i64)> {
        self.keys.iter().zip(self.vals.iter().copied())
    }

    pub fn items(&self) -> Vec<(K, i64)> {
        self.iter().map(|(k, v)| (k.clone(), v)).collect()
    }

    pub fn update(&mut self, other: &Self) {
        for (k, v) in other.iter() {
            self.add(k.clone(), v);
        }
    }

    /// Descending by count, ties in insertion order; `None` = all.
    pub fn most_common(&self, n: Option<i64>) -> Vec<(K, i64)> {
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

    pub fn total(&self) -> i64 {
        self.vals.iter().sum()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// `json.dumps(v)` with the Python defaults: `ensure_ascii`, ", " and ": ".
pub fn py_dumps(v: &Value) -> String {
    let mut s = String::new();
    dump(v, &mut s);
    s
}

/// Append the Python-style JSON text of `v` to `out`.
pub fn dump(v: &Value, out: &mut String) {
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

static NON_ALNUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9]").expect("a fixed pattern"));

/// Where `find_session` looks when no session is named and `CLAUDE_CODE_SESSION_ID` finds nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Search {
    /// The newest transcript of the current folder's project.
    ThisProject,
    /// The newest transcript of the current folder's project, else the newest of all.
    ThisProjectThenAll,
    /// The newest transcript of all projects.
    All,
}

/// `~/.claude/projects`.
pub fn projects_dir() -> PathBuf {
    paths::home().join(".claude").join("projects")
}

fn is_jsonl(p: &Path) -> bool {
    p.extension().is_some_and(|x| x == "jsonl")
}

/// Transcripts `<projects>/<project>/<prefix>*.jsonl`, or exactly `<prefix>.jsonl`.
fn session_hits(prefix: &str, exact: bool) -> Vec<PathBuf> {
    let exact_name = format!("{prefix}.jsonl");
    let mut hits = vec![];
    for dir in list_dir(&projects_dir()) {
        for f in list_dir(&dir) {
            let name = fname(&f);
            let ok = if exact { name == exact_name } else { name.starts_with(prefix) && is_jsonl(&f) };
            if ok {
                hits.push(f);
            }
        }
    }
    hits
}

/// The most recently changed file; the first one wins a tie.
fn newest(files: Vec<PathBuf>) -> Option<PathBuf> {
    let mut best: Option<(f64, PathBuf)> = None;
    for p in files {
        let m = mtime(&p).unwrap_or(0.0);
        if best.as_ref().is_none_or(|(bm, _)| m > *bm) {
            best = Some((m, p));
        }
    }
    best.map(|(_, p)| p)
}

/// The transcript to read: a file path or a session id prefix from the command line, else the
/// current session (`CLAUDE_CODE_SESSION_ID`), else the newest one `search` allows.
pub fn find_session(arg: Option<&str>, search: Search) -> Option<PathBuf> {
    if let Some(a) = arg.filter(|a| !a.is_empty()) {
        if Path::new(a).is_file() {
            return Some(PathBuf::from(a));
        }
        return session_hits(a, false).into_iter().next();
    }
    if let Ok(sid) = std::env::var("CLAUDE_CODE_SESSION_ID")
        && !sid.is_empty()
        && let Some(hit) = session_hits(&sid, true).into_iter().next()
    {
        return Some(hit);
    }
    if search != Search::All {
        let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let project = projects_dir().join(NON_ALNUM.replace_all(&cwd, "-").as_ref());
        let mine: Vec<PathBuf> = list_dir(&project).into_iter().filter(|p| is_jsonl(p)).collect();
        if !mine.is_empty() || search == Search::ThisProject {
            return newest(mine);
        }
    }
    newest(session_hits("", false))
}
