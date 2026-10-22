//! G18 (preset workflow): the third try of a step that failed twice is denied, and G19 (preset
//! workflow): an unattended run that edited a project is held back once until a check passed. Runs
//! the `guard` bin on a temp git repo that has a `check:fast` script.

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
    /// A repo whose project states a fast check, unless `fast` is false.
    fn with(fast: bool) -> Self {
        let base = temp_home("stuck");
        let repo = base.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        let scripts = if fast { r#"{"scripts": {"check:fast": "true"}}"# } else { "{}" };
        fs::write(repo.join("package.json"), scripts).unwrap();
        fs::write(repo.join("src/a.txt"), "one\n").unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "init"]);
        let n = SID.fetch_add(1, Ordering::SeqCst);
        Self { repo, tmp: base.join("tmp"), sid: format!("stuck-{}-{n}", std::process::id()) }
    }

    fn new() -> Self {
        Self::with(true)
    }

    fn run(&self, mode: &str, event: &Value, env: &[(&str, &str)]) -> (i32, String, String) {
        let tmp = self.tmp.to_string_lossy().into_owned();
        let home = self.tmp.join("home").to_string_lossy().into_owned();
        let mut e = vec![("TMPDIR", tmp.as_str()), ("HOME", home.as_str())];
        e.extend_from_slice(env);
        fs::create_dir_all(self.tmp.join("home")).unwrap();
        run_bin(GUARD, &[mode], &event.to_string(), &e, Some(&self.repo))
    }

    fn base(&self) -> Value {
        json!({"session_id": self.sid, "cwd": self.repo})
    }

    fn bash(&self, cmd: &str) -> Value {
        let mut ev = self.base();
        ev["tool_name"] = json!("Bash");
        ev["tool_input"] = json!({"command": cmd});
        ev
    }

    fn edit(&self, file: &Path, old: &str) -> Value {
        let mut ev = self.base();
        ev["tool_name"] = json!("Edit");
        ev["tool_input"] = json!({"file_path": file, "old_string": old, "new_string": "x"});
        ev
    }

    fn as_helper(mut ev: Value, id: &str) -> Value {
        ev["agent_id"] = json!(id);
        ev
    }

    /// The call failed (PostToolUseFailure): the hook output, if any.
    fn failed(&self, ev: &Value) -> String {
        let (code, out, _) = self.run("posttoolfailure", ev, &[]);
        assert_eq!(code, 0);
        out
    }

    fn passed(&self, ev: &Value) {
        let mut ev = ev.clone();
        ev["tool_response"] = json!({"stdout": "ok", "interrupted": false});
        let (code, _, _) = self.run("posttool", &ev, &[]);
        assert_eq!(code, 0);
    }

    /// The call is about to run: the exit code and the deny text.
    fn pre(&self, ev: &Value) -> (i32, String) {
        self.pre_env(ev, &[])
    }

    fn pre_env(&self, ev: &Value, env: &[(&str, &str)]) -> (i32, String) {
        let (code, _, err) = self.run("pretool", ev, env);
        (code, err)
    }

    fn denied(&self, ev: &Value) -> bool {
        let (code, err) = self.pre(ev);
        if code == 2 {
            assert!(err.contains("stop and report: 2 failed tries on this step"), "{err}");
        }
        code == 2
    }

    /// A transcript that shows the autopilot skill was started, or a plain chat.
    fn transcript(&self, autopilot: bool) -> PathBuf {
        let path = self.tmp.join(if autopilot { "auto.jsonl" } else { "chat.jsonl" });
        let mut lines = vec![json!({"type": "user", "message": {"role": "user", "content": "go"}})];
        if autopilot {
            lines.push(json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "Skill", "input": {"skill": "ps:autopilot"}}]}}));
        }
        lines.push(json!({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "text", "text": "All tasks are done."}]}}));
        let body: Vec<String> = lines.iter().map(Value::to_string).collect();
        fs::create_dir_all(&self.tmp).unwrap();
        fs::write(&path, body.join("\n") + "\n").unwrap();
        path
    }

    /// The main thread stops.
    fn stop(&self, transcript: &Path, again: bool) -> (i32, String) {
        let mut ev = self.base();
        ev["transcript_path"] = json!(transcript);
        ev["stop_hook_active"] = json!(again);
        ev["last_assistant_message"] = json!("All tasks are done.");
        let (code, _, err) = self.run("stop", &ev, &[]);
        (code, err)
    }

    /// The helper `id` stops.
    fn helper_stop(&self, id: &str, again: bool, env: &[(&str, &str)]) -> (i32, String) {
        let mut ev = self.base();
        ev["agent_id"] = json!(id);
        ev["stop_hook_active"] = json!(again);
        ev["last_assistant_message"] = json!("Done.");
        let (code, _, err) = self.run("subagentstop", &ev, env);
        (code, err)
    }

    fn src(&self) -> PathBuf {
        self.repo.join("src/a.txt")
    }
}

// --- G18 ---

