//! The `leftovers` bin in temp git repos: each test runs the real binary with a temp HOME.
mod common;

use common::{run_bin, temp_home};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const LEFTOVERS: &str = env!("CARGO_BIN_EXE_leftovers");

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=Tester", "-c", "user.email=tester@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A temp home with a repo `proj` on branch main with one commit.
fn temp_repo(tag: &str) -> (PathBuf, PathBuf) {
    let home = temp_home(tag);
    let repo = home.join("proj");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    fs::write(repo.join("f.txt"), "x\n").unwrap();
    git(&repo, &["add", "f.txt"]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    (home, repo)
}

fn run(home: &Path, cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let env = [("HOME", s(home)), ("GIT_CEILING_DIRECTORIES", s(home))];
    run_bin(LEFTOVERS, args, "", &env, Some(cwd))
}

/// The id an `added <id> to <store>` line names.
fn added_id(out: &str) -> String {
    out.strip_prefix("added ").and_then(|r| r.split_whitespace().next()).unwrap_or_else(|| panic!("{out}")).to_string()
}

fn same_file(printed: &str, want: &Path) -> bool {
    let (a, b) = (Path::new(printed.trim()), want);
    a.parent().unwrap().canonicalize().unwrap() == b.parent().unwrap().canonicalize().unwrap()
        && a.file_name() == b.file_name()
}

fn store_of(repo: &Path) -> PathBuf {
    repo.join(".agents").join("leftovers.jsonl")
}

#[test]
fn add_list_done_drop_count_and_path() {
    let (home, repo) = temp_repo("lo-basic");
    let (code, out, err) = run(&home, &repo, &["list"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("no leftovers here ("), "{out}");

    let (code, out, err) = run(&home, &repo, &["add", "first thing", "--tag", "ui"]);
    assert_eq!(code, 0, "{err}");
    let one = added_id(&out);
    let (_, out, _) = run(&home, &repo, &["add", "- second thing"]);
    let two = added_id(&out);
    assert_eq!(one.len(), 8);

    let (_, out, _) = run(&home, &repo, &["list"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].starts_with(&format!("{one}  20")) && lines[0].ends_with("  first thing"), "{out}");
    assert!(lines[1].ends_with("  - second thing"), "{out}");
    assert_eq!(run(&home, &repo, &["count"]).1, "2\n");
    let (_, out, _) = run(&home, &repo, &["path"]);
    assert!(same_file(&out, &store_of(&repo)), "{out}");

    // done by a unique prefix, drop by the full id, both with a note
    let (code, out, _) = run(&home, &repo, &["done", &one[..5], "--note", "shipped"]);
    assert_eq!((code, out), (0, format!("done {one}\n")));
    let (code, out, _) = run(&home, &repo, &["drop", &two]);
    assert_eq!((code, out), (0, format!("dropped {two}\n")));
    assert_eq!(run(&home, &repo, &["count"]).1, "0\n");
    let (_, out, _) = run(&home, &repo, &["list"]);
    assert_eq!(out, "");
    let (_, out, _) = run(&home, &repo, &["list", "--all"]);
    assert!(out.contains("first thing  [done 20") && out.contains(": shipped]"), "{out}");
    assert!(out.contains("- second thing  [dropped 20"), "{out}");

    // a closed item cannot close again; an unknown id is exit 1
    let (code, _, err) = run(&home, &repo, &["done", &one]);
    assert_eq!(code, 1);
    assert!(err.contains("already done"), "{err}");
    let (code, _, err) = run(&home, &repo, &["done", "ffffffff"]);
    assert_eq!(code, 1);
    assert!(err.contains("no leftover with id ffffffff"), "{err}");
}

#[test]
fn list_json_is_an_array_of_items() {
    let (home, repo) = temp_repo("lo-json");
    let (_, out, _) = run(&home, &repo, &["add", "a thing", "--tag", "x", "--tag", "y"]);
    let id = added_id(&out);
    let (code, out, _) = run(&home, &repo, &["list", "--json"]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["id"], id.as_str());
    assert_eq!(v[0]["text"], "a thing");
    assert_eq!(v[0]["tags"], serde_json::json!(["x", "y"]));
    assert_eq!(v[0]["from"], "proj");
    assert_eq!(v[0]["status"], "open");
    // the file holds one event per line with the version first
    let text = fs::read_to_string(store_of(&repo)).unwrap();
    assert!(text.starts_with("{\"v\":1,\"op\":\"add\",\"id\":\""), "{text}");
    assert_eq!(text.lines().count(), 1);
}

#[test]
fn a_worktree_writes_to_the_main_checkout() {
    let (home, repo) = temp_repo("lo-worktree");
    let wt = home.join("wt-one");
    git(&repo, &["worktree", "add", "-q", s(&wt), "-b", "wt-one"]);
    let (code, out, err) = run(&home, &wt, &["add", "from the worktree"]);
    assert_eq!(code, 0, "{err}");
    assert!(same_file(out.lines().next().unwrap().split(" to ").nth(1).unwrap(), &store_of(&repo)), "{out}");
    assert!(store_of(&repo).is_file());
    assert!(!store_of(&wt).exists());
    // both places see it, and the item remembers the worktree name
    assert_eq!(run(&home, &repo, &["count"]).1, "1\n");
    assert_eq!(run(&home, &wt, &["count"]).1, "1\n");
    let (_, out, _) = run(&home, &repo, &["list", "--json"]);
    assert!(out.contains("\"from\":\"wt-one\""), "{out}");
    // --repo reaches it from anywhere
    let (_, out, _) = run(&home, &home, &["count", "--repo", s(&wt)]);
    assert_eq!(out, "1\n");
}

#[test]
fn the_gitignore_line_is_added_once() {
    let (home, repo) = temp_repo("lo-ignore");
    // an existing .gitignore without a final newline
    fs::write(repo.join(".gitignore"), "target").unwrap();
    let (_, out, _) = run(&home, &repo, &["add", "one"]);
    assert!(out.lines().nth(1).is_some_and(|l| l.starts_with("added .agents/ to ")), "{out}");
    assert_eq!(fs::read_to_string(repo.join(".gitignore")).unwrap(), "target\n.agents/\n");
    // the second add finds the store ignored and says nothing more
    let (_, out, _) = run(&home, &repo, &["add", "two"]);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert_eq!(fs::read_to_string(repo.join(".gitignore")).unwrap(), "target\n.agents/\n");
    // a repo with no .gitignore gets one
    let (home2, repo2) = temp_repo("lo-ignore2");
    run(&home2, &repo2, &["add", "one"]);
    assert_eq!(fs::read_to_string(repo2.join(".gitignore")).unwrap(), ".agents/\n");
}

#[test]
fn outside_git_is_exit_two() {
    let home = temp_home("lo-nogit");
    let plain = home.join("plain");
    fs::create_dir_all(&plain).unwrap();
    for args in [&["add", "x"][..], &["list"], &["count"], &["path"]] {
        let (code, out, err) = run(&home, &plain, args);
        assert_eq!(code, 2, "{args:?}: {out}{err}");
        assert!(err.contains("not inside a git repository"), "{err}");
    }
}

#[test]
fn mic_check_note_outside_git_is_exit_two() {
    let home = temp_home("lo-mic-nogit");
    let plain = home.join("plain");
    fs::create_dir_all(&plain).unwrap();
    let env = [("HOME", s(&home)), ("GIT_CEILING_DIRECTORIES", s(&home))];
    let (code, _, err) = run_bin(env!("CARGO_BIN_EXE_mic_check"), &["--note", "x"], "", &env, Some(&plain));
    assert_eq!(code, 2);
    assert!(err.contains("not inside a git repository"), "{err}");
}

#[test]
fn compact_keeps_open_and_recent_closed_items() {
    let (home, repo) = temp_repo("lo-compact");
    let (_, out, _) = run(&home, &repo, &["add", "stays open"]);
    let open = added_id(&out);
    let (_, out, _) = run(&home, &repo, &["add", "closed now"]);
    let recent = added_id(&out);
    run(&home, &repo, &["done", &recent]);
    // an old item, closed long ago, and a line that is not an event
    let mut text = fs::read_to_string(store_of(&repo)).unwrap();
    text.push_str("{\"v\":1,\"op\":\"add\",\"id\":\"0ld00001\",\"at\":\"2025-01-01T00:00:00Z\",\"text\":\"old\",\"tags\":[],\"from\":\"proj\"}\n");
    text.push_str("{\"v\":1,\"op\":\"done\",\"id\":\"0ld00001\",\"at\":\"2025-01-02T00:00:00Z\"}\nnot json\n");
    fs::write(store_of(&repo), text).unwrap();
    let (code, out, err) = run(&home, &repo, &["compact"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("kept 2, removed 1"), "{out}");
    let after = fs::read_to_string(store_of(&repo)).unwrap();
    assert_eq!(after.lines().count(), 3, "{after}"); // open add, recent add, recent done
    assert!(after.contains(&open) && after.contains(&recent) && !after.contains("0ld00001"), "{after}");
    assert_eq!(run(&home, &repo, &["count"]).1, "1\n");
    // no temp file is left behind
    let names: Vec<_> = fs::read_dir(repo.join(".agents")).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

const OLD: &str = "# Leftovers\n\nOptional fixes.\n\
- [x] 2026-10-07 · **proj** · closed already\n\
- [ ] 2026-10-08 · **proj** · first\n\
- [ ] 2026-10-09 · **wt-one** · from a worktree\n\
- [ ] 2026-10-10 · **ghost** · nobody lives here\n";

#[test]
fn import_splits_the_old_file_and_keeps_it_until_all_resolve() {
    let home = temp_home("lo-import");
    let repo = home.join("proj");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    fs::write(repo.join("f.txt"), "x\n").unwrap();
    git(&repo, &["add", "f.txt"]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(&repo, &["worktree", "add", "-q", s(&repo.join(".claude").join("worktrees").join("wt-one")), "-b", "wt-one"]);
    let other = home.join("other");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q"]);
    let old = home.join(".claude").join("leftovers.md");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, OLD).unwrap();

    // a dry run changes nothing
    let (code, out, err) = run(&home, &home, &["import", "--dry-run"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("would import 2 of 2"), "{out}");
    assert!(out.contains("unresolved: 2026-10-10 · **ghost** · nobody lives here"), "{out}");
    assert!(!store_of(&repo).exists() && old.is_file());

    // ghost has no project: the items that resolve go in, the file stays, exit 1
    let (code, out, err) = run(&home, &home, &["import"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("imported 2 of 2") && out.contains("kept "), "{out}");
    assert!(old.is_file());
    let (_, out, _) = run(&home, &repo, &["list", "--json"]);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!((v[0]["at"].as_str(), v[0]["from"].as_str()), (Some("2026-10-08T00:00:00Z"), Some("proj")));
    assert_eq!((v[1]["text"].as_str(), v[1]["from"].as_str()), (Some("from a worktree"), Some("wt-one")));
    assert!(fs::read_to_string(repo.join(".gitignore")).unwrap().contains(".agents/"));

    // with a map the last line resolves; the ones already in are not added twice; the file goes
    let map = format!("ghost={}", s(&other));
    let (code, out, err) = run(&home, &home, &["import", "--map", &map]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("imported 0 of 2") && out.contains("imported 1 of 1") && out.contains("removed "), "{out}");
    assert!(!old.exists());
    assert_eq!(run(&home, &repo, &["count"]).1, "2\n");
    assert_eq!(run(&home, &other, &["count"]).1, "1\n");
}

#[test]
fn whatsleft_scan_shows_the_open_leftovers_of_a_touched_repo() {
    let (home, repo) = temp_repo("lo-scan");
    let (_, out, _) = run(&home, &repo, &["add", "parked idea"]);
    let done = added_id(&out);
    run(&home, &repo, &["add", "also parked"]);
    run(&home, &repo, &["done", &done]);
    // a one-call transcript that ran in the repo
    let cwd = repo.canonicalize().unwrap().to_string_lossy().into_owned();
    let call = serde_json::json!({"type": "assistant", "timestamp": "2026-10-14T10:01:00+00:00", "cwd": cwd,
        "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"}}]}});
    let reply = serde_json::json!({"type": "user", "timestamp": "2026-10-14T10:02:00+00:00",
        "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": false}]}});
    let session = home.join("session.jsonl");
    fs::write(&session, format!("{call}\n{reply}\n")).unwrap();
    let env = [("HOME", s(&home)), ("GIT_CEILING_DIRECTORIES", s(&home))];
    let args = ["--session", s(&session), "--no-fetch", "--no-ci"];
    let (code, out, err) = run_bin(env!("CARGO_BIN_EXE_whatsleft_scan"), &args, "", &env, Some(&repo));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("== LEFTOVERS (") && out.contains("leftovers.jsonl) =="), "{out}");
    assert!(out.contains("LATER") && out.contains(" · **proj** · also parked"), "{out}");
    assert!(!out.contains("parked idea"), "{out}");
}

#[test]
fn import_of_a_missing_file_is_exit_one() {
    let home = temp_home("lo-import-none");
    let (code, _, err) = run(&home, &home, &["import"]);
    assert_eq!(code, 1);
    assert!(err.contains("leftovers.md"), "{err}");
}
