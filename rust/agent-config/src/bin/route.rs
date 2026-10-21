//! route: decide who does each what's-left row, and what comes next.
//!
//! Port of claude/skills/ps_whatsleft/scripts/route.py. It matches the Python
//! script byte for byte on stdout, stderr and exit code: the JSON reader, the
//! argparse errors and the OSError texts are rebuilt to say what Python says.
//! Known gaps: lone surrogates in JSON strings become U+FFFD, the Unicode
//! printable table behind `repr()` is approximate, and the argparse help text is
//! the Python 3.9 one.

use agent_config::pycompat::{Item, classify_arg, py_strip, repr_str};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{Read, Write};

const KINDS: &[&str] =
    &["decision", "login", "no-tool", "optional", "payment", "refused", "secret", "task", "value", "work"];
const RISKS: &[&str] = &["delete", "deploy", "paid", "prod-config", "push"];
const USER_ONLY: &[&str] = &["login", "no-tool", "payment", "refused", "secret"];
const CAP: usize = 4;

const PROG: &str = "route.py";
const USAGE: &str = "usage: route.py [-h] [--kill] [--cap CAP] [--json] [file]\n";
const HELP: &str = "usage: route.py [-h] [--kill] [--cap CAP] [--json] [file]

route.py: decide who does each what's-left row, and what comes next.

positional arguments:
  file        rows JSON (default: stdin)

optional arguments:
  -h, --help  show this help message and exit
  --kill      `kill dead tasks`: STOP tasks are DO
  --cap CAP   max questions (default 4)
  --json      print JSON
";

// ---------------------------------------------------------------------------
// A JSON value with Python semantics (truthiness, repr, str, json.dumps).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Py {
    Null,
    Bool(bool),
    Int(String),
    Float(f64),
    Str(String),
    List(Vec<Self>),
    Dict(Vec<(String, Self)>),
}

impl Py {
    fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Int(s) => s != "0",
            Self::Float(f) => *f != 0.0,
            Self::Str(s) => !s.is_empty(),
            Self::List(l) => !l.is_empty(),
            Self::Dict(d) => !d.is_empty(),
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
}

fn dget<'a>(d: &'a [(String, Py)], key: &str) -> Option<&'a Py> {
    d.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Python's repr(float).
fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f < 0.0 { "-inf".into() } else { "inf".into() };
    }
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let e = format!("{:e}", f.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let exp: i32 = exp.parse().unwrap();
    let decpt = exp + 1;
    let n = digits.len() as i32;
    let body = if !(-3..=16).contains(&decpt) {
        let m = if n > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits };
        let x = decpt - 1;
        format!("{}e{}{:02}", m, if x < 0 { '-' } else { '+' }, x.abs())
    } else if decpt <= 0 {
        format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
    } else if decpt >= n {
        format!("{}{}.0", digits, "0".repeat((decpt - n) as usize))
    } else {
        format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
    };
    format!("{sign}{body}")
}

