//! Reading a Claude Code transcript from its end. The guard hooks for Claude Code and Cursor use
//! it to find the context size of the newest assistant turn without reading the whole file.

use crate::pycompat::truthy;
use serde_json::Value;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Newest-first lines of a file, read backwards in chunks, so a caller that stops early never
/// reads the rest of the file. Blank lines are skipped.
pub struct RevLines {
    f: fs::File,
    pos: u64,
    chunk: u64,
    buf: Vec<u8>,
    pending: Vec<Vec<u8>>, // forward order; pop() yields the newest
    finished: bool,
}

/// The lines of `path`, newest first.
pub fn rev_lines(path: &Path) -> io::Result<RevLines> {
    let mut f = fs::File::open(path)?;
    let len = f.seek(SeekFrom::End(0))?;
    Ok(RevLines { f, pos: len, chunk: 1 << 16, buf: Vec::new(), pending: Vec::new(), finished: false })
}

fn is_blank(b: &[u8]) -> bool {
    b.iter().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
}

impl Iterator for RevLines {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some(l) = self.pending.pop() {
                return Some(l);
            }
            if self.pos == 0 {
                if self.finished {
                    return None;
                }
                self.finished = true;
                let b = std::mem::take(&mut self.buf);
                return if is_blank(&b) { None } else { Some(b) };
            }
            let n = self.chunk.min(self.pos);
            self.pos -= n;
            let mut data = vec![0u8; n as usize];
            if self.f.seek(SeekFrom::Start(self.pos)).is_err() || self.f.read_exact(&mut data).is_err() {
                self.pos = 0;
                self.buf.clear();
                continue;
            }
            data.extend_from_slice(&self.buf);
            let parts: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
            if parts.len() == 1 {
                self.chunk *= 2; // one giant line: read bigger pieces
            }
            self.buf = parts[0].to_vec();
            self.pending = parts[1..].iter().filter(|l| !is_blank(l)).map(|l| l.to_vec()).collect();
        }
    }
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// A line that holds a JSON object.
fn record(line: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(line).ok().filter(Value::is_object)
}

/// The context size of the newest assistant record with usage: input plus cache tokens.
/// 0 when a compact comes first, when the file cannot be read, or when no record has usage.
pub fn context_size(path: &Path) -> i64 {
    let Ok(lines) = rev_lines(path) else { return 0 };
    for line in lines {
        if (has(&line, b"compact_boundary") || has(&line, b"isCompactSummary"))
            && let Some(rec) = record(&line)
            && (rec["subtype"].as_str() == Some("compact_boundary") || truthy(&rec["isCompactSummary"]))
        {
            return 0;
        }
        if !has(&line, b"\"assistant\"") {
            continue;
        }
        let Some(rec) = record(&line) else { continue };
        if rec["type"].as_str() != Some("assistant") {
            continue;
        }
        let usage = &rec["message"]["usage"];
        if !usage.is_object() {
            continue;
        }
        let tokens = |k: &str| usage[k].as_f64().unwrap_or(0.0);
        let total = tokens("input_tokens") + tokens("cache_creation_input_tokens") + tokens("cache_read_input_tokens");
        if total > 0.0 {
            return total as i64;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rev_lines_is_newest_first_and_lazy() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.txt");
        let mut body = String::new();
        for i in 0..20000 {
            body.push_str(&format!("line{i}\n\n"));
        }
        fs::write(&p, body).unwrap();
        let mut it = rev_lines(&p).unwrap();
        assert_eq!(it.next().as_deref(), Some(&b"line19999"[..]));
        assert_eq!(it.next().as_deref(), Some(&b"line19998"[..]));
        assert_eq!(it.count(), 19998);
    }
}
