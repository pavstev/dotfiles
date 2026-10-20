//! Cursor store. v1 has the path only; the reader waits for a pure-Rust SQLite reader.

use std::path::{Path, PathBuf};

/// `state.vscdb` in Cursor's `User/globalStorage`, per OS.
pub fn cursor_db(home: &Path) -> PathBuf {
    let user = if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("Cursor").join("User")
    } else if cfg!(windows) {
        home.join("AppData").join("Roaming").join("Cursor").join("User")
    } else {
        home.join(".config").join("Cursor").join("User")
    };
    user.join("globalStorage").join("state.vscdb")
}
