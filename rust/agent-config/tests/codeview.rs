//! Edge-case tests for the `codeview` line scan (written first, before `src/codeview.rs`).
//!
//! Fixtures live in `tests/fixtures/codeview/`. Each fixture `F` has one expectation file:
//! `F.outline` (the exact `render` output, trailing newlines ignored) or `F.refusal`
//! (the refusal name: `TooBig`, `LongLine`, `NotText` or `NoScanner(<ext>)`).
//!
//! What the expectations pin down (Task 3 builds to these):
//!
//! - Lines are 1-based and inclusive. `depth` is the number of enclosing symbols.
//! - Kinds: `function`, `class`, `interface`, `type`, `enum`, `const` (arrow functions),
//!   `method` (inside a class body, TS only), `fn`, `struct`, `trait`, `impl`, `mod`,
//!   `macro_rules`, `def` (also `async def`), `h1` to `h6`, `function` (shell), `rule`
//!   (CSS, also `@media`).
//! - Names: the identifier. A Rust `impl` takes the text after its own generics, so
//!   `impl<T: Copy> Pair<T>` is `Pair<T>` and `impl std::fmt::Display for Point` is
//!   `std::fmt::Display for Point`. A CSS rule takes the prelude before its `{`. A heading
//!   takes its text without the closing `#` marks.
//! - Ends: brace families end at the matching brace. An item with no body ends on its own
//!   line (`fn area(&self) -> f64;`, `struct Unit;`, `type Id = string;`). An expression-bodied
//!   arrow ends at the line with the closing `;`. Indent and Markdown symbols end at the last
//!   non-blank, non-comment line before the next line that closes them.
//! - Decorators (TS `@x`, Python `@x`), single or multi-line, move `start` up to the first `@`
//!   line. Rust `#[attr]` lines are not covered here.

use agent_config::codeview::{Refusal, Symbol, outline, render, scan_name};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const TWO_MIB: usize = 2 * 1024 * 1024;

/// Every case from the spec. A missing fixture or expectation fails `fixture_set_is_complete`.
const CASES: [&str; 16] = [
    "nested.ts",
    "arrow.ts",
    "signatures.rs",
    "decorators.ts",
    "decorators.py",
    "strings.ts",
    "rust_items.rs",
    "nested.py",
    "fenced.md",
    "shell.sh",
    "selectors.css",
    "empty.ts",
    "minified.js",
    "crlf.ts",
    "tabs.py",
    "unknown.xyz",
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codeview")
}

/// Reads a fixture. `crlf*` fixtures are stored with LF (`.gitattributes` forces `eol=lf`)
/// and turned into CRLF here.
fn load(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let stem = path.file_name().unwrap().to_string_lossy();
    if stem.starts_with("crlf") {
        let crlf = to_crlf(&text);
        assert!(crlf.contains("\r\n"), "{stem} must hold CRLF line ends");
        return crlf;
    }
    text
}

fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn refusal_name(r: &Refusal) -> String {
    match r {
        Refusal::TooBig => "TooBig".to_string(),
        Refusal::LongLine => "LongLine".to_string(),
        Refusal::NotText => "NotText".to_string(),
        Refusal::NoScanner(ext) => format!("NoScanner({ext})"),
    }
}

/// The outline as text, or the refusal name.
fn show(path: &Path, text: &str) -> String {
    match outline(path, text) {
        Ok(symbols) => render(&symbols),
        Err(r) => refusal_name(&r),
    }
}

fn trimmed(s: &str) -> &str {
    s.trim_end_matches('\n')
}

/// `(fixture path, expectation text)` for every expectation file in the folder.
fn expectations() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir()).expect("fixtures dir") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let fixture = name
            .strip_suffix(".outline")
            .or_else(|| name.strip_suffix(".refusal"));
        if let Some(fixture) = fixture {
            let expected = fs::read_to_string(&path).unwrap();
            out.push((dir().join(fixture), expected));
        }
    }
    out.sort();
    out
}

fn sized(n: usize) -> String {
    let mut s = "//\n".repeat(n / 3);
    s.push_str(&"\n".repeat(n % 3));
    assert_eq!(s.len(), n);
    s
}

#[test]
fn fixture_set_is_complete() {
    let found: BTreeSet<String> = expectations()
        .into_iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    let wanted: BTreeSet<String> = CASES.iter().map(|s| s.to_string()).collect();
    assert_eq!(found, wanted, "expectation files and CASES differ");
    for case in CASES {
        assert!(dir().join(case).is_file(), "missing fixture {case}");
    }
}

