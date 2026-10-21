//! `sessions`: search past Claude Code sessions with a tantivy index in the user cache dir.
//!
//! `sessions index [--rebuild]` reads new and changed transcripts into the index.
//! `sessions find QUERY [--project P] [--since YYYY-MM-DD] [--source S] [--limit N]` updates the
//! index (waiting up to 5 s for the writer lock), then prints one block per session that matches.
//!
//! Opencode and Cursor are not supported yet: `--source opencode|cursor` prints one line.

// println! that stops quietly on a closed pipe (`sessions find x | head`).
#[macro_use]
#[path = "../quiet_out.rs"]
mod quiet_out;

use agent_config::pycompat::{civil_from_days, days_from_civil, days_in_month};
use agent_config::redact::redact;
use agent_config::stores::{Doc, Skip, read_claude};
use clap::{Parser, Subcommand};
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fs;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::directory::error::LockError;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer};
use tantivy::{Index, IndexWriter, TantivyDocument, TantivyError, Term};

type Res<T> = Result<T, Box<dyn Error>>;

/// Writer heap. One indexing thread, so the budget is not split.
const HEAP: usize = 50_000_000;
/// How long `index` and `find` wait for the writer lock held by another run: 5 s, or
/// `SESSIONS_LOCK_WAIT_MS` (the tests set it).
fn lock_wait() -> Duration {
    let ms = std::env::var("SESSIONS_LOCK_WAIT_MS").ok().and_then(|v| v.parse().ok());
    ms.map_or(Duration::from_secs(5), Duration::from_millis)
}
const STATE_VERSION: u64 = 1;
const SNIPPET_CHARS: usize = 200;
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;
const TOKENIZER: &str = "en_stem";

#[derive(Parser)]
#[command(name = "sessions", about = "Search past Claude Code sessions")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Read new and changed transcripts into the index.
    Index {
        /// Drop every indexed document and read everything again.
        #[arg(long)]
        rebuild: bool,
    },
    /// Search the index (updated first).
    Find {
        /// Words to look for. Every word must match; with no such session, any word may.
        #[arg(required = true)]
        query: Vec<String>,
        /// Only this project (the short name shown in the hit line).
        #[arg(long)]
        project: Option<String>,
        /// Only messages from this day on, UTC (YYYY-MM-DD).
        #[arg(long)]
        since: Option<String>,
        /// Only this source.
        #[arg(long, value_parser = ["claude", "opencode", "cursor"])]
        source: Option<String>,
        /// How many sessions to show (default 10, at most 50).
        #[arg(long)]
        limit: Option<usize>,
    },
}

struct Fields {
    source: Field,
    file: Field,
    session: Field,
    project: Field,
    title: Field,
    text: Field,
    time: Field,
    role: Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let words = || {
        TextOptions::default().set_stored().set_indexing_options(
            TextFieldIndexing::default().set_tokenizer(TOKENIZER).set_index_option(IndexRecordOption::WithFreqs),
        )
    };
    let f = Fields {
        source: b.add_text_field("source", STRING | STORED),
        file: b.add_text_field("file", STRING | STORED),
        session: b.add_text_field("session", STRING | STORED),
        project: b.add_text_field("project", STRING | STORED),
        title: b.add_text_field("title", words()),
        text: b.add_text_field("text", words()),
        time: b.add_i64_field("time", NumericOptions::default().set_indexed().set_stored().set_fast()),
        role: b.add_text_field("role", STRING | STORED),
    };
    (b.build(), f)
}

fn index_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("dots").join("sessions")
}

fn open_index(dir: &Path) -> Res<(Index, Fields)> {
    fs::create_dir_all(dir)?;
    let (schema, f) = build_schema();
    let index = Index::open_or_create(MmapDirectory::open(dir)?, schema)?;
    index.tokenizers().register(
        TOKENIZER,
        TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(RemoveLongFilter::limit(40))
            .filter(LowerCaser)
            .filter(Stemmer::new(Language::English))
            .build(),
    );
    Ok((index, f))
}

