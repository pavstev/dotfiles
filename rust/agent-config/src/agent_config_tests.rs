//! Unit tests ported from `scripts/test_agent_config.py`: the helpers and the parts that write files,
//! called as functions. The command-level tests (`agent-config cursor|opencode|user-rules|
//! claude-settings` as a process) are in `tests/agent_config.rs`. `tests/PORTED.md` maps each Python
//! test to its twin here.
//!
//! Every test gets its own temp HOME and its own fake repo (`claude/`, `cursor/`). The functions read
//! HOME and the repo from the environment (`HOME`, `DOTFILES_REPO`) and the dry-run flag from a
//! global, so a test takes `LOCK` for its whole run and sets all three. Nothing here touches the
//! real `~/.cursor` or `~/.claude`.
use super::*;
use serde_json::json;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

const CLAUDE_MD_TEXT: &str = "# Global\n\nintro line\n\n## Environment\n\n- one\n- two\n\n## Working rules\n\n1. a\n2. b\n";
const OWNER_TEXT: &str = "## Off-limits\n\n- never do the secret thing\n";

// Only the tests of this file write the environment, and each holds LOCK. Other lib tests (for
// example `dots::os`) may read it at the same time without LOCK, so no test here writes a variable
// they depend on: PATH is never written (the clipboard test passes it as an argument).
fn set_env(k: &str, v: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: see the note above: every writer holds LOCK, and the variables written are HOME,
    // DOTFILES_* and NO_COLOR.
    unsafe { env::set_var(k, v) }
}
fn unset_env(k: &str) {
    // SAFETY: as in set_env.
    unsafe { env::remove_var(k) }
}

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
/// Names in a folder, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}
/// How many `<file>.bak-*` backups are in `dir`.
fn baks(dir: &Path, file: &str) -> usize {
    match fs::read_dir(dir) {
        Ok(rd) => rd.filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(&format!("{file}.bak-"))).count(),
        Err(_) => 0,
    }
}
#[cfg(unix)]
fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}
#[cfg(unix)]
fn link(target: &Path, at: &Path) {
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, at).unwrap();
}
fn m(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Run `f`, return (result, stdout, stderr) of what the module printed.
fn call<T>(f: impl FnOnce() -> T) -> (T, String, String) {
    let (r, lines) = out::capture(f);
    let join = |err: bool| lines.iter().filter(|(_, e)| *e == err).map(|(l, _)| format!("{l}\n")).collect::<String>();
    (r, join(false), join(true))
}

struct Fx {
    _guard: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    tmp: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    claude: PathBuf,
    cursor_src: PathBuf,
    guard: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::Builder::new().prefix("agent-config-test-").tempdir().unwrap();
        let tmp = dir.path().to_path_buf();
        let home = tmp.join("home");
        let repo = tmp.join("repo");
        fs::create_dir_all(&home).unwrap();
        set_env("HOME", &home);
        set_env("DOTFILES_REPO", &repo);
        set_env("DOTFILES_VERBOSE", "1"); // the tests read each line; the Quiet tests are in tests/agent_config.rs
        set_env("NO_COLOR", "1");
        for k in ["DOTFILES_PROFILE", "DOTFILES_MCP", "DOTFILES_PROJECTS", "DOTFILES_WARN_FILE"] {
            unset_env(k);
        }
        set_dry(false);
        let claude = repo.join("claude");
        let cursor_src = repo.join("cursor");
        let g = claude.join("hooks").join("rust-bin");
        wr(&claude.join("CLAUDE.md"), CLAUDE_MD_TEXT);
        wr(&g, "# fake guard\n");
        assert_eq!(home_dir(), home); // never the real HOME
        Fx { _guard: guard, _dir: dir, tmp, home, repo, claude, cursor_src, guard: g }
    }

    fn cdir(&self) -> PathBuf {
        self.home.join(".cursor")
    }
    fn overlay(&self, text: &str) -> PathBuf {
        let f = self.home.join(OWNER_MD);
        wr(&f, text);
        f
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
    fn setting_file(&self, text: &str) {
        wr(&self.home.join(SETTINGS), text);
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        set_dry(false);
    }
}

// --- pure text helpers --------------------------------------------------------

fn t(rule: &str) -> Option<String> {
    translate_permission(rule, "allow")
}
fn td(rule: &str) -> Option<String> {
    translate_permission(rule, "deny")
}
fn s(x: &str) -> Option<String> {
    Some(x.to_string())
}

#[test]
fn test_bash_prefix_forms() {
    assert_eq!(t("Bash(git status:*)"), s("Shell(git status)"));
    assert_eq!(t("Bash(npm run *)"), s("Shell(npm run)"));
    assert_eq!(t("Bash(ls:*)"), s("Shell(ls)"));
    assert_eq!(td("Bash(rm -rf *)"), s("Shell(rm -rf)"));
}

#[test]
fn test_bash_prefix_edge_cases_are_skipped() {
    assert_eq!(t("Bash(*)"), None);
    assert_eq!(t("Bash(:*)"), None);
    assert_eq!(t("Bash(a* b:*)"), None); // star inside the command part
}

#[test]
fn test_bash_exact_form() {
    assert_eq!(t("Bash(git status)"), s("Shell(git:status)"));
    assert_eq!(td("Bash(git push origin main)"), s("Shell(git:push origin main)"));
}

#[test]
fn test_bash_exact_without_args_or_with_star_is_skipped() {
    assert_eq!(t("Bash(ls)"), None);
    assert_eq!(t("Bash(git * foo)"), None);
}

#[test]
fn test_read_edit_write_allow_keeps_relative_path() {
    assert_eq!(t("Read(./src/**)"), s("Read(src/**)"));
    assert_eq!(t("Read(.env)"), s("Read(.env)"));
    assert_eq!(t("Read(/etc/hosts)"), s("Read(/etc/hosts)"));
}

#[test]
fn test_edit_and_write_both_become_write() {
    assert_eq!(t("Edit(./docs/**)"), s("Write(docs/**)"));
    assert_eq!(t("Write(notes.md)"), s("Write(notes.md)"));
}

#[test]
fn test_deny_relative_path_gets_double_star_prefix() {
    assert_eq!(td("Read(./.env)"), s("Read(**/.env)"));
    assert_eq!(td("Read(.env)"), s("Read(**/.env)"));
    assert_eq!(td("Edit(secrets/**)"), s("Write(**/secrets/**)"));
}

#[test]
fn test_deny_absolute_home_and_glob_paths_unchanged() {
    assert_eq!(td("Read(/etc/x)"), s("Read(/etc/x)"));
    assert_eq!(td("Read(~/.ssh/**)"), s("Read(~/.ssh/**)"));
    assert_eq!(td("Read(**/.env)"), s("Read(**/.env)"));
}

#[test]
fn test_webfetch_domain() {
    assert_eq!(t("WebFetch(domain:example.com)"), s("WebFetch(example.com)"));
    assert_eq!(t("WebFetch(https://example.com)"), None);
}

#[test]
fn test_unknown_or_malformed_rules() {
    for rule in ["WebSearch(x)", "Task(foo)", "mcp__srv__tool", "Read", "", "Bash", "Bash(unclosed"] {
        assert_eq!(t(rule), None, "{rule}");
    }
}

#[test]
fn test_surrounding_whitespace_is_ignored() {
    assert_eq!(t("  Bash(git status:*)  "), s("Shell(git status)"));
}

// --- split_frontmatter --------------------------------------------------------

#[test]
fn test_basic() {
    let (meta, body) = split_frontmatter("---\nname: foo\ndescription: does things\n---\nbody here\n");
    assert_eq!(meta, m(&[("name", "foo"), ("description", "does things")]));
    assert_eq!(body, "body here\n");
}

#[test]
fn test_no_frontmatter() {
    assert_eq!(split_frontmatter("just text\n"), (BTreeMap::new(), "just text\n".to_string()));
}

#[test]
fn test_value_containing_colon_and_dashes_in_key() {
    let (meta, _) = split_frontmatter("---\nallowed-tools: Read, Grep\ndescription: a: b\n---\nx");
    assert_eq!(meta["allowed-tools"], "Read, Grep");
    assert_eq!(meta["description"], "a: b");
}

#[test]
fn test_comments_skipped_and_indented_line_continues_value() {
    let (meta, _) = split_frontmatter("---\nname: x\n  more words\n- stray item\n# note\n---\nb");
    assert_eq!(meta, m(&[("name", "x more words")]));
}

#[test]
fn test_quoted_values() {
    let (meta, _) = split_frontmatter("---\na: \"x: y\"\nb: 'it''s'\nc: \"tab\\tend\"\n---\nb");
    assert_eq!(meta, m(&[("a", "x: y"), ("b", "it's"), ("c", "tab\tend")]));
}

#[test]
fn test_folded_and_literal_block_scalars() {
    let (meta, _) = split_frontmatter("---\nd: >\n  one\n  two\nl: |-\n  first\n  second\nn: z\n---\nb");
    assert_eq!(meta, m(&[("d", "one two"), ("l", "first\nsecond"), ("n", "z")]));
}

#[test]
fn test_list_under_empty_key_is_joined() {
    let (meta, _) = split_frontmatter("---\ntools:\n  - Read\n  - \"Grep\"\nname: r\n---\nb");
    assert_eq!(meta, m(&[("tools", "Read, Grep"), ("name", "r")]));
}

#[test]
fn test_crlf_frontmatter() {
    let (meta, body) = split_frontmatter("---\r\nname: w\r\n---\r\nbody\r\n");
    assert_eq!(meta, m(&[("name", "w")]));
    assert_eq!(body, "body\n");
}

#[test]
fn test_closing_fence_without_trailing_newline() {
    assert_eq!(split_frontmatter("---\na: b\n---"), (m(&[("a", "b")]), String::new()));
}

#[test]
fn test_unterminated_frontmatter_is_plain_text() {
    let text = "---\na: b\nno closing fence\n";
    assert_eq!(split_frontmatter(text), (BTreeMap::new(), text.to_string()));
}

#[test]
fn test_empty_value() {
    let (meta, _) = split_frontmatter("---\ntools:\n---\nb");
    assert_eq!(meta, m(&[("tools", "")]));
}

// --- small helpers ------------------------------------------------------------

#[test]
fn test_tilde() {
    let fx = Fx::new();
    let h = fx.home.display().to_string();
    assert_eq!(tilde(&fx.home.join(".cursor").join("x")), "~/.cursor/x");
    assert_eq!(tilde(&fx.home), "~");
    assert_eq!(tilde(Path::new("/elsewhere/x")), "/elsewhere/x");
    assert_eq!(tilde(Path::new(&format!("{h}-other/x"))), format!("{h}-other/x"));
}

#[test]
fn test_mdc_and_stamp() {
    assert_eq!(mdc("d", "B"), "---\ndescription: d\nalwaysApply: true\n---\nB");
    assert!(stamp(&["claude/CLAUDE.md"]).contains(MARK));
}

#[cfg(unix)]
#[test]
fn test_atomic_write_creates_parents_and_sets_mode() {
    let fx = Fx::new();
    let p = fx.tmp.join("a").join("b").join("f.txt");
    atomic_write(&p, "hi", Some(0o600)).unwrap();
    assert_eq!(rd(&p), "hi");
    assert_eq!(mode_of(&p), 0o600);
    atomic_write(&p, "again", None).unwrap(); // keeps the existing mode
    assert_eq!(mode_of(&p), 0o600);
    assert_eq!(names(p.parent().unwrap()), ["f.txt"]); // no temp leftovers
}

#[cfg(unix)]
#[test]
fn test_atomic_write_default_mode_for_new_file() {
    let fx = Fx::new();
    let p = fx.tmp.join("n.txt");
    atomic_write(&p, "x", None).unwrap();
    assert_eq!(mode_of(&p), 0o644);
}

#[test]
fn test_load_json() {
    let fx = Fx::new();
    let p = fx.tmp.join("j.json");
    assert_eq!(load_json(&p), Some(json!({}))); // missing
    fs::write(&p, "{bad").unwrap();
    assert_eq!(load_json(&p), None);
    fs::write(&p, "[1, 2]").unwrap();
    assert_eq!(load_json(&p), None); // not an object
    fs::write(&p, "{\"a\": 1}").unwrap();
    assert_eq!(load_json(&p), Some(json!({"a": 1})));
}

#[test]
fn test_setting_reads_like_setup_lib() {
    let fx = Fx::new();
    assert_eq!(setting(&fx.home, "PROFILE"), ""); // no file
    fx.setting_file("# PROFILE=\"note\"\nPROFILE=\"owner\"\nX=1\nPROFILE=shared  \nPROFILEX=\"no\"\n");
    assert_eq!(setting(&fx.home, "PROFILE"), "shared"); // the last line wins
    assert_eq!(setting(&fx.home, "X"), "1");
    assert_eq!(setting(&fx.home, "MISSING"), "");
    fx.setting_file("JAVA_HOME=\"/a b/c\"\r\nEMPTY=\"\"\n");
    assert_eq!(setting(&fx.home, "JAVA_HOME"), "/a b/c");
    assert_eq!(setting(&fx.home, "EMPTY"), "");
}

#[test]
fn test_default_profile_order() {
    let fx = Fx::new();
    assert_eq!(default_profile(), "shared");
    fx.setting_file("PROFILE=\"owner\"\n");
    assert_eq!(default_profile(), "owner");
    set_env("DOTFILES_PROFILE", "shared");
    assert_eq!(default_profile(), "shared"); // env beats the setting
    set_env("DOTFILES_PROFILE", "bogus");
    assert_eq!(default_profile(), "shared"); // invalid -> default
}

#[test]
fn test_default_profile_owner_when_overlay_exists() {
    let fx = Fx::new();
    fx.overlay("x\n");
    assert_eq!(default_profile(), "owner");
    fx.setting_file("PROFILE=\"shared\"\n");
    assert_eq!(default_profile(), "shared"); // the setting beats the overlay
    set_env("DOTFILES_PROFILE", "shared");
    assert_eq!(default_profile(), "shared");
}

#[test]
fn test_default_profile_garbage_in_setting() {
    let fx = Fx::new();
    fx.setting_file("PROFILE=\"admin\"\n");
    assert_eq!(default_profile(), "shared");
}

// --- profiles -----------------------------------------------------------------

#[test]
fn test_owner_gets_source_then_overlay() {
    let fx = Fx::new();
    fx.overlay(OWNER_TEXT);
    assert_eq!(rules_text(&fx.home, "owner"), format!("{}\n\n{}", CLAUDE_MD_TEXT.trim_end_matches('\n'), OWNER_TEXT));
}

#[test]
fn test_owner_without_overlay_gets_the_source() {
    let fx = Fx::new();
    assert_eq!(rules_text(&fx.home, "owner"), CLAUDE_MD_TEXT);
    assert_eq!(owner_overlay(&fx.home, "owner"), None);
}

#[test]
fn test_shared_never_reads_the_overlay() {
    let fx = Fx::new();
    fx.overlay(OWNER_TEXT);
    assert_eq!(rules_text(&fx.home, "shared"), CLAUDE_MD_TEXT);
    assert_eq!(owner_overlay(&fx.home, "shared"), None);
}

#[test]
fn test_overlay_that_is_a_directory_is_ignored() {
    let fx = Fx::new();
    fs::create_dir_all(fx.home.join(OWNER_MD)).unwrap();
    assert_eq!(rules_text(&fx.home, "owner"), CLAUDE_MD_TEXT);
}

#[test]
fn test_user_rules_text_has_cursor_note_first() {
    let fx = Fx::new();
    let text = user_rules_text(&fx.home, "owner");
    assert!(text.starts_with(CURSOR_NOTE));
    assert!(text.ends_with(CLAUDE_MD_TEXT));
}

#[test]
fn test_cursor_note_routes_named_agents_through_general_purpose() {
    // Cursor's Task rejects `code-reviewer`; the note must say what to send instead.
    let fx = Fx::new();
    for word in ["generalPurpose", "explore", "cursor-guide", "ci-investigator", "bugbot", "security-review", "best-of-n-runner", "~/.claude/agents/<name>.md"] {
        assert!(CURSOR_NOTE.contains(word), "{word}");
    }
    assert!(CURSOR_NOTE.contains("Read ~/.claude/agents/<name>.md first and follow it"));
    assert!(user_rules_text(&fx.home, "shared").contains(CURSOR_NOTE));
}

#[test]
fn test_user_rules_text_appends_output_style_after_the_overlay() {
    let fx = Fx::new();
    fx.overlay(OWNER_TEXT);
    fx.settings(json!({"outputStyle": "Terse"}));
    wr(&fx.claude.join("output-styles").join("terse.md"), "---\nname: Terse\n---\n\nBe brief.\n");
    let text = user_rules_text(&fx.home, "shared");
    assert!(text.contains("# Reply style: Terse\n\nBe brief.\n"));
    assert!(!text.contains("Off-limits"));
    let text = user_rules_text(&fx.home, "owner");
    assert!(text.find("## Off-limits").unwrap() < text.find("# Reply style: Terse").unwrap());
}

// --- generated Markdown ownership ---------------------------------------------

fn gen_text() -> String {
    format!("<!-- {MARK} -->\nhello\n")
}

#[test]
fn test_creates_file_and_parents() {
    let fx = Fx::new();
    let f = fx.cdir().join("rules").join("x.md");
    let ((), out, _) = call(|| write_generated(&f, &gen_text(), None));
    assert_eq!(rd(&f), gen_text());
    assert!(out.contains("wrote ~/.cursor/rules/x.md"), "{out}");
}

#[test]
fn test_rewrites_our_own_file_when_text_changes() {
    let fx = Fx::new();
    let f = fx.cdir().join("rules").join("x.md");
    call(|| write_generated(&f, &gen_text(), None));
    let new = format!("{}more\n", gen_text());
    call(|| write_generated(&f, &new, None));
    assert_eq!(rd(&f), new);
}

#[test]
fn test_unchanged_text_is_skipped() {
    let fx = Fx::new();
    let f = fx.cdir().join("rules").join("x.md");
    call(|| write_generated(&f, &gen_text(), None));
    let before = fs::metadata(&f).unwrap().modified().unwrap();
    let ((), out, _) = call(|| write_generated(&f, &gen_text(), None));
    assert!(out.contains("already set up"), "{out}");
    assert_eq!(fs::metadata(&f).unwrap().modified().unwrap(), before);
}

#[test]
fn test_user_file_without_marker_is_left_alone() {
    let fx = Fx::new();
    let f = fx.cdir().join("rules").join("x.md");
    wr(&f, "my own rules\n");
    let ((), out, err) = call(|| write_generated(&f, &gen_text(), None));
    assert_eq!(rd(&f), "my own rules\n");
    assert!(err.contains("not ours"), "{err}");
    assert_eq!(out, "");
}

#[cfg(unix)]
#[test]
fn test_symlink_is_left_alone_even_if_target_has_marker() {
    let fx = Fx::new();
    let f = fx.cdir().join("rules").join("x.md");
    let target = fx.tmp.join("elsewhere.md");
    fs::write(&target, gen_text()).unwrap();
    link(&target, &f);
    call(|| write_generated(&f, &format!("{}changed\n", gen_text()), None));
    assert!(is_symlink(&f));
    assert_eq!(rd(&target), gen_text());
}

#[test]
fn test_write_generated_dry_run_writes_nothing() {
    let fx = Fx::new();
    let f = fx.cdir().join("rules").join("x.md");
    set_dry(true);
    let ((), out, _) = call(|| write_generated(&f, &gen_text(), None));
    assert!(!f.exists());
    assert!(!fx.cdir().exists());
    assert!(out.contains("would write"), "{out}");
}

fn agent_file(fx: &Fx) -> PathBuf {
    let f = fx.cdir().join("agents").join("a.md");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    f
}

#[test]
fn test_removes_marked_file() {
    let fx = Fx::new();
    let f = agent_file(&fx);
    fs::write(&f, format!("x {MARK} y")).unwrap();
    let ((), out, _) = call(|| remove_generated(&f, "gone"));
    assert!(!f.exists());
    assert!(out.contains("removed ~/.cursor/agents/a.md (gone)"), "{out}");
}

#[test]
fn test_keeps_user_file() {
    let fx = Fx::new();
    let f = agent_file(&fx);
    fs::write(&f, "mine").unwrap();
    call(|| remove_generated(&f, ""));
    assert_eq!(rd(&f), "mine");
}

#[test]
fn test_missing_file_is_fine() {
    let fx = Fx::new();
    let f = agent_file(&fx);
    let ((), out, err) = call(|| remove_generated(&f, ""));
    assert_eq!((out.as_str(), err.as_str()), ("", ""));
}

#[cfg(unix)]
#[test]
fn test_symlink_is_kept() {
    let fx = Fx::new();
    let f = agent_file(&fx);
    let target = fx.tmp.join("t.md");
    fs::write(&target, MARK).unwrap();
    link(&target, &f);
    call(|| remove_generated(&f, ""));
    assert!(is_symlink(&f));
    assert!(target.exists());
}

#[test]
fn test_directory_is_ignored() {
    let fx = Fx::new();
    agent_file(&fx);
    let d = fx.cdir().join("agents").join("dir.md");
    fs::create_dir(&d).unwrap();
    call(|| remove_generated(&d, ""));
    assert!(d.is_dir());
}

#[test]
fn test_dry_run_keeps_file() {
    let fx = Fx::new();
    let f = agent_file(&fx);
    fs::write(&f, MARK).unwrap();
    set_dry(true);
    let ((), out, _) = call(|| remove_generated(&f, ""));
    assert!(f.exists());
    assert!(out.contains("would remove"), "{out}");
}

// --- cursor_rules, agents -----------------------------------------------------

fn style(fx: &Fx) {
    fx.settings(json!({"outputStyle": "Terse"}));
    wr(&fx.claude.join("output-styles").join("terse.md"), "---\nname: Terse\n---\n\nBe brief.\n");
}

fn mark(fx: &Fx, content: &str) {
    wr(&fx.home.join(USER_RULES_MARK), content);
}

#[test]
fn test_global_rule_for_each_profile() {
    let fx = Fx::new();
    fx.overlay(OWNER_TEXT);
    call(|| cursor_rules(&fx.home, "shared"));
    let f = fx.cdir().join("rules").join("claude-global.mdc");
    let text = rd(&f);
    assert!(text.starts_with("---\ndescription: "));
    assert!(text.contains("alwaysApply: true"));
    assert!(text.contains(MARK));
    assert!(!text.contains("Off-limits"));
    assert!(!text.contains("owner.md"));
    call(|| cursor_rules(&fx.home, "owner"));
    assert!(rd(&f).contains("## Off-limits"));
    assert!(rd(&f).contains("claude/CLAUDE.md and ~/.config/dotfiles/owner.md"));
}

#[test]
fn test_cursor_rules_rerun_is_idempotent() {
    let fx = Fx::new();
    call(|| cursor_rules(&fx.home, "shared"));
    let ((), out, _) = call(|| cursor_rules(&fx.home, "shared"));
    assert!(out.contains("already set up"), "{out}");
}

#[test]
fn test_output_style_rule_written_then_removed() {
    let fx = Fx::new();
    style(&fx);
    call(|| cursor_rules(&fx.home, "shared"));
    let sf = fx.cdir().join("rules").join("claude-output-style.mdc");
    assert!(rd(&sf).contains("# Reply style: Terse"));
    assert!(rd(&sf).contains("claude/output-styles/terse.md"));
    fx.settings(json!({"outputStyle": "default"}));
    call(|| cursor_rules(&fx.home, "shared"));
    assert!(!sf.exists());
}

#[test]
fn test_user_owned_style_rule_is_not_removed() {
    let fx = Fx::new();
    let sf = fx.cdir().join("rules").join("claude-output-style.mdc");
    wr(&sf, "mine\n");
    call(|| cursor_rules(&fx.home, "shared")); // no active style -> remove_generated
    assert_eq!(rd(&sf), "mine\n");
}

#[test]
fn test_user_owned_global_rule_is_not_overwritten() {
    let fx = Fx::new();
    let gf = fx.cdir().join("rules").join("claude-global.mdc");
    wr(&gf, "mine\n");
    let ((), _, err) = call(|| cursor_rules(&fx.home, "shared"));
    assert_eq!(rd(&gf), "mine\n");
    assert!(err.contains("not ours"), "{err}");
}

#[test]
fn test_unknown_style_name_means_no_style_file() {
    let fx = Fx::new();
    fx.settings(json!({"outputStyle": "Missing"}));
    call(|| cursor_rules(&fx.home, "shared"));
    assert!(!fx.cdir().join("rules").join("claude-output-style.mdc").exists());
}

#[test]
fn test_style_matched_by_file_stem() {
    let fx = Fx::new();
    fx.settings(json!({"outputStyle": "terse"}));
    wr(&fx.claude.join("output-styles").join("terse.md"), "no frontmatter body\n");
    call(|| cursor_rules(&fx.home, "shared"));
    assert!(rd(&fx.cdir().join("rules").join("claude-output-style.mdc")).contains("no frontmatter body"));
}

#[test]
fn test_mark_present_removes_our_files_and_skips_when_hash_matches() {
    let fx = Fx::new();
    style(&fx);
    call(|| cursor_rules(&fx.home, "shared")); // file copies exist
    let rules = fx.cdir().join("rules");
    assert!(rules.join("claude-global.mdc").exists());
    mark(&fx, &format!("{}\n", sha(&user_rules_text(&fx.home, "shared"))));
    let ((), out, err) = call(|| cursor_rules(&fx.home, "shared"));
    assert!(!rules.join("claude-global.mdc").exists());
    assert!(!rules.join("claude-output-style.mdc").exists());
    assert!(out.contains("Cursor User Rules"), "{out}");
    assert!(out.contains("already set up"), "{out}");
    assert_eq!(err, "");
}

#[test]
fn test_mark_present_warns_when_rules_changed() {
    let fx = Fx::new();
    mark(&fx, &format!("{}\n", sha("old text")));
    let ((), out, err) = call(|| cursor_rules(&fx.home, "shared"));
    assert!(err.contains("Run cursor-rules"), "{err}");
    assert!(!out.contains("already set up"), "{out}");
    assert!(!fx.cdir().join("rules").join("claude-global.mdc").exists());
}

#[test]
fn test_mark_present_leaves_user_files_alone() {
    let fx = Fx::new();
    let gf = fx.cdir().join("rules").join("claude-global.mdc");
    wr(&gf, "mine\n");
    mark(&fx, &sha(&user_rules_text(&fx.home, "shared")));
    call(|| cursor_rules(&fx.home, "shared"));
    assert_eq!(rd(&gf), "mine\n");
}

#[test]
fn test_mark_hash_depends_on_profile() {
    let fx = Fx::new();
    fx.overlay(OWNER_TEXT);
    mark(&fx, &sha(&user_rules_text(&fx.home, "owner")));
    let ((), _, err) = call(|| cursor_rules(&fx.home, "shared"));
    assert!(err.contains("Run cursor-rules"), "{err}");
}

#[test]
fn test_removing_the_mark_brings_the_file_copy_back() {
    let fx = Fx::new();
    mark(&fx, &sha(&user_rules_text(&fx.home, "shared")));
    call(|| cursor_rules(&fx.home, "shared"));
    fs::remove_file(fx.home.join(USER_RULES_MARK)).unwrap();
    call(|| cursor_rules(&fx.home, "shared"));
    assert!(fx.cdir().join("rules").join("claude-global.mdc").exists());
}

#[test]
fn test_plain_when_safe() {
    assert_eq!(yaml_str("Reviews a diff, then reports."), "Reviews a diff, then reports.");
}

#[test]
fn test_quotes_when_needed() {
    for v in ["a: b", "x #y", "- dash", "'q", "multi\nline", " pad"] {
        assert_eq!(serde_json::from_str::<String>(&yaml_str(v)).unwrap(), v, "{v}");
    }
}

// The indented line `  Use it: after edits` is a continuation line, not a new key `it`: the key match
// is anchored at the line start, like `re.match` in agent-config.py.
#[test]
fn test_description_with_colon_is_quoted_and_round_trips() {
    let fx = Fx::new();
    fx.agent("r.md", "---\nname: r\ndescription: >\n  Use it: after edits\ntools:\n  - Read\n---\nB\n");
    call(|| cursor_agents(&fx.home));
    let (meta, _) = split_frontmatter(&rd(&fx.cdir().join("agents").join("r.md")));
    assert_eq!(meta["description"], "Use it: after edits");
    assert_eq!(meta["readonly"], "true");
}

#[test]
fn test_cursor_agents_converts_frontmatter() {
    let fx = Fx::new();
    fx.agent("rev.md", "---\nname: reviewer\ndescription: reviews\nmodel: sonnet\ntools: Read, Grep\n---\n\nBody\n");
    call(|| cursor_agents(&fx.home));
    let out = rd(&fx.cdir().join("agents").join("reviewer.md"));
    assert!(out.starts_with("---\nname: reviewer\ndescription: reviews\nmodel: inherit\nreadonly: true\n---\n"), "{out}");
    assert!(out.contains(MARK));
    assert!(!out.contains("sonnet"));
    assert!(out.ends_with("Body\n"));
}

#[test]
fn test_write_capable_tools_are_not_readonly() {
    let fx = Fx::new();
    fx.agent("w.md", "---\nname: w\ntools: Read, Bash\n---\nb\n");
    fx.agent("n.md", "---\nname: n\n---\nb\n"); // no tools field
    call(|| cursor_agents(&fx.home));
    assert!(!rd(&fx.cdir().join("agents").join("w.md")).contains("readonly"));
    assert!(!rd(&fx.cdir().join("agents").join("n.md")).contains("readonly"));
}

#[test]
fn test_name_falls_back_to_file_stem_and_readme_is_skipped() {
    let fx = Fx::new();
    fx.agent("stem-name.md", "no frontmatter\n");
    fx.agent("README.md", "# readme\n");
    call(|| cursor_agents(&fx.home));
    assert_eq!(names(&fx.cdir().join("agents")), ["stem-name.md"]);
}

#[test]
fn test_cursor_agents_stale_generated_agent_removed_user_agent_kept() {
    let fx = Fx::new();
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    call(|| cursor_agents(&fx.home));
    let mine = fx.cdir().join("agents").join("mine.md");
    fs::write(&mine, "my agent\n").unwrap();
    fs::remove_file(fx.claude.join("agents").join("a.md")).unwrap();
    call(|| cursor_agents(&fx.home));
    assert!(!fx.cdir().join("agents").join("a.md").exists());
    assert_eq!(rd(&mine), "my agent\n");
}

#[test]
fn test_cursor_agents_user_agent_with_same_name_is_not_overwritten() {
    let fx = Fx::new();
    fx.agent("a.md", "---\nname: a\n---\nb\n");
    wr(&fx.cdir().join("agents").join("a.md"), "mine\n");
    let ((), _, err) = call(|| cursor_agents(&fx.home));
    assert_eq!(rd(&fx.cdir().join("agents").join("a.md")), "mine\n");
    assert!(err.contains("not ours"), "{err}");
}

#[test]
fn test_cursor_agents_no_agents_dir_is_fine() {
    let fx = Fx::new();
    call(|| cursor_agents(&fx.home));
    assert!(!fx.cdir().join("agents").exists());
}

// --- JSON merges --------------------------------------------------------------

#[test]
fn test_backup_made_before_change() {
    let fx = Fx::new();
    let p = fx.cdir().join("x.json");
    wj(&p, json!({"a": 1}));
    let ((), out, _) = call(|| save_json(&p, &json!({"a": 1}), &json!({"a": 2}), false));
    let bak: Vec<_> = fs::read_dir(fx.cdir()).unwrap().map(|e| e.unwrap().path()).filter(|p| p.to_string_lossy().contains("x.json.bak-")).collect();
    assert_eq!(bak.len(), 1);
    assert_eq!(rj(&bak[0]), json!({"a": 1}));
    assert_eq!(rj(&p), json!({"a": 2}));
    assert!(out.contains("updated"), "{out}");
}

#[test]
fn test_no_backup_for_new_file() {
    let fx = Fx::new();
    let p = fx.cdir().join("x.json");
    call(|| save_json(&p, &json!({}), &json!({"a": 1}), false));
    assert_eq!(names(&fx.cdir()), ["x.json"]);
}

#[test]
fn test_unchanged_is_skipped() {
    let fx = Fx::new();
    let p = fx.cdir().join("x.json");
    wj(&p, json!({"a": 1}));
    let ((), out, _) = call(|| save_json(&p, &json!({"a": 1}), &json!({"a": 1}), false));
    assert!(out.contains("already set up"), "{out}");
    assert_eq!(names(&fx.cdir()), ["x.json"]);
}

#[cfg(unix)]
#[test]
fn test_symlink_untouched() {
    let fx = Fx::new();
    let target = fx.tmp.join("t.json");
    wj(&target, json!({"a": 1}));
    let p = fx.cdir().join("x.json");
    link(&target, &p);
    let ((), _, err) = call(|| save_json(&p, &json!({"a": 1}), &json!({"a": 2}), false));
    assert_eq!(rj(&target), json!({"a": 1}));
    assert!(err.contains("symlink"), "{err}");
}

#[cfg(unix)]
#[test]
fn test_private_mode() {
    let fx = Fx::new();
    let p = fx.cdir().join("x.json");
    call(|| save_json(&p, &json!({}), &json!({"a": 1}), true));
    assert_eq!(mode_of(&p), 0o600);
}

#[test]
fn test_dry_run() {
    let fx = Fx::new();
    set_dry(true);
    let p = fx.cdir().join("x.json");
    let ((), out, _) = call(|| save_json(&p, &json!({}), &json!({"a": 1}), false));
    assert!(!p.exists());
    assert!(out.contains("would update"), "{out}");
}

// --- cursor_hooks -------------------------------------------------------------

const HOOK_EVENTS: [(&str, &str, Option<&str>); 9] = [
    ("beforeSubmitPrompt", "prompt", None),
    ("subagentStart", "subagent", None),
    ("preToolUse", "subagent", Some("^(Task|Subagent|mcp_task)$")),
    ("preToolUse", "tool", Some("^(Shell|Read|Grep)$")),
    ("beforeShellExecution", "tool", None),
    ("beforeMCPExecution", "tool", None),
    ("beforeReadFile", "tool", None),
    ("preCompact", "compact", None),
    ("afterAgentResponse", "response", None),
];
const GUARD_TAG: &str = "--hook guard_cursor";

fn hooks_path(fx: &Fx) -> PathBuf {
    fx.cdir().join("hooks.json")
}

#[test]
fn test_creates_file_with_all_events() {
    let fx = Fx::new();
    call(|| cursor_hooks(&fx.home));
    let data = rj(&hooks_path(&fx));
    assert_eq!(data["version"], 1);
    let got: BTreeSet<&str> = data["hooks"].as_object().unwrap().keys().map(String::as_str).collect();
    let want: BTreeSet<&str> = HOOK_EVENTS.iter().map(|e| e.0).collect();
    assert_eq!(got, want);
    for (event, mode, matcher) in HOOK_EVENTS {
        let entries = data["hooks"][event].as_array().unwrap();
        let want = HOOK_EVENTS.iter().filter(|e| e.0 == event).count();
        assert_eq!(entries.len(), want, "{event}");
        // our entries sit in the order of HOOK_EVENTS
        let nth = HOOK_EVENTS.iter().filter(|e| e.0 == event).position(|e| e.1 == mode && e.2 == matcher).unwrap();
        let entry = &entries[nth];
        let cmd = entry["command"].as_str().unwrap();
        assert!(cmd.starts_with("test -r "), "{cmd}");
        assert!(cmd.contains("|| exit 0;"), "{cmd}"); // a missing script must not deny
        assert!(cmd.ends_with(&format!("bash {} --hook guard_cursor {}", fx.guard.display(), mode)), "{cmd}");
        assert!(!cmd.contains("python"), "{cmd}");
        assert_eq!(entry["timeout"], 10);
        match matcher {
            Some(mt) => assert_eq!(entry["matcher"], mt),
            None => assert!(entry.get("matcher").is_none()),
        }
    }
}

#[test]
fn test_user_entries_and_keys_are_kept() {
    let fx = Fx::new();
    let mine = json!({"command": "echo mine", "timeout": 3});
    wj(&hooks_path(&fx), json!({"version": 2, "hooks": {"preToolUse": [mine], "stop": [mine]}, "extra": true}));
    call(|| cursor_hooks(&fx.home));
    let data = rj(&hooks_path(&fx));
    assert_eq!(data["version"], 2); // user's version wins
    assert_eq!(data["extra"], true);
    assert_eq!(data["hooks"]["stop"], json!([mine]));
    assert_eq!(data["hooks"]["preToolUse"][0], mine);
    assert_eq!(data["hooks"]["preToolUse"].as_array().unwrap().len(), 3); // the user's, subagent, tool
}

#[test]
fn cursor_hooks_json_uses_rust_bin() {
    let fx = Fx::new();
    call(|| cursor_hooks(&fx.home));
    let text = rd(&hooks_path(&fx));
    assert!(text.contains("rust-bin --hook guard_cursor"), "{text}");
    assert!(!text.contains("python"), "{text}");
    let data = rj(&hooks_path(&fx));
    // the tool mode runs on the shell, MCP and read events, and the entries keep their order on a rerun
    let modes = |event: &str| -> Vec<String> {
        data["hooks"][event].as_array().unwrap().iter().map(|e| e["command"].as_str().unwrap().rsplit(' ').next().unwrap().to_string()).collect()
    };
    assert_eq!(modes("preToolUse"), ["subagent", "tool"]);
    assert_eq!(modes("beforeShellExecution"), ["tool"]);
    assert_eq!(modes("beforeMCPExecution"), ["tool"]);
    assert_eq!(modes("beforeReadFile"), ["tool"]);
    call(|| cursor_hooks(&fx.home));
    assert_eq!(rj(&hooks_path(&fx)), data);
}

#[test]
fn test_our_entry_keeps_its_place_between_user_entries() {
    let fx = Fx::new();
    call(|| cursor_hooks(&fx.home));
    let mut data = rj(&hooks_path(&fx));
    let mut list = vec![json!({"command": "echo a"})];
    list.extend(data["hooks"]["preCompact"].as_array().unwrap().clone());
    list.push(json!({"command": "echo b"}));
    data["hooks"]["preCompact"] = Value::Array(list);
    wj(&hooks_path(&fx), data);
    call(|| cursor_hooks(&fx.home));
    let cmds: Vec<String> = rj(&hooks_path(&fx))["hooks"]["preCompact"].as_array().unwrap().iter().map(|e| e["command"].as_str().unwrap().to_string()).collect();
    assert_eq!(cmds[0], "echo a");
    assert!(cmds[1].contains(GUARD_TAG));
    assert_eq!(cmds[2], "echo b");
}

#[test]
fn test_rerun_is_idempotent_and_makes_no_backup() {
    let fx = Fx::new();
    call(|| cursor_hooks(&fx.home));
    let first = rd(&hooks_path(&fx));
    let ((), out, _) = call(|| cursor_hooks(&fx.home));
    assert_eq!(rd(&hooks_path(&fx)), first);
    assert!(out.contains("already set up"), "{out}");
    assert_eq!(baks(&fx.cdir(), "hooks.json"), 0);
}

#[test]
fn test_old_entries_for_a_moved_script_are_replaced() {
    let fx = Fx::new();
    let old = json!({"command": "python3 /old/home/cursor/hooks/guard_cursor.py prompt"});
    wj(&hooks_path(&fx), json!({"version": 1, "hooks": {"beforeSubmitPrompt": [old]}}));
    call(|| cursor_hooks(&fx.home));
    let cmds: Vec<String> = rj(&hooks_path(&fx))["hooks"]["beforeSubmitPrompt"].as_array().unwrap().iter().map(|e| e["command"].as_str().unwrap().to_string()).collect();
    assert_eq!(cmds.len(), 1);
    assert!(!cmds[0].contains("/old/home"));
    assert_eq!(baks(&fx.cdir(), "hooks.json"), 1);
}

#[test]
fn test_old_python_entries_are_replaced_by_the_rust_launcher() {
    let fx = Fx::new();
    let mine = json!({"command": "echo mine"});
    let old_cmd = format!("test -r '{0}/cursor/hooks/guard_cursor.py' || exit 0; python3 '{0}/cursor/hooks/guard_cursor.py' prompt", fx.repo.display());
    let old = json!({"command": old_cmd, "timeout": 10});
    wj(&hooks_path(&fx), json!({"version": 1, "hooks": {"beforeSubmitPrompt": [mine, old]}}));
    call(|| cursor_hooks(&fx.home));
    let cmds: Vec<String> = rj(&hooks_path(&fx))["hooks"]["beforeSubmitPrompt"].as_array().unwrap().iter().map(|e| e["command"].as_str().unwrap().to_string()).collect();
    assert_eq!(cmds.len(), 2, "{cmds:?}");
    assert_eq!(cmds[0], "echo mine");
    assert!(cmds[1].ends_with(&format!("bash {} --hook guard_cursor prompt", fx.guard.display())), "{cmds:?}");
    assert!(!cmds.iter().any(|c| c.contains("guard_cursor.py") || c.contains("python")), "{cmds:?}");
}

#[test]
fn test_missing_guard_removes_our_entries_and_empty_events() {
    let fx = Fx::new();
    call(|| cursor_hooks(&fx.home));
    let mine = json!({"command": "echo mine"});
    let mut data = rj(&hooks_path(&fx));
    data["hooks"]["preCompact"].as_array_mut().unwrap().push(mine.clone());
    wj(&hooks_path(&fx), data);
    fs::remove_file(&fx.guard).unwrap();
    call(|| cursor_hooks(&fx.home));
    assert_eq!(rj(&hooks_path(&fx))["hooks"], json!({"preCompact": [mine]}));
}

#[test]
fn test_missing_guard_and_no_file_still_writes_skeleton() {
    let fx = Fx::new();
    fs::remove_file(&fx.guard).unwrap();
    call(|| cursor_hooks(&fx.home));
    assert_eq!(rj(&hooks_path(&fx)), json!({"version": 1, "hooks": {}}));
}

#[test]
fn test_cursor_hooks_invalid_json_left_untouched() {
    let fx = Fx::new();
    wr(&hooks_path(&fx), "{oops");
    let ((), _, err) = call(|| cursor_hooks(&fx.home));
    assert_eq!(rd(&hooks_path(&fx)), "{oops");
    assert!(err.contains("not valid JSON"), "{err}");
}

#[test]
fn test_non_object_hooks_left_untouched() {
    let fx = Fx::new();
    wj(&hooks_path(&fx), json!({"hooks": []}));
    let ((), _, err) = call(|| cursor_hooks(&fx.home));
    assert_eq!(rj(&hooks_path(&fx)), json!({"hooks": []}));
    assert!(err.contains("non-object"), "{err}");
}

#[test]
fn test_path_with_spaces_is_quoted() {
    let fx = Fx::new();
    // The script path comes from DOTFILES_REPO, so a repo folder with a space moves it.
    let spaced = fx.repo.join("my cursor");
    wr(&spaced.join("claude").join("hooks").join("rust-bin"), "#");
    set_env("DOTFILES_REPO", &spaced);
    call(|| cursor_hooks(&fx.home));
    let cmd = rj(&hooks_path(&fx))["hooks"]["preCompact"][0]["command"].as_str().unwrap().to_string();
    assert!(cmd.contains(&format!("'{}'", spaced.join("claude").join("hooks").join("rust-bin").display())), "{cmd}");
    // a second run still recognises its own quoted entries
    call(|| cursor_hooks(&fx.home));
    assert_eq!(rj(&hooks_path(&fx))["hooks"]["preCompact"].as_array().unwrap().len(), 1);
}

#[test]
fn test_cursor_hooks_dry_run_writes_nothing() {
    let fx = Fx::new();
    set_dry(true);
    call(|| cursor_hooks(&fx.home));
    assert!(!fx.cdir().exists());
}

// --- cursor_mcp ---------------------------------------------------------------

fn mcp_fx() -> Fx {
    let fx = Fx::new();
    fx.setting_file("CURSOR_MCP=\"on\"\n"); // MCP is opt-in; most tests here are about the merge
    fx
}
fn mcp_path(fx: &Fx) -> PathBuf {
    fx.cdir().join("mcp.json")
}
fn ours(fx: &Fx, servers: Value) {
    wj(&fx.cursor_src.join("mcp.json"), json!({"mcpServers": servers}));
}

#[test]
fn test_adds_ours_to_new_file() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {"url": "https://example.invalid/mcp"}}));
    call(|| cursor_mcp(&fx.home));
    assert_eq!(rj(&mcp_path(&fx)), json!({"mcpServers": {"docs": {"url": "https://example.invalid/mcp"}}}));
}

