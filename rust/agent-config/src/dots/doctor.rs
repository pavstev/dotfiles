//! `dots doctor`: one line per check, `PASS <name>` or `FAIL <name>: <why>`. Exit 1 on any FAIL.
//!
//! It only reads. The guard check runs the guard functions in this process with canned input, so
//! it needs no setting of the machine and never opens a private file. A FAIL line names the fix.

use super::os;
use crate::guard_checks::{self as gc, Verdict};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

/// The marker that `dots` writes into every file it generates.
const MARK: &str = "generated-by: dotfiles";

/// One finished check: its name and `Ok`, or the reason it failed.
pub struct Check {
    /// The check name printed after `PASS` or `FAIL`.
    pub name: &'static str,
    /// `Ok`, or why the check failed (with the fix).
    pub result: Result<(), String>,
}

/// Every check, in the order they print. `repo` is the dotfiles checkout, `home` is `$HOME`.
pub fn checks(repo: &Path, home: &Path) -> Vec<Check> {
    let c = |name, result| Check { name, result };
    vec![
        c("links", links(repo, home)),
        c("settings-built", settings_built(repo, home)),
        c("bins-fresh", bins_fresh(repo)),
        c("guard-selftest", guard_selftest(repo)),
        c("presets", presets(repo, home)),
        c("generated", generated(repo, home)),
        c("timezone", timezone(home)),
        c("no-leak", no_leak(repo, home)),
    ]
}

/// Print the checks. Exit 1 when one failed.
pub fn run() -> i32 {
    let (repo, home) = (super::repo(), os::home());
    let mut failed = 0;
    for c in checks(&repo, &home) {
        match c.result {
            Ok(()) => println!("PASS {}", c.name),
            Err(why) => {
                failed += 1;
                println!("FAIL {}: {}", c.name, why.replace('\n', " "));
            }
        }
    }
    i32::from(failed > 0)
}

fn join(items: &[String]) -> String {
    items.join(", ")
}

// --- links -----------------------------------------------------------------------

/// `dst` is a link that points at a file or folder inside `root`.
fn points_into(dst: &Path, root: &Path) -> bool {
    if !os::is_symlink(dst) {
        return false;
    }
    let Ok(target) = fs::read_link(dst) else { return false };
    let target = if target.is_relative() { dst.parent().map(|p| p.join(&target)).unwrap_or(target) } else { target };
    if !target.exists() {
        return false;
    }
    if target.starts_with(root) {
        return true;
    }
    matches!((fs::canonicalize(&target), fs::canonicalize(root)), (Ok(a), Ok(b)) if a.starts_with(&b))
}

/// Every `links()` item and the Claude items point into the repo.
fn links(repo: &Path, home: &Path) -> Result<(), String> {
    let mut bad: Vec<String> = Vec::new();
    let home_src = repo.join("home");
    for item in super::links() {
        // an item the repo does not hold is skipped by `dots` too
        if home_src.join(item).exists() && !points_into(&home.join(item), &home_src) {
            bad.push(format!("~/{item}"));
        }
    }
    let claude_src = repo.join("claude");
    for item in ["agents", "hooks", "output-styles", "CLAUDE.md"] {
        if claude_src.join(item).exists() && !points_into(&home.join(".claude").join(item), &claude_src) {
            bad.push(format!("~/.claude/{item}"));
        }
    }
    if bad.is_empty() { Ok(()) } else { Err(format!("not a link into the repo: {}. Run: dots", join(&bad))) }
}

// --- settings-built --------------------------------------------------------------

/// The `command` strings of every hook in a settings file.
fn hook_commands(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == "command"
                    && let Some(s) = x.as_str()
                {
                    out.push(s.to_string());
                } else {
                    hook_commands(x, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| hook_commands(x, out)),
        _ => {}
    }
}

