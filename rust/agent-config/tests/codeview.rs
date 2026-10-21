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

use agent_config::codeview::{Refusal, outline, render, scan_name};
use std::fs;
use std::path::{Path, PathBuf};

const TWO_MIB: usize = 2 * 1024 * 1024;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codeview")
}

/// Reads a fixture.
fn load(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
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
fn limits_are_inclusive() {
    // Exactly 2 MiB is allowed (the limit is "over 2 MB"); one byte more is too big, not a long line.
    assert!(outline(Path::new("edge.ts"), &sized(TWO_MIB)).is_ok());
    assert!(matches!(outline(Path::new("big.ts"), &sized(TWO_MIB + 1)), Err(Refusal::TooBig)));

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

    // A NUL byte means the file is not text. The long-line and unknown-extension refusals have
    // golden files (minified.js, unknown.xyz).
    assert!(matches!(outline(Path::new("bin.ts"), "function a() {\n}\0\n"), Err(Refusal::NotText)));
    assert!(matches!(outline(Path::new("bin.py"), "def a():\n    pass\0\n"), Err(Refusal::NotText)));
}
