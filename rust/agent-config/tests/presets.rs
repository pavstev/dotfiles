//! The preset settings fragments (`claude/preset-settings/`), the neutral base and neutral text in
//! `claude/`, the profile, and `agent-config claude-settings demote`. The preset rules live in the
//! ps plugin (pavstev/agent-kit) with their tests.

#![allow(clippy::unwrap_used, reason = "a test fails by panicking")]
mod common;

use common::{rj, run_bin, temp_home, wr};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const AC: &str = env!("CARGO_BIN_EXE_agent-config");

fn claude_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("claude")
}

/// Every text file under `claude/` (skips binaries and build output).
fn claude_text_files() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(claude_dir())
        .into_iter()
        .filter_map(Result::ok)
    {
        if !e.file_type().is_file() {
            continue;
        }
        if let Ok(text) = fs::read_to_string(e.path()) {
            out.push((e.path().to_path_buf(), text));
        }
    }
    assert!(!out.is_empty(), "walked no files under claude/");
    out
}

#[test]
fn no_personal_words() {
    let city: String = "anneiV".chars().rev().collect();
    let words = [city.as_str(), "Monday to Saturday", "Mon-Sat", "my brain"];
    let mut hits = Vec::new();
    for (p, text) in claude_text_files() {
        for (i, l) in text.lines().enumerate() {
            for w in words {
                if l.contains(w) {
                    hits.push(format!("{}:{}: {w}", p.display(), i + 1));
                }
            }
            // "the owner" alone is personal; "the owner profile" and "the owner overlay" name this repo's profile.
            let mut rest = l;
            while let Some(at) = rest.find("the owner") {
                let after = &rest[at + "the owner".len()..];
                if !(after.starts_with(" profile") || after.starts_with(" overlay")) {
                    hits.push(format!("{}:{}: the owner", p.display(), i + 1));
                }
                rest = after;
            }
        }
    }
    assert!(
        hits.is_empty(),
        "personal words in claude/:\n{}",
        hits.join("\n")
    );
}

#[test]
fn base_settings_neutral() {
    let text = fs::read_to_string(claude_dir().join("settings.json")).unwrap();
    let base: Value = serde_json::from_str(&text).unwrap();
    for key in [
        "model",
        "outputStyle",
        "enabledPlugins",
        "extraKnownMarketplaces",
        "skillOverrides",
        "attribution",
        "includeCoAuthoredBy",
    ] {
        assert!(base.get(key).is_none(), "base settings still has {key}");
    }
    assert!(base.pointer("/env/CLAUDE_CODE_SUBAGENT_MODEL").is_none());
    for d in base["permissions"]["deny"].as_array().unwrap() {
        let d = d.as_str().unwrap();
        let id = d
            .strip_prefix("mcp__")
            .and_then(|r| r.strip_suffix("__*"))
            .unwrap_or("");
        let uuid_like = id.len() == 36 && id.matches('-').count() == 4;
        assert!(!uuid_like, "base settings names a connector id: {d}");
    }
}

// --- claude-settings demote -----------------------------------------------------

struct Fx {
    tmp: PathBuf,
    home: PathBuf,
    repo: PathBuf,
}

