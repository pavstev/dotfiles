//! `dots doctor`: each check passes on a fresh setup and fails, with a reason, when its part is broken.
//! A fake repo and a temp HOME hold everything; nothing reads the real home of the machine.

#![allow(clippy::unwrap_used, reason = "a test fails by panicking")]
mod common;

use agent_config::dots::{doctor, links};
use common::{run_bin, temp_home, wr};
use std::fs;
use std::path::{Path, PathBuf};
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

fn set_mtime(p: &Path, t: SystemTime) {
    fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
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
    common::git_env(&fx.repo, &[("HOME", fx.home.to_str().unwrap())], args);
}

impl Fx {
    fn ac(&self, args: &[&str]) -> (i32, String, String) {
        let (home, repo) = (self.home.to_str().unwrap(), self.repo.to_str().unwrap());
        run_bin(
            AC,
            args,
            "",
            &[("HOME", home), ("DOTFILES_REPO", repo)],
            None,
        )
    }

    fn checks(&self) -> Vec<doctor::Check> {
        doctor::checks(&self.repo, &self.home)
    }

    fn get(&self, name: &str) -> Result<(), String> {
        self.checks()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no check {name}"))
            .result
    }

    fn setting(&self, text: &str) {
        wr(&self.home.join(".config/dotfiles/settings"), text);
    }
}

/// What `dots` leaves behind: the links, the built settings with the ps plugin on, the plugin's
/// binaries, a repo with its bins built after its sources.
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
    // the base settings, and the ps plugin with its binaries (its first session fetches them)
    wr(
        &fx.repo.join("claude/settings.json"),
        r#"{"theme": "dark"}"#,
    );
    let bin = fx.home.join(".claude/plugins/data/ps-agent-kit/bin");
    wr(&bin.join("rust-bin"), "# fake launcher\n");
    wr(&bin.join(exe("agent-config")), "bin\n");
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
    // the built settings, with the plugin on (the `claude plugin install` of `dots` adds it)
    wr(
        &fx.home.join(".config/dotfiles/claude-settings.json"),
        r#"{"enabledPlugins": {"ps@agent-kit": true}}"#,
    );
    assert_eq!(fx.ac(&["claude-settings", "build"]).0, 0);
    // a clean tracked tree
    git(&fx, &["init", "-q"]);
    git(&fx, &["add", "-A"]);
    fx
}

fn assert_all_pass(fx: &Fx) {
    let bad: Vec<String> = fx
        .checks()
        .into_iter()
        .filter_map(|c| c.result.err().map(|e| format!("{}: {e}", c.name)))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn doctor_all_pass_after_dots() {
    let fx = fixture("doctor-pass");
    let names: Vec<&str> = fx.checks().iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        [
            "links",
            "settings-built",
            "bins-fresh",
            "plugin",
            "timezone",
            "no-leak"
        ]
    );
    assert_all_pass(&fx);
    // the command prints one line per check and exits 0
    let (home, repo) = (fx.home.to_str().unwrap(), fx.repo.to_str().unwrap());
    let (code, out, err) = run_bin(
        DOTS,
        &["doctor"],
        "",
        &[("HOME", home), ("DOTFILES_REPO", repo)],
        None,
    );
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        out.lines().filter(|l| l.starts_with("PASS ")).count(),
        names.len(),
        "{out}"
    );
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
    // the command fails with the FAIL line
    let (home, repo) = (fx.home.to_str().unwrap(), fx.repo.to_str().unwrap());
    let (code, out, _) = run_bin(
        DOTS,
        &["doctor"],
        "",
        &[("HOME", home), ("DOTFILES_REPO", repo)],
        None,
    );
    assert_eq!(code, 1, "{out}");
    assert!(out.lines().any(|l| l.starts_with("FAIL links: ")), "{out}");
    assert!(
        out.lines().any(|l| l.starts_with("PASS settings-built")),
        "{out}"
    );
}

