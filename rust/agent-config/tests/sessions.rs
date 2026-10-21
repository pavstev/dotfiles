//! `sessions`: index and search Claude Code transcripts in a temp HOME.

mod common;

use common::{iso_utc, run_bin, temp_home};
use regex::Regex;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tantivy::{Index, TantivyDocument};

const BIN: &str = env!("CARGO_BIN_EXE_sessions");

const SID_A: &str = "aaaaaaaa-1111-2222-3333-444444444444";
const SID_B: &str = "bbbbbbbb-1111-2222-3333-444444444444";
const SID_C: &str = "cccccccc-1111-2222-3333-444444444444";

/// 2026-10-20T10:00:00Z and 2026-09-01T10:00:00Z.
const T_OCT: i64 = 1_792_490_400;
const T_SEP: i64 = 1_788_256_800;

fn sessions(home: &Path, args: &[&str]) -> (i32, String, String) {
    run_bin(BIN, args, "", &[("HOME", home.to_str().unwrap())], None)
}

/// Where `dirs::cache_dir()` points for a child run with this HOME (see `common::run_bin`).
fn index_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Caches").join("dots").join("sessions")
    } else if cfg!(windows) {
        dirs::cache_dir().expect("cache dir").join("dots").join("sessions")
    } else {
        home.join(".cache").join("dots").join("sessions")
    }
}

fn transcript(home: &Path, project_dir: &str, sid: &str) -> PathBuf {
    home.join(".claude").join("projects").join(project_dir).join(format!("{sid}.jsonl"))
}

fn user_line(t: i64, text: &str) -> String {
    json!({"type":"user","timestamp":iso_utc(t),"message":{"role":"user","content":text}}).to_string()
}

fn assistant_line(t: i64, text: &str) -> String {
    json!({"type":"assistant","timestamp":iso_utc(t),"message":{"id":"m","content":[{"type":"text","text":text}]}})
        .to_string()
}

fn write_lines(path: &Path, lines: &[String]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut s = lines.join("\n");
    s.push('\n');
    fs::write(path, s).unwrap();
}

fn append(path: &Path, text: &str) {
    let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn title_record(home: &Path, sid: &str, title: &str) {
    let p = home.join("Library/Application Support/Claude/claude-code-sessions/a/b").join(format!("local_{sid}.json"));
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, json!({"cliSessionId": sid, "title": title, "isArchived": false}).to_string()).unwrap();
}

/// Session A (demo, October, titled), session B (other, September), both with "common".
fn fixtures(home: &Path) {
    let long = format!("{} flaky vitest {}", "padding words here ".repeat(30), "more padding text ".repeat(30));
    write_lines(
        &transcript(home, "-work-demo", SID_A),
        &[user_line(T_OCT, "why is the flaky vitest suite failing, common problem"), assistant_line(T_OCT + 5, &long)],
    );
    title_record(home, SID_A, "Fix flaky vitest");
    write_lines(
        &transcript(home, "-work-other", SID_B),
        &[user_line(T_SEP, "deploy the worker to cloudflare, common problem")],
    );
}

fn hit_re() -> Regex {
    Regex::new(r"(?m)^\d{4}-\d{2}-\d{2}  claude  \S+  [0-9a-f-]{36}  .+$").unwrap()
}

fn hit_count(out: &str) -> usize {
    hit_re().find_iter(out).count()
}

fn doc_count(home: &Path) -> u64 {
    let index = Index::open_in_dir(index_dir(home)).expect("open index");
    index.reader().unwrap().searcher().num_docs()
}

#[test]
fn find_returns_hit_block() {
    let home = temp_home("sessions-hit");
    fixtures(&home);
    let (code, out, err) = sessions(&home, &["find", "flaky vitest"]);
    assert_eq!(code, 0, "{out}\n{err}");
    let lines: Vec<&str> = out.lines().collect();
    let i = lines.iter().position(|l| hit_re().is_match(l)).unwrap_or_else(|| panic!("no hit line in:\n{out}"));
    assert!(lines[i].contains("  demo  "), "{}", lines[i]);
    assert!(lines[i].contains(SID_A), "{}", lines[i]);
    assert!(lines[i].ends_with("Fix flaky vitest"), "{}", lines[i]);
    assert!(lines[i].starts_with("2026-10-20"), "{}", lines[i]);
    let snippet = lines.get(i + 1).expect("snippet line").trim();
    assert!(!snippet.is_empty());
    assert!(snippet.chars().count() <= 200, "{} chars", snippet.chars().count());
    assert!(snippet.to_lowercase().contains("vitest"), "{snippet}");
}

