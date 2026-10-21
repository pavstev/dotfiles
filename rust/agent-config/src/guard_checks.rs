//! Guard checks G14-G17 as a library, so the Claude guard, the Cursor adapter and the Opencode
//! plugin run the same code. Each check takes plain values (a command, a tool name and input, a
//! cwd) and returns a [`Verdict`]. Whether a check is on (registry, presets, `GUARD_OFF`) is the
//! caller's business; so is turning a verdict into a hook answer.
//!
//! G14 BSD `sed -i` on macOS (deny). G15 reads of `local.zsh` and `ps` command lines (deny).
//! G16 outbound sends that hold a private word or a secret (ask). G17 assistant attribution
//! lines (deny).

use regex::Regex;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

const SECRET_FILE: &str = "~/.config/zsh/local.zsh";

const MSG_G14: &str = "G14: bare sed -i on macOS eats the script as the backup suffix. Give a suffix: `sed -i '' 's/a/b/' f` (or `-i.bak`). Or use the Edit tool.";
const MSG_G15_FILE: &str = "G15: never read local.zsh: it holds API keys. Use `test -f` or `git check-ignore` for facts about it, or ask the user.";
const MSG_G15_PS: &str =
    "G15: this ps form prints full command lines, which can hold API keys. Use `ps -o pid=,etime=,ucomm= -p <pid>`.";
const MSG_G15_PGREP: &str =
    "G15: pgrep with -l or -a prints full command lines, which can hold API keys. Use `pgrep -f <name>` for pids.";
const MSG_G15_PROCS: &str = "G15: procs prints full command lines, which can hold API keys. Use `pgrep -f <name>` for pids, then `ps -o pid=,etime=,ucomm= -p <pid>`.";

/// The G15 message for a search that can reach local.zsh: the exact exclude flag of the tool used.
fn msg_g15_search(tool: &str) -> String {
    let flag = match tool {
        "rg" => "rg -g '!local.zsh' ...".to_string(),
        "ag" => "ag --ignore local.zsh ...".to_string(),
        "ack" => "ack --ignore-file=is:local.zsh ...".to_string(),
        other => format!("{other} --exclude=local.zsh ..."),
    };
    format!("G15: this search can reach local.zsh (API keys). Exclude it with `{flag}`.")
}

static GLOB_CHARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*?\[]").unwrap());

/// What a check says about one tool call: let it through, ask the user, or refuse.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow,
    Ask(String),
    Deny(String),
}

fn deny_or_allow(msg: Option<&str>) -> Verdict {
    msg.map(|m| Verdict::Deny(m.to_string())).unwrap_or(Verdict::Allow)
}

// --- small path helpers (shared with the guard binary) ----------------------------

/// macOS rules (BSD sed, case-folded paths, ps). `GUARD_PLATFORM=darwin|linux|win32` overrides the
/// build target, so a Linux run can test the macOS branches.
pub fn is_macos() -> bool {
    match env::var("GUARD_PLATFORM") {
        Ok(p) if !p.is_empty() => p == "darwin",
        _ => cfg!(target_os = "macos"),
    }
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn path_join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_string()
    } else if a.ends_with('/') {
        format!("{}{}", a, b)
    } else {
        format!("{}/{}", a, b)
    }
}

pub fn expanduser(p: &str) -> String {
    if !p.starts_with('~') {
        return p.to_string();
    }
    let i = p.find('/').unwrap_or(p.len());
    if i != 1 {
        return p.to_string(); // ~user form: left as is
    }
    let home = env::var("HOME").ok().or_else(|| dirs::home_dir().map(|h| h.to_string_lossy().into_owned()));
    let Some(home) = home else { return p.to_string() };
    let home = home.trim_end_matches('/');
    let out = format!("{}{}", home, &p[i..]);
    if out.is_empty() { "/".to_string() } else { out }
}

/// Like Python's non-strict os.path.realpath: resolve symlinks of the existing prefix, normalise
/// `..` as it goes, append the rest.
pub fn realpath(p: &str) -> String {
    let abs = if p.starts_with('/') {
        p.to_string()
    } else {
        let cwd = env::current_dir().map(|c| c.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".to_string());
        format!("{}/{}", cwd, p)
    };
    let mut resolved = String::from("/");
    let mut todo: VecDeque<String> = abs.split('/').map(String::from).collect();
    let mut links = 0;
    while let Some(c) = todo.pop_front() {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            if let Some(idx) = resolved.rfind('/') {
                resolved.truncate(if idx == 0 { 1 } else { idx });
            }
            continue;
        }
        let cand = if resolved == "/" { format!("/{}", c) } else { format!("{}/{}", resolved, c) };
        let is_link = fs::symlink_metadata(&cand).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if is_link && links < 40 {
            links += 1;
            match fs::read_link(&cand) {
                Ok(t) => {
                    let t = t.to_string_lossy().into_owned();
                    if t.starts_with('/') {
                        resolved = String::from("/");
                    }
                    for comp in t.split('/').rev() {
                        todo.push_front(comp.to_string());
                    }
                }
                Err(_) => resolved = cand,
            }
        } else {
            resolved = cand;
        }
    }
    resolved
}

// --- helpers shared by G14 to G17: shell splitting and Python path functions ----

/// `os.path.normpath` for posix paths.
fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let initial = if p.starts_with("//") && !p.starts_with("///") {
        2
    } else if p.starts_with('/') {
        1
    } else {
        0
    };
    let mut out: Vec<&str> = Vec::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (initial == 0 && out.is_empty()) || out.last() == Some(&"..") {
            out.push(c);
        } else if !out.is_empty() {
            out.pop();
        }
    }
    let body = out.join("/");
    let s = format!("{}{}", "/".repeat(initial), body);
    if s.is_empty() { ".".to_string() } else { s }
}

/// `os.path.split`.
fn path_split(p: &str) -> (String, String) {
    let i = p.rfind('/').map(|x| x + 1).unwrap_or(0);
    let (mut head, tail) = (p[..i].to_string(), p[i..].to_string());
    if !head.is_empty() && head.chars().any(|c| c != '/') {
        head = head.trim_end_matches('/').to_string();
    }
    (head, tail)
}

fn dirname(p: &str) -> String {
    path_split(p).0
}

