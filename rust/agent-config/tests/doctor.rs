//! `dots doctor`: each check passes on a fresh setup and fails, with a reason, when its part is broken.
//! A fake repo and a temp HOME hold everything; nothing reads the real home of the machine.
mod common;

use agent_config::dots::{doctor, links};
use agent_config::presets;
use common::{run_bin, temp_home};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const AC: &str = env!("CARGO_BIN_EXE_agent-config");
const DOTS: &str = env!("CARGO_BIN_EXE_dots");

struct Fx {
    tmp: PathBuf,
    home: PathBuf,
    repo: PathBuf,
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

fn wr(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn set_mtime(p: &Path, t: SystemTime) {
    fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) {
    std::os::unix::fs::symlink(src, dst).unwrap();
}
#[cfg(windows)]
fn symlink(src: &Path, dst: &Path) {
    std::os::windows::fs::symlink_file(src, dst).unwrap();
}

fn git(fx: &Fx, args: &[&str]) {
    let st = Command::new("git").args(args).current_dir(&fx.repo).env("HOME", &fx.home).status().unwrap();
    assert!(st.success(), "git {args:?}");
}

impl Fx {
    fn ac(&self, args: &[&str]) -> (i32, String, String) {
        let (home, repo) = (self.home.to_str().unwrap(), self.repo.to_str().unwrap());
        run_bin(AC, args, "", &[("HOME", home), ("DOTFILES_REPO", repo)], None)
    }

    fn checks(&self) -> Vec<doctor::Check> {
        doctor::checks(&self.repo, &self.home)
    }

    fn get(&self, name: &str) -> Result<(), String> {
        self.checks().into_iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no check {name}")).result
    }

    fn setting(&self, text: &str) {
        wr(&self.home.join(".config/dotfiles/settings"), text);
    }
}

/// What `dots` leaves behind: the links, the built settings, the generated Cursor and Opencode
/// files, a repo with its bins built after its sources.
fn fixture(tag: &str) -> Fx {
    let tmp = temp_home(tag);
    let (home, repo) = (tmp.join("home"), tmp.join("repo"));
    fs::create_dir_all(&home).unwrap();
    let fx = Fx { tmp, home, repo };
    // home/ items and their links
    for item in links() {
        let src = fx.repo.join("home").join(item);
        wr(&src, "# fake\n"); // a comment is valid in a git config too
        let dst = fx.home.join(item);
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        symlink(&src, &dst);
    }
    // claude/ items and their links
    let c = fx.repo.join("claude");
    wr(&c.join("agents/a.md"), "---\nname: a\n---\nA.\n");
    wr(&c.join("hooks/rust-bin"), "# fake launcher\n");
    wr(&c.join("output-styles/ELI5.md"), "---\nname: ELI5\n---\nExplain it simply.\n");
    wr(&c.join("CLAUDE.md"), "# Core\n\ncore line\n");
    wr(
        &c.join("settings.json"),
        r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "test -r ~/.claude/hooks/rust-bin || exit 0; sh ~/.claude/hooks/rust-bin --hook guard pretool"}]}]}, "statusLine": {"type": "command", "command": "sh ~/.claude/hooks/rust-bin statusline"}}"#,
    );
    wr(
        &c.join("presets/workflow/rules.md"),
        "---\nname: workflow\ndescription: d\nguard: [G1]\n---\n# Workflow\n\nworkflow line\n",
    );
    wr(
        &c.join("presets/clock/rules.md"),
        "---\nname: clock\ndescription: d\nguard: [G11]\n---\n# Clock\n\nclock line\n",
    );
    fs::create_dir_all(fx.home.join(".claude")).unwrap();
    for d in ["agents", "hooks", "output-styles", "CLAUDE.md"] {
        symlink(&c.join(d), &fx.home.join(".claude").join(d));
    }
    let on = presets::enabled(&fx.home, &presets::profile(&fx.home));
    presets::link_all(&fx.repo, &fx.home, &on);
    // the Rust crate: sources first, bins after
    let crate_dir = fx.repo.join("rust/agent-config");
    wr(
        &crate_dir.join("Cargo.toml"),
        "[package]\nname = \"x\"\n\n[[bin]]\nname = \"alpha\"\npath = \"src/main.rs\"\n\n[[bin]]\nname = \"beta\"\npath = \"src/b.rs\"\n",
    );
    wr(&crate_dir.join("src/main.rs"), "fn main() {}\n");
    wr(&crate_dir.join("src/b.rs"), "fn main() {}\n");
    let old = SystemTime::now() - Duration::from_secs(600);
    for f in ["Cargo.toml", "src/main.rs", "src/b.rs"] {
        set_mtime(&crate_dir.join(f), old);
    }
    for b in ["alpha", "beta"] {
        wr(&crate_dir.join("target/release").join(exe(b)), "bin\n");
    }
    // generated files and the built settings
    fs::create_dir_all(fx.home.join(".cursor/rules")).unwrap();
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    assert_eq!(fx.ac(&["opencode", "--profile", "shared"]).0, 0);
    assert_eq!(fx.ac(&["cursor", "--profile", "shared"]).0, 0);
    // a clean tracked tree
    git(&fx, &["init", "-q"]);
    git(&fx, &["add", "-A"]);
    fx
}

