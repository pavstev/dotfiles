//! `dots update`: find the updates of every package manager on this Mac and install them in one
//! run (macOS only).
//!
//! The check phase runs every manager's read-only check in parallel and prints one line each. The
//! update phase then runs one manager at a time, in table order, and goes on after a failure. Every
//! command is logged to `~/Library/Logs/dots/update-<time>.log`. The parsers and the planner are
//! pure functions, tested with real output samples. The macOS and App Store managers only report (`mas upgrade` needs root).
//!
//! Test hook: with `DOTS_UPDATE_PATH` set, managers are found and run only from that PATH (and
//! children get it as their PATH), so a test never reaches a real package manager.
//! `DOTS_UPDATE_TIMEOUT=<secs>` sets both timeouts (a test hook too).

use super::{os, steps};
use crate::out::{self, info, ok, plain, plain_err, step, step_end, warn};
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// A package manager `dots update` knows.
#[derive(Debug)]
pub struct Mgr {
    pub id: &'static str,
    pub name: &'static str,
}

/// The managers, in update order.
pub const MANAGERS: [Mgr; 12] = [
    Mgr { id: "brew", name: "Homebrew" },
    Mgr { id: "mise", name: "mise" },
    Mgr { id: "rustup", name: "Rust (rustup)" },
    Mgr { id: "cargo", name: "cargo crates" },
    Mgr { id: "uv", name: "uv tools" },
    Mgr { id: "npm", name: "npm globals" },
    Mgr { id: "pnpm", name: "pnpm globals" },
    Mgr { id: "gh", name: "gh extensions" },
    Mgr { id: "omz", name: "Oh My Zsh" },
    Mgr { id: "mas", name: "App Store apps" },
    Mgr { id: "claude", name: "Claude Code" },
    Mgr { id: "macos", name: "macOS" },
];

/// The options of `dots update`.
pub struct Opts {
    pub check: bool,
    pub only: Vec<String>,
    pub skip: Vec<String>,
}

/// What a check found out outdated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// Names for the check line (at most 8 are shown).
    pub items: Vec<String>,
    /// Oh My Zsh: the git repos that are behind (the main one and custom plugins and themes).
    pub repos: Vec<PathBuf>,
}

/// The result of one manager's check.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    /// Not installed, or nothing to manage.
    Missing,
    UpToDate,
    Outdated(Found),
    /// No check-only command exists: the update runs anyway.
    WillRun,
    /// Updates exist, but `dots update` never installs them.
    CheckOnly(Vec<String>),
    Failed(String),
}

// ---------- pure parts ----------

/// The managers to run, in table order. Err: an unknown id, with the valid ones.
pub fn plan(only: &[String], skip: &[String]) -> Result<Vec<&'static Mgr>, String> {
    for id in only.iter().chain(skip) {
        if !MANAGERS.iter().any(|m| m.id == id) {
            let valid: Vec<&str> = MANAGERS.iter().map(|m| m.id).collect();
            return Err(format!("unknown manager \"{id}\". Valid ids: {}", valid.join(", ")));
        }
    }
    Ok(MANAGERS
        .iter()
        .filter(|m| (only.is_empty() || only.iter().any(|o| o == m.id)) && !skip.iter().any(|s| s == m.id))
        .collect())
}

fn json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text.trim()).map_err(|e| format!("could not read the JSON answer: {e}"))
}

/// `brew outdated --json=v2`: formulae first, then casks. Pinned formulae are left out:
/// `brew upgrade` skips them, so they would look outdated on every run.
pub fn parse_brew(text: &str) -> Result<Vec<String>, String> {
    let v = json(text)?;
    let mut names = Vec::new();
    for key in ["formulae", "casks"] {
        if let Some(list) = v.get(key).and_then(Value::as_array) {
            names.extend(
                list.iter()
                    .filter(|x| x.get("pinned").and_then(Value::as_bool) != Some(true))
                    .filter_map(|x| x.get("name").and_then(Value::as_str))
                    .map(str::to_string),
            );
        }
    }
    Ok(names)
}

/// `mise outdated --json`: an object keyed by tool, each with `current` and `latest`.
pub fn parse_mise(text: &str) -> Result<Vec<String>, String> {
    let v = json(text)?;
    let Some(map) = v.as_object() else { return Err("mise did not answer with an object".into()) };
    Ok(map
        .iter()
        .map(|(name, t)| match (t.get("current").and_then(Value::as_str), t.get("latest").and_then(Value::as_str)) {
            (Some(c), Some(l)) => format!("{name} {c} -> {l}"),
            _ => name.clone(),
        })
        .collect())
}

/// `npm outdated -g --json` and `pnpm outdated -g --format json`: an object keyed by package (or a
/// list of objects with a name). No output means nothing is outdated. Offline, npm prints
/// `{"error":{"code":..,"summary":..}}` instead: that is an error, not a package named `error`.
pub fn parse_node_outdated(text: &str) -> Result<Vec<String>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    match json(text)? {
        Value::Object(m) => {
            if let Some(e) = m.get("error").and_then(Value::as_object)
                && !e.contains_key("latest")
                && !e.contains_key("current")
                && let Some(why) =
                    e.get("summary").or_else(|| e.get("code")).and_then(Value::as_str).filter(|s| !s.is_empty())
            {
                return Err(why.lines().next().unwrap_or(why).trim().to_string());
            }
            Ok(m.keys().cloned().collect())
        }
        Value::Array(a) => Ok(a
            .iter()
            .filter_map(|x| x.get("packageName").or_else(|| x.get("name")).and_then(Value::as_str))
            .map(str::to_string)
            .collect()),
        _ => Err("unexpected answer".into()),
    }
}

/// `rustup check`: lines with `update available`, named by the text before ` - `.
pub fn parse_rustup(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.to_lowercase().contains("update available"))
        .map(|l| l.split(" - ").next().unwrap_or(l).trim().to_string())
        .collect()
}

/// `cargo install-update --list`: rows whose last column ("Needs update") is `Yes`.
pub fn parse_cargo_update(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            (cols.len() >= 4 && cols.last() == Some(&"Yes")).then(|| cols[0].to_string())
        })
        .collect()
}