#[test]
fn test_user_entry_of_same_name_wins_and_others_kept() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {"url": "ours"}, "extra": {"command": "x"}}));
    wj(&mcp_path(&fx), json!({"mcpServers": {"docs": {"url": "users"}, "mine": {"command": "m"}}, "other": 1}));
    call(|| cursor_mcp(&fx.home));
    let data = rj(&mcp_path(&fx));
    assert_eq!(data["mcpServers"]["docs"], json!({"url": "users"}));
    assert_eq!(data["mcpServers"]["mine"], json!({"command": "m"}));
    assert_eq!(data["mcpServers"]["extra"], json!({"command": "x"}));
    assert_eq!(data["other"], 1);
}

#[test]
fn test_idempotent() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {"url": "ours"}}));
    call(|| cursor_mcp(&fx.home));
    let ((), out, _) = call(|| cursor_mcp(&fx.home));
    assert!(out.contains("already set up"), "{out}");
    assert_eq!(baks(&fx.cdir(), "mcp.json"), 0);
}

#[test]
fn test_no_source_file_keeps_user_file_content() {
    let fx = mcp_fx();
    wj(&mcp_path(&fx), json!({"mcpServers": {"mine": {}}}));
    call(|| cursor_mcp(&fx.home));
    assert_eq!(rj(&mcp_path(&fx)), json!({"mcpServers": {"mine": {}}}));
}

