//! bag_check: is this Mac safe to put in a bag with Claude still running? Replaces
//! claude/skills/ps_bag-mode/scripts/bag-check.sh (same flags, card and exit codes).
//!
//! ```text
//! bag_check [--eta 45m]    one compact card (about 3 s). ETA = how long the session still needs.
//! bag_check --throttle     also drop dev servers / indexers to background priority (reversible)
//! bag_check --restore      undo --throttle
//! bag_check --watch [N]    repeat the card every N seconds (default 60); the ETA counts down
//! ```
//!
//! ETA formats: 45  45m  2h  1h30m. Always ends with one "VERDICT:" line.

#[macro_use]
#[path = "../quiet_out.rs"]
mod quiet_out;

use regex::Regex;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

const USAGE: &str = "\
bag_check: is this Mac safe to put in a bag with Claude still running?
  bag_check [--eta 45m]    one compact card (about 3 s). ETA = how long the session still needs.
  bag_check --throttle     also drop dev servers / indexers to background priority (reversible)
  bag_check --restore      undo --throttle
  bag_check --watch [N]    repeat the card every N seconds (default 60); the ETA counts down
ETA formats: 45  45m  2h  1h30m. Always ends with one \"VERDICT:\" line.
";

const MAXROWS: usize = 5; // heavy rows shown before "+N more"
const BUFFER: i64 = 25; // % safety margin added to the ETA

