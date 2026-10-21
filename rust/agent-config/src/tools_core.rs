//! The tool resolver behind `scripts/tools`: reads `tools.list`, and for this OS installs each
//! missing tool from the first source that works. No crates beyond std: the list is a plain
//! `name | commands | OSes | sources` file (see its header). `scripts/tools` is a shim that
//! builds and runs the `tools` bin.
//!
//! The logic is split from the real world by the `Env` trait: `resolve` asks the `Env` whether a
//! command is there and tells it to install from a source. The tests use a fake `Env`.

use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// An operating system `tools.list` can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// macOS.
    Macos,
    /// Linux, WSL included.
    Linux,
    /// Windows.
    Windows,
}

/// The machine a run is for. `musl` is Alpine: glibc binaries do not run there and `apk` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Host {
    /// The operating system.
    pub os: Os,
    /// True on a musl Linux (Alpine).
    pub musl: bool,
}

impl Host {
    /// The machine this process runs on.
    pub fn detect() -> Host {
        let os = if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        };
        let musl = os == Os::Linux
            && (Path::new("/etc/alpine-release").exists()
                || Path::new("/lib/ld-musl-x86_64.so.1").exists()
                || Path::new("/lib/ld-musl-aarch64.so.1").exists());
        Host { os, musl }
    }

    /// `macos`, `linux`, `linux-musl` or `windows`: the names `--host` takes.
    pub fn parse(s: &str) -> Option<Host> {
        match s {
            "macos" => Some(Host { os: Os::Macos, musl: false }),
            "linux" => Some(Host { os: Os::Linux, musl: false }),
            "linux-musl" | "alpine" => Some(Host { os: Os::Linux, musl: true }),
            "windows" => Some(Host { os: Os::Windows, musl: false }),
            _ => None,
        }
    }
}

/// Where a tool can be installed from. Each one holds its argument (a package or a script).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A mise package.
    Mise(String),
    /// An Alpine package.
    Apk(String),
    /// A WinGet package.
    Winget(String),
    /// A Scoop package.
    Scoop(String),
    /// A crate for `cargo install`.
    Cargo(String),
    /// A Homebrew formula.
    Brew(String),
    /// A shell command.
    Sh(String),
    /// Mise itself, which the resolver installs before anything else.
    Boot(String),
}

impl Source {
    /// The source name as `tools.list` spells it (`mise`, `apk`, ...).
    pub fn kind(&self) -> &'static str {
        match self {
            Source::Mise(_) => "mise",
            Source::Apk(_) => "apk",
            Source::Winget(_) => "winget",
            Source::Scoop(_) => "scoop",
            Source::Cargo(_) => "cargo",
            Source::Brew(_) => "brew",
            Source::Sh(_) => "sh",
            Source::Boot(_) => "boot",
        }
    }
    /// The package or command the source holds.
    pub fn arg(&self) -> &str {
        match self {
            Source::Mise(a)
            | Source::Apk(a)
            | Source::Winget(a)
            | Source::Scoop(a)
            | Source::Cargo(a)
            | Source::Brew(a)
            | Source::Sh(a)
            | Source::Boot(a) => a,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind(), self.arg())
    }
}

/// One line of `tools.list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    /// The tool's name in the list.
    pub name: String,
    /// Commands that count as "present" (any one).
    pub cmds: Vec<String>,
    /// Extra command names made as links to the first command (`jaq>jq`).
    pub links: Vec<String>,
    /// The OSes the tool is for.
    pub oses: Vec<Os>,
    /// The OS field had `!musl`: not for Alpine.
    pub no_musl: bool,
    /// Where to install it from, in the order to try.
    pub sources: Vec<Source>,
}

impl Tool {
    /// True if the tool is for this host.
    pub fn applies(&self, host: Host) -> bool {
        self.oses.contains(&host.os) && !(host.musl && self.no_musl)
    }
}

