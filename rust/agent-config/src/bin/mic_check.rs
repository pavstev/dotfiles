//! mic_check: is this repo really done? Read-only (except `git fetch`). Replaces
//! claude/skills/ps_mic-drop/scripts/mic-check.sh (same flags, lines and exit codes).
//!
//! ```text
//! mic_check [repo...]            report for each repo (default: current repo)
//! mic_check --no-fetch           skip `git fetch` (offline / fast)
//! mic_check --note "text" [repo] append an optional leftover (tagged with that repo, default: current)
//! mic_check --leftovers          print the leftovers file
//! mic_check --since <sha|time> [repo]  also list this session's commits since that sha or time (for squash)
//! ```
//!
//! Every line is tagged:
//!
//! ```text
//! OK    nothing to do
//! FIX   mandatory: the session is NOT done until this is gone
//! WARN  look at it; fix or note it as a leftover
//! ```
//!
//! Last line: VERDICT: CLEAN (exit 0) | NOT CLEAN (n mandatory) (exit 1)

use agent_config::paths;
use agent_config::pycompat::is_executable;
use agent_config::{quiet_print as print, quiet_println as println};
use clap::Parser;
use clap::error::{ContextKind, ErrorKind};
use regex::Regex;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::LazyLock;

const USAGE: &str = "\
mic_check: is this repo really done? Read-only (except `git fetch`).
  mic_check [repo...]            report for each repo (default: current repo)
  mic_check --no-fetch           skip `git fetch` (offline / fast)
  mic_check --note \"text\" [repo] append an optional leftover (tagged with that repo, default: current)
  mic_check --leftovers          print the leftovers file
  mic_check --since <sha|time> [repo]  also list this session's commits since that sha or time (for squash)
Every line is tagged:
  OK    nothing to do
  FIX   mandatory — the session is NOT done until this is gone
  WARN  look at it; fix or note it as a leftover
Last line: VERDICT: CLEAN (exit 0) | NOT CLEAN (n mandatory) (exit 1)
";

fn leftovers_path() -> PathBuf {
    std::env::var_os("MIC_LEFTOVERS")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| paths::home().join(".claude").join("leftovers.md"))
}

/// Run `git -C dir args`: (success, stdout with trailing newlines cut).
fn git(dir: &Path, args: &[&str]) -> (bool, String) {
    match Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).trim_end_matches(['\n', '\r']).to_string()),
        Err(_) => (false, String::new()),
    }
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    git(dir, args).1
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    git(dir, args).0
}

fn lines_of(s: &str) -> Vec<&str> {
    s.lines().filter(|l| !l.is_empty()).collect()
}

fn count_lines(s: &str) -> usize {
    lines_of(s).len()
}

fn on_path(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else { return false };
    let exts: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::split_paths(&paths).any(|d| exts.iter().any(|e| d.join(format!("{name}{e}")).is_file()))
}

/// Today's local date, `YYYY-MM-DD`.
fn today() -> String {
    jiff::Zoned::now().strftime("%F").to_string()
}

fn base_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Append a leftover under today's date and the repo name of `dir`.
fn note(text: &str, dir: &Path) -> i32 {
    let left = leftovers_path();
    let top = git_out(dir, &["rev-parse", "--show-toplevel"]);
    let name = if top.is_empty() { base_name(dir) } else { base_name(Path::new(&top)) };
    if let Some(parent) = left.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if !left.is_file() {
        let _ = std::fs::write(&left, "# Leftovers\n\nOptional fixes saved by ps:mic-drop. Delete a line when done.\n");
    }
    let line = format!("- [ ] {} · **{}** · {}\n", today(), name, text);
    let res = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&left)
        .and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = res {
        eprintln!("mic_check: cannot write {}: {e}", left.display());
        return 1;
    }
    println!("saved to {}", left.display());
    0
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a fixed pattern")
}

