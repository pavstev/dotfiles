//! Command-level tests of `agent-config claude-settings`, run as a process in a temp HOME with a
//! fake repo (`claude/settings.json`). Cursor and Opencode config moved to the ps plugin
//! (pavstev/agent-kit) with their tests.

#![allow(clippy::unwrap_used, reason = "a test fails by panicking")]
mod common;

#[cfg(unix)]
use common::{link, mode_of};
use common::{rd, rj, run_bin, temp_home, wj, wr};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const AC: &str = env!("CARGO_BIN_EXE_agent-config");

fn is_link(p: &Path) -> bool {
    p.symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// A temp HOME and a fake repo with `claude/settings.json`.
struct Fx {
    tmp: PathBuf,
    home: PathBuf,
    claude: PathBuf,
}

impl Fx {
    fn new(tag: &str) -> Self {
        let tmp = temp_home(tag);
        let home = tmp.join("home");
        fs::create_dir_all(&home).unwrap();
        Self {
            claude: tmp.join("repo").join("claude"),
            tmp,
            home,
        }
    }
    fn settings(&self, v: &Value) {
        wj(&self.claude.join("settings.json"), v);
    }
    /// Run `agent-config <args>` with verbose output.
    fn ac(&self, args: &[&str]) -> (i32, String, String) {
        let home = self.home.to_str().unwrap();
        let repo = self.tmp.join("repo");
        let repo = repo.to_str().unwrap();
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
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

// --- claude-settings: repo base + private layer -------------------------------

fn base() -> Value {
    json!({"model": "opus", "permissions": {"allow": ["Bash(git status:*)"], "deny": []}, "hooks": {"Stop": []}, "theme": "dark"})
}

struct Cs {
    fx: Fx,
    live: PathBuf,
    layer: PathBuf,
    last: PathBuf,
}

impl Cs {
    fn new(tag: &str) -> Self {
        let fx = Fx::new(tag);
        fx.settings(&base());
        let (live, layer, last) = (
            fx.home.join(".claude").join("settings.json"),
            fx.home.join(".config/dotfiles/claude-settings.json"),
            fx.home.join(".config/dotfiles/claude-settings.last.json"),
        );
        Self {
            fx,
            live,
            layer,
            last,
        }
    }
    fn build(&self) -> (i32, String, String) {
        self.fx.ac(&["claude-settings", "build"])
    }
}

#[cfg(unix)]
#[test]
fn test_first_build_writes_a_real_file() {
    let cs = Cs::new("ac-cs-first");
    let (code, out, _) = cs.build();
    assert_eq!(code, 0);
    assert!(!is_link(&cs.live));
    assert_eq!(rj(&cs.live), base());
    assert!(out.contains("built"), "{out}");
    assert_eq!(mode_of(&cs.last), 0o600);
}

#[test]
fn test_layer_goes_on_top() {
    let cs = Cs::new("ac-cs-layer");
    wj(
        &cs.layer,
        &json!({"model": "sonnet", "permissions": {"allow": ["Bash(ls:*)"]}, "theme": null}),
    );
    cs.build();
    let got = rj(&cs.live);
    assert_eq!(got["model"], "sonnet");
    assert_eq!(
        got["permissions"]["allow"],
        json!(["Bash(git status:*)", "Bash(ls:*)"])
    );
    assert!(got.get("theme").is_none());
}

#[test]
fn test_second_run_changes_nothing() {
    let cs = Cs::new("ac-cs-second");
    cs.build();
    let before = rd(&cs.live);
    let (code, out, err) = cs.build();
    assert_eq!((code, rd(&cs.live)), (0, before));
    assert!(!format!("{out}{err}").contains("kept"));
    assert!(!cs.layer.exists());
}

#[cfg(unix)]
#[test]
fn test_claude_change_is_kept_in_the_layer() {
    let cs = Cs::new("ac-cs-kept");
    cs.build();
    let mut live = rj(&cs.live);
    live["skipDangerousModePermissionPrompt"] = json!(true);
    live["permissions"]["allow"]
        .as_array_mut()
        .unwrap()
        .push(json!("WebFetch"));
    live["model"] = json!("fable");
    live.as_object_mut().unwrap().remove("theme");
    fs::write(&cs.live, serde_json::to_string_pretty(&live).unwrap()).unwrap(); // Claude's own layout
    let (code, out, _) = cs.build();
    assert_eq!(code, 0);
    assert_eq!(
        rj(&cs.layer),
        json!({"skipDangerousModePermissionPrompt": true, "model": "fable", "permissions": {"allow": ["WebFetch"]}, "theme": null})
    );
    assert_eq!(mode_of(&cs.layer), 0o600);
    assert!(
        out.contains("kept in your private layer: permissions.allow (+1 item(s))"),
        "{out}"
    );
    assert!(!out.contains("WebFetch"), "{out}"); // values never printed
    let got = rj(&cs.live);
    assert_eq!(got["skipDangerousModePermissionPrompt"], true);
    assert!(got.get("theme").is_none());
    let (_, out2, _) = cs.build();
    assert!(!out2.contains("kept"), "{out2}");
}

#[test]
fn test_layout_only_rewrite_is_no_drift() {
    let cs = Cs::new("ac-cs-layout");
    cs.build();
    // Claude's layout: 4-space indent, keys sorted
    let sorted: std::collections::BTreeMap<String, Value> = rj(&cs.live)
        .as_object()
        .unwrap()
        .clone()
        .into_iter()
        .collect();
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    serde::Serialize::serialize(
        &sorted,
        &mut serde_json::Serializer::with_formatter(&mut buf, fmt),
    )
    .unwrap();
    fs::write(&cs.live, buf).unwrap();
    let (_, out, _) = cs.build();
    assert!(!out.contains("kept"), "{out}");
    assert!(!cs.layer.exists());
}

#[test]
fn test_upstream_change_flows_through() {
    let cs = Cs::new("ac-cs-upstream");
    cs.build();
    let mut b = base();
    b["model"] = json!("haiku");
    cs.fx.settings(&b);
    cs.build();
    assert_eq!(rj(&cs.live)["model"], "haiku");
    assert!(!cs.layer.exists());
}

#[cfg(unix)]
#[test]
fn test_link_is_replaced_without_drift() {
    let cs = Cs::new("ac-cs-link");
    link(&cs.fx.claude.join("settings.json"), &cs.live);
    let (_, out, _) = cs.build();
    assert!(!is_link(&cs.live));
    assert!(out.contains("was a link into the repo"), "{out}");
    assert_eq!(rj(&cs.fx.claude.join("settings.json")), base()); // repo file untouched
}

#[test]
fn test_bad_json_touches_nothing() {
    let cs = Cs::new("ac-cs-bad");
    cs.build();
    let before = rd(&cs.live);
    wr(&cs.layer, "{nope");
    let (code, _, err) = cs.build();
    assert_eq!((code, rd(&cs.live)), (1, before));
    assert!(err.contains("not valid JSON"), "{err}");
    fs::remove_file(&cs.layer).unwrap();
    fs::write(&cs.live, "{half").unwrap();
    let (code, _, _) = cs.build();
    assert_eq!(code, 1);
    assert_eq!(rd(&cs.live), "{half");
}

#[test]
fn test_removed_union_item_warns() {
    let cs = Cs::new("ac-cs-removed");
    cs.build();
    let mut live = rj(&cs.live);
    live["permissions"]["allow"] = json!([]);
    fs::write(&cs.live, live.to_string()).unwrap();
    let (_, _, err) = cs.build();
    assert!(
        err.contains("Claude removed 1 item(s) from permissions.allow"),
        "{err}"
    );
    assert_eq!(
        rj(&cs.live)["permissions"]["allow"],
        json!(["Bash(git status:*)"])
    );
}

#[test]
fn test_promote_moves_a_key_to_the_repo() {
    let cs = Cs::new("ac-cs-promote");
    wj(
        &cs.layer,
        &json!({"permissions": {"allow": ["WebFetch"]}, "statusLine": {"type": "command"}}),
    );
    cs.build();
    let (code, out, _) = cs
        .fx
        .ac(&["claude-settings", "promote", "permissions.allow"]);
    assert_eq!(code, 0);
    assert_eq!(
        rj(&cs.fx.claude.join("settings.json"))["permissions"]["allow"],
        json!(["Bash(git status:*)", "WebFetch"])
    );
    assert_eq!(rj(&cs.layer), json!({"statusLine": {"type": "command"}}));
    assert!(out.contains("Commit it to share it"), "{out}");
    assert_eq!(
        rj(&cs.live)["permissions"]["allow"],
        json!(["Bash(git status:*)", "WebFetch"])
    );
    let (code, _, _) = cs.fx.ac(&["claude-settings", "promote", "nope.key"]);
    assert_eq!(code, 1);
}

#[test]
fn test_claude_settings_dry_run_writes_nothing() {
    let cs = Cs::new("ac-cs-dry");
    let (_, out, _) = cs.fx.ac(&["claude-settings", "build", "--dry-run"]);
    assert!(!cs.live.exists());
    assert!(out.contains("would write"), "{out}");
}
