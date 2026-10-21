//! statusline: the Claude Code status line, copper themed. Reads the session JSON on stdin.
//! Replaces claude/statusline.sh and needs no `jq`.
//!
//! Prints: `<dir>  <git branch>  <model>  ctx <K> <%>  5h <%> · 7d <%>`
//! The branch is read from the `HEAD` file, with no git process, in the usual case; `git branch
//! --show-current` runs only when the repo is not plain (git env vars, reftable, odd layouts).
//! Context turns copper at 300 K: time to /ps:handoff. Rate limits are absent until the first API
//! reply; then nothing is printed for them. Writes the pace cache `claude-pace.json` (read by the
//! pace hook) when the weekly limit is present. Ends with `▸ 2 left: <first open step>` when the
//! guard's Stop pass left a status file for the session (`agent_config::status_path`). `NO_COLOR` set (non-empty) or `THEME=plain` in the
//! dotfiles settings prints the same text without escape codes.

use serde_json::{Map, Value};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const COPPER: &str = "\x1b[38;2;224;130;95m"; // #E0825F
const SAGE: &str = "\x1b[38;2;135;176;131m"; // #87B083
const STONE: &str = "\x1b[38;2;168;162;158m"; // #A8A29E
const RST: &str = "\x1b[0m";
/// Context size (in K) from which the segment turns copper.
const CTX_WARN_K: i64 = 300;

use agent_config::paths::home as home_dir;

/// `jq -r "<path> // empty"`: nothing for null, false or a missing key; strings as they are;
/// anything else as JSON text.
fn jq_str(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |cur, k| cur.get(*k))
}

/// A number the way `printf '%.0f'` rounds it (half to even).
fn int_text(v: Option<&Value>) -> Option<String> {
    let n = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    Some(format!("{:.0}", n.round_ties_even()))
}

/// The parts of the session JSON that the line shows, read once.
#[derive(Default)]
struct Input {
    /// `workspace.current_dir`, else `cwd`; empty when neither is set.
    cwd: String,
    model: String,
    /// Context size in tokens.
    ctx_tokens: Option<f64>,
    ctx_percent: Option<String>,
    five_hour_percent: Option<String>,
    seven_day_percent: Option<String>,
    session_id: String,
}

impl Input {
    fn from_json(v: &Value) -> Self {
        let text = |path: &[&str]| jq_str(at(v, path)).unwrap_or_default();
        let cwd = Some(text(&["workspace", "current_dir"])).filter(|s| !s.is_empty()).unwrap_or_else(|| text(&["cwd"]));
        Self {
            cwd,
            model: text(&["model", "display_name"]),
            ctx_tokens: ctx_tokens(v),
            ctx_percent: int_text(at(v, &["context_window", "used_percentage"])),
            five_hour_percent: int_text(at(v, &["rate_limits", "five_hour", "used_percentage"])),
            seven_day_percent: int_text(at(v, &["rate_limits", "seven_day", "used_percentage"])),
            session_id: text(&["session_id"]),
        }
    }
}

/// The last call's input plus cache tokens (`total_input_tokens` is a session running total).
fn ctx_tokens(input: &Value) -> Option<f64> {
    let from_usage = at(input, &["context_window", "current_usage"]).and_then(|u| {
        let u = u.as_object()?;
        let mut sum = 0.0;
        let mut any = false;
        for key in ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"] {
            match u.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::Number(n)) => {
                    sum += n.as_f64()?;
                    any = true;
                }
                Some(_) => return None, // jq would fail on the addition
            }
        }
        any.then_some(sum)
    });
    match from_usage {
        Some(t) if t != 0.0 => Some(t),
        _ => match at(input, &["context_window", "total_input_tokens"]) {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(s)) => s.trim().parse().ok(),
            _ => None,
        },
    }
}

fn basename(path: &str) -> String {
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let trimmed = path.trim_end_matches(is_sep);
    if trimmed.is_empty() {
        return if path.is_empty() { String::new() } else { "/".to_string() };
    }
    trimmed.rsplit(is_sep).next().unwrap_or(trimmed).to_string()
}

fn git_branch(cwd: &str) -> String {
    if let Some(answer) = fast_branch(cwd) {
        return answer;
    }
    Command::new("git")
        .args(["-C", cwd, "branch", "--show-current"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end_matches(['\n', '\r']).to_string())
        .unwrap_or_default()
}

/// Env vars that change how git finds the repo. With one set, only git itself can answer.
const GIT_ENV: [&str; 5] =
    ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM"];

/// The branch without a git process, or `None` when git must answer.
fn fast_branch(cwd: &str) -> Option<String> {
    if GIT_ENV.iter().any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty())) {
        return None;
    }
    branch_from_head(Path::new(cwd))
}

