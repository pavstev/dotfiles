//! Tests for the project detector (`agent_config::proj`) and the `proj` bin: fixture projects in
//! temp folders, a fake PATH for the detector, and fake tools in `node_modules/.bin` for the
//! format hook.

mod common;

use agent_config::proj::{self, Cmd};
use common::{run_bin, temp_home};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PROJ: &str = env!("CARGO_BIN_EXE_proj");

/// A project folder `<temp>/p` with the given files (path, text). A path ending in `/bin/<x>` under
/// `node_modules` or `.venv` is written as an executable script.
fn project(files: &[(&str, &str)]) -> PathBuf {
    let root = temp_home("proj").join("p");
    fs::create_dir_all(&root).unwrap();
    for (name, text) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        if name.contains("/.bin/") || name.contains("/bin/") {
            make_exe(&path);
        }
    }
    root
}

fn make_exe(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// A fake PATH: only the named programs exist.
fn with<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
    move |n| names.contains(&n)
}

fn lines(cmds: Vec<Cmd>) -> Vec<String> {
    cmds.iter().map(Cmd::line).collect()
}

fn rel(root: &Path, line: &str) -> String {
    line.replace(&format!("{}/", root.display()), "")
}

#[test]
fn pnpm_with_prettier_and_eslint() {
    let root = project(&[
        ("package.json", r#"{"scripts": {"check": "x", "check:fast": "y", "fix": "z"}}"#),
        ("pnpm-lock.yaml", ""),
        (".prettierrc", "{}"),
        ("eslint.config.js", ""),
        ("node_modules/.bin/prettier", "#!/bin/sh\n"),
        ("node_modules/.bin/eslint", "#!/bin/sh\n"),
        ("src/a.ts", ""),
        ("README.md", ""),
    ]);
    let p = proj::detect_with(&root.join("src/a.ts"), &with(&[])).unwrap();
    assert_eq!(p.root, root);
    assert_eq!(p.pm, Some("pnpm"));
    assert_eq!(p.tools(), ["prettier", "eslint"]);
    assert_eq!(lines(p.check_fast()), ["pnpm run check:fast"]);
    assert_eq!(lines(p.check_full()), ["pnpm run check"]);
    assert_eq!(lines(p.fix()), ["pnpm run fix"]);
    // eslint fixes first, prettier has the last word
    let fmt = lines(p.format_cmds(&[root.join("src/a.ts")]));
    assert_eq!(fmt.len(), 2, "{fmt:?}");
    assert!(rel(&root, &fmt[0]).starts_with("node_modules/.bin/eslint --fix "), "{fmt:?}");
    assert!(rel(&root, &fmt[1]).starts_with("node_modules/.bin/prettier --write "), "{fmt:?}");
    // a Markdown file gets no lint autofix
    let md = lines(p.format_cmds(&[root.join("README.md")]));
    assert_eq!(md.len(), 1);
    assert!(md[0].contains("prettier --write"));
}

#[test]
fn installed_but_unconfigured_prettier_is_left_alone() {
    let root =
        project(&[("package.json", "{}"), ("package-lock.json", ""), ("node_modules/.bin/prettier", "#!/bin/sh\n")]);
    let p = proj::detect_with(&root, &with(&[])).unwrap();
    assert_eq!(p.tools(), [] as [&str; 0]);
    assert_eq!(p.format_cmds(&[root.join("a.ts")]), [] as [Cmd; 0]);
}

#[test]
fn npm_with_biome_and_no_scripts() {
    let root = project(&[
        ("package.json", r#"{"scripts": {}}"#),
        ("package-lock.json", ""),
        ("biome.json", "{}"),
        ("node_modules/.bin/biome", "#!/bin/sh\n"),
    ]);
    let p = proj::detect_with(&root, &with(&[])).unwrap();
    assert_eq!(p.pm, Some("npm"));
    assert_eq!(p.tools(), ["biome"]);
    let fmt = lines(p.format_cmds(&[root.join("x.ts")]));
    assert!(
        rel(&root, &fmt[0]).starts_with("node_modules/.bin/biome check --write --no-errors-on-unmatched "),
        "{fmt:?}"
    );
    assert_eq!(
        lines(p.check_full()).iter().map(|l| rel(&root, l)).collect::<Vec<_>>(),
        ["node_modules/.bin/biome check ."]
    );
}

#[test]
fn package_manager_from_lockfile() {
    for (lock, pm) in [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("package-lock.json", "npm"),
    ] {
        let root = project(&[("package.json", r#"{"scripts": {"check": "x"}}"#), (lock, "")]);
        let p = proj::detect_with(&root, &with(&[])).unwrap();
        assert_eq!(p.pm, Some(pm), "{lock}");
        assert_eq!(lines(p.check_full()), [format!("{pm} run check")]);
    }
}

#[test]
fn package_manager_field_when_there_is_no_lockfile() {
    let root = project(&[("package.json", r#"{"packageManager": "pnpm@9.1.0"}"#)]);
    assert_eq!(proj::detect_with(&root, &with(&[])).unwrap().pm, Some("pnpm"));
}

#[test]
fn monorepo_package_finds_the_lockfile_and_tools_above() {
    let top = project(&[
        (".git/HEAD", "ref: refs/heads/main"),
        ("pnpm-lock.yaml", ""),
        (".prettierrc", "{}"),
        ("node_modules/.bin/prettier", "#!/bin/sh\n"),
        ("packages/a/package.json", "{}"),
        ("packages/a/src/x.ts", ""),
    ]);
    let p = proj::detect_with(&top.join("packages/a/src/x.ts"), &with(&[])).unwrap();
    assert_eq!(p.root, top.join("packages/a"));
    assert_eq!(p.pm, Some("pnpm"));
    assert_eq!(p.tools(), ["prettier"]);
}

#[test]
fn uv_with_ruff_and_ty() {
    let root = project(&[
        ("pyproject.toml", "[tool.ruff]\nline-length = 100\n[tool.ty.rules]\n"),
        ("uv.lock", ""),
        ("pkg/m.py", ""),
    ]);
    let p = proj::detect_with(&root, &with(&["uv"])).unwrap();
    assert_eq!(p.tools(), ["ruff", "ty", "uv"]);
    let fmt = lines(p.format_cmds(&[root.join("pkg/m.py")]));
    assert_eq!(fmt.len(), 2, "{fmt:?}");
    // a hook never syncs
    assert!(fmt[0].starts_with("uv run --no-sync ruff check --fix --quiet --unfixable F401 "), "{fmt:?}");
    assert!(fmt[1].starts_with("uv run --no-sync ruff format --quiet "), "{fmt:?}");
    assert_eq!(lines(p.check_full()), ["uv run ruff check .", "uv run ruff format --check .", "uv run ty check"]);
}

#[test]
fn venv_tools_win_over_uv_and_missing_tools_give_nothing() {
    let root = project(&[("pyproject.toml", "[tool.ruff]\n"), ("uv.lock", ""), (".venv/bin/ruff", "#!/bin/sh\n")]);
    let p = proj::detect_with(&root, &with(&["uv"])).unwrap();
    let fmt = lines(p.format_cmds(&[root.join("m.py")]));
    assert!(fmt[0].starts_with(&format!("{}/.venv/bin/ruff check --fix", root.display())), "{fmt:?}");
    let bare = project(&[("pyproject.toml", "[tool.ruff]\n")]);
    let p = proj::detect_with(&bare, &with(&[])).unwrap();
    assert_eq!(p.format_cmds(&[bare.join("m.py")]), [] as [Cmd; 0]);
}

#[test]
fn go_module() {
    let root = project(&[("go.mod", "module x\n"), (".golangci.yml", ""), ("main.go", "")]);
    let p = proj::detect_with(&root, &with(&["go", "gofmt", "golangci-lint"])).unwrap();
    assert_eq!(lines(p.format_cmds(&[root.join("main.go")])), [format!("gofmt -w {}/main.go", root.display())]);
    assert_eq!(lines(p.check_full()), ["go vet ./...", "golangci-lint run", "go test ./..."]);
    // no go toolchain on PATH: nothing to run
    let p = proj::detect_with(&root, &with(&[])).unwrap();
    assert_eq!(p.format_cmds(&[root.join("main.go")]), [] as [Cmd; 0]);
    assert_eq!(p.check_full(), [] as [Cmd; 0]);
}

#[test]
fn cargo_project() {
    let manifest = "[package]\nname = \"x\"\nedition = \"2024\"\n";
    let plain = project(&[("Cargo.toml", manifest), ("src/lib.rs", "")]);
    let p = proj::detect_with(&plain, &with(&["cargo", "rustfmt", "cargo-nextest"])).unwrap();
    // rustfmt has no config to find: the project must say it uses it
    assert_eq!(p.format_cmds(&[plain.join("src/lib.rs")]), [] as [Cmd; 0]);
    assert_eq!(
        lines(p.check_full()),
        ["cargo fmt --check", "cargo clippy --all-targets -- -D warnings", "cargo nextest run"]
    );
    let p = proj::detect_with(&plain, &with(&["cargo"])).unwrap();
    assert_eq!(lines(p.check_full()).last().unwrap(), "cargo test");

    let configured = project(&[("Cargo.toml", manifest), ("rustfmt.toml", ""), ("src/lib.rs", "")]);
    let p = proj::detect_with(&configured, &with(&["cargo", "rustfmt"])).unwrap();
    assert_eq!(
        lines(p.format_cmds(&[configured.join("src/lib.rs")])),
        [format!("rustfmt --edition 2024 {}/src/lib.rs", configured.display())]
    );
    let ci = project(&[("Cargo.toml", manifest), (".github/workflows/ci.yml", "run: cargo fmt --check\n")]);
    assert!(proj::detect_with(&ci, &with(&["cargo", "rustfmt"])).unwrap().tools().contains(&"rustfmt"));
}

#[test]
fn rust_project_is_not_a_python_project() {
    let root = project(&[("Cargo.toml", "[package]\n"), ("tests/t.rs", "")]);
    let p = proj::detect_with(&root, &with(&["cargo", "pytest"])).unwrap();
    assert!(!p.tools().contains(&"pytest"));
}

#[test]
fn justfile_contract() {
    let root = project(&[("justfile", "set shell := [\"sh\"]\ncheck:\n  true\ncheck-fast:\n  true\n@fix:\n  true\n")]);
    let p = proj::detect_with(&root, &with(&["just"])).unwrap();
    assert_eq!(lines(p.check_full()), ["just check"]);
    assert_eq!(lines(p.check_fast()), ["just check-fast"]);
    assert_eq!(lines(p.fix()), ["just fix"]);
    // without `just` on PATH the contract cannot run
    assert_eq!(proj::detect_with(&root, &with(&[])).unwrap().check_full(), [] as [Cmd; 0]);
}

#[test]
fn makefile_contract_and_package_json_goes_first() {
    let root = project(&[
        ("Makefile", ".PHONY: check\nCC = cc\ncheck:\n\ttrue\ncheck-fast:\n\ttrue\n"),
        ("package.json", r#"{"scripts": {"check": "x"}}"#),
        ("pnpm-lock.yaml", ""),
    ]);
    let p = proj::detect_with(&root, &with(&["make"])).unwrap();
    assert_eq!(lines(p.check_full()), ["pnpm run check"]);
    assert_eq!(lines(p.check_fast()), ["make check-fast"]);
}

#[test]
fn no_project() {
    let dir = temp_home("proj-none");
    fs::write(dir.join("notes.txt"), "").unwrap();
    assert!(proj::find_root(&dir.join("notes.txt")).is_none());
    assert!(proj::detect_with(&dir.join("notes.txt"), &with(&[])).is_none());
    let (code, out, _) = run_bin(PROJ, &["detect", dir.to_str().unwrap()], "", &[], None);
    assert_eq!((code, out.trim()), (1, "{\"root\": null}"));
}

#[test]
fn fast_check_default_uses_the_changed_files() {
    let root = project(&[
        ("package.json", "{}"),
        ("package-lock.json", ""),
        (".prettierrc", "{}"),
        ("node_modules/.bin/prettier", "#!/bin/sh\n"),
        ("old.ts", "1"),
    ]);
    let git = |args: &[&str]| {
        let st = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(st.status.success(), "{args:?}: {}", String::from_utf8_lossy(&st.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    let p = proj::detect_with(&root, &with(&[])).unwrap();
    assert!(p.check_fast().is_empty(), "nothing changed, nothing to check");
    fs::write(root.join("old.ts"), "2").unwrap();
    fs::write(root.join("new.css"), "").unwrap();
    let fast = lines(p.check_fast()).iter().map(|l| rel(&root, l)).collect::<Vec<_>>();
    assert_eq!(fast.len(), 1, "{fast:?}");
    assert!(fast[0].starts_with("node_modules/.bin/prettier --check "), "{fast:?}");
    assert!(fast[0].contains("new.css") && fast[0].contains("old.ts"), "{fast:?}");
}

#[test]
fn detect_cli_prints_json() {
    let root = project(&[("package.json", r#"{"scripts": {"check": "x"}}"#), ("pnpm-lock.yaml", "")]);
    let (code, out, _) = run_bin(PROJ, &["detect", root.to_str().unwrap()], "", &[], None);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["package_manager"], "pnpm");
    assert_eq!(v["contract"]["check"], "pnpm run check");
    assert_eq!(v["check_full"][0], "pnpm run check");
}

// --- the format hook ------------------------------------------------------------

/// A fake tool: logs its arguments in `calls.log` (in the project root, where it runs) and writes
/// `formatted` into its last argument.
const FAKE: &str =
    "#!/bin/sh\nfor a; do last=$a; done\necho \"$0 $@\" >> calls.log\nprintf 'formatted\\n' > \"$last\"\n";

fn hook_json(root: &Path, file: &str) -> String {
    serde_json::json!({"tool_name": "Edit", "cwd": root, "tool_input": {"file_path": file}}).to_string()
}

fn prettier_project(tool: &str) -> PathBuf {
    project(&[
        ("package.json", "{}"),
        (".prettierrc", "{}"),
        ("node_modules/.bin/prettier", tool),
        ("src/a.ts", "const a   =   1\n"),
    ])
}

#[cfg(unix)]
#[test]
fn hook_formats_the_edited_file_silently() {
    let root = prettier_project(FAKE);
    let (code, out, err) =
        run_bin(PROJ, &["format"], &hook_json(&root, root.join("src/a.ts").to_str().unwrap()), &[], None);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
    assert_eq!(fs::read_to_string(root.join("src/a.ts")).unwrap(), "formatted\n");
    let log = fs::read_to_string(root.join("calls.log")).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");
}

#[cfg(unix)]
#[test]
fn hook_takes_a_path_relative_to_cwd() {
    let root = prettier_project(FAKE);
    let (code, _, _) = run_bin(PROJ, &["format"], &hook_json(&root, "src/a.ts"), &[], None);
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(root.join("src/a.ts")).unwrap(), "formatted\n");
}

#[test]
fn hook_with_a_missing_tool_exits_0_and_changes_nothing() {
    let root = project(&[("package.json", "{}"), (".prettierrc", "{}"), ("src/a.ts", "x  =  1\n")]);
    let (code, out, err) =
        run_bin(PROJ, &["format"], &hook_json(&root, root.join("src/a.ts").to_str().unwrap()), &[], None);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
    assert_eq!(fs::read_to_string(root.join("src/a.ts")).unwrap(), "x  =  1\n");
}

#[test]
fn hook_skips_files_outside_a_project_and_bad_input() {
    let dir = temp_home("proj-outside");
    fs::write(dir.join("a.ts"), "x  =  1\n").unwrap();
    for stdin in [hook_json(&dir, dir.join("a.ts").to_str().unwrap()), "not json".into(), "{}".into(), String::new()] {
        let (code, out, err) = run_bin(PROJ, &["format"], &stdin, &[], None);
        assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "{stdin}");
    }
    assert_eq!(fs::read_to_string(dir.join("a.ts")).unwrap(), "x  =  1\n");
}

#[cfg(unix)]
#[test]
fn hook_skips_dependency_folders() {
    let root = prettier_project(FAKE);
    let dep = root.join("node_modules/pkg/index.js");
    fs::create_dir_all(dep.parent().unwrap()).unwrap();
    fs::write(&dep, "a  =  1\n").unwrap();
    run_bin(PROJ, &["format"], &hook_json(&root, dep.to_str().unwrap()), &[], None);
    assert_eq!(fs::read_to_string(&dep).unwrap(), "a  =  1\n");
    assert!(!root.join("calls.log").exists());
}

#[cfg(unix)]
#[test]
fn hook_exits_0_when_the_tool_fails_as_it_does_for_an_ignored_file() {
    let root = prettier_project("#!/bin/sh\necho oops >&2\nexit 2\n");
    let (code, out, err) =
        run_bin(PROJ, &["format"], &hook_json(&root, root.join("src/a.ts").to_str().unwrap()), &[], None);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
    assert_eq!(fs::read_to_string(root.join("src/a.ts")).unwrap(), "const a   =   1\n");
}

#[cfg(unix)]
#[test]
fn hook_stops_a_slow_tool_at_its_budget() {
    let root = prettier_project("#!/bin/sh\nexec sleep 30\n");
    let t = Instant::now();
    let (code, out, err) = run_bin(
        PROJ,
        &["format"],
        &hook_json(&root, root.join("src/a.ts").to_str().unwrap()),
        &[("PROJ_FORMAT_BUDGET_MS", "300")],
        None,
    );
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
    assert!(t.elapsed() < Duration::from_secs(5), "took {:?}", t.elapsed());
}

#[cfg(unix)]
#[test]
fn hook_runs_eslint_then_prettier_with_a_shared_budget() {
    let root = project(&[
        ("package.json", "{}"),
        (".prettierrc", "{}"),
        ("eslint.config.js", ""),
        ("node_modules/.bin/prettier", FAKE),
        ("node_modules/.bin/eslint", FAKE),
        ("a.ts", ""),
    ]);
    run_bin(PROJ, &["format"], &hook_json(&root, root.join("a.ts").to_str().unwrap()), &[], None);
    let log = fs::read_to_string(root.join("calls.log")).unwrap();
    let tools: Vec<&str> =
        log.lines().map(|l| l.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("")).collect();
    assert_eq!(tools, ["eslint", "prettier"], "{log}");
}
