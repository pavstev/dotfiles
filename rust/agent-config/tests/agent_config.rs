//! Command-level tests ported from `scripts/test_agent_config.py`: `agent-config cursor`, `opencode`,
//! `user-rules` and `claude-settings` run as a process in a temp HOME with a fake repo (`claude/`,
//! `cursor/`). The helpers and the single parts, called as functions, are unit tests in
//! `src/agent_config_tests.rs`. `tests/PORTED.md` maps each Python test to its Rust twin.
mod common;

use common::{run_bin, temp_home};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

type R = (i32, String, String);

const AC: &str = env!("CARGO_BIN_EXE_agent-config");
const CLAUDE_MD: &str = "# Global\n\nintro line\n\n## Environment\n\n- one\n- two\n\n## Working rules\n\n1. a\n2. b\n";
const OWNER_MD: &str = "## Off-limits\n\n- never do the secret thing\n";
const OWNER_REL: &str = ".config/dotfiles/owner.md";
const SETTINGS_REL: &str = ".config/dotfiles/settings";
const MARK: &str = "generated-by: dotfiles";
const MANAGED: &str = ".dotfiles-managed.json";
const USER_RULES_MARK: &str = ".config/dotfiles/cursor-user-rules.sha256";

fn wr(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}
fn wj(p: &Path, v: Value) {
    wr(p, &v.to_string());
}
fn rd(p: &Path) -> String {
    fs::read_to_string(p).unwrap()
}
fn rj(p: &Path) -> Value {
    serde_json::from_str(&rd(p)).unwrap()
}
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}
#[cfg(unix)]
fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}
#[cfg(unix)]
fn set_mode(p: &Path, m: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
}
#[cfg(unix)]
fn link(target: &Path, at: &Path) {
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, at).unwrap();
}
fn is_link(p: &Path) -> bool {
    p.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

/// A temp HOME and a fake repo with `claude/CLAUDE.md` and a fake Cursor guard.
struct Fx {
    tmp: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    cursor_src: PathBuf,
    repo: PathBuf,
}

impl Fx {
    fn new(tag: &str) -> Fx {
        let tmp = temp_home(tag);
        let home = tmp.join("home");
        let repo = tmp.join("repo");
        fs::create_dir_all(&home).unwrap();
        let fx = Fx { claude: repo.join("claude"), cursor_src: repo.join("cursor"), tmp, home, repo };
        wr(&fx.claude.join("CLAUDE.md"), CLAUDE_MD);
        wr(&fx.claude.join("hooks").join("rust-bin"), "# fake launcher\n");
        fx
    }
    fn cdir(&self) -> PathBuf {
        self.home.join(".cursor")
    }
    fn odir(&self) -> PathBuf {
        self.home.join(".config").join("opencode")
    }
    fn overlay(&self, text: &str) {
        wr(&self.home.join(OWNER_REL), text);
    }
    fn setting_file(&self, text: &str) {
        wr(&self.home.join(SETTINGS_REL), text);
    }
    fn settings(&self, v: Value) {
        wj(&self.claude.join("settings.json"), v);
    }
    fn perms(&self, allow: &[&str], deny: &[&str]) {
        self.settings(json!({"permissions": {"allow": allow, "deny": deny}}));
    }
    fn agent(&self, fname: &str, text: &str) {
        wr(&self.claude.join("agents").join(fname), text);
    }
    fn mcp_source(&self, servers: Value) {
        wj(&self.cursor_src.join("mcp.json"), json!({"mcpServers": servers}));
    }
    /// Run `agent-config <args>`; the output is verbose, as the Python tests ran it.
    fn ac(&self, args: &[&str]) -> R {
        self.ac_env(args, &[])
    }
    fn ac_env(&self, args: &[&str], extra: &[(&str, &str)]) -> R {
        let home = self.home.to_str().unwrap();
        let repo = self.repo.to_str().unwrap();
        let mut env = vec![("HOME", home), ("DOTFILES_REPO", repo), ("DOTFILES_VERBOSE", "1")];
        env.extend_from_slice(extra);
        run_bin(AC, args, "", &env, None)
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

// --- cursor -------------------------------------------------------------------

#[test]
fn test_full_run_creates_everything_and_never_writes_claude_dir() {
    let fx = Fx::new("ac-cursor-full");
    fx.perms(&["Bash(ls:*)"], &[]);
    fx.mcp_source(json!({"docs": {"url": "u"}}));
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    let (rc, _out, err) = fx.ac(&["cursor", "--profile", "shared"]);
    assert_eq!(rc, 0);
    assert_eq!(err, "");
    for rel in ["rules/claude-global.mdc", "agents/a.md", "hooks.json", "cli-config.json", MANAGED] {
        assert!(fx.cdir().join(rel).exists(), "{rel}");
    }
    assert!(!fx.cdir().join("mcp.json").exists()); // MCP is opt-in on every profile
    assert!(!fx.home.join(".claude").exists());
    assert!(!rd(&fx.cdir().join("rules").join("claude-global.mdc")).contains("Off-limits"));
}

#[test]
fn test_owner_profile_gets_overlay_but_mcp_stays_opt_in() {
    let fx = Fx::new("ac-cursor-owner");
    fx.overlay(OWNER_MD);
    fx.mcp_source(json!({"docs": {"url": "u"}}));
    fx.ac(&["cursor", "--profile", "owner"]);
    assert!(!fx.cdir().join("mcp.json").exists());
    assert!(rd(&fx.cdir().join("rules").join("claude-global.mdc")).contains("## Off-limits"));
    fx.setting_file("CURSOR_MCP=\"on\"\n");
    fx.ac(&["cursor", "--profile", "owner"]);
    assert!(fx.cdir().join("mcp.json").exists());
}

#[cfg(unix)]
#[test]
fn test_owner_rule_file_is_private_and_names_both_sources() {
    let fx = Fx::new("ac-cursor-private");
    fx.overlay(OWNER_MD);
    fx.ac(&["cursor", "--profile", "owner"]);
    let rule = fx.cdir().join("rules").join("claude-global.mdc");
    assert_eq!(mode_of(&rule), 0o600);
    assert!(rd(&rule).contains(&format!("and ~/{OWNER_REL}. Edit those files")));
}

#[cfg(unix)]
#[test]
fn test_owner_rule_file_turns_private_when_the_text_is_unchanged() {
    let fx = Fx::new("ac-cursor-turns-private");
    fx.overlay(OWNER_MD);
    fx.ac(&["cursor", "--profile", "owner"]);
    let rule = fx.cdir().join("rules").join("claude-global.mdc");
    set_mode(&rule, 0o644);
    fx.ac(&["cursor", "--profile", "owner"]);
    assert_eq!(mode_of(&rule), 0o600);
}

#[cfg(unix)]
#[test]
fn test_owner_without_overlay_warns_and_names_one_source() {
    let fx = Fx::new("ac-cursor-no-overlay");
    fx.mcp_source(json!({"docs": {"url": "u"}}));
    let (rc, _, err) = fx.ac(&["cursor", "--profile", "owner"]);
    assert_eq!(rc, 0);
    assert!(err.contains("no owner-only rules"), "{err}");
    let rule = fx.cdir().join("rules").join("claude-global.mdc");
    assert!(rd(&rule).contains("from claude/CLAUDE.md. Edit that file"));
    assert_eq!(mode_of(&rule), 0o644);
}

#[test]
fn test_second_run_is_all_skips() {
    let fx = Fx::new("ac-cursor-second");
    fx.perms(&["Bash(ls:*)"], &[]);
    fx.ac(&["cursor"]);
    let (rc, out, err) = fx.ac(&["cursor"]);
    assert_eq!(rc, 0);
    assert!(!out.contains("wrote"), "{out}");
    assert!(!out.contains("updated"), "{out}");
    assert_eq!(err, "");
}

#[test]
fn test_dry_run_leaves_home_empty() {
    let fx = Fx::new("ac-cursor-dry");
    let (rc, out, _) = fx.ac(&["cursor", "--dry-run"]);
    assert_eq!(rc, 0);
    assert!(names(&fx.home).is_empty(), "{:?}", names(&fx.home));
    assert!(out.contains("would"), "{out}");
}

// --- opencode agents ----------------------------------------------------------

#[test]
fn test_opencode_agents_converts_frontmatter() {
    let fx = Fx::new("ac-oc-agents-convert");
    fx.agent("rev.md", "---\nname: reviewer\ndescription: reviews\ntools: Read, Grep, Bash, Write\nmodel: sonnet\neffort: high\n---\n\nBody\n");
    let (rc, _, _) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(rc, 0);
    let out = rd(&fx.odir().join("agents").join("reviewer.md"));
    assert!(out.starts_with("---\ndescription: reviews\nmode: subagent\npermission:\n  skill: allow\n---"), "{out}");
    assert!(out.contains(MARK));
    assert!(!out.split("---").nth(1).unwrap().contains("model"));
    assert!(!out.contains("effort"));
    assert!(!out.contains("edit: deny"));
    assert!(out.ends_with("Body\n"));
}

#[test]
fn test_readonly_agent_gets_edit_deny() {
    let fx = Fx::new("ac-oc-agents-readonly");
    fx.agent("cr.md", "---\nname: code-reviewer\ndescription: reviews\ntools: Read, Grep, Glob, Bash\ndisallowedTools: Write, Edit, NotebookEdit, Agent\n---\n\nBody\n");
    let (rc, _, _) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(rc, 0);
    assert!(rd(&fx.odir().join("agents").join("code-reviewer.md")).contains("permission:\n  edit: deny\n  skill: allow"));
}

#[test]
fn test_write_capable_agent_does_not_deny_edit() {
    let fx = Fx::new("ac-oc-agents-write");
    fx.agent("ti.md", "---\nname: task-implementer\ndescription: builds\ntools: Read, Edit, Write, Bash, Grep, Glob\n---\n\nBody\n");
    let (rc, _, _) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(rc, 0);
    assert!(!rd(&fx.odir().join("agents").join("task-implementer.md")).contains("edit: deny"));
}

#[test]
fn test_name_from_file_stem_and_readme_skipped() {
    let fx = Fx::new("ac-oc-agents-stem");
    fx.agent("stem-name.md", "no frontmatter\n");
    fx.agent("README.md", "# readme\n");
    let (rc, _, _) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(rc, 0);
    assert_eq!(names(&fx.odir().join("agents")), ["stem-name.md"]);
}

#[test]
fn test_opencode_agents_stale_generated_agent_removed_user_agent_kept() {
    let fx = Fx::new("ac-oc-agents-stale");
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    assert_eq!(fx.ac(&["opencode", "--profile", "shared"]).0, 0);
    let mine = fx.odir().join("agents").join("mine.md");
    wr(&mine, "my agent\n");
    fs::remove_file(fx.claude.join("agents").join("a.md")).unwrap();
    assert_eq!(fx.ac(&["opencode", "--profile", "shared"]).0, 0);
    assert!(!fx.odir().join("agents").join("a.md").exists());
    assert_eq!(rd(&mine), "my agent\n");
}

#[test]
fn test_opencode_agents_user_agent_with_same_name_is_not_overwritten() {
    let fx = Fx::new("ac-oc-agents-same-name");
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    wr(&fx.odir().join("agents").join("a.md"), "mine\n");
    let (_, _, err) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(rd(&fx.odir().join("agents").join("a.md")), "mine\n");
    assert!(err.contains("not ours"), "{err}");
}

#[test]
fn test_opencode_agents_no_agents_dir_is_fine() {
    let fx = Fx::new("ac-oc-agents-none");
    let (rc, _, _) = fx.ac(&["opencode", "--profile", "shared"]);
    assert_eq!(rc, 0);
    assert!(!fx.odir().join("agents").exists());
}

// --- opencode config ----------------------------------------------------------

fn oc_cfg(fx: &Fx) -> PathBuf {
    fx.odir().join("opencode.jsonc")
}
fn run_oc(fx: &Fx) -> R {
    fx.ac(&["opencode", "--profile", "shared"])
}

#[test]
fn test_translated_permissions_added() {
    let fx = Fx::new("ac-oc-cfg-perms");
    fx.perms(&["Bash(git status:*)", "Read(./src/**)", "Edit(./notes.md)"], &["Read(./.env)"]);
    run_oc(&fx);
    let p = rj(&oc_cfg(&fx))["permission"].clone();
    assert_eq!(p["bash"], json!({"git status *": "allow"}));
    assert_eq!(p["read"], json!({"src/**": "allow", "**/.env": "deny"}));
    assert_eq!(p["edit"], json!({"notes.md": "allow"}));
}

#[test]
fn test_skill_and_ask_allowed() {
    let fx = Fx::new("ac-oc-cfg-skill");
    fx.perms(&[], &[]);
    run_oc(&fx);
    let p = rj(&oc_cfg(&fx))["permission"].clone();
    assert_eq!(p["skill"], json!({"*": "allow"}));
    assert_eq!(p["ask"], json!({"*": "allow"}));
}

#[test]
fn test_user_keys_win_ours_added() {
    let fx = Fx::new("ac-oc-cfg-user-keys");
    fx.perms(&["Bash(git status:*)"], &[]);
    wj(&oc_cfg(&fx), json!({"permission": {"bash": {"npm *": "ask"}, "read": {"keep.me": "allow"}}}));
    run_oc(&fx);
    let p = rj(&oc_cfg(&fx))["permission"].clone();
    assert_eq!(p["bash"], json!({"npm *": "ask", "git status *": "allow"}));
    assert_eq!(p["read"], json!({"keep.me": "allow"}));
}

#[test]
fn test_mcp_added_when_opt_in() {
    let fx = Fx::new("ac-oc-cfg-mcp");
    fx.mcp_source(json!({"cloudflare-docs": {"url": "https://docs.mcp.cloudflare.com/mcp"}}));
    fx.setting_file("OPENCODE_MCP=\"on\"\n");
    run_oc(&fx);
    let m = rj(&oc_cfg(&fx))["mcp"].clone();
    assert_eq!(m["cloudflare-docs"], json!({"type": "remote", "url": "https://docs.mcp.cloudflare.com/mcp", "enabled": true}));
}

#[test]
fn test_mcp_user_entry_wins() {
    let fx = Fx::new("ac-oc-cfg-mcp-user");
    fx.mcp_source(json!({"cloudflare-docs": {"url": "ours"}}));
    wj(&oc_cfg(&fx), json!({"mcp": {"cloudflare-docs": {"type": "remote", "url": "users", "enabled": false}}}));
    fx.setting_file("OPENCODE_MCP=\"on\"\n");
    run_oc(&fx);
    assert_eq!(rj(&oc_cfg(&fx))["mcp"]["cloudflare-docs"]["url"], "users");
}

#[test]
fn test_mcp_skipped_when_off() {
    let fx = Fx::new("ac-oc-cfg-mcp-off");
    fx.mcp_source(json!({"cloudflare-docs": {"url": "u"}}));
    fx.setting_file("OPENCODE_MCP=\"off\"\n");
    run_oc(&fx);
    assert!(rj(&oc_cfg(&fx)).get("mcp").is_none());
}

#[test]
fn test_mcp_env_wins_over_setting() {
    let fx = Fx::new("ac-oc-cfg-mcp-env");
    fx.mcp_source(json!({"cloudflare-docs": {"url": "u"}}));
    fx.setting_file("OPENCODE_MCP=\"on\"\n");
    fx.ac_env(&["opencode", "--profile", "shared"], &[("DOTFILES_MCP", "off")]);
    assert!(rj(&oc_cfg(&fx)).get("mcp").is_none());
    fx.ac_env(&["opencode", "--profile", "shared"], &[("DOTFILES_MCP", "1")]);
    assert!(rj(&oc_cfg(&fx))["mcp"].get("cloudflare-docs").is_some());
}

#[test]
fn test_mcp_merges_into_existing_mcp_block() {
    let fx = Fx::new("ac-oc-cfg-mcp-merge");
    fx.mcp_source(json!({"cloudflare-docs": {"url": "ours"}}));
    wj(&oc_cfg(&fx), json!({"mcp": {"existing": {"type": "remote", "url": "x"}}}));
    fx.setting_file("OPENCODE_MCP=\"on\"\n");
    run_oc(&fx);
    let m = rj(&oc_cfg(&fx))["mcp"].clone();
    assert_eq!(m["existing"]["url"], "x");
    assert_eq!(m["cloudflare-docs"]["url"], "ours");
}

#[test]
fn test_opencode_config_invalid_json_left_untouched() {
    let fx = Fx::new("ac-oc-cfg-invalid");
    wr(&oc_cfg(&fx), "{oops");
    let (_, _, err) = run_oc(&fx);
    assert_eq!(rd(&oc_cfg(&fx)), "{oops");
    assert!(err.contains("not valid JSON"), "{err}");
}

#[test]
fn test_opencode_config_dry_run_writes_nothing() {
    let fx = Fx::new("ac-oc-cfg-dry");
    let (rc, out, _) = fx.ac(&["opencode", "--profile", "shared", "--dry-run"]);
    assert_eq!(rc, 0);
    assert!(!oc_cfg(&fx).exists());
    assert!(out.contains("would"), "{out}");
}

// --- opencode managed permissions (the sidecar) -------------------------------

fn oc_side(fx: &Fx) -> PathBuf {
    fx.odir().join(MANAGED)
}

#[test]
fn test_sidecar_holds_the_rules_we_added() {
    let fx = Fx::new("ac-oc-side-holds");
    fx.perms(&["Bash(git status:*)"], &["Read(./.env)"]);
    run_oc(&fx);
    assert_eq!(rj(&oc_side(&fx))["bash"], json!({"git status *": "allow"}));
}

fn bash_keys(fx: &Fx) -> Vec<String> {
    let mut k: Vec<String> = rj(&oc_cfg(fx))["permission"]["bash"].as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[test]
fn test_rule_removed_from_the_repo_goes_away_next_run() {
    let fx = Fx::new("ac-oc-side-removed");
    fx.perms(&["Bash(git status:*)", "Bash(ls:*)"], &[]);
    run_oc(&fx);
    assert_eq!(bash_keys(&fx), ["git status *", "ls *"]);
    fx.perms(&["Bash(git status:*)"], &[]); // ls dropped from claude/settings.json
    run_oc(&fx);
    assert_eq!(bash_keys(&fx), ["git status *"]);
}

#[test]
fn test_a_rule_the_user_edited_is_not_taken_away() {
    let fx = Fx::new("ac-oc-side-edited");
    fx.perms(&["Bash(ls:*)"], &[]);
    run_oc(&fx);
    let mut data = rj(&oc_cfg(&fx));
    data["permission"]["bash"]["ls *"] = json!("ask"); // the user changed our rule
    wj(&oc_cfg(&fx), data);
    fx.perms(&[], &[]); // and the repo dropped it
    run_oc(&fx);
    assert_eq!(rj(&oc_cfg(&fx))["permission"]["bash"], json!({"ls *": "ask"}));
}

#[test]
fn test_a_rule_the_user_added_is_never_touched() {
    let fx = Fx::new("ac-oc-side-user-added");
    fx.perms(&["Bash(git status:*)"], &[]);
    wj(&oc_cfg(&fx), json!({"permission": {"bash": {"npm *": "ask"}}}));
    run_oc(&fx);
    assert_eq!(rj(&oc_cfg(&fx))["permission"]["bash"], json!({"npm *": "ask", "git status *": "allow"}));
    fx.perms(&[], &[]);
    run_oc(&fx);
    assert_eq!(rj(&oc_cfg(&fx))["permission"]["bash"], json!({"npm *": "ask"}));
}

#[test]
fn test_dry_run_leaves_the_sidecar_alone() {
    let fx = Fx::new("ac-oc-side-dry");
    fx.perms(&["Bash(ls:*)"], &[]);
    fx.ac(&["opencode", "--profile", "shared", "--dry-run"]);
    assert!(!oc_side(&fx).exists());
}

#[cfg(unix)]
#[test]
fn test_a_symlinked_config_is_never_claimed() {
    let fx = Fx::new("ac-oc-side-symlink");
    fx.perms(&["Bash(ls:*)"], &[]);
    let target = fx.tmp.join("mine.jsonc");
    wj(&target, json!({"permission": {"bash": {"ls *": "allow"}}}));
    link(&target, &oc_cfg(&fx));
    let (rc, _, err) = run_oc(&fx);
    assert_eq!(rc, 0);
    assert!(err.contains("symlink"), "{err}");
    assert!(!oc_side(&fx).exists());
}

// --- opencode theme -----------------------------------------------------------

/// The key set the Opencode theme loader reads, read out of the shipped themes in opencode 2.0.25.
/// A missing key aborts the whole theme ("Failed to load theme").
const SHIPPED_KEYS: &[&str] = &[
    "primary", "secondary", "accent", "error", "warning", "success", "info",
    "text", "textMuted", "background", "backgroundPanel", "backgroundElement",
    "border", "borderActive", "borderSubtle",
    "diffAdded", "diffRemoved", "diffContext", "diffHunkHeader",
    "diffHighlightAdded", "diffHighlightRemoved",
    "diffAddedBg", "diffRemovedBg", "diffContextBg", "diffLineNumber",
    "diffAddedLineNumberBg", "diffRemovedLineNumberBg",
    "markdownText", "markdownHeading", "markdownLink", "markdownLinkText", "markdownCode",
    "markdownBlockQuote", "markdownEmph", "markdownStrong", "markdownHorizontalRule",
    "markdownListItem", "markdownListEnumeration", "markdownImage", "markdownImageText",
    "markdownCodeBlock",
    "syntaxComment", "syntaxKeyword", "syntaxFunction", "syntaxVariable", "syntaxString",
    "syntaxNumber", "syntaxType", "syntaxOperator", "syntaxPunctuation",
];

fn theme_path(fx: &Fx) -> PathBuf {
    fx.odir().join("themes").join("copper.json")
}
fn tui_path(fx: &Fx) -> PathBuf {
    fx.odir().join("tui.json")
}

#[test]
fn test_theme_file_holds_the_copper_palette() {
    let fx = Fx::new("ac-oc-theme-palette");
    assert_eq!(run_oc(&fx).0, 0);
    let data = rj(&theme_path(&fx));
    assert_eq!(data["theme"]["primary"], "#E0825F");
    assert_eq!(data["theme"]["background"], "#1C1917");
    assert_eq!(data["theme"]["accent"], "#E8BC77");
    let mut keys: Vec<&str> = data["theme"].as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    let mut want = SHIPPED_KEYS.to_vec();
    want.sort();
    assert_eq!(keys, want);
    assert_eq!(data["$schema"], "https://opencode.ai/theme.json");
}

#[test]
fn test_theme_has_every_key_the_opencode_loader_reads() {
    // regression: a missing key makes the TUI say "Failed to load theme" and use none
    let fx = Fx::new("ac-oc-theme-keys");
    run_oc(&fx);
    let theme = rj(&theme_path(&fx))["theme"].clone();
    let keys = theme.as_object().unwrap();
    let extra: Vec<&String> = keys.keys().filter(|k| !SHIPPED_KEYS.contains(&k.as_str())).collect();
    let missing: Vec<&str> = SHIPPED_KEYS.iter().copied().filter(|k| !keys.contains_key(*k)).collect();
    assert!(extra.is_empty(), "{extra:?}");
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn test_every_colour_is_a_hex_string() {
    let fx = Fx::new("ac-oc-theme-hex");
    run_oc(&fx);
    let re = regex::Regex::new(r"^#[0-9A-Fa-f]{6}$").unwrap();
    for (key, value) in rj(&theme_path(&fx))["theme"].as_object().unwrap() {
        assert!(re.is_match(s(value)), "{key}: {value}");
    }
}

#[test]
fn test_tui_names_the_theme_because_the_schema_takes_a_string() {
    let fx = Fx::new("ac-oc-theme-tui");
    run_oc(&fx);
    let data = rj(&tui_path(&fx));
    assert_eq!(data["theme"], "copper");
    assert_eq!(data["$schema"], "https://opencode.ai/tui.json");
    // tui.json has additionalProperties: false, so no marker key can live there
    let mut keys: Vec<&str> = data.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["$schema", "theme"]);
}

#[test]
fn test_user_theme_name_wins_and_is_reported() {
    let fx = Fx::new("ac-oc-theme-user");
    wr(&tui_path(&fx), "{\"theme\": \"tokyonight\"}\n");
    let (rc, out, _) = run_oc(&fx);
    assert_eq!(rc, 0);
    assert_eq!(rj(&tui_path(&fx))["theme"], "tokyonight");
    assert!(out.contains("tokyonight"), "{out}");
    assert!(theme_path(&fx).exists()); // copper is still there to pick
}

#[test]
fn test_user_chosen_theme_is_restored_next_run() {
    let fx = Fx::new("ac-oc-theme-restored");
    run_oc(&fx);
    wr(&tui_path(&fx), "{\"theme\": \"gruvbox\"}\n");
    run_oc(&fx);
    assert_eq!(rj(&tui_path(&fx))["theme"], "gruvbox");
}

#[test]
fn test_other_tui_keys_survive() {
    let fx = Fx::new("ac-oc-theme-other-keys");
    wr(&tui_path(&fx), "{\"scroll_speed\": 3}\n");
    run_oc(&fx);
    let data = rj(&tui_path(&fx));
    assert_eq!(data["scroll_speed"], 3);
    assert_eq!(data["theme"], "copper");
}

#[test]
fn test_rerun_skips_an_unchanged_theme() {
    let fx = Fx::new("ac-oc-theme-rerun");
    run_oc(&fx);
    let before = rd(&theme_path(&fx));
    let (_, out, _) = run_oc(&fx);
    assert_eq!(rd(&theme_path(&fx)), before);
    assert!(out.contains("already set up"), "{out}");
}

#[test]
fn test_invalid_tui_json_is_left_alone() {
    let fx = Fx::new("ac-oc-theme-invalid");
    wr(&tui_path(&fx), "{ not json\n");
    let (_, _, err) = run_oc(&fx);
    assert_eq!(rd(&tui_path(&fx)), "{ not json\n");
    assert!(err.contains("not valid JSON"), "{err}");
}

// --- opencode rules (AGENTS.md) -----------------------------------------------

fn agents_md(fx: &Fx) -> PathBuf {
    fx.odir().join("AGENTS.md")
}

#[test]
fn test_shared_profile_carries_the_rules_and_the_note() {
    let fx = Fx::new("ac-oc-rules-shared");
    let (rc, _, _) = run_oc(&fx);
    assert_eq!(rc, 0);
    let text = rd(&agents_md(&fx));
    assert!(text.contains(MARK));
    assert!(text.contains("AskUserQuestion = `question`"));
    assert!(text.contains(CLAUDE_MD.trim()));
}

#[cfg(unix)]
#[test]
fn test_owner_profile_gets_the_private_overlay_at_0600() {
    let fx = Fx::new("ac-oc-rules-owner");
    fx.overlay("# mine\nkeep this\n");
    let (rc, _, _) = fx.ac(&["opencode", "--profile", "owner"]);
    assert_eq!(rc, 0);
    let text = rd(&agents_md(&fx));
    assert!(text.contains("keep this"));
    assert!(text.contains(MARK));
    assert_eq!(mode_of(&agents_md(&fx)), 0o600);
}

#[test]
fn test_owner_profile_without_the_file_warns() {
    let fx = Fx::new("ac-oc-rules-owner-none");
    let (rc, _, err) = fx.ac(&["opencode", "--profile", "owner"]);
    assert_eq!(rc, 0);
    assert!(err.contains("dots settings"), "{err}");
}

#[test]
fn test_output_style_is_appended() {
    let fx = Fx::new("ac-oc-rules-style");
    wr(&fx.claude.join("output-styles").join("warm.md"), "---\nname: warm\n---\nShort lines.\n");
    fx.settings(json!({"outputStyle": "warm"}));
    let (rc, _, _) = run_oc(&fx);
    assert_eq!(rc, 0);
    let text = rd(&agents_md(&fx));
    assert!(text.contains("# Reply style: warm"));
    assert!(text.contains("Short lines."));
}

#[test]
fn test_user_own_agents_md_is_never_overwritten() {
    let fx = Fx::new("ac-oc-rules-user-own");
    wr(&agents_md(&fx), "my own opencode rules\n");
    let (rc, _, err) = run_oc(&fx);
    assert_eq!(rc, 0);
    assert_eq!(rd(&agents_md(&fx)), "my own opencode rules\n");
    assert!(err.contains("owner.md"), "{err}");
}

#[cfg(unix)]
#[test]
fn test_symlink_is_left_alone() {
    let fx = Fx::new("ac-oc-rules-symlink");
    let target = fx.tmp.join("elsewhere.md");
    wr(&target, "mine\n");
    link(&target, &agents_md(&fx));
    let (rc, _, _) = run_oc(&fx);
    assert_eq!(rc, 0);
    assert!(is_link(&agents_md(&fx)));
    assert_eq!(rd(&target), "mine\n");
}

#[test]
fn test_opencode_rules_rerun_is_idempotent() {
    let fx = Fx::new("ac-oc-rules-rerun");
    run_oc(&fx);
    let before = rd(&agents_md(&fx));
    let (_, out, _) = run_oc(&fx);
    assert_eq!(rd(&agents_md(&fx)), before);
    assert!(out.contains("already set up"), "{out}");
}

// --- opencode projects --------------------------------------------------------

fn seed_global_agent(fx: &Fx) {
    wr(&fx.odir().join("agents").join("a.md"), "---\nname: a\n---\nb\n");
}

#[cfg(unix)]
#[test]
fn test_creates_project_opencode_and_agent_links() {
    let fx = Fx::new("ac-oc-proj-links");
    let p1 = temp_home("ac-oc-proj-1").join("proj1");
    let p2 = temp_home("ac-oc-proj-2").join("proj2");
    fs::create_dir_all(&p1).unwrap();
    fs::create_dir_all(&p2).unwrap();
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    seed_global_agent(&fx);
    let projects = format!("{} {}", p1.display(), p2.display());
    let (rc, _, err) = fx.ac(&["opencode", "--profile", "shared", "--projects", &projects]);
    assert_eq!(rc, 0, "{err}");
    let cfg1 = p1.join(".opencode").join("opencode.jsonc");
    assert!(is_link(&cfg1));
    assert_eq!(fs::canonicalize(&cfg1).unwrap(), fs::canonicalize(oc_cfg(&fx)).unwrap());
    let ag = p1.join(".opencode").join("agents").join("a.md");
    assert!(is_link(&ag));
    assert_eq!(fs::canonicalize(&ag).unwrap(), fs::canonicalize(fx.odir().join("agents").join("a.md")).unwrap());
    assert!(is_link(&p2.join(".opencode").join("opencode.jsonc")));
    assert!(is_link(&p2.join(".opencode").join("agents").join("a.md")));
    let _ = fs::remove_dir_all(p1.parent().unwrap());
    let _ = fs::remove_dir_all(p2.parent().unwrap());
}

#[test]
fn test_does_not_clobber_existing_project_config_or_agent() {
    let fx = Fx::new("ac-oc-proj-clobber");
    let p = temp_home("ac-oc-proj-c").join("proj");
    wr(&p.join(".opencode").join("opencode.jsonc"), "mine\n");
    wr(&p.join(".opencode").join("agents").join("a.md"), "project agent\n");
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    seed_global_agent(&fx);
    let (rc, _, _) = fx.ac(&["opencode", "--profile", "shared", "--projects", p.to_str().unwrap()]);
    assert_eq!(rc, 0);
    assert_eq!(rd(&p.join(".opencode").join("opencode.jsonc")), "mine\n");
    assert_eq!(rd(&p.join(".opencode").join("agents").join("a.md")), "project agent\n");
    let _ = fs::remove_dir_all(p.parent().unwrap());
}

#[test]
fn test_a_relative_path_is_skipped_with_a_warning() {
    let fx = Fx::new("ac-oc-proj-relative");
    let (rc, _, err) = fx.ac(&["opencode", "--profile", "shared", "--projects", "code/app"]);
    assert_eq!(rc, 0);
    assert!(err.contains("non-absolute"), "{err}");
}

#[test]
fn test_full_run_creates_agents_and_config_and_never_writes_claude_dir() {
    let fx = Fx::new("ac-oc-full");
    fx.perms(&["Bash(ls:*)"], &[]);
    fx.mcp_source(json!({"docs": {"url": "u"}}));
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    fx.setting_file("OPENCODE_MCP=\"on\"\n");
    let (rc, _, err) = run_oc(&fx);
    assert_eq!(rc, 0);
    assert_eq!(err, "");
    assert!(fx.odir().join("agents").join("a.md").exists());
    assert!(oc_cfg(&fx).exists());
    assert_eq!(rj(&oc_cfg(&fx))["mcp"]["docs"], json!({"type": "remote", "url": "u", "enabled": true}));
    assert!(!fx.home.join(".claude").exists());
}

// --- user-rules ---------------------------------------------------------------

/// A bin folder whose `pbcopy`, `wl-copy`, `xclip`, `xsel` and `clip` write stdin to `clip.out`:
/// whichever the platform picks, the rules land in a file instead of the real clipboard.
#[cfg(unix)]
fn fake_clipboard(fx: &Fx) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let bin = fx.tmp.join("bin");
    let out = fx.tmp.join("clip.out");
    fs::create_dir_all(&bin).unwrap();
    for tool in ["pbcopy", "wl-copy", "xclip", "xsel", "clip"] {
        let f = bin.join(tool);
        fs::write(&f, format!("#!/bin/sh\ncat > '{}'\n", out.display())).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
    }
    (format!("{}:/usr/bin:/bin", bin.display()), out)
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

#[test]
fn test_print_mode_writes_text_to_stdout_only() {
    let fx = Fx::new("ac-ur-print");
    let (rc, out, _) = fx.ac(&["user-rules", "--profile", "shared"]);
    assert_eq!(rc, 0);
    // the Cursor note, then the rules, nothing else (Python: `user_rules_text(home, "shared")`)
    assert_eq!(out, format!("{}{}", agent_config::CURSOR_NOTE, CLAUDE_MD));
    assert!(!fx.home.join(USER_RULES_MARK).exists());
}

#[test]
fn test_default_profile_comes_from_env() {
    let fx = Fx::new("ac-ur-env");
    fx.overlay(OWNER_MD);
    let (_, out, _) = fx.ac_env(&["user-rules"], &[("DOTFILES_PROFILE", "owner")]);
    assert!(out.contains("## Off-limits"), "{out}");
}

#[cfg(unix)]
#[test]
fn test_copy_sends_text_to_clipboard_writes_mark_and_drops_file_copy() {
    let fx = Fx::new("ac-ur-copy");
    fx.ac(&["cursor"]); // file copies exist
    let gf = fx.cdir().join("rules").join("claude-global.mdc");
    assert!(gf.exists());
    let mine = fx.cdir().join("rules").join("claude-output-style.mdc");
    wr(&mine, "user's own\n");
    let (path, clip) = fake_clipboard(&fx);
    let (rc, out, _) = fx.ac_env(&["user-rules", "--profile", "shared", "--copy"], &[("PATH", &path)]);
    assert_eq!(rc, 0);
    let (_, printed, _) = fx.ac(&["user-rules", "--profile", "shared"]);
    let text = rd(&clip);
    assert_eq!(text, printed); // what went to the clipboard is what print mode shows
    assert_eq!(rd(&fx.home.join(USER_RULES_MARK)), format!("{}\n", sha256_hex(&text)));
    assert!(!gf.exists());
    assert_eq!(rd(&mine), "user's own\n"); // not ours, kept
    assert!(out.contains("copied the rules"), "{out}");
}

#[cfg(unix)]
#[test]
fn test_copy_without_a_clipboard_tool_prints_the_rules_and_fails() {
    let fx = Fx::new("ac-ur-no-clip");
    let empty = fx.tmp.join("empty-bin");
    fs::create_dir_all(&empty).unwrap();
    let (rc, out, _) = fx.ac_env(&["user-rules", "--profile", "shared", "--copy"], &[("PATH", empty.to_str().unwrap())]);
    assert_eq!(rc, 1);
    assert!(out.contains("## "), "{out}");
    assert!(!fx.home.join(USER_RULES_MARK).exists());
}

#[cfg(unix)]
#[test]
fn test_after_copy_cursor_rules_skips_then_warns_on_change() {
    let fx = Fx::new("ac-ur-after-copy");
    let (path, _) = fake_clipboard(&fx);
    fx.ac_env(&["user-rules", "--profile", "shared", "--copy"], &[("PATH", &path)]);
    let (_, out, err) = fx.ac(&["cursor"]);
    assert!(out.contains("already set up"), "{out}");
    assert_eq!(err, "");
    wr(&fx.claude.join("CLAUDE.md"), &format!("{CLAUDE_MD}\n## New\n\nmore\n"));
    let (_, _, err) = fx.ac(&["cursor"]);
    assert!(err.contains("Run cursor-rules"), "{err}");
    assert!(!fx.cdir().join("rules").join("claude-global.mdc").exists());
}

#[cfg(unix)]
#[test]
fn test_full_cursor_run_after_copy_creates_no_rule_files() {
    let fx = Fx::new("ac-ur-full-after-copy");
    let (path, _) = fake_clipboard(&fx);
    fx.ac_env(&["user-rules", "--profile", "shared", "--copy"], &[("PATH", &path)]);
    fx.ac(&["cursor"]);
    assert!(!fx.cdir().join("rules").join("claude-global.mdc").exists());
    assert!(fx.cdir().join("hooks.json").exists());
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
    fn new(tag: &str) -> Cs {
        let fx = Fx::new(tag);
        fx.settings(base());
        let (live, layer, last) = (
            fx.home.join(".claude").join("settings.json"),
            fx.home.join(".config/dotfiles/claude-settings.json"),
            fx.home.join(".config/dotfiles/claude-settings.last.json"),
        );
        Cs { fx, live, layer, last }
    }
    fn build(&self) -> R {
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
    wj(&cs.layer, json!({"model": "sonnet", "permissions": {"allow": ["Bash(ls:*)"]}, "theme": null}));
    cs.build();
    let got = rj(&cs.live);
    assert_eq!(got["model"], "sonnet");
    assert_eq!(got["permissions"]["allow"], json!(["Bash(git status:*)", "Bash(ls:*)"]));
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
    live["permissions"]["allow"].as_array_mut().unwrap().push(json!("WebFetch"));
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
    assert!(out.contains("kept in your private layer: permissions.allow (+1 item(s))"), "{out}");
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
    let sorted: std::collections::BTreeMap<String, Value> = rj(&cs.live).as_object().unwrap().clone().into_iter().collect();
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    serde::Serialize::serialize(&sorted, &mut serde_json::Serializer::with_formatter(&mut buf, fmt)).unwrap();
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
    cs.fx.settings(b);
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
    assert!(err.contains("Claude removed 1 item(s) from permissions.allow"), "{err}");
    assert_eq!(rj(&cs.live)["permissions"]["allow"], json!(["Bash(git status:*)"]));
}

#[test]
fn test_promote_moves_a_key_to_the_repo() {
    let cs = Cs::new("ac-cs-promote");
    wj(&cs.layer, json!({"permissions": {"allow": ["WebFetch"]}, "statusLine": {"type": "command"}}));
    cs.build();
    let (code, out, _) = cs.fx.ac(&["claude-settings", "promote", "permissions.allow"]);
    assert_eq!(code, 0);
    assert_eq!(rj(&cs.fx.claude.join("settings.json"))["permissions"]["allow"], json!(["Bash(git status:*)", "WebFetch"]));
    assert_eq!(rj(&cs.layer), json!({"statusLine": {"type": "command"}}));
    assert!(out.contains("Commit it to share it"), "{out}");
    assert_eq!(rj(&cs.live)["permissions"]["allow"], json!(["Bash(git status:*)", "WebFetch"]));
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

#[test]
fn test_cli() {
    let cs = Cs::new("ac-cs-cli");
    let (code, _, _) = cs.fx.ac(&["claude-settings", "build"]);
    assert_eq!(code, 0);
    let (code, _, _) = cs.fx.ac(&["claude-settings", "promote"]);
    assert_eq!(code, 2);
}

// --- main ---------------------------------------------------------------------

#[test]
fn test_requires_a_subcommand() {
    let fx = Fx::new("ac-main-subcommand");
    let (rc, _, _) = fx.ac(&[]);
    assert_ne!(rc, 0);
}

#[test]
fn test_dry_flag_is_set_per_invocation() {
    let fx = Fx::new("ac-main-dry-flag");
    fx.ac(&["cursor", "--dry-run"]);
    assert!(!fx.cdir().exists()); // the dry run wrote nothing
    fx.ac(&["cursor"]);
    assert!(fx.cdir().join("hooks.json").exists()); // the next run is not dry
}

// --- quiet output -------------------------------------------------------------

#[test]
fn test_a_second_opencode_run_is_one_line() {
    let fx = Fx::new("ac-quiet-second");
    fs::create_dir_all(fx.claude.join("agents")).unwrap();
    let quiet = |args: &[&str]| {
        let (home, repo) = (fx.home.to_str().unwrap(), fx.repo.to_str().unwrap());
        run_bin(AC, args, "", &[("HOME", home), ("DOTFILES_REPO", repo), ("DOTFILES_PROFILE", "shared")], None)
    };
    quiet(&["opencode"]);
    let (_, out, _) = quiet(&["opencode"]);
    assert_eq!(out.trim().lines().count(), 1, "{out}");
    assert!(out.contains("Opencode config:"), "{out}");
    assert!(out.contains("up to date"), "{out}");
}

// --- behavior the Rust tool first lacked, now the same as agent-config.py ----------

#[test]
fn test_one_failing_part_does_not_stop_the_others() {
    let fx = Fx::new("ac-cursor-failing-part");
    wj(&fx.cdir().join("cli-config.json"), json!({"permissions": {"allow": "not a list"}}));
    let (rc, _, err) = fx.ac(&["cursor"]);
    assert_eq!(rc, 1, "{err}");
    assert!(err.contains("skipped a part"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    assert!(fx.cdir().join("hooks.json").exists());
    assert!(fx.cdir().join("rules").join("claude-global.mdc").exists());
}

#[test]
fn test_an_older_generated_tui_is_replaced() {
    let fx = Fx::new("ac-oc-theme-legacy");
    // the first layout put the palette inline, which the schema rejects
    wj(
        &tui_path(&fx),
        json!({"theme": {"name": "copper", "background": "#2a2523"}, "_dotfiles": {"generated": true, "source": "claude/"}}),
    );
    let (rc, _, _) = run_oc(&fx);
    assert_eq!(rc, 0);
    assert_eq!(rj(&tui_path(&fx)), json!({"$schema": "https://opencode.ai/tui.json", "theme": "copper"}));
}

#[test]
fn test_a_broken_inline_theme_without_our_marker_is_the_users() {
    let fx = Fx::new("ac-oc-theme-inline");
    wr(&tui_path(&fx), "{\"theme\": {\"background\": \"#fff\"}}\n");
    let (rc, out, _) = run_oc(&fx);
    assert_eq!(rc, 0);
    assert!(out.contains("your Opencode theme"), "{out}");
    assert!(out.contains("{'background': '#fff'}"), "{out}"); // reported the way Python prints it
    assert!(rj(&tui_path(&fx))["theme"].as_object().unwrap().contains_key("background"));
}

#[test]
fn test_a_leading_tilde_means_the_home_folder() {
    let fx = Fx::new("ac-oc-proj-tilde");
    let p = fx.home.join("code").join("app");
    fs::create_dir_all(&p).unwrap();
    let (rc, _, err) = fx.ac(&["opencode", "--profile", "shared", "--projects", "~/code/app"]);
    assert_eq!(rc, 0, "{err}");
    assert!(p.join(".opencode").is_dir());
    assert!(!err.contains("non-absolute"), "{err}");
}

#[test]
fn test_rejects_unknown_profile() {
    let fx = Fx::new("ac-main-profile");
    for cmd in ["cursor", "opencode", "user-rules"] {
        let (rc, _, err) = fx.ac(&[cmd, "--profile", "root"]);
        assert_eq!(rc, 2, "{cmd}: {err}"); // argparse and clap both exit 2
        assert!(err.contains("invalid value 'root' for '--profile"), "{cmd}: {err}");
        assert!(err.contains("owner") && err.contains("shared"), "{cmd}: {err}");
    }
    assert!(!fx.cdir().exists() && !fx.odir().exists()); // nothing ran
}
