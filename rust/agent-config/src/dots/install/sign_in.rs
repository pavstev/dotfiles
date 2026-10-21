//! 90-sign-in: the sign-ins the tools need (GitHub, Claude Code, Opencode, Cursor). On the
//! first run at a terminal: a menu, then each picked login runs here. Later runs, or no
//! terminal: one line that says what is left.

use super::super::{ask, os};
use super::run;
use crate::out::{self, info, skip, verbose, warn};
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

/// Runs a command and gives (it exited 0, what it printed on stdout).
type Probe<'a> = &'a (dyn Fn(&str, &[&str]) -> (bool, String) + Sync);

/// PATH as it was; puts it back when the step ends (the bash step had its own process).
struct PathGuard(Option<OsString>);
impl Drop for PathGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(p) => os::set_env("PATH", p),
            None => os::unset_env("PATH"),
        }
    }
}

/// `claude auth status` prints JSON with `"loggedIn": true`.
fn claude_logged_in(status: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(status).is_ok_and(|v| v["loggedIn"] == true)
}

/// A tool that needs a sign-in: how to ask it, and how to read the answer.
struct Login {
    /// The program to probe and its arguments.
    prog: &'static str,
    args: &'static [&'static str],
    /// The menu label and the command that signs in.
    label: &'static str,
    cmd: &'static str,
    /// True when the probe (exit 0, stdout) says the user is signed out.
    signed_out: fn(bool, &str) -> bool,
}

const LOGINS: [Login; 4] = [
    Login {
        prog: "gh",
        args: &["auth", "status"],
        label: "\u{1F419} GitHub",
        cmd: "gh auth login",
        signed_out: |ok, _| !ok,
    },
    Login {
        prog: "claude",
        args: &["auth", "status"],
        label: "\u{2733}\u{FE0F}  Claude Code",
        cmd: "claude auth login",
        signed_out: |_, out| !claude_logged_in(out),
    },
    Login {
        prog: "opencode",
        args: &["auth", "list"],
        label: "\u{1F33F} Opencode",
        cmd: "opencode auth login",
        signed_out: |_, out| out.to_lowercase().contains("no authenticated"),
    },
    Login {
        prog: "agent",
        args: &["status"],
        label: "\u{1F5B1}\u{FE0F}  Cursor",
        cmd: "agent login",
        signed_out: |_, out| !out.to_lowercase().contains("logged in as"),
    },
];

/// The logins still to do: (menu label, command), for the tools that exist and are not signed in.
/// The probes of the tools that exist run at the same time.
fn pending(have: &dyn Fn(&str) -> bool, probe: Probe) -> Vec<(&'static str, &'static str)> {
    std::thread::scope(|s| {
        let probes: Vec<_> =
            LOGINS.iter().filter(|l| have(l.prog)).map(|l| (l, s.spawn(|| probe(l.prog, l.args)))).collect();
        probes
            .into_iter()
            .filter_map(|(l, h)| {
                let (ok, out) = h.join().ok()?;
                (l.signed_out)(ok, &out).then_some((l.label, l.cmd))
            })
            .collect()
    })
}

/// Run a probe for real: stderr dropped, stdin closed.
fn probe_real(prog: &str, args: &[&str]) -> (bool, String) {
    match Command::new(prog).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
        Err(_) => (false, String::new()),
    }
}

pub fn step(_repo: &Path) {
    os::brew_env();
    let _restore = PathGuard(std::env::var_os("PATH"));
    let local_bin = os::home().join(".local").join("bin");
    if local_bin.is_dir() {
        // claude, agent
        let mut parts = vec![local_bin];
        parts.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        if let Ok(joined) = std::env::join_paths(parts) {
            os::set_env("PATH", joined);
        }
    }

    out::step("Sign in");
    let list = pending(&os::have, &probe_real);
    if list.is_empty() {
        if verbose() {
            skip("sign-ins");
        }
        return;
    }
    let first_run = std::env::var("DOTFILES_FIRST_RUN").is_ok_and(|v| !v.is_empty());
    if !first_run || !os::at_terminal() {
        for (_, c) in &list {
            info(&format!("not signed in yet, run: {c}"));
        }
        return;
    }

    let labels: Vec<&str> = list.iter().map(|(l, _)| *l).collect();
    let yes = format!("\u{2705} Yes, sign in to: {}", labels.join(", "));
    let later = "\u{23ED}\u{FE0F}  Later (dots shows the commands)".to_string();
    let answer =
        ask::one("\u{1F511} Sign in now? A browser opens for each one.", &[yes.clone(), later]).unwrap_or_default();
    if answer != yes {
        for (_, c) in &list {
            info(&format!("later, run: {c}"));
        }
        return;
    }
    for (label, cmd) in &list {
        info(&format!("{label}: {cmd}"));
        let words: Vec<&str> = cmd.split_whitespace().collect();
        if !run(words[0], &words[1..]) {
            warn(&format!("{cmd} did not finish. Run it again later."));
        }
        if os::is_macos()
            && *cmd == "agent login"
            && Path::new("/Applications/Cursor.app").is_dir()
            && run("open", &["-a", "Cursor"])
        {
            info("sign in inside the Cursor app too");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(logged: &'static [(&'static str, bool, &'static str)]) -> impl Fn(&str, &[&str]) -> (bool, String) {
        move |p, _| {
            logged
                .iter()
                .find(|(n, _, _)| *n == p)
                .map_or((false, String::new()), |(_, ok, out)| (*ok, out.to_string()))
        }
    }

    #[test]
    fn claude_status_text() {
        assert!(claude_logged_in("{\n  \"loggedIn\": true\n}"));
        assert!(claude_logged_in("{\"loggedIn\":true}"));
        assert!(!claude_logged_in("{\"loggedIn\": false}"));
        assert!(!claude_logged_in(""));
    }

    #[test]
    fn only_tools_that_exist_and_are_signed_out_are_listed() {
        assert_eq!(pending(&|_| false, &fake(&[])), [] as [(&str, &str); 0]); // nothing installed: nothing to do
        let probes = fake(&[
            ("gh", false, ""),
            ("claude", true, "{\"loggedIn\": true}"),
            ("opencode", true, "0 credentials: No authenticated providers"),
            ("agent", true, "Logged in as someone"),
        ]);
        let all = pending(&|_| true, &probes);
        let cmds: Vec<&str> = all.iter().map(|(_, c)| *c).collect();
        assert_eq!(cmds, ["gh auth login", "opencode auth login"]);
        let only_agent = pending(&|c| c == "agent", &fake(&[("agent", true, "Not logged in")]));
        assert_eq!(only_agent, [("\u{1F5B1}\u{FE0F}  Cursor", "agent login")]);
        let claude_out = pending(&|c| c == "claude", &fake(&[("claude", true, "{\"loggedIn\": false}")]));
        assert_eq!(claude_out[0].1, "claude auth login");
    }
}
