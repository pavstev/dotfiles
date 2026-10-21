//! The project detector: facts about the project a path lives in, for any language.
//!
//! "Facts in the project, smarts in the dotfiles." A project states its facts in files it already
//! has (lockfile, tool config, `package.json` scripts, a `justfile` or `Makefile`). This module
//! reads them and answers four questions: which project is this, which package manager and tools
//! does it use, how is one file formatted, and what are the fast and the full check. The contract
//! comes first (`check`, `check:fast`, `fix`), then language defaults. The project's own installed
//! tools run in the project folder (`node_modules/.bin`, `.venv/bin`, `uv run`, `cargo`), so `mise`
//! picks the pinned runtime and any tool version just works.
//!
//! Reached through the `proj` bin (`rust-bin proj detect [path]`, `rust-bin --hook proj format`)
//! and by the guard (is a Bash command a check, did the work tree change). Everything here fails
//! open: a missing tool or file gives an empty answer, never an error.

use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Files that mark a project folder. The nearest folder up from a path with one of them is the root.
const MARKERS: &[&str] = &[
    "package.json",
    "pyproject.toml",
    "Cargo.toml",
    "go.mod",
    "justfile",
    "Justfile",
    "Makefile",
    "makefile",
    "GNUmakefile",
    "biome.json",
    "biome.jsonc",
    "ruff.toml",
    ".ruff.toml",
];

/// Folders whose files a format hook never touches.
const SKIP_DIRS: &[&str] = &["node_modules", ".git", ".venv", "venv", "target", "dist", "build", ".next", ".astro"];

const JS_CODE: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts", "vue", "svelte", "astro"];
const PRETTIER_EXT: &[&str] = &[
    "js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts", "vue", "svelte", "astro", "json", "jsonc", "css", "scss",
    "less", "md", "mdx", "html", "yaml", "yml", "graphql",
];
const BIOME_EXT: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts", "json", "jsonc", "css"];

/// One command: the program and its arguments, run in `dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub argv: Vec<String>,
    pub dir: PathBuf,
}

impl Cmd {
    fn new(argv: Vec<String>, dir: &Path) -> Self {
        Self { argv, dir: dir.to_path_buf() }
    }