fn assert_all_pass(fx: &Fx) {
    let bad: Vec<String> =
        fx.checks().into_iter().filter_map(|c| c.result.err().map(|e| format!("{}: {e}", c.name))).collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn doctor_all_pass_after_dots() {
    let fx = fixture("doctor-pass");
    let names: Vec<&str> = fx.checks().iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        ["links", "settings-built", "bins-fresh", "guard-selftest", "presets", "generated", "timezone", "no-leak"]
    );
    assert_all_pass(&fx);
    // the command prints one line per check and exits 0
    let (home, repo) = (fx.home.to_str().unwrap(), fx.repo.to_str().unwrap());
    let (code, out, err) = run_bin(DOTS, &["doctor"], "", &[("HOME", home), ("DOTFILES_REPO", repo)], None);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(out.lines().filter(|l| l.starts_with("PASS ")).count(), names.len(), "{out}");
    assert!(!out.contains("FAIL"), "{out}");
}

#[test]
fn doctor_broken_link_fails() {
    let fx = fixture("doctor-link");
    let item = links()[0];
    let dst = fx.home.join(item);
    fs::remove_file(&dst).unwrap();
    wr(&dst, "my own file\n"); // a regular file, not a link into the repo
    let why = fx.get("links").unwrap_err();
    assert!(why.contains(item), "{why}");
    assert!(why.contains("dots"), "the fix is named: {why}");
    // a Claude item points elsewhere
    let fx2 = fixture("doctor-link2");
    let hooks = fx2.home.join(".claude/hooks");
    fs::remove_file(&hooks).unwrap();
    fs::create_dir_all(&hooks).unwrap();
    assert!(fx2.get("links").unwrap_err().contains(".claude/hooks"));
    // the command fails with the FAIL line
    let (home, repo) = (fx.home.to_str().unwrap(), fx.repo.to_str().unwrap());
    let (code, out, _) = run_bin(DOTS, &["doctor"], "", &[("HOME", home), ("DOTFILES_REPO", repo)], None);
    assert_eq!(code, 1, "{out}");
    assert!(out.lines().any(|l| l.starts_with("FAIL links: ")), "{out}");
    assert!(out.lines().any(|l| l.starts_with("PASS settings-built")), "{out}");
}

#[test]
fn doctor_stale_bin_fails() {
    let fx = fixture("doctor-stale");
    let crate_dir = fx.repo.join("rust/agent-config");
    // a source newer than a bin
    set_mtime(&crate_dir.join("target/release").join(exe("beta")), SystemTime::now() - Duration::from_secs(3600));
    let why = fx.get("bins-fresh").unwrap_err();
    assert!(why.contains("beta") && !why.contains("alpha"), "{why}");
    assert!(why.contains("Run: dots") || why.contains("--bins"), "the fix is named: {why}");
    // a missing bin
    fs::remove_file(crate_dir.join("target/release").join(exe("alpha"))).unwrap();
    let why = fx.get("bins-fresh").unwrap_err();
    assert!(why.contains("alpha") && why.contains("beta"), "{why}");
}

