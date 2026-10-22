//! Per-project leftovers: optional work parked for later.
//!
//! The store is `<main checkout>/.agents/leftovers.jsonl`, an append-only log of JSON lines, one
//! event per line (`add`, `done`, `drop`). The state is the fold of the events in order. Only this
//! module writes the file: JSON survives a model's edits better than Markdown. A call from inside a
//! worktree reaches the main checkout's store.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const VERSION: u32 = 1;
/// Closed items older than this many days go away on `compact`.
const KEEP_DAYS: i64 = 30;
/// Shortest id prefix that may stand for an id.
const MIN_PREFIX: usize = 4;
const IGNORE_LINE: &str = ".agents/";
/// The store, relative to the main checkout, as git reads it.
const STORE_REL: &str = ".agents/leftovers.jsonl";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not inside a git repository: {}", .0.display())]
    NotGit(PathBuf),
    #[error("no leftover with id {0}")]
    Unknown(String),
    #[error("id prefix {0} fits more than one leftover")]
    Ambiguous(String),
    #[error("{0} is already {1}")]
    Closed(String, Status),
    #[error("{}: {}", .0.display(), .1)]
    Io(PathBuf, io::Error),
}

impl Error {
    /// The exit code a command line ends with.
    pub fn code(&self) -> u8 {
        if matches!(self, Self::NotGit(_)) { 2 } else { 1 }
    }
}

fn io_at(path: &Path) -> impl Fn(io::Error) -> Error + '_ {
    |e| Error::Io(path.to_path_buf(), e)
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Event {
    Add {
        id: String,
        at: String,
        text: String,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        from: String,
    },
    Done {
        id: String,
        at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Drop {
        id: String,
        at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
}

/// The version field and the event, side by side on one line.
#[derive(Serialize, Deserialize)]
struct Line<E> {
    v: u32,
    #[serde(flatten)]
    event: E,
}

/// The event on one line of the log, or `None` for a bad line, an unknown op or another version.
pub fn parse_line(line: &str) -> Option<Event> {
    serde_json::from_str::<Line<Event>>(line).ok().filter(|l| l.v == VERSION).map(|l| l.event)
}

/// The event as a log line, newline included.
pub fn encode(event: &Event) -> String {
    let mut s = serde_json::to_string(&Line { v: VERSION, event }).expect("an event is plain data");
    s.push('\n');
    s
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Done,
    Dropped,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::Dropped => "dropped",
        })
    }
}

/// One leftover after the fold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: String,
    pub at: String,
    pub text: String,
    pub tags: Vec<String>,
    pub from: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Item {
    fn add_event(&self) -> Event {
        Event::Add {
            id: self.id.clone(),
            at: self.at.clone(),
            text: self.text.clone(),
            tags: self.tags.clone(),
            from: self.from.clone(),
        }
    }

    fn close_event(&self) -> Option<Event> {
        let (id, at, note) = (self.id.clone(), self.closed_at.clone()?, self.note.clone());
        match self.status {
            Status::Open => None,
            Status::Done => Some(Event::Done { id, at, note }),
            Status::Dropped => Some(Event::Drop { id, at, note }),
        }
    }
}

/// The state after the events, in the order of their `add`. A repeated id, a close of an unknown or
/// a closed item and the rest of a bad log are skipped.
pub fn fold(events: impl IntoIterator<Item = Event>) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for event in events {
        let (id, status, at, note) = match event {
            Event::Add { id, at, text, tags, from } => {
                if !index.contains_key(&id) {
                    index.insert(id.clone(), items.len());
                    items.push(Item { id, at, text, tags, from, status: Status::Open, closed_at: None, note: None });
                }
                continue;
            }
            Event::Done { id, at, note } => (id, Status::Done, at, note),
            Event::Drop { id, at, note } => (id, Status::Dropped, at, note),
        };
        if let Some(item) = index.get(&id).map(|&i| &mut items[i]).filter(|i| i.status == Status::Open) {
            item.status = status;
            item.closed_at = Some(at);
            item.note = note;
        }
    }
    items
}

