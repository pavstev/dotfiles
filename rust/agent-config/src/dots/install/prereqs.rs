//! 05-linux-prereqs: the packages Homebrew on Linux and the other steps need (build tools,
//! curl, git, zsh). Linux only. Also the setup-lib.sh helpers `linux_prereqs` and
//! `linux_prereqs_missing`. The lists are in scripts/linux-prereqs.txt.

use super::super::os;
use super::{quiet, stdout_of};
use crate::out::{info, ok, skip, warn};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};

/// The package manager of this Linux: the first of apt-get, dnf, pacman, apk on PATH, or "".
pub fn pick_manager(have: impl Fn(&str) -> bool) -> &'static str {
    ["apt-get", "dnf", "pacman", "apk"].into_iter().find(|m| have(m)).unwrap_or("")
}

/// The packages for `manager` from the text of linux-prereqs.txt (`<manager>: <packages>`).
/// An unknown or empty manager reads the `other` line (commands, not packages).
pub fn parse_prereqs(text: &str, manager: &str) -> Vec<String> {
    let want = if ["apt-get", "dnf", "pacman", "apk"].contains(&manager) { manager } else { "other" };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') { continue; }
        if let Some((m, pkgs)) = line.split_once(':') && m.trim() == want {
            return pkgs.split_whitespace().map(String::from).collect();
        }
    }
    Vec::new()
}

/// The manager name in messages: apt-get is "apt".
fn short_name(manager: &str) -> &str { manager.strip_suffix("-get").unwrap_or(manager) }

/// The packages to check and install, for the manager this Linux has.
fn wanted(repo: &Path) -> (&'static str, Vec<String>) {
    let manager = pick_manager(os::have);
    let text = std::fs::read_to_string(repo.join("scripts").join("linux-prereqs.txt")).unwrap_or_default();
    (manager, parse_prereqs(&text, manager))
}

/// Is this package installed? One query per manager; `other` asks PATH for the command.
fn installed(manager: &str, pkg: &str) -> bool {
    match manager {
        "apt-get" => stdout_of("dpkg-query", &["-W", "-f=${Status}", pkg]).contains("install ok installed"),
        "dnf" => quiet("rpm", &["-q", pkg]),
        "pacman" => quiet("pacman", &["-Q", pkg]),
        "apk" => quiet("apk", &["info", "-e", pkg]),
        _ => os::have(pkg),
    }
}

/// setup-lib.sh `linux_prereqs_missing`: the packages that are not installed.
pub fn linux_prereqs_missing(repo: &Path) -> Vec<String> {
    let (manager, pkgs) = wanted(repo);
    pkgs.into_iter().filter(|p| !installed(manager, p)).collect()
}