/// Find the git dir the way git does for the plain cases and read its HEAD. `Some(answer)` only
/// when sure; `None` for anything odd (the caller then runs git). Answers `""` for no repo, a
/// detached HEAD, or a HEAD outside `refs/heads`, as `git branch --show-current` prints nothing.
///
/// Known differences from git: git refuses a repo owned by another user (safe.directory) and
/// prints nothing, while this prints the branch. A branch that is itself a symref is not followed.
fn branch_from_head(cwd: &Path) -> Option<String> {
    let cwd = cwd.canonicalize().ok()?;
    if cwd.components().any(|c| c.as_os_str() == ".git") {
        return None;
    }
    #[cfg(unix)]
    let dev = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&cwd).ok()?.dev()
    };
    for dir in cwd.ancestors() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if std::fs::metadata(dir).ok()?.dev() != dev {
                return None; // git stops at a filesystem boundary
            }
        }
        let dot_git = dir.join(".git");
        let gitdir = match std::fs::metadata(&dot_git) {
            Ok(m) if m.is_dir() => dot_git,
            Ok(m) if m.is_file() => {
                let text = std::fs::read_to_string(&dot_git).ok()?;
                let path = text.lines().next()?.strip_prefix("gitdir:")?.trim();
                if path.is_empty() {
                    return None;
                }
                dir.join(path) // an absolute path replaces `dir`
            }
            Ok(_) => return None,
            Err(_) => {
                // A bare repo has HEAD and objects right here; leave that to git.
                if dir.join("HEAD").is_file() && dir.join("objects").is_dir() {
                    return None;
                }
                continue;
            }
        };
        return parse_head(&std::fs::read_to_string(gitdir.join("HEAD")).ok()?);
    }
    Some(String::new())
}

/// The branch named by the text of a HEAD file; see `branch_from_head` for the `None` cases.
fn parse_head(head: &str) -> Option<String> {
    let head = head.trim_end();
    if let Some(r) = head.strip_prefix("ref:") {
        return match r.trim_start().strip_prefix("refs/heads/") {
            Some("" | ".invalid") => None, // `.invalid` marks a reftable repo
            Some(name) => Some(name.to_string()),
            None => Some(String::new()),
        };
    }
    let is_sha = matches!(head.len(), 40 | 64) && head.bytes().all(|b| b.is_ascii_hexdigit());
    is_sha.then(String::new)
}

/// Build the line. `branch` is looked up by the caller so tests can pass it in.
fn render(input: &Input, pwd: &str, branch_of: &dyn Fn(&str) -> String, color: bool) -> String {
    let cwd = if input.cwd.is_empty() { pwd } else { &input.cwd };
    let dir = basename(cwd);
    let branch = branch_of(cwd);
    let paint = |code: &str, text: &str| if color { format!("{code}{text}{RST}") } else { text.to_string() };

    let mut line = paint(COPPER, &format!(" {dir}"));
    if !branch.is_empty() {
        line.push_str("  ");
        line.push_str(&paint(SAGE, &format!(" {branch}")));
    }
    if !input.model.is_empty() {
        line.push_str("  ");
        line.push_str(&paint(STONE, &input.model));
    }

    if let Some(tok) = input.ctx_tokens {
        let k = (tok.round_ties_even() as i64) / 1000;
        let code = if k >= CTX_WARN_K { COPPER } else { STONE };
        let mut ctx = format!("ctx {k}K");
        if let Some(p) = &input.ctx_percent {
            let _ = write!(ctx, " {p}%");
        }
        line.push_str("  ");
        line.push_str(&paint(code, &ctx));
    }

    let mut caps = String::new();
    if let Some(p) = &input.five_hour_percent {
        caps = format!("5h {p}%");
    }
    if let Some(p) = &input.seven_day_percent {
        if !caps.is_empty() {
            caps.push_str(" · ");
        }
        let _ = write!(caps, "7d {p}%");
    }
    if !caps.is_empty() {
        line.push_str("  ");
        line.push_str(&paint(STONE, &caps));
    }
    line
}

/// Longest step text on the line, in characters, the cut mark included.
const LEFT_MAX: usize = 40;

