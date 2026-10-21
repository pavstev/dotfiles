//! `tools [--check] [--list] [--only a,b] [--host NAME] [--file tools.list] [-q] [-v]`: installs the
//! command-line tools of `tools.list` for this OS. The `tools` bin and step 12-tools of `dots` both
//! call `run`. See `tools_core.rs` for the resolver.
//!
//! mise goes in first when a missing tool has a `mise:` source (the official installer on macOS and
//! Linux, WinGet on Windows). Short output: one count line when nothing changed
//! ("• Tools: 36 up to date"). A tool that was installed or that failed gets a line. `-v` (or
//! DOTFILES_VERBOSE=1) prints every tool. `--check` installs nothing: it lists what is missing and
//! exits 1. `-q` prints nothing when nothing changed. `--list` prints the plan for this OS (name,
//! then its sources) and exits 0. Install runs exit 0 even when a tool failed (a warning says so),
//! so `dots` goes on.

use crate::out;
use crate::tools_core::{self as core, Env, Host, Outcome, RealEnv, Source};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// The tools whose failure this process already reported. `dots` runs `run` twice (step 12, then a
/// quiet pass after step 40), and the second pass must not warn about the same tool again.
static WARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// True the first time `name` fails in this process.
fn first_failure(name: &str) -> bool {
    let mut w = WARNED.lock().unwrap_or_else(|e| e.into_inner());
    if w.iter().any(|n| n == name) {
        return false;
    }
    w.push(name.to_string());
    true
}

fn color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}
fn c(code: &str, s: &str) -> String {
    if color() { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
}
fn green(s: &str) -> String {
    c("38;5;108", s)
}
fn dim(s: &str) -> String {
    c("38;5;245", s)
}

fn list_path(arg: Option<PathBuf>) -> Option<PathBuf> {
    if arg.is_some() {
        return arg;
    }
    if let Some(r) = std::env::var_os("DOTFILES_REPO") {
        return Some(PathBuf::from(r).join("tools.list"));
    }
    // .../rust/agent-config/target/release/tools -> the repo root is five levels up.
    let exe = std::env::current_exe().ok()?;
    let root = exe.ancestors().nth(5)?;
    Some(root.join("tools.list"))
}

/// Install mise when it is missing. True when mise is there after. mise is the main source of the
/// tools, so it comes first and needs nothing else (no Homebrew, no cargo).
fn ensure_mise() -> bool {
    let have = || RealEnv::new().has("mise");
    if have() {
        return true;
    }
    let quiet = |mut c: Command| c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok();
    if cfg!(windows) {
        if !RealEnv::new().has("winget") {
            out::warn(
                "mise is missing and winget was not found. Install mise: https://mise.jdx.dev/installing-mise.html",
            );
            return false;
        }
        out::info("installing mise (WinGet)");
        let mut cmd = Command::new("winget");
        cmd.args([
            "install",
            "--id",
            "jdx.mise",
            "-e",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--silent",
        ]);
        quiet(cmd);
    } else {
        if !RealEnv::new().has("curl") {
            out::warn("mise is missing and curl was not found, so the tools did not install.");
            return false;
        }
        out::info("installing mise");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "curl -fsSL https://mise.run | sh"]);
        quiet(cmd);
    }
    if have() {
        out::ok("mise installed");
        return true;
    }
    out::warn("mise did not install. Try: curl https://mise.run | sh   (Windows: winget install jdx.mise)");
    false
}

/// Is a wanted tool missing that has a mise source? Then mise goes in first. A run with nothing
/// to do installs nothing, not even mise.
fn needs_mise(tools: &[core::Tool], host: Host, only: &[String]) -> bool {
    let env = RealEnv::new();
    let miss = core::missing(tools, host, &env, only);
    tools.iter().any(|t| miss.contains(&t.name) && t.sources.iter().any(|s| matches!(s, Source::Mise(_))))
}

