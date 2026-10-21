//! Python-compat helpers shared by the bins that were ported from Python: civil date math, ISO
//! timestamps, float and string reprs, JSON text, truthiness, argparse-style option sorting and
//! small path and file helpers. One copy of each, so every bin prints the same bytes as the old
//! scripts.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

// --- civil dates (proleptic Gregorian), Howard Hinnant's algorithms -------------------------------

/// Days since 1970-01-01 of a civil date. Works for negative years.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The civil date (year, month, day) of a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Days in month `m` of year `y` (leap years included).
pub fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// `datetime.fromisoformat(s.replace("Z", "+00:00"))` as (UTC microseconds, UTC offset in seconds).
/// A missing offset counts as UTC.
pub fn parse_ts_micros(s: &str) -> Option<(i64, i64)> {
    let s = s.replace('Z', "+00:00");
    let b: Vec<char> = s.chars().collect();
    let num_at = |a: usize, n: usize| -> Option<i64> {
        let t: String = b.get(a..a + n)?.iter().collect();
        if t.chars().all(|c| c.is_ascii_digit()) { t.parse().ok() } else { None }
    };
    if b.len() < 10 || b[4] != '-' || b[7] != '-' {
        return None;
    }
    let (y, mo, d) = (num_at(0, 4)?, num_at(5, 2)?, num_at(8, 2)?);
    if !(1..=12).contains(&mo) || y < 1 || d < 1 || d > days_in_month(y, mo) {
        return None;
    }
    let (mut hh, mut mm, mut ss, mut us, mut off) = (0i64, 0i64, 0i64, 0i64, 0i64);
    if b.len() > 10 {
        let rest: Vec<char> = b[11..].to_vec();
        let tz_at = rest.iter().position(|c| *c == '+' || *c == '-');
        let (time, tz) = match tz_at {
            Some(i) => (&rest[..i], Some(&rest[i..])),
            None => (&rest[..], None),
        };
        let ts: String = time.iter().collect();
        let (hms, frac) = match ts.split_once('.') {
            Some((a, f)) => (a.to_string(), Some(f.to_string())),
            None => (ts.clone(), None),
        };
        let parts: Vec<&str> = hms.split(':').collect();
        let pn = |x: &str| -> Option<i64> {
            if x.len() == 2 && x.chars().all(|c| c.is_ascii_digit()) { x.parse().ok() } else { None }
        };
        hh = pn(parts.first()?)?;
        if parts.len() > 1 {
            mm = pn(parts[1])?;
        }
        if parts.len() > 2 {
            ss = pn(parts[2])?;
        }
        if parts.len() > 3 {
            return None;
        }
        if let Some(f) = frac {
            if f.is_empty() || f.len() > 9 || !f.chars().all(|c| c.is_ascii_digit()) || parts.len() < 3 {
                return None;
            }
            let mut p6 = f.clone();
            p6.truncate(6);
            while p6.len() < 6 {
                p6.push('0');
            }
            us = p6.parse().ok()?;
        }
        if hh > 23 || mm > 59 || ss > 59 {
            return None;
        }
        if let Some(tz) = tz {
            let sign = if tz[0] == '-' { -1 } else { 1 };
            let t: String = tz[1..].iter().collect();
            let p: Vec<&str> = t.split(':').collect();
            if p.len() < 2 || p.len() > 3 {
                return None;
            }
            let (th, tm) = (pn(p[0])?, pn(p[1])?);
            let tsec = if p.len() == 3 { pn(p[2])? } else { 0 };
            if th > 23 || tm > 59 || tsec > 59 {
                return None;
            }
            off = sign * (th * 3600 + tm * 60 + tsec);
        }
    }
    let secs = days_from_civil(y, mo, d) * 86400 + hh * 3600 + mm * 60 + ss - off;
    Some((secs * 1_000_000 + us, off))
}

/// `datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()`; a missing offset counts as UTC.
pub fn parse_ts(s: &str) -> Option<f64> {
    parse_ts_micros(s).map(|(micros, _)| micros as f64 / 1e6)
}