/// The text of a "what is left" file (`<n>\t<first open step>`) as `▸ 2 left: <step>`, with a
/// step over `LEFT_MAX` characters cut and ended with `…`. `None` for anything malformed.
fn left_text(file: &str) -> Option<String> {
    let (n, step) = file.lines().next()?.split_once('\t')?;
    let n: u32 = n.trim().parse().ok().filter(|n| *n > 0)?;
    let step = step.trim();
    if step.is_empty() {
        return None;
    }
    let step = if step.chars().count() > LEFT_MAX {
        format!("{}…", step.chars().take(LEFT_MAX - 1).collect::<String>())
    } else {
        step.to_string()
    };
    Some(format!("▸ {n} left: {step}"))
}

/// The line segment for what is left in this session: written by the guard's Stop pass, so it
/// only shows when the file for the input's `session_id` exists.
fn left_segment(input: &Input, color: bool) -> Option<String> {
    if input.session_id.is_empty() {
        return None;
    }
    let text = left_text(&std::fs::read_to_string(agent_config::status_path(&input.session_id)).ok()?)?;
    Some(if color { format!("{COPPER}{text}{RST}") } else { text })
}

/// The pace cache JSON, field order as in the shell: `updated`, `five_hour`, `seven_day`.
fn pace_json(input: &Value, now: u64) -> Option<String> {
    jq_str(at(input, &["rate_limits", "seven_day", "used_percentage"]))?;
    let pick = |v: &Value| {
        let mut m = Map::new();
        for k in ["used_percentage", "resets_at"] {
            m.insert(k.to_string(), v.get(k).cloned().unwrap_or(Value::Null));
        }
        Value::Object(m)
    };
    let mut out = Map::new();
    out.insert("updated".into(), Value::from(now));
    let five = match at(input, &["rate_limits", "five_hour"]) {
        None | Some(Value::Null | Value::Bool(false)) => Value::Null,
        Some(v) => pick(v),
    };
    out.insert("five_hour".into(), five);
    out.insert("seven_day".into(), pick(at(input, &["rate_limits", "seven_day"])?));
    serde_json::to_string(&Value::Object(out)).ok()
}