#[test]
fn every_fixture_matches_its_outline() {
    let mut failures = Vec::new();
    for (path, expected) in expectations() {
        let text = load(&path);
        let got = show(&path, &text);
        if trimmed(&got) != trimmed(&expected) {
            failures.push(format!(
                "--- {}\nexpected:\n{}\ngot:\n{}\n",
                path.file_name().unwrap().to_string_lossy(),
                trimmed(&expected),
                trimmed(&got)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn crlf_changes_nothing() {
    for (path, _) in expectations() {
        let lf = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
        let want = show(&path, &lf);
        let got = show(&path, &to_crlf(&lf));
        assert_eq!(
            trimmed(&got),
            trimmed(&want),
            "CRLF changed the outline of {}",
            path.display()
        );
    }
}

#[test]
fn missing_final_newline_changes_nothing() {
    for (path, _) in expectations() {
        let text = load(&path);
        let cut = text.trim_end_matches(['\n', '\r']);
        assert_eq!(
            trimmed(&show(&path, cut)),
            trimmed(&show(&path, &text)),
            "a missing final newline changed the outline of {}",
            path.display()
        );
    }
}

/// Checks that the symbols are consistent, whatever the language.
fn check_shape(name: &str, text: &str, symbols: &[Symbol]) {
    let lines = text.lines().count();
    let mut stack: Vec<&Symbol> = Vec::new();
    let mut prev_start = 0;
    for s in symbols {
        assert!(
            1 <= s.start && s.start <= s.end && s.end <= lines,
            "{name}: bad range {}-{} for {} (file has {lines} lines)",
            s.start,
            s.end,
            s.name
        );
        assert!(
            !s.kind.is_empty() && !s.name.is_empty(),
            "{name}: empty kind or name"
        );
        assert!(
            s.start >= prev_start,
            "{name}: symbols not in start order at {}",
            s.name
        );
        prev_start = s.start;
        assert!(s.depth <= stack.len(), "{name}: depth jumps at {}", s.name);
        stack.truncate(s.depth);
        if let Some(parent) = stack.last() {
            assert!(
                parent.start <= s.start && s.end <= parent.end,
                "{name}: {} is not inside {}",
                s.name,
                parent.name
            );
        }
        stack.push(s);
    }
}

#[test]
fn outlines_are_well_formed() {
    for (path, _) in expectations() {
        let text = load(&path);
        if let Ok(symbols) = outline(&path, &text) {
            check_shape(&path.display().to_string(), &text, &symbols);
        }
    }
}

#[test]
fn scan_names() {
    let table: [(&str, Option<&str>); 22] = [
        ("a.ts", Some("c-like")),
        ("a.tsx", Some("c-like")),
        ("a.js", Some("c-like")),
        ("a.jsx", Some("c-like")),
        ("a.mjs", Some("c-like")),
        ("a.cjs", Some("c-like")),
        ("a.rs", Some("c-like")),
        ("a.go", Some("c-like")),
        ("a.java", Some("c-like")),
        ("a.css", Some("c-like")),
        ("a.scss", Some("c-like")),
        ("types/a.d.ts", Some("c-like")),
        ("a.py", Some("indent")),
        ("a.md", Some("markdown")),
        ("a.mdx", Some("markdown")),
        ("a.sh", Some("shell")),
        ("a.bash", Some("shell")),
        ("a.zsh", Some("shell")),
        ("a.xyz", None),
        ("Makefile", None),
        (".gitignore", None),
        ("a.tar.gz", None),
    ];
    for (path, want) in table {
        assert_eq!(scan_name(Path::new(path)), want, "scan_name({path})");
    }
}

#[test]
fn refusals() {
    // The minified fixture has one line over 10,000 chars.
    let minified = dir().join("minified.js");
    let text = load(&minified);
    assert!(text.lines().any(|l| l.len() > 10_000));
    assert!(matches!(outline(&minified, &text), Err(Refusal::LongLine)));

    // 2 MiB + 1 byte of short lines is too big, not a long line.
    assert!(matches!(
        outline(Path::new("big.ts"), &sized(TWO_MIB + 1)),
        Err(Refusal::TooBig)
    ));

    // A NUL byte means the file is not text.
    assert!(matches!(
        outline(Path::new("bin.ts"), "function a() {\n}\0\n"),
        Err(Refusal::NotText)
    ));
    assert!(matches!(
        outline(Path::new("bin.py"), "def a():\n    pass\0\n"),
        Err(Refusal::NotText)
    ));

    // An unknown extension has no scanner. The note carries the extension.
    match outline(Path::new("notes.xyz"), "hello\n") {
        Err(Refusal::NoScanner(ext)) => assert_eq!(ext, "xyz"),
        _ => panic!("expected NoScanner(\"xyz\")"),
    }
}

#[test]
fn limits_are_inclusive() {
    // Exactly 2 MiB is allowed (the limit is "over 2 MB").
    let at_limit = sized(TWO_MIB);
    assert!(outline(Path::new("edge.ts"), &at_limit).is_ok());

    // A line of exactly 10,000 chars is allowed, 10,001 is not.
    let ok = format!("// {}\n", "a".repeat(9_997));
    assert_eq!(ok.trim_end().chars().count(), 10_000);
    assert!(outline(Path::new("edge.ts"), &ok).is_ok());
    let long = format!("// {}\n", "a".repeat(9_998));
    assert!(matches!(
        outline(Path::new("edge.ts"), &long),
        Err(Refusal::LongLine)
    ));

    // A long line is refused whatever the language.
    let long_py = format!("# {}\n", "a".repeat(9_999));
    assert!(matches!(
        outline(Path::new("edge.py"), &long_py),
        Err(Refusal::LongLine)
    ));
}

#[test]
fn render_format() {
    let symbols = vec![
        Symbol {
            kind: "class".into(),
            name: "A".into(),
            start: 1,
            end: 9,
            depth: 0,
        },
        Symbol {
            kind: "method".into(),
            name: "b".into(),
            start: 2,
            end: 4,
            depth: 1,
        },
        Symbol {
            kind: "def".into(),
            name: "c".into(),
            start: 3,
            end: 3,
            depth: 2,
        },
    ];
    assert_eq!(
        trimmed(&render(&symbols)),
        "class A 1-9\n  method b 2-4\n    def c 3-3"
    );
    assert_eq!(trimmed(&render(&[])), "");
}
