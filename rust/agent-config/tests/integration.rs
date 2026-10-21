mod common;

use std::fs;
use std::process::Command;

/// Two marks of the Cursor agents note: the path rule and one Cursor agent name.
const NOTE_WORDS: [&str; 2] = ["~/.claude/agents/<name>.md", "generalPurpose"];

#[test]
fn cursor_smoke() {
    let tmp = common::temp_home("cursor");
    let home = tmp.join("home");
    let repo = tmp.join("repo");
    fs::create_dir_all(home.join(".cursor")).unwrap();
    fs::create_dir_all(repo.join("claude")).unwrap();
    fs::create_dir_all(repo.join("claude/hooks")).unwrap();
    fs::write(repo.join("claude/CLAUDE.md"), "# H\n").unwrap();
    fs::write(repo.join("claude/settings.json"), "{}\n").unwrap();
    fs::write(repo.join("claude/hooks/rust-bin"), "# g\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_agent-config"))
        .args(["cursor", "--profile", "shared"])
        .env("HOME", &home)
        .env("DOTFILES_REPO", &repo)
        .env("NO_COLOR", "1")
        .output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let rule = fs::read_to_string(home.join(".cursor/rules/claude-global.mdc")).unwrap();
    for w in NOTE_WORDS {
        assert!(rule.contains(w), "rule lacks {w}: {rule}");
    }
    let _ = fs::remove_dir_all(&tmp);
}
#[test]
fn opencode_smoke() {
    let tmp = common::temp_home("opencode");
    let home = tmp.join("home");
    let repo = tmp.join("repo");
    fs::create_dir_all(repo.join("claude")).unwrap();
    fs::create_dir_all(repo.join("cursor")).unwrap();
    fs::write(repo.join("claude/CLAUDE.md"), "# H\n").unwrap();
    fs::write(repo.join("claude/settings.json"), "{}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_agent-config"))
        .args(["opencode","--profile","shared"])
        .env("HOME",&home).env("DOTFILES_REPO",&repo).env("NO_COLOR","1")
        .output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(home.join(".config/opencode/tui.json").exists());
    let _ = fs::remove_dir_all(&tmp);
}
#[test]
fn user_rules_print() {
    let tmp = common::temp_home("rules");
    let home = tmp.join("home");
    let repo = tmp.join("repo");
    fs::create_dir_all(repo.join("claude")).unwrap();
    fs::write(repo.join("claude/CLAUDE.md"), "# H\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_agent-config"))
        .args(["user-rules","--profile","shared"])
        .env("HOME",&home).env("DOTFILES_REPO",&repo).env("NO_COLOR","1")
        .output().unwrap();
    assert!(out.status.success());
    assert!(!out.stdout.is_empty());
    let text = String::from_utf8_lossy(&out.stdout);
    for w in NOTE_WORDS {
        assert!(text.contains(w), "user rules lack {w}");
    }
    let _ = fs::remove_dir_all(&tmp);
}
