//! Readers for the stores of past agent sessions. v1 reads Claude Code only.
//! Opencode and Cursor have path functions here and wait for a pure-Rust SQLite reader.

pub mod claude;
pub mod cursor;
pub mod opencode;

pub use claude::{claude_root, read_claude};
pub use cursor::cursor_db;
pub use opencode::opencode_db;

/// One searchable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub source: &'static str,
    /// The file the message was read from (empty for a store that is not file based). An index
    /// uses it to replace the documents of a file that changed.
    pub file: String,
    pub session: String,
    pub project: String,
    pub title: String,
    /// Seconds since the epoch, UTC.
    pub time: i64,
    /// `user`, `assistant`, `tool` or `tool_error`.
    pub role: String,
    /// Already passed through `redact::redact`.
    pub text: String,
}

/// Why a whole source was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    Missing,
    Locked,
    Error(String),
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("not found"),
            Self::Locked => f.write_str("locked"),
            Self::Error(e) => f.write_str(e),
        }
    }
}