#[test]
fn doctor_bad_timezone_fails() {
    let fx = fixture("doctor-tz");
    fx.setting("TIMEZONE=\"Nowhere/Bad_Zone\"\n");
    let tz = fx.get("timezone");
    if cfg!(windows) {
        assert!(tz.is_ok());
    } else {
        assert!(tz.unwrap_err().contains("Nowhere/Bad_Zone"));
    }
    // a path that climbs out of the zone folder is no zone
    fx.setting("TIMEZONE=\"../../etc/passwd\"\n");
    if !cfg!(windows) {
        assert!(fx.get("timezone").is_err());
    }
    // empty is fine; a real zone is fine where the zone files exist
    fx.setting("TIMEZONE=\"\"\n");
    assert!(fx.get("timezone").is_ok());
    if Path::new("/usr/share/zoneinfo/UTC").is_file() {
        fx.setting("TIMEZONE=\"UTC\"\n");
        assert!(fx.get("timezone").is_ok());
    }
}

#[test]
fn doctor_settings_without_rust_bin_fails() {
    let fx = fixture("doctor-settings");
    let live = fx.home.join(".claude/settings.json");
    let text = fs::read_to_string(&live).unwrap().replace("rust-bin", "other-bin");
    fs::write(&live, text).unwrap();
    assert!(fx.get("settings-built").unwrap_err().contains("rust-bin"));
    fs::remove_file(&live).unwrap();
    assert!(fx.get("settings-built").is_err());
}