/// Take the writer lock, retrying for up to `lock_wait()`. None means another run holds it.
fn lock_writer(index: &Index) -> Res<Option<IndexWriter>> {
    let deadline = Instant::now() + lock_wait();
    loop {
        match index.writer_with_num_threads::<TantivyDocument>(1, HEAP) {
            Ok(w) => return Ok(Some(w)),
            Err(TantivyError::LockFailure(LockError::LockBusy, _)) => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

type Stamps = BTreeMap<PathBuf, (u64, i64)>;

/// What the last run indexed: the (size, mtime) of every transcript and how many documents each gave.
#[derive(Default)]
struct State {
    claude: Stamps,
    docs: BTreeMap<String, u64>,
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

fn load_state(dir: &Path) -> State {
    let Ok(text) = fs::read_to_string(state_path(dir)) else { return State::default() };
    let Ok(v) = serde_json::from_str::<Json>(&text) else { return State::default() };
    if v.get("version").and_then(Json::as_u64) != Some(STATE_VERSION) {
        return State::default();
    }
    let mut s = State::default();
    for (k, e) in v.get("claude").and_then(Json::as_object).into_iter().flatten() {
        if let (Some(size), Some(mtime)) = (e.get(0).and_then(Json::as_u64), e.get(1).and_then(Json::as_i64)) {
            s.claude.insert(PathBuf::from(k), (size, mtime));
        }
    }
    for (k, n) in v.get("docs").and_then(Json::as_object).into_iter().flatten() {
        if let Some(n) = n.as_u64() {
            s.docs.insert(k.clone(), n);
        }
    }
    s
}

fn save_state(dir: &Path, s: &State) -> Res<()> {
    let claude: Map<String, Json> =
        s.claude.iter().map(|(p, (size, mtime))| (p.to_string_lossy().into_owned(), json!([size, mtime]))).collect();
    let docs: Map<String, Json> = s.docs.iter().map(|(p, n)| (p.clone(), json!(n))).collect();
    let body = json!({"version": STATE_VERSION, "claude": claude, "docs": docs});
    let tmp = dir.join("state.json.tmp");
    fs::write(&tmp, serde_json::to_vec(&body)?)?;
    fs::rename(&tmp, state_path(dir))?;
    Ok(())
}

fn to_tantivy(f: &Fields, d: &Doc) -> TantivyDocument {
    let mut t = TantivyDocument::default();
    t.add_text(f.source, d.source);
    t.add_text(f.file, &d.file);
    t.add_text(f.session, &d.session);
    t.add_text(f.project, &d.project);
    t.add_text(f.title, &d.title);
    t.add_text(f.text, &d.text);
    t.add_i64(f.time, d.time);
    t.add_text(f.role, &d.role);
    t
}

/// Read what changed since `state` into the writer and commit once. Returns the number of new
/// documents, or the reason the source was skipped. The state is saved after the commit.
fn update(home: &Path, dir: &Path, f: &Fields, writer: &mut IndexWriter, state: &mut State) -> Res<Result<u64, Skip>> {
    let mut added: BTreeMap<String, u64> = BTreeMap::new();
    let mut last = String::new();
    let mut failed: Option<TantivyError> = None;
    let read = read_claude(home, &state.claude, &mut |d: Doc| {
        if failed.is_some() {
            return;
        }
        if d.file != last {
            // The file is read again in full: drop what an earlier run indexed from it.
            writer.delete_term(Term::from_field_text(f.file, &d.file));
            last.clone_from(&d.file);
        }
        match added.get_mut(&d.file) {
            Some(n) => *n += 1,
            None => {
                added.insert(d.file.clone(), 1);
            }
        }
        if let Err(e) = writer.add_document(to_tantivy(f, &d)) {
            failed = Some(e);
        }
    });
    if let Some(e) = failed {
        return Err(e.into());
    }
    let fresh = match read {
        Ok(s) => s,
        Err(skip) => {
            let _ = writer.rollback();
            return Ok(Err(skip));
        }
    };

    let mut dirty = !added.is_empty();
    // Changed files that gave no document now, and files that are gone: nothing is added for
    // them, so the delete is safe.
    for (path, old) in &state.claude {
        let key = path.to_string_lossy().into_owned();
        let gone = !fresh.contains_key(path) && !path.exists();
        let changed = fresh.get(path).is_some_and(|n| n != old);
        if (gone || changed) && !added.contains_key(&key) {
            writer.delete_term(Term::from_field_text(f.file, &key));
            dirty = true;
        }
    }

    let mut new_docs = 0u64;
    let mut docs = BTreeMap::new();
    for path in fresh.keys() {
        let key = path.to_string_lossy().into_owned();
        let before = if state.claude.contains_key(path) { state.docs.get(&key).copied().unwrap_or(0) } else { 0 };
        let count = match added.get(&key) {
            Some(&n) => {
                new_docs += n.saturating_sub(before);
                n
            }
            None if state.claude.get(path) == fresh.get(path) => before,
            None => 0,
        };
        docs.insert(key, count);
    }
    if dirty {
        writer.commit()?;
    }
    let next = State { claude: fresh, docs };
    let changed = next.claude != state.claude || next.docs != state.docs;
    *state = next;
    if changed {
        save_state(dir, state)?;
    }
    Ok(Ok(new_docs))
}

/// Wait for running merges before the writer is dropped.
fn finish(writer: IndexWriter) {
    let _ = writer.wait_merging_threads();
}

/// Open the index, take the lock, optionally empty it, read new transcripts. None: lock busy.
fn refresh(home: &Path, dir: &Path, index: &Index, f: &Fields, rebuild: bool) -> Res<Option<Result<u64, Skip>>> {
    let Some(mut writer) = lock_writer(index)? else { return Ok(None) };
    let mut state = load_state(dir);
    let empty = index.reader()?.searcher().num_docs() == 0;
    if rebuild {
        writer.delete_all_documents()?;
        state = State::default();
    } else if empty {
        state = State::default(); // the index lost its documents: read everything again
    }
    let out = update(home, dir, f, &mut writer, &mut state)?;
    finish(writer);
    Ok(Some(out))
}

fn one_line(s: &str, n: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(n).collect()
}

/// Run `job`; when it fails, the index is a cache we can build again: wipe it, say so, retry once.
fn guarded<T>(dir: &Path, mut job: impl FnMut() -> Res<T>) -> Res<T> {
    match job() {
        Ok(v) => Ok(v),
        Err(e) => {
            println!("• sessions: index rebuilt: {}", one_line(&e.to_string(), 100));
            let _ = fs::remove_dir_all(dir);
            job()
        }
    }
}

fn print_skip(skip: &Skip) {
    println!("• sessions: claude skipped: {skip}");
}

fn run_index(home: &Path, rebuild: bool) -> Res<()> {
    let dir = index_dir();
    let out = guarded(&dir, || {
        let (index, f) = open_index(&dir)?;
        refresh(home, &dir, &index, &f, rebuild)
    })?;
    match out {
        None => println!("• sessions: index busy, skipped"),
        Some(Err(skip)) => print_skip(&skip),
        Some(Ok(n)) => println!("• sessions: {n} new"),
    }
    Ok(())
}

struct FindArgs {
    query: String,
    project: Option<String>,
    since: Option<i64>,
    limit: usize,
}

struct Hit {
    time: i64,
    project: String,
    session: String,
    title: String,
    snippet: String,
}

fn date_of(epoch: i64) -> String {
    let (y, m, d) = civil_from_days(epoch.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` to the epoch second of that day's start, UTC.
fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400)
}

/// Words only: query syntax in a typed question (`:`, `-`, quotes, parentheses) is not wanted.
fn plain_words(q: &str) -> String {
    // Lower case: the tokenizer lowers anyway, and upper-case NOT, AND, OR are query syntax.
    q.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn str_of(d: &TantivyDocument, f: Field) -> String {
    d.get_first(f).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn search(index: &Index, f: &Fields, a: &FindArgs) -> Res<Vec<Hit>> {
    let searcher = index.reader()?.searcher();
    let words = plain_words(&a.query);
    if words.is_empty() || searcher.num_docs() == 0 {
        return Ok(Vec::new());
    }
    let mut parser = QueryParser::for_index(index, vec![f.text, f.title]);
    let mut filters: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    filters.push((
        Occur::Must,
        Box::new(TermQuery::new(Term::from_field_text(f.source, "claude"), IndexRecordOption::Basic)),
    ));
    if let Some(p) = &a.project {
        filters.push((
            Occur::Must,
            Box::new(TermQuery::new(Term::from_field_text(f.project, p), IndexRecordOption::Basic)),
        ));
    }
    if let Some(t) = a.since {
        filters.push((
            Occur::Must,
            Box::new(RangeQuery::new(Bound::Included(Term::from_field_i64(f.time, t)), Bound::Unbounded)),
        ));
    }
    // Several documents of one session match; take enough to fill `limit` sessions.
    let take = (a.limit * 8).clamp(40, 400);
    let mut hits = Vec::new();
    for all_words in [true, false] {
        if all_words {
            parser.set_conjunction_by_default();
        } else {
            parser = QueryParser::for_index(index, vec![f.text, f.title]);
        }
        let text_q = parser.parse_query(&words)?;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = filters.iter().map(|(o, q)| (*o, q.box_clone())).collect();
        clauses.push((Occur::Must, text_q));
        let query = BooleanQuery::new(clauses);
        let top = searcher.search(&query, &TopDocs::with_limit(take).order_by_score())?;
        if top.is_empty() {
            continue;
        }
        let mut snips = SnippetGenerator::create(&searcher, &query, f.text)?;
        snips.set_max_num_chars(SNIPPET_CHARS);
        let mut seen: HashSet<String> = HashSet::new();
        for (_, addr) in top {
            let d: TantivyDocument = searcher.doc(addr)?;
            let session = str_of(&d, f.session);
            if !seen.insert(session.clone()) {
                continue;
            }
            let mut snippet = snips.snippet_from_doc(&d).fragment().to_string();
            if snippet.trim().is_empty() {
                snippet = str_of(&d, f.text);
            }
            let snippet = one_line(&redact(&snippet), SNIPPET_CHARS);
            hits.push(Hit {
                time: d.get_first(f.time).and_then(|v| v.as_i64()).unwrap_or(0),
                project: str_of(&d, f.project),
                session,
                title: one_line(&str_of(&d, f.title), 120),
                snippet,
            });
            if hits.len() >= a.limit {
                break;
            }
        }
        break;
    }
    Ok(hits)
}

fn run_find(home: &Path, a: &FindArgs) -> Res<()> {
    let dir = index_dir();
    // Only opening and refreshing may wipe the index; a failed search never does.
    let (index, f, busy, skip) = guarded(&dir, || {
        let (index, f) = open_index(&dir)?;
        let out = refresh(home, &dir, &index, &f, false)?;
        let (busy, skip) = match out {
            None => (true, None),
            Some(Err(skip)) => (false, Some(skip)),
            Some(Ok(n)) => {
                if n > 0 {
                    eprintln!("• sessions: indexed {n} new");
                }
                (false, None)
            }
        };
        Ok((index, f, busy, skip))
    })?;
    let hits = search(&index, &f, a)?;
    if busy {
        println!("• sessions: index busy, searched the last index");
    }
    if let Some(s) = skip {
        print_skip(&s);
    }
    if hits.is_empty() {
        println!("• sessions: no hits");
    }
    for h in &hits {
        let title = if h.title.is_empty() { "(no title)" } else { &h.title };
        println!("{}  claude  {}  {}  {}", date_of(h.time), h.project, h.session, title);
        println!("  {}", h.snippet);
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(home) = dirs::home_dir() else {
        println!("• sessions: no home folder found");
        return ExitCode::from(1);
    };
    let result = match cli.cmd {
        Cmd::Index { rebuild } => run_index(&home, rebuild),
        Cmd::Find { query, project, since, source, limit } => {
            if let Some(s) = source.as_deref().filter(|s| *s != "claude") {
                println!("• sessions: {s} skipped: not supported yet");
                return ExitCode::SUCCESS;
            }
            let since = match since.as_deref().map(parse_day) {
                Some(None) => {
                    eprintln!("sessions: --since wants a date like 2026-10-20");
                    return ExitCode::from(2);
                }
                Some(Some(t)) => Some(t),
                None => None,
            };
            let wanted = limit.unwrap_or(DEFAULT_LIMIT).max(1);
            if wanted > MAX_LIMIT {
                println!("• sessions: --limit cut to {MAX_LIMIT}");
            }
            run_find(&home, &FindArgs { query: query.join(" "), project, since, limit: wanted.min(MAX_LIMIT) })
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            println!("• sessions: failed: {}", one_line(&e.to_string(), 150));
            ExitCode::from(1)
        }
    }
}
