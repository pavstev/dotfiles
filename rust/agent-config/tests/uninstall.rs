//! `dots uninstall`: it takes back the links and generated files and the entries merged into the
//! user's own config, and leaves everything that is the user's. A temp HOME and a fake repo hold
//! it all; nothing reads the real home of the machine.
#![cfg(unix)]
mod common;

use common::{run_bin, temp_home};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

const DOTS: &str = env!("CARGO_BIN_EXE_dots");
const MARK: &str = "<!-- generated-by: dotfiles -->\n";

fn wr(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn json_at(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

fn dots(home: &Path, repo: &Path, args: &[&str]) -> (i32, String, String) {
    run_bin(DOTS, args, "", &[("HOME", home.to_str().unwrap()), ("DOTFILES_REPO", repo.to_str().unwrap())], None)
}

/// A machine after `dots`: links into the repo, generated files, merged entries, and some files of the user's.
fn fixture(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let tmp = temp_home(tag);
    let (home, repo) = (tmp.join("home"), tmp.join("repo"));
    // a link from home/ and the Claude folders
    wr(&repo.join("home/.config/gh/config.yml"), "git_protocol: ssh\n");
    fs::create_dir_all(home.join(".config/gh")).unwrap();
    symlink(repo.join("home/.config/gh/config.yml"), home.join(".config/gh/config.yml")).unwrap();
    wr(&repo.join("claude/agents/a.md"), "---\nname: a\n---\nA.\n");
    wr(&repo.join("claude/skills/ps_x/SKILL.md"), "---\nname: ps:x\n---\nX.\n");
    fs::create_dir_all(home.join(".claude/skills")).unwrap();
    symlink(repo.join("claude/agents"), home.join(".claude/agents")).unwrap();
    symlink(repo.join("claude/skills/ps_x"), home.join(".claude/skills/ps_x")).unwrap();
    wr(&repo.join("claude/presets/clock/rules.md"), "# Clock\n");
    fs::create_dir_all(home.join(".claude/rules")).unwrap();
    symlink(repo.join("claude/presets/clock/rules.md"), home.join(".claude/rules/preset-clock.md")).unwrap();
    wr(&home.join(".claude/settings.json"), "{\"mine\": true}\n");
    // Cursor
    wr(&repo.join("cursor/mcp.json"), r#"{"mcpServers": {"docs": {"url": "https://example.test/mcp"}}}"#);
    wr(&home.join(".cursor/rules/claude-global.mdc"), &format!("{MARK}rules\n"));
    wr(&home.join(".cursor/rules/mine.mdc"), "my rule\n");
    wr(&home.join(".cursor/agents/a.md"), &format!("{MARK}agent\n"));
    wr(&home.join(".cursor/agents/own.md"), "my agent\n");
    wr(
        &home.join(".cursor/hooks.json"),
        &json!({"version": 1, "hooks": {
            "preToolUse": [{"command": "mine"}, {"command": "test -r x; sh x --hook guard_cursor tool"}],
            "beforeSubmitPrompt": [{"command": "sh x --hook guard_cursor prompt"}]}})
        .to_string(),
    );
    wr(
        &home.join(".cursor/mcp.json"),
        &json!({"mcpServers": {"docs": {"url": "https://example.test/mcp"}, "mine": {"url": "https://mine.test"}}})
            .to_string(),
    );
    wr(
        &home.join(".cursor/cli-config.json"),
        &json!({"permissions": {"allow": ["Shell(ls)", "Shell(mine)"], "deny": []}}).to_string(),
    );
    wr(&home.join(".cursor/.dotfiles-managed.json"), &json!({"allow": ["Shell(ls)"], "deny": []}).to_string());
    // Opencode
    let oc = home.join(".config/opencode");
    wr(&oc.join("AGENTS.md"), &format!("{MARK}rules\n"));
    wr(&oc.join("agents/a.md"), &format!("{MARK}agent\n"));
    wr(&oc.join("tui.json"), r#"{"theme": "copper", "other": 1}"#);
    wr(&oc.join("opencode.jsonc"), &json!({"permission": {"bash": {"rm *": "deny", "mine *": "allow"}}, "mcp": {"docs": {"type": "remote", "url": "https://example.test/mcp", "enabled": true}}}).to_string());
    wr(&oc.join(".dotfiles-managed.json"), &json!({"bash": {"rm *": "deny"}}).to_string());
    (tmp, home, repo)
}

#[test]
fn removes_ours_and_keeps_the_users() {
    let (tmp, home, repo) = fixture("uninstall-all");
    let (code, _out, err) = dots(&home, &repo, &["uninstall", "-y"]);
    assert_eq!(code, 0, "stderr: {err}");
    // links and generated files are gone
    for gone in [
        ".config/gh/config.yml",
        ".claude/agents",
        ".claude/skills/ps_x",
        ".claude/rules/preset-clock.md",
        ".cursor/rules/claude-global.mdc",
        ".cursor/agents/a.md",
        ".cursor/.dotfiles-managed.json",
        ".config/opencode/AGENTS.md",
        ".config/opencode/agents/a.md",
        ".config/opencode/.dotfiles-managed.json",
    ] {
        assert!(!home.join(gone).exists() && home.join(gone).symlink_metadata().is_err(), "{gone} should be gone");
    }
    // what is the user's stays
    for kept in [".claude/settings.json", ".cursor/rules/mine.mdc", ".cursor/agents/own.md"] {
        assert!(home.join(kept).is_file(), "{kept} should stay");
    }
    // merged entries: ours out, the user's in
    let hooks = json_at(&home.join(".cursor/hooks.json"));
    assert_eq!(hooks["hooks"]["preToolUse"], json!([{"command": "mine"}]));
    assert!(hooks["hooks"].get("beforeSubmitPrompt").is_none());
    assert_eq!(json_at(&home.join(".cursor/mcp.json"))["mcpServers"], json!({"mine": {"url": "https://mine.test"}}));
    assert_eq!(json_at(&home.join(".cursor/cli-config.json"))["permissions"]["allow"], json!(["Shell(mine)"]));
    assert_eq!(
        json_at(&home.join(".config/opencode/opencode.jsonc"))["permission"],
        json!({"bash": {"mine *": "allow"}})
    );
    assert!(json_at(&home.join(".config/opencode/opencode.jsonc")).get("mcp").is_none());
    assert_eq!(json_at(&home.join(".config/opencode/tui.json")), json!({"other": 1}));
    // the repo itself stays, and a second run changes nothing and still succeeds
    assert!(repo.join("claude/agents/a.md").is_file());
    let (code, _, err) = dots(&home, &repo, &["uninstall", "-y"]);
    assert_eq!(code, 0, "second run, stderr: {err}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn restores_the_file_a_link_replaced() {
    let (tmp, home, repo) = fixture("uninstall-bak");
    let dst = home.join(".config/gh/config.yml");
    fs::remove_file(&dst).unwrap();
    wr(&home.join(".config/gh/config.yml.bak-20260101-000000"), "mine\n");
    symlink(repo.join("home/.config/gh/config.yml"), &dst).unwrap();
    assert_eq!(dots(&home, &repo, &["uninstall", "-y"]).0, 0);
    assert_eq!(fs::read_to_string(&dst).unwrap(), "mine\n");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn asks_first_and_rejects_bad_arguments() {
    let (tmp, home, repo) = fixture("uninstall-args");
    // no terminal and no -y: it must not remove anything
    let (code, _, err) = dots(&home, &repo, &["uninstall"]);
    assert_eq!(code, 2);
    assert!(err.contains("-y"), "stderr: {err}");
    assert!(home.join(".claude/agents").symlink_metadata().is_ok());
    let (code, _, err) = dots(&home, &repo, &["uninstall", "--now"]);
    assert_eq!(code, 2);
    assert!(err.contains("usage: dots uninstall"), "stderr: {err}");
    let _ = fs::remove_dir_all(tmp);
}
