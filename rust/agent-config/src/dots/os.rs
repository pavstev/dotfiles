//! Which OS this is, commands on PATH, links, the clock. Every OS branch of `dots` asks here.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// True on macOS.
pub fn is_macos() -> bool { cfg!(target_os = "macos") }
/// True on Linux (WSL too).
pub fn is_linux() -> bool { cfg!(target_os = "linux") }
/// True on Windows.
pub fn is_windows() -> bool { cfg!(windows) }

/// This Linux uses musl, not glibc (Alpine). Homebrew on Linux needs glibc.
pub fn is_musl() -> bool {
    is_linux()
        && fs::read_dir("/lib")
            .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-")))
            .unwrap_or(false)
}

/// The home folder: $HOME, else the OS answer.
pub fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()).or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

/// A person can answer: stdin and stderr are the terminal (prompts draw on stderr).
pub fn at_terminal() -> bool { io::stdin().is_terminal() && io::stderr().is_terminal() }

/// The full path of a command on PATH. On Windows the PATHEXT names come first, the bare name
/// last: `cursor` finds cursor.cmd, not the shell script next to it.
pub fn which(cmd: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let exts: Vec<String> = if is_windows() {
        let pe = env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
        pe.split(';').filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).chain(std::iter::once(String::new())).collect()
    } else {
        vec![String::new()]
    };
    for dir in env::split_paths(&path) {
        for e in &exts {
            let p = dir.join(format!("{}{}", cmd, e));
            if is_executable(&p) { return Some(p); }
        }
    }
    None
}

/// The bash that runs steps and the shim. On Windows a bare "bash" finds System32\bash.exe
/// (WSL) before PATH, so take the one on PATH (Git Bash).
pub fn bash() -> PathBuf { which("bash").unwrap_or_else(|| PathBuf::from("bash")) }

/// True if the command resolves on PATH. The basis of "skip if installed".
pub fn have(cmd: &str) -> bool { which(cmd).is_some() }

/// True if the path is a file this user can run.
pub use crate::pycompat::is_executable;

/// Put a folder first on PATH, once.
pub fn path_prepend(dir: &Path) {
    let cur = env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<PathBuf> = env::split_paths(&cur).collect();
    if parts.iter().any(|p| p == dir) { return; }
    parts.insert(0, dir.to_path_buf());
    if let Ok(joined) = env::join_paths(parts) { set_env("PATH", joined); }
}

/// Set an environment variable of this process.
pub fn set_env<V: Into<OsString>>(k: &str, v: V) {
    let v: OsString = v.into();
    // SAFETY: dots sets its environment from the main thread, before any thread starts.
    unsafe { env::set_var(k, v) }
}
/// Remove an environment variable of this process.
pub fn unset_env(k: &str) {
    // SAFETY: as in set_env.
    unsafe { env::remove_var(k) }
}

/// Put a folder last on PATH, once.
pub fn path_append(dir: &Path) {
    let cur = env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<PathBuf> = env::split_paths(&cur).collect();
    if parts.iter().any(|p| p == dir) { return; }
    parts.push(dir.to_path_buf());
    if let Ok(joined) = env::join_paths(parts) { set_env("PATH", joined); }
}

/// scripts/tools-env.sh `tools_add_paths`: where tools from mise, cargo, WinGet and the mise
/// installer land. They go on the END of PATH, so a tool already on PATH keeps its place. Not
/// tested with is_dir: a step that runs later in the same run makes these folders.
pub fn tool_paths() {
    let h = home();
    for d in [".local/bin", ".local/share/mise/shims", ".cargo/bin"] { path_append(&h.join(d)); }
    if is_windows()
        && let Some(l) = env::var_os("LOCALAPPDATA").map(PathBuf::from)
    {
        for d in ["mise/shims", "Microsoft/WinGet/Links"] {
            let p = l.join(d);
            if p.is_dir() { path_append(&p); }
        }
    }
}

