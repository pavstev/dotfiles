//! `dots doctor`: one line per check, `PASS <name>` or `FAIL <name>: <why>`. Exit 1 on any FAIL.
//!
//! It only reads. A FAIL line names the fix. The AI harness (rules, skills, hooks) is the ps
//! plugin (pavstev/agent-kit): the `plugin` check only says it is installed and has its binaries.

use super::leak as gc;
use super::os;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::SystemTime;

/// One finished check: its name and `Ok`, or the reason it failed.
#[derive(Debug)]
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
        c("settings-built", settings_built(home)),
        c("bins-fresh", bins_fresh(repo)),
        c("plugin", plugin(home)),
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
    let Ok(target) = fs::read_link(dst) else {
        return false;
    };
    let target = if target.is_relative() {
        dst.parent().map(|p| p.join(&target)).unwrap_or(target)
    } else {
        target
    };
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
    // The AI files come from the ps plugin now: an old link into claude/ would load them twice.
    for p in super::agent_kit::old_links(repo, home) {
        let rel = p.strip_prefix(home).unwrap_or(&p);
        bad.push(format!("~/{} (an old link)", rel.display()));
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "not a link into the repo: {}. Run: dots",
            join(&bad)
        ))
    }
}

// --- settings-built --------------------------------------------------------------

/// `~/.claude/settings.json` exists, turns the ps plugin on and holds no old guard hook (the
/// plugin runs the guard; a second copy in the settings would run it twice).
fn settings_built(home: &Path) -> Result<(), String> {
    let p = home.join(".claude").join("settings.json");
    let text = fs::read_to_string(&p)
        .map_err(|_| "~/.claude/settings.json is missing. Run: dots".to_string())?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| "~/.claude/settings.json is not valid JSON. Run: dots".to_string())?;
    let on = v
        .get("enabledPlugins")
        .and_then(|p| p.get(super::agent_kit::PLUGIN));
    if on != Some(&Value::Bool(true)) {
        return Err(format!(
            "the ps plugin ({}) is not on in ~/.claude/settings.json. Run: dots",
            super::agent_kit::PLUGIN
        ));
    }
    let hooks = v.get("hooks").map(ToString::to_string).unwrap_or_default();
    if hooks.contains("--hook guard") {
        return Err("~/.claude/settings.json still runs the old guard hooks. Run: dots".into());
    }
    Ok(())
}

// --- plugin ----------------------------------------------------------------------

/// The ps plugin fetched its binaries into its data folder (its first session does).
fn plugin(home: &Path) -> Result<(), String> {
    let bin = super::agent_kit::data_dir(home).join("bin");
    let exe = if os::is_windows() {
        "agent-config.exe"
    } else {
        "agent-config"
    };
    if bin.join("rust-bin").is_file() && bin.join(exe).is_file() {
        Ok(())
    } else {
        Err(format!(
            "the ps plugin has no binaries in {} yet. Start Claude Code once, then run: dots",
            crate::tilde(&bin)
        ))
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

/// Each `[[bin]]` of the crate is built and not older than its sources. This is the one place a
/// stale binary shows.
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
        let file = if os::is_windows() {
            format!("{n}.exe")
        } else {
            n.clone()
        };
        match fs::metadata(crate_dir.join("target").join("release").join(file))
            .and_then(|m| m.modified())
        {
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
        Err(format!(
            "{}. Run: dots (or scripts/agent-config --bins)",
            parts.join("; ")
        ))
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
/// short one: the rule of the git hooks) replaced by `***`, so a printed path never shows
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
    // git runs in the repo: its own identity and origin URL count, wherever doctor runs from
    let words = gc::private_words_at(home, Some(repo));
    if words.is_empty() {
        return Ok(());
    }
    let hosts = gc::public_hosts(home);
    let mut hits: Vec<String> = Vec::new();
    for name in String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|n| !n.is_empty())
    {
        let lower = name.to_lowercase();
        let shown_name = mask(name, &words);
        if words.iter().any(|w| gc::word_hit(&lower, w)) {
            hits.push(format!("{shown_name} (file name)"));
        }
        let path = repo.join(name);
        if fs::metadata(&path).map_or(true, |m| !m.is_file() || m.len() > 4 << 20) {
            continue;
        }
        let Ok(text) = fs::read(&path).map(|b| String::from_utf8_lossy(&b).to_lowercase()) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let line = gc::strip_hosts(line, &hosts);
            if words.iter().any(|w| gc::word_hit(&line, w)) {
                hits.push(format!("{shown_name}:{}", i + 1));
            }
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    let shown: Vec<String> = hits.iter().take(5).cloned().collect();
    let more = if hits.len() > 5 {
        format!(" and {} more", hits.len() - 5)
    } else {
        String::new()
    };
    Err(format!(
        "{} place(s) hold a private word: {}{}. Remove it before you commit",
        hits.len(),
        join(&shown),
        more
    ))
}