fn py_repr(v: &Py) -> String {
    match v {
        Py::Null => "None".into(),
        Py::Bool(true) => "True".into(),
        Py::Bool(false) => "False".into(),
        Py::Int(s) => s.clone(),
        Py::Float(f) => float_repr(*f),
        Py::Str(s) => repr_str(s),
        Py::List(l) => format!("[{}]", l.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Py::Dict(d) => format!(
            "{{{}}}",
            d.iter().map(|(k, v)| format!("{}: {}", repr_str(k), py_repr(v))).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn py_str(v: &Py) -> String {
    match v {
        Py::Str(s) => s.clone(),
        other => py_repr(other),
    }
}

fn str_list_repr(items: &[&str]) -> String {
    format!("[{}]", items.iter().map(|s| repr_str(s)).collect::<Vec<_>>().join(", "))
}

fn json_escape(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// json.dumps(v, indent=2, ensure_ascii=False), nested at `level`.
fn dumps(v: &Py, level: usize) -> String {
    let pad = "  ".repeat(level + 1);
    let end = "  ".repeat(level);
    match v {
        Py::Null => "null".into(),
        Py::Bool(true) => "true".into(),
        Py::Bool(false) => "false".into(),
        Py::Int(s) => s.clone(),
        Py::Float(f) => {
            if f.is_nan() {
                "NaN".into()
            } else if f.is_infinite() {
                if *f < 0.0 { "-Infinity".into() } else { "Infinity".into() }
            } else {
                float_repr(*f)
            }
        }
        Py::Str(s) => json_escape(s),
        Py::List(l) if l.is_empty() => "[]".into(),
        Py::List(l) => format!(
            "[\n{}\n{}]",
            l.iter().map(|x| format!("{}{}", pad, dumps(x, level + 1))).collect::<Vec<_>>().join(",\n"),
            end
        ),
        Py::Dict(d) if d.is_empty() => "{}".into(),
        Py::Dict(d) => format!(
            "{{\n{}\n{}}}",
            d.iter()
                .map(|(k, x)| format!("{}{}: {}", pad, json_escape(k), dumps(x, level + 1)))
                .collect::<Vec<_>>()
                .join(",\n"),
            end
        ),
    }
}

// ---------------------------------------------------------------------------
// json.loads with Python's error messages.
// ---------------------------------------------------------------------------

struct JErr {
    msg: &'static str,
    pos: usize,
}

type JR<T> = Result<T, JErr>;

struct Reader {
    s: Vec<char>,
}

impl Reader {
    fn ws(&self, mut i: usize) -> usize {
        while i < self.s.len() && matches!(self.s[i], ' ' | '\t' | '\n' | '\r') {
            i += 1;
        }
        i
    }

    fn lit(&self, i: usize, word: &str) -> bool {
        let w: Vec<char> = word.chars().collect();
        self.s.len() >= i + w.len() && self.s[i..i + w.len()] == w[..]
    }

    fn value(&self, i: usize) -> JR<(Py, usize)> {
        let c = match self.s.get(i) {
            Some(c) => *c,
            None => return Err(JErr { msg: "Expecting value", pos: i }),
        };
        match c {
            '"' => self.string(i + 1).map(|(s, e)| (Py::Str(s), e)),
            '{' => self.object(i + 1),
            '[' => self.array(i + 1),
            'n' if self.lit(i, "null") => Ok((Py::Null, i + 4)),
            't' if self.lit(i, "true") => Ok((Py::Bool(true), i + 4)),
            'f' if self.lit(i, "false") => Ok((Py::Bool(false), i + 5)),
            'N' if self.lit(i, "NaN") => Ok((Py::Float(f64::NAN), i + 3)),
            'I' if self.lit(i, "Infinity") => Ok((Py::Float(f64::INFINITY), i + 8)),
            '-' if self.lit(i, "-Infinity") => Ok((Py::Float(f64::NEG_INFINITY), i + 9)),
            _ => self.number(i),
        }
    }

    fn number(&self, start: usize) -> JR<(Py, usize)> {
        let s = &self.s;
        let digit = |i: usize| s.get(i).is_some_and(char::is_ascii_digit);
        let mut i = start;
        if s.get(i) == Some(&'-') {
            i += 1;
        }
        match s.get(i) {
            Some('1'..='9') => {
                while digit(i) {
                    i += 1;
                }
            }
            Some('0') => i += 1,
            _ => return Err(JErr { msg: "Expecting value", pos: start }),
        }
        let mut is_float = false;
        if s.get(i) == Some(&'.') && digit(i + 1) {
            i += 1;
            while digit(i) {
                i += 1;
            }
            is_float = true;
        }
        if matches!(s.get(i), Some('e' | 'E')) {
            let mut j = i + 1;
            if matches!(s.get(j), Some('+' | '-')) {
                j += 1;
            }
            if digit(j) {
                while digit(j) {
                    j += 1;
                }
                i = j;
                is_float = true;
            }
        }
        let text: String = s[start..i].iter().collect();
        if is_float {
            let f: f64 = text.parse().unwrap_or(f64::NAN);
            Ok((Py::Float(f), i))
        } else if text == "-0" {
            Ok((Py::Int("0".into()), i))
        } else {
            Ok((Py::Int(text), i))
        }
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        if at + 4 > self.s.len() {
            return None;
        }
        let mut v = 0;
        for k in 0..4 {
            v = v * 16 + self.s[at + k].to_digit(16)?;
        }
        Some(v)
    }

    /// `i` is the index just after the opening quote.
    fn string(&self, mut i: usize) -> JR<(String, usize)> {
        let begin = i - 1;
        let n = self.s.len();
        let mut out = String::new();
        loop {
            if i >= n {
                return Err(JErr { msg: "Unterminated string starting at", pos: begin });
            }
            let c = self.s[i];
            if c == '"' {
                return Ok((out, i + 1));
            }
            if c != '\\' {
                if (c as u32) < 0x20 {
                    return Err(JErr { msg: "Invalid control character at", pos: i });
                }
                out.push(c);
                i += 1;
                continue;
            }
            // escape
            if i + 1 >= n {
                return Err(JErr { msg: "Unterminated string starting at", pos: begin });
            }
            let e = self.s[i + 1];
            if e != 'u' {
                let r = match e {
                    '"' => '"',
                    '\\' => '\\',
                    '/' => '/',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    _ => return Err(JErr { msg: "Invalid \\escape", pos: i }),
                };
                out.push(r);
                i += 2;
                continue;
            }
            // \uXXXX; the C scanner reports a short or bad escape at the 'u'
            let u = i + 1;
            if u + 5 >= n {
                return Err(JErr { msg: "Invalid \\uXXXX escape", pos: u });
            }
            let Some(mut code) = self.hex4(u + 1) else {
                return Err(JErr { msg: "Invalid \\uXXXX escape", pos: u });
            };
            i = u + 5;
            if (0xD800..=0xDBFF).contains(&code) && i + 6 < n && self.s[i] == '\\' && self.s[i + 1] == 'u' {
                match self.hex4(i + 2) {
                    None => return Err(JErr { msg: "Invalid \\uXXXX escape", pos: i + 1 }),
                    Some(lo) if (0xDC00..=0xDFFF).contains(&lo) => {
                        code = 0x10000 + (((code - 0xD800) << 10) | (lo - 0xDC00));
                        i += 6;
                    }
                    Some(_) => {}
                }
            }
            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
        }
    }

    fn object(&self, mut i: usize) -> JR<(Py, usize)> {
        let n = self.s.len();
        let mut items: Vec<(String, Py)> = Vec::new();
        i = self.ws(i);
        if i >= n || self.s[i] != '}' {
            loop {
                if i >= n || self.s[i] != '"' {
                    return Err(JErr { msg: "Expecting property name enclosed in double quotes", pos: i });
                }
                let (key, e) = self.string(i + 1)?;
                i = self.ws(e);
                if i >= n || self.s[i] != ':' {
                    return Err(JErr { msg: "Expecting ':' delimiter", pos: i });
                }
                i = self.ws(i + 1);
                let (val, e) = self.value(i)?;
                match items.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = val,
                    None => items.push((key, val)),
                }
                i = self.ws(e);
                if i < n && self.s[i] == '}' {
                    break;
                }
                if i >= n || self.s[i] != ',' {
                    return Err(JErr { msg: "Expecting ',' delimiter", pos: i });
                }
                i = self.ws(i + 1);
            }
        }
        Ok((Py::Dict(items), i + 1))
    }

    fn array(&self, mut i: usize) -> JR<(Py, usize)> {
        let n = self.s.len();
        let mut items = Vec::new();
        i = self.ws(i);
        if i >= n || self.s[i] != ']' {
            loop {
                let (val, e) = self.value(i)?;
                items.push(val);
                i = self.ws(e);
                if i < n && self.s[i] == ']' {
                    break;
                }
                if i >= n || self.s[i] != ',' {
                    return Err(JErr { msg: "Expecting ',' delimiter", pos: i });
                }
                i = self.ws(i + 1);
            }
        }
        Ok((Py::List(items), i + 1))
    }
}

/// json.loads(text): the error text is Python's.
fn json_loads(text: &str) -> Result<Py, String> {
    let r = Reader { s: text.chars().collect() };
    let fail = |msg: &str, pos: usize| -> String {
        let before = &r.s[..pos.min(r.s.len())];
        let line = before.iter().filter(|c| **c == '\n').count() + 1;
        let col = match before.iter().rposition(|c| *c == '\n') {
            Some(nl) => pos - nl,
            None => pos + 1,
        };
        format!("{msg}: line {line} column {col} (char {pos})")
    };
    if r.s.first() == Some(&'\u{feff}') {
        return Err(fail("Unexpected UTF-8 BOM (decode using utf-8-sig)", 0));
    }
    let start = r.ws(0);
    let (v, e) = r.value(start).map_err(|e| fail(e.msg, e.pos))?;
    let end = r.ws(e);
    if end != r.s.len() {
        return Err(fail("Extra data", end));
    }
    Ok(v)
}

/// Strict UTF-8 decode that fails with Python's UnicodeDecodeError text. A file
/// is read the way Python's open() reads it (\r\n and \r become \n); stdin is not.
fn decode_text(bytes: &[u8], universal_newlines: bool) -> Result<String, String> {
    match std::str::from_utf8(bytes) {
        Ok(s) if universal_newlines => Ok(s.replace("\r\n", "\n").replace('\r', "\n")),
        Ok(s) => Ok(s.to_string()),
        Err(e) => {
            let p = e.valid_up_to();
            let b = bytes[p];
            let (what, reason) = match e.error_len() {
                Some(1) => (
                    format!("byte 0x{b:02x} in position {p}"),
                    if matches!(b, 0x80..=0xC1 | 0xF5..=0xFF) {
                        "invalid start byte"
                    } else {
                        "invalid continuation byte"
                    },
                ),
                Some(n) => (format!("bytes in position {}-{}", p, p + n - 1), "invalid continuation byte"),
                None => {
                    let last = bytes.len() - 1;
                    if last == p {
                        (format!("byte 0x{b:02x} in position {p}"), "unexpected end of data")
                    } else {
                        (format!("bytes in position {p}-{last}"), "unexpected end of data")
                    }
                }
            };
            Err(format!("'utf-8' codec can't decode {what}: {reason}"))
        }
    }
}

/// Python's OSError text: `[Errno 2] No such file or directory: 'path'`.
fn os_error_text(e: &std::io::Error, path: &str) -> String {
    let text = e.to_string();
    let msg = match text.rfind(" (os error") {
        Some(i) => &text[..i],
        None => &text,
    };
    format!("[Errno {}] {}: {}", e.raw_os_error().unwrap_or(0), msg, repr_str(path))
}

// ---------------------------------------------------------------------------
// Validation and routing.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Row {
    id: String,
    text: Py,
    kind: String,
    who_you: bool,
    risk: bool,
    asked: bool,
    stop: bool,
    done: bool,
    skip: bool,
    after: Vec<String>,
}

fn bad(msg: String) -> Result<(Vec<Row>, Vec<String>), String> {
    Err(msg)
}

/// Python's `validate`: rows with a default id where one is missing, without the rows that have no
/// text, plus one note per dropped row. The caller's rows are not changed.
fn validate(rows: &Py) -> Result<(Vec<Row>, Vec<String>), String> {
    let Py::List(list) = rows else { return bad("rows must be a JSON list".into()) };
    let mut dicts: Vec<&[(String, Py)]> = Vec::new();
    for (i, r) in list.iter().enumerate() {
        let Py::Dict(d) = r else { return bad(format!("row {}: not an object", i + 1)) };
        match dget(d, "id") {
            None | Some(Py::Null | Py::Str(_)) => {}
            Some(_) => return bad(format!("row {}: id must be a string", i + 1)),
        }
        dicts.push(d);
    }
    let mut taken: HashSet<String> = dicts
        .iter()
        .filter_map(|d| match dget(d, "id") {
            Some(Py::Str(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut dropped: Vec<String> = Vec::new();
    type Kept<'a> = (String, &'a [(String, Py)], Py); // id, the row, its text
    let mut kept: Vec<Kept> = Vec::new();
    for (i, d) in dicts.iter().enumerate() {
        let rid = match dget(d, "id") {
            Some(Py::Str(s)) if !s.is_empty() => s.clone(),
            _ => {
                let mut n = i + 1;
                while taken.contains(&format!("r{n}")) {
                    n += 1;
                }
                let rid = format!("r{n}");
                taken.insert(rid.clone());
                rid
            }
        };
        if !seen.insert(rid.clone()) {
            return bad(format!("row {}: duplicate id {}", i + 1, rid));
        }
        let text = dget(d, "text").cloned().unwrap_or(Py::Null);
        let blank = !text.truthy() || matches!(&text, Py::Str(s) if py_strip(s).is_empty());
        if blank {
            dropped.push(rid);
            continue;
        }
        kept.push((rid, d, text));
    }
    let gone: HashSet<&str> = dropped.iter().map(String::as_str).collect();
    let mut out: Vec<Row> = Vec::new();
    for (rid, d, text) in kept {
        let d: &[(String, Py)] = d;
        let kind_v = dget(d, "kind").cloned().unwrap_or(Py::Null);
        let kind = match kind_v.as_str() {
            Some(k) if KINDS.contains(&k) => k.to_string(),
            _ => {
                return bad(format!("{}: kind {} not in {}", rid, py_repr(&kind_v), str_list_repr(KINDS)));
            }
        };
        let mut has_risk = false;
        if let Some(rv) = dget(d, "risk")
            && *rv != Py::Null
        {
            match rv.as_str() {
                Some(s) if RISKS.contains(&s) => has_risk = true,
                _ => {
                    return bad(format!("{}: risk {} not in {}", rid, py_repr(rv), str_list_repr(RISKS)));
                }
            }
        }
        if has_risk && kind != "work" {
            return bad(format!("{rid}: risk only goes on kind work"));
        }
        let who = dget(d, "who").cloned().unwrap_or_else(|| Py::Str("me".into()));
        let who_s = who.as_str();
        if who_s != Some("me") && who_s != Some("you") {
            return bad(format!("{rid}: who must be me or you"));
        }
        if kind == "no-tool" {
            let tried = match dget(d, "tried") {
                Some(v) => py_str(v),
                None => String::new(),
            };
            if py_strip(&tried).is_empty() {
                return bad(format!("{rid}: no-tool needs 'tried': name the tool you tried first"));
            }
        }
        let mut after: Vec<String> = Vec::new();
        match dget(d, "after") {
            None => {}
            Some(Py::List(items)) => {
                for it in items {
                    match it {
                        // nothing waits on a dropped row
                        Py::Str(s) => {
                            if !gone.contains(s.as_str()) {
                                after.push(s.clone());
                            }
                        }
                        _ => return bad(format!("{rid}: after must be a list of ids")),
                    }
                }
            }
            Some(_) => return bad(format!("{rid}: after must be a list of ids")),
        }
        let flag = |k: &str| dget(d, k).is_some_and(Py::truthy);
        out.push(Row {
            id: rid,
            text,
            kind,
            who_you: who_s == Some("you"),
            risk: has_risk,
            asked: flag("asked"),
            stop: flag("stop"),
            done: flag("done"),
            skip: flag("skip"),
            after,
        });
    }
    let ids: HashSet<&str> = out.iter().map(|r| r.id.as_str()).collect();
    for r in &out {
        for dep in &r.after {
            if !ids.contains(dep.as_str()) {
                return bad(format!("{}: after names unknown id {}", r.id, dep));
            }
            if *dep == r.id {
                return bad(format!("{}: waits on itself", r.id));
            }
        }
    }
    let deps: HashMap<&str, &Vec<String>> = out.iter().map(|r| (r.id.as_str(), &r.after)).collect();
    let mut state: HashMap<String, u8> = HashMap::new(); // 1 visiting, 2 done

    fn visit(
        rid: &str,
        path: &[String],
        deps: &HashMap<&str, &Vec<String>>,
        state: &mut HashMap<String, u8>,
    ) -> Result<(), String> {
        match state.get(rid) {
            Some(2) => return Ok(()),
            Some(1) => {
                let mut p = path.to_vec();
                p.push(rid.to_string());
                return Err(format!("after forms a cycle: {}", p.join(" -> ")));
            }
            _ => {}
        }
        state.insert(rid.to_string(), 1);
        let mut np = path.to_vec();
        np.push(rid.to_string());
        for dep in deps[rid] {
            visit(dep, &np, deps, state)?;
        }
        state.insert(rid.to_string(), 2);
        Ok(())
    }

    for r in &out {
        visit(&r.id, &[], &deps, &mut state)?;
    }
    Ok((out, dropped.iter().map(|d| format!("dropped row {d}: no text")).collect()))
}

fn first_route(r: &Row, kill: bool) -> &'static str {
    if r.done {
        return "DONE";
    }
    if r.skip {
        return "SKIP";
    }
    let kind = r.kind.as_str();
    if USER_ONLY.contains(&kind) {
        return "YOU";
    }
    if kind == "decision" || kind == "value" {
        return "ASK";
    }
    if kind == "optional" {
        return if r.who_you { "YOU" } else { "ASK" };
    }
    if kind == "task" {
        if !r.stop {
            return "KEEP";
        }
        return if kill { "DO" } else { "ASK" };
    }
    if r.risk && !r.asked {
        return "ASK";
    }
    "DO"
}

fn priority(r: &Row) -> u8 {
    match r.kind.as_str() {
        "optional" => 2,
        "task" => 1,
        _ => 0,
    }
}

struct Res {
    routes: Vec<(String, String, Py)>, // id, route, text
    do_ids: Vec<String>,
    ask: Vec<String>,
    default: Vec<String>,
    you: Vec<String>,
    then: &'static str,
    dropped: Vec<String>,
}

fn route(rows_py: &Py, kill: bool, cap: usize) -> Result<Res, String> {
    let (rows, dropped) = validate(rows_py)?;
    let mut out: HashMap<String, &'static str> = HashMap::new();
    for r in &rows {
        out.insert(r.id.clone(), first_route(r, kill));
    }
    let closed = ["DONE", "SKIP"];
    let mut changed = true;
    while changed {
        changed = false;
        for r in &rows {
            let deps: Vec<&str> = r.after.iter().map(|d| out[d]).collect();
            let cur = out[&r.id];
            let blocked = (cur == "DO" && deps.iter().any(|d| !closed.contains(d)))
                || (cur == "ASK" && (deps.contains(&"ASK") || deps.contains(&"WAIT")));
            if blocked {
                out.insert(r.id.clone(), "WAIT");
                changed = true;
            }
        }
    }
    let mut ask: Vec<&Row> = rows.iter().filter(|r| out[&r.id] == "ASK").collect();
    ask.sort_by_key(|r| priority(r)); // stable: input order breaks ties
    let split = cap.min(ask.len());
    let (asked, defaulted) = ask.split_at(split);
    for r in defaulted {
        out.insert(r.id.clone(), "DEFAULT");
    }
    let do_ids: Vec<String> = rows.iter().filter(|r| out[&r.id] == "DO").map(|r| r.id.clone()).collect();
    let waits_on_do = rows.iter().any(|r| out[&r.id] == "WAIT" && r.after.iter().any(|d| out[d] == "DO"));
    let then = if !asked.is_empty() || waits_on_do { "rerun" } else { "mic-drop" };
    Ok(Res {
        routes: rows.iter().map(|r| (r.id.clone(), out[&r.id].to_string(), r.text.clone())).collect(),
        do_ids,
        ask: asked.iter().map(|r| r.id.clone()).collect(),
        default: defaulted.iter().map(|r| r.id.clone()).collect(),
        you: rows.iter().filter(|r| out[&r.id] == "YOU").map(|r| r.id.clone()).collect(),
        then,
        dropped,
    })
}

fn pad_to(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width { s.to_string() } else { format!("{}{}", s, " ".repeat(width - n)) }
}

fn render(res: &Res) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (id, rt, text) in &res.routes {
        lines.push(format!("{} {} {}", pad_to(rt, 7), pad_to(id, 4), py_str(text)));
    }
    lines.extend(res.dropped.iter().cloned());
    lines.push(String::new());
    let do_s = if res.do_ids.is_empty() { "nothing".to_string() } else { res.do_ids.join(", ") };
    lines.push(format!("NEXT 1 do now: {do_s}"));
    if res.ask.is_empty() {
        lines.push("NEXT 2 ps:askme: nothing to ask".to_string());
    } else {
        let parts: Vec<String> = res
            .ask
            .iter()
            .map(|i| {
                let text = res.routes.iter().find(|(id, _, _)| id == i).map(|(_, _, t)| py_str(t)).unwrap_or_default();
                format!("{} {}", i, text.replace(';', ","))
            })
            .collect();
        lines.push(format!("NEXT 2 ps:askme: whatsleft: {}", parts.join("; ")));
    }
    if !res.default.is_empty() {
        lines.push(format!("       your pick, named in one line: {}", res.default.join(", ")));
    }
    if res.then == "mic-drop" {
        lines.push("NEXT 3 run ps:mic-drop".to_string());
    } else {
        lines.push("NEXT 3 update the rows (answers, done), run route again".to_string());
    }
    lines.join("\n")
}

fn res_json(res: &Res) -> String {
    let strs = |v: &[String]| Py::List(v.iter().map(|s| Py::Str(s.clone())).collect());
    let routes = Py::List(
        res.routes
            .iter()
            .map(|(id, rt, text)| {
                Py::Dict(vec![
                    ("id".into(), Py::Str(id.clone())),
                    ("route".into(), Py::Str(rt.clone())),
                    ("text".into(), text.clone()),
                ])
            })
            .collect(),
    );
    let doc = Py::Dict(vec![
        ("routes".into(), routes),
        ("do".into(), strs(&res.do_ids)),
        ("ask".into(), strs(&res.ask)),
        ("default".into(), strs(&res.default)),
        ("you".into(), strs(&res.you)),
        ("then".into(), Py::Str(res.then.to_string())),
        ("dropped".into(), strs(&res.dropped)),
    ]);
    dumps(&doc, 0)
}

// ---------------------------------------------------------------------------
// argparse, for this one parser: usage errors read like Python's.
// ---------------------------------------------------------------------------

enum ArgExit {
    Help,
    Error(String),
}

#[derive(Default, Debug)]
struct Args {
    file: Option<String>,
    kill: bool,
    cap: usize,
    json: bool,
}

/// int(v) as Python reads it, for a value that must end up in a usize.
/// Ok(None) means a negative number.
fn py_int(v: &str) -> Option<(bool, String)> {
    let t = py_strip(v);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    if !digits.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return None;
    }
    Some((neg, digits.replace('_', "")))
}

fn positive_int(v: &str) -> Result<usize, String> {
    match py_int(v) {
        None => Err(format!("invalid positive_int value: {}", repr_str(v))),
        Some((neg, digits)) => {
            let trimmed = digits.trim_start_matches('0');
            if neg || trimmed.is_empty() {
                return Err("must be 1 or more".into());
            }
            Ok(trimmed.parse::<usize>().unwrap_or(usize::MAX))
        }
    }
}

// Option names in argparse's registration order: -h, --help, --kill, --cap, --json.
const OPTS: &[(&str, usize)] = &[("-h", 0), ("--help", 0), ("--kill", 1), ("--cap", 2), ("--json", 3)];

fn action_name(idx: usize) -> &'static str {
    ["-h/--help", "--kill", "--cap", "--json"][idx]
}

fn parse_args(argv: &[String]) -> Result<Args, ArgExit> {
    let mut items: Vec<Item> = Vec::new();
    let mut after_dd = false;
    for a in argv {
        if after_dd {
            items.push(Item::Pos);
        } else if a == "--" {
            items.push(Item::Dash);
            after_dd = true;
        } else {
            items.push(classify_arg(a, OPTS).map_err(ArgExit::Error)?);
        }
    }
    let mut args = Args { cap: CAP, ..Default::default() };
    let mut extras: Vec<String> = Vec::new();
    let mut pos_taken = false;
    let n = argv.len();
    let mut i = 0;
    while i < n {
        match &items[i] {
            Item::Pos | Item::Dash => {
                if pos_taken {
                    extras.push(argv[i].clone());
                    i += 1;
                } else {
                    // -*A?-*
                    pos_taken = true;
                    while i < n && matches!(items[i], Item::Dash) {
                        i += 1;
                    }
                    if i < n && matches!(items[i], Item::Pos) {
                        args.file = Some(argv[i].clone());
                        i += 1;
                    }
                    while i < n && matches!(items[i], Item::Dash) {
                        i += 1;
                    }
                }
            }
            Item::Opt(None, _) => {
                extras.push(argv[i].clone());
                i += 1;
            }
            Item::Opt(Some(idx), explicit) => {
                let idx = *idx;
                let mut explicit = explicit.clone();
                let single_dash = !argv[i].starts_with("--");
                i += 1;
                // flags: run now (help exits); explicit arguments are rejected
                if idx != 2 {
                    let reject = |ex: &str| {
                        ArgExit::Error(format!(
                            "argument {}: ignored explicit argument {}",
                            action_name(idx),
                            repr_str(ex)
                        ))
                    };
                    if let Some(mut ex) = explicit.take() {
                        if !(single_dash && idx == 0) {
                            return Err(reject(&ex));
                        }
                        // -hX: X may be more short flags; -h is the only one
                        while !ex.is_empty() {
                            if !ex.starts_with('h') {
                                return Err(reject(&ex));
                            }
                            ex = ex.chars().skip(1).collect();
                        }
                    }
                    match idx {
                        0 => return Err(ArgExit::Help),
                        1 => args.kill = true,
                        _ => args.json = true,
                    }
                    continue;
                }
                // --cap: one value, explicit or the next argument
                let value = if let Some(v) = explicit.take() {
                    v
                } else {
                    // an optional's value must be the very next plain argument
                    if i < n && matches!(items[i], Item::Pos) {
                        let v = argv[i].clone();
                        i += 1;
                        v
                    } else {
                        return Err(ArgExit::Error("argument --cap: expected one argument".into()));
                    }
                };
                args.cap = positive_int(&value).map_err(|m| ArgExit::Error(format!("argument --cap: {m}")))?;
            }
        }
    }
    if !extras.is_empty() {
        return Err(ArgExit::Error(format!("unrecognized arguments: {}", extras.join(" "))));
    }
    Ok(args)
}

// ---------------------------------------------------------------------------
// The CLI, with the streams passed in so tests can drive it.
// ---------------------------------------------------------------------------

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(argv: &[String], stdin: &mut dyn Read) -> Outcome {
    let a = match parse_args(argv) {
        Ok(a) => a,
        Err(ArgExit::Help) => return Outcome { code: 0, stdout: HELP.into(), stderr: String::new() },
        Err(ArgExit::Error(m)) => {
            return Outcome { code: 2, stdout: String::new(), stderr: format!("{USAGE}{PROG}: error: {m}\n") };
        }
    };
    let bad_input =
        |m: String| Outcome { code: 2, stdout: format!("route.py: bad input: {m}\n"), stderr: String::new() };
    let (bytes, from_file) = match a.file.as_deref() {
        Some(path) if !path.is_empty() => match std::fs::read(path) {
            Ok(b) => (b, true),
            Err(e) => return bad_input(os_error_text(&e, path)),
        },
        _ => {
            let mut b = Vec::new();
            let _ = stdin.read_to_end(&mut b);
            (b, false)
        }
    };
    let text = match decode_text(&bytes, from_file) {
        Ok(t) => t,
        Err(m) => return bad_input(m),
    };
    let rows = match json_loads(&text) {
        Ok(v) => v,
        Err(m) => return bad_input(m),
    };
    let res = match route(&rows, a.kill, a.cap) {
        Ok(r) => r,
        Err(m) => return bad_input(m),
    };
    let body = if a.json { res_json(&res) } else { render(&res) };
    Outcome { code: 0, stdout: format!("{body}\n"), stderr: String::new() }
}

fn main() {
    let argv: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let o = run(&argv, &mut std::io::stdin());
    let _ = std::io::stdout().write_all(o.stdout.as_bytes());
    let _ = std::io::stderr().write_all(o.stderr.as_bytes());
    std::process::exit(o.code);
}

// ---------------------------------------------------------------------------
// Tests: one counterpart per case in test_route.py.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Py {
        json_loads(s).unwrap()
    }

    fn routes_of(res: &Res) -> Vec<(String, String)> {
        res.routes.iter().map(|(i, r, _)| (i.clone(), r.clone())).collect()
    }

    fn r(res: &Res, id: &str) -> String {
        res.routes.iter().find(|(i, _, _)| i == id).unwrap().1.clone()
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    const SCENARIO: &str = r#"[
      {"id": "m1", "kind": "work", "text": "No test for the export"},
      {"id": "m2", "kind": "work", "text": "Lint and tests after the last edit", "after": ["m1"]},
      {"id": "m3", "kind": "work", "risk": "prod-config", "text": "Set REPORTS_BUCKET in Production"},
      {"id": "m4", "kind": "work", "risk": "push", "asked": true, "text": "Push and deploy", "after": ["m2", "m3"]},
      {"id": "y1", "kind": "no-tool", "tried": "Chrome: AWS sign-in page", "text": "Sign in to AWS"},
      {"id": "o1", "kind": "optional", "text": "CSV export on invoices too"},
      {"id": "o2", "kind": "optional", "text": "README line for the export"},
      {"id": "o3", "kind": "optional", "who": "you", "text": "Speed Insights banner"},
      {"id": "t1", "kind": "task", "stop": true, "text": "dev server, idle 90m"},
      {"id": "t2", "kind": "task", "text": "test watcher, busy"},
      {"id": "d0", "kind": "work", "done": true, "text": "CSV export code"}
    ]"#;

    #[test]
    fn test_scenario() {
        let res = route(&p(SCENARIO), false, CAP).unwrap();
        assert_eq!(r(&res, "m1"), "DO");
        assert_eq!(r(&res, "m2"), "WAIT");
        assert_eq!(r(&res, "m3"), "ASK");
        assert_eq!(r(&res, "m4"), "WAIT");
        assert_eq!(r(&res, "y1"), "YOU");
        assert_eq!(r(&res, "o3"), "YOU");
        assert_eq!(r(&res, "t2"), "KEEP");
        assert_eq!(r(&res, "d0"), "DONE");
        assert_eq!(res.ask, ids(&["m3", "t1", "o1", "o2"]));
        assert_eq!(res.default, [] as [String; 0]);
        assert_eq!(res.then, "rerun");
    }

    #[test]
    fn test_cap_defaults_the_cheapest() {
        let rows = p(r#"[
          {"id": "o0", "kind": "optional", "text": "idea 0"},
          {"id": "d1", "kind": "decision", "text": "merge or PR"},
          {"id": "o1", "kind": "optional", "text": "idea 1"},
          {"id": "o2", "kind": "optional", "text": "idea 2"}]"#);
        let res = route(&rows, false, 2).unwrap();
        assert_eq!(res.ask, ids(&["d1", "o0"]));
        assert_eq!(res.default, ids(&["o1", "o2"]));
        assert_eq!(r(&res, "o2"), "DEFAULT");
    }

    #[test]
    fn test_kill_makes_stop_tasks_do() {
        let rows = p(r#"[{"id": "t1", "kind": "task", "stop": true, "text": "idle watcher"}]"#);
        assert_eq!(r(&route(&rows, false, CAP).unwrap(), "t1"), "ASK");
        assert_eq!(r(&route(&rows, true, CAP).unwrap(), "t1"), "DO");
    }

    #[test]
    fn test_nothing_to_ask_means_mic_drop() {
        let rows = p(r#"[{"id": "m1", "kind": "work", "text": "run lint"},
                         {"id": "y1", "kind": "login", "text": "gh auth login"}]"#);
        let res = route(&rows, false, CAP).unwrap();
        assert_eq!(res.ask, [] as [String; 0]);
        assert_eq!(res.then, "mic-drop");
        assert_eq!(route(&p("[]"), false, CAP).unwrap().then, "mic-drop");
    }

    #[test]
    fn test_answers_unlock() {
        // second round: the user said yes to m3, m1 and m2 are done
        let Py::List(mut rows) = p(SCENARIO) else { panic!() };
        for row in &mut rows {
            let Py::Dict(d) = row else { panic!() };
            let id = dget(d, "id").unwrap().as_str().unwrap().to_string();
            let mut set = |k: &str| {
                d.retain(|(key, _)| key != k);
                d.push((k.to_string(), Py::Bool(true)));
            };
            if id == "m1" || id == "m2" {
                set("done");
            }
            if id == "m3" {
                set("asked");
            }
            if id == "o1" || id == "o2" || id == "t1" {
                set("done");
            }
        }
        let res = route(&Py::List(rows), false, CAP).unwrap();
        assert_eq!(r(&res, "m3"), "DO");
        assert_eq!(r(&res, "m4"), "WAIT");
        assert_eq!(res.then, "rerun");
    }

    #[test]
    fn test_dependent_question_waits() {
        let mut rows = p(r#"[{"id": "v1", "kind": "value", "text": "bucket name?"},
            {"id": "e1", "kind": "work", "risk": "prod-config", "text": "set it in Production",
             "after": ["v1"]}]"#);
        let res = route(&rows, false, CAP).unwrap();
        assert_eq!(res.ask, ids(&["v1"]));
        assert_eq!(r(&res, "e1"), "WAIT");
        if let Py::List(l) = &mut rows
            && let Py::Dict(d) = &mut l[0]
        {
            d.push(("done".into(), Py::Bool(true)));
        }
        assert_eq!(route(&rows, false, CAP).unwrap().ask, ids(&["e1"]));
    }

    #[test]
    fn test_wait_chain_is_order_free() {
        let a = r#"{"id": "a", "kind": "decision", "text": "merge or PR"}"#;
        let b = r#"{"id": "b", "kind": "decision", "text": "branch name", "after": ["a"]}"#;
        let c = r#"{"id": "c", "kind": "decision", "text": "PR title", "after": ["b"]}"#;
        for order in [[a, b, c], [c, b, a], [b, c, a]] {
            let res = route(&p(&format!("[{}]", order.join(","))), false, CAP).unwrap();
            assert_eq!(res.ask, ids(&["a"]));
            let mut got = routes_of(&res);
            got.sort();
            assert_eq!(got, pairs(&[("a", "ASK"), ("b", "WAIT"), ("c", "WAIT")]));
        }
        // a DO row behind a waiting DO row waits too
        let rows = p(r#"[{"id": "d2", "kind": "work", "text": "deploy", "after": ["d1"]},
                         {"id": "d1", "kind": "work", "text": "test", "after": ["q"]},
                         {"id": "q", "kind": "decision", "text": "which env"}]"#);
        assert_eq!(
            routes_of(&route(&rows, false, CAP).unwrap()),
            pairs(&[("d2", "WAIT"), ("d1", "WAIT"), ("q", "ASK")])
        );
    }

    #[test]
    fn test_cycle_is_bad_input() {
        let rows = p(r#"[{"id": "a", "kind": "work", "text": "t", "after": ["b"]},
                         {"id": "b", "kind": "work", "text": "u", "after": ["a"]}]"#);
        assert_eq!(route(&rows, false, CAP).err().unwrap(), "after forms a cycle: a -> b -> a");
        let nested = p(r#"[{"id": "a", "kind": "work", "text": "t", "after": [["b"]]}]"#);
        assert_eq!(route(&nested, false, CAP).err().unwrap(), "a: after must be a list of ids");
    }

    #[test]
    fn test_semicolon_in_text_keeps_the_handoff_line() {
        let rows = p(r#"[{"id": "d1", "kind": "decision", "text": "keep a; or drop b"},
                         {"id": "d2", "kind": "decision", "text": "merge or PR"}]"#);
        let out = render(&route(&rows, false, CAP).unwrap());
        let line = out.lines().find(|l| l.contains("whatsleft:")).unwrap();
        let parts: Vec<&str> = line.split("whatsleft: ").nth(1).unwrap().split("; ").collect();
        assert_eq!(parts, vec!["d1 keep a, or drop b", "d2 merge or PR"]);
    }

    #[test]
    fn test_skip_closes_a_row() {
        let rows = p(r#"[{"id": "o1", "kind": "optional", "text": "README line", "skip": true},
                         {"id": "m1", "kind": "work", "text": "release notes", "after": ["o1"]}]"#);
        let res = route(&rows, false, CAP).unwrap();
        assert_eq!(routes_of(&res), pairs(&[("o1", "SKIP"), ("m1", "DO")]));
        assert_eq!(res.then, "mic-drop");
    }

    #[test]
    fn test_bad_input_messages() {
        // each bad row is an error that names the row and the field
        let cases = [
            (r#"[{"id": "x", "kind": "nope", "text": "t"}]"#, "x: kind 'nope' not in"),
            (r#"[{"id": "x", "kind": [1], "text": "t"}]"#, "x: kind"),
            (r#"[{"id": "x", "text": "t"}]"#, "x: kind"),
            (r#"[{"id": "x", "kind": "work", "risk": "rm -rf", "text": "t"}]"#, "x: risk 'rm -rf' not in"),
            (r#"[{"id": "x", "kind": "work", "text": "t", "who": null}]"#, "x: who must be me or you"),
            (r#"{"id": "x"}"#, "rows must be a JSON list"),
            (r"[5]", "row 1: not an object"),
            (r#"[{"id": "ok", "kind": "work", "text": "t"}, 5]"#, "row 2: not an object"),
            (r#"[{"id": 5, "kind": "work", "text": "t"}]"#, "row 1: id must be a string"),
            (r#"[{"id": ["a"], "kind": "work", "text": "t"}]"#, "row 1: id must be a string"),
            (
                r#"[{"id": "x", "kind": "work", "text": "t"}, {"id": "x", "kind": "work", "text": "u"}]"#,
                "row 2: duplicate id x",
            ),
            (r#"[{"id": "x", "kind": "work", "text": "t", "after": ["x"]}]"#, "x: waits on itself"),
            (r#"[{"id": "x", "kind": "work", "text": "t", "after": ["zz"]}]"#, "x: after names unknown id zz"),
            (r#"[{"kind": "work", "text": "t", "after": ["gone"]}]"#, "r1: after names unknown id gone"), // the default id
            (r#"[{"id": "x", "kind": "decision", "risk": "push", "text": "t"}]"#, "x: risk only goes on kind work"),
            (r#"[{"id": "x", "kind": "no-tool", "tried": "  ", "text": "t"}]"#, "x: no-tool needs 'tried'"),
        ];
        for (src, want) in cases {
            let err = route(&p(src), false, CAP).err().unwrap_or_else(|| panic!("{src} was accepted"));
            assert!(err.starts_with(want), "{src}: {err}");
        }
        // a row with no text is dropped, never an error, and a later row cannot name it
        let res = route(&p(r#"[{"id": "x", "kind": "work", "text": ""}]"#), false, CAP).unwrap();
        assert_eq!(res.routes, [] as [(String, String, Py); 0]);
        assert_eq!(res.dropped, ["dropped row x: no text"]);
        let res = route(&p(r#"[{"id": "a", "kind": "work", "text": "t", "after": ["b"]}, {"id": "b"}]"#), false, CAP)
            .unwrap();
        assert_eq!(r(&res, "a"), "DO");
    }

    #[test]
    fn test_json_errors() {
        for src in [
            "",
            "[",
            "{",
            "{\"a\"",
            "{\"a\":1",
            "[1,]",
            "\"abc",
            "\"a\\x\"",
            "\"\\u12\"",
            "\"a\tb\"",
            "01",
            "[1] x",
            "\u{feff}[]",
            "[\n1\n,\n]",
        ] {
            assert!(json_loads(src).is_err(), "{src:?}");
        }
        assert_eq!(p("\"\\ud83d\\ude00\""), Py::Str("\u{1F600}".into()));
    }

    fn cli(args: &[&str], stdin: &str) -> Outcome {
        let argv: Vec<String> = args.iter().map(ToString::to_string).collect();
        run(&argv, &mut stdin.as_bytes())
    }

    fn temp_file(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn test_row_without_id_gets_a_default_id() {
        let rows = p(r#"[{"id": "r2", "kind": "work", "text": "taken"},
                         {"kind": "work", "text": "no id, place 2"},
                         {"id": "", "kind": "decision", "text": "empty id, place 3"},
                         {"id": null, "kind": "work", "text": "null id, place 4"}]"#);
        let res = route(&rows, false, CAP).unwrap();
        // place 2 would be r2, which is taken: next free is r3; then r3 is taken too
        let got: Vec<&str> = res.routes.iter().map(|(i, _, _)| i.as_str()).collect();
        assert_eq!(got, ["r2", "r3", "r4", "r5"]);
        // the caller's rows are not changed
        let Py::List(l) = &rows else { panic!() };
        let Py::Dict(d) = &l[1] else { panic!() };
        assert!(dget(d, "id").is_none());
    }

    #[test]
    fn test_row_without_text_is_dropped_and_named() {
        let rows = p(r#"[{"id": "a", "kind": "work", "text": "build"},
                         {"kind": "work"},
                         {"id": "c", "kind": "nope", "text": ""},
                         {"id": "d", "kind": "work", "text": "   ", "after": ["a"]},
                         {"id": "e", "kind": "work", "text": "ship", "after": ["d"]}]"#);
        let res = route(&rows, false, CAP).unwrap();
        let got: Vec<&str> = res.routes.iter().map(|(i, _, _)| i.as_str()).collect();
        assert_eq!(got, ["a", "e"]);
        assert_eq!(res.dropped, ["dropped row r2: no text", "dropped row c: no text", "dropped row d: no text"]);
        assert_eq!(r(&res, "e"), "DO"); // a dropped row is not waited on
        let out = render(&res);
        assert_eq!(out.matches("dropped row c: no text").count(), 1);
    }

    #[test]
    fn test_cli_json_lists_the_dropped_rows() {
        let o = cli(&["--json"], r#"[{"id":"a","kind":"work","text":"x"},{"id":"b"}]"#);
        assert_eq!(o.code, 0);
        assert!(
            o.stdout.ends_with("  \"then\": \"mic-drop\",\n  \"dropped\": [\n    \"dropped row b: no text\"\n  ]\n}\n"),
            "{}",
            o.stdout
        );
        let o = cli(&[], r#"[{"id":"a","kind":"work","text":"x"},{"id":"b"}]"#);
        assert!(o.stdout.starts_with("DO      a    x\ndropped row b: no text\n\nNEXT 1"), "{}", o.stdout);
    }

    #[test]
    fn test_cli() {
        // an empty file is not JSON
        let empty = temp_file("");
        let o = cli(&[empty.path().to_str().unwrap()], "");
        assert_eq!(o.code, 2);
        assert!(o.stdout.starts_with("route.py: bad input: "), "{}", o.stdout);

        let f = temp_file(SCENARIO);
        let path = f.path().to_str().unwrap().to_string();
        let o = cli(&[&path], "");
        assert_eq!(o.code, 0);
        assert!(o.stdout.contains("NEXT 2 ps:askme: whatsleft: m3 "));

        // a bad option value, a missing file and wrong types exit 2
        for cap in ["0", "-1"] {
            assert_eq!(cli(&["--cap", cap, &path], "").code, 2, "--cap {cap}");
        }
        let missing = std::env::temp_dir().join("no-such-rows.json");
        let o = cli(&[missing.to_str().unwrap()], "");
        assert_eq!(o.code, 2);
        assert!(o.stdout.starts_with("route.py: bad input: "), "{}", o.stdout);
        let f = temp_file(r#"[{"id": 5, "kind": "work", "text": "t"}, {"id": "x", "kind": [1], "text": "t"}]"#);
        assert_eq!(cli(&[f.path().to_str().unwrap()], "").code, 2);
    }

    #[test]
    fn test_cli_stdin_json_and_flags() {
        let o = cli(&["--json"], r#"[{"id":"a","kind":"work","text":"x"}]"#);
        assert_eq!(o.code, 0);
        let v: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
        assert_eq!(v["routes"], serde_json::json!([{"id": "a", "route": "DO", "text": "x"}]));
        assert_eq!(v["do"], serde_json::json!(["a"]));
        assert_eq!(v["then"], "mic-drop");
        let o = cli(&["--kill", "--cap=1"], r#"[{"id":"t","kind":"task","stop":true,"text":"x"}]"#);
        assert!(o.stdout.starts_with("DO      t    x\n"));
        assert_eq!(cli(&["-h"], "").code, 0);
        for bad in [&["--bogus"][..], &["--kill=1"], &["--cap"], &["--cap", "x"], &["a", "b"]] {
            let o = cli(bad, "");
            assert_eq!(o.code, 2, "{bad:?}");
            assert!(o.stderr.starts_with(USAGE), "{bad:?}: {}", o.stderr);
        }
    }
}