    /// The command as one shell line (arguments with odd characters in single quotes).
    pub fn line(&self) -> String {
        self.argv
            .iter()
            .map(|a| {
                let plain = !a.is_empty() && a.chars().all(|c| c.is_alphanumeric() || "-_./:=@+,%".contains(c));
                if plain { a.clone() } else { format!("'{}'", a.replace('\'', "'\\''")) }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Which programs the machine has on PATH. A fake in a test stands in for the real lookup.
pub type Have<'a> = &'a dyn Fn(&str) -> bool;

/// True when `name` is an executable on PATH.
pub fn on_path(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else { return false };
    env::split_paths(&path).any(|d| {
        is_exe(&d.join(name))
            || (cfg!(windows) && ["exe", "cmd"].iter().any(|e| is_exe(&d.join(format!("{name}.{e}")))))
    })
}

fn is_exe(p: &Path) -> bool {
    let Ok(m) = fs::metadata(p) else { return false };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The project root of `path` (a file or a folder): the nearest folder up with a marker file. The
/// home folder is never a root.
pub fn find_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    let home = crate::paths::home();
    for dir in start.ancestors() {
        if dir == home || dir.parent().is_none() {
            return None;
        }
        if MARKERS.iter().any(|m| dir.join(m).is_file()) {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// `root` and its parents, up to and including the first with `.git` (a monorepo keeps its lockfile
/// and `node_modules` at the top). Never reaches the home folder.
fn ancestors_to_git(root: &Path) -> Vec<PathBuf> {
    let home = crate::paths::home();
    let mut out = Vec::new();
    for dir in root.ancestors() {
        if dir == home || dir.parent().is_none() {
            break;
        }
        out.push(dir.to_path_buf());
        if dir.join(".git").exists() {
            break;
        }
    }
    out
}

/// A package manager, from the lockfile.
fn pm_from_lock(dirs: &[PathBuf]) -> Option<&'static str> {
    for d in dirs {
        for (file, pm) in [
            ("pnpm-lock.yaml", "pnpm"),
            ("bun.lock", "bun"),
            ("bun.lockb", "bun"),
            ("yarn.lock", "yarn"),
            ("package-lock.json", "npm"),
        ] {
            if d.join(file).is_file() {
                return Some(pm);
            }
        }
    }
    None
}

fn pm_name(s: &str) -> Option<&'static str> {
    ["pnpm", "npm", "yarn", "bun"].into_iter().find(|p| *p == s)
}

/// Recipe or target names of a `justfile` or `Makefile`.
fn rule_names(text: &str, make: bool) -> Vec<String> {
    static JUST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^@?([A-Za-z_][A-Za-z0-9_-]*)[^:=\n]*:(?:[^=]|$)").unwrap());
    static MAKE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([A-Za-z0-9][A-Za-z0-9_.-]*)\s*:(?:[^=]|$)").unwrap());
    let re: &Regex = if make { &MAKE } else { &JUST };
    text.lines().filter_map(|l| re.captures(l).map(|c| c[1].to_string())).collect()
}

/// True when a file that wires tools (scripts, hooks, CI) mentions one of `needles`.
fn mentions(root: &Path, needles: &[&str]) -> bool {
    let mut files: Vec<PathBuf> = [
        "package.json",
        "justfile",
        "Justfile",
        "Makefile",
        ".pre-commit-config.yaml",
        "lefthook.yml",
        "lefthook.yaml",
    ]
    .iter()
    .map(|f| root.join(f))
    .collect();
    if let Ok(rd) = fs::read_dir(root.join(".github/workflows")) {
        files.extend(rd.flatten().map(|e| e.path()));
    }
    files.iter().filter_map(|f| fs::read_to_string(f).ok()).any(|t| needles.iter().any(|n| t.contains(n)))
}

/// Everything the detector knows about one project.
#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    dirs: Vec<PathBuf>,
    pub pm: Option<&'static str>,
    scripts: Vec<String>,
    just: Vec<String>,
    make: Vec<String>,
    biome: Option<PathBuf>,
    prettier: Option<PathBuf>,
    eslint: Option<PathBuf>,
    tsc: Option<PathBuf>,
    ruff: bool,
    ty: bool,
    pytest: bool,
    uv: bool,
    go: bool,
    golangci: bool,
    cargo: bool,
    edition: String,
    rustfmt: bool,
    shfmt: bool,
    have: BTreeMap<&'static str, bool>,
}

const PROBES: &[&str] = &[
    "uv",
    "ruff",
    "ty",
    "pytest",
    "go",
    "gofmt",
    "rustfmt",
    "cargo",
    "cargo-nextest",
    "golangci-lint",
    "shfmt",
    "just",
    "make",
];

/// Detect the project of `path`, with the real PATH.
pub fn detect(path: &Path) -> Option<Project> {
    detect_with(path, &on_path)
}

/// Detect the project of `path`; `have` says which programs exist on PATH.
pub fn detect_with(path: &Path, have: Have) -> Option<Project> {
    let root = find_root(path)?;
    let dirs = ancestors_to_git(&root);
    let read = |name: &str| fs::read_to_string(root.join(name)).unwrap_or_default();
    let pkg: Value = serde_json::from_str(&read("package.json")).unwrap_or(Value::Null);
    let scripts: Vec<String> = pkg["scripts"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    let pm = pm_from_lock(&dirs).or_else(|| {
        let field = pkg["packageManager"].as_str()?;
        pm_name(field.split('@').next()?)
    });
    let pm = pm.or(if pkg.is_object() { Some("npm") } else { None });

    let bin = |name: &str| -> Option<PathBuf> {
        let file = if cfg!(windows) { format!("{name}.cmd") } else { name.to_string() };
        dirs.iter().map(|d| d.join("node_modules/.bin").join(&file)).find(|p| p.is_file())
    };
    let has_file = |pred: &dyn Fn(&str) -> bool| -> bool {
        dirs.iter()
            .any(|d| fs::read_dir(d).is_ok_and(|rd| rd.flatten().any(|e| pred(&e.file_name().to_string_lossy()))))
    };
    let biome_cfg = has_file(&|n| n == "biome.json" || n == "biome.jsonc");
    let prettier_cfg = has_file(&|n| n.starts_with(".prettierrc") || n.starts_with("prettier.config."))
        || pkg.get("prettier").is_some();
    let eslint_cfg = has_file(&|n| n.starts_with("eslint.config.") || n.starts_with(".eslintrc"))
        || pkg.get("eslintConfig").is_some();

    let pyproject = read("pyproject.toml");
    static TY_DEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""ty(?:[<>=!~\[ ;]|")"#).unwrap());
    let ruff = root.join("ruff.toml").is_file() || root.join(".ruff.toml").is_file() || pyproject.contains("ruff");
    let ty = root.join("ty.toml").is_file() || pyproject.contains("[tool.ty") || TY_DEP.is_match(&pyproject);
    let pytest = root.join("pytest.ini").is_file()
        || pyproject.contains("[tool.pytest")
        || (!pyproject.is_empty() && root.join("tests").is_dir());
    let uv = dirs.iter().any(|d| d.join("uv.lock").is_file()) || pyproject.contains("[tool.uv");

    let cargo_toml = read("Cargo.toml");
    static EDITION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^\s*edition\s*=\s*"(\d{4})""#).unwrap());
    let edition = EDITION.captures(&cargo_toml).map_or_else(|| "2021".into(), |c| c[1].to_string());

    // rustfmt and shfmt have no config the project must have: format only when the project says it uses them
    let rustfmt = dirs.iter().any(|d| d.join("rustfmt.toml").is_file() || d.join(".rustfmt.toml").is_file())
        || mentions(&root, &["cargo fmt", "rustfmt"]);
    let shfmt = mentions(&root, &["shfmt"]);

    let just = ["justfile", "Justfile"]
        .iter()
        .find(|f| root.join(f).is_file())
        .map(|f| rule_names(&read(f), false))
        .unwrap_or_default();
    let make = ["Makefile", "makefile", "GNUmakefile"]
        .iter()
        .find(|f| root.join(f).is_file())
        .map(|f| rule_names(&read(f), true))
        .unwrap_or_default();

    let have_map = PROBES.iter().map(|p| (*p, have(p))).collect();
    Some(Project {
        biome: if biome_cfg { bin("biome") } else { None },
        prettier: if prettier_cfg { bin("prettier") } else { None },
        eslint: if eslint_cfg { bin("eslint") } else { None },
        tsc: if root.join("tsconfig.json").is_file() { bin("tsc") } else { None },
        pm,
        scripts,
        just,
        make,
        ruff,
        ty,
        pytest,
        uv,
        go: root.join("go.mod").is_file(),
        golangci: [".golangci.yml", ".golangci.yaml", ".golangci.toml", ".golangci.json"]
            .iter()
            .any(|f| root.join(f).is_file()),
        cargo: root.join("Cargo.toml").is_file(),
        edition,
        rustfmt,
        shfmt,
        dirs,
        root,
        have: have_map,
    })
}

fn ext(file: &Path) -> String {
    file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

impl Project {
    fn has(&self, prog: &str) -> bool {
        self.have.get(prog).copied().unwrap_or(false)
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    /// The names of the tools this project uses, for `detect`.
    pub fn tools(&self) -> Vec<&'static str> {
        let mut t = Vec::new();
        let flags = [
            (self.prettier.is_some(), "prettier"),
            (self.eslint.is_some(), "eslint"),
            (self.biome.is_some(), "biome"),
            (self.tsc.is_some(), "tsc"),
            (self.ruff, "ruff"),
            (self.ty, "ty"),
            (self.pytest, "pytest"),
            (self.uv, "uv"),
            (self.go, "go"),
            (self.golangci, "golangci-lint"),
            (self.cargo, "cargo"),
            (self.cargo && self.has("cargo-nextest"), "nextest"),
            (self.shfmt && self.has("shfmt"), "shfmt"),
            (self.rustfmt && self.has("rustfmt"), "rustfmt"),
        ];
        t.extend(flags.iter().filter(|(on, _)| *on).map(|(_, n)| *n));
        t
    }

    /// A Python tool as a command prefix: `.venv/bin/<name>`, else `uv run [--no-sync] <name>`, else
    /// the PATH program. `sync` false is for hooks, which must never install anything.
    fn py(&self, name: &str, sync: bool) -> Option<Vec<String>> {
        for d in &self.dirs {
            for venv in [".venv", "venv"] {
                let dir = if cfg!(windows) { "Scripts" } else { "bin" };
                let p = d.join(venv).join(dir).join(name);
                if p.is_file() {
                    return Some(vec![Self::s(&p)]);
                }
            }
        }
        if self.uv && self.has("uv") {
            let mut v = vec!["uv".to_string(), "run".into()];
            if !sync {
                v.push("--no-sync".into());
            }
            v.push(name.into());
            return Some(v);
        }
        self.has(name).then(|| vec![name.to_string()])
    }

    /// `<pm> run <script>`.
    fn pm_run(&self, script: &str) -> Cmd {
        let pm = self.pm.unwrap_or("npm");
        Cmd::new(vec![pm.into(), "run".into(), script.into()], &self.root)
    }

    /// The contract command for `names` (a `package.json` script, then a `justfile` recipe, then a
    /// `Makefile` target): `script` is the package.json name, `rule` the justfile or Makefile name.
    fn contract(&self, script: &str, rule: &str) -> Option<Cmd> {
        if self.scripts.iter().any(|s| s == script) {
            return Some(self.pm_run(script));
        }
        if self.just.iter().any(|s| s == rule) && self.has("just") {
            return Some(Cmd::new(vec!["just".into(), rule.into()], &self.root));
        }
        if self.make.iter().any(|s| s == rule) && self.has("make") {
            return Some(Cmd::new(vec!["make".into(), rule.into()], &self.root));
        }
        None
    }

    /// The contract commands `(check, check:fast, fix)` the project states, as shell lines.
    pub fn contract_lines(&self) -> Value {
        let line = |c: Option<Cmd>| c.map_or(Value::Null, |c| Value::String(c.line()));
        json!({
            "check": line(self.contract("check", "check")),
            "check_fast": line(self.contract("check:fast", "check-fast")),
            "fix": line(self.contract("fix", "fix")),
        })
    }

    /// The commands that format `files` (all in this project): the formatter of each language, plus a
    /// safe lint autofix where the project has one. Only tools the project installed and configured.
    pub fn format_cmds(&self, files: &[PathBuf]) -> Vec<Cmd> {
        let pick = |exts: &[&str]| -> Vec<String> {
            files.iter().filter(|f| exts.contains(&ext(f).as_str())).map(|f| Self::s(f)).collect()
        };
        let mut out = Vec::new();
        let mut add = |prefix: Vec<String>, tail: &[&str], files: Vec<String>| {
            if !files.is_empty() {
                let mut argv = prefix;
                argv.extend(tail.iter().map(ToString::to_string));
                argv.extend(files);
                out.push(Cmd::new(argv, &self.root));
            }
        };
        if let Some(e) = &self.eslint {
            add(vec![Self::s(e)], &["--fix"], pick(JS_CODE));
        }
        if let Some(b) = &self.biome {
            add(vec![Self::s(b)], &["check", "--write", "--no-errors-on-unmatched"], pick(BIOME_EXT));
        } else if let Some(p) = &self.prettier {
            add(vec![Self::s(p)], &["--write"], pick(PRETTIER_EXT));
        }
        let py = pick(&["py", "pyi"]);
        if self.ruff
            && !py.is_empty()
            && let Some(r) = self.py("ruff", false)
        {
            add(r.clone(), &["check", "--fix", "--quiet", "--unfixable", "F401"], py.clone());
            add(r, &["format", "--quiet"], py);
        }
        if self.go && self.has("gofmt") {
            add(vec!["gofmt".into()], &["-w"], pick(&["go"]));
        }
        if self.cargo && self.rustfmt && self.has("rustfmt") {
            add(vec!["rustfmt".into()], &["--edition", &self.edition], pick(&["rs"]));
        }
        if self.shfmt && self.has("shfmt") {
            add(vec!["shfmt".into()], &["-w"], pick(&["sh", "bash"]));
        }
        out
    }

    /// The fast check: the contract `check:fast`, else the language defaults on the files changed in
    /// the git tree. Empty when nothing changed.
    pub fn check_fast(&self) -> Vec<Cmd> {
        if let Some(c) = self.contract("check:fast", "check-fast") {
            return vec![c];
        }
        let changed = self.changed_files();
        let pick = |exts: &[&str]| -> Vec<String> {
            changed.iter().filter(|f| exts.contains(&ext(f).as_str())).map(|f| Self::s(f)).collect()
        };
        let mut out = Vec::new();
        let mut add = |prefix: Vec<String>, tail: &[&str], files: Vec<String>| {
            if !files.is_empty() {
                let mut argv = prefix;
                argv.extend(tail.iter().map(ToString::to_string));
                argv.extend(files);
                out.push(Cmd::new(argv, &self.root));
            }
        };
        if let Some(b) = &self.biome {
            add(vec![Self::s(b)], &["check", "--no-errors-on-unmatched"], pick(BIOME_EXT));
        } else {
            if let Some(e) = &self.eslint {
                add(vec![Self::s(e)], &[], pick(JS_CODE));
            }
            if let Some(p) = &self.prettier {
                add(vec![Self::s(p)], &["--check"], pick(PRETTIER_EXT));
            }
        }
        let py = pick(&["py", "pyi"]);
        if self.ruff
            && !py.is_empty()
            && let Some(r) = self.py("ruff", true)
        {
            add(r.clone(), &["check"], py.clone());
            add(r, &["format", "--check"], py);
        }
        if self.go && self.has("go") {
            let mut dirs: Vec<String> = pick(&["go"])
                .iter()
                .map(|f| {
                    let rel = Path::new(f).strip_prefix(&self.root).unwrap_or(Path::new(f));
                    let d = rel.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    if d.is_empty() { ".".to_string() } else { format!("./{d}") }
                })
                .collect();
            dirs.sort();
            dirs.dedup();
            add(vec!["go".into()], &["vet"], dirs);
        }
        if self.cargo && self.has("cargo") && !pick(&["rs", "toml"]).is_empty() {
            out.push(Cmd::new(vec!["cargo".into(), "check".into(), "--all-targets".into()], &self.root));
        }
        out
    }

    /// The full check: the contract `check`, else the language defaults over the whole project.
    pub fn check_full(&self) -> Vec<Cmd> {
        if let Some(c) = self.contract("check", "check") {
            return vec![c];
        }
        let mut out = Vec::new();
        let has_script = |n: &str| self.scripts.iter().any(|s| s == n);
        if self.pm.is_some() {
            let named: Vec<&str> =
                ["lint", "typecheck", "types", "test"].into_iter().filter(|n| has_script(n)).collect();
            if named.is_empty() {
                if let Some(b) = &self.biome {
                    out.push(Cmd::new(vec![Self::s(b), "check".into(), ".".into()], &self.root));
                }
                if let Some(e) = &self.eslint {
                    out.push(Cmd::new(vec![Self::s(e), ".".into()], &self.root));
                }
                if self.biome.is_none()
                    && let Some(p) = &self.prettier
                {
                    out.push(Cmd::new(vec![Self::s(p), "--check".into(), ".".into()], &self.root));
                }
                if let Some(t) = &self.tsc {
                    out.push(Cmd::new(vec![Self::s(t), "--noEmit".into()], &self.root));
                }
            } else {
                out.extend(named.into_iter().map(|n| self.pm_run(n)));
            }
        }
        if self.ruff
            && let Some(r) = self.py("ruff", true)
        {
            out.push(Cmd::new([r.clone(), vec!["check".into(), ".".into()]].concat(), &self.root));
            out.push(Cmd::new([r, vec!["format".into(), "--check".into(), ".".into()]].concat(), &self.root));
        }
        if self.ty
            && let Some(t) = self.py("ty", true)
        {
            out.push(Cmd::new([t, vec!["check".into()]].concat(), &self.root));
        }
        if self.pytest
            && let Some(p) = self.py("pytest", true)
        {
            out.push(Cmd::new(p, &self.root));
        }
        if self.go && self.has("go") {
            out.push(Cmd::new(vec!["go".into(), "vet".into(), "./...".into()], &self.root));
            if self.golangci && self.has("golangci-lint") {
                out.push(Cmd::new(vec!["golangci-lint".into(), "run".into()], &self.root));
            }
            out.push(Cmd::new(vec!["go".into(), "test".into(), "./...".into()], &self.root));
        }
        if self.cargo && self.has("cargo") {
            let c = |a: &[&str]| {
                Cmd::new(std::iter::once("cargo").chain(a.iter().copied()).map(String::from).collect(), &self.root)
            };
            out.push(c(&["fmt", "--check"]));
            out.push(c(&["clippy", "--all-targets", "--", "-D", "warnings"]));
            out.push(if self.has("cargo-nextest") { c(&["nextest", "run"]) } else { c(&["test"]) });
        }
        out
    }

    /// The fix command: the contract `fix`, else the formatters over the files changed in the git tree.
    pub fn fix(&self) -> Vec<Cmd> {
        if let Some(c) = self.contract("fix", "fix") {
            return vec![c];
        }
        self.format_cmds(&self.changed_files())
    }

    /// Files changed in the git tree (staged, unstaged, new), absolute, that still exist. Empty
    /// outside git.
    pub fn changed_files(&self) -> Vec<PathBuf> {
        let git = |args: &[&str]| -> Vec<String> {
            let Ok(o) = Command::new("git")
                .args(args)
                .current_dir(&self.root)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
            else {
                return Vec::new();
            };
            if !o.status.success() {
                return Vec::new();
            }
            String::from_utf8_lossy(&o.stdout).split('\0').filter(|s| !s.is_empty()).map(String::from).collect()
        };
        let mut names = git(&["diff", "--name-only", "--relative", "-z", "HEAD"]);
        names.extend(git(&["ls-files", "-o", "--exclude-standard", "-z"]));
        names.sort();
        names.dedup();
        names.into_iter().map(|n| self.root.join(n)).filter(|p| p.is_file()).collect()
    }

    /// The `detect` answer as JSON. With `file`, it also lists the format commands for that file.
    pub fn to_json(&self, file: Option<&Path>) -> Value {
        let lines = |v: Vec<Cmd>| -> Vec<String> { v.iter().map(Cmd::line).collect() };
        let mut v = json!({
            "root": Self::s(&self.root),
            "package_manager": self.pm,
            "tools": self.tools(),
            "contract": self.contract_lines(),
            "check_fast": lines(self.check_fast()),
            "check_full": lines(self.check_full()),
            "fix": lines(self.fix()),
        });
        if let Some(f) = file {
            v["format"] = json!(lines(self.format_cmds(&[f.to_path_buf()])));
        }
        v
    }
}

// --- running commands ---------------------------------------------------------------

/// How a run ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ran {
    Ok,
    Failed,
    TimedOut,
    Missing,
}

/// Run `cmd` silently in its folder until `deadline`. A run past the deadline is killed, with its
/// process group on Unix.
pub fn run(cmd: &Cmd, deadline: Instant) -> Ran {
    let Some((prog, args)) = cmd.argv.split_first() else { return Ran::Missing };
    let mut c = Command::new(prog);
    c.args(args).current_dir(&cmd.dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let Ok(mut child) = c.spawn() else { return Ran::Missing };
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return if st.success() { Ran::Ok } else { Ran::Failed },
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                #[cfg(unix)]
                {
                    let _ = Command::new("kill")
                        .args(["-KILL", &format!("-{}", child.id())])
                        .stderr(Stdio::null())
                        .status();
                }
                let _ = child.kill();
                let _ = child.wait();
                return Ran::TimedOut;
            }
        }
    }
}

/// Format `file` with the formatter of its project: silent, within `budget`, never an error. Returns
/// the number of commands that ran. A file outside a project, in a skipped folder, missing or large
/// is left alone.
pub fn format_file(file: &Path, budget: Duration) -> usize {
    format_file_with(file, budget, &on_path)
}

pub fn format_file_with(file: &Path, budget: Duration, have: Have) -> usize {
    let Ok(meta) = fs::metadata(file) else { return 0 };
    if !meta.is_file() || meta.len() > 1_000_000 {
        return 0;
    }
    if file.components().any(|c| SKIP_DIRS.iter().any(|s| c.as_os_str() == *s)) {
        return 0;
    }
    let Some(project) = detect_with(file, have) else { return 0 };
    let deadline = Instant::now() + budget;
    let mut ran = 0;
    for cmd in project.format_cmds(&[file.to_path_buf()]) {
        match run(&cmd, deadline) {
            Ran::TimedOut => break,
            Ran::Missing => {}
            _ => ran += 1,
        }
    }
    ran
}

// --- is a Bash command a check? -------------------------------------------------------

/// The command in one line with single spaces, when it is one plain check command: optional
/// `cd <dir> &&` steps and `VAR=value` prefixes, then a check, with no pipe, `;`, `||`, background
/// or subshell. Those shapes hide the exit code of the check, so a pass could not be trusted.
pub fn plain_check(cmd: &str) -> Option<String> {
    static CHECK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"^(?:time )?(?:",
            r"(?:pnpm|npm|yarn|bun)(?: run)? (?:check|test|lint|typecheck|types|build)(?:[:\-][\w:\-]+)?(?: .*)?",
            r"|npm t(?: .*)?",
            r"|cargo(?: \+\S+)? (?:test|nextest|clippy|check)(?: .*)?",
            r"|go (?:test|vet)(?: .*)?",
            r"|(?:python3? -m )?pytest(?: .*)?",
            r"|uv run (?:python3? -m pytest|pytest|ruff check|ty check)(?: .*)?",
            r"|(?:ruff|ty) check(?: .*)?",
            r"|(?:just|make) (?:check|test|lint)(?:[:\-][\w:\-]+)?(?: .*)?",
            r")$"
        ))
        .unwrap()
    });
    static ENV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[A-Za-z_]\w*=\S*\s+)+").unwrap());
    let cmd = cmd.replace("2>&1", "");
    if cmd.contains(['|', ';', '\n', '`']) || cmd.contains("$(") {
        return None;
    }
    let steps: Vec<&str> = cmd.split("&&").map(str::trim).collect();
    let (last, lead) = steps.split_last()?;
    if lead.iter().any(|s| !s.starts_with("cd ")) || steps.iter().any(|s| s.contains('&')) {
        return None;
    }
    let one: String = last.split_whitespace().collect::<Vec<_>>().join(" ");
    let bare = ENV.replace(&one, "");
    if bare.contains("--watch") || bare.split(' ').any(|a| a == "-w") {
        return None;
    }
    CHECK.is_match(&bare).then(|| {
        steps.iter().map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" && ")
    })
}