/// `os.path.expandvars`: $name and ${name}; unknown names stay as they are.
fn expandvars(p: &str) -> String {
    if !p.contains('$') {
        return p.to_string();
    }
    let cs: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '$' && i + 1 < cs.len() {
            let (name, end) = if cs[i + 1] == '{' {
                match cs[i + 2..].iter().position(|&c| c == '}') {
                    Some(k) => (Some(cs[i + 2..i + 2 + k].iter().collect::<String>()), i + 3 + k),
                    None => (None, i),
                }
            } else {
                let mut j = i + 1;
                while j < cs.len() && (cs[j].is_ascii_alphanumeric() || cs[j] == '_') {
                    j += 1;
                }
                if j > i + 1 { (Some(cs[i + 1..j].iter().collect::<String>()), j) } else { (None, i) }
            };
            if let Some(n) = name {
                if let Ok(v) = env::var(&n) {
                    out.push_str(&v);
                    i = end;
                    continue;
                }
                out.extend(&cs[i..end]);
                i = end;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// Python `fnmatch.fnmatchcase`: `*` matches anything (also `/`), `?` one char, `[seq]`, `[!seq]`.
fn fnmatch(name: &str, pat: &str) -> bool {
    type Class = (usize, bool, Vec<(char, char)>);
    fn class(pat: &[char], i: usize) -> Option<Class> {
        // pat[i] == '['; returns (index after ']', negated, ranges)
        let n = pat.len();
        let mut j = i + 1;
        if j < n && pat[j] == '!' {
            j += 1;
        }
        if j < n && pat[j] == ']' {
            j += 1;
        }
        while j < n && pat[j] != ']' {
            j += 1;
        }
        if j >= n {
            return None;
        }
        let mut k = i + 1;
        let neg = pat[k] == '!';
        if neg {
            k += 1;
        }
        let mut ranges = Vec::new();
        while k < j {
            if k + 2 < j && pat[k + 1] == '-' {
                ranges.push((pat[k], pat[k + 2]));
                k += 3;
            } else {
                ranges.push((pat[k], pat[k]));
                k += 1;
            }
        }
        Some((j + 1, neg, ranges))
    }
    fn go(n: &[char], p: &[char]) -> bool {
        if p.is_empty() {
            return n.is_empty();
        }
        match p[0] {
            '*' => {
                let mut rest = p;
                while !rest.is_empty() && rest[0] == '*' {
                    rest = &rest[1..];
                }
                (0..=n.len()).any(|k| go(&n[k..], rest))
            }
            '?' => !n.is_empty() && go(&n[1..], &p[1..]),
            '[' => match class(p, 0) {
                Some((end, neg, ranges)) => {
                    !n.is_empty()
                        && (ranges.iter().any(|&(a, b)| a <= n[0] && n[0] <= b) != neg)
                        && go(&n[1..], &p[end..])
                }
                None => !n.is_empty() && n[0] == '[' && go(&n[1..], &p[1..]),
            },
            c => !n.is_empty() && n[0] == c && go(&n[1..], &p[1..]),
        }
    }
    let n: Vec<char> = name.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    go(&n, &p)
}

fn case_fold(s: &str) -> String {
    if is_macos() { s.to_lowercase() } else { s.to_string() }
}

/// `shlex.split(s)` (posix, no comments); None where Python raises ValueError.
fn shlex_split(s: &str) -> Option<Vec<String>> {
    let cs: Vec<char> = s.chars().collect();
    let ws = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        // one token
        let mut piece = String::new();
        let mut quoted = false;
        let mut state = ' ';
        let mut escaped = ' ';
        let mut got = false;
        while i <= cs.len() {
            let c = cs.get(i).copied();
            i += 1;
            match state {
                ' ' => match c {
                    None => break,
                    Some(c) if ws(c) => {
                        if !piece.is_empty() || quoted {
                            got = true;
                            break;
                        }
                    }
                    Some('\\') => {
                        escaped = 'a';
                        state = '\\';
                    }
                    Some(c @ ('\'' | '"')) => state = c,
                    Some(c) => {
                        piece.push(c);
                        state = 'a';
                    }
                },
                '\'' | '"' => {
                    quoted = true;
                    let c = c?; // No closing quotation
                    if c == state {
                        state = 'a';
                    } else if state == '"' && c == '\\' {
                        escaped = state;
                        state = '\\';
                    } else {
                        piece.push(c);
                    }
                }
                '\\' => {
                    let c = c?; // No escaped character
                    if (escaped == '\'' || escaped == '"') && c != state && c != escaped {
                        piece.push(state);
                    }
                    piece.push(c);
                    state = escaped;
                }
                _ => match c {
                    None => break,
                    Some(c) if ws(c) => {
                        state = ' ';
                        if !piece.is_empty() || quoted {
                            got = true;
                            break;
                        }
                    }
                    Some(c @ ('\'' | '"')) => state = c,
                    Some('\\') => {
                        escaped = 'a';
                        state = '\\';
                    }
                    Some(c) => piece.push(c),
                },
            }
        }
        if got || !piece.is_empty() || quoted {
            out.push(piece);
        }
        if i > cs.len() {
            break;
        }
    }
    Some(out)
}

/// Delimiters of the here-documents started on one line: `<<WORD`, `<<-'WORD'`, not `<<<`.
fn heredoc_delims(line: &str) -> Vec<String> {
    let cs: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < cs.len() {
        if cs[i] == '<' && cs[i + 1] == '<' && (i == 0 || cs[i - 1] != '<') {
            let mut j = i + 2;
            if j < cs.len() && cs[j] == '-' {
                j += 1;
            }
            while j < cs.len() && cs[j].is_whitespace() {
                j += 1;
            }
            let quote = if j < cs.len() && (cs[j] == '\'' || cs[j] == '"') { Some(cs[j]) } else { None };
            let mut k = if quote.is_some() { j + 1 } else { j };
            if k < cs.len() && (cs[k].is_ascii_alphabetic() || cs[k] == '_') {
                let st = k;
                while k < cs.len() && (cs[k].is_ascii_alphanumeric() || cs[k] == '_') {
                    k += 1;
                }
                let word: String = cs[st..k].iter().collect();
                if let Some(q) = quote {
                    if k < cs.len() && cs[k] == q {
                        out.push(word);
                        i = k + 1;
                        continue;
                    }
                } else {
                    out.push(word);
                    i = k;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn strip_heredocs(cmd: &str) -> String {
    let (mut out, mut pending): (Vec<&str>, Vec<String>) = (Vec::new(), Vec::new());
    for line in cmd.split('\n') {
        if !pending.is_empty() {
            if line.trim() == pending[0] {
                pending.remove(0);
            }
            continue;
        }
        out.push(line);
        pending.extend(heredoc_delims(line));
    }
    out.join("\n")
}

/// The text of every here-document in a command (an unterminated one runs to the end).
fn heredoc_bodies(cmd: &str) -> Vec<String> {
    let (mut out, mut pending, mut cur): (Vec<String>, Vec<String>, String) = (Vec::new(), Vec::new(), String::new());
    for line in cmd.split('\n') {
        if !pending.is_empty() {
            if line.trim() == pending[0] {
                pending.remove(0);
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push_str(line);
                cur.push('\n');
            }
            continue;
        }
        pending.extend(heredoc_delims(line));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Split a command line at ; | & newline ( ) ` and $( outside quotes. Quotes stay in the text.
/// Each piece comes with the line (newlines before it, quoted ones too) it starts on.
fn segments(cmd: &str) -> Vec<(String, usize)> {
    let cs: Vec<char> = cmd.chars().collect();
    let (mut segs, mut cur, mut quote, mut i) = (Vec::new(), String::new(), None::<char>, 0);
    let (mut line, mut start) = (0usize, 0usize);
    while i < cs.len() {
        let c = cs[i];
        if let Some(q) = quote {
            cur.push(c);
            if c == '\n' {
                line += 1;
            }
            if q == '"' && c == '\\' && i + 1 < cs.len() {
                cur.push(cs[i + 1]);
                if cs[i + 1] == '\n' {
                    line += 1;
                }
                i += 1;
            } else if c == q {
                quote = None;
            }
        } else if c == '\\' && i + 1 < cs.len() {
            cur.push(c);
            cur.push(cs[i + 1]);
            if cs[i + 1] == '\n' {
                line += 1;
            }
            i += 1;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            cur.push(c);
        } else if ";|&\n()`".contains(c) || (c == '$' && cs.get(i + 1) == Some(&'(')) {
            segs.push((std::mem::take(&mut cur), start));
            if c == '\n' {
                line += 1;
            }
            if c == '$' {
                i += 1;
            }
            start = line;
        } else {
            cur.push(c);
        }
        i += 1;
    }
    segs.push((cur, start));
    segs
}

fn is_assign(t: &str) -> bool {
    let mut it = t.chars();
    match it.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    for c in it {
        if c == '=' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    false
}

fn wrapper_opts(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "sudo" => &["-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U"],
        "doas" => &["-u", "-C"],
        "env" => &["-u", "-C", "-S"],
        "xargs" => &["-I", "-n", "-P", "-L", "-s", "-d", "-E", "-a", "-J"],
        "nice" => &["-n"],
        "timeout" => &["-s", "-k"],
        "exec" => &["-a"],
        "command" | "builtin" | "nohup" | "time" => &[],
        _ => return None,
    })
}

/// Drop env assignments, reserved words and wrappers (sudo, env, xargs ...) from the front.
fn peel(mut tokens: Vec<String>) -> Vec<String> {
    const RESERVED: &[&str] = &["if", "then", "else", "elif", "do", "while", "until", "{", "}", "!"];
    for _ in 0..8 {
        let mut i = 0;
        while i < tokens.len() && (RESERVED.contains(&tokens[i].as_str()) || is_assign(&tokens[i])) {
            i += 1;
        }
        tokens.drain(..i);
        if tokens.is_empty() {
            return tokens;
        }
        let base = basename(&tokens[0]).to_string();
        let Some(optval) = wrapper_opts(&base) else { return tokens };
        let mut i = 1;
        while i < tokens.len() && tokens[i].starts_with('-') && tokens[i] != "-" {
            if tokens[i] == "--" {
                i += 1;
                break;
            }
            i += if optval.contains(&tokens[i].as_str()) { 2 } else { 1 };
        }
        if base == "timeout" {
            i += 1; // the duration
        }
        tokens.drain(..i.min(tokens.len()));
    }
    tokens
}

fn abspath(p: &str, cwd: Option<&str>) -> String {
    let mut p = expanduser(&expandvars(p));
    if !p.starts_with('/')
        && let Some(c) = cwd.filter(|c| !c.is_empty())
    {
        p = path_join(c, &p);
    }
    realpath(&p)
}

struct Cmd {
    toks: Vec<String>,
    extra: Vec<String>,
    cwd: Option<String>,
    /// A here-document starts on the line this command starts on (its stdin may be that body).
    heredoc: bool,
}

/// Every simple command of a Bash command line, wrappers peeled; commands inside `bash -c`, `eval`
/// and `find -exec` too (extra = the find words before -exec). cwd follows `cd`.
fn walk(cmd: &str, cwd: Option<String>, depth: usize) -> Vec<Cmd> {
    walk_full(cmd, cwd, depth, &mut Vec::new())
}

/// [`walk`], and the raw text of every piece that cannot be split into words (an odd quote, a
/// comment with an apostrophe, `$'..\'..'`) goes to `unparsed`: such a piece is not skipped by the
/// checks that must not miss a gh, git, curl or wget call.
fn walk_full(cmd: &str, cwd: Option<String>, depth: usize, unparsed: &mut Vec<String>) -> Vec<Cmd> {
    let mut out = Vec::new();
    let mut cwd = cwd;
    let stripped = strip_heredocs(cmd);
    let heredoc_lines: Vec<bool> = stripped.split('\n').map(|l| !heredoc_delims(l).is_empty()).collect();
    for (seg, line) in segments(&stripped) {
        let Some(split) = shlex_split(&seg) else {
            unparsed.push(seg);
            continue;
        };
        let toks = peel(split);
        if toks.is_empty() {
            continue;
        }
        let heredoc = heredoc_lines.get(line).copied().unwrap_or(false);
        let name = basename(&toks[0]).to_string();
        if (name == "cd" || name == "pushd")
            && let Some(d) = toks[1..].iter().find(|t| !t.starts_with('-'))
        {
            cwd = Some(abspath(d, cwd.as_deref()));
        }
        out.push(Cmd { toks: toks.clone(), extra: Vec::new(), cwd: cwd.clone(), heredoc });
        if depth >= 3 {
            continue;
        }
        if matches!(name.as_str(), "bash" | "sh" | "zsh" | "dash" | "ksh") {
            for (k, t) in toks.iter().enumerate().skip(1) {
                let is_c = t.len() >= 2
                    && t.starts_with('-')
                    && t.ends_with('c')
                    && t[1..].chars().all(|c| c.is_ascii_alphabetic());
                if is_c && k + 1 < toks.len() {
                    out.extend(walk_full(&toks[k + 1], cwd.clone(), depth + 1, unparsed));
                    break;
                }
            }
        } else if name == "eval" {
            out.extend(walk_full(&toks[1..].join(" "), cwd.clone(), depth + 1, unparsed));
        } else if name == "find" {
            for (j, t) in toks.iter().enumerate() {
                if matches!(t.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                    let sub: Vec<String> =
                        toks[j + 1..].iter().take_while(|u| u.as_str() != ";" && u.as_str() != "+").cloned().collect();
                    let sub = peel(sub);
                    if !sub.is_empty() {
                        out.push(Cmd { toks: sub, extra: toks[1..j].to_vec(), cwd: cwd.clone(), heredoc });
                    }
                }
            }
        }
    }
    out
}

/// True when the raw text of an unparsed piece names gh, git, curl or wget as a word, also with the
/// quote characters taken out (`g''h`, `"gh"`).
fn names_outbound_tool(raw: &str, tools: &[&str]) -> bool {
    let plain: String = raw.chars().filter(|c| !matches!(c, '\'' | '"' | '\\')).collect();
    plain
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|w| tools.iter().any(|t| w.eq_ignore_ascii_case(t)))
}

/// The Bash command of a tool input, when there is a non-empty one.
fn command_of(input: &Value) -> Option<&str> {
    input.get("command").and_then(Value::as_str).filter(|s| !s.is_empty())
}

// --- G14 sed -i ----------------------------------------------------------------

/// True when the word after a bare `-i` is an explicit BSD backup suffix: empty (`''`) or a
/// dot-word such as `.bak`. Anything else is the script, which BSD sed would eat as the suffix.
fn is_bsd_suffix(word: &str) -> bool {
    word.is_empty()
        || (word.starts_with('.') && word.len() > 1 && !word.contains(|c: char| c.is_whitespace() || c == '/'))
}

/// True when sed's arguments hold a BSD-unsafe in-place flag: -i, -ni, -Ei, --in-place[=SUFFIX]
/// with no explicit suffix. `-i.bak`, `-i ''` and `-i .bak` carry one and are safe on BSD sed.
fn sed_in_place(args: &[String]) -> bool {
    let mut skip = false;
    for (n, a) in args.iter().enumerate() {
        if skip {
            skip = false;
        } else if a == "--" {
            break;
        } else if a.starts_with("--") {
            if a == "--in-place" || a.starts_with("--in-place=") {
                return true;
            }
            skip = matches!(a.as_str(), "--expression" | "--file" | "--line-length");
        } else if a.starts_with('-') && a.chars().count() > 1 {
            let cs: Vec<char> = a.chars().collect();
            for k in 1..cs.len() {
                if cs[k] == 'i' {
                    // -i.bak: the rest of the word is the suffix. A bare -i is safe only when the
                    // next word is an explicit suffix.
                    if k == cs.len() - 1 && !args.get(n + 1).is_some_and(|next| is_bsd_suffix(next)) {
                        return true;
                    }
                    skip = k == cs.len() - 1;
                    break;
                }
                if "efl".contains(cs[k]) {
                    skip = k == cs.len() - 1;
                    break;
                }
            }
        }
    }
    false
}

/// G14: deny a bare BSD `sed -i` on macOS (it eats the script as the backup suffix). A suffix
/// argument (`-i ''`, `-i.bak`, `-i '.bak'`) passes. Quoted text, `gsed` and `sed -n` pass too.
pub fn g14_sed(cmd: &str, macos: bool) -> Verdict {
    if !macos || !cmd.contains("sed") {
        return Verdict::Allow;
    }
    let hit = std::panic::catch_unwind(|| {
        walk(cmd, None, 0).iter().any(|c| basename(&c.toks[0]) == "sed" && sed_in_place(&c.toks[1..]))
    });
    deny_or_allow(hit.unwrap_or(false).then_some(MSG_G14))
}

// --- H1 hint: read code with ctx --------------------------------------------------

/// The file that `sed -n 'A,Bp' FILE`, `cat FILE` or `cat -n FILE` reads, when `codeview` knows its
/// extension; else None. Pipes, `&&` and `;` split the line (so does `walk`); the first command of
/// the two that names a code file wins, and of several files the first one counts. Anything odd (an
/// unbalanced quote, a glob, a variable, a control character) gives None.
pub fn h1_read_target(cmd: &str) -> Option<String> {
    if !cmd.contains("sed") && !cmd.contains("cat") {
        return None; // the cheap exit: most Bash calls read nothing
    }
    std::panic::catch_unwind(|| {
        walk(cmd, None, 0).iter().find_map(|c| {
            let file = match basename(c.toks.first()?) {
                "cat" => cat_file(&c.toks[1..]),
                "sed" => sed_n_file(&c.toks[1..]),
                _ => None,
            }?;
            let odd = file.len() > 200 || file.chars().any(|ch| ch.is_control() || "*?$`{}".contains(ch));
            (!odd && crate::codeview::scan_name(Path::new(&file)).is_some()).then_some(file)
        })
    })
    .ok()
    .flatten()
}

/// A redirection word (`>`, `2>f`, `<in`) ends the file arguments.
fn is_redirect(t: &str) -> bool {
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit());
    t.starts_with('>') || t.starts_with('<')
}

fn cat_file(args: &[String]) -> Option<String> {
    args.iter().take_while(|t| !is_redirect(t)).find(|t| !t.starts_with('-')).cloned()
}

/// `sed -n SCRIPT FILE` (or `-n -e SCRIPT FILE`): the file. No `-n` or an in-place edit: None.
fn sed_n_file(args: &[String]) -> Option<String> {
    let (mut quiet, mut script) = (false, false);
    let mut it = args.iter().take_while(|t| !is_redirect(t));
    while let Some(t) = it.next() {
        if t == "--quiet" || t == "--silent" {
            quiet = true;
        } else if t == "--expression" || t == "--file" {
            script = true;
            it.next();
        } else if t.starts_with("--expression=") || t.starts_with("--file=") {
            script = true;
        } else if t.starts_with("--") {
            if t.starts_with("--in-place") {
                return None;
            }
        } else if t.len() > 1 && t.starts_with('-') {
            let flags = &t[1..];
            if flags.contains('i') {
                return None;
            }
            quiet |= flags.contains('n');
            if flags.ends_with('e') || flags.ends_with('f') {
                script = true;
                it.next(); // the script text
            }
        } else if !script {
            script = true; // the first plain word is the script
        } else {
            return quiet.then(|| t.clone());
        }
    }
    None
}

// --- G15 secret reads ----------------------------------------------------------

const SECRET_READERS: &[&str] = &[
    "cat", "bat", "batcat", "less", "more", "most", "head", "tail", "rg", "grep", "egrep", "fgrep", "ag", "ack", "sed",
    "gsed", "awk", "gawk", "mawk", "cut", "strings", "xxd", "hexdump", "od", "nl", "tac", "rev", "sort", "uniq",
    "paste", "fold", "column", "cp", "mv", "scp", "rsync", "open", "code", "cursor", "vim", "vi", "nvim", "nano",
    "view", "pbcopy", "base64", "jq", "yq", "diff", "cmp", "ln", "install",
];
const SECRET_SEARCHERS: &[&str] = &["rg", "grep", "egrep", "fgrep", "ag", "ack"];
const ARGS_FIELDS: &[&str] = &["args", "command", "cmd", "command_line", "cmdline"];
const PS_VALUE_LONG: &[&str] = &[
    "--format",
    "--pid",
    "--ppid",
    "--sid",
    "--tty",
    "--user",
    "--group",
    "--sort",
    "--cols",
    "--columns",
    "--width",
    "--lines",
    "--rows",
];

type Target = (String, String);

fn secret_target() -> Target {
    let raw = normpath(&expanduser(SECRET_FILE));
    let real = realpath(&raw);
    (raw, real)
}

/// True when `path` (word as typed) is the secret file, a glob that matches it, or (dir_ok) its folder.
fn is_secret(path: &str, cwd: Option<&str>, target: &Target, dir_ok: bool) -> bool {
    let (raw, real) = (case_fold(&target.0), case_fold(&target.1));
    let mut p = expanduser(&expandvars(path));
    if p.is_empty() || p.starts_with('-') {
        return false;
    }
    if !p.starts_with('/')
        && let Some(c) = cwd.filter(|c| !c.is_empty())
    {
        p = path_join(c, &p);
    }
    if GLOB_CHARS.is_match(&p) {
        let norm = normpath(&p);
        let (d, base) = path_split(&norm);
        return fnmatch(&raw, &case_fold(&norm)) || fnmatch(&real, &case_fold(&path_join(&realpath(&d), &base)));
    }
    let got = case_fold(&realpath(&p));
    got == real || (dir_ok && got == dirname(&real))
}

/// True for a ps call that lists processes with their command lines (aux, -ef, -o args, ww, eww).
fn ps_leaks(args: &[String]) -> bool {
    use std::collections::HashMap;
    let mut flags: HashMap<char, i32> = HashMap::new();
    let mut bare: HashSet<char> = HashSet::new();
    let mut fmt: Vec<String> = Vec::new();
    let (mut i, n) = (0, args.len());
    let mut has_o = false;
    while i < n {
        let a = &args[i];
        i += 1;
        if a.starts_with("--") {
            if let Some(v) = a.strip_prefix("--format=") {
                fmt.push(v.to_string());
                has_o = true;
            } else if PS_VALUE_LONG.contains(&a.as_str()) && i < n {
                if a == "--format" {
                    fmt.push(args[i].clone());
                    has_o = true;
                }
                i += 1;
            }
        } else if a.starts_with('-') && a.chars().count() > 1 {
            let cs: Vec<char> = a.chars().collect();
            for k in 1..cs.len() {
                let c = cs[k];
                if "oOpCtuUgGsD".contains(c) {
                    let mut val: String = cs[k + 1..].iter().collect();
                    if val.is_empty() && i < n {
                        val = args[i].clone();
                        i += 1;
                    }
                    if c == 'o' || c == 'O' {
                        fmt.push(val);
                        has_o = has_o || c == 'o';
                    }
                    break;
                }
                *flags.entry(c).or_insert(0) += 1;
            }
        } else if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphabetic()) {
            for c in a.chars() {
                *flags.entry(c).or_insert(0) += 1;
                bare.insert(c);
            }
        }
    }
    for val in &fmt {
        let hit = val
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|f| !f.is_empty())
            .any(|f| ARGS_FIELDS.contains(&f.split('=').next().unwrap_or("").to_lowercase().as_str()));
        if hit {
            return true;
        }
    }
    if flags.get(&'w').copied().unwrap_or(0) >= 2
        || flags.contains_key(&'f')
        || bare.contains(&'e')
        || bare.contains(&'u')
    {
        return true;
    }
    if fmt.is_empty() && "eAaxd".chars().any(|c| flags.contains_key(&c)) {
        return true;
    }
    if is_macos() {
        return !has_o && !flags.contains_key(&'c') && !args.iter().any(|a| a == "--help" || a == "--version");
    }
    false
}

/// True for pgrep -fl / -af / -lf / --list-full (they print full command lines).
fn pgrep_leaks(args: &[String]) -> bool {
    let mut flags: HashSet<char> = HashSet::new();
    let (mut i, n) = (0, args.len());
    while i < n {
        let a = &args[i];
        i += 1;
        if a == "--list-full" {
            return true;
        }
        if a.starts_with("--") || !a.starts_with('-') || a.chars().count() < 2 {
            continue;
        }
        let cs: Vec<char> = a.chars().collect();
        for k in 1..cs.len() {
            if "dFgGLPrstuU".contains(cs[k]) {
                if k == cs.len() - 1 {
                    i += 1;
                }
                break;
            }
            flags.insert(cs[k]);
        }
    }
    flags.contains(&'f') && (flags.contains(&'l') || flags.contains(&'a'))
}

fn search_val(fam: &str) -> (&'static str, &'static [&'static str]) {
    match fam {
        "grep" => (
            "efmABCdD",
            &[
                "--regexp",
                "--file",
                "--max-count",
                "--after-context",
                "--before-context",
                "--context",
                "--directories",
                "--devices",
                "--include",
                "--exclude",
                "--exclude-dir",
                "--exclude-from",
                "--label",
                "--binary-files",
                "--group-separator",
            ],
        ),
        "rg" => (
            "efgmABCdjMrtTE",
            &[
                "--regexp",
                "--file",
                "--glob",
                "--iglob",
                "--max-count",
                "--after-context",
                "--before-context",
                "--context",
                "--max-depth",
                "--maxdepth",
                "--threads",
                "--max-columns",
                "--replace",
                "--type",
                "--type-not",
                "--type-add",
                "--type-clear",
                "--ignore-file",
                "--encoding",
                "--colors",
                "--color",
                "--sort",
                "--sortr",
                "--path-separator",
                "--pre",
                "--pre-glob",
                "--engine",
                "--dfa-size-limit",
                "--regex-size-limit",
                "--max-filesize",
                "--context-separator",
                "--field-context-separator",
                "--field-match-separator",
                "--hostname-bin",
                "--hyperlink-format",
            ],
        ),
        "ag" => (
            "ABCGgmpW",
            &[
                "--ignore",
                "--ignore-dir",
                "--depth",
                "--file-search-regex",
                "--path-to-ignore",
                "--pager",
                "--after",
                "--before",
                "--context",
                "--max-count",
                "--width",
                "--workers",
            ],
        ),
        _ => (
            "ABCmgt",
            &[
                "--ignore-dir",
                "--noignore-dir",
                "--ignore-file",
                "--type",
                "--after-context",
                "--before-context",
                "--context",
                "--max-count",
                "--output",
                "--pager",
            ],
        ),
    }
}

#[derive(Default)]
struct SearchOpts {
    pos: Vec<String>,
    pat: bool,
    rec: bool,
    unres: bool,
    globs: Vec<String>,
    inc: Vec<String>,
    xfile: Vec<String>,
    xdir: Vec<String>,
}

/// Fold one short option (val is None for a flag) into the search options.
fn search_short(fam: &str, c: char, val: Option<&str>, o: &mut SearchOpts) {
    if (fam == "grep" || fam == "rg") && (c == 'e' || c == 'f') && val.is_some() {
        o.pat = true;
    } else if fam == "grep" {
        if ((c == 'r' || c == 'R') && val.is_none()) || (c == 'd' && val == Some("recurse")) {
            o.rec = true;
        }
    } else if fam == "rg" || fam == "ag" {
        if let (true, Some(v)) = (c == 'g', val) {
            o.globs.push(v.to_string());
            o.pat = o.pat || fam == "ag"; // ag -g PATTERN replaces the content pattern
        } else if val.is_none() && (c == 'u' || (c == '.' && fam == "rg") || (c == 'U' && fam == "ag")) {
            o.unres = true;
        }
    } else if fam == "ack" && (c == 'g' || c == 'f') {
        o.pat = true;
    }
}

/// Fold one long option (value already split off) into the search options.
fn search_long(fam: &str, key: &str, val: &str, o: &mut SearchOpts) {
    if (fam == "grep" || fam == "rg") && (key == "--regexp" || key == "--file") {
        o.pat = true;
    } else if fam == "grep" {
        if key == "--recursive" || key == "--dereference-recursive" || (key == "--directories" && val == "recurse") {
            o.rec = true;
        } else if key == "--include" {
            o.inc.push(val.to_string());
        } else if key == "--exclude" {
            o.xfile.push(val.to_string());
        } else if key == "--exclude-dir" {
            o.xdir.push(val.to_string());
        }
    } else if fam == "rg" {
        if key == "--files" {
            o.pat = true;
        } else if key == "--glob" || key == "--iglob" {
            o.globs.push(val.to_string());
        } else if key == "--hidden"
            || key == "--unrestricted"
            || (key.starts_with("--no-ignore") && key != "--no-ignore-messages")
        {
            o.unres = true;
        }
    } else if fam == "ag" {
        if key == "--ignore" {
            o.globs.push(format!("!{}", val));
        } else if key == "--ignore-dir" {
            o.xdir.push(val.to_string());
        } else if key == "--unrestricted" || key == "--skip-vcs-ignores" || key == "--hidden" {
            o.unres = true;
        } else if key == "--file-search-regex" {
            o.pat = true;
        }
    } else if fam == "ack" && key == "--ignore-dir" {
        o.xdir.push(val.to_string());
    }
}

/// Parse the arguments of grep, rg, ag or ack: path operands, recursion, unrestricted flags, excludes.
fn search_opts(fam: &str, args: &[String]) -> SearchOpts {
    let (short_val, long_val) = search_val(fam);
    let mut o = SearchOpts { rec: fam == "ack", ..Default::default() };
    let (mut i, n) = (0, args.len());
    while i < n {
        let a = &args[i];
        i += 1;
        if a == "--" {
            o.pos.extend(args[i..].iter().cloned());
            break;
        }
        if a.starts_with("--") {
            let (key, eq, mut val) = match a.find('=') {
                Some(x) => (&a[..x], true, a[x + 1..].to_string()),
                None => (a.as_str(), false, String::new()),
            };
            if !eq && long_val.contains(&key) && i < n {
                val = args[i].clone();
                i += 1;
            }
            search_long(fam, key, &val, &mut o);
        } else if a.starts_with('-') && a.chars().count() > 1 {
            let cs: Vec<char> = a.chars().collect();
            for k in 1..cs.len() {
                let c = cs[k];
                if short_val.contains(c) {
                    let mut val: String = cs[k + 1..].iter().collect();
                    if val.is_empty() && i < n {
                        val = args[i].clone();
                        i += 1;
                    }
                    search_short(fam, c, Some(&val), &mut o);
                    break;
                }
                search_short(fam, c, None, &mut o);
            }
        } else {
            o.pos.push(a.clone());
        }
    }
    o
}

/// True when an ignore-style glob (rg -g '!GLOB') matches the file, one of its folders, or its path.
fn glob_hits(g: &str, dirs: &[String], fname: &str, path: &str) -> bool {
    let g = case_fold(g);
    let g = g.trim_start_matches('/');
    let only_dir = g.ends_with('/');
    let g = g.trim_end_matches('/');
    if g.is_empty() {
        return false;
    }
    let mut names: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
    if !only_dir {
        names.push(fname);
    }
    if names.iter().any(|c| fnmatch(c, g)) {
        return true;
    }
    [g.to_string(), format!("*/{}", g), format!("*/{}/*", g)].iter().any(|p| fnmatch(path, p))
}

/// True when the search options leave `path` out (--exclude, --exclude-dir, --include, rg/ag globs).
fn search_excluded(o: &SearchOpts, path: &str, dirs: &[String], fname: &str) -> bool {
    let path = case_fold(path);
    if o.xfile.iter().any(|g| fnmatch(fname, &case_fold(g)) || fnmatch(&path, &case_fold(g))) {
        return true;
    }
    if o.xdir.iter().any(|g| dirs.iter().any(|d| fnmatch(d, case_fold(g).trim_end_matches('/')))) {
        return true;
    }
    if !o.inc.is_empty() && !o.inc.iter().any(|g| fnmatch(fname, &case_fold(g))) {
        return true;
    }
    let keep: Vec<&String> = o.globs.iter().filter(|g| !g.starts_with('!')).collect();
    if o.globs.iter().filter(|g| g.starts_with('!')).any(|g| glob_hits(&g[1..], dirs, fname, &path)) {
        return true;
    }
    if !keep.is_empty() {
        return !keep.iter().any(|g| {
            let g = case_fold(g);
            fnmatch(fname, &g) || fnmatch(&path, &g) || fnmatch(&path, &format!("*/{}", g))
        });
    }
    false
}

/// True when the search root covers `cand` (the secret file) and no option leaves it out.
fn search_reaches(o: &SearchOpts, root: &str, cand: &str) -> bool {
    let root = normpath(root);
    let cand = case_fold(cand);
    let mut base = root.clone();
    if GLOB_CHARS.is_match(&root) {
        // a glob operand: the part before the first glob word is the base folder
        let mut keep: Vec<&str> = Vec::new();
        for part in root.split('/') {
            if GLOB_CHARS.is_match(part) {
                break;
            }
            keep.push(part);
        }
        base = keep.join("/");
        if base.is_empty() {
            base = "/".to_string();
        }
        let pat = case_fold(&root);
        if !(fnmatch(&cand, &pat) || fnmatch(&cand, &format!("{}/*", pat.trim_end_matches('/')))) {
            return false;
        }
    } else {
        let r = case_fold(&root);
        if !(r == cand || cand.starts_with(&format!("{}/", r.trim_end_matches('/')))) {
            return false;
        }
    }
    let base = case_fold(&base);
    let prefix = format!("{}/", base.trim_end_matches('/'));
    if !cand.starts_with(&prefix) {
        return true;
    }
    let parts: Vec<String> = cand[prefix.len()..].split('/').map(String::from).collect();
    let (fname, dirs) = parts.split_last().unwrap();
    !search_excluded(o, &cand, dirs, fname)
}

/// True for a recursive search (grep -r, ack, and rg/ag with ignore rules off) that reaches the secret file.
fn search_leaks(name: &str, args: &[String], cwd: Option<&str>, target: &Target) -> bool {
    let fam = if matches!(name, "grep" | "egrep" | "fgrep") { "grep" } else { name };
    let o = search_opts(fam, args);
    if (fam == "grep" && !o.rec) || ((fam == "rg" || fam == "ag") && !o.unres) {
        return false; // rg and ag skip the hidden, git-ignored file unless told not to
    }
    let operands: Vec<&String> = if o.pat { o.pos.iter().collect() } else { o.pos.iter().skip(1).collect() };
    let mut paths: Vec<String> = operands.into_iter().filter(|p| p.as_str() != "-").cloned().collect();
    let cwd_s = cwd.map(String::from).unwrap_or_else(|| {
        env::current_dir().map(|c| c.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".to_string())
    });
    if paths.is_empty() {
        paths.push(cwd_s.clone());
    }
    for p in &paths {
        let mut q = expanduser(&expandvars(p));
        if !q.starts_with('/') {
            q = path_join(&cwd_s, &q);
        }
        for root in [normpath(&q), realpath(&q)] {
            if [&target.0, &target.1].iter().any(|cand| search_reaches(&o, &root, cand)) {
                return true;
            }
        }
    }
    false
}

/// The G15 message for a Bash command that reads local.zsh or prints command lines, else None.
fn bash_leaks(cmd: &str, cwd: &str) -> Option<String> {
    let target = secret_target();
    for c in walk(cmd, Some(cwd.to_string()), 0) {
        let name = basename(&c.toks[0]).to_string();
        let args = &c.toks[1..];
        let ccwd = c.cwd.as_deref();
        let head = &args[..args.len().min(200)];
        if name == "procs" {
            return Some(MSG_G15_PROCS.to_string());
        }
        if name == "ps" && ps_leaks(args) {
            return Some(MSG_G15_PS.to_string());
        }
        if name == "pgrep" && pgrep_leaks(args) {
            return Some(MSG_G15_PGREP.to_string());
        }
        for (k, t) in c.toks.iter().enumerate() {
            // input redirect: cmd < file
            if t == "<" && k + 1 < c.toks.len() && is_secret(&c.toks[k + 1], ccwd, &target, false) {
                return Some(MSG_G15_FILE.to_string());
            }
            if t.starts_with('<')
                && !t.starts_with("<<")
                && !t.starts_with("<(")
                && t.chars().count() > 1
                && is_secret(&t[1..], ccwd, &target, false)
            {
                return Some(MSG_G15_FILE.to_string());
            }
        }
        if name == "dd" && head.iter().any(|a| a.starts_with("if=") && is_secret(&a[3..], ccwd, &target, false)) {
            return Some(MSG_G15_FILE.to_string());
        }
        if SECRET_READERS.contains(&name.as_str()) {
            if head.iter().any(|a| is_secret(a, ccwd, &target, false)) {
                return Some(MSG_G15_FILE.to_string());
            }
            if SECRET_SEARCHERS.contains(&name.as_str())
                && (head.iter().any(|a| is_secret(a, ccwd, &target, true)) || search_leaks(&name, head, ccwd, &target))
            {
                return Some(msg_g15_search(&name));
            }
            if c.extra.iter().take(200).any(|a| is_secret(a, ccwd, &target, true)) {
                return Some(MSG_G15_FILE.to_string());
            }
        }
    }
    None
}

/// G15 with the process cwd for relative paths. `tool` is `Read`, `Grep` or `Bash`; `input` is the
/// tool input.
pub fn g15_secrets(tool: &str, input: &Value) -> Verdict {
    let cwd = env::current_dir().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    g15_secrets_at(tool, input, &cwd)
}

/// G15: deny reading local.zsh (Read, Grep or Bash) and printing full process command lines. The
/// message names the safe command for the matched case (`pgrep -f`, `ps -o ... -p`, the exclude flag).
pub fn g15_secrets_at(tool: &str, input: &Value, cwd: &str) -> Verdict {
    if !matches!(tool, "Read" | "Grep" | "Bash") {
        return Verdict::Allow;
    }
    let r = std::panic::catch_unwind(|| {
        if tool == "Bash" {
            return command_of(input).and_then(|cmd| bash_leaks(cmd, cwd));
        }
        let key = if tool == "Read" { "file_path" } else { "path" };
        match input.get(key).and_then(Value::as_str) {
            Some(p) if !p.is_empty() && is_secret(p, Some(cwd), &secret_target(), tool == "Grep") => {
                Some(MSG_G15_FILE.to_string())
            }
            _ => None,
        }
    });
    deny_or_allow(r.ok().flatten().as_deref())
}

// --- G16 outbound sends --------------------------------------------------------

/// A body file is read only when it is a regular file under this size; a bigger one asks.
const MAX_BODY: u64 = 1 << 20;

/// The MCP tools the PreToolUse matcher routes here. `claude/settings.json` names the same words
/// with explicit `[Ss]` case alternatives (a test compares the two lists).
static MCP_SEND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^mcp__.*(send|post|reply|forward|comment|create_?issue|update_?issue|create_?pull_?request|update_?pull_?request|push_?files|create_?or_?update_?file|publish|upload|submit_?review|draft|message|tweet|notify)",
    )
    .unwrap()
});