fn parse_os_field(s: &str) -> Result<(Vec<Os>, bool), String> {
    let mut out = Vec::new();
    let mut no_musl = false;
    for t in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match t {
            "!musl" => no_musl = true,
            "all" => out.extend([Os::Macos, Os::Linux, Os::Windows]),
            "unix" => out.extend([Os::Macos, Os::Linux]),
            "macos" => out.push(Os::Macos),
            "linux" => out.push(Os::Linux),
            "windows" => out.push(Os::Windows),
            other => return Err(format!("unknown OS '{other}'")),
        }
    }
    if out.is_empty() {
        return Err("no OS".into());
    }
    Ok((out, no_musl))
}

fn parse_source(t: &str) -> Result<Source, String> {
    let (k, a) = t.split_once(':').ok_or_else(|| format!("source '{t}' needs the form kind:arg"))?;
    if a.is_empty() {
        return Err(format!("source '{t}' has no argument"));
    }
    let a = a.to_string();
    Ok(match k {
        "mise" => Source::Mise(a),
        "apk" => Source::Apk(a),
        "winget" => Source::Winget(a),
        "scoop" => Source::Scoop(a),
        "cargo" => Source::Cargo(a),
        "brew" => Source::Brew(a),
        "sh" => Source::Sh(a),
        "boot" => Source::Boot(a),
        other => return Err(format!("unknown source kind '{other}'")),
    })
}

/// Parse the text of `tools.list`. Blank lines and `#` lines are skipped. An error names the line.
pub fn parse(text: &str) -> Result<Vec<Tool>, String> {
    let mut tools: Vec<Tool> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |m: String| format!("line {}: {}", i + 1, m);
        let f: Vec<&str> = line.split('|').map(str::trim).collect();
        if f.len() != 4 {
            return Err(at(format!("want 4 fields split by '|', found {}", f.len())));
        }
        if f[0].is_empty() {
            return Err(at("empty name".into()));
        }
        if tools.iter().any(|t| t.name == f[0]) {
            return Err(at(format!("'{}' is listed twice", f[0])));
        }
        let mut cmds = Vec::new();
        let mut links = Vec::new();
        for c in f[1].split(',').map(str::trim).filter(|c| !c.is_empty()) {
            match c.split_once('>') {
                Some((cmd, link)) if !cmd.is_empty() && !link.is_empty() => {
                    cmds.push(cmd.to_string());
                    links.push(link.to_string());
                }
                Some(_) => return Err(at(format!("bad link in '{c}'"))),
                None => cmds.push(c.to_string()),
            }
        }
        if cmds.is_empty() {
            return Err(at("no commands".into()));
        }
        let (oses, no_musl) = parse_os_field(f[2]).map_err(at)?;
        let mut sources = Vec::new();
        for t in f[3].split_whitespace() {
            sources.push(parse_source(t).map_err(at)?);
        }
        if sources.is_empty() {
            return Err(at("no sources".into()));
        }
        tools.push(Tool { name: f[0].to_string(), cmds, links, oses, no_musl, sources });
    }
    Ok(tools)
}

/// What a run found for one tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Already there: the tool name.
    Present(String),
    /// Installed now: the tool name, then the source that worked.
    Installed(String, String),
    /// name, then one "kind:arg: reason" per source tried (empty: no source fits this machine).
    Failed(String, Vec<String>),
}

/// The real world, as far as the resolver needs it.
pub trait Env {
    /// Is this command on PATH?
    fn has(&self, cmd: &str) -> bool;
    /// Does this command start? False for a binary that cannot run here (glibc on Alpine).
    fn runs(&self, cmd: &str) -> bool;
    /// Can this source be used on this machine now (its installer is there, root for apk)?
    fn usable(&self, host: Host, src: &Source) -> Result<(), String>;
    /// Install from this source.
    fn install(&mut self, host: Host, src: &Source) -> Result<(), String>;
    /// Undo a source that installed a command that does not start.
    fn undo(&mut self, src: &Source);
    /// Make `link` point at `cmd` (a copy on Windows). Ok when the link is there.
    fn link(&mut self, cmd: &str, link: &str) -> Result<(), String>;
}