/// Each name with the regex that finds it as a `package.json` script key.
static PACKAGE_SCRIPTS: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
    ["typecheck", "lint", "test", "check", "build"].map(|t| (t, regex(&format!(r#""{t}"\s*:"#)))).into()
});
/// Each name with the regex that finds it as a `Makefile` target.
static MAKE_TARGETS: LazyLock<Vec<(&str, Regex)>> =
    LazyLock::new(|| ["check", "lint", "test", "verify"].map(|t| (t, regex(&format!(r"(?m)^{t}:")))).into());
static MISE_TASKS: LazyLock<Regex> = LazyLock::new(|| regex(r"(?m)^\[tasks\.(check|lint|test)\]"));
static SHA_LIKE: LazyLock<Regex> = LazyLock::new(|| regex(r"^[0-9a-fA-F]{7,40}$"));
static BAD_CI: LazyLock<Regex> = LazyLock::new(|| regex(r"^completed\|(failure|cancelled|timed_out|startup_failure)"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| regex(r"\s+"));

/// The masking steps of `safe_args`, in order: pattern and replacement.
static MASKS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    let words = "(key|token|secret|passw|pwd|auth|credential|cookie|session)";
    let steps = [
        (r"(://)[^/@\s]+@".to_string(), "${1}***@"),
        (format!(r"(?i)(--?[A-Za-z0-9_.-]*{words}[A-Za-z0-9_.-]*)(=|\s+)[^\s]+"), "${1}${3}***"),
        (format!(r"(?i)([A-Za-z0-9_.-]*{words}[A-Za-z0-9_.-]*)=[^\s]+"), "${1}=***"),
        (r"(?i)(bearer|basic)\s+[^\s]+".to_string(), "${1} ***"),
        (
            r"(crsr_|sk-|sk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|AIza|ya29\.)[A-Za-z0-9_.-]+"
                .to_string(),
            "${1}***",
        ),
        (r"[A-Za-z0-9_+]{32,}".to_string(), "***"),
    ];
    steps.into_iter().map(|(pat, rep)| (regex(&pat), rep)).collect()
});

/// The verification commands this repo offers.
fn checks_for(r: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let file = |n: &str| r.join(n).is_file();
    if file("package.json") {
        let mut pm = "npm";
        if file("pnpm-lock.yaml") {
            pm = "pnpm";
        }
        if file("bun.lockb") || file("bun.lock") {
            pm = "bun";
        }
        if file("yarn.lock") {
            pm = "yarn";
        }
        let text = std::fs::read_to_string(r.join("package.json")).unwrap_or_default();
        for (t, re) in PACKAGE_SCRIPTS.iter() {
            if re.is_match(&text) {
                out.push(format!("{pm} run {t}"));
            }
        }
    }
    if file("Makefile") {
        let text = std::fs::read_to_string(r.join("Makefile")).unwrap_or_default();
        for (t, re) in MAKE_TARGETS.iter() {
            if re.is_match(&text) {
                out.push(format!("make {t}"));
            }
        }
    }
    let justfile = ["justfile", "Justfile"].into_iter().map(|n| r.join(n)).find(|p| p.is_file());
    if let Some(jf) = justfile {
        let summary = Command::new("just")
            .arg("--justfile")
            .arg(&jf)
            .arg("--summary")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let words: Vec<&str> = summary.split_whitespace().collect();
        for t in ["check", "lint", "test", "verify"] {
            if words.contains(&t) {
                out.push(format!("just {t}"));
            }
        }
    }
    if file("mise.toml") {
        let text = std::fs::read_to_string(r.join("mise.toml")).unwrap_or_default();
        if MISE_TASKS.is_match(&text) {
            out.push("mise tasks ls  # then run check/lint/test".into());
        }
    }
    if file(".pre-commit-config.yaml") {
        out.push("pre-commit run --all-files".into());
    }
    if file("Cargo.toml") {
        out.push("cargo clippy --all-targets && cargo nextest run".into());
    }
    if file("go.mod") {
        out.push("golangci-lint run && go test ./...".into());
    }
    if file("pyproject.toml") {
        out.push("uv run ruff check && uv run ruff format --check && uv run pytest".into());
    }
    if file("biome.json") || file("biome.jsonc") {
        out.push("biome check .".into());
    }
    let hp = git_out(r, &["config", "core.hooksPath"]);
    let own_hook = !hp.is_empty() && is_executable(&r.join(&hp).join("pre-commit"));
    let default_hook = {
        let p = git_out(r, &["rev-parse", "--git-path", "hooks/pre-commit"]);
        !p.is_empty() && is_executable(&r.join(p))
    };
    if own_hook || default_hook {
        out.push("git hook run pre-commit  # this repo's own pre-commit hook".into());
    }
    if workflows(r) {
        out.push("(CI) gh run list --limit 3".into());
    }
    out
}

/// True when `.github/workflows/*.y*ml` matches something.
fn workflows(root: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(root.join(".github").join("workflows")) else { return false };
    rd.filter_map(Result::ok).any(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        n.ends_with("ml") && n[..n.len() - 2].contains(".y")
    })
}