// --- did the work tree change? -------------------------------------------------------

/// A fingerprint of the git work tree at `cwd`: the HEAD commit, the `git status` lines, and the size
/// and time of each listed file. It changes when a commit, an edit, a new file or a stage happens. None
/// outside git or on any error. Two small git calls, about 20 ms on a normal tree.
pub fn tree_fingerprint(cwd: &Path) -> Option<String> {
    let git = |args: &[&str]| -> Option<Vec<u8>> {
        let o =
            Command::new("git").args(args).current_dir(cwd).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        o.status.success().then_some(o.stdout)
    };
    let top = String::from_utf8(git(&["rev-parse", "--show-toplevel"])?).ok()?;
    let top = PathBuf::from(top.trim());
    let head = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let status = git(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let mut h = Sha256::new();
    h.update(&head);
    h.update(&status);
    let mut parts = status.split(|b| *b == 0).filter(|p| !p.is_empty());
    let mut n = 0;
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            parts.next(); // the old name of a rename
        }
        n += 1;
        if n > 5000 {
            break;
        }
        let path = top.join(String::from_utf8_lossy(&entry[3..]).as_ref());
        if let Ok(m) = fs::metadata(&path) {
            let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            h.update(m.len().to_le_bytes());
            h.update(t.to_le_bytes());
        }
    }
    Some(format!("{:x}", h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_checks() {
        for c in [
            "pnpm check",
            "pnpm run check:fast --staged",
            "npm test",
            "npm run lint",
            "cd app && pnpm run build",
            "CI=1 cargo nextest run",
            "cargo clippy --all-targets -- -D warnings",
            "go test ./...",
            "pytest -x",
            "uv run ruff check .",
            "ty check",
            "just check",
            "make check-fast",
            "pnpm check 2>&1",
        ] {
            assert!(plain_check(c).is_some(), "{c}");
        }
        for c in [
            "pnpm check | tail",
            "pnpm check; echo done",
            "pnpm check || true",
            "pnpm check &",
            "ls",
            "git commit -m check",
            "pnpm test --watch",
            "echo $(pnpm test)",
            "cd x; pnpm test",
            "pnpm dev",
            "cargo build",
        ] {
            assert!(plain_check(c).is_none(), "{c}");
        }
        assert_eq!(plain_check("cd  a  &&  pnpm   check").as_deref(), Some("cd a && pnpm check"));
    }

    #[test]
    fn rule_name_parsing() {
        let just = "set shell := [\"sh\"]\nversion := \"1\"\n@check *args:\n  pnpm check\ncheck-fast: deps\nfix:\n";
        assert_eq!(rule_names(just, false), ["check", "check-fast", "fix"]);
        let make = ".PHONY: check\nCC = gcc\ncheck:\n\ttrue\ncheck-fast: x\n";
        assert_eq!(rule_names(make, true), ["check", "check-fast"]);
    }

    #[test]
    fn line_quotes_odd_arguments() {
        let c = Cmd::new(vec!["prettier".into(), "--write".into(), "a b.ts".into()], Path::new("/"));
        assert_eq!(c.line(), "prettier --write 'a b.ts'");
    }
}
