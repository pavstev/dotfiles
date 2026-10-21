//! Tests for the `ctx` bin: `ctx outline FILE` and `ctx read FILE SYMBOL [--if-changed HASH]`.
//!
//! Messages (refusals, not found) go to stdout, so an agent sees them in the same stream as the
//! code. Only a usage error writes to stderr (exit 2).

mod common;

use common::{run_bin, temp_home};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

const CTX: &str = env!("CARGO_BIN_EXE_ctx");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codeview").join(name)
}

fn ctx(args: &[&str]) -> (i32, String, String) {
    run_bin(CTX, args, "", &[], None)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A temp file with the given bytes.
fn temp_file(tag: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let dir = temp_home(tag);
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn outline_prints_the_golden() {
    let (code, out, _) = ctx(&["outline", &s(&fixture("nested.ts"))]);
    assert_eq!(code, 0);
    assert_eq!(out, fs::read_to_string(fixture("nested.ts.outline")).unwrap());
}

#[test]
fn outline_of_unknown_extension_says_no_scanner() {
    let path = fixture("unknown.xyz");
    let (code, out, _) = ctx(&["outline", &s(&path)]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no scanner"), "{out}");
    assert!(out.contains("3 lines"), "{out}");
    // read refuses it too
    let (code, out, _) = ctx(&["read", &s(&path), "x"]);
    assert_eq!(code, 1);
    assert!(out.contains("no scanner"), "{out}");
}

#[test]
fn outline_of_a_file_with_no_symbols_says_so() {
    let (code, out, _) = ctx(&["outline", &s(&fixture("empty.ts"))]);
    assert_eq!(code, 0);
    assert!(out.contains("no symbols"), "{out}");
}

#[test]
fn read_prints_header_and_numbered_lines() {
    let path = fixture("nested.ts");
    let (code, out, _) = ctx(&["read", &s(&path), "outer.inner"]);
    assert_eq!(code, 0, "{out}");
    let mut lines = out.lines();
    let header = lines.next().unwrap();
    let re = Regex::new(r"^.+nested\.ts:(\d+)-(\d+)  outer\.inner  scan=c-like  hash=[0-9a-f]{8}$").unwrap();
    let caps = re.captures(header).unwrap_or_else(|| panic!("bad header: {header}"));
    let (start, end): (usize, usize) = (caps[1].parse().unwrap(), caps[2].parse().unwrap());
    assert_eq!((start, end), (6, 11));
    let src = fs::read_to_string(&path).unwrap();
    let src: Vec<&str> = src.lines().collect();
    let body: Vec<&str> = lines.collect();
    assert_eq!(body.len(), end - start + 1);
    for (i, line) in body.iter().enumerate() {
        let n = start + i;
        assert_eq!(*line, format!("{n}\t{}", src[n - 1]));
    }
}

#[test]
fn read_accepts_a_bare_name_when_it_is_unique() {
    let (code, out, _) = ctx(&["read", &s(&fixture("nested.ts")), "innermost"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.lines().next().unwrap().contains("  outer.inner.innermost  "), "{out}");
}

#[test]
fn read_if_changed_same_hash_says_unchanged() {
    let path = s(&fixture("nested.ts"));
    let (_, out, _) = ctx(&["read", &path, "sibling"]);
    let header = out.lines().next().unwrap();
    let hash = header.rsplit("hash=").next().unwrap().to_string();
    assert_eq!(hash.len(), 8);

    let (code, same, _) = ctx(&["read", &path, "sibling", "--if-changed", &hash]);
    assert_eq!(code, 0);
    assert_eq!(same.trim_end(), format!("unchanged (hash {hash})"));

    // Another hash prints the symbol as usual.
    let (code, other, _) = ctx(&["read", &path, "sibling", "--if-changed", "00000000"]);
    assert_eq!(code, 0);
    assert_eq!(other, out);
}

#[test]
fn hash_follows_the_file_bytes() {
    let a = temp_file("ctx-hash", "a.ts", b"function f() {\n}\n");
    let b = temp_file("ctx-hash", "b.ts", b"function f() {\n}\n\n");
    let hash = |p: &Path| {
        let (_, out, _) = ctx(&["read", &s(p), "f"]);
        out.lines().next().unwrap().rsplit("hash=").next().unwrap().to_string()
    };
    assert_eq!(hash(&a), hash(&a));
    assert_ne!(hash(&a), hash(&b));
}

#[test]
fn read_not_found_lists_closest() {
    let (code, out, _) = ctx(&["read", &s(&fixture("nested.ts")), "innr"]);
    assert_eq!(code, 1);
    assert!(out.contains("not found"), "{out}");
    let listed: Vec<&str> = out.lines().filter(|l| l.starts_with("  ")).collect();
    assert!(!listed.is_empty() && listed.len() <= 5, "{out}");
    assert!(listed[0].contains("inner"), "closest first: {out}");
}

#[test]
fn read_ambiguous_lists_all() {
    let src = "function dup() {\n  return 1;\n}\n\nfunction dup() {\n  return 2;\n}\n";
    let path = temp_file("ctx-dup", "dup.ts", src.as_bytes());
    let (code, out, _) = ctx(&["read", &s(&path), "dup"]);
    assert_eq!(code, 1);
    assert!(out.contains("ambiguous"), "{out}");
    assert!(out.contains(":1-3"), "{out}");
    assert!(out.contains(":5-7"), "{out}");
}

#[test]
fn binary_file_refused() {
    let nul = temp_file("ctx-bin", "bin.ts", b"function a() {\n}\0\n");
    let bad_utf8 = temp_file("ctx-bin", "latin.py", b"def a():\n    return '\xff\xfe'\n");
    for path in [nul, bad_utf8] {
        for args in [vec!["outline", &s(&path)], vec!["read", &s(&path), "a"]] {
            let (code, out, err) = ctx(&args);
            assert_eq!(code, 1, "{args:?}: {out}{err}");
            assert!(out.contains("not text"), "{args:?}: {out}{err}");
            assert!(!err.contains("panicked"), "{err}");
        }
    }
}

#[test]
fn big_and_minified_files_refused() {
    let big = temp_file("ctx-big", "big.ts", "//\n".repeat(700_000).as_bytes());
    let (code, out, _) = ctx(&["outline", &s(&big)]);
    assert_eq!(code, 1);
    assert!(out.contains("2 MB"), "{out}");

    let (code, out, _) = ctx(&["read", &s(&fixture("minified.js")), "x"]);
    assert_eq!(code, 1);
    assert!(out.contains("10,000"), "{out}");
}

#[test]
fn missing_file_is_a_one_line_refusal() {
    let (code, out, err) = ctx(&["outline", "/nonexistent/dir/nope.ts"]);
    assert_eq!(code, 1);
    assert!(out.contains("nope.ts"), "{out}");
    assert!(!err.contains("panicked"), "{err}");
}

/// `read` of a function `big` that is `body + 2` lines long.
fn read_big(body: usize) -> (i32, String) {
    let mut src = String::from("function big() {\n");
    for i in 0..body {
        src.push_str(&format!("  call({i});\n"));
    }
    src.push_str("}\n");
    let path = temp_file("ctx-cap", "big.ts", src.as_bytes());
    let (code, out, _) = ctx(&["read", &s(&path), "big"]);
    (code, out)
}

#[test]
fn output_cap() {
    // exactly 400 lines are not cut
    let (code, out) = read_big(398);
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 1 + 400);
    assert!(!out.contains("cut at"), "{out}");
    let (code, out) = read_big(500);
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].contains(":1-502  big  "), "{}", lines[0]);
    assert_eq!(lines.len(), 1 + 400 + 1, "header, 400 lines, the cut line");
    assert!(lines[400].starts_with("400\t"), "{}", lines[400]);
    assert_eq!(lines[401], "… cut at 400 of 502 lines; use Read with offset");
}

#[test]
fn usage_errors_exit_2() {
    for args in [vec![], vec!["read", "only-a-file.ts"], vec!["frobnicate"], vec!["outline"]] {
        let (code, _, err) = ctx(&args);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert_ne!(err, "");
    }
}