/// Mask what could be a key in a command line (a flag value, KEY=VALUE, a password in a URL).
/// Over-masking is fine.
fn safe_args(raw: &str) -> String {
    let mut s: String = raw.chars().map(|c| if c == '\n' || c == '\t' { ' ' } else { c }).collect();
    for (re, rep) in MASKS.iter() {
        s = re.replace_all(&s, *rep).into_owned();
    }
    SPACES.replace_all(&s, " ").trim_matches(' ').to_string()
}

/// "pid name: args" for one process, args masked and cut to 100 chars; nothing for mic_check itself.
fn proc_line(pid: &str) -> Option<String> {
    let ps = |field: &str| {
        Command::new("ps")
            .args(["-o", field, "-p", pid])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_string())
    };
    let args = ps("args=")?;
    if args.is_empty() || args.contains("mic-check") || args.contains("mic_check") {
        return None;
    }
    let name = ps("ucomm=").unwrap_or_default();
    let mut args = safe_args(&args);
    if args.chars().count() > 100 {
        args = format!("{}…", args.chars().take(100).collect::<String>());
    }
    let name = if name.is_empty() { "?" } else { name.as_str() };
    Some(format!("{pid} {name}: {args}"))
}

struct Run {
    fetch: bool,
    since: String,
    fixes: usize,
}

fn ok(msg: &str) {
    println!("  OK    {msg}");
}

fn warn(msg: &str) {
    println!("  WARN  {msg}");
}

/// The items, each followed by one space.
fn each_then_space<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let mut s = String::new();
    for item in items {
        s.push_str(item);
        s.push(' ');
    }
    s
}

impl Run {
    fn fix(&mut self, msg: &str) {
        println!("  FIX   {msg}");
        self.fixes += 1;
    }

