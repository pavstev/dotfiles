//! `dots update`: the check and update phases, run against fake package managers. A temp HOME, a
//! fake bin dir and `DOTS_UPDATE_PATH` hold it all: no real manager can run, and nothing reads the
//! real home of the machine.
#![cfg(target_os = "macos")]
mod common;

use common::{run_bin, temp_home};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DOTS: &str = env!("CARGO_BIN_EXE_dots");

/// A fake manager: a `sh` script that appends its name and arguments to the calls file, then runs `body`.
fn fake(bin: &Path, name: &str, body: &str) {
    let p = bin.join(name);
    fs::write(&p, format!("#!/bin/sh\necho \"{name} $*\" >> \"$DOTS_TEST_CALLS\"\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Setup {
    home: PathBuf,
    bin: PathBuf,
    calls: PathBuf,
}

/// Fakes for brew (2 outdated), mise (up to date), rustup (1 outdated), npm and pnpm (outdated) and
/// softwareupdate (1 update). The rest are not installed. brew, rustup and npm are up to date once
/// their update ran; pnpm's item stays (a pin holds it back).
fn setup(tag: &str) -> Setup {
    let tmp = temp_home(tag);
    let (home, bin) = (tmp.join("home"), tmp.join("bin"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fake(
        &bin,
        "brew",
        r#"case "$1" in
  outdated) if /usr/bin/grep -q "^brew upgrade" "$DOTS_TEST_CALLS"; then echo '{"formulae":[],"casks":[]}'; exit 0; fi
            echo '{"formulae":[{"name":"git"},{"name":"node@20","pinned":true},{"name":"wget","pinned":false}],"casks":[]}' ;;
  upgrade) echo "==> Upgrading 2 outdated packages" ;;
esac"#,
    );
    fake(
        &bin,
        "mise",
        r#"pwd > "$DOTS_TEST_CALLS.pwd"
case "$1" in
  outdated) echo '{}' ;;
esac"#,
    );
    fake(
        &bin,
        "rustup",
        r#"case "$1" in
  check) if /usr/bin/grep -q "^rustup update" "$DOTS_TEST_CALLS"; then echo "rustup - up to date : 1.29.2"; exit 0; fi
         echo "stable-aarch64-apple-darwin - update available: 1.99.0 (b940084d7 2026-09-28) -> 1.99.1 (58cf8f9c3 2026-10-13)"
         echo "rustup - up to date : 1.29.2"
         exit 100 ;;
esac"#,
    );
    fake(
        &bin,
        "npm",
        r#"case "$1" in
  outdated) if [ -n "$HANG_NPM" ]; then exec /bin/sleep 30; fi
            if [ -n "$NPM_OFFLINE" ]; then
              echo '{"error":{"code":"ECONNREFUSED","summary":"request to https://registry.npmjs.org/npm failed","detail":""}}'
              exit 1
            fi
            if /usr/bin/grep -q "^npm update" "$DOTS_TEST_CALLS"; then echo '{}'; exit 0; fi
            echo '{"typescript":{"current":"5.0.0","wanted":"5.1.0","latest":"5.1.0"}}'
            exit 1 ;;
  update) if [ -n "$FAIL_NPM" ]; then
            i=1; while [ $i -le 12 ]; do echo "npm line $i"; i=$((i+1)); done
            echo "npm exploded" >&2
            exit 1
          fi ;;
esac"#,
    );
    fake(
        &bin,
        "pnpm",
        r#"case "$1" in
  outdated) echo '{"eslint":{"current":"9.0.0","latest":"9.1.0"}}'; exit 1 ;;
esac"#,
    );
    fake(
        &bin,
        "softwareupdate",
        r#"case "$1" in
  --list) echo "Finding available software" >&2
          echo "Software Update found the following new or updated software:"
          echo "* Label: macOS 27.2-26B123"
          echo "	Title: macOS 27.2, Version: 27.2, Size: 6067473KiB, Recommended: YES, Action: restart, " ;;
  *) echo "INSTALL ATTEMPT" >&2; exit 9 ;;
esac"#,
    );
    let calls = tmp.join("calls.txt");
    fs::write(&calls, "").unwrap();
    Setup { home, bin, calls }
}

