//! Line-scan outline of source files, for `ctx`. Pure Rust, no parser crate, no regex over files.
//!
//! Each language family has a hand-written scanner:
//!
//! - **c-like** (ts, tsx, js, jsx, mjs, cjs, rs, go, java, css, scss): a masking pass blanks
//!   comments, strings, template literals, regex literals, char literals and raw strings (same
//!   length, newlines kept). The symbol pass then works on the masked text, so a brace inside a
//!   string can never count. A symbol ends at its matching brace, or at its `;`.
//! - **indent** (py): a masking pass blanks comments and strings and flags continuation lines
//!   (inside a string, inside brackets, or after a backslash). A symbol ends at the last line
//!   before the next logical line at the same or a lower indent.
//! - **markdown** (md, mdx): ATX headings outside fenced code. A heading ends before the next
//!   heading of the same or a higher level.
//! - **shell** (sh, bash, zsh): the c-like pass with shell masking (quotes, comments, heredocs).
//!
//! Known gaps: a regex literal at the start of a line after an operand, JSX text with a quote
//! or brace that is not balanced, shell arithmetic like `$((a<<b))`, Python f-strings that nest
//! the same quote (3.12), Markdown setext headings.

use std::path::Path;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_LINE_CHARS: usize = 10_000;

/// One symbol. Lines are 1-based and inclusive. `depth` is the number of enclosing symbols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub depth: usize,
}

/// Why a file is not scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Over 2 MiB.
    TooBig,
    /// A line over 10,000 chars (minified code).
    LongLine,
    /// A NUL byte.
    NotText,
    /// No scanner for this extension (the extension is carried).
    NoScanner(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ts,
    Rust,
    Go,
    Java,
    Css,
    Scss,
    Shell,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Brace(Lang),
    Indent,
    Markdown,
}

fn ext_lower(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn family_of(path: &Path) -> Option<Family> {
    Some(match ext_lower(path).as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Family::Brace(Lang::Ts),
        "rs" => Family::Brace(Lang::Rust),
        "go" => Family::Brace(Lang::Go),
        "java" => Family::Brace(Lang::Java),
        "css" => Family::Brace(Lang::Css),
        "scss" => Family::Brace(Lang::Scss),
        "py" => Family::Indent,
        "md" | "mdx" => Family::Markdown,
        "sh" | "bash" | "zsh" => Family::Brace(Lang::Shell),
        _ => return None,
    })
}

/// The scanner family of a path (`c-like`, `indent`, `markdown`, `shell`), or `None`.
pub fn scan_name(path: &Path) -> Option<&'static str> {
    family_of(path).map(|f| match f {
        Family::Brace(Lang::Shell) => "shell",
        Family::Brace(_) => "c-like",
        Family::Indent => "indent",
        Family::Markdown => "markdown",
    })
}

/// The symbols of `text`, or the reason it is not scanned. `path` picks the scanner.
pub fn outline(path: &Path, text: &str) -> Result<Vec<Symbol>, Refusal> {
    if text.len() > MAX_BYTES {
        return Err(Refusal::TooBig);
    }
    if text.as_bytes().contains(&0) {
        return Err(Refusal::NotText);
    }
    if has_long_line(text) {
        return Err(Refusal::LongLine);
    }
    let Some(family) = family_of(path) else {
        return Err(Refusal::NoScanner(ext_lower(path)));
    };
    Ok(match family {
        Family::Brace(lang) => scan_brace(lang, text),
        Family::Indent => scan_python(text),
        Family::Markdown => scan_markdown(text),
    })
}

/// One line per symbol: two spaces per depth, then `kind name start-end`.
pub fn render(symbols: &[Symbol]) -> String {
    let mut out = String::new();
    for s in symbols {
        out.push_str(&"  ".repeat(s.depth));
        out.push_str(&format!("{} {} {}-{}\n", s.kind, s.name, s.start, s.end));
    }
    out
}

fn has_long_line(text: &str) -> bool {
    text.split('\n').any(|l| {
        let l = l.strip_suffix('\r').unwrap_or(l);
        l.len() > MAX_LINE_CHARS && l.chars().count() > MAX_LINE_CHARS
    })
}

// ---------------------------------------------------------------------------------------------
// Shared helpers

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The text as chars, CRLF turned into LF, a leading BOM dropped.
fn normalize(text: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(text.len());
    let mut it = text.strip_prefix('\u{feff}').unwrap_or(text).chars().peekable();
    while let Some(c) = it.next() {
        if c == '\r' && it.peek() == Some(&'\n') {
            continue;
        }
        out.push(c);
    }
    out
}

fn line_ranges(chars: &[char]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut s = 0;
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            v.push((s, i));
            s = i + 1;
        }
    }
    if s < chars.len() {
        v.push((s, chars.len()));
    }
    v
}

/// The original text and a masked copy of the same length, cut into lines.
struct Doc {
    orig: Vec<char>,
    masked: Vec<char>,
    lines: Vec<(usize, usize)>,
}

impl Doc {
    fn new(orig: Vec<char>, masked: Vec<char>) -> Doc {
        let lines = line_ranges(&orig);
        Doc { orig, masked, lines }
    }
    fn len(&self) -> usize {
        self.lines.len()
    }
    fn m(&self, i: usize) -> &[char] {
        let (a, b) = self.lines[i];
        &self.masked[a..b]
    }
    fn o(&self, i: usize) -> &[char] {
        let (a, b) = self.lines[i];
        &self.orig[a..b]
    }
}