    fn report(&mut self, dir: &Path) {
        let top = git_out(dir, &["rev-parse", "--show-toplevel"]);
        if top.is_empty() {
            println!("== {} ==", dir.display());
            warn("not a git repo — skipped");
            return;
        }
        let root = PathBuf::from(&top);
        let g = |args: &[&str]| git_out(&root, args);
        let gd = PathBuf::from(g(&["rev-parse", "--absolute-git-dir"]));
        println!("== {}  ({}) ==", base_name(&root), top);

        if self.fetch && !git_ok(&root, &["fetch", "--quiet", "--prune"]) {
            warn("git fetch failed — remote state may be old");
        }

        let branch = g(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let mut main = g(&["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]);
        if let Some(m) = main.strip_prefix("origin/") {
            main = m.to_string();
        }
        if main.is_empty() {
            main = if git_ok(&root, &["show-ref", "--verify", "--quiet", "refs/remotes/origin/main"]) {
                "main"
            } else {
                "master"
            }
            .into();
        }

        // 1. Merge / rebase in progress, conflicts
        let mut state = "";
        if gd.join("MERGE_HEAD").is_file() {
            state = "merge";
        }
        if gd.join("rebase-merge").is_dir() || gd.join("rebase-apply").is_dir() {
            state = "rebase";
        }
        if gd.join("CHERRY_PICK_HEAD").is_file() {
            state = "cherry-pick";
        }
        if gd.join("REVERT_HEAD").is_file() {
            state = "revert";
        }
        if !state.is_empty() {
            self.fix(&format!("a {state} is half done — finish it or abort it"));
        }
        let unmerged = g(&["diff", "--name-only", "--diff-filter=U"]);
        let conflicts = count_lines(&unmerged);
        if conflicts > 0 {
            let names = each_then_space(lines_of(&unmerged).into_iter().take(5));
            self.fix(&format!("{conflicts} file(s) with merge conflicts: {names}"));
        }
        let marked = g(&["grep", "-l", "-E", "^(<<<<<<<|>>>>>>>) "]);
        let markers = each_then_space(lines_of(&marked).into_iter().take(3));
        if !markers.is_empty() {
            self.fix(&format!("conflict markers in files: {markers}"));
        }
        if state.is_empty() && conflicts == 0 {
            ok("no conflicts, no half-done merge/rebase");
        }

        // 2. Branch
        if branch.is_empty() {
            self.fix("detached HEAD — not on a branch");
        } else if branch == main {
            ok(&format!("on {main}"));
        } else {
            warn(&format!("on branch '{branch}', not '{main}' — merge it, or open a PR (ask the user)"));
        }

        // 3. Uncommitted work
        let staged = count_lines(&g(&["diff", "--cached", "--name-only"]));
        let unstaged = count_lines(&g(&["diff", "--name-only"]));
        let untracked = count_lines(&g(&["ls-files", "--others", "--exclude-standard"]));
        if staged + unstaged + untracked == 0 {
            ok("working tree clean");
        } else {
            self.fix(&format!("not committed: {staged} staged, {unstaged} changed, {untracked} new file(s)"));
            for l in lines_of(&g(&["status", "--short"])).iter().take(8) {
                println!("          {l}");
            }
        }

        // 4. Pushed?
        if !branch.is_empty() {
            let up = g(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]);
            if up.is_empty() {
                self.fix(&format!("branch '{branch}' has no upstream — never pushed"));
            } else {
                let counts = g(&["rev-list", "--left-right", "--count", &format!("{up}...HEAD")]);
                let mut it = counts.split_whitespace().map(|n| n.parse::<usize>().unwrap_or(0));
                let (behind, ahead) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
                if ahead > 0 {
                    self.fix(&format!("{ahead} commit(s) not pushed to {up}"));
                }
                if behind > 0 {
                    self.fix(&format!("{behind} commit(s) on {up} not pulled (pull --rebase, then re-check)"));
                }
                if ahead == 0 && behind == 0 {
                    ok(&format!("in sync with {up} ({})", g(&["rev-parse", "--short", "HEAD"])));
                }
            }
        }
        let origin_main = format!("refs/remotes/origin/{main}");
        if !branch.is_empty() && branch != main && git_ok(&root, &["show-ref", "--verify", "--quiet", &origin_main]) {
            let notin: usize = g(&["rev-list", "--count", &format!("origin/{main}..HEAD")]).parse().unwrap_or(0);
            if notin > 0 {
                warn(&format!("{notin} commit(s) on '{branch}' are not in origin/{main} yet"));
            }
        }

        // 4b. This session's commits (squash mode): only with --since <sha|time>
        if !self.since.is_empty() && !branch.is_empty() {
            self.since_section(&root);
        }

        // 5. Other local branches with unpushed work
        let refs = g(&["for-each-ref", "--format=%(refname:short)|%(upstream:short)|%(upstream:track)", "refs/heads"]);
        for l in lines_of(&refs) {
            let mut p = l.splitn(3, '|');
            let (b, u, t) = (p.next().unwrap_or(""), p.next().unwrap_or(""), p.next().unwrap_or(""));
            if b == branch {
                continue;
            }
            if u.is_empty() {
                if !git_ok(&root, &["merge-base", "--is-ancestor", b, &format!("origin/{main}")]) {
                    warn(&format!("branch '{b}' is local only and not merged"));
                }
            } else if t.contains("ahead") {
                warn(&format!("branch '{b}' {t} vs {u}"));
            }
        }

        // 6. Stashes and extra worktrees
        let stashes = g(&["stash", "list"]);
        let s = count_lines(&stashes);
        if s > 0 {
            warn(&format!("{s} stash(es) — forgotten work? ({})", lines_of(&stashes)[0]));
        }
        let wt = g(&["worktree", "list"]);
        let w = count_lines(&wt);
        if w > 1 {
            let first_words = lines_of(&wt).into_iter().skip(1).map(|l| l.split_whitespace().next().unwrap_or(""));
            let paths = each_then_space(first_words);
            warn(&format!("{} extra worktree(s): {paths}", w - 1));
        }

        // 7. CI for the latest pushed commit
        if on_path("gh") && workflows(&root) && !branch.is_empty() {
            self.ci_section(&root);
        }

        // 8. Verification checks this repo offers (the agent runs them)
        let c = checks_for(&root);
        if c.is_empty() {
            warn("no known check command found — ask how this repo is verified, or check README/AGENTS.md");
        } else {
            println!("  RUN   verification checks for this repo:");
            for l in &c {
                println!("          {l}");
            }
        }

        // 9. Stray processes started from this repo (dev servers, watchers): one short, masked
        // line each, never the raw command line.
        let pids = Command::new("pgrep")
            .arg("-f")
            .arg(&top)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let procs: Vec<String> = pids.split_whitespace().filter_map(proc_line).take(5).collect();
        if !procs.is_empty() {
            warn("processes still running from this repo:");
            for p in &procs {
                println!("          {p}");
            }
        }
        println!();
    }

    fn since_section(&self, root: &Path) {
        let g = |args: &[&str]| git_out(root, args);
        let since = self.since.as_str();
        let sha_like = SHA_LIKE.is_match(since);
        let mut base = String::new();
        if sha_like {
            // a sha: must be in this branch, never read as a date
            if git_ok(root, &["merge-base", "--is-ancestor", since, "HEAD"]) {
                base = g(&["rev-parse", "--verify", "--quiet", &format!("{since}^{{commit}}^")]); // inclusive: the base is the parent
            } else {
                warn(&format!("--since {since} is not an ancestor of HEAD here: no squash range for this repo"));
            }
        } else {
            let revs = g(&["rev-list", &format!("--since={since}"), "HEAD"]);
            if let Some(first) = lines_of(&revs).last() {
                base = g(&["rev-parse", "--verify", "--quiet", &format!("{first}^")]);
            }
        }
        let rng = if base.is_empty() { String::new() } else { format!("{base}..HEAD") };
        let n: usize = if rng.is_empty() { 0 } else { g(&["rev-list", "--count", &rng]).parse().unwrap_or(0) };
        if n == 0 {
            ok(&format!("no session commits since {since} (or the base is not found)"));
            return;
        }
        let me = g(&["config", "user.email"]);
        let foreign = g(&["log", "--format=%ae", &rng]).lines().filter(|l| *l != me).count();
        let short = g(&["rev-parse", "--short", &base]);
        ok(&format!("{n} session commit(s) on top of base {short} (squash: git reset --soft {short})"));
        for l in lines_of(&g(&["log", "--format=          %h %an  %s", &rng])).iter().take(8) {
            println!("{l}");
        }
        if foreign > 0 {
            warn(&format!("{foreign} of those commit(s) have another author — do not squash them"));
        }
        if !g(&["rev-list", "--merges", &rng]).is_empty() {
            warn("merge commit(s) in that range — do not squash");
        }
    }

    fn ci_section(&mut self, root: &Path) {
        let head = git_out(root, &["rev-parse", "HEAD"]);
        let out = Command::new("gh")
            .current_dir(root)
            .args(["run", "list", "--commit", &head, "--json", "name,status,conclusion", "-q"])
            .arg(r#".[] | "\(.status)|\(.conclusion)|\(.name)""#)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let runs = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string(),
            _ => {
                warn("could not read CI (gh auth?)");
                return;
            }
        };
        if runs.is_empty() {
            let short = git_out(root, &["rev-parse", "--short", "HEAD"]);
            warn(&format!("no CI run found for {short} (not pushed yet, or CI not triggered)"));
            return;
        }
        let bad: BTreeSet<&str> =
            runs.lines().filter(|l| BAD_CI.is_match(l)).filter_map(|l| l.split('|').nth(2)).collect();
        let pending = runs.lines().filter(|l| !l.starts_with("completed")).count();
        if !bad.is_empty() {
            let names = each_then_space(bad.iter().copied());
            self.fix(&format!("CI failed on HEAD: {names}"));
        }
        if pending > 0 {
            warn(&format!("{pending} CI run(s) still running on HEAD — wait, then re-check"));
        }
        if bad.is_empty() && pending == 0 {
            ok("CI green on HEAD");
        }
    }
}

/// The flags. Help is printed from `USAGE`, so clap's own help flag is off.
#[derive(Parser)]
#[command(name = "mic_check", disable_help_flag = true, disable_version_flag = true)]
struct Cli {
    /// Print the usage text.
    #[arg(short = 'h', long = "help")]
    help: bool,
    /// Skip `git fetch`.
    #[arg(long = "no-fetch")]
    no_fetch: bool,
    /// Also list this session's commits since a sha or a time.
    #[arg(long, value_name = "SHA|TIME")]
    since: Option<String>,
    /// Append a leftover.
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,
    /// Print the leftovers file.
    #[arg(long)]
    leftovers: bool,
    /// Repos to report on (default: the current repo).
    repos: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // A flag that needs a value and has none exits 1, other bad input 2.
        Err(e) if e.kind() == ErrorKind::InvalidValue => {
            let flag = e.get(ContextKind::InvalidArg).map(ToString::to_string).unwrap_or_default();
            let need = if flag.contains("--since") { "--since needs a sha or time" } else { "--note needs text" };
            eprintln!("mic_check: {need}");
            return ExitCode::from(1);
        }
        Err(e) => e.exit(),
    };
    if cli.since.as_deref() == Some("") {
        eprintln!("mic_check: --since needs a sha or time");
        return ExitCode::from(1);
    }
    if cli.note.as_deref() == Some("") {
        eprintln!("mic_check: --note needs text");
        return ExitCode::from(1);
    }
    if cli.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if cli.leftovers {
        let left = leftovers_path();
        match std::fs::read_to_string(&left) {
            Ok(t) => print!("{t}"),
            Err(_) => println!("no leftovers file yet ({})", left.display()),
        }
        return ExitCode::SUCCESS;
    }
    let mut run = Run { fetch: !cli.no_fetch, since: cli.since.unwrap_or_default(), fixes: 0 };
    let mut repos = cli.repos;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(text) = cli.note {
        let dir = repos.first().cloned().unwrap_or(cwd);
        return ExitCode::from(note(&text, &dir) as u8);
    }
    if repos.is_empty() {
        repos.push(cwd);
    }
    for r in &repos {
        run.report(r);
    }
    let left = leftovers_path();
    if let Ok(text) = std::fs::read_to_string(&left) {
        let open = text.lines().filter(|l| l.starts_with("- [ ]")).count();
        println!("leftovers file: {} ({open} open)", left.display());
    }
    if run.fixes == 0 {
        println!("VERDICT: CLEAN");
        ExitCode::SUCCESS
    } else {
        println!("VERDICT: NOT CLEAN ({} mandatory)", run.fixes);
        ExitCode::from(1)
    }
}