/// The item that `prefix` names: the exact id, else a unique prefix of `MIN_PREFIX` hex or more.
pub fn find<'a>(items: &'a [Item], prefix: &str) -> Result<&'a Item, Error> {
    if let Some(item) = items.iter().find(|i| i.id == prefix) {
        return Ok(item);
    }
    if prefix.len() < MIN_PREFIX || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Unknown(prefix.to_string()));
    }
    let mut hits = items.iter().filter(|i| i.id.starts_with(prefix));
    match (hits.next(), hits.next()) {
        (Some(item), None) => Ok(item),
        (Some(_), Some(_)) => Err(Error::Ambiguous(prefix.to_string())),
        _ => Err(Error::Unknown(prefix.to_string())),
    }
}

/// `2026-10-07` of an RFC 3339 stamp.
pub fn date_of(at: &str) -> &str {
    at.get(..10).unwrap_or(at)
}

/// The lines `leftovers list` prints: `<id>  <date>  <text>`; with `all`, closed items follow their
/// place in the log and carry their state.
pub fn list_lines(items: &[Item], all: bool) -> Vec<String> {
    items
        .iter()
        .filter(|i| all || i.status == Status::Open)
        .map(|i| {
            let text = i.text.replace(['\n', '\r'], " ");
            let state = i.closed_at.as_deref().map(|closed| {
                let note = i.note.as_deref().map(|n| format!(": {}", n.replace(['\n', '\r'], " ")));
                format!("  [{} {}{}]", i.status, date_of(closed), note.unwrap_or_default())
            });
            format!("{}  {}  {}{}", i.id, date_of(&i.at), text, state.unwrap_or_default())
        })
        .collect()
}