/// The arguments of `tools`.
#[derive(clap::Parser, Debug)]
#[command(name = "tools", about = "Install the command-line tools of tools.list for this OS.")]
struct Args {
    /// Install nothing: list what is missing and exit 1.
    #[arg(long)]
    check: bool,
    /// Print the plan for this OS (name, then its sources).
    #[arg(long)]
    list: bool,
    /// Only these tools.
    #[arg(long, value_name = "a,b", value_delimiter = ',', num_args = 1)]
    only: Vec<String>,
    /// Plan for another machine: macos, linux, linux-musl or windows.
    #[arg(long, value_name = "NAME", value_parser = parse_host)]
    host: Option<Host>,
    /// Read this list instead of the repo's tools.list.
    #[arg(long, value_name = "tools.list")]
    file: Option<PathBuf>,
    /// Print nothing when nothing changed.
    #[arg(short, long)]
    quiet: bool,
    /// Print every tool.
    #[arg(short, long)]
    verbose: bool,
}

fn parse_host(s: &str) -> Result<Host, String> {
    Host::parse(s).ok_or_else(|| format!("unknown host {s} (macos, linux, linux-musl, windows)"))
}

/// The whole command. `args` are the arguments after the program name. Returns the exit code.
pub fn run(args: Vec<String>) -> u8 {
    use clap::Parser;
    let args = match Args::try_parse_from(std::iter::once("tools".to_string()).chain(args)) {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };
    let Args { check, list, mut only, host, file, quiet, verbose } = args;
    only.retain(|s| !s.is_empty());
    let verbose = verbose || out::verbose();
    let host = host.unwrap_or_else(Host::detect);
    let Some(path) = list_path(file) else {
        eprintln!("tools: cannot find tools.list (set DOTFILES_REPO)");
        return 2;
    };
    let tools = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| core::parse(&t)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tools: {}: {e}", path.display());
            return 2;
        }
    };

    if list {
        for t in tools.iter().filter(|t| t.applies(host)) {
            let srcs: Vec<String> = t.sources.iter().map(Source::to_string).collect();
            println!("{}\t{}", t.name, srcs.join(" "));
        }
        return 0;
    }

    if check {
        let env = RealEnv::new();
        let miss = core::missing(&tools, host, &env, &only);
        if miss.is_empty() {
            println!("{} every tool is installed", green("✓"));
            return 0;
        }
        println!("missing ({}): {}", miss.len(), miss.join(" "));
        return 1;
    }

    if needs_mise(&tools, host, &only) {
        ensure_mise();
    }
    let mut env = RealEnv::new();
    let outcomes = core::resolve(&tools, host, &mut env, &only);
    let (mut present, mut installed, mut failed) = (0usize, 0usize, 0usize);
    for o in &outcomes {
        match o {
            Outcome::Present(n) => {
                present += 1;
                if verbose {
                    println!("{} {} {}", dim("•"), n, dim("(already there)"));
                }
            }
            Outcome::Installed(n, src) => {
                installed += 1;
                println!("{} {} installed {}", green("✓"), n, dim(&format!("({src})")));
            }
            Outcome::Failed(n, tried) => {
                if !first_failure(n) {
                    continue;
                }
                failed += 1;
                let why = if tried.is_empty() { "no source fits this machine".to_string() } else { tried.join("; ") };
                out::warn(&format!("{n} did not install: {why}"));
            }
        }
    }
    if outcomes.is_empty() {
        println!("{} Tools: none for this machine", dim("•"));
    } else if installed == 0 && failed == 0 {
        if !quiet {
            println!("{} Tools: {} up to date", dim("•"), present);
        }
    } else if installed > 0 {
        println!(
            "{} Tools: {} installed, {} up to date{}",
            green("✓"),
            installed,
            present,
            if failed > 0 { format!(", {failed} failed") } else { String::new() }
        );
    }
    if failed > 0 {
        out::warn(&format!(
            "{} {} missing. Fix the error above, then run dots again.",
            failed,
            if failed == 1 { "tool is" } else { "tools are" }
        ));
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{first_failure, run};

    #[test]
    fn a_flag_without_its_value_is_an_error() {
        assert_eq!(run(vec!["--only".into()]), 2, "an empty --only must not mean every tool");
        assert_eq!(run(vec!["--host".into()]), 2);
        assert_eq!(run(vec!["--host".into(), "beos".into()]), 2);
        assert_eq!(run(vec!["--file".into()]), 2);
    }

    #[test]
    fn a_failed_tool_warns_once_per_process() {
        assert!(first_failure("warn-once-test-tool"));
        assert!(!first_failure("warn-once-test-tool"));
        assert!(first_failure("warn-once-other-tool"));
    }
}
