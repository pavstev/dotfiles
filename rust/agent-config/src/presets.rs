//! The opt-in presets: their names, which are on (the `PRESETS` setting) and the profile. The
//! `dots settings` menu uses it. The ps plugin (pavstev/agent-kit) holds the preset rules and
//! reads the same `PRESETS` setting. Everything here reads only.

use crate::out::warn;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

/// Every preset of the ps plugin, sorted. Fragments merge in this order.
pub const ALL: [&str; 6] = [
    "clock",
    "models",
    "no-ai-attribution",
    "plain-writing",
    "ship-to-main",
    "workflow",
];

/// The preset that a new shared install leaves off: a push to a work repo's main can hurt.
const OFF_FOR_SHARED: &str = "ship-to-main";

/// The kind of machine (AGENTS.md "Profiles"): `owner` loads the private rules, `shared` does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    Owner,
    #[default]
    Shared,
}

/// The text is neither `owner` nor `shared`.
#[derive(Debug, thiserror::Error)]
#[error("unknown profile {0:?}: use owner or shared")]
pub struct UnknownProfile(String);

impl Profile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Shared => "shared",
        }
    }
}

impl FromStr for Profile {
    type Err = UnknownProfile;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "owner" => Ok(Self::Owner),
            "shared" => Ok(Self::Shared),
            other => Err(UnknownProfile(other.to_string())),
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name that is no profile reads as `shared`.
impl From<&str> for Profile {
    fn from(s: &str) -> Self {
        s.parse().unwrap_or_default()
    }
}

impl From<&String> for Profile {
    fn from(s: &String) -> Self {
        s.as_str().into()
    }
}

impl From<&Self> for Profile {
    fn from(p: &Self) -> Self {
        *p
    }
}

/// The profile named by `DOTFILES_PROFILE`, else by the `PROFILE` setting; empty when neither is set.
pub fn named_profile(home: &Path) -> String {
    let p = std::env::var("DOTFILES_PROFILE").unwrap_or_default();
    if p.is_empty() {
        crate::setting(home, "PROFILE")
    } else {
        p
    }
}

/// The profile: the named one, else `owner` when the private rules exist, else `shared`. Read
/// only (`dots` saves the `owner` guess itself).
pub fn current_profile(home: &Path) -> Profile {
    let named = named_profile(home);
    if !named.is_empty() {
        named.as_str().into()
    } else if home.join(".config/dotfiles/owner.md").is_file() {
        Profile::Owner
    } else {
        Profile::Shared
    }
}

/// The presets that are on, in `ALL` order, and the names in the setting that no preset has.
fn parse(home: &Path, profile: Profile) -> (Vec<String>, Vec<String>) {
    let raw = crate::setting(home, "PRESETS");
    let words: Vec<&str> = raw.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    let default = || -> Vec<String> {
        ALL.iter()
            .filter(|n| profile == Profile::Owner || **n != OFF_FOR_SHARED)
            .map(ToString::to_string)
            .collect()
    };
    if words.is_empty() {
        return (default(), Vec::new());
    }
    if words == ["none"] {
        return (Vec::new(), Vec::new());
    }
    if words.contains(&"all") {
        return (ALL.iter().map(ToString::to_string).collect(), Vec::new());
    }
    let want: BTreeSet<&str> = words.iter().copied().collect();
    let on = ALL
        .iter()
        .filter(|n| want.contains(**n))
        .map(ToString::to_string)
        .collect();
    let unknown = words
        .iter()
        .filter(|w| !ALL.contains(w))
        .map(ToString::to_string)
        .collect();
    (on, unknown)
}

/// The presets that are on: the `PRESETS` setting (`all`, `none`, or a list of names), else the
/// profile default (owner: every preset; shared: every preset but `ship-to-main`). A name that is
/// no preset is dropped.
pub fn enabled_quiet(home: &Path, profile: impl Into<Profile>) -> Vec<String> {
    parse(home, profile.into()).0
}

/// One line for a menu or a note: the presets that are on.
pub fn summary(home: &Path, profile: impl Into<Profile>) -> String {
    let on = enabled_quiet(home, profile);
    if on.len() == ALL.len() {
        "all".into()
    } else if on.is_empty() {
        "none".into()
    } else {
        on.join(" ")
    }
}

/// The settings fragments of the named presets, in the order given: `claude/preset-settings/
/// <name>.json` of the repo (the plugin cannot set these keys, so they stay here). A missing file
/// adds nothing; a broken one is skipped with a warning.
pub fn fragments(repo: &Path, names: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for n in names {
        let p = repo
            .join("claude")
            .join("preset-settings")
            .join(format!("{n}.json"));
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(v) if v.is_object() => out.push(v),
            _ => warn(&format!(
                "{} is not a JSON object, so the {} fragment is left out",
                crate::tilde(&p),
                n
            )),
        }
    }
    out
}
