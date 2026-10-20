//! `ctx`: outline a source file and read one symbol of it, so an agent does not cat the whole file.
//!
//! ```text
//! ctx outline FILE                         symbols with line ranges
//! ctx read FILE SYMBOL [--if-changed HASH] one symbol's lines, numbered
//! ```
//!
//! Exit 0 on output, 1 on a refusal or a symbol that is not found, 2 on a usage error. Messages
//! go to stdout, next to the code. The scan is a line scan (`agent_config::codeview`), the same
//! on every OS.

use agent_config::codeview::{self, Refusal, Symbol};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// Longest symbol body printed, in lines.
const CAP: usize = 400;
/// Same limit as `codeview`: a file over this is refused.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Parser)]
#[command(name = "ctx", about = "Outline a source file and read one symbol of it")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the symbols of FILE with their line ranges
    Outline { file: String },
    /// Print the lines of one symbol, numbered like the Read tool
    Read {
        file: String,
        /// A name (`inner`) or a path from the outermost symbol (`outer.inner`)
        symbol: String,
        /// Print `unchanged (hash H)` instead of the code when the file still has this hash
        #[arg(long = "if-changed", value_name = "HASH")]
        if_changed: Option<String>,
    },
}

/// What a command prints, and its exit code.
struct Reply {
    text: String,
    code: i32,
}

fn ok(text: String) -> Reply {
    Reply { text, code: 0 }
}

fn refused(file: &str, why: &str) -> Reply {
    Reply { text: format!("• ctx: {file} skipped: {why}\n"), code: 1 }
}

/// The file as text and the first 8 hex chars of the sha256 of its bytes.
fn load(file: &str) -> Result<(String, String), Reply> {
    let mut buf = Vec::new();
    let read = File::open(file).and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut buf));
    if let Err(e) = read {
        return Err(refused(file, &format!("cannot read: {e}")));
    }
    if buf.len() as u64 > MAX_BYTES {
        return Err(refused(file, "over 2 MB"));
    }
    let digest = Sha256::digest(&buf);
    let hash = format!("{:02x}{:02x}{:02x}{:02x}", digest[0], digest[1], digest[2], digest[3]);
    match String::from_utf8(buf) {
        Ok(text) if !text.contains('\0') => Ok((text, hash)),
        _ => Err(refused(file, "not text (binary or not UTF-8)")),
    }
}

fn refusal_reply(file: &str, r: &Refusal) -> Reply {
    match r {
        Refusal::TooBig => refused(file, "over 2 MB"),
        Refusal::LongLine => refused(file, "a line is over 10,000 chars (minified code?)"),
        Refusal::NotText => refused(file, "not text (binary or not UTF-8)"),
        Refusal::NoScanner(ext) => refused(file, &no_scanner(ext)),
    }
}

fn no_scanner(ext: &str) -> String {
    if ext.is_empty() { "no scanner for this file type; use Read".into() } else { format!("no scanner for .{ext}; use Read") }
}

/// `outer.inner` style path of every symbol.
fn paths(symbols: &[Symbol]) -> Vec<String> {
    let mut stack: Vec<&str> = Vec::new();
    symbols
        .iter()
        .map(|s| {
            stack.truncate(s.depth);
            stack.push(&s.name);
            stack.join(".")
        })
        .collect()
}

fn lev(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = (diag + usize::from(ca != cb)).min(up + 1).min(row[j] + 1);
            diag = up;
        }
    }
    row[b.len()]
}

/// Indexes of the symbols that SYMBOL names: an exact path first, then a bare name, then the end
/// of a path.
fn matches(symbols: &[Symbol], paths: &[String], query: &str) -> Vec<usize> {
    let exact: Vec<usize> = (0..symbols.len()).filter(|&i| paths[i] == query).collect();
    if !exact.is_empty() {
        return exact;
    }
    let bare: Vec<usize> = (0..symbols.len()).filter(|&i| symbols[i].name == query).collect();
    if !bare.is_empty() {
        return bare;
    }
    let tail = format!(".{query}");
    (0..symbols.len()).filter(|&i| paths[i].ends_with(&tail)).collect()
}

