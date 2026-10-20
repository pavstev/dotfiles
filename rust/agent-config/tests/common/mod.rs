//! Helpers shared by the integration tests: run a binary hermetically and make temp homes.
//!
//! Every run gets its own `HOME` and `XDG_CACHE_HOME` under the temp dir (a fresh `temp_home` when
//! the caller passes no `HOME`), `NO_COLOR=1`, and none of the variables that change a hook's
//! behavior (`GUARD_OFF`, `GUARD_SHADOW`, `GUARD_PLATFORM`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CONFIG_DIR`, `TZ`)
//! unless the caller sets them. A test never reads the real home of the machine it runs on.
#![allow(dead_code)] // each test file uses a part of this

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A new, empty directory for one test: `<tmp>/agent-config-tests/<tag>-<pid>-<n>`. Parallel test
/// threads never share one. The caller removes it (or leaves it to the OS temp cleaner).
pub fn temp_home(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir()
        .join("agent-config-tests")
        .join(format!("{}-{}-{}", tag, std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp home");
    dir
}

/// `%Y-%m-%dT%H:%M:%S.000Z` of a Unix time (UTC), the stamp transcripts carry.
pub fn iso_utc(epoch: i64) -> String {
    let days = epoch.div_euclid(86400);
    let rem = epoch.rem_euclid(86400);
    let z = days + 719468; // days from 0000-03-01, then the civil-from-days algorithm
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

const CLEARED: [&str; 12] = [
    "GUARD_OFF", "GUARD_SHADOW", "GUARD_PLATFORM", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CONFIG_DIR", "TZ",
    "DOTFILES_REPO", "DOTFILES_PROFILE", "DOTFILES_MCP", "DOTFILES_PROJECTS", "DOTFILES_VERBOSE", "DOTFILES_WARN_FILE",
];

/// Run `bin` (a path such as `env!("CARGO_BIN_EXE_guard")`) with `args`, feed `stdin`, return
/// `(exit code, stdout, stderr)`. `env` entries win over the defaults above; `cwd` is optional.
pub fn run_bin(
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
    let own_home = if given("HOME") { None } else { Some(temp_home("run")) };
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
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
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