#[test]
fn test_cursor_mcp_invalid_json_left_untouched() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {}}));
    wr(&mcp_path(&fx), "[]");
    let ((), _, err) = call(|| cursor_mcp(&fx.home));
    assert_eq!(rd(&mcp_path(&fx)), "[]");
    assert!(err.contains("not valid JSON"), "{err}");
}

#[test]
fn test_skips_without_opt_in_on_every_profile() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {}}));
    fs::remove_file(fx.home.join(SETTINGS)).unwrap();
    fx.overlay(OWNER_TEXT); // private rules (the owner profile) do not turn MCP on
    let ((), out, _) = call(|| cursor_mcp(&fx.home));
    assert!(!mcp_path(&fx).exists());
    assert!(out.contains("opt-in"), "{out}");
}

#[test]
fn test_opt_in_setting() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {}}));
    fx.setting_file("CURSOR_MCP=\"on\"\n");
    call(|| cursor_mcp(&fx.home));
    assert!(rj(&mcp_path(&fx))["mcpServers"].get("docs").is_some());
}

#[test]
fn test_off_setting() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {}}));
    fx.setting_file("CURSOR_MCP=\"off\"\n");
    let ((), out, _) = call(|| cursor_mcp(&fx.home));
    assert!(!mcp_path(&fx).exists());
    assert!(out.contains("dots settings"), "{out}");
}