/// Resolve every tool for `host`. `only` limits it to those names or commands. Tools for other
/// OSes are left out of the result.
pub fn resolve(tools: &[Tool], host: Host, env: &mut dyn Env, only: &[String]) -> Vec<Outcome> {
    let mut out = Vec::new();
    for t in tools {
        if !t.applies(host) {
            continue;
        }
        if !only.is_empty() && !only.iter().any(|o| o == &t.name || t.cmds.iter().any(|c| c == o)) {
            continue;
        }
        // On Alpine a glibc binary from an earlier run (or another tool) can sit on PATH and not start.
        let present = t.cmds.iter().any(|c| env.has(c) && (!host.musl || env.runs(c)));
        if present {
            link_all(t, env);
            out.push(Outcome::Present(t.name.clone()));
            continue;
        }
        let mut tried: Vec<String> = Vec::new();
        let mut done = None;
        for src in &t.sources {
            if let Err(why) = env.usable(host, src) {
                if !skipped_quietly(host, src) {
                    tried.push(format!("{src}: {why}"));
                }
                continue;
            }
            match env.install(host, src) {
                Err(why) => tried.push(format!("{src}: {why}")),
                Ok(()) => {
                    if t.cmds.iter().any(|c| env.has(c) && env.runs(c)) {
                        done = Some(src.to_string());
                        break;
                    }
                    env.undo(src);
                    tried.push(format!("{src}: installed, but the command does not start here"));
                }
            }
        }
        match done {
            Some(src) => {
                link_all(t, env);
                out.push(Outcome::Installed(t.name.clone(), src));
            }
            None => out.push(Outcome::Failed(t.name.clone(), tried)),
        }
    }
    out
}

/// A source for another OS or flavor is not "tried", it just does not fit: leave it out of the report.
fn skipped_quietly(host: Host, src: &Source) -> bool {
    match src {
        Source::Apk(_) => !host.musl,
        Source::Winget(_) | Source::Scoop(_) => host.os != Os::Windows,
        _ => false,
    }
}

fn link_all(t: &Tool, env: &mut dyn Env) {
    for l in &t.links {
        // A real command of that name (a jq already installed) is never replaced.
        if !env.has(l) {
            let _ = env.link(&t.cmds[0], l);
        }
    }
}

/// The text of the tools a `--check` run reports as missing: one name per element.
pub fn missing(tools: &[Tool], host: Host, env: &dyn Env, only: &[String]) -> Vec<String> {
    tools
        .iter()
        .filter(|t| t.applies(host))
        .filter(|t| only.is_empty() || only.iter().any(|o| o == &t.name || t.cmds.iter().any(|c| c == o)))
        .filter(|t| !t.cmds.iter().any(|c| env.has(c)))
        .map(|t| t.name.clone())
        .collect()
}

// ---- the real Env ---------------------------------------------------------------------------

/// The home folder (USERPROFILE on Windows).
pub fn home() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where mise puts its shims.
pub fn mise_shims() -> PathBuf {
    if let Some(d) = env::var_os("MISE_DATA_DIR") {
        return PathBuf::from(d).join("shims");
    }
    if cfg!(windows)
        && let Some(d) = env::var_os("LOCALAPPDATA")
    {
        return PathBuf::from(d).join("mise").join("shims");
    }
    home().join(".local").join("share").join("mise").join("shims")
}

/// The directories a tool may land in that a bare script PATH misses.
pub fn extra_dirs() -> Vec<PathBuf> {
    let h = home();
    let mut v = vec![h.join(".local").join("bin"), mise_shims(), h.join(".cargo").join("bin")];
    if !cfg!(windows) {
        for p in ["/opt/homebrew/bin", "/usr/local/bin", "/home/linuxbrew/.linuxbrew/bin"] {
            v.push(PathBuf::from(p));
        }
    } else if let Some(l) = env::var_os("LOCALAPPDATA") {
        // WinGet puts the programs it installs (mise, ...) here.
        v.push(PathBuf::from(l).join("Microsoft").join("WinGet").join("Links"));
    }
    v
}

