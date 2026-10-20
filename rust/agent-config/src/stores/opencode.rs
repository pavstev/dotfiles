//! Opencode store. v1 has the path only; the reader waits for a pure-Rust SQLite reader.

use std::path::{Path, PathBuf};

/// `opencode.db` in the Opencode data dir (the same path on every OS).
pub fn opencode_db(home: &Path) -> PathBuf {
    home.join(".local").join("share").join("opencode").join("opencode.db")
}