#[test]
fn test_env_wins_over_setting() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {}}));
    fx.setting_file("CURSOR_MCP=\"on\"\n");
    set_env("DOTFILES_MCP", "off");
    call(|| cursor_mcp(&fx.home));
    assert!(!mcp_path(&fx).exists());
    fx.setting_file("CURSOR_MCP=\"off\"\n");
    set_env("DOTFILES_MCP", "1");
    call(|| cursor_mcp(&fx.home));
    assert!(mcp_path(&fx).exists());
}

#[test]
fn test_change_makes_backup() {
    let fx = mcp_fx();
    ours(&fx, json!({"docs": {}}));
    wj(&mcp_path(&fx), json!({"mcpServers": {}}));
    call(|| cursor_mcp(&fx.home));
    assert_eq!(baks(&fx.cdir(), "mcp.json"), 1);
}

// --- cursor_cli_config --------------------------------------------------------

fn cfg_path(fx: &Fx) -> PathBuf {
    fx.cdir().join("cli-config.json")
}
fn side_path(fx: &Fx) -> PathBuf {
    fx.cdir().join(MANAGED)
}
fn run_cfg(fx: &Fx) -> ((), String, String) {
    call(|| cursor_cli_config(&fx.home))
}

#[cfg(unix)]
#[test]
fn test_defaults_on_empty_home() {
    let fx = Fx::new();
    run_cfg(&fx);
    let data = rj(&cfg_path(&fx));
    assert_eq!(data["version"], 1);
    assert_eq!(data["editor"], json!({"vimMode": false}));
    assert_eq!(data["permissions"], json!({"allow": [], "deny": []}));
    assert_eq!(data["attribution"], json!({"attributeCommitsToAgent": false, "attributePRsToAgent": false}));
    assert_eq!(mode_of(&cfg_path(&fx)), 0o600);
}

