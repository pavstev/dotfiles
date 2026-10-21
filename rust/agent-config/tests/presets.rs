//! The opt-in presets under `claude/presets/`, the neutral core and neutral text in `claude/`, and
//! `agent-config claude-settings demote`.
mod common;

use common::{run_bin, temp_home};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const AC: &str = env!("CARGO_BIN_EXE_agent-config");


fn claude_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("claude")
}

fn lines(p: &Path) -> usize {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())).lines().count()
}

/// `(name, description, guard ids)` from the front matter of a rules file.
fn front_matter(p: &Path) -> (Option<String>, Option<String>, Option<Vec<String>>) {
    let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    let mut it = text.lines();
    assert_eq!(it.next(), Some("---"), "{} must start with front matter", p.display());
    let (mut name, mut desc, mut guard) = (None, None, None);
    for l in it {
        if l == "---" {
            break;
        }
        if let Some(v) = l.strip_prefix("name:") {
            name = Some(v.trim().to_string());
        } else if let Some(v) = l.strip_prefix("description:") {
            desc = Some(v.trim().to_string());
        } else if let Some(v) = l.strip_prefix("guard:") {
            let inner = v.trim().trim_start_matches('[').trim_end_matches(']');
            guard = Some(inner.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
        }
    }
    (name, desc, guard)
}

#[test]
fn presets_well_formed() {
    // which guard ids each preset owns is checked against the guard registry in src/bin/guard.rs
    let names = preset_names();
    assert!(names.contains(&"workflow".to_string()), "{names:?}");
    for name in &names {
        let rules = claude_dir().join("presets").join(name).join("rules.md");
        assert!(rules.is_file(), "{} missing", rules.display());
        let (n, d, g) = front_matter(&rules);
        assert_eq!(n.as_deref(), Some(name.as_str()), "{name}: name must equal the folder");
        assert!(d.as_deref().is_some_and(|d| !d.is_empty()), "{name}: description is empty");
        assert!(g.is_some(), "{name}: guard: line missing");
    }
}

/// The preset folders under claude/presets, sorted.
fn preset_names() -> Vec<String> {
    let dir = claude_dir().join("presets");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn core_is_short() {
    let c = claude_dir();
    let n = lines(&c.join("CLAUDE.md"));
    assert!(n <= 45, "claude/CLAUDE.md has {n} lines, the limit is 45");
    for name in preset_names() {
        let p = c.join("presets").join(name).join("rules.md");
        let n = lines(&p);
        assert!(n <= 30, "{} has {n} lines, the limit is 30", p.display());
    }
}

/// Every text file under `claude/` (skips binaries and build output).
fn claude_text_files() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(claude_dir()).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() {
            continue;
        }
        if let Ok(text) = fs::read_to_string(e.path()) {
            out.push((e.path().to_path_buf(), text));
        }
    }
    assert!(out.len() > 20, "walked too few files under claude/");
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
    assert!(hits.is_empty(), "personal words in claude/:\n{}", hits.join("\n"));
}