/// `~/.claude/settings.json` exists and holds each guard hook of the repo's `claude/settings.json`,
/// and each of them runs `rust-bin`. Other hooks (the user's own) are not
/// checked: `dots` cannot change them.
fn settings_built(repo: &Path, home: &Path) -> Result<(), String> {
    let p = home.join(".claude").join("settings.json");
    let text = fs::read_to_string(&p).map_err(|_| "~/.claude/settings.json is missing. Run: dots".to_string())?;
    let v: Value =
        serde_json::from_str(&text).map_err(|_| "~/.claude/settings.json is not valid JSON. Run: dots".to_string())?;
    let base = fs::read_to_string(repo.join("claude").join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .ok_or_else(|| "claude/settings.json of the repo is missing or not valid JSON".to_string())?;
    let (mut want, mut have) = (Vec::new(), Vec::new());
    if let Some(h) = base.get("hooks") {
        hook_commands(h, &mut want);
    }
    if let Some(h) = v.get("hooks") {
        hook_commands(h, &mut have);
    }
    if want.is_empty() {
        return Err("claude/settings.json of the repo has no hooks".into());
    }
    let bad: Vec<String> = want
        .iter()
        .filter(|w| !w.contains("rust-bin") || !have.contains(w))
        .map(|c| c.chars().take(40).collect())
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("a guard hook is missing or does not run rust-bin ({}). Run: dots", join(&bad)))
    }
}

// --- bins-fresh ------------------------------------------------------------------

/// The `name = "..."` of each `[[bin]]` table.
fn bin_names(cargo_toml: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_bin = false;
    for line in cargo_toml.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_bin = l == "[[bin]]";
            continue;
        }
        if !in_bin {
            continue;
        }
        if let Some(rest) = l.strip_prefix("name")
            && let Some(v) = rest.trim_start().strip_prefix('=')
        {
            names.push(v.trim().trim_matches('"').to_string());
        }
    }
    names
}

/// The newest modification time of the sources `scripts/agent-config` watches.
fn newest_source(crate_dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut see = |p: &Path| {
        if let Ok(t) = fs::metadata(p).and_then(|m| m.modified())
            && newest.is_none_or(|n| t > n)
        {
            newest = Some(t);
        }
    };
    see(&crate_dir.join("Cargo.toml"));
    see(&crate_dir.join("Cargo.lock"));
    for sub in ["src", "themes"] {
        for e in walkdir::WalkDir::new(crate_dir.join(sub))
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            see(e.path());
        }
    }
    newest
}

/// Each `[[bin]]` of the crate is built and not older than its sources. `claude/hooks/rust-bin`
/// never checks this, so this is the one place a stale binary shows.
fn bins_fresh(repo: &Path) -> Result<(), String> {
    let crate_dir = repo.join("rust").join("agent-config");
    let toml = fs::read_to_string(crate_dir.join("Cargo.toml"))
        .map_err(|_| "rust/agent-config/Cargo.toml is missing".to_string())?;
    let names = bin_names(&toml);
    if names.is_empty() {
        return Err("Cargo.toml lists no bins".into());
    }
    let newest = newest_source(&crate_dir);
    let (mut missing, mut stale) = (Vec::new(), Vec::new());
    for n in names {
        let file = if os::is_windows() { format!("{n}.exe") } else { n.clone() };
        match fs::metadata(crate_dir.join("target").join("release").join(file)).and_then(|m| m.modified()) {
            Err(_) => missing.push(n),
            Ok(t) if newest.is_some_and(|s| s > t) => stale.push(n),
            Ok(_) => {}
        }
    }
    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!("not built: {}", join(&missing)));
    }
    if !stale.is_empty() {
        parts.push(format!("older than the sources: {}", join(&stale)));
    }
    if parts.is_empty() {
        Ok(())
    } else {
        Err(format!("{}. Run: dots (or scripts/agent-config --bins)", parts.join("; ")))
    }
}

// --- guard-selftest --------------------------------------------------------------