static SECRET_SHAPES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"\bsk-ant-[A-Za-z0-9_-]{10,}", "Anthropic API key"),
        (r"\bsk-[A-Za-z0-9_-]{16,}", "API key"),
        (r"\bgh[pousr]_[A-Za-z0-9]{20,}", "GitHub token"),
        (r"\bgithub_pat_[A-Za-z0-9_]{20,}", "GitHub token"),
        (r"\bxox[abpr]-[A-Za-z0-9-]{8,}", "Slack token"),
        (r"\bAKIA[0-9A-Z]{16}\b", "AWS access key id"),
        (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "private key"),
    ]
    .into_iter()
    .map(|(re, kind)| (Regex::new(re).unwrap(), kind))
    .collect()
});

/// `$(`, a backtick or `$name` / `${`: text the shell would still change. The heredoc idiom
/// `$(cat <<'EOF' ...)` is not opaque: the heredoc is scanned on its own.
static EXPANSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$[A-Za-z_{(]|`").unwrap());

fn is_opaque(text: &str) -> bool {
    EXPANSION.is_match(&text.replace("$(cat <<", ""))
}

/// True when a command line holds a `$(...)` or backtick substitution outside single quotes, other
/// than the heredoc idiom: a body built by it cannot be read before it runs. (A substitution splits
/// the command into pieces, so the token check alone misses `--body $(cat f)`.)
fn has_substitution(cmd: &str) -> bool {
    let text = strip_heredocs(cmd).replace("$(cat <<", "");
    let (mut single, mut double) = (false, false);
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' && !single {
            i += 2;
            continue;
        }
        if c == '\'' && !double {
            single = !single;
        } else if c == '"' && !single {
            double = !double;
        } else if !single && (c == '`' || (c == '$' && cs.get(i + 1) == Some(&'('))) {
            return true;
        }
        i += 1;
    }
    false
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A word of 4 letters or fewer matches whole words only; a longer one matches anywhere.
/// Both sides are lower case.
pub fn word_hit(text: &str, word: &str) -> bool {
    !word_spans(text, word).is_empty()
}

/// The byte spans of `word` in `text` that count as a hit by the rule of `word_hit`. Both sides
/// are lower case.
pub fn word_spans(text: &str, word: &str) -> Vec<(usize, usize)> {
    if word.is_empty() {
        return Vec::new();
    }
    let long = word.chars().count() > 4;
    text.match_indices(word)
        .filter(|(i, _)| {
            long || {
                let before = text[..*i].chars().next_back();
                let after = text[*i + word.len()..].chars().next();
                !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
            }
        })
        .map(|(i, _)| (i, i + word.len()))
        .collect()
}

fn add_kind(kinds: &mut Vec<String>, kind: String) {
    if !kinds.contains(&kind) {
        kinds.push(kind);
    }
}

/// Note in `kinds` what kind of thing `text` holds; the match itself is never kept.
fn scan_text(text: &str, words: &[String], kinds: &mut Vec<String>) {
    if text.is_empty() {
        return;
    }
    let lower = text.to_lowercase();
    if words.iter().any(|w| word_hit(&lower, w)) {
        add_kind(kinds, "a private word".to_string());
    }
    for (re, kind) in SECRET_SHAPES.iter() {
        if *kind == "API key" && kinds.iter().any(|k| k.contains("Anthropic")) {
            continue;
        }
        if re.is_match(text) {
            add_kind(kinds, format!("a secret shape ({kind})"));
        }
    }
}

/// Every string in a JSON value, keys excluded.
fn strings_of<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(a) => a.iter().for_each(|x| strings_of(x, out)),
        Value::Object(o) => o.values().for_each(|x| strings_of(x, out)),
        _ => {}
    }
}