/// PATH with the extra directories on the end (a command already on PATH keeps its place).
pub fn full_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = env::var_os("PATH").map(|p| env::split_paths(&p).collect()).unwrap_or_default();
    for d in extra_dirs() {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    env::join_paths(dirs).unwrap_or_default()
}

/// The first runnable file for `cmd` in `path`, or None.
pub fn find_in_path(cmd: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        let pe = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        std::iter::once(String::new()).chain(pe.split(';').map(|s| s.to_ascii_lowercase())).collect()
    } else {
        vec![String::new()]
    };
    for d in env::split_paths(path) {
        for e in &exts {
            let p = d.join(format!("{cmd}{e}"));
            if crate::pycompat::is_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// The `Env` that checks PATH and runs the real installers.
pub struct RealEnv {
    /// The PATH used to find and run commands.
    pub path: std::ffi::OsString,
}

impl RealEnv {
    /// An env over the full PATH (the extra tool folders included).
    pub fn new() -> RealEnv {
        RealEnv { path: full_path() }
    }

    fn command(&self, prog: &str) -> Command {
        let mut c = Command::new(find_in_path(prog, &self.path).unwrap_or_else(|| PathBuf::from(prog)));
        c.env("PATH", &self.path).stdin(Stdio::null());
        c
    }

    fn run(&self, mut c: Command) -> Result<(), String> {
        let out = c.stdout(Stdio::piped()).stderr(Stdio::piped()).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
        let last = text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("failed");
        Err(last.chars().take(160).collect())
    }

    fn is_root(&self) -> bool {
        Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0").unwrap_or(false)
    }
}

impl Default for RealEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl Env for RealEnv {
    fn has(&self, cmd: &str) -> bool {
        find_in_path(cmd, &self.path).is_some()
    }

    fn runs(&self, cmd: &str) -> bool {
        let Some(p) = find_in_path(cmd, &self.path) else { return false };
        // `cmd --version`: a spawn error (a glibc binary on Alpine has no loader) or exit 126/127 means
        // it cannot run here. Any other exit status means it started. A slow start counts as started.
        let mut child = match Command::new(&p)
            .arg("--version")
            .env("PATH", &self.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        let t0 = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) => return !matches!(st.code(), Some(126) | Some(127)),
                Ok(None) if t0.elapsed() > Duration::from_secs(10) => {
                    let _ = child.kill();
                    return true;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => return false,
            }
        }
    }

    fn usable(&self, host: Host, src: &Source) -> Result<(), String> {
        let need = |c: &str| if self.has(c) { Ok(()) } else { Err(format!("{c} is not installed")) };
        match src {
            Source::Mise(_) => need("mise"),
            Source::Apk(_) if !host.musl => Err("not Alpine".into()),
            Source::Apk(_) => {
                need("apk")?;
                if self.is_root() || self.has("sudo") { Ok(()) } else { Err("needs root".into()) }
            }
            Source::Winget(_) | Source::Scoop(_) if host.os != Os::Windows => Err("not Windows".into()),
            Source::Winget(_) => need("winget"),
            Source::Scoop(_) => need("scoop"),
            Source::Cargo(_) => need("cargo"),
            Source::Brew(_) if host.os == Os::Windows => Err("not on Windows".into()),
            Source::Brew(_) => need("brew"),
            Source::Sh(r) if r == "rustup" => {
                if host.os == Os::Windows {
                    need("winget")
                } else {
                    need("curl")
                }
            }
            Source::Sh(r) => Err(format!("unknown recipe '{r}'")),
            Source::Boot(_) => Err("scripts/tools installs it first; run: dots".into()),
        }
    }

    fn install(&mut self, host: Host, src: &Source) -> Result<(), String> {
        match src {
            Source::Mise(id) => {
                let mut c = self.command("mise");
                c.args(["--yes", "use", "-g", &format!("{id}@latest")]);
                self.run(c)
            }
            Source::Apk(pkg) => {
                let mut c = if self.is_root() {
                    self.command("apk")
                } else {
                    let mut s = self.command("sudo");
                    s.arg("-n").arg("apk");
                    s
                };
                c.args(["add", "--no-cache", pkg]);
                self.run(c)
            }
            Source::Winget(id) => {
                let mut c = self.command("winget");
                c.args([
                    "install",
                    "--id",
                    id,
                    "-e",
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                    "--silent",
                ]);
                self.run(c)
            }
            Source::Scoop(pkg) => {
                let mut c = self.command("scoop");
                c.args(["install", pkg]);
                self.run(c)
            }
            Source::Cargo(krate) => {
                let mut c = self.command("cargo");
                c.args(["install", "--locked", krate]);
                self.run(c)
            }
            Source::Brew(f) => {
                let mut c = self.command("brew");
                c.args(["install", "--quiet", f]);
                self.run(c)
            }
            Source::Sh(r) if r == "rustup" => {
                if host.os == Os::Windows {
                    return self.install(host, &Source::Winget("Rustlang.Rustup".into()));
                }
                let mut c = self.command("sh");
                c.args([
                    "-c",
                    "curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain stable",
                ]);
                self.run(c)
            }
            Source::Sh(r) => Err(format!("unknown recipe '{r}'")),
            Source::Boot(_) => Err("not installable here".into()),
        }
    }

    fn undo(&mut self, src: &Source) {
        if let Source::Mise(id) = src {
            let mut c = self.command("mise");
            c.args(["--yes", "unuse", "-g", id]);
            let _ = self.run(c);
        }
    }

    fn link(&mut self, cmd: &str, link: &str) -> Result<(), String> {
        let target = find_real(cmd, &self.path).ok_or_else(|| format!("{cmd} not found"))?;
        let dir = home().join(".local").join("bin");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = if cfg!(windows) { format!("{link}.exe") } else { link.to_string() };
        let dst = dir.join(name);
        if dst.symlink_metadata().is_ok() {
            // An earlier link that still points at a working target stays; a dangling one is redone.
            if dst.exists() {
                return Ok(());
            }
            let _ = std::fs::remove_file(&dst);
        }
        make_link(&target, &dst).map_err(|e| e.to_string())
    }
}

/// The real file behind a command: mise shims point back at mise, so ask mise where the tool is.
fn find_real(cmd: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let found = find_in_path(cmd, path)?;
    if found.starts_with(mise_shims())
        && let Ok(o) = Command::new(find_in_path("mise", path).unwrap_or_else(|| PathBuf::from("mise")))
            .env("PATH", path)
            .args(["which", cmd])
            .output()
    {
        let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if o.status.success() && !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    Some(found)
}

#[cfg(unix)]
fn make_link(target: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, dst)
}
#[cfg(not(unix))]
fn make_link(target: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(target, dst).map(|_| ())
}

// ---- tests ----------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    const MAC: Host = Host { os: Os::Macos, musl: false };
    const LINUX: Host = Host { os: Os::Linux, musl: false };
    const ALPINE: Host = Host { os: Os::Linux, musl: true };
    const WIN: Host = Host { os: Os::Windows, musl: false };

    /// A fake machine: some commands present; sources that work and what they provide.
    #[derive(Default)]
    struct Fake {
        cmds: HashSet<String>,
        broken: HashSet<String>,
        tools: HashSet<String>,
        /// source string -> commands it installs
        provides: HashMap<String, Vec<String>>,
        failing: HashSet<String>,
        log: Vec<String>,
        links: Vec<(String, String)>,
    }
    impl Fake {
        fn with(cmds: &[&str], tools: &[&str]) -> Fake {
            Fake {
                cmds: cmds.iter().map(|s| s.to_string()).collect(),
                tools: tools.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }
        }
        fn provide(mut self, src: &str, cmds: &[&str]) -> Fake {
            self.provides.insert(src.into(), cmds.iter().map(|s| s.to_string()).collect());
            self
        }
    }
    impl Env for Fake {
        fn has(&self, cmd: &str) -> bool {
            self.cmds.contains(cmd)
        }
        fn runs(&self, cmd: &str) -> bool {
            !self.broken.contains(cmd)
        }
        fn usable(&self, host: Host, src: &Source) -> Result<(), String> {
            match src {
                Source::Apk(_) if !host.musl => Err("not Alpine".into()),
                Source::Winget(_) | Source::Scoop(_) if host.os != Os::Windows => Err("not Windows".into()),
                _ if self.tools.contains(src.kind()) => Ok(()),
                _ => Err(format!("{} is not installed", src.kind())),
            }
        }
        fn install(&mut self, _h: Host, src: &Source) -> Result<(), String> {
            let key = src.to_string();
            self.log.push(key.clone());
            if self.failing.contains(&key) {
                return Err("boom".into());
            }
            for c in self.provides.get(&key).cloned().unwrap_or_default() {
                self.cmds.insert(c);
            }
            Ok(())
        }
        fn undo(&mut self, src: &Source) {
            self.log.push(format!("undo {src}"));
            for c in self.provides.get(&src.to_string()).cloned().unwrap_or_default() {
                self.cmds.remove(&c);
            }
        }
        fn link(&mut self, cmd: &str, link: &str) -> Result<(), String> {
            self.links.push((cmd.into(), link.into()));
            Ok(())
        }
    }

    fn list(s: &str) -> Vec<Tool> {
        parse(s).unwrap()
    }

    #[test]
    fn parses_a_line_with_comments_and_blanks() {
        let t = list("# c\n\nrg | rg | all | mise:aqua:BurntSushi/ripgrep cargo:ripgrep\n");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "rg");
        assert_eq!(t[0].oses, vec![Os::Macos, Os::Linux, Os::Windows]);
        assert_eq!(t[0].sources, vec![Source::Mise("aqua:BurntSushi/ripgrep".into()), Source::Cargo("ripgrep".into())]);
    }

    #[test]
    fn parses_os_words_commands_and_links() {
        let t = list("a | a1, a2 | unix | mise:x\nb | jaq>jq | macos,windows | mise:y\n");
        assert_eq!(t[0].cmds, vec!["a1", "a2"]);
        assert_eq!(t[0].oses, vec![Os::Macos, Os::Linux]);
        assert_eq!(t[1].cmds, vec!["jaq"]);
        assert_eq!(t[1].links, vec!["jq"]);
        assert_eq!(t[1].oses, vec![Os::Macos, Os::Windows]);
    }

    #[test]
    fn not_musl_leaves_a_tool_out_on_alpine() {
        let t = list("w | w | all,!musl | mise:x\n");
        assert!(t[0].applies(LINUX));
        assert!(!t[0].applies(ALPINE));
        assert!(t[0].applies(WIN));
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert!(parse("a | a | all\n").unwrap_err().starts_with("line 1:"));
        assert!(parse("# x\na | a | all | nope:x\n").unwrap_err().starts_with("line 2:"));
        assert!(parse("a | a | planet | mise:x\n").unwrap_err().contains("unknown OS"));
        assert!(parse("a | a | all | mise\n").unwrap_err().contains("kind:arg"));
        assert!(parse("a | a | all | mise:x\na | a | all | mise:x\n").unwrap_err().contains("twice"));
        assert!(parse("a | | all | mise:x\n").unwrap_err().contains("no commands"));
    }

    #[test]
    fn the_real_list_parses_and_has_no_duplicates() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools.list")).unwrap();
        let t = parse(&text).unwrap();
        assert!(t.len() > 30);
        let mut cmds = HashSet::new();
        for tool in &t {
            for c in &tool.cmds {
                assert!(cmds.insert(c.clone()), "command {c} is listed twice");
            }
        }
        // mise is the one tool every OS needs first.
        assert!(t.iter().any(|x| x.name == "mise" && x.sources == vec![Source::Boot("mise".into())]));
    }

    #[test]
    fn present_tools_are_left_alone() {
        let t = list("rg | rg | all | mise:x\n");
        let mut f = Fake::with(&["rg"], &["mise"]);
        assert_eq!(resolve(&t, MAC, &mut f, &[]), vec![Outcome::Present("rg".into())]);
        assert!(f.log.is_empty());
    }

    #[test]
    fn on_alpine_a_present_command_that_does_not_start_counts_as_missing() {
        let t = list("fastfetch | fastfetch | unix | apk:fastfetch mise:aqua:fastfetch-cli/fastfetch\n");
        let mut f = Fake::with(&["fastfetch"], &["apk", "mise"]).provide("apk:fastfetch", &["fastfetch"]);
        f.broken.insert("fastfetch".into());
        // On Alpine a binary that does not start counts as missing, so apk installs it again.
        // The fake keeps it broken, so only the attempt is checked.
        let out = resolve(&t, ALPINE, &mut f, &[]);
        assert!(f.log.contains(&"apk:fastfetch".to_string()), "{out:?}");
        let mut g = Fake::with(&["fastfetch"], &["apk"]);
        g.broken.insert("fastfetch".into());
        assert!(matches!(resolve(&t, LINUX, &mut g, &[])[0], Outcome::Present(_)));
    }

    #[test]
    fn installs_from_the_first_source_that_works() {
        let t = list("rg | rg | all | mise:x cargo:ripgrep\n");
        let mut f = Fake::with(&[], &["mise", "cargo"]).provide("mise:x", &["rg"]).provide("cargo:ripgrep", &["rg"]);
        let out = resolve(&t, LINUX, &mut f, &[]);
        assert_eq!(out, vec![Outcome::Installed("rg".into(), "mise:x".into())]);
        assert_eq!(f.log, vec!["mise:x"]);
    }

    #[test]
    fn falls_through_when_a_source_fails_or_is_unusable() {
        let t = list("rg | rg | all | brew:r mise:x cargo:ripgrep\n");
        let mut f = Fake::with(&[], &["mise", "cargo"]).provide("cargo:ripgrep", &["rg"]);
        f.failing.insert("mise:x".into());
        let out = resolve(&t, LINUX, &mut f, &[]);
        assert_eq!(out, vec![Outcome::Installed("rg".into(), "cargo:ripgrep".into())]);
        assert_eq!(f.log, vec!["mise:x", "cargo:ripgrep"]);
    }

    #[test]
    fn a_command_that_does_not_start_is_undone_and_the_next_source_is_tried() {
        // The glibc build from mise cannot run on Alpine: apk comes first there, but when apk is
        // not usable (no root) mise installs it, the check fails, and cargo takes over.
        let t = list("eza | eza | unix | apk:eza mise:aqua:eza cargo:eza\n");
        let mut f =
            Fake::with(&[], &["mise", "cargo"]).provide("mise:aqua:eza", &["eza"]).provide("cargo:eza", &["eza2"]);
        f.broken.insert("eza".into());
        let out = resolve(&t, ALPINE, &mut f, &[]);
        assert!(
            matches!(&out[0], Outcome::Failed(n, tried) if n == "eza" && tried.iter().any(|s| s.contains("does not start")))
        );
        assert!(f.log.contains(&"undo mise:aqua:eza".to_string()));
        assert!(!f.has("eza"));
    }

    #[test]
    fn apk_is_first_on_alpine_and_skipped_elsewhere() {
        let t = list("eza | eza | unix | apk:eza mise:aqua:eza\n");
        let mut alp = Fake::with(&[], &["apk", "mise"]).provide("apk:eza", &["eza"]).provide("mise:aqua:eza", &["eza"]);
        assert_eq!(resolve(&t, ALPINE, &mut alp, &[]), vec![Outcome::Installed("eza".into(), "apk:eza".into())]);
        let mut lin = Fake::with(&[], &["apk", "mise"]).provide("apk:eza", &["eza"]).provide("mise:aqua:eza", &["eza"]);
        assert_eq!(resolve(&t, LINUX, &mut lin, &[]), vec![Outcome::Installed("eza".into(), "mise:aqua:eza".into())]);
        assert_eq!(lin.log, vec!["mise:aqua:eza"]);
    }

    #[test]
    fn tools_for_other_oses_are_left_out() {
        let t = list("tmux | tmux | unix | mise:t\nrg | rg | all | mise:x\n");
        let mut f = Fake::with(&[], &["mise"]).provide("mise:x", &["rg"]);
        let out = resolve(&t, WIN, &mut f, &[]);
        assert_eq!(out, vec![Outcome::Installed("rg".into(), "mise:x".into())]);
    }

    #[test]
    fn failure_lists_what_was_tried_and_leaves_out_other_os_sources() {
        let t = list("ff | ffmpeg | all | apk:ffmpeg mise:conda:ffmpeg winget:Gyan.FFmpeg\n");
        let mut f = Fake::with(&[], &[]);
        let out = resolve(&t, MAC, &mut f, &[]);
        match &out[0] {
            Outcome::Failed(n, tried) => {
                assert_eq!(n, "ff");
                assert_eq!(tried, &vec!["mise:conda:ffmpeg: mise is not installed".to_string()]);
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn only_filters_by_name_or_command() {
        let t = list("ripgrep | rg | all | mise:x\nfd | fd | all | mise:y\n");
        let mut f = Fake::with(&[], &["mise"]).provide("mise:x", &["rg"]).provide("mise:y", &["fd"]);
        let out = resolve(&t, MAC, &mut f, &["rg".to_string()]);
        assert_eq!(out, vec![Outcome::Installed("ripgrep".into(), "mise:x".into())]);
        assert_eq!(f.log, vec!["mise:x"]);
    }

    #[test]
    fn a_link_is_made_once_and_never_over_a_real_command() {
        let t = list("jaq | jaq>jq | all | mise:j\n");
        let mut f = Fake::with(&["jaq"], &["mise"]);
        resolve(&t, MAC, &mut f, &[]);
        assert_eq!(f.links, vec![("jaq".to_string(), "jq".to_string())]);
        let mut g = Fake::with(&["jaq", "jq"], &["mise"]);
        resolve(&t, MAC, &mut g, &[]);
        assert!(g.links.is_empty());
        let mut h = Fake::with(&[], &["mise"]).provide("mise:j", &["jaq"]);
        resolve(&t, MAC, &mut h, &[]);
        assert_eq!(h.links.len(), 1);
    }

    #[test]
    fn missing_lists_names_for_this_os_only() {
        let t = list("ripgrep | rg | all | mise:x\ntmux | tmux | unix | mise:t\nfd | fd | all | mise:y\n");
        let f = Fake::with(&["fd"], &[]);
        assert_eq!(missing(&t, WIN, &f, &[]), vec!["ripgrep"]);
        assert_eq!(missing(&t, MAC, &f, &[]), vec!["ripgrep", "tmux"]);
    }

    #[test]
    fn host_names_parse() {
        assert_eq!(Host::parse("alpine"), Some(ALPINE));
        assert_eq!(Host::parse("linux"), Some(LINUX));
        assert_eq!(Host::parse("nope"), None);
    }

    #[test]
    fn path_search_finds_a_file_and_skips_a_missing_one() {
        let d = std::env::temp_dir().join(format!("tools-core-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let name = if cfg!(windows) { "faketool.exe" } else { "faketool" };
        let p = d.join(name);
        std::fs::write(&p, "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = env::join_paths([d.clone()]).unwrap();
        assert!(find_in_path("faketool", &path).is_some());
        assert!(find_in_path("no-such-tool", &path).is_none());
        std::fs::remove_dir_all(&d).ok();
    }
}