#[test]
fn base_settings_neutral() {
    let text = fs::read_to_string(claude_dir().join("settings.json")).unwrap();
    let base: Value = serde_json::from_str(&text).unwrap();
    for key in ["model", "outputStyle", "enabledPlugins", "extraKnownMarketplaces", "skillOverrides", "attribution", "includeCoAuthoredBy"] {
        assert!(base.get(key).is_none(), "base settings still has {key}");
    }
    assert!(base.pointer("/env/CLAUDE_CODE_SUBAGENT_MODEL").is_none());
    for d in base["permissions"]["deny"].as_array().unwrap() {
        let d = d.as_str().unwrap();
        let id = d.strip_prefix("mcp__").and_then(|r| r.strip_suffix("__*")).unwrap_or("");
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
    fn new(tag: &str, base: &Value) -> Fx {
        let tmp = temp_home(tag);
        let (home, repo) = (tmp.join("home"), tmp.join("repo"));
        fs::create_dir_all(&home).unwrap();
        let fx = Fx { tmp, home, repo };
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
        run_bin(AC, args, "", &[("HOME", home), ("DOTFILES_REPO", repo), ("DOTFILES_VERBOSE", "1")], None)
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

fn rj(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
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
    let fx = Fx::new("presets-demote-nested", &json!({"env": {"CLAUDE_CODE_SUBAGENT_MODEL": "sonnet", "KEEP": "1"}}));
    fx.ac(&["claude-settings", "build"]);
    fx.set_base(&json!({"env": {"KEEP": "1"}}));
    let (code, _, _) = fx.ac(&["claude-settings", "demote", "env.CLAUDE_CODE_SUBAGENT_MODEL"]);
    assert_eq!(code, 0);
    assert_eq!(rj(&fx.layer()), json!({"env": {"CLAUDE_CODE_SUBAGENT_MODEL": "sonnet"}}));
    fx.ac(&["claude-settings", "build"]);
    assert_eq!(rj(&fx.live())["env"], json!({"KEEP": "1", "CLAUDE_CODE_SUBAGENT_MODEL": "sonnet"}));
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

fn wr(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// A fake repo (core rules, two presets, the ELI5 output style) in a temp HOME.
fn px(tag: &str) -> Fx {
    let fx = Fx::new(tag, &json!({}));
    let c = fx.repo.join("claude");
    wr(&c.join("CLAUDE.md"), "# Core\n\ncore line\n");
    wr(&c.join("hooks/rust-bin"), "# fake launcher\n");
    wr(&c.join("output-styles/ELI5.md"), "---\nname: ELI5\n---\nExplain it simply.\n");
    wr(&c.join("presets/workflow/rules.md"), "---\nname: workflow\ndescription: d\nguard: [G1]\n---\n# Workflow\n\nworkflow line\n");
    wr(&c.join("presets/plain-writing/rules.md"), "---\nname: plain-writing\ndescription: d\nguard: []\n---\n# Plain\n\nplain line\n");
    wr(&c.join("presets/plain-writing/settings.json"), r#"{"outputStyle": "ELI5"}"#);
    fx
}

fn setting_file(fx: &Fx, text: &str) {
    wr(&fx.home.join(".config/dotfiles/settings"), text);
}

fn real_repo() -> PathBuf {
    claude_dir().parent().unwrap().to_path_buf()
}

fn preset_links(home: &Path) -> Vec<String> {
    let mut v: Vec<String> = match fs::read_dir(home.join(".claude/rules")) {
        Ok(d) => d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("preset-")).collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
}

fn link_enabled(home: &Path, profile: &str) {
    let on = presets::enabled(home, profile);
    presets::link_all(&real_repo(), home, &on);
}

#[test]
fn shared_profile_links_all_but_ship_to_main() {
    let home = temp_home("presets-shared");
    link_enabled(&home, "shared");
    let links = preset_links(&home);
    assert_eq!(links.len(), preset_names().len() - 1, "{links:?}");
    assert!(!links.contains(&"preset-ship-to-main.md".to_string()));
    let t = fs::read_link(home.join(".claude/rules/preset-clock.md")).unwrap();
    assert!(t.ends_with("claude/presets/clock/rules.md"), "{}", t.display());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn presets_list_unlinks_rest() {
    let home = temp_home("presets-list");
    link_enabled(&home, "owner");
    assert_eq!(preset_links(&home).len(), preset_names().len()); // the owner gets every preset
    wr(&home.join(".config/dotfiles/settings"), "PRESETS=\"workflow\"\n");
    link_enabled(&home, "owner");
    assert_eq!(preset_links(&home), ["preset-workflow.md"]);
    // "all" and "none" are words too
    wr(&home.join(".config/dotfiles/settings"), "PRESETS=\"none\"\n");
    link_enabled(&home, "owner");
    assert!(preset_links(&home).is_empty());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn foreign_preset_file_untouched() {
    let home = temp_home("presets-foreign");
    let mine = home.join(".claude/rules/preset-clock.md");
    wr(&mine, "my own rule\n");
    wr(&home.join(".config/dotfiles/settings"), "PRESETS=\"workflow\"\n");
    link_enabled(&home, "owner"); // clock off
    assert_eq!(fs::read_to_string(&mine).unwrap(), "my own rule\n");
    assert!(!mine.symlink_metadata().unwrap().file_type().is_symlink());
    wr(&home.join(".config/dotfiles/settings"), "PRESETS=\"all\"\n");
    link_enabled(&home, "owner"); // clock on
    assert_eq!(fs::read_to_string(&mine).unwrap(), "my own rule\n");
    assert!(!mine.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(home.join(".claude/rules").read_dir().unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".bak-")));
    assert_eq!(preset_links(&home).len(), 7); // six links and the regular file
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn unknown_preset_warns_and_skips() {
    let home = temp_home("presets-unknown");
    wr(&home.join(".config/dotfiles/settings"), "PRESETS=\"workflow nope\"\n");
    assert_eq!(presets::enabled_quiet(&home, "shared"), ["workflow"]);
    // the warning shows in a real run
    let fx = px("presets-unknown-run");
    setting_file(&fx, "PRESETS=\"workflow nope\"\n");
    let (code, _, err) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("nope"), "{err}");
    let agents = fs::read_to_string(fx.home.join(".config/opencode/AGENTS.md")).unwrap();
    assert!(agents.contains("workflow line"));
    assert!(!agents.contains("plain line"));
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
    assert!(!fx.layer().exists(), "a preset key must not drift into the private layer");
}

#[test]
fn co_author_switch_follows_the_no_ai_attribution_preset() {
    // the real repo, a temp HOME: built with the preset on, then off
    let home = temp_home("presets-co-author");
    let (h, r) = (home.to_str().unwrap().to_string(), real_repo().to_str().unwrap().to_string());
    let build = |settings: &str| -> Value {
        wr(&home.join(".config/dotfiles/settings"), settings);
        let (code, out, err) = run_bin(AC, &["claude-settings", "build"], "", &[("HOME", &h), ("DOTFILES_REPO", &r)], None);
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
fn opencode_rules_include_presets() {
    let fx = px("presets-opencode");
    wr(&fx.home.join(".config/dotfiles/owner.md"), "## Private\n\nowner line\n");
    let (code, _, err) = fx.ac(&["opencode", "--profile", "owner"]);
    assert_eq!(code, 0, "{err}");
    let text = fs::read_to_string(fx.home.join(".config/opencode/AGENTS.md")).unwrap();
    let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s} missing in\n{text}"));
    assert!(at("core line") < at("workflow line"));
    assert!(at("plain line") < at("owner line"));
    assert!(at("owner line") < at("# Reply style: ELI5"));
}

#[test]
fn cursor_rule_includes_presets() {
    let fx = px("presets-cursor");
    let (code, _, err) = fx.ac(&["cursor", "--profile", "shared"]);
    assert_eq!(code, 0, "{err}");
    let text = fs::read_to_string(fx.home.join(".cursor/rules/claude-global.mdc")).unwrap();
    assert!(text.contains("core line") && text.contains("workflow line") && text.contains("plain line"), "{text}");
    let (_, out, _) = fx.ac(&["user-rules", "--profile", "shared", "--dry-run"]);
    assert!(out.contains("workflow line"), "{out}");
}

#[test]
fn output_style_from_built_settings() {
    let fx = px("presets-style");
    setting_file(&fx, "PRESETS=\"workflow\"\n");
    // the repo base has no style; the built file does
    wr(&fx.live(), r#"{"outputStyle": "ELI5"}"#);
    assert_eq!(fx.ac(&["opencode", "--profile", "shared"]).0, 0);
    let text = fs::read_to_string(fx.home.join(".config/opencode/AGENTS.md")).unwrap();
    assert!(text.contains("# Reply style: ELI5") && text.contains("Explain it simply."), "{text}");
    // and none when the built file says default
    wr(&fx.live(), r#"{"outputStyle": "default"}"#);
    assert_eq!(fx.ac(&["opencode", "--profile", "shared"]).0, 0);
    assert!(!fs::read_to_string(fx.home.join(".config/opencode/AGENTS.md")).unwrap().contains("Reply style"));
}

#[test]
fn theme_plain_skips_opencode_theme() {
    let fx = px("presets-theme");
    setting_file(&fx, "THEME=\"plain\"\n");
    let (code, out, err) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(code, 0, "{err}");
    let dir = fx.home.join(".config/opencode");
    assert!(!dir.join("themes/copper.json").exists());
    assert!(!dir.join("tui.json").exists());
    assert!(!format!("{out}{err}").contains("uses your Opencode theme"), "{out}{err}");
    // copper still writes it
    setting_file(&fx, "THEME=\"copper\"\n");
    fx.ac(&["opencode", "--profile", "shared"]);
    assert!(dir.join("themes/copper.json").is_file());
}

#[test]
fn profile_read_only() {
    let home = temp_home("presets-profile");
    wr(&home.join(".config/dotfiles/owner.md"), "x\n");
    assert_eq!(presets::profile(&home), "owner");
    assert!(!home.join(".config/dotfiles/settings").exists(), "profile() must not write the settings file");
    wr(&home.join(".config/dotfiles/settings"), "PROFILE=\"shared\"\n");
    assert_eq!(presets::profile(&home), "shared");
    assert_eq!(fs::read_to_string(home.join(".config/dotfiles/settings")).unwrap(), "PROFILE=\"shared\"\n");
    let _ = fs::remove_dir_all(&home);
}