/// Read a body file: a regular file under 1 MB, relative paths against `cwd`.
fn read_body_file(path: &str, cwd: &Path) -> Result<String, &'static str> {
    let p = expanduser(&expandvars(path));
    if p.is_empty() || p.contains('$') || p.contains('`') {
        return Err("a body file it cannot read");
    }
    let full = if Path::new(&p).is_absolute() { PathBuf::from(&p) } else { cwd.join(&p) };
    let meta = fs::metadata(&full).map_err(|_| "a body file it cannot read")?;
    if !meta.is_file() || meta.len() >= MAX_BODY {
        return Err("a body file it cannot read");
    }
    let bytes = fs::read(&full).map_err(|_| "a body file it cannot read")?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// What one outbound command sends: inline text, files to read, and whether the body is stdin.
#[derive(Default)]
struct Payload {
    texts: Vec<String>,
    files: Vec<String>,
    stdin: bool,
    /// URL operands of curl and wget: a secret shape in one asks (GET too); a private word asks
    /// only in the query part.
    urls: Vec<String>,
    /// The call carries a body (or a config file that can name one), not only a URL.
    body: bool,
}

impl Payload {
    /// A file operand: `-` is stdin.
    fn file(&mut self, path: &str) {
        if path == "-" {
            self.stdin = true;
        } else if !path.is_empty() {
            self.files.push(path.to_string());
        }
    }
}

