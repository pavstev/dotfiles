//! Paths every tool of this crate needs.

use std::env;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The user's home folder: `$HOME`, then `%USERPROFILE%` on Windows, then the OS answer, then `.`.
/// `$HOME` comes first so tests and Git Bash can point it elsewhere. An empty value counts as unset.
pub fn home() -> PathBuf {
    let var = |k: &str| env::var_os(k).filter(|v| !v.is_empty());
    var("HOME")
        .or_else(|| {
            if cfg!(windows) {
                var("USERPROFILE")
            } else {
                None
            }
        })
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Seconds since the Unix epoch; 0 when the clock is before it.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