/// `uv tool list --outdated`: `name vX [latest: Y]` lines (the `- command` lines are skipped).
pub fn parse_uv(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.starts_with('-') && l.contains("[latest:"))
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// `gh extension upgrade --all --dry-run`: `[name]: would have upgraded from A to B`.
pub fn parse_gh(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains("would have upgraded"))
        .filter_map(|l| {
            let rest = l.trim().strip_prefix('[')?;
            rest.split_once(']').map(|(n, _)| n.to_string())
        })
        .collect()
}

/// `mas outdated`: `497799835  Xcode (15.0)`. Lines that do not start with an app id are skipped.
pub fn parse_mas(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let (id, rest) = l.trim().split_once(char::is_whitespace)?;
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let rest = rest.trim();
            let name = match rest.rfind(" (") {
                Some(i) if rest.ends_with(')') => &rest[..i],
                _ => rest,
            };
            Some(name.trim().to_string())
        })
        .collect()
}

/// `softwareupdate --list`: the label of each `* Label: ...` line.
pub fn parse_softwareupdate(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("* Label:")).map(|l| l.trim().to_string()).collect()
}

/// `git rev-list --count HEAD..@{u}`: the number, 0 when it is not one.
pub fn parse_behind(text: &str) -> usize {
    text.trim().parse().unwrap_or(0)
}

/// The names for a check line: at most 8, then `+N more`.
pub fn summarize(items: &[String]) -> String {
    const MAX: usize = 8;
    let s = items.iter().take(MAX).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > MAX { format!("{s}, +{} more", items.len() - MAX) } else { s }
}

/// The line that says what went wrong: the first one that mentions an error, else the first one.
pub fn first_useful_line(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let pick = lines
        .iter()
        .find(|l| {
            let low = l.to_lowercase();
            low.contains("error") || low.contains("err!") || low.contains("fatal")
        })
        .or_else(|| lines.first());
    pick.map_or_else(String::new, |l| l.chars().take(160).collect())
}

/// Seconds for a message: "2s", "5m", "60m".
pub fn span(secs: u64) -> String {
    if secs >= 60 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else if secs < 60 {
        format!("{secs}s")
    } else {
        out::duration(secs)
    }
}

/// True when `path` (already resolved) is inside `prefix`.
pub fn under_prefix(path: &Path, prefix: Option<&Path>) -> bool {
    prefix.is_some_and(|p| path.starts_with(p))
}

/// The items of `before` that the check after the update still lists, matched by name (the first word).
pub fn still_outdated(before: &[String], after: &[String]) -> Vec<String> {
    let name = |s: &str| s.split_whitespace().next().unwrap_or("").to_string();
    let after: Vec<String> = after.iter().map(|a| name(a)).collect();
    before.iter().filter(|b| after.contains(&name(b))).cloned().collect()
}

/// PATH plus pnpm's global bin folders ($PNPM_HOME, else ~/Library/pnpm, and its bin/), when they
/// are missing. `pnpm update -g` stops when its bin folder is not on PATH, and a shell that skipped
/// .zshrc (a launcher, `zsh -l`) does not have it.
pub fn with_pnpm_home(path: &OsString, pnpm_home: Option<PathBuf>, home: &Path) -> OsString {
    let ph = pnpm_home.unwrap_or_else(|| home.join("Library/pnpm"));
    let mut dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    for d in [ph.clone(), ph.join("bin")] {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| path.clone())
}

/// Where gh keeps its extensions: $GH_DATA_DIR, else $XDG_DATA_HOME/gh, else ~/.local/share/gh.
pub fn gh_extensions_dir(gh_data: Option<PathBuf>, xdg_data: Option<PathBuf>, home: &Path) -> PathBuf {
    gh_data
        .or_else(|| xdg_data.map(|x| x.join("gh")))
        .unwrap_or_else(|| home.join(".local/share/gh"))
        .join("extensions")
}

/// The log files to delete so that `keep` remain (newest by name, which is the time stamp).
pub fn stale_logs(mut names: Vec<String>, keep: usize) -> Vec<String> {
    names.retain(|n| n.starts_with("update-") && Path::new(n).extension().is_some_and(|e| e == "log"));
    names.sort();
    let drop = names.len().saturating_sub(keep);
    names.truncate(drop);
    names
}

/// The variables that keep git (and ssh under it) from asking questions on the terminal: a dead
/// remote or a host key prompt would hang the run. `ssh_command`: the user's own GIT_SSH_COMMAND.
pub fn git_env(ssh_command: Option<&str>) -> Vec<(String, String)> {
    let ssh = match ssh_command.filter(|s| !s.trim().is_empty()) {
        Some(s) => format!("{s} -o BatchMode=yes"),
        None => "ssh -o BatchMode=yes".to_string(),
    };
    vec![("GIT_TERMINAL_PROMPT".into(), "0".into()), ("GIT_SSH_COMMAND".into(), ssh)]
}