/// Now, as `2026-10-21T08:30:00Z`.
pub fn now() -> String {
    jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A new id: 8 hex from sha2 over the time, the text, the process id and a counter (the counter
/// keeps two equal adds of one run apart).
pub fn new_id(at: &str, text: &str) -> String {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let mut sha = Sha256::new();
    sha.update(at.as_bytes());
    sha.update(text.as_bytes());
    sha.update(std::process::id().to_le_bytes());
    sha.update(COUNT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let sum = sha.finalize();
    format!("{:08x}", u32::from_be_bytes([sum[0], sum[1], sum[2], sum[3]]))
}

/// Run `git -C dir args`: stdout without the trailing newline, `None` when git fails.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output();
    let out = out.ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\n', '\r']).to_string())
}

/// What `add` did.
#[derive(Debug)]
pub struct Added {
    pub id: String,
    /// The `.agents/` line went into the `.gitignore` of the main checkout just now.
    pub ignore_added: bool,
}

/// The leftovers of one project.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    path: PathBuf,
    from: String,
}

impl Store {
    /// The store of the project that holds `dir`. A worktree reaches its main checkout.
    pub fn at(dir: &Path) -> Result<Self, Error> {
        let not_git = || Error::NotGit(dir.to_path_buf());
        let args = ["rev-parse", "--path-format=absolute", "--git-common-dir", "--show-toplevel"];
        let out = git(dir, &args).ok_or_else(not_git)?;
        let mut lines = out.lines();
        let (Some(common), Some(top)) = (lines.next(), lines.next()) else { return Err(not_git()) };
        let (common, top) = (PathBuf::from(common), PathBuf::from(top));
        // A common dir named `.git` sits in the main checkout. Elsewhere (a submodule) use this checkout.
        let root = match common.parent() {
            Some(parent) if common.file_name().is_some_and(|n| n == ".git") => parent.to_path_buf(),
            _ => top.clone(),
        };
        let from = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Self { path: root.join(".agents").join("leftovers.jsonl"), root, from })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The main checkout.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The events in the file; none when it does not exist.
    pub fn events(&self) -> Vec<Event> {
        let Ok(bytes) = std::fs::read(&self.path) else { return Vec::new() };
        String::from_utf8_lossy(&bytes).lines().filter_map(parse_line).collect()
    }

    pub fn items(&self) -> Vec<Item> {
        fold(self.events())
    }

    pub fn open_items(&self) -> Vec<Item> {
        self.items().into_iter().filter(|i| i.status == Status::Open).collect()
    }

    /// Append one event with a single write, so two sessions can add at once.
    pub fn append(&self, event: &Event) -> Result<(), Error> {
        let fail = io_at(&self.path);
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io_at(parent))?;
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path).map_err(&fail)?;
        file.write_all(encode(event).as_bytes()).map_err(&fail)
    }

    /// Make git ignore the store: append `.agents/` to the `.gitignore` of the main checkout when it
    /// does not. True when the line was added.
    pub fn ensure_ignored(&self) -> Result<bool, Error> {
        if git(&self.root, &["check-ignore", "-q", STORE_REL]).is_some() {
            return Ok(false);
        }
        let file = self.root.join(".gitignore");
        let mut text = std::fs::read_to_string(&file).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(IGNORE_LINE);
        text.push('\n');
        std::fs::write(&file, text).map_err(io_at(&file))?;
        Ok(true)
    }

    /// Add a leftover from this checkout, now.
    pub fn add(&self, text: &str, tags: &[String]) -> Result<Added, Error> {
        let at = now();
        let id = new_id(&at, text);
        let event =
            Event::Add { id: id.clone(), at, text: text.to_string(), tags: tags.to_vec(), from: self.from.clone() };
        self.append(&event)?;
        Ok(Added { id, ignore_added: self.ensure_ignored()? })
    }

    /// Close the open item that `prefix` names as done or dropped.
    pub fn close(&self, prefix: &str, status: Status, note: Option<&str>) -> Result<Item, Error> {
        let items = self.items();
        let item = find(&items, prefix)?;
        if item.status != Status::Open {
            return Err(Error::Closed(item.id.clone(), item.status));
        }
        let (id, at, note) = (item.id.clone(), now(), note.map(str::to_string));
        let event = if status == Status::Dropped { Event::Drop { id, at, note } } else { Event::Done { id, at, note } };
        self.append(&event)?;
        let items = fold(self.events());
        find(&items, &item.id).cloned()
    }

    /// Rewrite the file with the open items and those closed in the last 30 days (temp file, then
    /// rename). Returns (kept, removed). An add that lands during the rewrite is lost.
    pub fn compact(&self) -> Result<(usize, usize), Error> {
        let items = self.items();
        let cutoff = jiff::Timestamp::now().checked_sub(jiff::SignedDuration::from_hours(KEEP_DAYS * 24)).ok();
        let recent = |item: &Item| {
            let closed = item.closed_at.as_deref().and_then(|c| c.parse::<jiff::Timestamp>().ok());
            // A close stamp that does not parse is kept, not lost.
            closed.is_none_or(|c| cutoff.is_none_or(|cut| c >= cut))
        };
        let keep: Vec<&Item> = items.iter().filter(|i| i.status == Status::Open || recent(i)).collect();
        let mut text = String::new();
        for item in &keep {
            text.push_str(&encode(&item.add_event()));
            if let Some(close) = item.close_event() {
                text.push_str(&encode(&close));
            }
        }
        let dir = self.path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(dir).map_err(io_at(dir))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(io_at(dir))?;
        tmp.write_all(text.as_bytes()).map_err(io_at(&self.path))?;
        tmp.persist(&self.path).map_err(|e| Error::Io(self.path.clone(), e.error))?;
        Ok((keep.len(), items.len() - keep.len()))
    }
}

/// One open line of the old shared Markdown file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldItem {
    pub date: String,
    pub name: String,
    pub text: String,
}