impl Fx {
    fn new(tag: &str, base: &Value) -> Self {
        let tmp = temp_home(tag);
        let (home, repo) = (tmp.join("home"), tmp.join("repo"));
        fs::create_dir_all(&home).unwrap();
        let fx = Self { tmp, home, repo };
        fx.set_base(base);
        fx
    }
    fn set_base(&self, base: &Value) {
        let p = self.repo.join("claude").join("settings.json");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, base.to_string()).unwrap();
    }
    fn ac(&self, args: &[&str]) -> (i32, String, String) {
        let (home, repo) = (self.home.to_str().unwrap(), self.repo.to_str().unwrap());
        run_bin(
            AC,
            args,
            "",
            &[
                ("HOME", home),
                ("DOTFILES_REPO", repo),
                ("DOTFILES_VERBOSE", "1"),
            ],
            None,
        )
    }
    fn live(&self) -> PathBuf {
        self.home.join(".claude/settings.json")
    }
    fn layer(&self) -> PathBuf {
        self.home.join(".config/dotfiles/claude-settings.json")
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

#[test]
fn demote_copies_live_value() {
    let plugins = json!({"cloudflare@cloudflare": false, "data@synced": false});
    let old_base = json!({"theme": "dark", "enabledPlugins": plugins});
    let fx = Fx::new("presets-demote", &old_base);
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    assert_eq!(rj(&fx.live())["enabledPlugins"], plugins);
    // The repo base drops the key. A plain build would now lose it from the live file.
    fx.set_base(&json!({"theme": "dark"}));
    let (code, out, _) = fx.ac(&["claude-settings", "demote", "enabledPlugins"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(rj(&fx.layer())["enabledPlugins"], plugins);
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    assert_eq!(rj(&fx.live())["enabledPlugins"], plugins);
    // Run again: nothing changes.
    let before = fs::read_to_string(fx.layer()).unwrap();
    assert_eq!(fx.ac(&["claude-settings", "demote", "enabledPlugins"]).0, 0);
    assert_eq!(fs::read_to_string(fx.layer()).unwrap(), before);
}

#[test]
fn demote_a_nested_key_keeps_its_siblings() {
    let fx = Fx::new(
        "presets-demote-nested",
        &json!({"env": {"CLAUDE_CODE_SUBAGENT_MODEL": "sonnet", "KEEP": "1"}}),
    );
    fx.ac(&["claude-settings", "build"]);
    fx.set_base(&json!({"env": {"KEEP": "1"}}));
    let (code, _, _) = fx.ac(&[
        "claude-settings",
        "demote",
        "env.CLAUDE_CODE_SUBAGENT_MODEL",
    ]);
    assert_eq!(code, 0);
    assert_eq!(
        rj(&fx.layer()),
        json!({"env": {"CLAUDE_CODE_SUBAGENT_MODEL": "sonnet"}})
    );
    fx.ac(&["claude-settings", "build"]);
    assert_eq!(
        rj(&fx.live())["env"],
        json!({"KEEP": "1", "CLAUDE_CODE_SUBAGENT_MODEL": "sonnet"})
    );
}

#[test]
fn demote_missing_key_is_noop() {
    let fx = Fx::new("presets-demote-missing", &json!({"theme": "dark"}));
    fx.ac(&["claude-settings", "build"]);
    let (code, out, err) = fx.ac(&["claude-settings", "demote", "enabledPlugins"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(!fx.layer().exists(), "a no-op must not create the layer");
    assert!(format!("{out}{err}").contains("not in"), "{out}{err}");
    // No live file at all is the same.
    let fx2 = Fx::new("presets-demote-nolive", &json!({"theme": "dark"}));
    let (code, _, _) = fx2.ac(&["claude-settings", "demote", "model"]);
    assert_eq!(code, 0);
    assert!(!fx2.layer().exists());
}

// --- dots: presets, new settings, theme -------------------------------------------

use agent_config::presets;

/// A fake repo with one preset settings fragment in a temp HOME.
fn px(tag: &str) -> Fx {
    let fx = Fx::new(tag, &json!({}));
    let c = fx.repo.join("claude");
    wr(
        &c.join("preset-settings/plain-writing.json"),
        r#"{"outputStyle": "ELI5"}"#,
    );
    fx
}

fn setting_file(fx: &Fx, text: &str) {
    wr(&fx.home.join(".config/dotfiles/settings"), text);
}

fn real_repo() -> PathBuf {
    claude_dir().parent().unwrap().to_path_buf()
}

#[test]
fn unknown_preset_is_skipped() {
    let home = temp_home("presets-unknown");
    wr(
        &home.join(".config/dotfiles/settings"),
        "PRESETS=\"workflow nope\"\n",
    );
    assert_eq!(presets::enabled_quiet(&home, "shared"), ["workflow"]);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn fragment_order() {
    let fx = px("presets-fragment-order");
    fx.set_base(&json!({"outputStyle": "default"}));
    setting_file(&fx, "PRESETS=\"plain-writing\"\n");
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    assert_eq!(rj(&fx.live())["outputStyle"], "ELI5");
    wr(&fx.layer(), r#"{"outputStyle": "concise"}"#);
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    assert_eq!(rj(&fx.live())["outputStyle"], "concise");
}

#[test]
fn fragments_leave_with_their_preset() {
    let fx = px("presets-fragment-off");
    fx.set_base(&json!({"theme": "dark"}));
    setting_file(&fx, "PRESETS=\"plain-writing\"\n");
    fx.ac(&["claude-settings", "build"]);
    assert_eq!(rj(&fx.live())["outputStyle"], "ELI5");
    setting_file(&fx, "PRESETS=\"workflow\"\n");
    fx.ac(&["claude-settings", "build"]);
    assert!(rj(&fx.live()).get("outputStyle").is_none());
    assert!(
        !fx.layer().exists(),
        "a preset key must not drift into the private layer"
    );
}

#[test]
fn co_author_switch_follows_the_no_ai_attribution_preset() {
    // the real repo, a temp HOME: built with the preset on, then off
    let home = temp_home("presets-co-author");
    let (h, r) = (
        home.to_str().unwrap().to_string(),
        real_repo().to_str().unwrap().to_string(),
    );
    let build = |settings: &str| -> Value {
        wr(&home.join(".config/dotfiles/settings"), settings);
        let (code, out, err) = run_bin(
            AC,
            &["claude-settings", "build"],
            "",
            &[("HOME", &h), ("DOTFILES_REPO", &r)],
            None,
        );
        assert_eq!(code, 0, "{out}{err}");
        rj(&home.join(".claude/settings.json"))
    };
    let on = build("PRESETS=\"no-ai-attribution\"\n");
    assert_eq!(on["includeCoAuthoredBy"], json!(false), "{on}");
    assert_eq!(on["attribution"], json!({"commit": "", "pr": ""}));
    let off = build("PRESETS=\"workflow\"\n");
    assert!(off.get("includeCoAuthoredBy").is_none(), "{off}");
    assert!(off.get("attribution").is_none(), "{off}");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn upgrade_keeps_private_layer() {
    let plugins = json!({"a@b": false});
    let fx = px("presets-upgrade");
    fx.set_base(&json!({"theme": "dark", "enabledPlugins": plugins}));
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    wr(&fx.layer(), r#"{"mine": "yes"}"#);
    fx.set_base(&json!({"theme": "dark"}));
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    let live = rj(&fx.live());
    assert_eq!(live["mine"], "yes");
    assert!(live.get("enabledPlugins").is_none(), "{live}");
}

#[test]
fn profile_parses_and_prints() {
    use presets::Profile;
    assert_eq!("owner".parse::<Profile>().unwrap(), Profile::Owner);
    assert_eq!("shared".parse::<Profile>().unwrap(), Profile::Shared);
    assert!("root".parse::<Profile>().is_err() && "".parse::<Profile>().is_err());
    // callers that take a name read an unknown one as shared
    assert_eq!(Profile::from("root"), Profile::Shared);
    assert_eq!(Profile::Owner.to_string(), "owner");
    assert_eq!(Profile::default().to_string(), "shared");
}

#[test]
fn profile_read_only() {
    let home = temp_home("presets-profile");
    wr(&home.join(".config/dotfiles/owner.md"), "x\n");
    assert_eq!(presets::current_profile(&home), presets::Profile::Owner);
    assert!(
        !home.join(".config/dotfiles/settings").exists(),
        "current_profile() must not write the settings file"
    );
    wr(
        &home.join(".config/dotfiles/settings"),
        "PROFILE=\"shared\"\n",
    );
    assert_eq!(presets::current_profile(&home), presets::Profile::Shared);
    assert_eq!(
        fs::read_to_string(home.join(".config/dotfiles/settings")).unwrap(),
        "PROFILE=\"shared\"\n"
    );
    let _ = fs::remove_dir_all(&home);
}