// --- numbers and truthiness -----------------------------------------------------------------------

/// Python `repr(float)`, JSON style: `NaN`, `Infinity` and `-Infinity` for the specials.
pub fn py_float(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    if !f.is_finite() {
        return if f.is_nan() {
            "NaN".into()
        } else if f > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let e = format!("{:e}", f.abs());
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    let body = if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                format!("{}{}.0", digits, "0".repeat(int_len - digits.len()))
            } else {
                format!("{}.{}", &digits[..int_len], &digits[int_len..])
            }
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        }
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.clone() };
        format!("{}e{}{:02}", m, if exp < 0 { "-" } else { "+" }, exp.abs())
    };
    format!("{}{}", sign, body)
}

/// Python truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_none_or(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

// --- JSON text ------------------------------------------------------------------------------------

/// json.dumps of a string with ensure_ascii, appended to `out`. DEL (0x7f) is escaped, like Python.
pub fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// json.dumps of a string (ensure_ascii), as a new String.
pub fn json_string(s: &str) -> String {
    let mut out = String::new();
    json_str(&mut out, s);
    out
}

// --- Python repr and str --------------------------------------------------------------------------

/// Python `repr()` of a str for the bins that keep non-ASCII text as is: only control characters
/// and DEL become `\x..`.
pub fn py_repr_str(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut o = String::new();
    o.push(q);
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if c == q => {
                o.push('\\');
                o.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => o.push_str(&format!("\\x{:02x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push(q);
    o
}

/// Python `repr(value)` of a JSON value.
pub fn py_repr(v: &Value) -> String {
    match v {
        Value::String(s) => py_repr_str(s),
        Value::Array(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => {
            format!(
                "{{{}}}",
                o.iter().map(|(k, x)| format!("{}: {}", py_repr_str(k), py_repr(x))).collect::<Vec<_>>().join(", ")
            )
        }
        other => py_str(other),
    }
}

/// Python str(value) of a JSON value.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                py_float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => s.clone(),
        other => py_repr(other),
    }
}

/// Python `str.isprintable()` for one char (an approximation of the Unicode table).
pub fn is_printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    if c.is_control() || c.is_whitespace() {
        return false;
    }
    let u = c as u32;
    // Format (Cf), private use (Co) and non-characters: not printable in Python.
    !(matches!(u, 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E)
        || (0x200B..=0x200F).contains(&u)
        || (0x202A..=0x202E).contains(&u)
        || (0x2060..=0x206F).contains(&u)
        || u == 0xFEFF
        || (0xFFF9..=0xFFFB).contains(&u)
        || (0xE000..=0xF8FF).contains(&u)
        || u >= 0xF0000
        || (u & 0xFFFE) == 0xFFFE
        || (0xFDD0..=0xFDEF).contains(&u))
}

/// Python's repr() of a str, with `\x..`, `\u....` and `\U........` escapes for unprintable chars.
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if is_printable(c) => out.push(c),
            c => {
                let u = c as u32;
                if u < 0x100 {
                    out.push_str(&format!("\\x{:02x}", u));
                } else if u < 0x10000 {
                    out.push_str(&format!("\\u{:04x}", u));
                } else {
                    out.push_str(&format!("\\U{:08x}", u));
                }
            }
        }
    }
    out.push(quote);
    out
}

// --- argparse-style option sorting ----------------------------------------------------------------

/// One command-line word, sorted the way argparse does it.
#[derive(Debug, PartialEq)]
pub enum Item {
    /// A positional word.
    Pos,
    /// A lone dash.
    Dash,
    /// An option: action index (None: unknown option), explicit argument
    Opt(Option<usize>, Option<String>),
}