/// One command to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdSpec {
    pub prog: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl CmdSpec {
    fn new(prog: &str, args: &[&str]) -> Self {
        Self { prog: prog.into(), args: args.iter().map(|a| (*a).to_string()).collect(), env: Vec::new() }
    }
    fn env(mut self, k: &str, v: &str) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
    /// The command as a line, for the log and the screen.
    pub fn line(&self) -> String {
        std::iter::once(self.prog.as_str()).chain(self.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
    }
}

/// The update commands of a manager, in order. `macos` and `mas` have none: they only report.
/// `mise_self_update`: mise is not a Homebrew package, so it updates itself first.
/// `repos`: the Oh My Zsh repos that are behind.
pub fn update_cmds(id: &str, mise_self_update: bool, zsh_dir: &Path, repos: &[PathBuf]) -> Vec<CmdSpec> {
    match id {
        "brew" => vec![CmdSpec::new("brew", &["upgrade"])],
        "mise" => {
            let mut v = Vec::new();
            if mise_self_update {
                v.push(CmdSpec::new("mise", &["self-update", "--yes"]).env("MISE_YES", "1"));
            }
            v.push(CmdSpec::new("mise", &["upgrade"]).env("MISE_YES", "1"));
            v
        }
        "rustup" => vec![CmdSpec::new("rustup", &["update"])],
        "cargo" => vec![CmdSpec::new("cargo", &["install-update", "--all"])],
        "uv" => vec![CmdSpec::new("uv", &["tool", "upgrade", "--all"])],
        "npm" => vec![CmdSpec::new("npm", &["update", "-g"])],
        "pnpm" => vec![CmdSpec::new("pnpm", &["update", "-g", "--latest"])],
        "gh" => vec![CmdSpec::new("gh", &["extension", "upgrade", "--all"])],
        "omz" => {
            let z = zsh_dir.to_string_lossy().into_owned();
            let mut v = Vec::new();
            if repos.iter().any(|r| r == zsh_dir) {
                let script = format!("{z}/tools/upgrade.sh");
                v.push(CmdSpec::new("zsh", &["-f", &script, "-v", "silent"]).env("ZSH", &z));
            }
            for r in repos.iter().filter(|r| r.as_path() != zsh_dir) {
                let dir = r.to_string_lossy().into_owned();
                v.push(CmdSpec::new("git", &["-C", &dir, "pull", "--ff-only", "--quiet"]));
            }
            v
        }
        "claude" => vec![CmdSpec::new("claude", &["update"])],
        _ => Vec::new(),
    }
}

// ---------- running commands ----------

/// The log file of this run.
struct Log {
    path: PathBuf,
    file: Mutex<fs::File>,
}

impl Log {
    fn put(&self, s: &str) {
        let mut f = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = f.write_all(s.as_bytes());
    }
}

/// `~/Library/Logs/dots/update-<time>.log`, with the older logs pruned to the last 10.
fn open_log() -> Option<Log> {
    let dir = os::home().join("Library/Logs/dots");
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("update-{}.log", os::timestamp()));
    let file = fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
    let names: Vec<String> = fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    for old in stale_logs(names, 10) {
        let _ = fs::remove_file(dir.join(old));
    }
    Some(Log { path, file: Mutex::new(file) })
}

/// What a command did.
#[derive(Default)]
struct Out {
    /// None: killed by a signal, or never started.
    code: Option<i32>,
    timed_out: bool,
    /// Why it never started.
    err: Option<String>,
    stdout: String,
    stderr: String,
    /// Both streams, one entry per line, in arrival order.
    all: Vec<String>,
    secs: u64,
}

impl Out {
    fn exit_ok(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }
    fn text(&self) -> String {
        self.all.join("\n")
    }
}

/// How `Runner::run` reports a command while it runs.
#[derive(Default)]
struct Tap {
    /// Write each line to the log as it arrives (an update). Else the whole block goes in at the end.
    live: bool,
    /// Print each line to the terminal (`dots -v`).
    echo: bool,
    /// The last line seen, for the "still working" line.
    last: Option<Arc<Mutex<String>>>,
}

/// The lines of one pipe, split at newlines and carriage returns (progress bars).
fn spawn_reader(pipe: Option<impl std::io::Read + Send + 'static>, is_err: bool, tx: mpsc::Sender<(bool, String)>) {
    let Some(pipe) = pipe else { return };
    std::thread::spawn(move || {
        let mut r = BufReader::new(pipe);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            for seg in String::from_utf8_lossy(&buf).split(['\n', '\r']) {
                if !seg.trim().is_empty() && tx.send((is_err, seg.trim_end().to_string())).is_err() {
                    return;
                }
            }
        }
    });
}

/// Everything a run needs, read once on the main thread.
struct Runner {
    /// Where managers are found, and the PATH every child gets: DOTS_UPDATE_PATH (the test hook)
    /// as it is, else PATH plus pnpm's bin folders.
    path: OsString,
    check_timeout: u64,
    update_timeout: u64,
    prefix: Option<PathBuf>,
    zsh_dir: PathBuf,
    zsh_custom: PathBuf,
    gh_extensions: PathBuf,
    /// The folder every child starts in.
    home: PathBuf,
    /// The user's GIT_SSH_COMMAND, if any.
    ssh_command: Option<String>,
    log: Option<Log>,
}

impl Runner {
    fn new() -> Self {
        let pinned_path = std::env::var_os("DOTS_UPDATE_PATH");
        let secs = std::env::var("DOTS_UPDATE_TIMEOUT").ok().and_then(|v| v.trim().parse::<u64>().ok());
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let zsh_dir = var("ZSH").unwrap_or_else(|| os::home().join(".oh-my-zsh"));
        let zsh_custom = var("ZSH_CUSTOM").unwrap_or_else(|| zsh_dir.join("custom"));
        let path = pinned_path.unwrap_or_else(|| {
            let env_path = std::env::var_os("PATH").unwrap_or_default();
            with_pnpm_home(&env_path, var("PNPM_HOME"), &os::home())
        });
        Self {
            path,
            check_timeout: secs.unwrap_or(300),
            update_timeout: secs.unwrap_or(3600),
            prefix: var("HOMEBREW_PREFIX").map(|p| fs::canonicalize(&p).unwrap_or(p)),
            zsh_dir,
            zsh_custom,
            gh_extensions: gh_extensions_dir(var("GH_DATA_DIR"), var("XDG_DATA_HOME"), &os::home()),
            home: os::home(),
            ssh_command: std::env::var("GIT_SSH_COMMAND").ok(),
            log: open_log(),
        }
    }

    fn find(&self, prog: &str) -> Option<PathBuf> {
        os::which_in(prog, &self.path)
    }

    fn put(&self, s: &str) {
        if let Some(l) = &self.log {
            l.put(s);
        }
    }

