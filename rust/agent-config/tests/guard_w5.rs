//! W5 (preset workflow): a plain check command that already passed in this session, with the git work
//! tree unchanged since, gets a note to skip it. Warn only. Runs the `guard` bin on a temp git repo.

mod common;

use common::{run_bin, temp_home};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const GUARD: &str = env!("CARGO_BIN_EXE_guard");
static SID: AtomicUsize = AtomicUsize::new(0);

struct W {
    repo: PathBuf,
    tmp: PathBuf,
    sid: String,
}

fn git(repo: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(st.status.success(), "{args:?}: {}", String::from_utf8_lossy(&st.stderr));
}

impl W {
    fn new() -> Self {
        let base = temp_home("w5");
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "init"]);
        let n = SID.fetch_add(1, Ordering::SeqCst);
        Self { repo, tmp: base.join("tmp"), sid: format!("w5-{}-{n}", std::process::id()) }
    }

    fn run(&self, mode: &str, event: Value, env: &[(&str, &str)]) -> (i32, String, String) {
        let tmp = self.tmp.to_string_lossy().into_owned();
        let home = self.tmp.join("home").to_string_lossy().into_owned();
        let mut e = vec![("TMPDIR", tmp.as_str()), ("HOME", home.as_str())];
        e.extend_from_slice(env);
        fs::create_dir_all(&self.tmp).unwrap();
        fs::create_dir_all(self.tmp.join("home")).unwrap();
        run_bin(GUARD, &[mode], &event.to_string(), &e, Some(&self.repo))
    }

    fn event(&self, cmd: &str, cwd: &Path) -> Value {
        json!({"session_id": self.sid, "cwd": cwd, "tool_name": "Bash", "tool_input": {"command": cmd}})
    }

    /// The Bash call is about to run: the W5 note, if any.
    fn pre(&self, cmd: &str) -> Option<String> {
        self.pre_in(cmd, &self.repo, &[])
    }

    fn pre_in(&self, cmd: &str, cwd: &Path, env: &[(&str, &str)]) -> Option<String> {
        let (code, out, err) = self.run("pretool", self.event(cmd, cwd), env);
        assert_eq!((code, err.as_str()), (0, ""), "W5 never denies: {out}");
        if out.trim().is_empty() {
            return None;
        }
        let v: Value = serde_json::from_str(&out).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or("").to_string();
        ctx.contains("W5").then_some(ctx)
    }

    /// The Bash call ended with success (PostToolUse).
    fn passed(&self, cmd: &str) {
        self.passed_in(cmd, &self.repo, json!({"stdout": "ok", "stderr": "", "interrupted": false}));
    }

    fn passed_in(&self, cmd: &str, cwd: &Path, response: Value) {
        let mut ev = self.event(cmd, cwd);
        ev["hook_event_name"] = json!("PostToolUse");
        ev["tool_response"] = response;
        let (code, _, _) = self.run("posttool", ev, &[]);
        assert_eq!(code, 0);
    }
}

#[test]
fn a_pass_then_the_same_check_warns() {
    let w = W::new();
    assert!(w.pre("pnpm check").is_none(), "nothing ran yet");
    w.passed("pnpm check");
    let note = w.pre("pnpm check").expect("a repeat warns");
    assert!(note.contains("Skip it"), "{note}");
}

#[test]
fn an_edit_in_between_clears_it() {
    let w = W::new();
    w.passed("cargo test");
    assert!(w.pre("cargo test").is_some());
    fs::write(w.repo.join("a.txt"), "two\n").unwrap();
    assert!(w.pre("cargo test").is_none(), "a tracked file changed");
    w.passed("cargo test");
    assert!(w.pre("cargo test").is_some());
    fs::write(w.repo.join("b.txt"), "new\n").unwrap();
    assert!(w.pre("cargo test").is_none(), "a new file");
    w.passed("cargo test");
    fs::write(w.repo.join("b.txt"), "newer content\n").unwrap();
    assert!(w.pre("cargo test").is_none(), "an untracked file changed again");
    w.passed("cargo test");
    git(&w.repo, &["add", "."]);
    git(&w.repo, &["commit", "-qm", "more"]);
    assert!(w.pre("cargo test").is_none(), "a commit");
}

#[test]
fn a_failed_run_does_not_count() {
    let w = W::new();
    // a failed Bash goes to PostToolUseFailure, not PostToolUse
    let mut ev = w.event("pnpm check", &w.repo);
    ev["hook_event_name"] = json!("PostToolUseFailure");
    ev["error"] = json!("Exit code 1");
    w.run("posttoolfailure", ev, &[]);
    assert!(w.pre("pnpm check").is_none());
    // an interrupted run does not count either
    w.passed_in("pnpm check", &w.repo, json!({"stdout": "", "stderr": "", "interrupted": true}));
    assert!(w.pre("pnpm check").is_none());
}

#[test]
fn a_different_command_does_not_count() {
    let w = W::new();
    w.passed("pnpm check");
    for other in ["pnpm test", "pnpm check:fast", "pnpm run check --staged", "cargo test", "ls"] {
        assert!(w.pre(other).is_none(), "{other}");
    }
    // spaces do not matter
    assert!(w.pre("pnpm   check").is_some());
}

#[test]
fn a_pipe_hides_the_exit_code_so_it_is_never_recorded() {
    let w = W::new();
    w.passed("pnpm check | tail -5");
    w.passed("pnpm check; echo done");
    w.passed("pnpm check || true");
    assert!(w.pre("pnpm check").is_none());
    assert!(w.pre("pnpm check | tail -5").is_none());
}

#[test]
fn a_cd_step_and_an_env_prefix_are_fine() {
    let w = W::new();
    w.passed("cd app && CI=1 pnpm run test");
    assert!(w.pre("cd app && CI=1 pnpm run test").is_some());
    assert!(w.pre("pnpm run test").is_none(), "another shape of the same command");
}

#[test]
fn another_session_has_its_own_table() {
    let w = W::new();
    w.passed("pnpm check");
    let mut ev = w.event("pnpm check", &w.repo);
    ev["session_id"] = json!(format!("{}-other", w.sid));
    let (code, out, _) = w.run("pretool", ev, &[]);
    assert_eq!(code, 0);
    assert!(!out.contains("W5"), "{out}");
}

#[test]
fn outside_git_it_says_nothing() {
    let w = W::new();
    let plain = temp_home("w5-nogit");
    w.passed_in("pnpm check", &plain, json!({"interrupted": false}));
    assert!(w.pre_in("pnpm check", &plain, &[]).is_none());
}

#[test]
fn guard_off_turns_it_off() {
    let w = W::new();
    w.passed("pnpm check");
    assert!(w.pre_in("pnpm check", &w.repo, &[("GUARD_OFF", "W5")]).is_none());
    assert!(w.pre("pnpm check").is_some());
}

#[test]
fn garbage_input_fails_open() {
    let (code, out, _) = run_bin(GUARD, &["posttool"], "{\"tool_name\": \"Bash\", \"tool_input\": 5}", &[], None);
    assert_eq!((code, out.as_str()), (0, ""));
    let (code, out, _) = run_bin(
        GUARD,
        &["pretool"],
        "{\"tool_name\": \"Bash\", \"tool_input\": {\"command\": \"pnpm check\"}, \"cwd\": \"/nonexistent\"}",
        &[],
        None,
    );
    assert_eq!((code, out.as_str()), (0, ""));
}