#[test]
fn the_same_command_fails_twice_then_the_third_try_is_denied() {
    let w = W::new();
    let ev = w.bash("git push origin main");
    assert!(!w.denied(&ev), "nothing failed yet");
    assert_eq!(w.failed(&ev), "", "one failure is quiet");
    assert!(!w.denied(&ev), "one failure allows the second try");
    let note = w.failed(&ev);
    assert!(note.contains("G18") && note.contains("stop and report"), "the second failure warns: {note}");
    assert!(w.denied(&ev), "the third try is denied");
    assert!(w.denied(&ev), "and it stays denied");
    assert!(!w.denied(&w.bash("git status")), "another command is fine");
}

#[test]
fn an_edit_in_between_makes_it_a_new_try() {
    let w = W::new();
    let ev = w.bash("npm test");
    w.failed(&ev);
    fs::write(w.src(), "two\n").unwrap();
    assert_eq!(w.failed(&ev), "", "the tree changed, so this is the first failure again");
    assert!(!w.denied(&ev));
    assert!(!w.failed(&ev).is_empty(), "no change since: the second failure of this try");
    assert!(w.denied(&ev));
    fs::write(w.src(), "three\n").unwrap();
    assert!(!w.denied(&ev), "an edit after the stop lets it run again");
}

#[test]
fn a_pass_clears_the_count() {
    let w = W::new();
    let ev = w.bash("make build");
    w.failed(&ev);
    w.failed(&ev);
    assert!(w.denied(&ev));
    w.passed(&ev);
    assert!(!w.denied(&ev));
    assert_eq!(w.failed(&ev), "", "the count starts at one");
}

#[test]
fn a_new_user_message_clears_the_count() {
    let w = W::new();
    let ev = w.bash("make build");
    w.failed(&ev);
    w.failed(&ev);
    assert!(w.denied(&ev));
    let mut p = w.base();
    p["hook_event_name"] = json!("UserPromptSubmit");
    p["prompt"] = json!("try again");
    let (code, _, _) = w.run("prompt", &p, &[]);
    assert_eq!(code, 0);
    assert!(!w.denied(&ev));
}

#[test]
fn an_interrupted_call_is_no_failed_try() {
    let w = W::new();
    let ev = w.bash("sleep 99");
    for _ in 0..3 {
        let mut f = ev.clone();
        f["is_interrupt"] = json!(true);
        assert_eq!(w.failed(&f), "");
    }
    assert!(!w.denied(&ev));
}

#[test]
fn the_same_edit_target_counts_and_another_does_not() {
    let w = W::new();
    let ev = w.edit(&w.src(), "one");
    w.failed(&ev);
    w.failed(&ev);
    assert!(w.denied(&ev), "same file, same old text");
    assert!(!w.denied(&w.edit(&w.src(), "other text")), "same file, another target");
    assert!(!w.denied(&w.edit(&w.repo.join("src/b.txt"), "one")), "another file");
}

#[test]
fn a_helper_has_its_own_tries() {
    let w = W::new();
    let ev = w.bash("git push");
    w.failed(&ev);
    w.failed(&ev);
    assert!(w.denied(&ev));
    assert!(!w.denied(&W::as_helper(ev.clone(), "h1")), "a helper starts fresh");
    let other = W::as_helper(ev, "h2");
    w.failed(&other);
    assert!(!w.denied(&other), "one failure of another helper is not two");
}

#[test]
fn g18_guard_off_turns_it_off() {
    let w = W::new();
    let ev = w.bash("git push");
    w.failed(&ev);
    w.failed(&ev);
    assert_eq!(w.pre_env(&ev, &[("GUARD_OFF", "G18")]).0, 0);
    assert_eq!(w.pre(&ev).0, 2);
}

#[test]
fn g18_follows_its_preset() {
    let w = W::new();
    let ev = w.bash("git push");
    w.failed(&ev);
    w.failed(&ev);
    fs::create_dir_all(w.tmp.join("home/.config/dotfiles")).unwrap();
    fs::write(w.tmp.join("home/.config/dotfiles/settings"), "PRESETS=\"models\"\n").unwrap();
    assert_eq!(w.pre(&ev).0, 0, "the workflow preset is off");
}

#[test]
fn garbage_input_fails_open() {
    for mode in ["posttoolfailure", "pretool", "posttool", "subagentstop", "stop"] {
        let (code, out, _) = run_bin(GUARD, &[mode], "{\"tool_name\": \"Edit\", \"tool_input\": 5}", &[], None);
        assert_eq!((code, out.as_str()), (0, ""), "{mode}");
    }
}

// --- G19 ---

#[test]
fn a_helper_that_edited_and_never_checked_is_held_back_once() {
    let w = W::new();
    w.passed(&W::as_helper(w.edit(&w.src(), "one"), "h1"));
    let (code, err) = w.helper_stop("h1", false, &[]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("run the fast check before you stop") && err.contains("npm run check:fast"), "{err}");
    assert_eq!(w.helper_stop("h1", true, &[]).0, 0, "the second stop goes through");
    assert_eq!(w.helper_stop("h1", false, &[]).0, 0, "even without the stop hook flag");
}

