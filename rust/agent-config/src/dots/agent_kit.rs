//! The hand-over to the ps plugin (github.com/pavstev/agent-kit). The plugin now holds the AI
//! harness: rules, presets, skills, agents, output styles, the guard hooks, the status line and
//! the Cursor and Opencode config. `dots` only:
//!
//! 1. installs the plugin with the `claude` CLI when it is missing,
//! 2. removes the old links into this repo's `claude/` folder (they would load everything twice),
//! 3. runs the plugin's own `agent-config` for Cursor and Opencode, once its binaries are there.

use super::os;
use crate::out::{note, ok, warn};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The plugin id in `enabledPlugins`.
pub const PLUGIN: &str = "ps@agent-kit";
/// The marketplace repo on GitHub.
pub const MARKETPLACE: &str = "pavstev/agent-kit";

/// The plugin data folder, where its `ensure` hook puts the binaries.
pub fn data_dir(home: &Path) -> PathBuf {
    home.join(".claude")
        .join("plugins")
        .join("data")
        .join("ps-agent-kit")
}

/// The plugin's launcher: `rust-bin <name> [args]` runs one of its binaries.
pub fn launcher(home: &Path) -> PathBuf {
    data_dir(home).join("bin").join("rust-bin")
}

/// `dst` is a link whose target is inside `root`, even when the target is gone.
fn old_link(dst: &Path, root: &Path) -> bool {
    os::is_symlink(dst) && os::readlink(dst).is_some_and(|t| t.starts_with(root))
}

/// The items of `~/.claude` that older versions of `dots` linked into the repo's `claude/` folder.
const OLD_TOP: [&str; 6] = [
    "agents",
    "hooks",
    "output-styles",
    "skills",
    "CLAUDE.md",
    "statusline.sh",
];

/// The links of ours in `dir` whose target is inside `root`, for the names `want` takes.
fn links_in(dir: &Path, root: &Path, want: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| want(&e.file_name().to_string_lossy()) && old_link(&e.path(), root))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

/// The old links into `<repo>/claude/` in `~/.claude`: the agents, hooks, output styles, skills,
/// global rules and status line, each preset rule and each skill. A file or link that is not ours
/// is not listed. `dots doctor` reports them; `dots` removes them.
pub fn old_links(repo: &Path, home: &Path) -> Vec<PathBuf> {
    let src = repo.join("claude");
    let dst = home.join(".claude");
    let mut v = links_in(&dst, &src, |name| OLD_TOP.contains(&name));
    v.extend(links_in(&dst.join("rules"), &src.join("presets"), |name| {
        name.starts_with("preset-")
            && Path::new(name)
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("md"))
    }));
    v.extend(links_in(&dst.join("skills"), &src.join("skills"), |_| true));
    v
}

/// Remove the old links into `<repo>/claude/` from `~/.claude` ([`old_links`]).
pub fn drop_old_links(repo: &Path, home: &Path) {
    let n = old_links(repo, home)
        .iter()
        .filter(|p| fs::remove_file(p).is_ok())
        .count();
    if n > 0 {
        ok(&format!(
            "removed {n} old links into claude/ (the ps plugin has them now)"
        ));
    }
}

/// True when the `claude` CLI lists the ps plugin as installed.
fn installed() -> bool {
    Command::new("claude")
        .args(["plugin", "list", "--json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains(PLUGIN))
}

fn claude_quiet(args: &[&str]) -> bool {
    Command::new("claude")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Install the ps plugin when the `claude` CLI is there and the plugin is not. `DOTS_NO_PLUGIN=1`
/// skips it (tests, a machine without network).
pub fn ensure_plugin() {
    if std::env::var("DOTS_NO_PLUGIN").is_ok_and(|v| v == "1") || !os::have("claude") {
        return;
    }
    if installed() {
        return;
    }
    // `marketplace add` fails when the marketplace is already known; the install says the rest.
    let _ = claude_quiet(&["plugin", "marketplace", "add", MARKETPLACE]);
    if claude_quiet(&["plugin", "install", PLUGIN]) {
        ok("installed the ps plugin (ps@agent-kit). Its first session fetches its binaries");
    } else {
        warn(&format!(
            "the ps plugin did not install. Run: claude plugin marketplace add {MARKETPLACE} && claude plugin install {PLUGIN}"
        ));
    }
}

/// Run the plugin's `agent-config` with `args` (for example `cursor --profile owner`). Before the
/// plugin has its binaries this only says so, and counts as a success.
pub fn agent_config(home: &Path, args: &[&str]) -> bool {
    let l = launcher(home);
    if !l.is_file() {
        note("the ps plugin has no binaries yet: start Claude Code once, then run dots again");
        return true;
    }
    // DOTFILES_REPO names this repo, not the plugin: the plugin finds its own folder.
    Command::new("sh")
        .arg(&l)
        .arg("agent-config")
        .args(args)
        .env_remove("DOTFILES_REPO")
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_whole_skills_link_into_claude_is_old() {
        let t = tempfile::tempdir().unwrap();
        let (repo, home) = (t.path().join("repo"), t.path().join("home"));
        fs::create_dir_all(repo.join("claude/skills/ps_x")).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        os::symlink(&repo.join("claude/skills"), &home.join(".claude/skills")).unwrap();
        assert_eq!(old_links(&repo, &home), [home.join(".claude/skills")]);
        drop_old_links(&repo, &home);
        assert!(!os::exists_or_link(&home.join(".claude/skills")));
        assert!(repo.join("claude/skills/ps_x").is_dir());
    }

    #[test]
    fn drop_old_links_removes_only_links_into_claude() {
        let t = tempfile::tempdir().unwrap();
        let (repo, home) = (t.path().join("repo"), t.path().join("home"));
        let c = home.join(".claude");
        fs::create_dir_all(c.join("rules")).unwrap();
        fs::create_dir_all(c.join("skills")).unwrap();
        fs::create_dir_all(repo.join("claude")).unwrap();
        // ours, even with the target gone
        os::symlink(&repo.join("claude/hooks"), &c.join("hooks")).unwrap();
        os::symlink(
            &repo.join("claude/presets/clock/rules.md"),
            &c.join("rules/preset-clock.md"),
        )
        .unwrap();
        os::symlink(
            &repo.join("claude/skills/ps_askme"),
            &c.join("skills/ps_askme"),
        )
        .unwrap();
        os::symlink(&repo.join("claude/statusline.sh"), &c.join("statusline.sh")).unwrap();
        assert_eq!(old_links(&repo, &home).len(), 4);
        // not ours
        fs::write(c.join("rules/owner.md"), "mine").unwrap();
        os::symlink(&t.path().join("elsewhere"), &c.join("skills/other")).unwrap();
        fs::write(c.join("CLAUDE.md"), "mine").unwrap();
        drop_old_links(&repo, &home);
        assert_eq!(old_links(&repo, &home), [] as [PathBuf; 0]);
        assert!(!os::exists_or_link(&c.join("hooks")));
        assert!(!os::exists_or_link(&c.join("statusline.sh")));
        assert!(!os::exists_or_link(&c.join("rules/preset-clock.md")));
        assert!(!os::exists_or_link(&c.join("skills/ps_askme")));
        assert!(c.join("rules/owner.md").is_file());
        assert!(os::is_symlink(&c.join("skills/other")));
        assert!(c.join("CLAUDE.md").is_file());
    }
}
