//! Tests for the `port` bin: a base port in a main checkout, a stable free port in a worktree.
//! Every case builds a fake repo layout in a temp folder: `<root>/<repo>/.git/` is a main checkout,
//! a `.git` file with a `gitdir:` line is a worktree.

#![allow(clippy::unwrap_used, reason = "a test fails by panicking")]

mod common;

use common::{run_bin, temp_home};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const PORT: &str = env!("CARGO_BIN_EXE_port");

/// A main checkout `<root>/<name>` with a `.git` folder.
fn main_checkout(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    fs::create_dir_all(repo.join(".git")).unwrap();
    repo
}

/// A worktree `<repo>/.claude/worktrees/<name>`: a `.git` file that points into the main `.git`.
fn worktree(repo: &Path, name: &str) -> PathBuf {
    let wt = repo.join(".claude").join("worktrees").join(name);
    fs::create_dir_all(&wt).unwrap();
    let admin = repo.join(".git").join("worktrees").join(name);
    fs::create_dir_all(&admin).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    wt
}

fn launch(repo: &Path, ports: &[u16]) {
    let entries: Vec<String> = ports
        .iter()
        .enumerate()
        .map(|(i, p)| {
            format!("{{\"name\": \"s{i}\", \"runtimeExecutable\": \"x\", \"port\": {p}}}")
        })
        .collect();
    let dir = repo.join(".claude");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("launch.json"),
        format!(
            "{{\"version\": \"0.0.1\", \"configurations\": [{}]}}",
            entries.join(",")
        ),
    )
    .unwrap();
}

fn port_in(cwd: &Path, name: &str, base: &str, extra: &[&str]) -> (i32, String) {
    let mut args = vec![name, base];
    args.extend_from_slice(extra);
    let (code, out, _) = run_bin(PORT, &args, "", &[], Some(cwd));
    (code, out.trim().to_string())
}

fn num(s: &str) -> u16 {
    s.parse().unwrap_or_else(|_| panic!("not a port: {s:?}"))
}

#[test]
fn main_checkout_prints_the_base_port() {
    let root = temp_home("port-main");
    let repo = main_checkout(&root, "app");
    assert_eq!(port_in(&repo, "web", "5173", &[]), (0, "5173".into()));
    let sub = repo.join("packages").join("web");
    fs::create_dir_all(&sub).unwrap();
    assert_eq!(port_in(&sub, "web", "5173", &[]), (0, "5173".into()));
}

#[test]
fn outside_a_repo_prints_the_base_port() {
    let root = temp_home("port-none");
    assert_eq!(port_in(&root, "web", "5173", &[]), (0, "5173".into()));
}

#[test]
fn worktree_gets_a_port_above_the_base_and_keeps_it() {
    let root = temp_home("port-wt");
    let repo = main_checkout(&root, "app");
    let wt = worktree(&repo, "feature-a");
    let (code, first) = port_in(&wt, "web", "5173", &[]);
    assert_eq!(code, 0);
    let p = num(&first);
    assert!((5174..=5272).contains(&p), "port {p} out of range");
    assert_eq!(port_in(&wt, "web", "5173", &[]).1, first);
    let sub = wt.join("src");
    fs::create_dir_all(&sub).unwrap();
    assert_eq!(
        port_in(&sub, "web", "5173", &[]).1,
        first,
        "same from a subfolder"
    );
}

#[test]
fn worktrees_and_server_names_get_different_ports() {
    let root = temp_home("port-many");
    let repo = main_checkout(&root, "app");
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..8 {
        let wt = worktree(&repo, &format!("wt-{i}"));
        seen.insert(port_in(&wt, "web", "5173", &["--no-probe"]).1);
    }
    assert!(
        seen.len() >= 5,
        "8 worktrees gave only {} ports: {seen:?}",
        seen.len()
    );
    let wt = worktree(&repo, "one");
    let a = port_in(&wt, "web", "5173", &["--no-probe"]).1;
    let b = port_in(&wt, "api", "5173", &["--no-probe"]).1;
    let c = port_in(&wt, "docs", "5173", &["--no-probe"]).1;
    assert!(a != b || b != c, "three names, one port: {a}");
}

#[test]
fn worktree_skips_ports_that_other_repos_claim() {
    let root = temp_home("port-claim");
    let repo = main_checkout(&root, "app");
    let wt = worktree(&repo, "feature-a");
    let first = num(&port_in(&wt, "web", "5173", &["--no-probe"]).1);
    // Another repo claims that port in its launch.json: the worktree moves on.
    let other = main_checkout(&root, "other");
    launch(&other, &[first]);
    let moved = num(&port_in(&wt, "web", "5173", &["--no-probe"]).1);
    assert_ne!(moved, first);
    assert!((5174..=5272).contains(&moved));
    // A launch.json at the folder that holds the repos counts too.
    launch(&root, &[moved]);
    let again = num(&port_in(&wt, "web", "5173", &["--no-probe"]).1);
    assert!(again != first && again != moved);
}

#[test]
fn worktree_skips_a_port_in_use() {
    let root = temp_home("port-use");
    let repo = main_checkout(&root, "app");
    let wt = worktree(&repo, "feature-a");
    let first = num(&port_in(&wt, "web", "5173", &["--no-probe"]).1);
    let Ok(_held) = TcpListener::bind(("127.0.0.1", first)) else {
        return; // the port is taken on this machine already: nothing to prove
    };
    let moved = num(&port_in(&wt, "web", "5173", &[]).1);
    assert_ne!(moved, first);
}

#[test]
fn worktree_skips_a_port_in_use_on_ipv6_loopback() {
    // Vite under modern Node binds `localhost`, which is ::1; 127.0.0.1 is then still free
    let root = temp_home("port-use-v6");
    let repo = main_checkout(&root, "app");
    let wt = worktree(&repo, "feature-b");
    let first = num(&port_in(&wt, "web", "5173", &["--no-probe"]).1);
    let Ok(_held) = TcpListener::bind(("::1", first)) else {
        return; // no IPv6 loopback here, or the port is taken already: nothing to prove
    };
    let moved = num(&port_in(&wt, "web", "5173", &[]).1);
    assert_ne!(moved, first);
}

#[test]
fn main_checkout_ignores_claims_and_use() {
    let root = temp_home("port-main-claim");
    let repo = main_checkout(&root, "app");
    let other = main_checkout(&root, "other");
    launch(&other, &[5173]);
    assert_eq!(port_in(&repo, "web", "5173", &[]), (0, "5173".into()));
}

#[test]
fn bad_arguments_exit_2() {
    let root = temp_home("port-bad");
    assert_eq!(run_bin(PORT, &[], "", &[], Some(&root)).0, 2);
    assert_eq!(run_bin(PORT, &["web"], "", &[], Some(&root)).0, 2);
    assert_eq!(
        run_bin(PORT, &["web", "notaport"], "", &[], Some(&root)).0,
        2
    );
    assert_eq!(run_bin(PORT, &["web", "0"], "", &[], Some(&root)).0, 2);
    assert_eq!(
        run_bin(PORT, &["web", "65535"], "", &[], Some(&root)).0,
        0,
        "main checkout needs no room"
    );
}

#[test]
fn a_base_with_no_room_above_it_fails_in_a_worktree() {
    let root = temp_home("port-top");
    let repo = main_checkout(&root, "app");
    let wt = worktree(&repo, "feature-a");
    let (code, _) = port_in(&wt, "web", "65535", &[]);
    assert_eq!(code, 1);
}