/// Run a command and give its stdout (empty when it cannot run).
fn out_of(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// "45" | "45m" | "2h" | "1h30m" -> minutes. Anything else counts as 0.
fn parse_min(s: &str) -> i64 {
    let s = s.to_lowercase();
    let h_re = Regex::new(r"[0-9]+h").unwrap();
    let (mut h, mut rest) = (0, s.clone());
    if let Some(m) = h_re.find(&s) {
        h = m.as_str().trim_end_matches('h').parse::<i64>().unwrap_or(0);
        rest = format!("{}{}", &s[..m.start()], &s[m.end()..]);
    }
    let m = Regex::new(r"[0-9]+").unwrap().find(&rest).and_then(|m| m.as_str().parse::<i64>().ok()).unwrap_or(0);
    h * 60 + m
}

fn fmt_min(m: i64) -> String {
    if m >= 60 { format!("{}h{:02}m", m / 60, m % 60) } else { format!("{m}m") }
}

fn base(cmd: &str) -> &str {
    cmd.trim_end_matches('/').rsplit('/').next().unwrap_or(cmd)
}

/// "kind|when it ends" for a command path.
fn classify(cmd: &str) -> (String, &'static str) {
    let has = |ks: &[&str]| ks.iter().any(|k| cmd.contains(k));
    if has(&["WindowServer"]) {
        ("🖥  screen drawing".into(), "when the lid closes")
    } else if has(&["Claude Helper (Renderer)"]) || cmd.ends_with("Claude.app/Contents/MacOS/Claude") {
        ("🪟 Claude window".into(), "when it is hidden")
    } else if has(&["claude-code", "claude.app/"]) || cmd.ends_with("/claude") {
        ("🤖 Claude agent".into(), "when its task ends")
    } else if has(&["next-server", "vite", "webpack", "workerd", "wrangler", "astro"]) {
        ("🛠  dev server".into(), "never - stop it")
    } else if has(&["rustc", "cargo", "tsc", "esbuild", "swc", "biome", "turbo", "pnpm"]) {
        ("🔨 build/lint".into(), "when the build ends")
    } else if has(&["dockerd", "com.docker", "colima", "qemu", "vz"]) {
        ("🐳 container VM".into(), "never - stop if idle")
    } else if has(&["mds", "mdworker", "mds_stores"]) {
        ("🔍 Spotlight".into(), "in a few minutes")
    } else if has(&["brew", "softwareupdate", "backupd"]) {
        ("🧰 system job".into(), "ends alone")
    } else if has(&["node", "bun", "deno", "python"]) {
        (format!("📜 script ({})", base(cmd)), "unknown")
    } else {
        (format!("❔ {}", base(cmd)), "unknown")
    }
}

/// Pids of things safe to throttle (dev servers and indexers).
fn throttle_pids() -> Vec<String> {
    let re = Regex::new(r"next-server|workerd|vite|webpack|wrangler|mdworker|mds_stores").unwrap();
    out_of("/bin/ps", &["-Ao", "pid,comm"])
        .lines()
        .filter(|l| re.is_match(l))
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

struct Card {
    ncpu: f64,
    heavy: f64,
    eta_min: i64,
    start: Instant,
}

/// Load numbers the way awk reads them: junk counts as 0.
fn num(s: &str) -> f64 {
    s.trim().parse().unwrap_or(0.0)
}

impl Card {
    fn report(&self) {
        let (mut lvl, mut why) = (0, String::new());
        let mut bump = |l: i32, w: &str| {
            if l > lvl {
                lvl = l;
                why = w.to_string();
            }
        };

        // --- power
        let b = out_of("pmset", &["-g", "batt"]);
        let src = Regex::new(r"'[^']*'")
            .unwrap()
            .find(b.lines().next().unwrap_or(""))
            .map(|m| m.as_str().trim_matches('\'').replacen(" Power", "", 1))
            .unwrap_or_default();
        let pct = Regex::new(r"[0-9]*%").unwrap().find(&b).map(|m| m.as_str().to_string()).unwrap_or_default();
        let rem_min: Option<i64> = Regex::new(r"([0-9]*):([0-9][0-9]) remaining").unwrap().captures(&b).and_then(|c| {
            let v = c[1].parse::<i64>().unwrap_or(0) * 60 + c[2].parse::<i64>().unwrap_or(0);
            (v != 0).then_some(v) // "0:00 remaining" = no estimate yet, not zero minutes
        });
        let amph = if out_of("pmset", &["-g", "assertions"]).contains("Amphetamine") {
            "✅ ON".to_string()
        } else {
            bump(2, "Amphetamine is OFF - the Mac will sleep");
            "❌ OFF - the Mac will sleep".to_string()
        };
        let mut pw =
            format!("{} on {}", if pct.is_empty() { "?" } else { &pct }, if src.is_empty() { "?" } else { &src });
        if src == "Battery"
            && let Some(r) = rem_min
        {
            pw = format!("{pw} · {} left", fmt_min(r));
        }

        // --- load
        let l1 = out_of("sysctl", &["-n", "vm.loadavg"]).split_whitespace().nth(1).unwrap_or("").to_string();
        let memfree = out_of("memory_pressure", &[])
            .lines()
            .find(|l| l.contains("free percentage"))
            .and_then(|l| l.split_whitespace().last().map(str::to_string))
            .unwrap_or_default();
        let load = num(&l1);
        if load > self.ncpu {
            bump(2, "load above core count - runs hot in a bag");
        } else if load > self.ncpu / 2.0 {
            bump(1, "busy - use --throttle");
        }

        // --- ETA
        let mut eta_line = "unknown - pass --eta 45m".to_string();
        if self.eta_min > 0 {
            let el = (self.start.elapsed().as_secs() / 60) as i64;
            let eff = (self.eta_min - el).max(0);
            let need = eff + eff * BUFFER / 100;
            eta_line = format!("~{} (+{BUFFER}% buffer = {})", fmt_min(eff), fmt_min(need));
            match (src == "Battery", rem_min) {
                (true, Some(r)) => {
                    if r < need {
                        eta_line.push_str(" · ⚠️ battery short");
                        bump(2, &format!("battery ({}) is less than ETA - plug in", fmt_min(r)));
                    } else {
                        eta_line.push_str(" · ✅ battery ok");
                    }
                }
                _ => eta_line.push_str(" · ✅ power ok"),
            }
        } else if src == "Battery" && pct.trim_end_matches('%').parse::<i64>().is_ok_and(|p| p < 20) {
            bump(1, "battery under 20% - plug in");
        }

        // --- heavy
        let top = out_of("top", &["-l", "2", "-n", "25", "-o", "cpu", "-stats", "pid,cpu"]);
        let mut seen = 0;
        let mut rows: Vec<String> = Vec::new();
        for line in top.lines() {
            if line.starts_with("PID") {
                seen += 1;
                continue;
            }
            if seen != 2 {
                continue;
            }
            let mut f = line.split_whitespace();
            let (Some(pid), Some(cpu)) = (f.next(), f.next()) else { continue };
            if pid.is_empty() || !pid.bytes().all(|c| c.is_ascii_digit()) || num(cpu) < self.heavy {
                continue;
            }
            let cmd = out_of("/bin/ps", &["-p", pid, "-o", "comm="]);
            let cmd = cmd.trim_end_matches('\n');
            if cmd.is_empty() {
                continue;
            }
            let (kind, dur) = classify(cmd);
            rows.push(format!("| {kind} | {:.0}% | {dur} |", num(cpu)));
        }
        let n = rows.len();

        // --- counts
        let comms = out_of("/bin/ps", &["-Ao", "comm"]);
        let count = |re: &str| {
            let re = Regex::new(re).unwrap();
            comms.lines().filter(|l| re.is_match(l)).count()
        };
        let ag = count("claude.app/Contents/MacOS/claude");
        let ds = count("next-server|workerd|vite|webpack");
        let nd = count("(^|/)node$");

        // --- verdict text
        let (lname, lemoji, msg) = match lvl {
            0 => ("LOW", "🟢", "safe to close the lid.".to_string()),
            1 => ("MEDIUM", "🟡", format!("{why}.")),
            _ => ("HIGH", "🔴", format!("{why}.")),
        };

        // --- card (markdown: renders as tables in Claude)
        println!("## 🎒 Bag Mode — {lemoji} {lname}\n");
        println!("| Check | Status |\n|---|---|");
        println!("| 🔋 Power | {pw} |");
        println!("| ☕ Amphetamine | {amph} |");
        println!(
            "| 🔥 Load | {l1} / {} cores · {} memory free |",
            self.ncpu,
            if memfree.is_empty() { "?" } else { &memfree }
        );
        println!("| ⏱ ETA | {eta_line} |");
        println!("| 🤖 Running | {ag} agents · {ds} dev servers · {nd} node |");
        println!("\n### 🏋 Heavy tasks ({n})\n");
        if n > 0 {
            println!("| Task | CPU | Ends |\n|---|---|---|");
            for r in rows.iter().take(MAXROWS) {
                println!("{r}");
            }
            if n > MAXROWS {
                println!("| … +{} more | | |", n - MAXROWS);
            }
        } else {
            println!("None. Nothing is above {}% of a core. ✨", self.heavy);
        }
        println!("\nVERDICT: {lemoji} {lname} - {msg}");
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if !cfg!(target_os = "macos") {
        let os = out_of("uname", &["-s"]);
        let os = if os.trim().is_empty() { std::env::consts::OS.to_string() } else { os.trim().to_string() };
        println!("bag-mode is for a Mac laptop (it reads pmset, sysctl and Amphetamine). Nothing to check on {os}.");
        println!("VERDICT: NOT APPLICABLE (not a Mac)");
        return ExitCode::SUCCESS;
    }
    let ncpu = num(&out_of("sysctl", &["-n", "hw.ncpu"]));
    let heavy = std::env::var("BAG_HEAVY_CPU").ok().filter(|v| !v.is_empty()).map_or(15.0, |v| num(&v)); // % of ONE core that counts as "heavy"
    let (mut mode, mut interval, mut eta_min) = ("report", "60".to_string(), 0);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--eta" => {
                eta_min = parse_min(args.get(i + 1).map_or("", String::as_str));
                i += 1;
            }
            "--throttle" => mode = "throttle",
            "--restore" => mode = "restore",
            "--watch" => {
                mode = "watch";
                if let Some(n) = args.get(i + 1).filter(|n| !n.is_empty() && !n.starts_with("--")) {
                    interval = n.clone();
                    i += 1;
                }
            }
            other => {
                eprintln!("unknown option: {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let card = Card { ncpu, heavy, eta_min, start: Instant::now() };
    let taskpolicy = |flag: &str, pid: &str| {
        Command::new("taskpolicy")
            .args([flag, "-p", pid])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    match mode {
        "throttle" => {
            for p in throttle_pids() {
                if taskpolicy("-b", &p) {
                    println!("throttled {p}");
                }
            }
            card.report();
        }
        "restore" => {
            for p in throttle_pids() {
                if taskpolicy("-B", &p) {
                    println!("restored {p}");
                }
            }
        }
        "watch" => loop {
            let _ = Command::new("clear").status();
            print!("{}", out_of("date", &[]));
            card.report();
            std::thread::sleep(Duration::from_secs_f64(interval.parse::<f64>().unwrap_or(60.0).max(0.0)));
        },
        _ => card.report(),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_formats() {
        for (s, want) in [("45", 45), ("45m", 45), ("2h", 120), ("1h30m", 90), ("1H5M", 65), ("", 0), ("abc", 0)] {
            assert_eq!(parse_min(s), want, "{s:?}");
        }
        assert_eq!(fmt_min(5), "5m");
        assert_eq!(fmt_min(60), "1h00m");
        assert_eq!(fmt_min(112), "1h52m");
    }

    #[test]
    fn classify_picks_the_first_match() {
        assert_eq!(classify("/usr/sbin/WindowServer").1, "when the lid closes");
        assert_eq!(classify("/Applications/Claude.app/Contents/MacOS/Claude").1, "when it is hidden");
        assert_eq!(classify("/usr/local/bin/claude").1, "when its task ends");
        assert_eq!(classify("/x/node_modules/.bin/vite").1, "never - stop it");
        assert_eq!(classify("/usr/bin/node"), ("📜 script (node)".to_string(), "unknown"));
        assert_eq!(classify("/usr/libexec/other"), ("❔ other".to_string(), "unknown"));
    }
}