/// `got` must be the kind `want` names; otherwise the failure text says what the guard did.
fn expect(what: &str, got: Option<Verdict>, want: &str) -> Option<String> {
    let kind = match &got {
        Some(Verdict::Allow) => "allow",
        Some(Verdict::Ask(_)) => "ask",
        Some(Verdict::Deny(_)) => "deny",
        None => "crash",
    };
    if kind == want { None } else { Some(format!("{what} gave {kind}, wanted {want}")) }
}

/// Canned calls through the guard checks. The private word is made up, so the check needs no setting.
fn guard_selftest(repo: &Path) -> Result<(), String> {
    const WORD: &str = "doctorprivateword";
    let run = |f: &dyn Fn() -> Verdict| std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok();
    let words = vec![WORD.to_string()];
    let send = json!({"command": format!("gh pr comment 1 --body \"{WORD}\"")});
    let ls = json!({"command": "ls"});
    let local = json!({"file_path": "~/.config/zsh/local.zsh"});
    let cwd: PathBuf = repo.to_path_buf();
    let problems: Vec<String> = [
        // BSD sed -i (macOS rules) is denied
        expect("sed -i on macOS", run(&|| gc::g14_sed("sed -i 's/a/b/' file.txt", true)), "deny"),
        expect("a read of local.zsh", run(&|| gc::g15_secrets("Read", &local)), "deny"),
        expect("a send with a private word", run(&|| gc::g16_outbound("Bash", &send, &cwd, &words)), "ask"),
        expect("ls (sed check)", run(&|| gc::g14_sed("ls", true)), "allow"),
        expect("ls (secrets check)", run(&|| gc::g15_secrets("Bash", &ls)), "allow"),
        expect("ls (outbound check)", run(&|| gc::g16_outbound("Bash", &ls, &cwd, &words)), "allow"),
        expect("ls (attribution check)", run(&|| gc::g17_attribution("ls", &cwd)), "allow"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{}. Run: dots, then scripts/agent-config --bins", problems.join("; ")))
    }
}

// --- presets ---------------------------------------------------------------------

/// The `~/.claude/rules/preset-*.md` links equal the presets that are on.
fn presets(repo: &Path, home: &Path) -> Result<(), String> {
    let on = crate::presets::enabled_quiet(home, &crate::presets::profile(home));
    let src = repo.join("claude").join("presets");
    // `repo` can be spelled another way than the links (a `..` in the exe path, a symlinked folder)
    let src_real = fs::canonicalize(&src).unwrap_or_else(|_| src.clone());
    let want: BTreeSet<String> = on.into_iter().filter(|n| src.join(n).join("rules.md").is_file()).collect();
    let mut have = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(home.join(".claude").join("rules")) {
        for e in entries.filter_map(Result::ok) {
            let file = e.file_name().to_string_lossy().into_owned();
            let Some(name) = file.strip_prefix("preset-").and_then(|r| r.strip_suffix(".md")) else { continue };
            if os::is_symlink(&e.path())
                && fs::read_link(e.path())
                    .is_ok_and(|t| t.starts_with(&src) || fs::canonicalize(&t).is_ok_and(|r| r.starts_with(&src_real)))
            {
                have.insert(name.to_string());
            }
        }
    }
    let missing: Vec<String> = want.difference(&have).cloned().collect();
    let extra: Vec<String> = have.difference(&want).cloned().collect();
    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!("no link for: {}", join(&missing)));
    }
    if !extra.is_empty() {
        parts.push(format!("links of presets that are off: {}", join(&extra)));
    }
    if parts.is_empty() { Ok(()) } else { Err(format!("{}. Run: dots", parts.join("; "))) }
}

// --- generated -------------------------------------------------------------------