/// The open items of the old file (`- [ ] 2026-10-07 · **name** · text`), and the open-looking lines
/// that do not fit that shape. Closed lines (`- [x]`) and everything else are skipped.
pub fn parse_old(text: &str) -> (Vec<OldItem>, Vec<String>) {
    let shape = regex::Regex::new(r"^- \[ \] (\d{4}-\d{2}-\d{2}) · \*\*(.+?)\*\* · (.+)$").expect("a fixed pattern");
    let (mut items, mut bad) = (Vec::new(), Vec::new());
    for line in text.lines().map(str::trim_end).filter(|l| l.starts_with("- [ ]")) {
        match shape.captures(line) {
            Some(c) => items.push(OldItem { date: c[1].to_string(), name: c[2].to_string(), text: c[3].to_string() }),
            None => bad.push(line.to_string()),
        }
    }
    (items, bad)
}

/// The folder that `name` stands for. `--map` entries win. Else a git checkout called `name` directly
/// under one of `bases`, else a worktree called `name` under `<checkout>/.claude/worktrees/` of a
/// checkout directly under a base. A name like `repo/sub/dir` falls back to `repo`.
pub fn resolve_repo(name: &str, maps: &[(String, PathBuf)], bases: &[PathBuf]) -> Option<PathBuf> {
    if let Some((_, path)) = maps.iter().find(|(n, _)| n == name) {
        return Some(path.clone());
    }
    let first = name.split('/').next().unwrap_or(name);
    let tries = if first == name { vec![name] } else { vec![name, first] };
    let is_checkout = |p: &Path| p.join(".git").exists();
    for name in tries {
        for base in bases {
            let direct = base.join(name);
            if is_checkout(&direct) {
                return Some(direct);
            }
            let Ok(children) = std::fs::read_dir(base) else { continue };
            let mut dirs: Vec<PathBuf> = children.filter_map(|e| e.ok().map(|e| e.path())).collect();
            dirs.sort();
            let tree = |d: &PathBuf| d.join(".claude").join("worktrees").join(name);
            if let Some(found) = dirs.iter().map(tree).find(|t| is_checkout(t)) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(id: &str, text: &str) -> Event {
        Event::Add {
            id: id.into(),
            at: "2026-10-07T00:00:00Z".into(),
            text: text.into(),
            tags: vec![],
            from: "r".into(),
        }
    }

    fn close(op: Status, id: &str, note: Option<&str>) -> Event {
        let (id, at, note) = (id.to_string(), "2026-10-08T00:00:00Z".to_string(), note.map(str::to_string));
        if op == Status::Done { Event::Done { id, at, note } } else { Event::Drop { id, at, note } }
    }

    #[test]
    fn fold_applies_events_in_order() {
        let items = fold([
            add("aaaa1111", "one"),
            add("bbbb2222", "two"),
            close(Status::Done, "aaaa1111", Some("shipped")),
            close(Status::Dropped, "bbbb2222", None),
            close(Status::Done, "bbbb2222", None), // second close of one item is ignored
            close(Status::Done, "cccc3333", None), // unknown id is ignored
            add("aaaa1111", "again"),              // repeated id is ignored
        ]);
        assert_eq!(items.len(), 2);
        assert_eq!(
            (items[0].status, items[0].note.as_deref(), items[0].text.as_str()),
            (Status::Done, Some("shipped"), "one")
        );
        assert_eq!((items[1].status, items[1].closed_at.as_deref()), (Status::Dropped, Some("2026-10-08T00:00:00Z")));
    }

    #[test]
    fn bad_lines_and_unknown_ops_are_skipped() {
        let good = encode(&add("aaaa1111", "one"));
        assert_eq!(parse_line(good.trim()), Some(add("aaaa1111", "one")));
        assert_eq!(parse_line("not json"), None);
        assert_eq!(parse_line(r#"{"v":1,"op":"frobnicate","id":"x","at":"y"}"#), None);
        assert_eq!(parse_line(r#"{"v":2,"op":"drop","id":"x","at":"y"}"#), None);
        assert_eq!(parse_line(r#"{"op":"drop","id":"x","at":"y"}"#), None);
    }

    #[test]
    fn a_line_has_the_version_first_and_no_empty_note() {
        let line = encode(&close(Status::Done, "aaaa1111", None));
        assert_eq!(line, "{\"v\":1,\"op\":\"done\",\"id\":\"aaaa1111\",\"at\":\"2026-10-08T00:00:00Z\"}\n");
        assert!(encode(&add("aaaa1111", "x")).starts_with("{\"v\":1,\"op\":\"add\",\"id\":\"aaaa1111\",\"at\":"));
    }

    #[test]
    fn id_prefix_needs_four_hex_and_a_single_match() {
        let items = fold([add("abcd1234", "a"), add("abcd5678", "b"), add("ef012345", "c")]);
        assert_eq!(find(&items, "ef01").unwrap().text, "c");
        assert_eq!(find(&items, "abcd1234").unwrap().text, "a");
        assert!(matches!(find(&items, "abcd"), Err(Error::Ambiguous(_))));
        assert!(matches!(find(&items, "ef0"), Err(Error::Unknown(_)))); // too short
        assert!(matches!(find(&items, "zzzz"), Err(Error::Unknown(_))));
        assert!(matches!(find(&items, "0000"), Err(Error::Unknown(_))));
    }

    #[test]
    fn ids_are_eight_hex_and_differ() {
        let (a, b) = (new_id("2026-10-07T00:00:00Z", "x"), new_id("2026-10-07T00:00:00Z", "x"));
        assert!(a.len() == 8 && a.bytes().all(|c| c.is_ascii_hexdigit()), "{a}");
        assert_ne!(a, b);
    }

    #[test]
    fn list_lines_show_open_and_with_all_the_closed() {
        let items =
            fold([add("aaaa1111", "one\ntwo"), add("bbbb2222", "b"), close(Status::Done, "bbbb2222", Some("ok"))]);
        assert_eq!(list_lines(&items, false), ["aaaa1111  2026-10-07  one two"]);
        assert_eq!(list_lines(&items, true)[1], "bbbb2222  2026-10-07  b  [done 2026-10-08: ok]");
    }

    #[test]
    fn old_file_parsing_keeps_open_items_and_reports_odd_lines() {
        let text = "# Leftovers\n\n- [x] 2026-10-07 · **site** · closed one\n\
                    - [ ] 2026-10-08 · **powertools/hosts/x1** · x1: a · b\n\
                    - [ ] no shape here\n- [ ] 2026-10-09 · **dotfiles** · last\r\n";
        let (items, bad) = parse_old(text);
        assert_eq!(
            items,
            [
                OldItem { date: "2026-10-08".into(), name: "powertools/hosts/x1".into(), text: "x1: a · b".into() },
                OldItem { date: "2026-10-09".into(), name: "dotfiles".into(), text: "last".into() },
            ]
        );
        assert_eq!(bad, ["- [ ] no shape here"]);
    }

    #[test]
    fn resolve_repo_prefers_map_then_checkout_then_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, work) = (tmp.path().to_path_buf(), tmp.path().join("work"));
        let mk = |p: PathBuf| std::fs::create_dir_all(p.join(".git")).unwrap();
        mk(home.join("alpha"));
        mk(work.join("beta"));
        mk(home.join("alpha").join(".claude").join("worktrees").join("wt-1"));
        std::fs::create_dir_all(home.join("plain")).unwrap(); // not a checkout
        let bases = [home.clone(), work.clone()];
        let maps = [("alpha".to_string(), PathBuf::from("/elsewhere"))];
        assert_eq!(resolve_repo("alpha", &maps, &bases), Some(PathBuf::from("/elsewhere")));
        assert_eq!(resolve_repo("alpha", &[], &bases), Some(home.join("alpha")));
        assert_eq!(resolve_repo("beta", &[], &bases), Some(work.join("beta")));
        let wt = home.join("alpha").join(".claude").join("worktrees").join("wt-1");
        assert_eq!(resolve_repo("wt-1", &[], &bases), Some(wt));
        assert_eq!(resolve_repo("alpha/hosts/x1", &[], &bases), Some(home.join("alpha")));
        assert_eq!(resolve_repo("plain", &[], &bases), None);
        assert_eq!(resolve_repo("nope", &[], &bases), None);
    }
}
