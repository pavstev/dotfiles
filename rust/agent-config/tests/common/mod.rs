//! Helpers shared by the integration tests: run a binary hermetically and make temp homes.
//!
//! Every run gets its own `HOME` and `XDG_CACHE_HOME` under the temp dir (a fresh `temp_home` when
//! the caller passes no `HOME`), `NO_COLOR=1`, and none of the `DOTFILES_*` variables that change
//! a run unless the caller sets them. A test never reads the real home of the machine it runs on.
#![allow(dead_code, reason = "each test file uses a part of this")]

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A new, empty directory for one test: `<tmp>/agent-config-tests/<tag>-<pid>-<n>`. Parallel test
/// threads never share one. The caller removes it (or leaves it to the OS temp cleaner).
pub(crate) fn temp_home(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir()
        .join("agent-config-tests")
        .join(format!("{}-{}-{}", tag, std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp home");
    dir
}

/// Write `text` to `p`, creating the parent folders.
pub(crate) fn wr(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Write a JSON value to `p`.
pub(crate) fn wj(p: &Path, v: &Value) {
    wr(p, &v.to_string());
}

pub(crate) fn rd(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

pub(crate) fn rj(p: &Path) -> Value {
    serde_json::from_str(&rd(p)).unwrap()
}

#[cfg(unix)]
pub(crate) fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

/// A symlink `at` pointing to `target`, parents created.
#[cfg(unix)]
pub(crate) fn link(target: &Path, at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, at).unwrap();
}

/// Run git in `dir` with a fixed identity, no signing and none of the `GIT_*` location variables;
/// `env` adds variables on top. Panics when git fails; returns the trimmed stdout.
pub(crate) fn git_env(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "user.name=Tester",
        "-c",
        "user.email=tester@example.com",
        "-c",
        "commit.gpgsign=false",
    ])
    .args(args)
    .current_dir(dir)
    .env_remove("GIT_DIR")
    .env_remove("GIT_WORK_TREE")
    .env_remove("GIT_INDEX_FILE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    git_env(dir, &[], args)
}

const CLEARED: [&str; 4] = [
    "DOTFILES_REPO",
    "DOTFILES_PROFILE",
    "DOTFILES_VERBOSE",
    "DOTFILES_WARN_FILE",
];

/// Run `bin` (a path such as `env!("CARGO_BIN_EXE_dots")`) with `args`, feed `stdin`, return
/// `(exit code, stdout, stderr)`. `env` entries win over the defaults above; `cwd` is optional.
pub(crate) fn run_bin(
    bin: &str,
    args: &[&str],
    stdin: &str,
    env: &[(&str, &str)],
    cwd: Option<&Path>,
) -> (i32, String, String) {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for k in CLEARED {
        cmd.env_remove(k);
    }
    let given = |k: &str| env.iter().any(|(n, _)| *n == k);
    let own_home = if given("HOME") {
        None
    } else {
        Some(temp_home("run"))
    };
    if let Some(h) = &own_home {
        cmd.env("HOME", h);
    }
    if !given("XDG_CACHE_HOME") {
        let base = match (&own_home, env.iter().find(|(n, _)| *n == "HOME")) {
            (Some(h), _) => h.clone(),
            (None, Some((_, h))) => PathBuf::from(h),
            (None, None) => unreachable!(),
        };
        cmd.env("XDG_CACHE_HOME", base.join(".cache"));
    }
    cmd.env("NO_COLOR", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("start {bin}: {e}"));
    {
        let mut si = child.stdin.take().expect("stdin");
        let _ = si.write_all(stdin.as_bytes()); // the child may exit without reading
    }
    let out = child.wait_with_output().expect("wait");
    if let Some(h) = own_home {
        let _ = std::fs::remove_dir_all(h);
    }
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}
