//! `dots uninstall`: take back what `dots` put in the home folder. It removes the links into the
//! repo (and puts back the files they replaced) and the owner rules link. The repo, the tools,
//! your settings, `~/.claude/settings.json` and the ps plugin (with the Cursor and Opencode config
//! it wrote) stay.

use super::{agent_kit, ask, editor_links, link, links, os};
use crate::out::{info, ok, step, step_end};
use std::path::{Path, PathBuf};

/// The files under home/ that `dots` links, as `(repo file, link)`.
fn home_links(repo: &Path, home: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut v: Vec<(PathBuf, PathBuf)> = links()
        .iter()
        .map(|i| (repo.join("home").join(i), home.join(i)))
        .collect();
    let starship = ".config/starship.toml";
    v.push((repo.join("home").join(starship), home.join(starship)));
    v.extend(editor_links(repo, home));
    v
}

/// Remove everything. Only a link into `repo`, a file with the `generated-by` marker and an
/// entry equal to ours go; anything else is the user's.
pub fn undo(repo: &Path, home: &Path) {
    step("Links");
    for (src, dst) in home_links(repo, home) {
        link::unlink_item(&src, &dst);
    }
    step("Claude");
    let dst = home.join(".claude");
    agent_kit::drop_old_links(repo, home);
    link::unlink_item(
        &home.join(".config/dotfiles/owner.md"),
        &dst.join("rules").join("owner.md"),
    );
    step_end();
}

/// `dots uninstall [-y]`. Asks first at a terminal; with no terminal it needs `-y`.
pub fn run(repo: &Path, yes: bool) -> i32 {
    if !yes {
        if !os::at_terminal() {
            eprintln!(
                "dots uninstall asks first and there is no terminal. Run it with -y to skip the question."
            );
            return 2;
        }
        let labels = [
            "No, keep everything".to_string(),
            "Yes, remove the links and generated files".to_string(),
        ];
        if ask::one("Remove what dots put in your home folder?", &labels).as_deref()
            != Some(labels[1].as_str())
        {
            info("nothing changed");
            return 0;
        }
    }
    undo(repo, &os::home());
    ok("uninstalled");
    info(
        "Left on purpose: this repo, your tools (mise, Homebrew apps, Oh My Zsh, toolchains), your settings in ~/.config/dotfiles and ~/.claude/settings.json, and the ps plugin (remove it with: claude plugin uninstall ps@agent-kit).",
    );
    info("To finish: delete the repo folder and ~/.config/dotfiles if you want them gone too.");
    0
}