#[test]
fn incremental_index() {
    let home = temp_home("sessions-incr");
    fixtures(&home);
    let (code, out, err) = sessions(&home, &["index"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "• sessions: 3 new");

    let (code, out, _) = sessions(&home, &["index"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "• sessions: 0 new");

    append(&transcript(&home, "-work-other", SID_B), &(user_line(T_SEP + 60, "one more line") + "\n"));
    let (_, out, _) = sessions(&home, &["index"]);
    assert_eq!(out.trim(), "• sessions: 1 new");
    assert_eq!(doc_count(&home), 4, "a re-read file must not leave duplicates");

    let (_, out, _) = sessions(&home, &["index"]);
    assert_eq!(out.trim(), "• sessions: 0 new");

    // --rebuild starts over
    let (code, out, err) = sessions(&home, &["index", "--rebuild"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "• sessions: 4 new");
    assert_eq!(doc_count(&home), 4);
}

#[test]
fn half_line_indexed_next_run() {
    let home = temp_home("sessions-half");
    let path = transcript(&home, "-work-demo", SID_A);
    write_lines(&path, &[user_line(T_OCT, "first message about parsers")]);
    let full = user_line(T_OCT + 30, "second message about zebras");
    let (head, tail) = full.split_at(40);
    append(&path, head);

    let (code, out, err) = sessions(&home, &["index"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "• sessions: 1 new");
    let (_, out, _) = sessions(&home, &["find", "zebras"]);
    assert_eq!(hit_count(&out), 0, "{out}");

    append(&path, &format!("{tail}\n"));
    let (_, out, _) = sessions(&home, &["index"]);
    assert_eq!(out.trim(), "• sessions: 1 new");
    let (_, out, _) = sessions(&home, &["find", "zebras"]);
    assert_eq!(hit_count(&out), 1, "{out}");
}

#[test]
fn lock_wait_then_stale() {
    let home = temp_home("sessions-lock");
    fixtures(&home);
    let (code, _, err) = sessions(&home, &["index"]);
    assert_eq!(code, 0, "{err}");

    let index = Index::open_in_dir(index_dir(&home)).expect("open index");
    let _writer = index.writer::<TantivyDocument>(50_000_000).expect("hold the writer lock");

    let start = Instant::now();
    let env = [("HOME", home.to_str().unwrap()), ("SESSIONS_LOCK_WAIT_MS", "300")];
    let (code, out, err) = run_bin(BIN, &["find", "flaky vitest"], "", &env, None);
    let took = start.elapsed();
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(took >= Duration::from_millis(300), "did not wait for the lock: {took:?}");
    assert!(took < Duration::from_secs(5), "took {took:?}");
    assert!(out.contains("• sessions: index busy, searched the last index"), "{out}");
    assert_eq!(hit_count(&out), 1, "{out}");
}

#[test]
fn filters() {
    let home = temp_home("sessions-filters");
    fixtures(&home);
    write_lines(&transcript(&home, "-work-demo", SID_C), &[user_line(T_OCT + 100, "a third common thing")]);

    let (_, out, _) = sessions(&home, &["find", "common"]);
    assert_eq!(hit_count(&out), 3, "{out}");

    let (_, out, _) = sessions(&home, &["find", "common", "--limit", "2"]);
    assert_eq!(hit_count(&out), 2, "{out}");

    let (_, out, _) = sessions(&home, &["find", "common", "--project", "other"]);
    assert_eq!(hit_count(&out), 1, "{out}");
    assert!(out.contains(SID_B), "{out}");

    let (_, out, _) = sessions(&home, &["find", "common", "--since", "2026-10-01"]);
    assert_eq!(hit_count(&out), 2, "{out}");
    assert!(!out.contains(SID_B), "{out}");

    let (_, out, _) = sessions(&home, &["find", "common", "--source", "claude"]);
    assert_eq!(hit_count(&out), 3, "{out}");

    let (code, out, _) = sessions(&home, &["find", "common", "--since", "2026-13-45"]);
    assert_ne!(code, 0, "{out}");
}

#[test]
fn no_secret_in_index() {
    let home = temp_home("sessions-secret");
    let body = "a1B2c3D4e5".repeat(4);
    let key = ["sk", "-", &body].concat();
    write_lines(
        &transcript(&home, "-work-demo", SID_A),
        &[
            user_line(T_OCT, &format!("use {key} for the upload call")),
            assistant_line(T_OCT + 1, &["export API", "_TOKEN", "=", &"q9X8w7E6r5".repeat(3)].concat()),
        ],
    );
    let (code, _, err) = sessions(&home, &["index"]);
    assert_eq!(code, 0, "{err}");

    let mut files = 0;
    for e in walkdir::WalkDir::new(index_dir(&home)).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let bytes = fs::read(e.path()).unwrap();
        let hay = String::from_utf8_lossy(&bytes).to_lowercase();
        files += 1;
        assert!(!hay.contains(&body.to_lowercase()), "key body in {}", e.path().display());
        assert!(!hay.contains(&"q9x8w7e6r5".to_string()), "token value in {}", e.path().display());
    }
    assert!(files > 0);

    let (_, out, _) = sessions(&home, &["find", &body.to_lowercase()]);
    assert_eq!(hit_count(&out), 0, "{out}");
    let (_, out, _) = sessions(&home, &["find", "upload call"]);
    assert_eq!(hit_count(&out), 1, "{out}");
    assert!(!out.contains(&key), "{out}");
}

#[test]
fn corrupt_index_rebuilt() {
    let home = temp_home("sessions-corrupt");
    fixtures(&home);
    let (code, _, err) = sessions(&home, &["index"]);
    assert_eq!(code, 0, "{err}");
    fs::write(index_dir(&home).join("meta.json"), "{ this is not json").unwrap();

    let (code, out, err) = sessions(&home, &["find", "flaky vitest"]);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.lines().any(|l| l.starts_with("• sessions: ") && l.contains("rebuil")), "{out}");
    assert_eq!(hit_count(&out), 1, "{out}");
    assert_eq!(doc_count(&home), 3);
}

#[test]
fn missing_claude_dir_is_one_line() {
    let home = temp_home("sessions-missing");
    let (code, out, _) = sessions(&home, &["index"]);
    assert_eq!(code, 0);
    assert!(out.contains("• sessions: claude skipped: not found"), "{out}");
}

/// Peak resident memory of `sessions index`, in bytes. None when the platform cannot say.
fn peak_rss(home: &Path) -> Option<u64> {
    let time = Path::new("/usr/bin/time");
    if cfg!(windows) || !time.exists() {
        return None;
    }
    let flag = if cfg!(target_os = "macos") { "-l" } else { "-v" };
    let env = [("HOME", home.to_str().unwrap())];
    let (code, _, err) = run_bin("/usr/bin/time", &[flag, BIN, "index"], "", &env, None);
    assert_eq!(code, 0, "{err}");
    for l in err.lines() {
        let l = l.trim();
        if let Some(n) = l.strip_suffix("maximum resident set size") {
            return n.trim().parse().ok(); // macOS: bytes
        }
        if let Some(n) = l.strip_prefix("Maximum resident set size (kbytes):") {
            return n.trim().parse::<u64>().ok().map(|k| k * 1024);
        }
    }
    panic!("no resident size in the time output:\n{err}");
}

/// Write about `megabytes` of transcript lines made of pseudo-random words.
fn generate_big(path: &Path, megabytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut w = std::io::BufWriter::new(fs::File::create(path).unwrap());
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut written = 0usize;
    let mut n = 0i64;
    while written < megabytes * 1024 * 1024 {
        let mut text = String::new();
        while text.len() < 900 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            text.push_str(&format!("w{:x} ", x % 60_000));
        }
        let line = if n % 2 == 0 { user_line(T_OCT + n, &text) } else { assistant_line(T_OCT + n, &text) };
        w.write_all(line.as_bytes()).unwrap();
        w.write_all(b"\n").unwrap();
        written += line.len() + 1;
        n += 1;
    }
    w.flush().unwrap();
}

#[test]
fn big_transcript_streams() {
    let home = temp_home("sessions-big");
    let file = transcript(&home, "-work-big", SID_A);
    let t0 = Instant::now();
    generate_big(&file, 32);
    eprintln!("generated {} bytes in {:?}", fs::metadata(&file).unwrap().len(), t0.elapsed());

    let t1 = Instant::now();
    match peak_rss(&home) {
        Some(bytes) => {
            eprintln!("indexed in {:?}, peak RSS {} MB", t1.elapsed(), bytes / 1_000_000);
            assert!(bytes < 140_000_000, "peak RSS {bytes} bytes");
        }
        None => {
            eprintln!("note: no peak RSS on this platform, memory check skipped");
            let (code, _, err) = sessions(&home, &["index"]);
            assert_eq!(code, 0, "{err}");
        }
    }
    let (_, out, _) = sessions(&home, &["index"]);
    assert_eq!(out.trim(), "• sessions: 0 new");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn query_syntax_words_never_wipe_the_index() {
    let home = temp_home("sessions-syntax");
    fixtures(&home);
    let (code, _, err) = sessions(&home, &["find", "flaky vitest"]);
    assert_eq!(code, 0, "{err}");
    let docs = doc_count(&home);
    assert!(docs > 0);
    for q in ["NOT", "AND", "OR", "vitest AND", "IN", "NOT OR AND"] {
        let (code, out, err) = sessions(&home, &["find", q]);
        assert_eq!(code, 0, "{q}: {out}\n{err}");
        assert!(!out.contains("index rebuilt"), "{q}: {out}");
        assert_eq!(doc_count(&home), docs, "{q} changed the index");
    }
}
