//! The opt-in presets under `claude/presets/<name>/`: which are on, their rules text, their
//! settings fragments and the links in `~/.claude/rules/`.
//!
//! Everything here reads only, except `link_all`, which makes and removes links it owns.
//! `profile` never writes the settings file, so the guard can call it on every hook run.

use crate::out::{ok, warn};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Every preset folder, sorted. Fragments merge in this order.
pub const ALL: [&str; 7] =
    ["clock", "models", "no-ai-attribution", "pace", "plain-writing", "ship-to-main", "workflow"];

/// The preset that a new shared install leaves off: a push to a work repo's main can hurt.
const OFF_FOR_SHARED: &str = "ship-to-main";

/// The profile: `DOTFILES_PROFILE`, else the `PROFILE` setting, else `owner` when the private
/// rules exist, else `shared`. Read only (`dots` saves the `owner` guess itself).
pub fn profile(home: &Path) -> String {
    let mut p = std::env::var("DOTFILES_PROFILE").unwrap_or_default();
    if p.is_empty() {
        p = crate::setting(home, "PROFILE");
    }
    if p.is_empty() && home.join(".config/dotfiles/owner.md").is_file() {
        p = "owner".into();
    }
    if p == "owner" || p == "shared" { p } else { "shared".into() }
}

/// The presets that are on, in `ALL` order, and the names in the setting that no preset has.
fn parse(home: &Path, profile: &str) -> (Vec<String>, Vec<String>) {
    let raw = crate::setting(home, "PRESETS");
    let words: Vec<&str> = raw.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    let default = |p: &str| -> Vec<String> {
        ALL.iter().filter(|n| p == "owner" || **n != OFF_FOR_SHARED).map(|n| n.to_string()).collect()
    };
    if words.is_empty() {
        return (default(profile), Vec::new());
    }
    if words == ["none"] {
        return (Vec::new(), Vec::new());
    }
    if words.contains(&"all") {
        return (ALL.iter().map(|n| n.to_string()).collect(), Vec::new());
    }
    let want: BTreeSet<&str> = words.iter().copied().collect();
    let on = ALL.iter().filter(|n| want.contains(**n)).map(|n| n.to_string()).collect();
    let unknown = words.iter().filter(|w| !ALL.contains(w)).map(|w| w.to_string()).collect();
    (on, unknown)
}

/// The presets that are on: the `PRESETS` setting (`all`, or a list of names), else the profile
/// default (owner: every preset; shared: every preset but `ship-to-main`). A name that is no preset
/// is dropped, with one warning per run. `none` alone turns every preset off.
pub fn enabled(home: &Path, profile: &str) -> Vec<String> {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let (on, unknown) = parse(home, profile);
    if !unknown.is_empty() && !WARNED.swap(true, Ordering::Relaxed) {
        warn(&format!(
            "PRESETS names no preset called {}, so it is left out. Presets: {}",
            unknown.join(", "),
            ALL.join(" ")
        ));
    }
    on
}

/// Like `enabled`, with no warning: for the guard, which must stay quiet.
pub fn enabled_quiet(home: &Path, profile: &str) -> Vec<String> {
    parse(home, profile).0
}

fn dir(repo: &Path, name: &str) -> PathBuf {
    repo.join("claude").join("presets").join(name)
}

/// The rules of the named presets without their front matter, one blank line apart. A preset with
/// no `rules.md` adds nothing.
pub fn rules_text(repo: &Path, names: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for n in names {
        let Ok(text) = fs::read_to_string(dir(repo, n).join("rules.md")) else { continue };
        let (_, body) = crate::split_frontmatter(&text);
        let body = body.trim();
        if !body.is_empty() {
            parts.push(body.to_string());
        }
    }
    parts.join("\n\n")
}

/// The settings fragments of the named presets, in the order given. A missing or broken file is
/// skipped (a broken one with a warning).
pub fn fragments(repo: &Path, names: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for n in names {
        let p = dir(repo, n).join("settings.json");
        let Ok(text) = fs::read_to_string(&p) else { continue };
        match serde_json::from_str::<Value>(&text) {
            Ok(v) if v.is_object() => out.push(v),
            _ => warn(&format!("{} is not a JSON object, so the {} fragment is left out", crate::tilde(&p), n)),
        }
    }
    out
}

/// A link of ours: `preset-<name>.md` that points into the repo's presets.
fn ours(link: &Path, presets: &Path) -> bool {
    crate::is_symlink(link) && fs::read_link(link).is_ok_and(|t| t.starts_with(presets))
}

/// Link `~/.claude/rules/preset-<name>.md` to each enabled preset and remove the links of the
/// others. Only links into the repo's presets are ever removed or replaced; a regular file, or a
/// link that points elsewhere, stays as it is (with a warning when it blocks an enabled preset).
pub fn link_all(repo: &Path, home: &Path, names: &[String]) {
    let rules = home.join(".claude").join("rules");
    let presets = repo.join("claude").join("presets");
    for n in names {
        let src = dir(repo, n).join("rules.md");
        let dst = rules.join(format!("preset-{n}.md"));
        if !src.is_file() {
            continue;
        }
        if crate::dots::os::exists_or_link(&dst) && !ours(&dst, &presets) {
            warn(&format!(
                "{} is not a link of ours, so the {} preset is not linked. Move it away to use the preset.",
                crate::tilde(&dst),
                n
            ));
            continue;
        }
        crate::dots::link::link_item(&src, &dst, true);
    }
    let Ok(entries) = fs::read_dir(&rules) else { return };
    for e in entries.filter_map(|e| e.ok()) {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some(name) = file.strip_prefix("preset-").and_then(|r| r.strip_suffix(".md")) else { continue };
        if names.iter().any(|n| n == name) || !ours(&e.path(), &presets) {
            continue;
        }
        if fs::remove_file(e.path()).is_ok() {
            ok(&format!("removed the {} preset link", name));
        }
    }
}

/// One line for a menu or a note: the presets that are on.
pub fn summary(home: &Path, profile: &str) -> String {
    let on = enabled_quiet(home, profile);
    if on.len() == ALL.len() {
        "all".into()
    } else if on.is_empty() {
        "none".into()
    } else {
        on.join(" ")
    }
}