    /// Run one command to the end or to the timeout. A grandchild that keeps a pipe open does not
    /// hold us: after the child exits (or is killed) we wait half a second for the last lines.
    fn run(&self, id: &str, cmd: &CmdSpec, timeout: u64, tap: &Tap) -> Out {
        let started = Instant::now();
        let header = format!("== {id}: {}\n", cmd.line());
        if tap.live {
            self.put(&header);
        }
        let finish = |o: Out, tail: &str| {
            let footer = format!("== exit {tail} in {}\n", out::duration(o.secs));
            if tap.live {
                self.put(&footer);
            } else {
                let mut block = header.clone();
                for l in &o.all {
                    block.push_str(l);
                    block.push('\n');
                }
                block.push_str(&footer);
                self.put(&block);
            }
            o
        };
        let Some(prog) = self.find(&cmd.prog) else {
            let o = Out { err: Some(format!("{} not found", cmd.prog)), ..Out::default() };
            return finish(o, "none");
        };
        let mut c = Command::new(prog);
        // Start in the home folder, not in the project the user ran dots from: `mise` there would
        // read and rewrite that project's mise.toml and lock file.
        c.args(&cmd.args).current_dir(&self.home).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c.env("PATH", &self.path);
        for (k, v) in git_env(self.ssh_command.as_deref()) {
            c.env(k, v);
        }
        for (k, v) in &cmd.env {
            c.env(k, v);
        }
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                let o = Out { err: Some(format!("could not start {}: {e}", cmd.prog)), ..Out::default() };
                return finish(o, "none");
            }
        };
        let (tx, rx) = mpsc::channel::<(bool, String)>();
        spawn_reader(child.stdout.take(), false, tx.clone());
        spawn_reader(child.stderr.take(), true, tx);

        let mut o = Out::default();
        let deadline = started + Duration::from_secs(timeout);
        let mut exited_at: Option<Instant> = None;
        let mut open = true;
        loop {
            if open {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok((is_err, line)) => {
                        if tap.live {
                            self.put(&format!("{line}\n"));
                        }
                        if tap.echo {
                            plain(&format!("{}{}{}", out::d(), line, out::z()));
                        }
                        if let Some(last) = &tap.last {
                            last.lock().unwrap_or_else(PoisonError::into_inner).clone_from(&line);
                        }
                        let side = if is_err { &mut o.stderr } else { &mut o.stdout };
                        side.push_str(&line);
                        side.push('\n');
                        o.all.push(line);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => open = false,
                }
            } else if exited_at.is_none() {
                std::thread::sleep(Duration::from_millis(20));
            }
            if exited_at.is_none() {
                if let Ok(Some(status)) = child.try_wait() {
                    o.code = status.code();
                    exited_at = Some(Instant::now());
                } else if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    o.timed_out = true;
                    exited_at = Some(Instant::now());
                }
            }
            if let Some(t) = exited_at
                && (!open || t.elapsed() > Duration::from_millis(500))
            {
                break;
            }
        }
        o.secs = started.elapsed().as_secs();
        let tail =
            if o.timed_out { "timeout".to_string() } else { o.code.map_or("signal".to_string(), |c| c.to_string()) };
        finish(o, &tail)
    }

    fn probe(&self, id: &str, prog: &str, args: &[&str]) -> Option<Out> {
        self.find(prog)?;
        Some(self.run(id, &CmdSpec::new(prog, args), self.check_timeout, &Tap::default()))
    }

    fn mise_self_update(&self) -> bool {
        self.find("mise").is_none_or(|p| {
            let real = fs::canonicalize(&p).unwrap_or(p);
            !under_prefix(&real, self.prefix.as_deref())
        })
    }
}

// ---------- the checks ----------

fn fail_text(o: &Out, timeout: u64) -> String {
    if o.timed_out {
        return format!("timed out after {}", span(timeout));
    }
    if let Some(e) = &o.err {
        return e.clone();
    }
    let line = first_useful_line(&o.text());
    let code = o.code.map_or("signal".to_string(), |c| c.to_string());
    if line.is_empty() { format!("exit {code}") } else { line }
}

/// Run a check command; `parse` turns its output into the outdated names.
fn simple(
    rt: &Runner,
    id: &str,
    prog: &str,
    args: &[&str],
    ok_codes: &[i32],
    parse: fn(&Out) -> Result<Vec<String>, String>,
) -> Check {
    let Some(o) = rt.probe(id, prog, args) else { return Check::Missing };
    if o.timed_out || o.err.is_some() || !o.code.is_some_and(|c| ok_codes.contains(&c)) {
        return Check::Failed(fail_text(&o, rt.check_timeout));
    }
    match parse(&o) {
        Ok(items) if items.is_empty() => Check::UpToDate,
        Ok(items) => Check::Outdated(Found { items, repos: Vec::new() }),
        Err(e) => Check::Failed(e),
    }
}

fn check_omz(rt: &Runner) -> Check {
    if !rt.zsh_dir.join(".git").exists() {
        return Check::Missing;
    }
    let mut repos: Vec<(String, PathBuf)> = vec![("oh-my-zsh".into(), rt.zsh_dir.clone())];
    for sub in ["plugins", "themes"] {
        let Ok(rd) = fs::read_dir(rt.zsh_custom.join(sub)) else { continue };
        let mut found: Vec<(String, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().join(".git").exists())
            .map(|e| (format!("{sub}/{}", e.file_name().to_string_lossy()), e.path()))
            .collect();
        found.sort();
        repos.extend(found);
    }
    let mut behind = Found::default();
    let mut skipped: Vec<String> = Vec::new();
    for (name, dir) in repos {
        let d = dir.to_string_lossy().into_owned();
        let Some(f) = rt.probe("omz", "git", &["-C", &d, "fetch", "--quiet"]) else { return Check::Missing };
        if !f.exit_ok() {
            // One dead remote must not stop the others from updating.
            skipped.push(format!("{name} ({})", fail_text(&f, rt.check_timeout)));
            continue;
        }
        let Some(c) = rt.probe("omz", "git", &["-C", &d, "rev-list", "--count", "HEAD..@{u}"]) else {
            return Check::Missing;
        };
        // No upstream (a local repo): nothing to pull.
        if c.exit_ok() && parse_behind(&c.stdout) > 0 {
            behind.items.push(name);
            behind.repos.push(dir);
        }
    }
    if !skipped.is_empty() {
        warn(&format!("Oh My Zsh: could not fetch, so skipped: {}", skipped.join("; ")));
    }
    if behind.items.is_empty() { Check::UpToDate } else { Check::Outdated(behind) }
}