struct Opts {
    opts: Vec<(String, String)>,
    pos: Vec<String>,
}

/// curl and wget accept a unique prefix of a long option.
fn long_matches(key: &str, full: &str) -> bool {
    key == full || (key.len() >= 5 && full.starts_with(key))
}

/// Split arguments into options (`-d`, `--data`, with their value) and operands. `short_val` lists
/// the short flags that take a value (`-d@file` and `-d file` both work, `-sd x` clusters too);
/// `long_val` the long ones (`--k=v` always carries its value).
fn parse_opts(args: &[String], short_val: &str, long_val: &[&str], abbr: bool) -> Opts {
    let (mut opts, mut pos) = (Vec::new(), Vec::new());
    let (mut i, n) = (0, args.len());
    while i < n {
        let a = &args[i];
        i += 1;
        if a == "--" {
            pos.extend(args[i..].iter().cloned());
            break;
        }
        if let Some(long) = a.strip_prefix("--") {
            let (key, inline) = match long.split_once('=') {
                Some((k, v)) => (format!("--{k}"), Some(v.to_string())),
                None => (a.clone(), None),
            };
            let takes = long_val.iter().any(|f| if abbr { long_matches(&key, f) } else { key == *f });
            let val = match inline {
                Some(v) => v,
                None if takes && i < n => {
                    i += 1;
                    args[i - 1].clone()
                }
                None => String::new(),
            };
            opts.push((key, val));
        } else if a.starts_with('-') && a.chars().count() > 1 {
            let cs: Vec<char> = a.chars().collect();
            for k in 1..cs.len() {
                if short_val.contains(cs[k]) {
                    let mut val: String = cs[k + 1..].iter().collect();
                    if val.is_empty() && i < n {
                        val = args[i].clone();
                        i += 1;
                    }
                    opts.push((format!("-{}", cs[k]), val));
                    break;
                }
                opts.push((format!("-{}", cs[k]), String::new()));
            }
        } else {
            pos.push(a.clone());
        }
    }
    Opts { opts, pos }
}

