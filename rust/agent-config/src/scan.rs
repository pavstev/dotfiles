//! What the session scanners (`hurry_scan`, `whatsleft_scan`) share: where Claude Code keeps its
//! transcripts and how to pick the session to read.

use crate::paths;
use crate::pycompat::{fname, list_dir, mtime};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

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
