//! Paths every tool of this crate needs.

use std::env;
use std::path::PathBuf;

/// The user's home folder: `$HOME`, then `%USERPROFILE%` on Windows, then the OS answer, then `.`.
/// `$HOME` comes first so tests and Git Bash can point it elsewhere. An empty value counts as unset.
pub fn home() -> PathBuf {
    let var = |k: &str| env::var_os(k).filter(|v| !v.is_empty());
    var("HOME")
        .or_else(|| if cfg!(windows) { var("USERPROFILE") } else { None })
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}