const CURL_SHORT_VAL: &str = "AbcCdeEFHKmoPQrtTuUwxXyYz";
const CURL_LONG_VAL: &[&str] = &[
    "--data",
    "--data-raw",
    "--data-binary",
    "--data-ascii",
    "--data-urlencode",
    "--json",
    "--form",
    "--form-string",
    "--upload-file",
    "--header",
    "--user",
    "--user-agent",
    "--output",
    "--request",
    "--url",
    "--proxy",
    "--cookie",
    "--cookie-jar",
    "--referer",
    "--cacert",
    "--cert",
    "--key",
    "--max-time",
    "--connect-timeout",
    "--retry",
    "--write-out",
    "--config",
    "--range",
    "--resolve",
    "--url-query",
    "--oauth2-bearer",
    "--proxy-user",
    "--time-cond",
    "--variable",
    "--expand-data",
];

#[derive(Clone, Copy, PartialEq)]
enum CurlBody {
    Data,
    Urlencode,
    Form,
    FormString,
    Upload,
}

fn curl_body(key: &str) -> Option<CurlBody> {
    const LONG: &[(&str, CurlBody)] = &[
        ("--data-urlencode", CurlBody::Urlencode),
        ("--data-raw", CurlBody::Data),
        ("--data-binary", CurlBody::Data),
        ("--data-ascii", CurlBody::Data),
        ("--data", CurlBody::Data),
        ("--json", CurlBody::Data),
        ("--form-string", CurlBody::FormString),
        ("--form", CurlBody::Form),
        ("--upload-file", CurlBody::Upload),
    ];
    match key {
        "-d" => Some(CurlBody::Data),
        "-F" => Some(CurlBody::Form),
        "-T" => Some(CurlBody::Upload),
        _ if key.starts_with("--") => LONG
            .iter()
            .find(|(f, _)| *f == key)
            .or_else(|| LONG.iter().find(|(f, _)| long_matches(key, f)))
            .map(|(_, k)| *k),
        _ => None,
    }
}

fn curl_payload(args: &[String]) -> Option<Payload> {
    let o = parse_opts(args, CURL_SHORT_VAL, CURL_LONG_VAL, true);
    let mut p = Payload::default();
    for (k, v) in &o.opts {
        // a config file can hold `data = ...` and `url = ...` lines: read it like a body file
        if k == "-K" || (k.starts_with("--") && long_matches(k, "--config")) {
            p.body = true;
            p.file(v);
            continue;
        }
        if k == "--url" {
            p.urls.push(v.clone());
            continue;
        }
        if k.starts_with("--") && long_matches(k, "--url-query") {
            p.texts.push(v.clone());
            continue;
        }
        let Some(kind) = curl_body(k) else { continue };
        p.body = true;
        match kind {
            CurlBody::Data => match v.strip_prefix('@') {
                Some(f) => p.file(f),
                None => p.texts.push(v.clone()),
            },
            CurlBody::Urlencode => match (v.contains('='), v.find('@')) {
                (false, Some(i)) => {
                    p.texts.push(v[..i].to_string());
                    p.file(&v[i + 1..]);
                }
                _ => p.texts.push(v.clone()),
            },
            CurlBody::Form => match v.find("=@").or_else(|| v.find("=<")) {
                Some(i) => {
                    p.texts.push(v[..i].to_string());
                    p.file(v[i + 2..].split(';').next().unwrap_or(""));
                }
                None => p.texts.push(v.clone()),
            },
            CurlBody::FormString => p.texts.push(v.clone()),
            CurlBody::Upload => p.file(v),
        }
    }
    p.urls.extend(o.pos.iter().cloned());
    (p.body || !p.urls.is_empty() || !p.texts.is_empty()).then_some(p)
}

const WGET_SHORT_VAL: &str = "OoaetTwPUiBQlIARXD";
const WGET_LONG_VAL: &[&str] = &[
    "--post-data",
    "--post-file",
    "--body-data",
    "--body-file",
    "--header",
    "--user-agent",
    "--output-document",
    "--output-file",
    "--method",
    "--referer",
    "--load-cookies",
    "--save-cookies",
    "--directory-prefix",
    "--http-user",
    "--http-password",
    "--user",
    "--password",
];

fn wget_payload(args: &[String]) -> Option<Payload> {
    let o = parse_opts(args, WGET_SHORT_VAL, WGET_LONG_VAL, true);
    let mut p = Payload::default();
    for (k, v) in &o.opts {
        if !k.starts_with("--") {
            continue;
        }
        let data = ["--post-data", "--body-data"].iter().any(|f| long_matches(k, f));
        let file = ["--post-file", "--body-file"].iter().any(|f| long_matches(k, f));
        p.body = p.body || data || file;
        if data {
            p.texts.push(v.clone());
        }
        if file {
            p.file(v);
        }
    }
    p.urls.extend(o.pos.iter().cloned());
    (p.body || !p.urls.is_empty()).then_some(p)
}

/// `gh api`: a write method, any `-f`/`-F` field, or `--input`.
fn gh_api_payload(args: &[String]) -> Option<Payload> {
    let o = parse_opts(
        args,
        "XfFHqtp",
        &[
            "--method",
            "--raw-field",
            "--field",
            "--header",
            "--input",
            "--jq",
            "--template",
            "--hostname",
            "--cache",
            "--preview",
        ],
        false,
    );
    let mut p = Payload::default();
    let mut sends = false;
    for (k, v) in &o.opts {
        match k.as_str() {
            "-X" | "--method" => sends = sends || matches!(v.to_uppercase().as_str(), "POST" | "PUT" | "PATCH"),
            "-f" | "-F" | "--field" | "--raw-field" => {
                sends = true;
                match v.split_once('=').and_then(|(_, rhs)| rhs.strip_prefix('@')) {
                    Some(f) => {
                        p.texts.push(v.split_once('=').map(|(name, _)| name).unwrap_or("").to_string());
                        p.file(f);
                    }
                    None => p.texts.push(v.clone()),
                }
            }
            "--input" => {
                sends = true;
                p.file(v);
            }
            _ => {}
        }
    }
    p.body = sends;
    if sends {
        p.urls.extend(o.pos.iter().cloned()); // `gh api 'repos/o/r/issues?body=..' -X POST`
    }
    sends.then_some(p)
}

const GH_VERBS: &[(&str, &[&str])] = &[
    ("pr", &["create", "comment", "review", "edit", "merge", "close", "reopen"]),
    ("issue", &["create", "comment", "edit", "close", "reopen"]),
    ("release", &["create", "edit", "upload"]),
    ("gist", &["create", "edit"]),
    ("repo", &["create", "edit"]),
    ("workflow", &["run"]),
    ("discussion", &["create", "comment", "edit"]),
    ("label", &["create", "edit"]),
];

/// `gh pr|issue|release|gist|repo|workflow|label ...`: every value and operand is text;
/// `--body-file`, `--notes-file` and `-F` name a file (a `-F` field of `gh workflow run` is
/// `key=value`, `key=@file` reads one); the files of `gh gist` and `gh release upload` are operands.
fn gh_text_payload(group: &str, verb: &str, args: &[String]) -> Payload {
    // flags that take a value differ by group: `-d` is a flag of `gh pr create` but a value of
    // `gh repo create` and `gh label create`
    let short = match group {
        "repo" => "aBbFHlmnpRrTtdh",
        "label" => "aBbFHlmnpRrTtdc",
        "workflow" => "aBbFHlmnpRrTtf",
        _ => "aBbFHlmnpRrTt",
    };
    let o = parse_opts(
        args,
        short,
        &[
            "--assignee",
            "--base",
            "--body",
            "--body-file",
            "--head",
            "--label",
            "--milestone",
            "--project",
            "--repo",
            "--reviewer",
            "--template",
            "--title",
            "--notes",
            "--notes-file",
            "--comment",
            "--reason",
            "--target",
            "--discussion-category",
            "--add-assignee",
            "--add-label",
            "--remove-label",
            "--add-reviewer",
            "--add-project",
            "--subject",
            "--match-head-commit",
            "--description",
            "--homepage",
            "--color",
            "--field",
            "--raw-field",
            "--ref",
            "--category",
        ],
        false,
    );
    let mut p = Payload::default();
    for (k, v) in &o.opts {
        if group == "workflow" && matches!(k.as_str(), "-F" | "--field") {
            match v.split_once("=@") {
                Some((name, f)) => {
                    p.texts.push(name.to_string());
                    p.file(f);
                }
                None => p.texts.push(v.clone()),
            }
        } else if k == "--json" && group == "workflow" {
            p.stdin = true; // the inputs come from stdin
        } else if matches!(k.as_str(), "-F" | "--body-file" | "--notes-file") {
            p.file(v);
        } else if matches!(k.as_str(), "-R" | "--repo" | "--head" | "--base" | "-B") || (group == "pr" && k == "-H") {
            // a repo or branch name is an address, not body text
        } else if !v.is_empty() {
            p.texts.push(v.clone());
        }
    }
    for (n, a) in o.pos.iter().enumerate() {
        if group == "gist" {
            p.file(a);
        } else if group == "release" && verb == "upload" && n > 0 {
            p.file(a.split('#').next().unwrap_or(a)); // `file#label`
        } else {
            p.texts.push(a.clone());
        }
    }
    p
}