#[test]
fn test_user_values_win_but_attribution_is_forced_off() {
    let fx = Fx::new();
    wj(&cfg_path(&fx), json!({
        "version": 7, "editor": {"vimMode": true}, "model": {"id": "x"},
        "attribution": {"attributeCommitsToAgent": true, "attributePRsToAgent": true, "keep": "me"},
    }));
    run_cfg(&fx);
    let data = rj(&cfg_path(&fx));
    assert_eq!(data["version"], 7);
    assert_eq!(data["editor"], json!({"vimMode": true}));
    assert_eq!(data["model"], json!({"id": "x"}));
    assert_eq!(data["attribution"]["attributeCommitsToAgent"], false);
    assert_eq!(data["attribution"]["attributePRsToAgent"], false);
    assert_eq!(data["attribution"]["keep"], "me");
}

#[test]
fn test_translated_rules_added_and_user_rules_kept() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)", "Read(./src/**)", "Glob(x)"], &["Read(./.env)"]);
    wj(&cfg_path(&fx), json!({"permissions": {"allow": ["Shell(mine)"], "deny": ["Read(secret)"], "other": 1}}));
    run_cfg(&fx);
    let p = rj(&cfg_path(&fx))["permissions"].clone();
    assert_eq!(p["allow"], json!(["Shell(mine)", "Shell(git status)", "Read(src/**)"]));
    assert_eq!(p["deny"], json!(["Read(secret)", "Read(**/.env)"]));
    assert_eq!(p["other"], 1);
}