/// argparse's `^-\d+$|^-\d*\.\d+$` test for a negative number.
pub fn negative_number_like(a: &str) -> bool {
    let Some(r) = a.strip_prefix('-') else { return false };
    if !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    match r.split_once('.') {
        Some((a, b)) => a.chars().all(|c| c.is_ascii_digit()) && !b.is_empty() && b.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// Sort one word into positional or option. `opts` lists the option names with their action index,
/// in argparse's registration order. Err is the "ambiguous option" message.
pub fn classify_arg(a: &str, opts: &[(&str, usize)]) -> Result<Item, String> {
    let exact = |s: &str| opts.iter().find(|(n, _)| *n == s).map(|(_, i)| *i);
    if a.is_empty() || !a.starts_with('-') {
        return Ok(Item::Pos);
    }
    if let Some(i) = exact(a) {
        return Ok(Item::Opt(Some(i), None));
    }
    if a.chars().count() == 1 {
        return Ok(Item::Pos);
    }
    if let Some((o, v)) = a.split_once('=')
        && let Some(i) = exact(o)
    {
        return Ok(Item::Opt(Some(i), Some(v.to_string())));
    }
    // abbreviations and -hX
    let mut hits: Vec<(&str, usize, Option<String>)> = Vec::new();
    let second = a.chars().nth(1).unwrap();
    if second == '-' {
        let (prefix, explicit) = match a.split_once('=') {
            Some((p, v)) => (p, Some(v.to_string())),
            None => (a, None),
        };
        for (n, i) in opts {
            if n.starts_with(prefix) {
                hits.push((n, *i, explicit.clone()));
            }
        }
    } else {
        let short: String = a.chars().take(2).collect();
        let rest: String = a.chars().skip(2).collect();
        for (n, i) in opts {
            if *n == short {
                hits.push((n, *i, Some(rest.clone())));
            } else if n.starts_with(a) {
                hits.push((n, *i, None));
            }
        }
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|h| h.0).collect();
        return Err(format!("ambiguous option: {} could match {}", a, names.join(", ")));
    }
    if let Some((_, i, explicit)) = hits.pop() {
        return Ok(Item::Opt(Some(i), explicit));
    }
    if negative_number_like(a) || a.contains(' ') {
        return Ok(Item::Pos);
    }
    Ok(Item::Opt(None, None))
}

// --- whitespace and lines -------------------------------------------------------------------------

/// Python's `str.isspace` for one char (Unicode White_Space plus U+001C to U+001F).
pub fn py_is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// Python's `str.strip()`: trims the `py_is_space` chars from both ends.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(py_is_space)
}

/// Python's `str.splitlines()`: splits on more than `\n`, and drops a last empty line.
pub fn py_splitlines(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            '\n' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}' => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// --- paths ----------------------------------------------------------------------------------------

/// os.path.dirname of a "/"-separated path.
pub fn dirname(p: &str) -> String {
    match p.rfind('/') {
        None => String::new(),
        Some(i) => {
            let head = &p[..=i];
            let t = head.trim_end_matches('/');
            if t.is_empty() { head.to_string() } else { t.to_string() }
        }
    }
}

/// os.path.basename of a "/"-separated path.
pub fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or("").to_string()
}

/// The last component of a path as text, or "".
pub fn fname(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Entries of `dir` whose name does not start with "." (glob "*"), in directory order.
pub fn list_dir(dir: &Path) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| e.path())
            .collect(),
        Err(_) => vec![],
    }
}

/// Modification time of a path in seconds since the epoch, or None.
pub fn mtime(p: &Path) -> Option<f64> {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
}