/// Write `text` to `file` through a temp file and a rename. Errors are ignored by the caller.
fn write_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = PathBuf::from(format!("{}.{}", file.display(), std::process::id()));
    let res = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, file));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn main() {
    let mut raw = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut raw);
    let doc: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Some(json) = pace_json(&doc, now) {
        let _ = write_atomic(&agent_config::paths::cache().join("claude-pace.json"), &format!("{json}\n"));
    }

    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let plain = agent_config::setting(&home_dir(), "THEME") == "plain";
    let pwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let color = !(no_color || plain);
    let input = Input::from_json(&doc);
    let mut line = render(&input, &pwd, &git_branch, color);
    if let Some(left) = left_segment(&input, color) {
        line.push_str("  ");
        line.push_str(&left);
    }
    let mut out = std::io::stdout();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    /// A fake repo: `<root>/.git/HEAD` holding `head`. Returns the root.
    fn repo(head: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join(".git")).unwrap();
        fs::write(t.path().join(".git/HEAD"), head).unwrap();
        t
    }

    #[test]
    fn percents_round_half_to_even() {
        for (given, shown) in [(0.5, "0"), (1.5, "2"), (2.5, "2"), (3.5, "4"), (-0.4, "-0"), (97.0, "97")] {
            assert_eq!(int_text(Some(&Value::from(given))).as_deref(), Some(shown), "{given}");
        }
        assert_eq!(int_text(Some(&Value::from(" 2.5 "))).as_deref(), Some("2"));
        assert_eq!(int_text(Some(&Value::Bool(true))), None);
    }

    #[test]
    fn line_from_the_input() {
        let doc: Value = serde_json::from_str(
            r#"{"workspace": {"current_dir": "/w/app"}, "model": {"display_name": "Opus"},
                "context_window": {"current_usage": {"input_tokens": 1500, "cache_read_input_tokens": 1000},
                                   "used_percentage": 2.5},
                "rate_limits": {"five_hour": {"used_percentage": 10.5}, "seven_day": {"used_percentage": 40}}}"#,
        )
        .unwrap();
        let line = render(&Input::from_json(&doc), "/elsewhere", &|_| "main".into(), false);
        assert_eq!(line, " app   main  Opus  ctx 2K 2%  5h 10% · 7d 40%");
        // no workspace and no cwd: the process folder
        let line = render(&Input::default(), "/elsewhere", &|_| String::new(), false);
        assert_eq!(line, " elsewhere");
    }

    #[test]
    fn branch_with_slash_from_a_subdir() {
        assert_eq!(branch_from_head(repo("ref: refs/heads/main\n").path()), Some("main".into()));
        let t = repo("ref: refs/heads/feat/x\n");
        let sub = t.path().join("a/b");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(branch_from_head(&sub), Some("feat/x".into()));
    }

    #[test]
    fn detached_sha_is_empty() {
        let t = repo(&format!("{SHA}\n"));
        assert_eq!(branch_from_head(t.path()), Some(String::new()));
        let t = repo(&format!("{SHA}{}\n", "0123456789abcdef01234567"));
        assert_eq!(branch_from_head(t.path()), Some(String::new()));
    }

    #[test]
    fn worktree_file_with_relative_gitdir() {
        let t = tempfile::tempdir().unwrap();
        let main_git = t.path().join("main/.git/worktrees/w");
        fs::create_dir_all(&main_git).unwrap();
        fs::write(main_git.join("HEAD"), "ref: refs/heads/wt\n").unwrap();
        let wt = t.path().join("w");
        fs::create_dir(&wt).unwrap();
        fs::write(wt.join(".git"), "gitdir: ../main/.git/worktrees/w\n").unwrap();
        assert_eq!(branch_from_head(&wt), Some("wt".into()));
    }

    #[test]
    fn worktree_file_with_absolute_gitdir() {
        let t = tempfile::tempdir().unwrap();
        let gd = t.path().join("elsewhere");
        fs::create_dir_all(&gd).unwrap();
        fs::write(gd.join("HEAD"), "ref: refs/heads/abs\n").unwrap();
        let wt = t.path().join("w");
        fs::create_dir(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        assert_eq!(branch_from_head(&wt), Some("abs".into()));
    }

    #[test]
    fn defers_to_git() {
        // each case gives None, so the status line asks git itself
        let gitdir_file = tempfile::tempdir().unwrap();
        fs::write(gitdir_file.path().join(".git"), "nonsense\n").unwrap();
        let no_head = tempfile::tempdir().unwrap();
        fs::create_dir(no_head.path().join(".git")).unwrap();
        let (invalid, unknown, main) =
            (repo("ref: refs/heads/.invalid\n"), repo("garbage\n"), repo("ref: refs/heads/main\n"));
        let plain = tempfile::tempdir().unwrap();
        let dirs = [
            gitdir_file.path().to_path_buf(),
            no_head.path().to_path_buf(),
            invalid.path().to_path_buf(),
            unknown.path().to_path_buf(),
            main.path().join(".git"),  // the cwd is inside the git dir
            plain.path().join("nope"), // a missing cwd
        ];
        for dir in dirs {
            assert_eq!(branch_from_head(&dir), None, "{}", dir.display());
        }
    }

    #[test]
    fn ref_outside_heads_is_empty() {
        let t = repo("ref: refs/remotes/x\n");
        assert_eq!(branch_from_head(t.path()), Some(String::new()));
    }

    #[test]
    fn no_repo_is_empty() {
        // The temp dir may sit under a repo on odd machines; only assert when it does not.
        let t = tempfile::tempdir().unwrap();
        let sub = t.path().join("x");
        fs::create_dir(&sub).unwrap();
        let got = branch_from_head(&sub);
        let under_repo = t.path().ancestors().any(|a| a.join(".git").exists());
        if !under_repo {
            assert_eq!(got, Some(String::new()));
        }
    }

    #[test]
    fn matches_real_git() {
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let git = |dir: &Path, args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", null)
                .env("GIT_CONFIG_SYSTEM", null)
                .stdin(Stdio::null())
                .output()
        };
        let t = tempfile::tempdir().unwrap();
        let ok = |r: std::io::Result<std::process::Output>| r.is_ok_and(|o| o.status.success());
        assert!(ok(git(t.path(), &["init", "-q", "-b", "main"])), "git is needed for this test");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        assert!(ok(git(t.path(), &commit)));
        let want = |dir: &Path| {
            let o = git(dir, &["branch", "--show-current"]).unwrap();
            String::from_utf8_lossy(&o.stdout).trim_end_matches(['\n', '\r']).to_string()
        };
        assert_eq!(branch_from_head(t.path()), Some(want(t.path())));
        assert!(ok(git(t.path(), &["checkout", "-q", "-b", "feat/y"])));
        assert_eq!(branch_from_head(t.path()), Some("feat/y".into()));
        assert_eq!(want(t.path()), "feat/y");
        assert!(ok(git(t.path(), &["checkout", "-q", "--detach"])));
        assert_eq!(branch_from_head(t.path()), Some(want(t.path())));
        assert_eq!(want(t.path()), "");
    }
}