/// The generated files carry the marker: the Cursor rule when it exists, and the Opencode rules
/// and guard plugin when the Opencode config folder exists.
fn generated(repo: &Path, home: &Path) -> Result<(), String> {
    let marked = |p: &Path| fs::read_to_string(p).is_ok_and(|t| t.contains(MARK));
    let mut bad: Vec<String> = Vec::new();
    let cursor = home.join(".cursor").join("rules").join("claude-global.mdc");
    if cursor.exists() && !marked(&cursor) {
        bad.push("~/.cursor/rules/claude-global.mdc has no marker".into());
    }
    let oc = home.join(".config").join("opencode");
    if oc.is_dir() {
        if !marked(&oc.join("AGENTS.md")) {
            bad.push("~/.config/opencode/AGENTS.md is missing or has no marker".into());
        }
        // the plugin is written only while the launcher exists
        if repo.join("claude").join("hooks").join("rust-bin").is_file()
            && !marked(&oc.join("plugins").join("dots-guard.ts"))
        {
            bad.push("~/.config/opencode/plugins/dots-guard.ts is missing or has no marker".into());
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("{}. If the file is yours, move it away. Then run: dots", bad.join("; ")))
    }
}

// --- timezone --------------------------------------------------------------------

/// `TIMEZONE` is empty or names a zone file (macOS and Linux).
fn timezone(home: &Path) -> Result<(), String> {
    let tz = crate::setting(home, "TIMEZONE");
    let tz = tz.trim();
    if tz.is_empty() || os::is_windows() {
        return Ok(());
    }
    let zone = Path::new("/usr/share/zoneinfo").join(tz);
    if !tz.contains("..") && !tz.starts_with('/') && zone.is_file() {
        return Ok(());
    }
    Err(format!(
        "TIMEZONE \"{tz}\" is no zone in /usr/share/zoneinfo. Use a name like Area/City, or empty. Fix it: dots settings"
    ))
}

// --- no-leak ---------------------------------------------------------------------

/// `path` with every span that holds a private word (case-insensitive, whole words only for a
/// short one: the rule of the git hook and G16) replaced by `***`, so a printed path never shows
/// the word.
fn mask(path: &str, words: &[String]) -> String {
    let chars: Vec<char> = path.chars().collect();
    // the lower-case text, with the index of the original char for each of its bytes
    let mut lower = String::new();
    let mut from: Vec<usize> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        for l in c.to_lowercase() {
            lower.push(l);
            from.extend(std::iter::repeat_n(i, l.len_utf8()));
        }
    }
    let mut hidden = vec![false; chars.len()];
    for w in words {
        for (s, e) in gc::word_spans(&lower, w) {
            for &i in &from[s..e] {
                hidden[i] = true;
            }
        }
    }
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if !hidden[i] {
            out.push(*c);
        } else if i == 0 || !hidden[i - 1] {
            out.push_str("***");
        }
    }
    out
}

/// No private word in a tracked file or file name. Prints `file:line`, never the word: the word
/// is masked in every printed path.
fn no_leak(repo: &Path, home: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-files", "-z"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o,
        _ => return Err("cannot list the tracked files (is the repo a git checkout?)".into()),
    };
    let words = gc::private_words(home);
    if words.is_empty() {
        return Ok(());
    }
    let mut hits: Vec<String> = Vec::new();
    for name in String::from_utf8_lossy(&out.stdout).split('\0').filter(|n| !n.is_empty()) {
        let lower = name.to_lowercase();
        let shown_name = mask(name, &words);
        if words.iter().any(|w| gc::word_hit(&lower, w)) {
            hits.push(format!("{shown_name} (file name)"));
        }
        let path = repo.join(name);
        if fs::metadata(&path).map_or(true, |m| !m.is_file() || m.len() > 4 << 20) {
            continue;
        }
        let Ok(text) = fs::read(&path).map(|b| String::from_utf8_lossy(&b).to_lowercase()) else { continue };
        for (i, line) in text.lines().enumerate() {
            if words.iter().any(|w| gc::word_hit(line, w)) {
                hits.push(format!("{shown_name}:{}", i + 1));
            }
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    let shown: Vec<String> = hits.iter().take(5).cloned().collect();
    let more = if hits.len() > 5 { format!(" and {} more", hits.len() - 5) } else { String::new() };
    Err(format!("{} place(s) hold a private word: {}{}. Remove it before you commit", hits.len(), join(&shown), more))
}