#[test]
fn a_passing_check_after_the_edit_lets_the_helper_stop() {
    let w = W::new();
    w.passed(&W::as_helper(w.edit(&w.src(), "one"), "h1"));
    w.passed(&W::as_helper(w.bash("npm test"), "h1"));
    assert_eq!(w.helper_stop("h1", false, &[]).0, 0);
    w.passed(&W::as_helper(w.edit(&w.src(), "two"), "h1"));
    assert_eq!(w.helper_stop("h1", false, &[]).0, 2, "a later edit needs a new pass");
    assert_eq!(w.helper_stop("h1", true, &[]).0, 0);
}

#[test]
fn a_check_that_is_not_a_plain_check_is_no_proof() {
    let w = W::new();
    w.passed(&W::as_helper(w.edit(&w.src(), "one"), "h1"));
    w.passed(&W::as_helper(w.bash("npm test | tail -3"), "h1")); // a pipe hides the exit code
    w.passed(&W::as_helper(w.bash("ls"), "h1"));
    assert_eq!(w.helper_stop("h1", false, &[]).0, 2);
}

#[test]
fn a_helper_that_edited_nothing_in_a_project_is_not_held_back() {
    let w = W::new();
    assert_eq!(w.helper_stop("h1", false, &[]).0, 0, "no edits at all");
    let outside = temp_home("stuck-outside").join("notes.md");
    w.passed(&W::as_helper(w.edit(&outside, "a"), "h1"));
    assert_eq!(w.helper_stop("h1", false, &[]).0, 0, "a file outside any project");
    // the edit of one helper does not hold back another
    w.passed(&W::as_helper(w.edit(&w.src(), "one"), "h2"));
    assert_eq!(w.helper_stop("h1", false, &[]).0, 0);
    assert_eq!(w.helper_stop("h2", false, &[]).0, 2);
}

#[test]
fn a_project_without_a_fast_check_is_not_held_back() {
    let w = W::with(false);
    w.passed(&W::as_helper(w.edit(&w.src(), "one"), "h1"));
    assert_eq!(w.helper_stop("h1", false, &[]).0, 0);
}

#[test]
fn g19_guard_off_turns_it_off() {
    let w = W::new();
    w.passed(&W::as_helper(w.edit(&w.src(), "one"), "h1"));
    assert_eq!(w.helper_stop("h1", false, &[("GUARD_OFF", "G19")]).0, 0);
    assert_eq!(w.helper_stop("h1", false, &[]).0, 2, "the stop was not marked while it was off");
}

#[test]
fn an_autopilot_session_is_held_back_once_on_stop() {
    let w = W::new();
    let t = w.transcript(true);
    w.passed(&w.edit(&w.src(), "one"));
    let (code, err) = w.stop(&t, false);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("run the fast check before you stop"), "{err}");
    assert_eq!(w.stop(&t, true).0, 0, "never twice in a row");
    assert_eq!(w.stop(&t, false).0, 0, "also when the flag is missing");
    // a new edit is held back again, once
    w.passed(&w.edit(&w.src(), "two"));
    assert_eq!(w.stop(&t, false).0, 2);
    assert_eq!(w.stop(&t, true).0, 0);
}

#[test]
fn an_autopilot_session_that_checked_may_stop() {
    let w = W::new();
    let t = w.transcript(true);
    w.passed(&w.edit(&w.src(), "one"));
    w.passed(&w.bash("cargo test"));
    assert_eq!(w.stop(&t, false).0, 0);
}

#[test]
fn a_normal_chat_is_never_held_back() {
    let w = W::new();
    let t = w.transcript(false);
    w.passed(&w.edit(&w.src(), "one"));
    assert_eq!(w.stop(&t, false).0, 0);
    assert_eq!(w.stop(&t, false).0, 0);
}

#[test]
fn the_helper_hold_back_does_not_free_the_slot_but_the_second_stop_does() {
    let w = W::new();
    let spawn = |id: &str| {
        let mut ev = w.base();
        ev["tool_name"] = json!("Agent");
        ev["tool_use_id"] = json!(id);
        ev["tool_input"] = json!({"description": "d", "subagent_type": "worker", "prompt": "Deadline 10 min. Hand back at most 4000 chars."});
        ev
    };
    let (code, _) = w.pre(&spawn("a1"));
    assert_eq!(code, 0);
    w.passed(&W::as_helper(w.edit(&w.src(), "one"), "h1"));
    assert_eq!(w.helper_stop("h1", false, &[]).0, 2);
    let state = |w: &W| -> Value {
        let p = w.tmp.join("claude-guard").join(format!("{}.json", w.sid));
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    };
    assert_eq!(state(&w)["spawns"].as_array().map(Vec::len), Some(1), "the slot is still taken");
    assert_eq!(w.helper_stop("h1", true, &[]).0, 0);
    assert_eq!(state(&w)["spawns"].as_array().map(Vec::len), Some(0), "the slot is free");
}