/// A regular file that can be run. On non-unix any regular file counts.
#[cfg(unix)]
pub fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// A regular file that can be run. On non-unix any regular file counts.
#[cfg(not(unix))]
pub fn is_executable(p: &Path) -> bool {
    p.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn civil_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(days_from_civil(2026, 10, 19), 20745);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(20745), (2026, 10, 19));
    }

    #[test]
    fn civil_round_trip() {
        let mut z = 0; // 1970-01-01
        while z < days_from_civil(2100, 1, 1) {
            let (y, m, d) = civil_from_days(z);
            assert!((1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m), "z={z}");
            assert_eq!(days_from_civil(y, m, d), z);
            z += 29;
        }
    }

    #[test]
    fn month_lengths() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2023, 4), 30);
        assert_eq!(days_in_month(2023, 12), 31);
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_ts("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(parse_ts("1970-01-01T00:00:01.5+01:00"), Some(-3598.5));
        assert_eq!(parse_ts("2026-10-19"), Some(20745.0 * 86400.0));
        assert_eq!(parse_ts("2026-10-19T12:34:56"), Some(1792413296.0));
        assert_eq!(parse_ts("2026-10-19 12:34:56+00:00"), Some(1792413296.0));
        assert_eq!(parse_ts("2026-02-30"), None);
        assert_eq!(parse_ts("0000-01-01"), None);
        assert_eq!(parse_ts(""), None);
        assert_eq!(parse_ts("not a date"), None);
        assert_eq!(parse_ts("2026-10-19T24:00:00"), None);
        assert_eq!(parse_ts("2026-10-19T12:00.5"), None);
        assert_eq!(parse_ts("2026-10-19T12:00:00.1234567890"), None);
        assert_eq!(parse_ts("2026-10-19T12:00:00+25:00"), None);
    }

    #[test]
    fn timestamp_micros_and_offset() {
        assert_eq!(parse_ts_micros("1970-01-01T00:00:00-05:30"), Some((19_800_000_000, -19_800)));
        assert_eq!(parse_ts_micros("1970-01-01T00:00:00.1234567Z"), Some((123_456, 0)));
        assert_eq!(parse_ts_micros("1970-01-01T00:00:00+00:00:30"), Some((-30_000_000, 30)));
        assert_eq!(parse_ts_micros("1970-01-01"), Some((0, 0)));
    }

    #[test]
    fn floats() {
        assert_eq!(py_float(0.0), "0.0");
        assert_eq!(py_float(40.0), "40.0");
        assert_eq!(py_float(12.3), "12.3");
        assert_eq!(py_float(-1234.5), "-1234.5");
        assert_eq!(py_float(0.0001), "0.0001");
        assert_eq!(py_float(0.0255), "0.0255");
        assert_eq!(py_float(1e15), "1000000000000000.0");
        assert_eq!(py_float(1e16), "1e+16");
        assert_eq!(py_float(1.5e-7), "1.5e-07");
    }

    #[test]
    fn truthiness() {
        assert!(!truthy(&json!(null)));
        assert!(!truthy(&json!(false)));
        assert!(truthy(&json!(true)));
        assert!(!truthy(&json!(0)));
        assert!(!truthy(&json!(0.0)));
        assert!(truthy(&json!(-1)));
        assert!(truthy(&json!(0.5)));
        assert!(!truthy(&json!("")));
        assert!(truthy(&json!("0")));
        assert!(!truthy(&json!([])));
        assert!(truthy(&json!([0])));
        assert!(!truthy(&json!({})));
        assert!(truthy(&json!({"a": null})));
    }

    #[test]
    fn json_text() {
        assert_eq!(json_string(""), "\"\"");
        assert_eq!(json_string("a\u{e9}\"b"), "\"a\\u00e9\\\"b\"");
        assert_eq!(json_string("\u{7f}"), "\"\\u007f\"");
        assert_eq!(json_string("\u{1}\u{8}\u{c}\n\r\t\\"), "\"\\u0001\\b\\f\\n\\r\\t\\\\\"");
        assert_eq!(json_string("\u{1F600}"), "\"\\ud83d\\ude00\"");
        let mut out = String::from("x=");
        json_str(&mut out, "k");
        assert_eq!(out, "x=\"k\"");
    }

    #[test]
    fn printable_and_str_repr() {
        assert!(is_printable(' '));
        assert!(is_printable('a'));
        assert!(is_printable('\u{1F600}'));
        assert!(!is_printable('\t'));
        assert!(!is_printable('\u{7f}'));
        assert!(!is_printable('\u{ad}'));
        assert!(!is_printable('\u{feff}'));
        assert!(!is_printable('\u{fffe}'));
        assert_eq!(repr_str(""), "''");
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(repr_str("\u{7f}"), "'\\x7f'");
        assert_eq!(repr_str("\u{200b}"), "'\\u200b'");
        assert_eq!(repr_str("\u{e000}"), "'\\ue000'");
        assert_eq!(repr_str("\u{f0000}"), "'\\U000f0000'");
        assert_eq!(repr_str("\u{1F600}\u{e9}"), "'\u{1F600}\u{e9}'");
    }

    const OPTS: &[(&str, usize)] = &[("-h", 0), ("--help", 0), ("--prompt-file", 1), ("--prompt", 2), ("--dir", 3)];

    #[test]
    fn option_sorting() {
        assert_eq!(classify_arg("", OPTS), Ok(Item::Pos));
        assert_eq!(classify_arg("word", OPTS), Ok(Item::Pos));
        assert_eq!(classify_arg("-", OPTS), Ok(Item::Pos));
        assert_eq!(classify_arg("--dir", OPTS), Ok(Item::Opt(Some(3), None)));
        assert_eq!(classify_arg("--dir=x", OPTS), Ok(Item::Opt(Some(3), Some("x".into()))));
        assert_eq!(classify_arg("--di", OPTS), Ok(Item::Opt(Some(3), None)));
        assert_eq!(classify_arg("-hx", OPTS), Ok(Item::Opt(Some(0), Some("x".into()))));
        assert_eq!(classify_arg("--prompt", OPTS), Ok(Item::Opt(Some(2), None)));
        assert_eq!(
            classify_arg("--pro", OPTS),
            Err("ambiguous option: --pro could match --prompt-file, --prompt".to_string())
        );
        assert_eq!(classify_arg("--nope", OPTS), Ok(Item::Opt(None, None)));
    }

    #[test]
    fn python_whitespace() {
        for c in ['\t', '\n', '\r', ' ', '\u{a0}'] {
            assert!(py_is_space(c), "{c:?}");
        }
        for c in ['a', '0', '\0', '\u{200b}'] {
            assert!(!py_is_space(c), "{c:?}");
        }
        assert_eq!(py_strip("  a b \n"), "a b");
        assert_eq!(py_strip(" \t "), "");
    }

    #[test]
    fn python_splitlines() {
        assert_eq!(py_splitlines("a\r\nb\rc\x0cd\n"), vec!["a", "b", "c", "d"]);
        assert_eq!(py_splitlines("a\nb\r\nc\n"), vec!["a", "b", "c"]);
        assert_eq!(py_splitlines("a\n\nb"), vec!["a", "", "b"]);
        assert_eq!(py_splitlines("a\u{2028}b\u{85}c"), vec!["a", "b", "c"]);
        assert_eq!(py_splitlines(""), Vec::<String>::new());
    }

    #[test]
    fn path_text() {
        assert_eq!(dirname("a/b/c"), "a/b");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("a"), "");
        assert_eq!(dirname(""), "");
        assert_eq!(dirname("/"), "/");
        assert_eq!(dirname("a/"), "a");
        assert_eq!(basename("a/b"), "b");
        assert_eq!(basename("a/"), "");
        assert_eq!(basename(""), "");
        assert_eq!(basename("plain"), "plain");
        assert_eq!(fname(Path::new("/x/y.txt")), "y.txt");
        assert_eq!(fname(Path::new("/")), "");
        assert_eq!(fname(Path::new("")), "");
    }

    #[test]
    fn dir_listing_and_times() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.txt"), "x").unwrap();
        fs::write(d.path().join(".hidden"), "x").unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        let mut names: Vec<String> = list_dir(d.path()).iter().map(|p| fname(p)).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "sub"]);
        assert!(list_dir(&d.path().join("missing")).is_empty());
        assert!(mtime(&d.path().join("a.txt")).is_some_and(|t| t > 0.0));
        assert_eq!(mtime(&d.path().join("missing")), None);
    }

    #[test]
    fn executable_files() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("tool");
        fs::write(&f, "#!/bin/sh\n").unwrap();
        assert!(!is_executable(&d.path().join("missing")));
        assert!(!is_executable(d.path()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(!is_executable(&f));
            fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(is_executable(&f));
        }
        #[cfg(not(unix))]
        assert!(is_executable(&f));
    }
}