/// `gh <group> <verb> rest...` as (group, verb, rest), global flags skipped; `gh api rest...` as
/// ("api", "", rest).
fn gh_split(args: &[String]) -> Option<(&str, &str, &[String])> {
    let mut i = 0;
    let mut group: Option<usize> = None;
    while i < args.len() {
        let a = &args[i];
        if matches!(a.as_str(), "-R" | "--repo" | "--hostname") {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        match group {
            None => {
                if a == "api" {
                    return Some(("api", "", &args[i + 1..]));
                }
                group = Some(i);
            }
            Some(g) => return Some((&args[g], a, &args[i + 1..])),
        }
        i += 1;
    }
    None
}

/// The payload of `gh ...` when it is a call that sends text.
fn gh_payload(args: &[String]) -> Option<Payload> {
    let (group, verb, rest) = gh_split(args)?;
    if group == "api" {
        return gh_api_payload(rest);
    }
    let verbs = GH_VERBS.iter().find(|(name, _)| *name == group)?.1;
    verbs.contains(&verb).then(|| Payload { body: true, ..gh_text_payload(group, verb, rest) })
}

/// The payload of one command when it sends text off the machine (gh, curl, wget).
fn payload_of(c: &Cmd) -> Option<Payload> {
    let name = basename(&c.toks[0]);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    let args = &c.toks[1..];
    match name {
        "gh" => gh_payload(args),
        "curl" => curl_payload(args),
        "wget" => wget_payload(args),
        _ => None,
    }
}

fn cwd_string(cwd: &Path) -> String {
    cwd.to_string_lossy().into_owned()
}

/// True when this call can send text off the machine: a send-like MCP tool or a Bash command with
/// a gh, curl or wget call that carries a body. Cheap: the guard asks before it loads the words.
pub fn g16_applies(tool: &str, input: &Value) -> bool {
    if tool.starts_with("mcp__") {
        return MCP_SEND.is_match(tool);
    }
    let Some(cmd) = (tool == "Bash").then(|| command_of(input)).flatten() else { return false };
    // no raw-text prefilter: `g''h` and `"curl"` are the same words once the shell has run
    let mut bad = Vec::new();
    let cmds = walk_full(cmd, None, 0, &mut bad);
    bad.iter().any(|raw| names_outbound_tool(raw, OUTBOUND_TOOLS)) || cmds.iter().any(|c| payload_of(c).is_some())
}

const OUTBOUND_TOOLS: &[&str] = &["gh", "curl", "wget"];

/// A URL operand: a secret shape anywhere in it, a private word only in its query (the path of a
/// private repo is a read, not a leak of the word).
fn scan_url(url: &str, words: &[String], kinds: &mut Vec<String>) {
    scan_text(url, &[], kinds);
    if let Some((_, query)) = url.split_once('?') {
        scan_text(query, words, kinds);
    }
}

/// G16: ask before text that holds a private word or a secret shape leaves the machine (`gh pr|issue`
/// writes, `gh api` writes, `curl`/`wget` with a body, MCP send tools). A body that cannot be read
/// (stdin, a shell expansion, a missing, big or odd file) asks too. `words` are the private words.
pub fn g16_outbound(tool: &str, input: &Value, cwd: &Path, words: &[String]) -> Verdict {
    let words: Vec<String> = words.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
    let mut kinds: Vec<String> = Vec::new();
    if tool.starts_with("mcp__") {
        if MCP_SEND.is_match(tool) {
            let mut all = Vec::new();
            strings_of(input, &mut all);
            all.iter().for_each(|s| scan_text(s, &words, &mut kinds));
        }
    } else if tool == "Bash"
        && let Some(cmd) = command_of(input)
    {
        let base = cwd_string(cwd);
        let mut sends = false;
        let mut bad = Vec::new();
        let cmds = walk_full(cmd, Some(base.clone()), 0, &mut bad);
        if bad.iter().any(|raw| names_outbound_tool(raw, OUTBOUND_TOOLS)) {
            add_kind(&mut kinds, "a command it cannot parse".to_string());
        }
        for c in cmds {
            let Some(mut p) = payload_of(&c) else { continue };
            sends = sends || p.body;
            let here = PathBuf::from(c.cwd.as_deref().unwrap_or(&base));
            p.urls.iter().for_each(|u| scan_url(u, &words, &mut kinds));
            if p.stdin {
                // the body comes from stdin: a redirect, a here-string or a heredoc that starts
                // on this command's own line (not one that belongs to another command)
                let mut known = c.heredoc;
                for (k, t) in c.toks.iter().enumerate() {
                    let next = c.toks.get(k + 1).cloned().unwrap_or_default();
                    if t == "<<<" {
                        p.texts.push(next);
                        known = true;
                    } else if let Some(r) = t.strip_prefix("<<<") {
                        p.texts.push(r.to_string());
                        known = true;
                    } else if t.starts_with("<<") {
                        known = true;
                    } else if t == "<" {
                        p.file(&next);
                        known = true;
                    } else if t.starts_with('<') && t.chars().count() > 1 && !t.starts_with("<(") {
                        p.file(&t[1..]);
                        known = true;
                    }
                }
                if !known {
                    add_kind(&mut kinds, "a body from stdin it cannot read".to_string());
                }
            }
            for t in &p.texts {
                if is_opaque(t) {
                    add_kind(&mut kinds, "a body it cannot read (a shell expansion)".to_string());
                } else {
                    scan_text(t, &words, &mut kinds);
                }
            }
            for f in &p.files {
                match read_body_file(f, &here) {
                    Ok(text) => scan_text(&text, &words, &mut kinds),
                    Err(why) => add_kind(&mut kinds, why.to_string()),
                }
            }
        }
        if sends {
            heredoc_bodies(cmd).iter().for_each(|b| scan_text(b, &words, &mut kinds));
            if has_substitution(cmd) {
                add_kind(&mut kinds, "a body it cannot read (a shell expansion)".to_string());
            }
        }
    }
    if kinds.is_empty() {
        return Verdict::Allow;
    }
    Verdict::Ask(format!(
        "G16: this sends text off the machine. Found: {}. Check it, then approve or edit it.",
        kinds.join("; ")
    ))
}

// --- G17 attribution -------------------------------------------------------------

static ATTRIBUTION: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"(?i)co-authored-by\s*[:=]\s*claude", "a co-author line for the assistant"),
        (r"(?i)noreply@anthropic\.com", "the vendor's no-reply mail address"),
        (r"(?i)generated\s+with\s+\[?claude", "a generated-with line"),
    ]
    .into_iter()
    .map(|(re, kind)| (Regex::new(re).unwrap(), kind))
    .collect()
});

const G17_GIT_VERBS: &[&str] = &["commit", "tag", "merge", "revert", "notes"];
const G17_GH_GROUPS: &[&str] = &["pr", "issue", "release"];

fn scan_attribution(text: &str, kinds: &mut Vec<String>) {
    for (re, kind) in ATTRIBUTION.iter() {
        if re.is_match(text) {
            add_kind(kinds, kind.to_string());
        }
    }
}

/// The message words and files of a `git commit|tag|merge|revert|notes` call, and the folder it
/// runs in (`-C <path>` moves it). None for any other git call.
fn git_message(c: &Cmd, base: &str) -> Option<(Payload, PathBuf)> {
    let args = &c.toks[1..];
    let mut dir = PathBuf::from(c.cwd.as_deref().unwrap_or(base));
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-C" => {
                if let Some(p) = args.get(i + 1) {
                    dir = dir.join(p);
                }
                i += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env" | "--super-prefix" => i += 2,
            a if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let verb = args.get(i)?;
    if !G17_GIT_VERBS.contains(&verb.as_str()) {
        return None;
    }
    let rest = &args[i + 1..];
    let o = parse_opts(
        rest,
        "mFCct",
        &[
            "--message",
            "--file",
            "--trailer",
            "--author",
            "--date",
            "--template",
            "--reuse-message",
            "--reedit-message",
            "--fixup",
            "--squash",
            "--cleanup",
            "--strategy",
            "--strategy-option",
        ],
        false,
    );
    let mut p = Payload { texts: rest.to_vec(), ..Default::default() };
    for (k, v) in &o.opts {
        if k == "-F" || k == "--file" {
            p.file(v);
        }
    }
    Some((p, dir))
}

/// G17: deny a `git commit|tag|merge|revert|notes` or `gh pr|issue|release` command whose text holds
/// an attribution line for the assistant: a co-author line, the vendor noreply address or a
/// "generated with" line. Reads a message file given by `-F`, `--file` or `--body-file` (relative to
/// `cwd`). A message that comes from stdin, or from a file the command has not made yet, is judged
/// by the whole command text.
pub fn g17_attribution(cmd: &str, cwd: &Path) -> Verdict {
    let base = cwd_string(cwd);
    let mut kinds: Vec<String> = Vec::new();
    let (mut whole, mut targeted) = (false, false);
    let mut bad = Vec::new();
    let cmds = walk_full(cmd, Some(base.clone()), 0, &mut bad);
    // a piece that cannot be split into words but names git or gh: judge the whole command text
    if bad.iter().any(|raw| names_outbound_tool(raw, &["git", "gh"])) {
        whole = true;
        targeted = true;
    }
    for c in cmds {
        let name = basename(&c.toks[0]);
        let name = name.strip_suffix(".exe").unwrap_or(name);
        let found = match name {
            "git" => git_message(&c, &base),
            "gh" => gh_split(&c.toks[1..]).filter(|(group, _, _)| G17_GH_GROUPS.contains(group)).map(
                |(group, verb, rest)| {
                    (gh_text_payload(group, verb, rest), PathBuf::from(c.cwd.as_deref().unwrap_or(&base)))
                },
            ),
            _ => None,
        };
        let Some((p, dir)) = found else { continue };
        targeted = true;
        p.texts.iter().for_each(|t| scan_attribution(t, &mut kinds));
        whole = whole || p.stdin;
        for f in &p.files {
            match read_body_file(f, &dir) {
                Ok(text) => scan_attribution(&text, &mut kinds),
                Err(_) => whole = true,
            }
        }
    }
    if targeted {
        heredoc_bodies(cmd).iter().for_each(|b| scan_attribution(b, &mut kinds));
        if whole {
            scan_attribution(cmd, &mut kinds);
        }
    }
    if kinds.is_empty() {
        return Verdict::Allow;
    }
    Verdict::Deny(format!(
        "G17: no attribution lines for the assistant in commits, tags or PR text. Found: {}. Remove them and run it again.",
        kinds.join("; ")
    ))
}