#[test]
fn test_sidecar_records_what_we_added() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)", "Nope(x)"], &["Read(./.env)"]);
    run_cfg(&fx);
    assert_eq!(rj(&side_path(&fx)), json!({"allow": ["Shell(git status)"], "deny": ["Read(**/.env)"]}));
}

#[test]
fn test_cli_config_rerun_is_idempotent() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &["Read(./.env)"]);
    wj(&cfg_path(&fx), json!({"permissions": {"allow": ["Shell(mine)"]}}));
    run_cfg(&fx);
    let (cfg, sidecar) = (rd(&cfg_path(&fx)), rd(&side_path(&fx)));
    let n = baks(&fx.cdir(), "cli-config.json");
    assert_eq!(n, 1);
    let (_, out, _) = run_cfg(&fx);
    assert_eq!(rd(&cfg_path(&fx)), cfg);
    assert_eq!(rd(&side_path(&fx)), sidecar);
    assert_eq!(baks(&fx.cdir(), "cli-config.json"), n);
    assert!(out.contains("already set up"), "{out}");
}

#[test]
fn test_no_duplicates_when_user_already_has_the_rule() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    wj(&cfg_path(&fx), json!({"permissions": {"allow": ["Shell(git status)"]}}));
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["permissions"]["allow"], json!(["Shell(git status)"]));
}