fn outline_cmd(file: &str) -> Reply {
    let (text, _) = match load(file) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let lines = text.lines().count();
    match codeview::outline(Path::new(file), &text) {
        Ok(symbols) if symbols.is_empty() => ok(format!("{file}: no symbols found ({lines} lines)\n")),
        Ok(symbols) => ok(codeview::render(&symbols)),
        Err(Refusal::NoScanner(ext)) => ok(format!("{file}: {lines} lines, {}\n", no_scanner(&ext))),
        Err(r) => refusal_reply(file, &r),
    }
}

fn read_cmd(file: &str, query: &str, if_changed: Option<&str>) -> Reply {
    let (text, hash) = match load(file) {
        Ok(x) => x,
        Err(r) => return r,
    };
    if if_changed.is_some_and(|h| h.eq_ignore_ascii_case(&hash)) {
        return ok(format!("unchanged (hash {hash})\n"));
    }
    let symbols = match codeview::outline(Path::new(file), &text) {
        Ok(s) => s,
        Err(r) => return refusal_reply(file, &r),
    };
    let paths = paths(&symbols);
    let found = matches(&symbols, &paths, query);
    match found.as_slice() {
        [] => not_found(file, query, &symbols, &paths),
        [i] => {
            let scan = codeview::scan_name(Path::new(file)).unwrap_or("?");
            ok(body(file, &text, &symbols[*i], &paths[*i], scan, &hash))
        }
        many => {
            let mut out = format!("ambiguous: {query} matches {} symbols in {file}; use a longer path\n", many.len());
            for &i in many {
                out.push_str(&format!("  {file}:{}-{}  {}\n", symbols[i].start, symbols[i].end, paths[i]));
            }
            Reply { text: out, code: 1 }
        }
    }
}

fn not_found(file: &str, query: &str, symbols: &[Symbol], paths: &[String]) -> Reply {
    if symbols.is_empty() {
        return Reply { text: format!("not found: {query} ({file} has no symbols)\n"), code: 1 };
    }
    let mut ranked: Vec<(usize, usize)> =
        (0..symbols.len()).map(|i| (lev(query, &symbols[i].name).min(lev(query, &paths[i])), i)).collect();
    ranked.sort();
    let mut out = format!("not found: {query} in {file}. Closest:\n");
    let mut seen: Vec<&str> = Vec::new();
    for (_, i) in ranked {
        if seen.len() == 5 {
            break;
        }
        if seen.contains(&paths[i].as_str()) {
            continue;
        }
        seen.push(&paths[i]);
        out.push_str(&format!("  {}  ({}-{})\n", paths[i], symbols[i].start, symbols[i].end));
    }
    Reply { text: out, code: 1 }
}

fn body(file: &str, text: &str, sym: &Symbol, path: &str, scan: &str, hash: &str) -> String {
    let mut out = format!("{file}:{}-{}  {path}  scan={scan}  hash={hash}\n", sym.start, sym.end);
    let total = sym.end - sym.start + 1;
    for (i, line) in text.lines().enumerate().skip(sym.start - 1).take(total.min(CAP)) {
        out.push_str(&format!("{}\t{line}\n", i + 1));
    }
    if total > CAP {
        out.push_str(&format!("… cut at {CAP} of {total} lines; use Read with offset\n"));
    }
    out
}

fn main() {
    let cli = Cli::parse();
    let reply = match &cli.cmd {
        Cmd::Outline { file } => outline_cmd(file),
        Cmd::Read { file, symbol, if_changed } => read_cmd(file, symbol, if_changed.as_deref()),
    };
    // A closed pipe (`ctx read ... | head`) is not an error worth a panic.
    let _ = std::io::stdout().write_all(reply.text.as_bytes());
    std::process::exit(reply.code);
}