/// Run `dots update <args>`. Returns the exit code and stdout plus stderr.
fn update(s: &Setup, args: &[&str], extra: &[(&str, &str)]) -> (i32, String) {
    update_in(s, args, extra, None)
}

/// Like `update`, started in the folder `cwd`.
fn update_in(s: &Setup, args: &[&str], extra: &[(&str, &str)], cwd: Option<&Path>) -> (i32, String) {
    let nozsh = s.home.join("no-oh-my-zsh");
    let mut env: Vec<(&str, &str)> = vec![
        ("HOME", s.home.to_str().unwrap()),
        ("DOTS_UPDATE_PATH", s.bin.to_str().unwrap()),
        ("DOTS_TEST_CALLS", s.calls.to_str().unwrap()),
        ("ZSH", nozsh.to_str().unwrap()),
    ];
    env.extend_from_slice(extra);
    let mut full = vec!["update"];
    full.extend_from_slice(args);
    let (code, out, err) = run_bin(DOTS, &full, "", &env, cwd);
    (code, format!("{out}{err}"))
}

fn calls(s: &Setup) -> Vec<String> {
    fs::read_to_string(&s.calls).unwrap().lines().map(str::to_string).collect()
}

fn logs(s: &Setup) -> Vec<PathBuf> {
    let dir = s.home.join("Library/Logs/dots");
    fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

const UPDATES: [&str; 5] =
    ["brew upgrade", "mise upgrade", "rustup update", "npm update -g", "pnpm update -g --latest"];

#[test]
fn check_lists_outdated_and_runs_no_update() {
    let s = setup("update-check");
    let (code, out) = update(&s, &["--check"], &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Homebrew: 2 to update (git, wget)"), "{out}");
    assert!(out.contains("Rust (rustup): 1 to update (stable-aarch64-apple-darwin)"), "{out}");
    assert!(out.contains("npm globals: 1 to update (typescript)"), "{out}");
    assert!(out.contains("pnpm globals: 1 to update (eslint)"), "{out}");
    assert!(out.contains("mise: up to date"), "{out}");
    assert!(out.contains("macOS: 1 update (install it in System Settings > General > Software Update)"), "{out}");
    assert!(out.contains("to update: Homebrew, Rust (rustup), npm globals, pnpm globals"), "{out}");
    assert!(!out.contains("cargo crates"), "missing managers stay silent: {out}");
    let c = calls(&s);
    assert!(c.contains(&"brew update --quiet".to_string()), "{c:?}");
    assert!(c.contains(&"brew outdated --json=v2".to_string()), "{c:?}");
    for u in UPDATES {
        assert!(!c.contains(&u.to_string()), "{u} ran in --check: {c:?}");
    }
    assert!(!c.iter().any(|l| l.contains("INSTALL")), "{c:?}");
}

#[test]
fn full_run_updates_only_outdated_managers() {
    let s = setup("update-full");
    let (code, out) = update(&s, &[], &[]);
    assert_eq!(code, 0, "{out}");
    let c = calls(&s);
    for u in ["brew upgrade", "rustup update", "npm update -g", "pnpm update -g --latest"] {
        assert!(c.contains(&u.to_string()), "{u} did not run: {c:?}");
    }
    assert!(!c.contains(&"mise upgrade".to_string()), "mise is up to date: {c:?}");
    assert!(!c.iter().any(|l| l.starts_with("mise self-update")), "{c:?}");
    // the order of the table
    let pos = |w: &str| c.iter().position(|l| l == w).unwrap();
    assert!(pos("brew upgrade") < pos("rustup update") && pos("rustup update") < pos("npm update -g"));
    assert!(out.contains("Homebrew: updated 2"), "{out}");
    assert!(out.contains("updated: Homebrew, Rust (rustup), npm globals\n"), "{out}");
    // pnpm exits 0, but the check after it still lists eslint
    assert!(out.contains("pnpm globals: updated 0 of 1"), "{out}");
    assert!(out.contains("pnpm globals: still outdated: eslint (a version pin may hold it back)"), "{out}");
    assert!(out.contains("still outdated: pnpm globals"), "{out}");
    assert!(out.contains("check only: macOS"), "{out}");
    assert!(out.contains("up to date: mise"), "{out}");
    let logs = logs(&s);
    assert_eq!(logs.len(), 1, "{logs:?}");
    let log = fs::read_to_string(&logs[0]).unwrap();
    assert!(log.contains("== brew: brew upgrade\n"), "{log}");
    assert!(log.contains("==> Upgrading 2 outdated packages"), "{log}");
    assert!(log.contains("== brew: brew outdated --json=v2\n"), "{log}");
    assert!(log.contains("== exit 0 in "), "{log}");
}

#[test]
fn a_failed_update_does_not_stop_the_next_one() {
    let s = setup("update-fail");
    let (code, out) = update(&s, &[], &[("FAIL_NPM", "1")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("npm globals failed (exit 1)"), "{out}");
    assert!(out.contains("npm exploded"), "{out}");
    assert!(out.contains("npm line 12"), "the last lines are shown: {out}");
    assert!(!out.contains("npm line 1\n"), "only the last 10 lines are shown: {out}");
    assert!(out.contains("Full log: "), "{out}");
    assert!(out.contains("failed: npm globals"), "{out}");
    assert!(calls(&s).contains(&"pnpm update -g --latest".to_string()), "the manager after the failure still runs");
    let log = fs::read_to_string(&logs(&s)[0]).unwrap();
    assert!(log.contains("== exit 1 in "), "{log}");
}

#[test]
fn a_hanging_check_times_out() {
    let s = setup("update-hang");
    let started = Instant::now();
    let (code, out) =
        update(&s, &["--check", "--only", "npm,brew"], &[("HANG_NPM", "1"), ("DOTS_UPDATE_TIMEOUT", "2")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("npm globals: check failed: timed out after 2s"), "{out}");
    assert!(out.contains("Homebrew: 2 to update"), "the other checks still report: {out}");
    assert!(started.elapsed().as_secs() < 15, "took {:?}", started.elapsed());
}

#[test]
fn only_and_skip_filter_and_unknown_ids_exit_2() {
    let s = setup("update-filter");
    let (code, out) = update(&s, &["--check", "--only", "brew,mise"], &[]);
    assert_eq!(code, 0, "{out}");
    let c = calls(&s);
    assert!(c.iter().all(|l| l.starts_with("brew ") || l.starts_with("mise ")), "{c:?}");
    assert!(!out.contains("npm globals"), "{out}");

    let s = setup("update-skip");
    let (code, out) = update(&s, &["--check", "--skip", "brew,mise,npm,pnpm,macos"], &[]);
    assert_eq!(code, 0, "{out}");
    let c = calls(&s);
    assert_eq!(c, ["rustup check"], "{c:?}");

    let s = setup("update-unknown");
    let (code, out) = update(&s, &["--only", "brew,nope"], &[]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("nope") && out.contains("brew, mise, rustup, cargo"), "valid ids are listed: {out}");
    assert!(calls(&s).is_empty(), "nothing runs on bad arguments");
}

#[test]
fn macos_is_reported_and_never_updated() {
    let s = setup("update-macos");
    let (code, out) = update(&s, &["--only", "macos"], &[]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(calls(&s), ["softwareupdate --list"]);
    assert!(out.contains("check only: macOS"), "{out}");
    assert!(!out.contains("INSTALL ATTEMPT"), "{out}");
}

#[test]
fn mas_is_reported_with_a_sudo_hint_and_never_upgraded() {
    let s = setup("update-mas");
    fake(
        &s.bin,
        "mas",
        r#"case "$1" in
  outdated) echo "497799835  Xcode (15.0)"; echo "1444383602  GoodNotes 5 (5.9.6)" ;;
  *) echo "mas needs root" >&2; exit 1 ;;
esac"#,
    );
    let (code, out) = update(&s, &["--only", "mas"], &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("App Store apps: 2 updates (run: sudo mas upgrade)"), "{out}");
    assert!(out.contains("check only: App Store apps"), "{out}");
    assert_eq!(calls(&s), ["mas outdated"]);
}

#[test]
fn offline_npm_is_a_failed_check_not_an_outdated_package() {
    let s = setup("update-npm-offline");
    let (code, out) = update(&s, &["--only", "npm"], &[("NPM_OFFLINE", "1")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("npm globals: check failed: request to https://registry.npmjs.org/npm failed"), "{out}");
    assert!(!out.contains("to update (error)"), "{out}");
    assert_eq!(calls(&s), ["npm outdated -g --json"], "no update ran");
}

#[test]
fn children_start_in_home_not_in_the_project() {
    let s = setup("update-cwd");
    let project = s.home.parent().unwrap().join("project");
    fs::create_dir_all(&project).unwrap();
    let (code, out) = update_in(&s, &["--check", "--only", "mise"], &[], Some(&project));
    assert_eq!(code, 0, "{out}");
    let pwd = fs::read_to_string(format!("{}.pwd", s.calls.display())).unwrap();
    assert_eq!(fs::canonicalize(pwd.trim()).unwrap(), fs::canonicalize(&s.home).unwrap());
}

#[test]
fn pinned_brew_formulae_do_not_count() {
    let s = setup("update-pinned");
    let (code, out) = update(&s, &["--check", "--only", "brew"], &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Homebrew: 2 to update (git, wget)"), "node@20 is pinned: {out}");
}

#[test]
fn oh_my_zsh_skips_a_repo_that_cannot_fetch_and_git_never_prompts() {
    let s = setup("update-omz");
    let omz = s.home.join("omz");
    for d in [".git", "custom/plugins/good/.git", "custom/plugins/bad/.git"] {
        fs::create_dir_all(omz.join(d)).unwrap();
    }
    fake(
        &s.bin,
        "git",
        r#"echo "env GTP=$GIT_TERMINAL_PROMPT SSH=$GIT_SSH_COMMAND" >> "$DOTS_TEST_CALLS.git"
case "$*" in
  *plugins/bad*fetch*) echo "fatal: repository not found" >&2; exit 128 ;;
  *rev-list*) echo 1 ;;
esac"#,
    );
    fake(&s.bin, "zsh", "exit 0");
    let o = omz.to_str().unwrap();
    let (code, out) = update(&s, &["--only", "omz"], &[("ZSH", o)]);
    assert_eq!(code, 0, "one dead remote is a warning, not a failure: {out}");
    assert!(out.contains("skipped: plugins/bad (fatal: repository not found"), "{out}");
    assert!(out.contains("Oh My Zsh: 2 to update (oh-my-zsh, plugins/good)"), "{out}");
    let c = calls(&s);
    assert!(c.contains(&format!("zsh -f {o}/tools/upgrade.sh -v silent")), "{c:?}");
    assert!(c.contains(&format!("git -C {o}/custom/plugins/good pull --ff-only --quiet")), "{c:?}");
    assert!(!c.iter().any(|l| l.contains("plugins/bad") && l.contains("pull")), "{c:?}");
    let git_env = fs::read_to_string(format!("{}.git", s.calls.display())).unwrap();
    assert!(git_env.lines().all(|l| l.contains("GTP=0") && l.contains("-o BatchMode=yes")), "{git_env}");
}

#[test]
fn gh_without_extensions_is_skipped_even_when_signed_out() {
    let s = setup("update-gh");
    fake(
        &s.bin,
        "gh",
        r#"echo "To get started with GitHub CLI, please run:  gh auth login" >&2
exit 4"#,
    );
    let data = s.home.join("gh-data");
    let d = data.to_str().unwrap();
    let (code, out) = update(&s, &["--check", "--only", "gh"], &[("GH_DATA_DIR", d)]);
    assert_eq!(code, 0, "no extensions, so nothing to check: {out}");
    assert!(!out.contains("check failed"), "{out}");
    assert!(!fs::read_to_string(&s.calls).unwrap_or_default().contains("gh "), "gh never ran");
    // With an extension, a signed-out gh is a real failure.
    fs::create_dir_all(data.join("extensions/gh-dash")).unwrap();
    let (code, out) = update(&s, &["--check", "--only", "gh"], &[("GH_DATA_DIR", d)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("gh extensions: check failed: To get started with GitHub CLI"), "{out}");
}
