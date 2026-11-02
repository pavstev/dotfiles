//! Run git and get its stdout: the one helper behind `dots::git::out`, `leftovers` and the Cursor
//! superlite clone.

use std::path::Path;
use std::process::{Command, Stdio};

/// stdout of `git [-C dir] args` without the trailing newline, or `None` when git cannot start or
/// exits non-zero. stdin and stderr are closed.
pub fn git_out(dir: Option<&Path>, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let o = cmd
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    o.status.success().then(|| {
        String::from_utf8_lossy(&o.stdout)
            .trim_end_matches(['\n', '\r'])
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_without_the_trailing_newline() {
        assert!(
            git_out(None, &["--version"])
                .is_some_and(|v| v.starts_with("git version") && !v.ends_with('\n'))
        );
        assert_eq!(git_out(None, &["no-such-subcommand-xyz"]), None);
    }

    #[test]
    fn a_dir_that_is_not_a_repo_is_none() {
        let dir = std::env::temp_dir().join(format!("gitutil-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(git_out(Some(&dir), &["rev-parse", "HEAD"]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
