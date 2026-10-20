//! The install steps (dots/install), in order, run in this process.

use super::{install, os, settings};
use crate::out::{info, step_end};
use std::path::Path;

/// The only steps a Windows run executes.
const WINDOWS: [&str; 2] = ["12-tools", "57-claude-code"];

/// The step names, in run order.
pub fn names() -> Vec<String> { install::STEPS.iter().map(|s| s.to_string()).collect() }

/// Whether a step runs. `only`: DOTFILES_STEPS (runs just those, also ones in
/// SKIP_STEPS). `skip`: SKIP_STEPS, when it applies. Err: skipped, with why.
pub fn wanted(name: &str, only: &str, skip: &str, windows: bool) -> Result<(), Option<String>> {
    let only_set = !only.is_empty();
    if only_set && !only.split_whitespace().any(|s| s == name) { return Err(None); }
    if windows && !only_set && !WINDOWS.contains(&name) { return Err(None); }
    if skip.split_whitespace().any(|s| s == name) {
        return Err(Some(format!("{} skipped (Steps to skip, in dots settings)", name)));
    }
    Ok(())
}

/// Run the steps. `own_steps`: DOTFILES_STEPS was set by `dots settings`, not by the user.
pub fn run_all(_repo: &Path, own_steps: bool) {
    let only = std::env::var("DOTFILES_STEPS").unwrap_or_default();
    let skip = if only.is_empty() || own_steps { settings::get("SKIP_STEPS") } else { String::new() };
    for name in install::STEPS {
        match wanted(name, &only, &skip, os::is_windows()) {
            Err(None) => continue,
            Err(Some(why)) => { info(&why); continue; }
            Ok(()) => {}
        }
        step_end();
        if let Some(f) = install::find(name) {
            f(_repo);
            step_end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters() {
        assert!(wanted("30-omz-plugins", "", "", false).is_ok());
        assert_eq!(wanted("30-omz-plugins", "60-cursor", "", false), Err(None));
        assert!(wanted("60-cursor", "60-cursor", "60-cursor", false).is_err()); // skip applies when given
        assert!(wanted("60-cursor", "60-cursor", "", false).is_ok());
        assert_eq!(wanted("30-omz-plugins", "", "", true), Err(None));
        assert!(wanted("57-claude-code", "", "", true).is_ok());
        assert!(wanted("12-tools", "", "", true).is_ok());
        assert_eq!(wanted("77-dotfiles-git-hooks", "", "77-dotfiles-git-hooks", false),
            Err(Some("77-dotfiles-git-hooks skipped (Steps to skip, in dots settings)".into())));
        assert_eq!(wanted("none", "none", "", false), Ok(()));
    }
}