/// The read-only check of one manager.
fn check(rt: &Runner, id: &str) -> Check {
    match id {
        "brew" => {
            let Some(u) = rt.probe(id, "brew", &["update", "--quiet"]) else { return Check::Missing };
            if !u.exit_ok() {
                return Check::Failed(fail_text(&u, rt.check_timeout));
            }
            simple(rt, id, "brew", &["outdated", "--json=v2"], &[0], |o| parse_brew(&o.stdout))
        }
        "mise" => simple(rt, id, "mise", &["outdated", "--json"], &[0], |o| parse_mise(&o.stdout)),
        "rustup" => simple(rt, id, "rustup", &["check"], &[0, 100], |o| {
            let mut items = parse_rustup(&o.text());
            if items.is_empty() && o.code == Some(100) {
                items.push("toolchain".into());
            }
            Ok(items)
        }),
        "cargo" => {
            if rt.find("cargo-install-update").is_none() {
                return Check::Missing;
            }
            simple(rt, id, "cargo", &["install-update", "--list"], &[0], |o| Ok(parse_cargo_update(&o.text())))
        }
        "uv" => simple(rt, id, "uv", &["tool", "list", "--outdated"], &[0], |o| Ok(parse_uv(&o.text()))),
        "npm" => simple(rt, id, "npm", &["outdated", "-g", "--json"], &[0, 1], |o| parse_node_outdated(&o.stdout)),
        "pnpm" => {
            simple(rt, id, "pnpm", &["outdated", "-g", "--format", "json"], &[0, 1], |o| parse_node_outdated(&o.stdout))
        }
        "gh" => {
            // No extensions: nothing to update. A gh that is signed out fails `extension list`.
            if fs::read_dir(&rt.gh_extensions).map_or(true, |mut rd| rd.next().is_none()) {
                return Check::Missing;
            }
            let Some(l) = rt.probe(id, "gh", &["extension", "list"]) else { return Check::Missing };
            if !l.exit_ok() {
                return Check::Failed(fail_text(&l, rt.check_timeout));
            }
            if l.stdout.trim().is_empty() {
                return Check::Missing;
            }
            simple(rt, id, "gh", &["extension", "upgrade", "--all", "--dry-run"], &[0], |o| Ok(parse_gh(&o.text())))
        }
        "omz" => check_omz(rt),
        "mas" => {
            let Some(o) = rt.probe(id, "mas", &["outdated"]) else { return Check::Missing };
            if !o.exit_ok() {
                return Check::Failed(fail_text(&o, rt.check_timeout));
            }
            let items = parse_mas(&o.stdout);
            if items.is_empty() { Check::UpToDate } else { Check::CheckOnly(items) }
        }
        "claude" => match rt.find("claude") {
            Some(p) => {
                let real = fs::canonicalize(&p).unwrap_or(p);
                if under_prefix(&real, rt.prefix.as_deref()) { Check::Missing } else { Check::WillRun }
            }
            None => Check::Missing,
        },
        "macos" => {
            let Some(o) = rt.probe(id, "softwareupdate", &["--list"]) else { return Check::Missing };
            if !o.exit_ok() {
                return Check::Failed(fail_text(&o, rt.check_timeout));
            }
            let items = parse_softwareupdate(&o.text());
            if items.is_empty() { Check::UpToDate } else { Check::CheckOnly(items) }
        }
        _ => Check::Missing,
    }
}

// ---------- the run ----------

/// The line of a manager that only reports: how many updates, and how to install them.
pub fn check_only_line(m: &Mgr, n: usize) -> String {
    let s = if n == 1 { "" } else { "s" };
    if m.id == "mas" {
        format!("{}: {n} update{s} (run: sudo mas upgrade)", m.name)
    } else {
        let it = if n == 1 { "it" } else { "them" };
        format!("{}: {n} update{s} (install {it} in System Settings > General > Software Update)", m.name)
    }
}

/// Print the line of one finished check.
fn show_check(m: &Mgr, c: &Check) {
    match c {
        Check::Missing => {
            if out::verbose() {
                info(&format!("{}: not found", m.name));
            }
        }
        Check::UpToDate => plain(&format!("{}• {}: up to date{}", out::d(), m.name, out::z())),
        Check::Outdated(f) => info(&format!("{}: {} to update ({})", m.name, f.items.len(), summarize(&f.items))),
        Check::WillRun => info(&format!("{}: will run claude update", m.name)),
        Check::CheckOnly(items) => info(&check_only_line(m, items.len())),
        Check::Failed(why) => warn(&format!("{}: check failed: {why}", m.name)),
    }
}