#[test]
fn test_stale_rule_we_owned_is_removed_user_rules_survive() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)", "Bash(npm test:*)"], &[]);
    run_cfg(&fx);
    let mut data = rj(&cfg_path(&fx));
    data["permissions"]["allow"].as_array_mut().unwrap().push(json!("Shell(mine)"));
    wj(&cfg_path(&fx), data);
    fx.perms(&["Bash(git status:*)"], &[]); // npm test dropped from the source
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["permissions"]["allow"], json!(["Shell(git status)", "Shell(mine)"]));
    assert_eq!(rj(&side_path(&fx))["allow"], json!(["Shell(git status)"]));
}

#[test]
fn test_rule_never_owned_is_never_removed() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    wj(&cfg_path(&fx), json!({"permissions": {"allow": ["Shell(npm test)"]}}));
    run_cfg(&fx);
    fx.perms(&[], &[]);
    run_cfg(&fx);
    // ours (git status) went, theirs stayed
    assert_eq!(rj(&cfg_path(&fx))["permissions"]["allow"], json!(["Shell(npm test)"]));
}

#[test]
fn test_deny_rules_are_tracked_separately_from_allow() {
    let fx = Fx::new();
    fx.perms(&["Read(./a)"], &["Read(./b)"]);
    run_cfg(&fx);
    fx.perms(&[], &[]);
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["permissions"], json!({"allow": [], "deny": []}));
    assert_eq!(rj(&side_path(&fx)), json!({"allow": [], "deny": []}));
}