fn blank(out: &mut [char], from: usize, to: usize) {
    let to = to.min(out.len());
    if from >= to {
        return;
    }
    for ch in &mut out[from..to] {
        if *ch != '\n' {
            *ch = ' ';
        }
    }
}

fn line_end(chars: &[char], i: usize) -> usize {
    chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p)
}

fn skip_ws(s: &[char], mut i: usize) -> usize {
    while i < s.len() && s[i].is_whitespace() {
        i += 1;
    }
    i
}

fn word_end(s: &[char], i: usize) -> usize {
    let mut j = i;
    while j < s.len() && is_ident(s[j]) {
        j += 1;
    }
    j
}

fn word(s: &[char], i: usize) -> (String, usize) {
    let e = word_end(s, i);
    (s[i.min(e)..e].iter().collect(), e)
}

/// The index after the `)` that matches the `(` at `i`, or the end of the line.
fn skip_balanced(s: &[char], i: usize) -> usize {
    let mut depth = 0usize;
    let mut j = i;
    while j < s.len() {
        match s[j] {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    s.len()
}

fn indent_width(m: &[char]) -> usize {
    let mut w = 0;
    for &c in m {
        match c {
            ' ' => w += 1,
            '\t' => w = (w / 8 + 1) * 8,
            _ => break,
        }
    }
    w
}

/// The end index (exclusive) of a quoted string that starts at `i`. A backslash skips the next
/// char. A one-line string stops before the newline.
fn quoted_end(chars: &[char], i: usize, q: char, multiline: bool) -> usize {
    let n = chars.len();
    let mut j = i + 1;
    while j < n {
        let c = chars[j];
        if c == '\\' {
            j += 2;
            continue;
        }
        if c == q {
            return j + 1;
        }
        if c == '\n' && !multiline {
            return j;
        }
        j += 1;
    }
    n
}

fn block_comment_end(chars: &[char], i: usize, nested: bool) -> usize {
    let n = chars.len();
    let mut depth = 1;
    let mut j = i + 2;
    while j < n && depth > 0 {
        if nested && chars[j] == '/' && chars.get(j + 1) == Some(&'*') {
            depth += 1;
            j += 2;
        } else if chars[j] == '*' && chars.get(j + 1) == Some(&'/') {
            depth -= 1;
            j += 2;
        } else {
            j += 1;
        }
    }
    j.min(n)
}

// ---------------------------------------------------------------------------------------------
// Masking: c-like

/// The last code chars seen, to tell a regex literal from a division.
#[derive(Default)]
struct Prev {
    c: Option<char>,
    c2: Option<char>,
    word: String,
    ident: bool,
    ws: bool,
}

impl Prev {
    fn sig(&mut self, c: char) {
        self.c2 = self.c;
        self.c = Some(c);
        if is_ident(c) {
            if self.ident && !self.ws {
                self.word.push(c);
            } else {
                self.word.clear();
                self.word.push(c);
            }
            self.ident = true;
        } else {
            self.ident = false;
            self.word.clear();
        }
        self.ws = false;
    }
}

fn regex_ok(p: &Prev) -> bool {
    match p.c {
        None => true,
        Some(c) if "(,=:[!&|?{};".contains(c) => true,
        Some('>') => p.c2 == Some('='),
        Some(c) if is_ident(c) => matches!(
            p.word.as_str(),
            "return" | "typeof" | "case" | "do" | "else" | "in" | "of" | "void" | "delete" | "throw" | "new" | "yield" | "await" | "instanceof"
        ),
        _ => false,
    }
}

/// The end (exclusive) of a regex literal that starts at `i`, if one closes on the same line.
fn regex_end(chars: &[char], i: usize) -> Option<usize> {
    let n = chars.len();
    let mut j = i + 1;
    let mut in_class = false;
    while j < n {
        match chars[j] {
            '\n' => return None,
            '\\' => j += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                j += 1;
                while j < n && chars[j].is_alphabetic() {
                    j += 1;
                }
                return Some(j);
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// A Rust char literal at `i` (`'x'`, `'\n'`, `'\u{1F600}'`), or `None` for a lifetime.
fn rust_char_end(chars: &[char], i: usize) -> Option<usize> {
    let n = chars.len();
    match chars.get(i + 1) {
        Some('\\') => {
            let mut j = i + 3;
            while j < n && chars[j] != '\'' && chars[j] != '\n' {
                j += 1;
            }
            (j < n && chars[j] == '\'').then_some(j + 1)
        }
        Some(&c) if c != '\'' && c != '\n' && chars.get(i + 2) == Some(&'\'') => Some(i + 3),
        _ => None,
    }
}

/// A Rust raw string (`r#"..."#`, `br"..."`) at `i`: the end, exclusive.
fn rust_raw_end(chars: &[char], i: usize) -> Option<usize> {
    let n = chars.len();
    let mut j = i;
    if matches!(chars[j], 'b' | 'c') {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    let mut k = j + 1;
    while k < n {
        if chars[k] == '"' && (0..hashes).all(|t| chars.get(k + 1 + t) == Some(&'#')) {
            return Some(k + 1 + hashes);
        }
        k += 1;
    }
    Some(n)
}

/// A Java text block `"""..."""` at `i`: the end, exclusive.
fn text_block_end(chars: &[char], i: usize) -> usize {
    let n = chars.len();
    let mut j = i + 3;
    while j < n {
        if chars[j] == '\\' {
            j += 2;
            continue;
        }
        if chars[j] == '"' && chars.get(j + 1) == Some(&'"') && chars.get(j + 2) == Some(&'"') {
            return j + 3;
        }
        j += 1;
    }
    n
}

/// Blanks a template literal segment that starts at `start` (a backtick, or the `}` that closes
/// an interpolation). Returns the end and whether it stopped at a `${` (code follows).
fn template_segment(chars: &[char], out: &mut [char], start: usize, interp: &mut Vec<usize>) -> (usize, bool) {
    let n = chars.len();
    let mut j = start + 1;
    loop {
        if j >= n {
            blank(out, start, n);
            return (n, false);
        }
        match chars[j] {
            '\\' => j += 2,
            '`' => {
                blank(out, start, j + 1);
                return (j + 1, false);
            }
            '$' if chars.get(j + 1) == Some(&'{') => {
                blank(out, start, j + 2);
                interp.push(0);
                return (j + 2, true);
            }
            _ => j += 1,
        }
    }
}

fn mask_brace(chars: &[char], lang: Lang) -> Vec<char> {
    let n = chars.len();
    let mut out = chars.to_vec();
    let mut interp: Vec<usize> = Vec::new();
    let mut prev = Prev::default();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            prev.ws = true;
            i += 1;
            continue;
        }
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') && lang != Lang::Css && !(lang == Lang::Scss && prev.c == Some(':')) {
            let e = line_end(chars, i);
            blank(&mut out, i, e);
            prev.ws = true;
            i = e;
            continue;
        }
        if c == '/' && next == Some('*') {
            let e = block_comment_end(chars, i, lang == Lang::Rust);
            blank(&mut out, i, e);
            prev.ws = true;
            i = e;
            continue;
        }
        match c {
            '"' => {
                let e = if lang == Lang::Java && chars[i..].starts_with(&['"', '"', '"']) {
                    text_block_end(chars, i)
                } else {
                    quoted_end(chars, i, '"', lang == Lang::Rust)
                };
                blank(&mut out, i, e);
                prev.sig('"');
                i = e;
                continue;
            }
            '\'' => {
                if lang == Lang::Rust {
                    if let Some(e) = rust_char_end(chars, i) {
                        blank(&mut out, i, e);
                        prev.sig('"');
                        i = e;
                        continue;
                    }
                } else {
                    let e = quoted_end(chars, i, '\'', false);
                    blank(&mut out, i, e);
                    prev.sig('"');
                    i = e;
                    continue;
                }
            }
            '`' if lang == Lang::Ts => {
                let (e, entered) = template_segment(chars, &mut out, i, &mut interp);
                prev.sig(if entered { '(' } else { '"' });
                i = e;
                continue;
            }
            '`' if lang == Lang::Go => {
                let e = chars[i + 1..].iter().position(|&x| x == '`').map_or(n, |p| i + 2 + p);
                blank(&mut out, i, e);
                prev.sig('"');
                i = e;
                continue;
            }
            '{' => {
                if let Some(t) = interp.last_mut() {
                    *t += 1;
                }
            }
            '}' => {
                if let Some(t) = interp.last_mut() {
                    if *t == 0 {
                        interp.pop();
                        let (e, entered) = template_segment(chars, &mut out, i, &mut interp);
                        prev.sig(if entered { '(' } else { '"' });
                        i = e;
                        continue;
                    }
                    *t -= 1;
                }
            }
            '/' if lang == Lang::Ts && regex_ok(&prev) => {
                if let Some(e) = regex_end(chars, i) {
                    blank(&mut out, i, e);
                    prev.sig('"');
                    i = e;
                    continue;
                }
            }
            'r' | 'b' | 'c' if lang == Lang::Rust && !(i > 0 && is_ident(chars[i - 1])) => {
                if let Some(e) = rust_raw_end(chars, i) {
                    blank(&mut out, i, e);
                    prev.sig('"');
                    i = e;
                    continue;
                }
            }
            _ => {}
        }
        prev.sig(c);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Masking: shell

fn heredoc_region(chars: &[char], i: usize, last: Option<&(usize, usize)>) -> Option<(usize, usize)> {
    let n = chars.len();
    let mut j = i + 2;
    if chars.get(j) == Some(&'-') {
        j += 1;
    }
    while matches!(chars.get(j), Some(' ' | '\t')) {
        j += 1;
    }
    let quote = chars.get(j).copied().filter(|&q| q == '\'' || q == '"');
    if quote.is_some() {
        j += 1;
    }
    let ws = j;
    while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
        j += 1;
    }
    if ws == j || !(chars[ws].is_alphabetic() || chars[ws] == '_') {
        return None;
    }
    let tag = &chars[ws..j];
    let le = line_end(chars, i);
    let body = last.map_or(le + 1, |r| (r.1 + 1).max(le + 1));
    let mut from = body;
    while from < n {
        let e = line_end(chars, from);
        let line = &chars[from..e];
        let start = line.iter().position(|&c| c != '\t').unwrap_or(line.len());
        let end = line.iter().rposition(|c| !c.is_whitespace()).map_or(start, |p| p + 1).max(start);
        if &line[start..end] == tag {
            return Some((body, e));
        }
        from = e + 1;
    }
    None
}

fn mask_shell(chars: &[char]) -> Vec<char> {
    let n = chars.len();
    let mut out = chars.to_vec();
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut ri = 0;
    let mut i = 0;
    while i < n {
        if ri < regions.len() && i >= regions[ri].0 {
            blank(&mut out, regions[ri].0, regions[ri].1);
            i = regions[ri].1.max(i);
            ri += 1;
            continue;
        }
        let c = chars[i];
        match c {
            '\\' => i += 2,
            '\'' => {
                let e = chars[i + 1..].iter().position(|&x| x == '\'').map_or(n, |p| i + 2 + p);
                blank(&mut out, i, e);
                i = e;
            }
            '"' => {
                let e = quoted_end(chars, i, '"', true);
                blank(&mut out, i, e);
                i = e;
            }
            '$' if chars.get(i + 1) == Some(&'\'') => {
                let e = quoted_end(chars, i + 1, '\'', true);
                blank(&mut out, i, e);
                i = e;
            }
            '#' if i == 0 || chars[i - 1].is_whitespace() || ";&|(".contains(chars[i - 1]) => {
                let e = line_end(chars, i);
                blank(&mut out, i, e);
                i = e;
            }
            '<' if chars.get(i + 1) == Some(&'<') && chars.get(i + 2) == Some(&'<') => i += 3,
            '<' if chars.get(i + 1) == Some(&'<') => {
                if let Some(r) = heredoc_region(chars, i, regions.last()) {
                    regions.push(r);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Brace families: symbols

struct Cand {
    kind: &'static str,
    name: String,
    /// The `=>` must show up before the statement ends (a `const x = ...` that is an arrow).
    arrow: bool,
}

fn cand(kind: &'static str, name: String) -> Option<Cand> {
    (!name.is_empty()).then_some(Cand { kind, name, arrow: false })
}

const TS_PREFIX: [&str; 5] = ["export", "default", "declare", "abstract", "async"];
const TS_MODS: [&str; 12] = [
    "public", "private", "protected", "static", "async", "readonly", "override", "abstract", "get", "set", "declare", "accessor",
];

fn ts_method(s: &[char], start: usize) -> Option<Cand> {
    let mut j = start;
    for _ in 0..8 {
        if s.get(j) == Some(&'*') {
            j = skip_ws(s, j + 1);
        }
        let st = j;
        if s.get(j) == Some(&'#') {
            j += 1;
        }
        let e = word_end(s, j);
        if e == j {
            return None;
        }
        let name: String = s[st..e].iter().collect();
        let mut k = skip_ws(s, e);
        if matches!(s.get(k).copied(), Some('?' | '!')) {
            k = skip_ws(s, k + 1);
        }
        if matches!(s.get(k).copied(), Some('(' | '<')) {
            if matches!(name.as_str(), "if" | "for" | "while" | "switch" | "catch" | "return" | "with" | "super" | "function") {
                return None;
            }
            return cand("method", name);
        }
        if TS_MODS.contains(&name.as_str()) {
            j = k;
            continue;
        }
        return None;
    }
    None
}

/// `const|let|var NAME [: T] = ...` that is an arrow function or a function expression.
fn ts_const(s: &[char], k: usize) -> Option<Cand> {
    let (name, e) = word(s, k);
    if name.is_empty() {
        return None;
    }
    let mut p = e;
    while p < s.len() && s[p] != '=' {
        if s[p] == ';' {
            return None;
        }
        p += 1;
    }
    if p >= s.len() || matches!(s.get(p + 1), Some('=' | '>')) {
        return None;
    }
    let r = skip_ws(s, p + 1);
    let (w, we) = word(s, r);
    if w == "function" {
        return cand("const", name);
    }
    let r2 = if w == "async" { skip_ws(s, we) } else { r };
    let arrow = match s.get(r2) {
        Some('(' | '<') => true,
        _ => {
            let (w2, e2) = word(s, r2);
            !w2.is_empty() && s[skip_ws(s, e2)..].starts_with(&['=', '>'])
        }
    };
    arrow.then_some(Cand { kind: "const", name, arrow: true })
}

fn detect_ts(s: &[char], col: usize, in_class: bool) -> Option<Cand> {
    let mut j = skip_ws(s, col);
    if in_class {
        return ts_method(s, j);
    }
    loop {
        let (w, e) = word(s, j);
        let k = skip_ws(s, e);
        if TS_PREFIX.contains(&w.as_str()) && k < s.len() && is_ident(s[k]) {
            j = k;
        } else {
            break;
        }
    }
    let (w, e) = word(s, j);
    let k = skip_ws(s, e);
    match w.as_str() {
        "function" => {
            let k = if s.get(k) == Some(&'*') { skip_ws(s, k + 1) } else { k };
            cand("function", word(s, k).0)
        }
        "class" => {
            let name = word(s, k).0;
            if matches!(name.as_str(), "extends" | "implements") { None } else { cand("class", name) }
        }
        "interface" => cand("interface", word(s, k).0),
        "enum" => cand("enum", word(s, k).0),
        "const" => {
            let (w2, e2) = word(s, k);
            if w2 == "enum" { cand("enum", word(s, skip_ws(s, e2)).0) } else { ts_const(s, k) }
        }
        "let" | "var" => ts_const(s, k),
        "type" => {
            let (name, ne) = word(s, k);
            let after = s.get(skip_ws(s, ne)).copied();
            if matches!(after, Some('=' | '<')) { cand("type", name) } else { None }
        }
        _ => None,
    }
}

fn detect_rust(doc: &Doc, ln: usize, col: usize) -> Option<Cand> {
    let s = doc.m(ln);
    let mut j = skip_ws(s, col);
    loop {
        let (w, e) = word(s, j);
        let mut k = skip_ws(s, e);
        match w.as_str() {
            "pub" => {
                if s.get(k) == Some(&'(') {
                    k = skip_ws(s, skip_balanced(s, k));
                }
                j = k;
            }
            "unsafe" | "async" | "const" | "extern" | "default" | "safe" => j = k,
            _ => break,
        }
    }
    let (w, e) = word(s, j);
    let k = skip_ws(s, e);
    let kind: &'static str = match w.as_str() {
        "fn" => "fn",
        "struct" => "struct",
        "enum" => "enum",
        "trait" => "trait",
        "mod" => "mod",
        "type" => "type",
        "impl" => return rust_impl(doc, ln, k),
        "macro_rules" if s.get(e) == Some(&'!') => return cand("macro_rules", word(s, skip_ws(s, e + 1)).0),
        _ => return None,
    };
    cand(kind, word(s, k).0)
}

/// The name of an `impl`: the text after its own generics, up to `{` or `where`.
fn rust_impl(doc: &Doc, ln: usize, k: usize) -> Option<Cand> {
    let name = impl_text(doc, ln, k)
        .filter(|t| !t.is_empty())
        .or_else(|| (ln + 1 < doc.len()).then(|| impl_text(doc, ln + 1, 0)).flatten())
        .filter(|t| !t.is_empty())?;
    Some(Cand { kind: "impl", name, arrow: false })
}

fn impl_text(doc: &Doc, ln: usize, from: usize) -> Option<String> {
    let o = doc.o(ln);
    let m = doc.m(ln);
    let mut p = skip_ws(o, from);
    if o.get(p) == Some(&'<') {
        let mut depth = 0usize;
        while p < o.len() {
            match o[p] {
                '<' => depth += 1,
                '>' if p > 0 && o[p - 1] != '-' => depth = depth.saturating_sub(1),
                _ => {}
            }
            p += 1;
            if depth == 0 {
                break;
            }
        }
    }
    let p = p.min(o.len());
    let stop = m[p..].iter().position(|&c| c == '{').map_or(o.len(), |x| p + x);
    let text: String = o[p..stop].iter().collect();
    let words: Vec<&str> = text.split_whitespace().take_while(|w| *w != "where").collect();
    Some(words.join(" "))
}

fn detect_go(s: &[char], col: usize) -> Option<Cand> {
    let j = skip_ws(s, col);
    let (w, e) = word(s, j);
    let mut k = skip_ws(s, e);
    match w.as_str() {
        "func" => {
            if s.get(k) == Some(&'(') {
                k = skip_ws(s, skip_balanced(s, k));
            }
            let (name, ne) = word(s, k);
            let after = s.get(skip_ws(s, ne)).copied();
            if matches!(after, Some('(' | '[')) { cand("func", name) } else { None }
        }
        "type" => cand("type", word(s, k).0),
        _ => None,
    }
}

const JAVA_MODS: [&str; 12] = [
    "public", "protected", "private", "static", "final", "abstract", "sealed", "strictfp", "synchronized", "native", "default", "transient",
];

fn detect_java(s: &[char], col: usize, host: Option<&str>) -> Option<Cand> {
    let mut j = skip_ws(s, col);
    if s.get(j) == Some(&'@') && word(s, j + 1).0 == "interface" {
        return cand("interface", word(s, skip_ws(s, word(s, j + 1).1)).0);
    }
    loop {
        let (w, e) = word(s, j);
        let k = skip_ws(s, e);
        if JAVA_MODS.contains(&w.as_str()) && k < s.len() && is_ident(s[k]) {
            j = k;
        } else if w == "non" && s.get(e) == Some(&'-') {
            j = skip_ws(s, word(s, e + 1).1);
        } else {
            break;
        }
    }
    let (w, e) = word(s, j);
    let k = skip_ws(s, e);
    match w.as_str() {
        "class" | "record" => return cand("class", word(s, k).0),
        "interface" => return cand("interface", word(s, k).0),
        "enum" => return cand("enum", word(s, k).0),
        _ => {}
    }
    java_method(s, skip_ws(s, col), host?)
}

fn java_method(s: &[char], start: usize, host: &str) -> Option<Cand> {
    let p = start + s[start.min(s.len())..].iter().position(|&c| c == '(')?;
    let before = &s[start..p];
    if before.contains(&'=') {
        return None;
    }
    let first = word(s, start).0;
    if matches!(
        first.as_str(),
        "return" | "new" | "if" | "else" | "for" | "while" | "switch" | "catch" | "try" | "throw" | "synchronized" | "do" | "case" | "assert" | "super" | "this" | "yield"
    ) {
        return None;
    }
    let name_end = before.iter().rposition(|c| !c.is_whitespace())? + 1;
    let mut name_start = name_end;
    while name_start > 0 && is_ident(before[name_start - 1]) {
        name_start -= 1;
    }
    if name_start == name_end || (name_start > 0 && before[name_start - 1] == '.') {
        return None;
    }
    let name: String = before[name_start..name_end].iter().collect();
    let prefix_empty = before[..name_start].iter().all(|c| c.is_whitespace());
    if prefix_empty && name != host {
        return None;
    }
    cand("method", name)
}

fn detect_css(doc: &Doc, ln: usize, col: usize) -> Option<Cand> {
    let m = doc.m(ln);
    let o = doc.o(ln);
    if m.get(col) == Some(&'}') {
        return None;
    }
    let brace_at = |line: &[char], from: usize| (from..line.len()).find(|&p| line[p] == '{' && !(p > 0 && line[p - 1] == '#'));
    if let Some(p) = brace_at(m, col) {
        let prelude: String = o[col..p].iter().collect();
        let prelude = prelude.trim();
        return cand("rule", prelude.to_string());
    }
    // A selector list over several lines: `h1,` then `h2 {`.
    let first: String = o[col..].iter().collect();
    let first = first.trim();
    if !first.ends_with(',') || first.contains(';') {
        return None;
    }
    let mut parts = vec![first.to_string()];
    for l in ln + 1..doc.len().min(ln + 20) {
        let (lm, lo) = (doc.m(l), doc.o(l));
        let c = skip_ws(lm, 0);
        if c >= lm.len() {
            continue;
        }
        if let Some(p) = brace_at(lm, c) {
            let t: String = lo[c..p].iter().collect();
            parts.push(t.trim().to_string());
            return cand("rule", parts.join(" "));
        }
        let t: String = lo[c..].iter().collect();
        let t = t.trim();
        if !t.ends_with(',') {
            return None;
        }
        parts.push(t.to_string());
    }
    None
}

fn shell_name_char(c: char) -> bool {
    c.is_alphanumeric() || "_-.:@+".contains(c)
}

fn detect_shell(s: &[char], col: usize) -> Option<Cand> {
    let j = skip_ws(s, col);
    let run = |from: usize| {
        let mut e = from;
        while e < s.len() && shell_name_char(s[e]) {
            e += 1;
        }
        e
    };
    let e = run(j);
    let first: String = s[j..e].iter().collect();
    let k = skip_ws(s, e);
    if first == "function" && k > e {
        let ne = run(k);
        if ne > k {
            return cand("function", s[k..ne].iter().collect());
        }
        return None;
    }
    if e > j && s.get(k) == Some(&'(') && s.get(skip_ws(s, k + 1)) == Some(&')') && !first.starts_with('-') {
        return cand("function", first);
    }
    None
}

fn detect(lang: Lang, doc: &Doc, ln: usize, col: usize, host: Option<&str>) -> Option<Cand> {
    match lang {
        Lang::Ts => detect_ts(doc.m(ln), col, host.is_some()),
        Lang::Rust => detect_rust(doc, ln, col),
        Lang::Go => detect_go(doc.m(ln), col),
        Lang::Java => detect_java(doc.m(ln), col, host),
        Lang::Css | Lang::Scss => detect_css(doc, ln, col),
        Lang::Shell => detect_shell(doc.m(ln), col),
    }
}

fn hosts_methods(lang: Lang, kind: &str) -> bool {
    match lang {
        Lang::Ts => kind == "class",
        Lang::Java => matches!(kind, "class" | "interface" | "enum"),
        _ => false,
    }
}

/// True when the statement that ends on line `l` is complete: the line does not end in an
/// operator and the next code line does not continue it.
fn line_complete(doc: &Doc, l: usize) -> bool {
    let line = doc.m(l);
    let Some(last) = line.iter().rposition(|c| !c.is_whitespace()) else {
        return false;
    };
    if last >= 1 && line[last] == '>' && line[last - 1] == '=' {
        return false;
    }
    if "=,([{|&?:+-*/%<.!".contains(line[last]) {
        return false;
    }
    for k in l + 1..doc.len() {
        let nl = doc.m(k);
        let c0 = skip_ws(nl, 0);
        if c0 < nl.len() {
            if "|&.?:+-*/%=,)]<>{".contains(nl[c0]) {
                return false;
            }
            return !matches!(word(nl, c0).0.as_str(), "extends" | "implements" | "where");
        }
    }
    true
}

struct End {
    line: usize,
    arrow: bool,
    opened: bool,
}

/// Where the symbol that starts at `(ln, col)` ends (0-based line).
fn find_end(doc: &Doc, lang: Lang, kind: &str, ln: usize, col: usize, budget: &mut usize) -> End {
    let asi = match lang {
        Lang::Ts => matches!(kind, "type" | "const" | "function" | "method"),
        Lang::Go => kind == "type",
        _ => false,
    };
    let max_head = if lang == Lang::Shell { 2 } else { 300 };
    let n = doc.len();
    let (mut paren, mut brace) = (0usize, 0usize);
    let (mut open, mut arrow) = (false, false);
    let mut l = ln;
    while l < n {
        if *budget == 0 {
            break;
        }
        let line = doc.m(l);
        *budget = budget.saturating_sub(line.len() + 1);
        let mut k = if l == ln { col } else { 0 };
        while k < line.len() {
            let c = line[k];
            if open {
                if c == '{' {
                    brace += 1;
                } else if c == '}' {
                    brace = brace.saturating_sub(1);
                    if brace == 0 {
                        let r = skip_ws(line, k + 1);
                        if r < line.len() && ">|&[={".contains(line[r]) {
                            open = false;
                            paren = 0;
                        } else {
                            return End { line: l, arrow, opened: true };
                        }
                    }
                }
            } else {
                match c {
                    '(' | '[' => paren += 1,
                    ')' | ']' => paren = paren.saturating_sub(1),
                    '{' if paren == 0 => {
                        open = true;
                        brace = 1;
                    }
                    ';' if paren == 0 => return End { line: l, arrow, opened: false },
                    '=' if paren == 0 && line.get(k + 1) == Some(&'>') => {
                        arrow = true;
                        k += 1;
                    }
                    _ => {}
                }
            }
            k += 1;
        }
        if !open && paren == 0 && asi && line_complete(doc, l) {
            return End { line: l, arrow, opened: false };
        }
        if !open && l - ln >= max_head {
            return End { line: ln, arrow, opened: false };
        }
        l += 1;
    }
    End { line: if open { n.saturating_sub(1) } else { ln }, arrow, opened: open }
}

/// The end of the decorators (or annotations) that start at `(ln, col)`: the line and column of
/// the first char after them.
fn skip_decorators(doc: &Doc, ln: usize, col: usize) -> Option<(usize, usize)> {
    let (mut l, mut c) = (ln, col);
    loop {
        let m = doc.m(l);
        c += 1;
        while c < m.len() && (is_ident(m[c]) || m[c] == '.') {
            c += 1;
        }
        if m.get(c) == Some(&'(') {
            let mut depth = 0usize;
            'outer: loop {
                let line = doc.m(l);
                while c < line.len() {
                    match line[c] {
                        '(' => depth += 1,
                        ')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                c += 1;
                                break 'outer;
                            }
                        }
                        _ => {}
                    }
                    c += 1;
                }
                l += 1;
                c = 0;
                if l >= doc.len() || l > ln + 100 {
                    return None;
                }
            }
        }
        let m = doc.m(l);
        c = skip_ws(m, c);
        if m.get(c) != Some(&'@') {
            return Some((l, c));
        }
    }
}

fn scan_brace(lang: Lang, text: &str) -> Vec<Symbol> {
    let orig = normalize(text);
    let masked = if lang == Lang::Shell { mask_shell(&orig) } else { mask_brace(&orig, lang) };
    let doc = Doc::new(orig, masked);
    let n = doc.len();

    // Paren depth and brace depth at the start of each line.
    let mut paren0: Vec<bool> = Vec::with_capacity(n);
    let mut brace_at: Vec<usize> = Vec::with_capacity(n);
    let (mut p, mut b) = (0usize, 0usize);
    for i in 0..n {
        paren0.push(p == 0);
        brace_at.push(b);
        for &c in doc.m(i) {
            match c {
                '(' | '[' => p += 1,
                ')' | ']' => p = p.saturating_sub(1),
                '{' => b += 1,
                '}' => b = b.saturating_sub(1),
                _ => {}
            }
        }
    }

    let decorators = matches!(lang, Lang::Ts | Lang::Java);
    let mut budget = doc.masked.len() * 40 + 100_000;
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (symbol index, brace depth of its line)
    let mut pending: Option<usize> = None;
    for ln in 0..n {
        while stack.last().is_some_and(|&(si, _)| symbols[si].end < ln + 1) {
            stack.pop();
        }
        if !paren0[ln] {
            continue;
        }
        let m = doc.m(ln);
        let c0 = skip_ws(m, 0);
        if c0 >= m.len() {
            continue;
        }
        let mut col = c0;
        if decorators && m[c0] == '@' && !(lang == Lang::Java && word(m, c0 + 1).0 == "interface") {
            let Some((end_ln, end_col)) = skip_decorators(&doc, ln, c0) else {
                pending = None;
                continue;
            };
            if end_col >= doc.m(end_ln).len() {
                pending.get_or_insert(ln);
                continue;
            }
            if end_ln != ln {
                pending = None;
                continue;
            }
            pending.get_or_insert(ln);
            col = end_col;
        }
        let host: Option<String> = stack
            .last()
            .filter(|&&(si, base)| hosts_methods(lang, &symbols[si].kind) && brace_at[ln] == base + 1)
            .map(|&(si, _)| symbols[si].name.clone());
        let found = detect(lang, &doc, ln, col, host.as_deref());
        let taken = pending.take();
        let Some(c) = found else { continue };
        let end = find_end(&doc, lang, c.kind, ln, col, &mut budget);
        if (c.arrow && !end.arrow) || (lang == Lang::Shell && !end.opened) {
            continue;
        }
        let mut start = taken.unwrap_or(ln);
        let mut last = end.line.max(ln);
        if let Some(&(pi, _)) = stack.last() {
            start = start.max(symbols[pi].start - 1);
            last = last.min(symbols[pi].end - 1);
        }
        symbols.push(Symbol { kind: c.kind.to_string(), name: c.name, start: start + 1, end: last + 1, depth: stack.len() });
        stack.push((symbols.len() - 1, brace_at[ln]));
    }
    symbols
}

// ---------------------------------------------------------------------------------------------
// Python

#[derive(Clone, Copy, PartialEq)]
enum PyState {
    Code,
    Str(char, bool),
}

/// Masks comments and strings. The second value flags, per line, a continuation line: it starts
/// inside a string, inside brackets, or after a backslash.
fn mask_python(chars: &[char]) -> (Vec<char>, Vec<bool>) {
    let n = chars.len();
    let mut out = chars.to_vec();
    let mut cont = vec![false];
    let mut st = PyState::Code;
    let mut depth = 0usize;
    let mut bs = false;
    let mut esc_nl = false;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\n' {
            let cur = st;
            let next_cont = match cur {
                PyState::Code => depth > 0 || bs,
                PyState::Str(_, true) => true,
                PyState::Str(_, false) if esc_nl => true,
                PyState::Str(_, false) => {
                    st = PyState::Code;
                    depth > 0
                }
            };
            cont.push(next_cont);
            bs = false;
            esc_nl = false;
            i += 1;
            continue;
        }
        match st {
            PyState::Code => match c {
                '#' => {
                    let e = line_end(chars, i);
                    blank(&mut out, i, e);
                    i = e;
                }
                '\\' => {
                    if chars.get(i + 1) == Some(&'\n') {
                        bs = true;
                    }
                    i += 1;
                }
                '\'' | '"' => {
                    if chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c) {
                        st = PyState::Str(c, true);
                        blank(&mut out, i, i + 3);
                        i += 3;
                    } else {
                        st = PyState::Str(c, false);
                        blank(&mut out, i, i + 1);
                        i += 1;
                    }
                }
                '(' | '[' | '{' => {
                    depth += 1;
                    i += 1;
                }
                ')' | ']' | '}' => {
                    depth = depth.saturating_sub(1);
                    i += 1;
                }
                _ => i += 1,
            },
            PyState::Str(q, triple) => {
                if c == '\\' {
                    blank(&mut out, i, i + 1);
                    if chars.get(i + 1) == Some(&'\n') {
                        esc_nl = true;
                        i += 1;
                    } else {
                        let e = (i + 2).min(n);
                        blank(&mut out, i + 1, e);
                        i = e;
                    }
                } else if c == q {
                    if !triple {
                        blank(&mut out, i, i + 1);
                        st = PyState::Code;
                        i += 1;
                    } else if chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
                        blank(&mut out, i, i + 3);
                        st = PyState::Code;
                        i += 3;
                    } else {
                        blank(&mut out, i, i + 1);
                        i += 1;
                    }
                } else {
                    blank(&mut out, i, i + 1);
                    i += 1;
                }
            }
        }
    }
    (out, cont)
}

fn py_header(m: &[char], c0: usize) -> Option<(&'static str, String)> {
    let (w, e) = word(m, c0);
    let k = skip_ws(m, e);
    match w.as_str() {
        "def" => Some(("def", word(m, k).0)),
        "class" => Some(("class", word(m, k).0)),
        "async" => {
            let (w2, e2) = word(m, k);
            (w2 == "def").then(|| ("def", word(m, skip_ws(m, e2)).0))
        }
        _ => None,
    }
    .filter(|(_, name)| !name.is_empty())
}

fn scan_python(text: &str) -> Vec<Symbol> {
    let orig = normalize(text);
    let (masked, cont) = mask_python(&orig);
    let doc = Doc::new(orig, masked);
    let n = doc.len();
    let is_cont = |l: usize| cont.get(l).copied().unwrap_or(false);
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut pending: Option<usize> = None;
    for ln in 0..n {
        while stack.last().is_some_and(|&si| symbols[si].end < ln + 1) {
            stack.pop();
        }
        if is_cont(ln) {
            continue;
        }
        let m = doc.m(ln);
        let c0 = skip_ws(m, 0);
        if c0 >= m.len() {
            continue;
        }
        if m[c0] == '@' {
            pending.get_or_insert(ln);
            continue;
        }
        let taken = pending.take();
        let Some((kind, name)) = py_header(m, c0) else { continue };
        let indent = indent_width(m);
        let mut last = ln;
        for l in ln + 1..n {
            if is_cont(l) {
                last = l;
                continue;
            }
            let lm = doc.m(l);
            if skip_ws(lm, 0) >= lm.len() {
                continue;
            }
            if indent_width(lm) <= indent {
                break;
            }
            last = l;
        }
        let mut start = taken.unwrap_or(ln);
        if let Some(&pi) = stack.last() {
            start = start.max(symbols[pi].start - 1);
            last = last.min(symbols[pi].end - 1);
        }
        symbols.push(Symbol { kind: kind.to_string(), name, start: start + 1, end: last + 1, depth: stack.len() });
        stack.push(symbols.len() - 1);
    }
    symbols
}

// ---------------------------------------------------------------------------------------------
// Markdown

fn fence_marker(t: &str) -> Option<(char, usize)> {
    let ch = t.chars().next().filter(|&c| c == '`' || c == '~')?;
    let run = t.chars().take_while(|&c| c == ch).count();
    (run >= 3).then_some((ch, run))
}

fn heading_of(t: &str) -> Option<(usize, String)> {
    let level = t.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &t[level..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let mut name = rest.trim();
    if name.ends_with('#') {
        let stripped = name.trim_end_matches('#');
        if stripped.is_empty() {
            name = "";
        } else if stripped.ends_with(|c: char| c.is_whitespace()) {
            name = stripped.trim_end();
        }
    }
    (!name.is_empty()).then(|| (level, name.to_string()))
}

fn scan_markdown(text: &str) -> Vec<Symbol> {
    let lines: Vec<&str> = text.strip_prefix('\u{feff}').unwrap_or(text).lines().collect();
    let n = lines.len();
    let mut heads: Vec<(usize, usize, String)> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut i = 0;
    if lines.first().map(|l| l.trim_end()) == Some("---")
        && let Some(k) = lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "..."))
    {
        i = k + 2;
    }
    while i < n {
        let line = lines[i];
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent <= 3 {
            let t = &line[indent..];
            if let Some((ch, len)) = fence_marker(t) {
                match fence {
                    None => fence = Some((ch, len)),
                    Some((fc, fl)) if ch == fc && len >= fl && t[len..].trim().is_empty() => fence = None,
                    _ => {}
                }
            } else if fence.is_none()
                && let Some((level, name)) = heading_of(t)
            {
                heads.push((level, i, name));
            }
        }
        i += 1;
    }
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (k, (level, ln, name)) in heads.iter().enumerate() {
        let next = heads[k + 1..].iter().find(|h| h.0 <= *level).map_or(n, |h| h.1);
        let mut end = next - 1;
        while end > *ln && lines[end].trim().is_empty() {
            end -= 1;
        }
        while stack.last().is_some_and(|&si| symbols[si].end < ln + 1) {
            stack.pop();
        }
        symbols.push(Symbol { kind: format!("h{level}"), name: name.clone(), start: ln + 1, end: end + 1, depth: stack.len() });
        stack.push(symbols.len() - 1);
    }
    symbols
}