/// Run all checks in parallel. The results are in the order of `mgrs`.
fn check_all(rt: &Runner, mgrs: &[&'static Mgr]) -> Vec<Check> {
    let mut results: Vec<Option<Check>> = mgrs.iter().map(|_| None).collect();
    let started = Instant::now();
    let mut next_notice = 20;
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel::<(usize, Check)>();
        for (i, m) in mgrs.iter().enumerate() {
            let tx = tx.clone();
            s.spawn(move || {
                let _ = tx.send((i, check(rt, m.id)));
            });
        }
        drop(tx);
        let mut waiting = mgrs.len();
        while waiting > 0 {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok((i, c)) => {
                    show_check(mgrs[i], &c);
                    results[i] = Some(c);
                    waiting -= 1;
                }
                Err(RecvTimeoutError::Timeout) => {
                    let secs = started.elapsed().as_secs();
                    if secs >= next_notice {
                        next_notice += 30;
                        let names: Vec<&str> =
                            mgrs.iter().zip(&results).filter(|(_, r)| r.is_none()).map(|(m, _)| m.name).collect();
                        info(&format!("still checking: {} ({})…", names.join(", "), out::duration(secs)));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    results.into_iter().map(|r| r.unwrap_or_else(|| Check::Failed("the check stopped".into()))).collect()
}

/// The managers by outcome, for the summary.
#[derive(Default)]
struct Tally {
    updated: Vec<&'static str>,
    failed: Vec<&'static str>,
    to_update: Vec<&'static str>,
    up_to_date: Vec<&'static str>,
    check_only: Vec<&'static str>,
    /// Updated with exit 0, but the check after it still finds items.
    left: Vec<&'static str>,
}

/// Run the update of one manager. Err: why it failed, with the last output lines.
fn update_one(rt: &Runner, m: &Mgr, found: &Found, verbose: bool) -> Result<(), (String, Vec<String>)> {
    let last = Arc::new(Mutex::new(String::new()));
    let ticker = {
        let last = Arc::clone(&last);
        let name = m.name;
        steps::ticker(20, 60, move |secs| {
            let l = last.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let l: String = l.chars().take(100).collect();
            format!("still working on {name} ({}): {l}", out::duration(secs))
        })
    };
    let tap = Tap { live: true, echo: verbose, last: Some(Arc::clone(&last)) };
    let mut failure: Option<(String, Vec<String>)> = None;
    for cmd in update_cmds(m.id, rt.mise_self_update(), &rt.zsh_dir, &found.repos) {
        info(&cmd.line());
        let o = rt.run(m.id, &cmd, rt.update_timeout, &tap);
        if o.exit_ok() {
            continue;
        }
        let why = if o.timed_out {
            format!("timed out after {}", span(rt.update_timeout))
        } else if let Some(e) = &o.err {
            e.clone()
        } else {
            format!("exit {}", o.code.map_or("signal".to_string(), |c| c.to_string()))
        };
        if failure.is_none() {
            let tail: Vec<String> = o.all.iter().rev().take(10).rev().cloned().collect();
            failure = Some((why, tail));
        }
    }
    drop(ticker);
    failure.map_or(Ok(()), Err)
}

/// `dots update`. Returns the exit code: 0, 1 when a check or an update failed, 2 for bad arguments.
pub fn run(o: &Opts) -> i32 {
    if !os::is_macos() {
        plain_err("dots update runs on macOS only.");
        return 2;
    }
    let mgrs = match plan(&o.only, &o.skip) {
        Ok(m) => m,
        Err(e) => {
            plain_err(&e);
            return 2;
        }
    };
    // Read verbose, color and the warn file now: the ticker threads print.
    out::init();
    let started = Instant::now();
    let rt = Runner::new();
    let verbose = out::verbose();

    step("Checking");
    info(&format!("looking for updates in {} package managers", mgrs.len()));
    let results = check_all(&rt, &mgrs);

    let mut tally = Tally::default();
    for (m, c) in mgrs.iter().zip(&results) {
        match c {
            Check::Missing => {}
            Check::UpToDate => tally.up_to_date.push(m.name),
            Check::Outdated(_) | Check::WillRun => tally.to_update.push(m.name),
            Check::CheckOnly(_) => tally.check_only.push(m.name),
            Check::Failed(_) => tally.failed.push(m.name),
        }
    }

    if !o.check {
        for (m, c) in mgrs.iter().zip(&results) {
            let found = match c {
                Check::Outdated(f) => f.clone(),
                Check::WillRun => Found::default(),
                _ => continue,
            };
            step(&if found.items.is_empty() {
                m.name.to_string()
            } else {
                format!("{} ({} to update)", m.name, found.items.len())
            });
            match update_one(&rt, m, &found, verbose) {
                Ok(()) if found.items.is_empty() => {
                    ok(&format!("{}: updated", m.name));
                    tally.updated.push(m.name);
                }
                Ok(()) => {
                    // Check again: an update that exits 0 can leave items behind (a version pin).
                    let left = match check(&rt, m.id) {
                        Check::Outdated(after) => still_outdated(&found.items, &after.items),
                        _ => Vec::new(),
                    };
                    let done = found.items.len() - left.len();
                    if left.is_empty() {
                        ok(&format!("{}: updated {done}", m.name));
                    } else {
                        ok(&format!("{}: updated {done} of {}", m.name, found.items.len()));
                        warn(&format!(
                            "{}: still outdated: {} (a version pin may hold it back)",
                            m.name,
                            left.join(", ")
                        ));
                        tally.left.push(m.name);
                    }
                    if done > 0 {
                        tally.updated.push(m.name);
                    }
                }
                Err((why, tail)) => {
                    warn(&format!("{} failed ({why})", m.name));
                    for l in tail {
                        plain_err(&format!("    {l}"));
                    }
                    if let Some(l) = &rt.log {
                        plain_err(&format!("    Full log: {}", l.path.display()));
                    }
                    tally.failed.push(m.name);
                }
            }
            step_end();
        }
        tally.to_update.retain(|n| ![&tally.updated, &tally.failed, &tally.left].iter().any(|g| g.contains(n)));
    }

    step("Summary");
    let list = |v: &[&str]| v.join(", ");
    if !tally.updated.is_empty() {
        ok(&format!("updated: {}", list(&tally.updated)));
    }
    if !tally.to_update.is_empty() {
        info(&format!("to update: {}", list(&tally.to_update)));
    }
    if !tally.failed.is_empty() {
        warn(&format!("failed: {}", list(&tally.failed)));
    }
    if !tally.left.is_empty() {
        info(&format!("still outdated: {}", list(&tally.left)));
    }
    if !tally.up_to_date.is_empty() {
        info(&format!("up to date: {}", list(&tally.up_to_date)));
    }
    if !tally.check_only.is_empty() {
        info(&format!("check only: {}", list(&tally.check_only)));
    }
    if let Some(l) = &rt.log {
        info(&format!("log: {}", l.path.display()));
    }
    info(&format!("run time: {}", out::duration(started.elapsed().as_secs())));
    step_end();
    i32::from(!tally.failed.is_empty())
}

#[cfg(test)]
#[allow(clippy::assert_is_empty)] // `assert!(parse(..).is_empty())` reads best for "nothing found"
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn plan_filters_in_table_order() {
        let all = plan(&[], &[]).unwrap();
        assert_eq!(all.len(), 12);
        assert_eq!(all[0].id, "brew");
        assert_eq!(all[11].id, "macos");
        let only = plan(&v(&["mise", "brew"]), &[]).unwrap();
        assert_eq!(only.iter().map(|m| m.id).collect::<Vec<_>>(), ["brew", "mise"]);
        let skip = plan(&[], &v(&["mas", "claude"])).unwrap();
        assert_eq!(skip.len(), 10);
        assert!(skip.iter().all(|m| m.id != "mas" && m.id != "claude"));
        let both = plan(&v(&["brew", "mise"]), &v(&["mise"])).unwrap();
        assert_eq!(both.len(), 1);
    }

    #[test]
    fn plan_rejects_unknown_ids() {
        let e = plan(&v(&["brew", "nope"]), &[]).unwrap_err();
        assert!(e.contains("\"nope\"") && e.contains("brew, mise, rustup"), "{e}");
        assert!(plan(&[], &v(&["zzz"])).is_err());
    }

    #[test]
    fn brew_json() {
        assert_eq!(parse_brew("{\n  \"formulae\": [],\n  \"casks\": []\n}\n").unwrap(), Vec::<String>::new());
        let sample = r#"{"formulae":[{"name":"git","installed_versions":["2.50.0"],"current_version":"2.51.0","pinned":false,"pinned_version":null},
            {"name":"wget","installed_versions":["1.25.0"],"current_version":"1.25.1","pinned":false,"pinned_version":null}],
            "casks":[{"name":"ghostty","installed_versions":"1.2.0","current_version":"1.2.1"}]}"#;
        assert_eq!(parse_brew(sample).unwrap(), v(&["git", "wget", "ghostty"]));
        assert!(parse_brew("Error: nope").is_err());
    }

    #[test]
    fn mise_json() {
        let sample = r#"{
  "aqua:01mf02/jaq": {
    "name": "aqua:01mf02/jaq",
    "requested": "latest",
    "current": "3.1.1",
    "bump": null,
    "latest": "3.1.2",
    "release_url": "https://github.com/01mf02/jaq/releases/tag/v3.1.2",
    "source": {"type": "mise.toml", "path": "/Users/x/.config/mise/config.toml"}
  },
  "npm:awwwards-mcp": {"name": "npm:awwwards-mcp", "requested": "latest", "current": "1.11.0", "bump": null, "latest": "1.13.0"}
}"#;
        assert_eq!(
            parse_mise(sample).unwrap(),
            v(&["aqua:01mf02/jaq 3.1.1 -> 3.1.2", "npm:awwwards-mcp 1.11.0 -> 1.13.0"])
        );
        assert_eq!(parse_mise("{}").unwrap(), Vec::<String>::new());
        assert!(parse_mise("[]").is_err());
    }

    #[test]
    fn node_json() {
        let npm = r#"{
  "npm": {
    "current": "11.19.1",
    "wanted": "12.2.1",
    "latest": "12.2.1",
    "dependent": "global",
    "location": "/Users/x/.local/share/mise/installs/node/24.22.1/lib/node_modules/npm"
  }
}"#;
        assert_eq!(parse_node_outdated(npm).unwrap(), v(&["npm"]));
        assert_eq!(parse_node_outdated("{}\n").unwrap(), Vec::<String>::new());
        assert_eq!(parse_node_outdated("").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_node_outdated(r#"[{"packageName":"typescript","current":"5.0.0"}]"#).unwrap(),
            v(&["typescript"])
        );
        assert!(parse_node_outdated("npm ERR! boom").is_err());
    }

    #[test]
    fn rustup_check() {
        let sample = "stable-aarch64-apple-darwin - update available: 1.99.0 (b940084d7 2026-09-28) -> 1.99.1 (58cf8f9c3 2026-10-13)\n\
                      1.89-aarch64-apple-darwin - up to date: 1.89.0 (29483883e 2025-08-04)\n\
                      rustup - up to date : 1.29.2\n";
        assert_eq!(parse_rustup(sample), v(&["stable-aarch64-apple-darwin"]));
        assert_eq!(parse_rustup("rustup - Update available : 1.29.2 -> 1.30.0\n"), v(&["rustup"]));
        assert!(parse_rustup("stable - up to date: 1.0.0\n").is_empty());
    }

    #[test]
    fn cargo_update_list() {
        let sample = "Package        Installed  Latest   Needs update\n\
                      cargo-update   v18.0.0    v18.0.0  No\n\
                      ripgrep        v13.0.0    v14.1.0  Yes\n\
                      fd-find        v9.0.0     v10.0.0  Yes\n";
        assert_eq!(parse_cargo_update(sample), v(&["ripgrep", "fd-find"]));
        assert!(parse_cargo_update("No packages installed.\n").is_empty());
    }

    #[test]
    fn uv_outdated() {
        let sample = "mlx-lm v0.32.0 [latest: 0.32.2]\n- mlx_lm\n- mlx_lm.awq\n- mlx_lm.chat\n";
        assert_eq!(parse_uv(sample), v(&["mlx-lm"]));
        assert!(parse_uv("marker-pdf v2.0.0\n- marker\n").is_empty());
    }

    #[test]
    fn gh_dry_run() {
        assert_eq!(parse_gh("[dash]: would have upgraded from v4.26.0 to v4.26.2\n"), v(&["dash"]));
        assert!(parse_gh("[dash]: already up to date\n").is_empty());
        assert!(parse_gh("").is_empty());
    }

    #[test]
    fn mas_outdated() {
        let sample = "497799835  Xcode (15.0)\n1444383602  GoodNotes 5 (5.9.6)\n";
        assert_eq!(parse_mas(sample), v(&["Xcode", "GoodNotes 5"]));
        assert!(parse_mas("").is_empty());
        assert!(parse_mas("Warning: not signed in\n").is_empty());
    }

    #[test]
    fn softwareupdate_list() {
        let sample = "Software Update Tool\n\nFinding available software\nSoftware Update found the following new or updated software:\n\
                      * Label: macOS 27.2-26B123\n\tTitle: macOS 27.2, Version: 27.2, Size: 6067473KiB, Recommended: YES, Action: restart, \n";
        assert_eq!(parse_softwareupdate(sample), v(&["macOS 27.2-26B123"]));
        assert!(
            parse_softwareupdate("Software Update Tool\n\nFinding available software\nNo new software available.\n")
                .is_empty()
        );
    }

    #[test]
    fn behind_count() {
        assert_eq!(parse_behind("3\n"), 3);
        assert_eq!(parse_behind(""), 0);
        assert_eq!(parse_behind("fatal: no upstream configured"), 0);
    }

    #[test]
    fn summaries() {
        assert_eq!(summarize(&v(&["git", "node"])), "git, node");
        let ten: Vec<String> = (1..=10).map(|n| format!("p{n}")).collect();
        assert_eq!(summarize(&ten), "p1, p2, p3, p4, p5, p6, p7, p8, +2 more");
        assert_eq!(summarize(&ten[..8]), "p1, p2, p3, p4, p5, p6, p7, p8");
    }

    #[test]
    fn useful_line() {
        assert_eq!(first_useful_line("\n  npm warn config\nnpm ERR! code E404\nmore\n"), "npm ERR! code E404");
        assert_eq!(first_useful_line("\nsomething odd\nsecond\n"), "something odd");
        assert_eq!(first_useful_line(""), "");
    }

    #[test]
    fn spans() {
        assert_eq!(span(2), "2s");
        assert_eq!(span(300), "5m");
        assert_eq!(span(3600), "60m");
        assert_eq!(span(90), "1m30s");
    }

    #[test]
    fn prefix_and_logs() {
        assert!(under_prefix(Path::new("/opt/homebrew/Cellar/mise/1/bin/mise"), Some(Path::new("/opt/homebrew"))));
        assert!(!under_prefix(Path::new("/Users/x/.local/bin/mise"), Some(Path::new("/opt/homebrew"))));
        assert!(!under_prefix(Path::new("/opt/homebrew/bin/x"), None));
        let before = v(&["jq 1.7.1 -> 1.8.2", "ruff", "git"]);
        assert_eq!(
            still_outdated(&before, &v(&["ruff", "jq 1.8.2 -> 1.8.3", "other"])),
            v(&["jq 1.7.1 -> 1.8.2", "ruff"])
        );
        assert!(still_outdated(&before, &[]).is_empty());
        let home = Path::new("/Users/x");
        let p = OsString::from("/usr/bin:/Users/x/Library/pnpm");
        assert_eq!(
            with_pnpm_home(&p, None, home),
            OsString::from("/usr/bin:/Users/x/Library/pnpm:/Users/x/Library/pnpm/bin")
        );
        assert_eq!(with_pnpm_home(&OsString::from("/bin"), Some("/p".into()), home), OsString::from("/bin:/p:/p/bin"));
        assert_eq!(gh_extensions_dir(None, None, home), Path::new("/Users/x/.local/share/gh/extensions"));
        assert_eq!(gh_extensions_dir(None, Some("/d".into()), home), Path::new("/d/gh/extensions"));
        assert_eq!(gh_extensions_dir(Some("/g".into()), Some("/d".into()), home), Path::new("/g/extensions"));
        let names: Vec<String> =
            (1..=12).map(|n| format!("update-202610{n:02}-120000.log")).chain(["notes.txt".to_string()]).collect();
        let stale = stale_logs(names, 10);
        assert_eq!(stale, v(&["update-20261001-120000.log", "update-20261002-120000.log"]));
        assert!(stale_logs(v(&["update-20261001-120000.log"]), 10).is_empty());
    }

    #[test]
    fn update_commands() {
        let z = Path::new("/h/.oh-my-zsh");
        let line = |id: &str, selfup: bool, repos: &[PathBuf]| -> Vec<String> {
            update_cmds(id, selfup, z, repos).iter().map(CmdSpec::line).collect()
        };
        assert_eq!(line("brew", false, &[]), ["brew upgrade"]);
        assert_eq!(line("mise", true, &[]), ["mise self-update --yes", "mise upgrade"]);
        assert_eq!(line("mise", false, &[]), ["mise upgrade"]);
        assert_eq!(update_cmds("mise", false, z, &[])[0].env, [("MISE_YES".to_string(), "1".to_string())]);
        assert_eq!(line("pnpm", false, &[]), ["pnpm update -g --latest"]);
        assert_eq!(line("cargo", false, &[]), ["cargo install-update --all"]);
        assert!(line("macos", false, &[]).is_empty(), "macOS is never installed");
        assert!(line("mas", false, &[]).is_empty(), "mas upgrade needs root: check only");
        let repos = [z.to_path_buf(), PathBuf::from("/h/.oh-my-zsh/custom/plugins/x")];
        assert_eq!(
            line("omz", false, &repos),
            [
                "zsh -f /h/.oh-my-zsh/tools/upgrade.sh -v silent",
                "git -C /h/.oh-my-zsh/custom/plugins/x pull --ff-only --quiet"
            ]
        );
        assert_eq!(update_cmds("omz", false, z, &repos)[0].env, [("ZSH".to_string(), "/h/.oh-my-zsh".to_string())]);
        assert_eq!(line("omz", false, &repos[1..]).len(), 1, "no main upgrade when only a plugin is behind");
    }

    #[test]
    fn brew_skips_pinned_formulae() {
        let sample = r#"{"formulae":[
            {"name":"node@20","installed_versions":["20.1.0"],"current_version":"20.2.0","pinned":true,"pinned_version":"20.1.0"},
            {"name":"git","installed_versions":["2.50.0"],"current_version":"2.51.0","pinned":false,"pinned_version":null}],
            "casks":[]}"#;
        assert_eq!(parse_brew(sample).unwrap(), v(&["git"]));
        let only_pinned = r#"{"formulae":[{"name":"node@20","pinned":true}],"casks":[]}"#;
        assert!(parse_brew(only_pinned).unwrap().is_empty());
    }

    #[test]
    fn npm_offline_is_an_error_not_a_package() {
        let offline = r#"{"error":{"code":"ECONNREFUSED","summary":"request to https://registry.npmjs.org/npm failed","detail":""}}"#;
        assert_eq!(parse_node_outdated(offline).unwrap_err(), "request to https://registry.npmjs.org/npm failed");
        assert_eq!(parse_node_outdated(r#"{"error":{"code":"ENOTFOUND"}}"#).unwrap_err(), "ENOTFOUND");
        // a real package that happens to be called error
        let pkg = r#"{"error":{"current":"1.0.0","wanted":"1.1.0","latest":"1.1.0"}}"#;
        assert_eq!(parse_node_outdated(pkg).unwrap(), v(&["error"]));
    }

    #[test]
    fn git_children_never_prompt() {
        let e = git_env(None);
        assert!(e.contains(&("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())));
        assert!(e.contains(&("GIT_SSH_COMMAND".to_string(), "ssh -o BatchMode=yes".to_string())));
        let own = git_env(Some("ssh -i /k/id"));
        assert!(own.contains(&("GIT_SSH_COMMAND".to_string(), "ssh -i /k/id -o BatchMode=yes".to_string())));
        assert!(git_env(Some("  ")).contains(&("GIT_SSH_COMMAND".to_string(), "ssh -o BatchMode=yes".to_string())));
    }

    #[test]
    fn check_only_hints() {
        let find = |id: &str| MANAGERS.iter().find(|m| m.id == id).unwrap();
        assert_eq!(check_only_line(find("mas"), 2), "App Store apps: 2 updates (run: sudo mas upgrade)");
        assert_eq!(
            check_only_line(find("macos"), 1),
            "macOS: 1 update (install it in System Settings > General > Software Update)"
        );
    }
}