#[test]
fn test_user_deleting_our_rule_gets_it_back() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    run_cfg(&fx);
    let mut data = rj(&cfg_path(&fx));
    data["permissions"]["allow"] = json!([]);
    wj(&cfg_path(&fx), data);
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["permissions"]["allow"], json!(["Shell(git status)"]));
}

#[test]
fn test_user_rule_equal_to_ours_is_kept_once() {
    // A hand-written rule that equals one of ours stays, and is not added twice.
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    wj(&cfg_path(&fx), json!({"permissions": {"allow": ["Shell(git status)"]}}));
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["permissions"]["allow"], json!(["Shell(git status)"]));
}

#[test]
fn test_corrupt_sidecar_is_survivable() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    wr(&side_path(&fx), "{not json");
    wj(&cfg_path(&fx), json!({"permissions": {"allow": ["Shell(mine)"]}}));
    let (_, _, err) = run_cfg(&fx);
    assert_eq!(err, "");
    assert_eq!(rj(&cfg_path(&fx))["permissions"]["allow"], json!(["Shell(mine)", "Shell(git status)"]));
    assert_eq!(rj(&side_path(&fx))["allow"], json!(["Shell(git status)"]));
}

#[cfg(unix)]
#[test]
fn test_sidecar_not_written_when_config_is_a_symlink() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    let target = fx.tmp.join("real.json");
    wj(&target, json!({"permissions": {"allow": []}}));
    link(&target, &cfg_path(&fx));
    let (_, _, err) = run_cfg(&fx);
    assert!(err.contains("symlink"), "{err}");
    assert!(!side_path(&fx).exists());
    assert_eq!(rj(&target), json!({"permissions": {"allow": []}}));
}

#[test]
fn test_invalid_config_json_left_untouched_and_no_sidecar() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    wr(&cfg_path(&fx), "nope");
    let (_, _, err) = run_cfg(&fx);
    assert_eq!(rd(&cfg_path(&fx)), "nope");
    assert!(err.contains("not valid JSON"), "{err}");
    assert!(!side_path(&fx).exists());
}

#[test]
fn test_dry_run_writes_neither_config_nor_sidecar() {
    let fx = Fx::new();
    fx.perms(&["Bash(git status:*)"], &[]);
    set_dry(true);
    run_cfg(&fx);
    assert!(!fx.cdir().exists());
}

#[test]
fn test_statusline_added_only_when_launcher_exists() {
    let fx = Fx::new();
    run_cfg(&fx);
    assert!(rj(&cfg_path(&fx)).get("statusLine").is_none());
    wr(&fx.home.join(".claude").join("hooks").join("rust-bin"), "#!/bin/sh\n");
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["statusLine"], json!({"type": "command", "command": "bash ~/.claude/hooks/rust-bin statusline", "padding": 0}));
}

#[test]
fn test_user_statusline_wins() {
    let fx = Fx::new();
    wr(&fx.home.join(".claude").join("hooks").join("rust-bin"), "#!/bin/sh\n");
    wj(&cfg_path(&fx), json!({"statusLine": {"type": "command", "command": "mine"}}));
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["statusLine"]["command"], "mine");
}

#[test]
fn test_old_statusline_sh_command_is_replaced() {
    let fx = Fx::new();
    wr(&fx.home.join(".claude").join("hooks").join("rust-bin"), "#!/bin/sh\n");
    let old = fx.home.join(".claude").join("statusline.sh");
    wj(&cfg_path(&fx), json!({"statusLine": {"type": "command", "command": old.to_str().unwrap(), "padding": 0}}));
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["statusLine"], json!({"type": "command", "command": "bash ~/.claude/hooks/rust-bin statusline", "padding": 0}));
    // a second run keeps the new command, and another user command still wins
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["statusLine"]["command"], "bash ~/.claude/hooks/rust-bin statusline");
    wj(&cfg_path(&fx), json!({"statusLine": {"type": "command", "command": "~/bin/my-line"}}));
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["statusLine"]["command"], "~/bin/my-line");
}

#[test]
fn test_missing_source_settings_is_fine() {
    let fx = Fx::new();
    run_cfg(&fx);
    assert_eq!(rj(&cfg_path(&fx))["permissions"], json!({"allow": [], "deny": []}));
}

// --- clipboard ----------------------------------------------------------------

#[test]
fn test_clipboard_command_picks_the_tool_for_the_platform() {
    // The Rust code picks the tool at compile time (`cfg!`), where Python patched `sys.platform`:
    // this runs the arm of the platform the test runs on.
    let fx = Fx::new();
    #[cfg(target_os = "macos")]
    assert_eq!(clipboard_command(), Some(vec!["pbcopy".to_string()]));
    #[cfg(target_os = "windows")]
    assert_eq!(clipboard_command(), Some(vec!["clip".to_string()]));
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = fx.tmp.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let xclip = bin.join("xclip");
        fs::write(&xclip, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&xclip, fs::Permissions::from_mode(0o755)).unwrap();
        let found = clipboard_command_in(bin.to_str());
        let none = clipboard_command_in(fx.tmp.join("nothing-here").to_str());
        assert_eq!(found, Some(vec!["xclip".to_string(), "-selection".to_string(), "clipboard".to_string()]));
        assert_eq!(none, None);
    }
    let _ = &fx;
}

// --- merge --------------------------------------------------------------------

#[test]
fn test_merge_rules() {
    let got = merge(
        &json!({"a": 1, "p": {"x": 1}, "permissions": {"allow": ["A"]}, "l": [1]}),
        &json!({"a": null, "p": {"y": 2}, "permissions": {"allow": ["B", "A"]}, "l": [2], "n": 3}),
    );
    assert_eq!(got, json!({"p": {"x": 1, "y": 2}, "permissions": {"allow": ["A", "B"]}, "l": [2], "n": 3}));
}