/// Make `brew` and brew-installed tools available, the way `brew shellenv` does. No-op
/// without Homebrew. Homebrew's rustup is keg-only: its cargo is in opt/rustup/bin.
pub fn brew_env() {
    let mut cands = vec![
        PathBuf::from("/opt/homebrew/bin/brew"),
        PathBuf::from("/usr/local/bin/brew"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin/brew"),
    ];
    cands.push(home().join(".linuxbrew/bin/brew"));
    for b in cands {
        if !is_executable(&b) { continue; }
        let Some(prefix) = b.parent().and_then(|p| p.parent()) else { continue };
        let repo = if prefix.join("Homebrew/.git").exists() { prefix.join("Homebrew") } else { prefix.to_path_buf() };
        set_env("HOMEBREW_PREFIX", prefix.as_os_str());
        set_env("HOMEBREW_CELLAR", prefix.join("Cellar").into_os_string());
        set_env("HOMEBREW_REPOSITORY", repo.into_os_string());
        path_prepend(&prefix.join("sbin"));
        path_prepend(&prefix.join("bin"));
        let r = prefix.join("opt/rustup/bin");
        if r.is_dir() { path_prepend(&r); }
        return;
    }
}

/// The target of a link, or None.
pub fn readlink(p: &Path) -> Option<PathBuf> { fs::read_link(p).ok() }

/// True if the path itself is a link (a broken one counts).
pub fn is_symlink(p: &Path) -> bool { p.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) }

/// The path exists, or is a link (even a broken one).
pub fn exists_or_link(p: &Path) -> bool { p.exists() || is_symlink(p) }

/// Make the link `dst` -> `src`. On Windows a folder needs a folder link.
pub fn symlink(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(unix)]
    { std::os::unix::fs::symlink(src, dst) }
    #[cfg(windows)]
    {
        if src.is_dir() { std::os::windows::fs::symlink_dir(src, dst) } else { std::os::windows::fs::symlink_file(src, dst) }
    }
}

/// Windows only: true when this user can make a real symlink (Developer Mode on).
pub fn windows_links_ready() -> bool {
    let Ok(d) = tempfile::tempdir() else { return false };
    let a = d.path().join("a");
    let b = d.path().join("b");
    if fs::write(&a, "").is_err() { return false; }
    symlink(&a, &b).is_ok() && is_symlink(&b)
}

/// Local year, month, day, hour, minute, second (UTC on Windows).
fn local_parts(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Tm { sec: i32, min: i32, hour: i32, mday: i32, mon: i32, year: i32, wday: i32, yday: i32, isdst: i32, gmtoff: i64, zone: *const u8 }
        unsafe extern "C" { fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm; }
        let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
        // SAFETY: both pointers are valid for the call; localtime_r only writes into `tm`.
        let r = unsafe { localtime_r(&secs, &mut tm) };
        if !r.is_null() {
            return (tm.year as i64 + 1900, tm.mon as i64 + 1, tm.mday as i64, tm.hour as i64, tm.min as i64, tm.sec as i64);
        }
    }
    utc_parts(secs)
}

/// Year, month, day, hour, minute, second in UTC.
pub fn utc_parts(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (yy, mm, dd) = crate::pycompat::civil_from_days(days);
    (yy, mm, dd, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The time now as YYYYmmdd-HHMMSS, for backup names.
pub fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, mo, d, h, mi, s) = local_parts(secs);
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, mo, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamp_shape() {
        let t = timestamp();
        assert_eq!(t.len(), 15);
        assert_eq!(&t[8..9], "-");
    }
    #[test]
    fn utc_date_math() {
        assert_eq!(utc_parts(1792413296), (2026, 10, 19, 12, 34, 56));
        assert_eq!(utc_parts(0), (1970, 1, 1, 0, 0, 0));
    }
    #[test]
    fn which_finds_sh() {
        if cfg!(unix) { assert!(have("sh")); }
        assert!(!have("surely-not-a-command-xyz"));
    }
}