#[test]
fn doctor_stale_bin_fails() {
    let fx = fixture("doctor-stale");
    let crate_dir = fx.repo.join("rust/agent-config");
    // a source newer than a bin
    set_mtime(
        &crate_dir.join("target/release").join(exe("beta")),
        SystemTime::now() - Duration::from_secs(3600),
    );
    let why = fx.get("bins-fresh").unwrap_err();
    assert!(why.contains("beta") && !why.contains("alpha"), "{why}");
    assert!(
        why.contains("Run: dots") || why.contains("--bins"),
        "the fix is named: {why}"
    );
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
fn doctor_no_leak_finds_a_private_word() {
    let fx = fixture("doctor-leak");
    fx.setting("PRIVATE_WORDS=\"zebraquux\"\n");
    assert!(fx.get("no-leak").is_ok());
    wr(
        &fx.repo.join("claude/note.md"),
        "ok\nthe ZebraQuux project\n",
    );
    git(&fx, &["add", "-A"]);
    let why = fx.get("no-leak").unwrap_err();
    assert!(why.contains("claude/note.md:2"), "{why}");
    assert!(
        !why.to_lowercase().contains("zebraquux"),
        "the word itself is never printed: {why}"
    );
}

#[test]
fn doctor_no_leak_matches_short_words_as_whole_words() {
    let fx = fixture("doctor-leak-short");
    fx.setting("PRIVATE_WORDS=\"abc\"\n");
    // a short word inside a longer word is no hit, in a line or in a file name
    wr(
        &fx.repo.join("notes/abcdef.md"),
        "the abcdef thing and xabc\n",
    );
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
    let (code, out, err) = run_bin(
        DOTS,
        &["doctor"],
        "",
        &[("HOME", home), ("DOTFILES_REPO", repo)],
        None,
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        out.lines().any(|l| l.starts_with("FAIL no-leak: ")),
        "{out}"
    );
    assert!(
        !format!("{out}{err}").contains("abc"),
        "word printed: {out}{err}"
    );
}

#[test]
fn doctor_no_leak_skips_public_hosts() {
    let fx = fixture("doctor-leak-hosts");
    fx.setting("PRIVATE_WORDS=\"zebrafish\"\n");
    wr(
        &fx.repo.join("claude/link.md"),
        "see https://zebrafish.example/story\n",
    );
    git(&fx, &["add", "-A"]);
    // without the host listed the line is a leak
    let why = fx.get("no-leak").unwrap_err();
    assert!(why.contains("claude/link.md:1"), "{why}");
    // with the host listed it passes, a hostile entry is ignored, the bare word still fails
    fx.setting("PRIVATE_WORDS=\"zebrafish\"\nPUBLIC_HOSTS=\"a|b, ZebraFish.example\"\n");
    assert!(fx.get("no-leak").is_ok(), "{:?}", fx.get("no-leak"));
    wr(&fx.repo.join("claude/bare.md"), "a zebrafish here\n");
    git(&fx, &["add", "-A"]);
    let why = fx.get("no-leak").unwrap_err();
    assert!(
        why.contains("claude/bare.md:1") && !why.contains("link.md"),
        "{why}"
    );
}

#[test]
fn doctor_settings_need_the_plugin_and_no_old_guard_hooks() {
    let fx = fixture("doctor-plugin-settings");
    let live = fx.home.join(".claude/settings.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&live).unwrap()).unwrap();
    v["hooks"] = serde_json::json!({"PreToolUse": [{"hooks": [
        {"type": "command", "command": "sh ~/.claude/hooks/rust-bin --hook guard pretool"}
    ]}]});
    fs::write(&live, v.to_string()).unwrap();
    assert!(fx.get("settings-built").unwrap_err().contains("old guard"));
    v.as_object_mut().unwrap().remove("hooks");
    v["enabledPlugins"]["ps@agent-kit"] = serde_json::json!(false);
    fs::write(&live, v.to_string()).unwrap();
    assert!(
        fx.get("settings-built")
            .unwrap_err()
            .contains("ps@agent-kit")
    );
}

#[test]
fn doctor_plugin_needs_its_binaries_and_old_links_fail() {
    let fx = fixture("doctor-plugin-bins");
    let bin = fx.home.join(".claude/plugins/data/ps-agent-kit/bin");
    fs::remove_file(bin.join(exe("agent-config"))).unwrap();
    assert!(
        fx.get("plugin")
            .unwrap_err()
            .contains("Start Claude Code once")
    );
    symlink(
        &fx.repo.join("claude/hooks"),
        &fx.home.join(".claude/hooks"),
    );
    assert!(
        fx.get("links")
            .unwrap_err()
            .contains("~/.claude/hooks (an old link)")
    );
}