#[test]
fn doctor_settings_ignores_other_hooks_but_needs_the_repo_ones() {
    let fx = fixture("doctor-own-hooks");
    let live = fx.home.join(".claude/settings.json");
    let mut v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&live).unwrap()).unwrap();
    // the user's own hooks, and the documented sample, next to the repo's
    let own = serde_json::json!([{"hooks": [
        {"type": "command", "command": "bash ~/.claude/hooks/format-after-edit.sh"},
        {"type": "command", "command": "/usr/local/bin/my-own-hook"}
    ]}]);
    v["hooks"]["PostToolUse"] = own;
    fs::write(&live, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    assert!(fx.get("settings-built").is_ok(), "{:?}", fx.get("settings-built"));
    // a repo hook that is gone fails, even when other hooks are there
    v["hooks"].as_object_mut().unwrap().remove("PreToolUse");
    fs::write(&live, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    let why = fx.get("settings-built").unwrap_err();
    assert!(why.contains("rust-bin") && why.contains("dots"), "{why}");
}

#[test]
fn doctor_presets_mismatch_fails() {
    let fx = fixture("doctor-presets");
    fx.setting("PRESETS=\"workflow\"\n"); // clock goes off, but its link stays
    let why = fx.get("presets").unwrap_err();
    assert!(why.contains("clock"), "{why}");
    // the setting says clock too, but its link is gone
    fx.setting("PRESETS=\"workflow clock\"\n");
    assert!(fx.get("presets").is_ok());
    fs::remove_file(fx.home.join(".claude/rules/preset-workflow.md")).unwrap();
    assert!(fx.get("presets").unwrap_err().contains("workflow"));
}

/// The repo is the same folder under another name (a symlink, a `..`, as `rust-bin` spells it).
#[cfg(unix)]
#[test]
fn doctor_presets_pass_when_the_repo_is_spelled_another_way() {
    let fx = fixture("doctor-presets-alias");
    let alias = fx.tmp.join("alias");
    std::os::unix::fs::symlink(&fx.repo, &alias).unwrap();
    let r = doctor::checks(&alias, &fx.home).into_iter().find(|c| c.name == "presets").unwrap().result;
    assert!(r.is_ok(), "{r:?}");
}

/// The links point into another checkout (a copy of the repo): the check still fails and names them.
#[cfg(unix)]
#[test]
fn doctor_presets_fail_when_the_links_point_into_another_repo() {
    let fx = fixture("doctor-presets-other");
    let other = fx.tmp.join("worktree");
    for n in ["workflow", "clock"] {
        wr(&other.join("claude/presets").join(n).join("rules.md"), "# copy\n");
    }
    let r = doctor::checks(&other, &fx.home).into_iter().find(|c| c.name == "presets").unwrap().result;
    let why = r.unwrap_err();
    assert!(why.contains("no link for") && why.contains("clock") && why.contains("workflow"), "{why}");
}

#[test]
fn doctor_generated_needs_the_marker() {
    let fx = fixture("doctor-generated");
    let agents = fx.home.join(".config/opencode/AGENTS.md");
    let good = fs::read_to_string(&agents).unwrap();
    fs::write(&agents, "my own rules\n").unwrap();
    assert!(fx.get("generated").unwrap_err().contains("AGENTS.md"));
    fs::write(&agents, good).unwrap();
    assert!(fx.get("generated").is_ok());
    let plugin = fx.home.join(".config/opencode/plugins/dots-guard.ts");
    assert!(plugin.is_file(), "the fixture must write the plugin");
    fs::write(&plugin, "// mine\n").unwrap();
    assert!(fx.get("generated").unwrap_err().contains("dots-guard.ts"));
    fs::remove_file(&plugin).unwrap();
    assert!(fx.get("generated").unwrap_err().contains("dots-guard.ts"));
    // the Cursor rule
    let fx = fixture("doctor-generated2");
    fs::write(fx.home.join(".cursor/rules/claude-global.mdc"), "mine\n").unwrap();
    assert!(fx.get("generated").unwrap_err().contains("claude-global.mdc"));
    // no Cursor and no Opencode config at all: nothing to check
    let fx = fixture("doctor-generated3");
    fs::remove_dir_all(fx.home.join(".cursor")).unwrap();
    fs::remove_dir_all(fx.home.join(".config/opencode")).unwrap();
    assert!(fx.get("generated").is_ok());
}

#[test]
fn doctor_no_leak_finds_a_private_word() {
    let fx = fixture("doctor-leak");
    fx.setting("PRIVATE_WORDS=\"zebraquux\"\n");
    assert!(fx.get("no-leak").is_ok());
    wr(&fx.repo.join("claude/note.md"), "ok\nthe ZebraQuux project\n");
    git(&fx, &["add", "-A"]);
    let why = fx.get("no-leak").unwrap_err();
    assert!(why.contains("claude/note.md:2"), "{why}");
    assert!(!why.to_lowercase().contains("zebraquux"), "the word itself is never printed: {why}");
}

#[test]
fn doctor_no_leak_matches_short_words_as_whole_words() {
    let fx = fixture("doctor-leak-short");
    fx.setting("PRIVATE_WORDS=\"abc\"\n");
    // a short word inside a longer word is no hit, in a line or in a file name
    wr(&fx.repo.join("notes/abcdef.md"), "the abcdef thing and xabc\n");
    git(&fx, &["add", "-A"]);
    assert!(fx.get("no-leak").is_ok(), "{:?}", fx.get("no-leak"));
    // the whole word is a hit, and a printed name masks it
    wr(&fx.repo.join("notes/x-abc-y.md"), "ok\nan abc here\n");
    git(&fx, &["add", "-A"]);
    let why = fx.get("no-leak").unwrap_err();
    assert!(why.contains("notes/x-***-y.md:2"), "{why}");
    assert!(why.contains("notes/x-***-y.md (file name)"), "{why}");
    assert!(!why.contains("abc"), "word printed: {why}");
    assert!(why.contains("2 place(s)"), "{why}");
    // the whole command output, not just the check text
    let (home, repo) = (fx.home.to_str().unwrap(), fx.repo.to_str().unwrap());
    let (code, out, err) = run_bin(DOTS, &["doctor"], "", &[("HOME", home), ("DOTFILES_REPO", repo)], None);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.lines().any(|l| l.starts_with("FAIL no-leak: ")), "{out}");
    assert!(!format!("{out}{err}").contains("abc"), "word printed: {out}{err}");
}

#[test]
fn doctor_guard_selftest_passes_without_settings() {
    // canned inputs and a canned private word: it needs no setting of the machine
    let fx = fixture("doctor-guard");
    assert!(fx.get("guard-selftest").is_ok());
}