/// The install commands for the manager, run in order until one fails.
pub fn install_commands(manager: &str, missing: &[String]) -> Vec<Vec<String>> {
    let plain = |v: &[&str]| -> Vec<String> { v.iter().map(|x| x.to_string()).collect() };
    let s = |v: &[&str]| -> Vec<String> { let mut a = plain(v); a.extend(missing.iter().cloned()); a };
    match manager {
        "apt-get" => vec![
            plain(&["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "update", "-qq"]),
            s(&["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", "-qq", "--no-install-recommends"]),
        ],
        "dnf" => vec![s(&["dnf", "install", "-y", "-q"])],
        "pacman" => vec![s(&["pacman", "-S", "--needed", "--noconfirm"])],
        "apk" => vec![s(&["apk", "add", "-q", "--no-cache"])],
        _ => Vec::new(),
    }
}

/// The last `n` lines of a text.
fn tail(text: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

/// setup-lib.sh `linux_prereqs`: install what `linux_prereqs_missing` lists. Quiet when
/// nothing is missing. True = all there.
pub fn linux_prereqs(repo: &Path) -> bool {
    if !os::is_linux() { return true; }
    let (manager, _) = wanted(repo);
    let name = short_name(manager);
    let missing = linux_prereqs_missing(repo);
    if missing.is_empty() { return true; }
    let list = missing.join(" ");
    if manager.is_empty() {
        warn(&format!("no apt, dnf, pacman or apk here. Install these commands yourself, then run dots again: {}", list));
        return false;
    }
    let mut use_sudo = false;
    if stdout_of("id", &["-u"]).trim() != "0" {
        if !os::have("sudo") {
            warn(&format!("sudo is missing. As root, install with {}: {}", name, list));
            return false;
        }
        use_sudo = true;
        if os::at_terminal() && !quiet("sudo", &["-n", "true"]) { info(&format!("{} needs your password once.", name)); }
    }
    info(&format!("installing with {}: {}", name, list));
    let Ok(mut log) = tempfile::tempfile() else { warn(&format!("{} could not install: {}", name, list)); return false };
    let mut good = true;
    for argv in install_commands(manager, &missing) {
        let (prog, args): (&str, Vec<&str>) = if use_sudo {
            ("sudo", argv.iter().map(String::as_str).collect())
        } else {
            (argv[0].as_str(), argv[1..].iter().map(String::as_str).collect())
        };
        let (Ok(o), Ok(e)) = (log.try_clone(), log.try_clone()) else { good = false; break };
        let done = Command::new(prog).args(&args).stdout(Stdio::from(o)).stderr(Stdio::from(e)).status();
        if !done.map(|s| s.success()).unwrap_or(false) { good = false; break; }
    }
    if good {
        ok(&format!("{} packages installed", name));
        return true;
    }
    warn(&format!("{} could not install: {}", name, list));
    let mut text = String::new();
    if log.seek(SeekFrom::Start(0)).is_ok() { let _ = log.read_to_string(&mut text); }
    for l in tail(&text, 5) { crate::out::plain_err(l); }
    false
}

pub fn step(repo: &Path) {
    if !os::is_linux() { return; }
    crate::out::step("Linux prerequisites");
    let (manager, pkgs) = wanted(repo);
    if linux_prereqs_missing(repo).is_empty() {
        skip(&format!("{} packages ({})", short_name(manager), pkgs.join(" ")));
        return;
    }
    if !linux_prereqs(repo) {
        warn("prerequisites missing, so later steps may fail. Fix the error above, then run dots again.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# note\napt-get: build-essential curl zsh\ndnf: gcc  make\npacman: base-devel\napk: bash zsh\nother: cc make curl\n";

    #[test]
    fn reads_each_manager_line() {
        assert_eq!(parse_prereqs(FILE, "apt-get"), ["build-essential", "curl", "zsh"]);
        assert_eq!(parse_prereqs(FILE, "dnf"), ["gcc", "make"]);
        assert_eq!(parse_prereqs(FILE, "apk"), ["bash", "zsh"]);
        assert_eq!(parse_prereqs(FILE, ""), ["cc", "make", "curl"]);
        assert_eq!(parse_prereqs(FILE, "zypper"), ["cc", "make", "curl"]);
        assert!(parse_prereqs("", "dnf").is_empty());
    }

    #[test]
    fn the_real_file_has_all_five_lines() {
        let text = include_str!("../../../../../scripts/linux-prereqs.txt");
        for m in ["apt-get", "dnf", "pacman", "apk", ""] {
            assert!(parse_prereqs(text, m).contains(&"zsh".to_string()), "{}", m);
        }
    }

    #[test]
    fn manager_is_the_first_one_on_path() {
        assert_eq!(pick_manager(|c| c == "dnf" || c == "apk"), "dnf");
        assert_eq!(pick_manager(|c| c == "apk" || c == "pacman"), "pacman");
        assert_eq!(pick_manager(|c| c == "apt-get" || c == "dnf"), "apt-get");
        assert_eq!(pick_manager(|_| false), "");
    }

    #[test]
    fn install_commands_match_the_bash() {
        let m = vec!["git".to_string(), "zsh".to_string()];
        let apt = install_commands("apt-get", &m);
        assert_eq!(apt[0].join(" "), "env DEBIAN_FRONTEND=noninteractive apt-get update -qq");
        assert_eq!(apt[1].join(" "), "env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends git zsh");
        assert_eq!(install_commands("dnf", &m)[0].join(" "), "dnf install -y -q git zsh");
        assert_eq!(install_commands("pacman", &m)[0].join(" "), "pacman -S --needed --noconfirm git zsh");
        assert_eq!(install_commands("apk", &m)[0].join(" "), "apk add -q --no-cache git zsh");
        assert!(install_commands("", &m).is_empty());
    }

    #[test]
    fn names_and_tails() {
        assert_eq!(short_name("apt-get"), "apt");
        assert_eq!(short_name("dnf"), "dnf");
        assert_eq!(short_name(""), "");
        assert_eq!(tail("a\nb\nc", 2), ["b", "c"]);
        assert_eq!(tail("a", 5), ["a"]);
    }
}