// --- private words, shadow mode ------------------------------------------------------

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from).or_else(dirs::home_dir)
}

/// The output of a git command run with HOME set to `home` (in `dir` when given), or "".
fn git_out(home: &Path, dir: Option<&Path>, args: &[&str]) -> String {
    let mut c = Command::new("git");
    c.args(args).env("HOME", home).stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    c.output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// The words the repo must not leak, lower case and without repeats: `PRIVATE_PROJECTS` and
/// `PRIVATE_WORDS` from the settings, and the git email, the full git name and that name as one
/// word (from the settings and from `git config`). A word of the identity that is part of the origin
/// URL (the GitHub handle in the clone URL) is left out, as `scripts/git-hooks/private-words.sh` does.
pub fn private_words(home: &Path) -> Vec<String> {
    private_words_at(home, None)
}

/// [`private_words`] with git run in `dir` (the hook's cwd), so a repo's own identity counts.
pub fn private_words_at(home: &Path, dir: Option<&Path>) -> Vec<String> {
    fn push(out: &mut Vec<String>, w: &str) {
        let w = w.trim().to_lowercase();
        if !w.is_empty() && !out.contains(&w) {
            out.push(w);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for name in ["PRIVATE_PROJECTS", "PRIVATE_WORDS"] {
        for w in crate::setting(home, name).split(|c: char| c.is_whitespace() || c == ',') {
            push(&mut out, w);
        }
    }
    let url = git_out(home, dir, &["remote", "get-url", "origin"]).to_lowercase();
    let idents = [
        crate::setting(home, "GIT_EMAIL"),
        crate::setting(home, "GIT_NAME"),
        git_out(home, dir, &["config", "user.email"]),
        git_out(home, dir, &["config", "user.name"]),
    ];
    for raw in idents {
        let name = raw.trim().to_lowercase();
        let joined: String = name.split_whitespace().collect();
        for w in [name.clone(), joined] {
            if w.chars().count() >= 3 && (url.is_empty() || !url.contains(&w)) {
                push(&mut out, &w);
            }
        }
    }
    out
}

/// True when check `id` runs in shadow mode: it is in env `GUARD_SHADOW` (even an empty one wins)
/// or else in the `GUARD_SHADOW` setting; a comma or space list.
pub fn shadow(id: &str) -> bool {
    let raw = match env::var("GUARD_SHADOW") {
        Ok(v) => v,
        Err(_) => home_dir().map(|h| crate::setting(&h, "GUARD_SHADOW")).unwrap_or_default(),
    };
    let id = id.to_uppercase();
    raw.to_uppercase().split(|c: char| c.is_whitespace() || c == ',').any(|p| p == id)
}

/// Append one line to `${XDG_CACHE_HOME:-~/.cache}/dots-guard-shadow.log`: time, check id, what the
/// check would have done, and a short summary. Never the matched text: callers pass the message.
pub fn shadow_log(id: &str, v: &Verdict, summary: &str) {
    let dir = env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".cache")));
    let Some(dir) = dir else { return };
    let _ = fs::create_dir_all(&dir);
    let what = match v {
        Verdict::Allow => "allow",
        Verdict::Ask(_) => "ask",
        Verdict::Deny(_) => "deny",
    };
    let one_line: String = summary.chars().map(|c| if c.is_control() { ' ' } else { c }).take(200).collect();
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(dir.join("dots-guard-shadow.log")) {
        let _ = writeln!(f, "{ts} {id} {what} {one_line}");
    }
}

// --- the gate: which checks run ------------------------------------------------------

/// Every check: its id and the preset that owns it (`core` is always on). The ids match the `guard:`
/// lists in `claude/presets/*/rules.md`; a test compares the two. The Claude guard, the Cursor adapter
/// and the Opencode plugin (through the guard) all read this one list.
pub const REGISTRY: &[(&str, &str)] = &[
    ("G1", "workflow"),
    ("G2", "workflow"),
    ("G3", "workflow"),
    ("G4", "workflow"),
    ("G5", "workflow"),
    ("G6", "workflow"),
    ("G7", "workflow"),
    ("G12", "workflow"),
    ("G13", "workflow"),
    ("H1", "workflow"),
    ("W5", "workflow"),
    ("G8", "pace"),
    ("PACE", "pace"),
    ("G9", "models"),
    ("ROUTER", "models"),
    ("G10", "ship-to-main"),
    ("G11", "clock"),
    ("G17", "no-ai-attribution"),
    ("G14", "core"),
    ("G15", "core"),
    ("G16", "core"),
];

/// What the gate reads from the settings: the checks switched off and the presets that are on.
pub struct Gate {
    pub off: Vec<String>,
    pub presets: Vec<String>,
}

impl Gate {
    /// Every preset on, nothing off: the answer when the settings cannot be read.
    pub fn all_on() -> Gate {
        Gate { off: Vec::new(), presets: crate::presets::ALL.iter().map(|p| p.to_string()).collect() }
    }

    /// Read `GUARD_OFF` (env, even an empty one, else the setting; a comma or space list) and the
    /// enabled presets of the profile from `home`.
    pub fn load(home: &Path) -> Gate {
        std::panic::catch_unwind(|| {
            let raw = env::var("GUARD_OFF").unwrap_or_else(|_| crate::setting(home, "GUARD_OFF"));
            let off = raw
                .to_uppercase()
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect();
            let profile = crate::presets::profile(home);
            Gate { off, presets: crate::presets::enabled_quiet(home, &profile) }
        })
        .unwrap_or_else(|_| Gate::all_on())
    }

    /// True when check `id` runs: it is not in `GUARD_OFF` and its preset is on. `core` checks
    /// ignore the presets.
    pub fn enabled(&self, id: &str) -> bool {
        let id = id.to_uppercase();
        if self.off.contains(&id) {
            return false;
        }
        match REGISTRY.iter().find(|(i, _)| *i == id) {
            Some((_, preset)) if *preset != "core" => self.presets.iter().any(|p| p == preset),
            _ => true,
        }
    }
}

/// [`Gate::enabled`] for this process, with the gate read once from the user's home.
pub fn check_enabled(id: &str) -> bool {
    static GATE: std::sync::OnceLock<Gate> = std::sync::OnceLock::new();
    GATE.get_or_init(|| home_dir().map(|h| Gate::load(&h)).unwrap_or_else(Gate::all_on)).enabled(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shlex_matches_python() {
        let split = |s: &str| shlex_split(s);
        assert_eq!(split("a 'b c' \"d e\" f\\ g"), Some(vec!["a".into(), "b c".into(), "d e".into(), "f g".into()]));
        assert_eq!(split("echo \"a\\\"b\" 'c\\d'"), Some(vec!["echo".into(), "a\"b".into(), "c\\d".into()]));
        assert_eq!(split("x '' y"), Some(vec!["x".into(), "".into(), "y".into()]));
        assert_eq!(split("unbalanced 'quote"), None);
        assert_eq!(split("trailing \\"), None);
        assert_eq!(split("   "), Some(vec![]));
    }

    #[test]
    fn fnmatch_cases() {
        assert!(fnmatch("/a/b/local.zsh", "*/local.zsh"));
        assert!(fnmatch("local.zsh", "local.z?h"));
        assert!(fnmatch("a1", "a[0-9]"));
        assert!(!fnmatch("a1", "a[!0-9]"));
        assert!(!fnmatch("abc", "ab"));
    }

    #[test]
    fn normpath_cases() {
        assert_eq!(normpath("/a//b/./c/../d"), "/a/b/d");
        assert_eq!(normpath(""), ".");
        assert_eq!(normpath("../x"), "../x");
        assert_eq!(normpath("/.."), "/");
    }

    #[test]
    fn heredoc_delims_match_python() {
        assert_eq!(heredoc_delims("cat <<EOF"), vec!["EOF"]);
        assert_eq!(heredoc_delims("cat <<-'EOF' > x"), vec!["EOF"]);
        assert!(heredoc_delims("cat <<< word").is_empty());
        assert_eq!(strip_heredocs("cat <<E\nsed -i x\nE\nls"), "cat <<E\nls");
    }

    #[test]
    fn heredoc_bodies_are_collected() {
        assert_eq!(heredoc_bodies("cat <<E\nsed -i x\nE\nls"), vec!["sed -i x\n"]);
        assert_eq!(heredoc_bodies("a <<A; b <<B\none\nA\ntwo\nB"), vec!["one\n", "two\n"]);
        assert_eq!(heredoc_bodies("cat <<E\nopen"), vec!["open\n"]);
        assert!(heredoc_bodies("echo hi").is_empty());
    }

    #[test]
    fn opaque_text() {
        assert!(is_opaque("$(cat f)"));
        assert!(is_opaque("`cat f`"));
        assert!(is_opaque("see $MSG"));
        assert!(is_opaque("${MSG}"));
        assert!(!is_opaque("costs $5"));
        assert!(!is_opaque("plain"));
        assert!(!is_opaque("$(cat <<'EOF'\n)"));
        assert!(is_opaque("$(cat <<'EOF'\n) $(date)"));
        assert!(is_opaque("$(cat <<'EOF'\n) $FOO"));
    }

    #[test]
    fn substitution_outside_single_quotes() {
        assert!(has_substitution("gh pr create --body $(cat f)"));
        assert!(has_substitution("gh pr create --body \"$(cat f)\""));
        assert!(has_substitution("gh pr create --body `cat f`"));
        assert!(!has_substitution("gh pr create --body '$(cat f)'"));
        assert!(!has_substitution("gh pr create --body \"$(cat <<'EOF'\nx\nEOF\n)\""));
        assert!(has_substitution("gh pr create --body \"$(cat <<'EOF'\nx\nEOF\n)\" $(date)"));
        assert!(!has_substitution("gh pr create --body \\$(x)"));
    }
}
