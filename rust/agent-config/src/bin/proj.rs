//! `proj`: facts about the project a path lives in, and the format-on-edit hook.
//!
//! ```text
//! proj detect [PATH]   JSON: root, package manager, tools, the project's check contract, the fast
//!                      and full check commands, the fix commands (and the format commands of a file)
//! proj format [FILE]   format FILE with its project's formatter; with no FILE, read the hook JSON
//!                      from stdin (PostToolUse on Edit, Write and MultiEdit)
//! ```
//!
//! `detect` exits 1 with `{"root": null}` outside a project. `format` is a hook: silent, always
//! exit 0, short budget (`PROJ_FORMAT_BUDGET_MS`, default 8000). The logic is `agent_config::proj`.

use agent_config::proj;
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "proj", about = "Detect the project of a path and format files on edit")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print what the project of PATH (default: the current folder) states and what the defaults add
    Detect { path: Option<String> },
    /// Format one file; with no FILE, read the PostToolUse hook JSON on stdin
    Format { file: Option<String> },
}

fn detect(path: Option<String>) -> i32 {
    let p = path.map(PathBuf::from).unwrap_or_else(|| env::current_dir().unwrap_or_default());
    let p = if p.is_relative() { env::current_dir().unwrap_or_default().join(p) } else { p };
    match proj::detect(&p) {
        Some(project) => {
            let file = p.is_file().then_some(p.as_path());
            println!("{}", serde_json::to_string_pretty(&project.to_json(file)).unwrap_or_default());
            0
        }
        None => {
            println!("{{\"root\": null}}");
            1
        }
    }
}

/// The edited file of a hook call: `tool_input.file_path` (or `path`), relative to the call's `cwd`.
fn hook_file() -> Option<PathBuf> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let ti = &v["tool_input"];
    let f = ti["file_path"].as_str().or_else(|| ti["path"].as_str()).filter(|s| !s.is_empty())?;
    let f = Path::new(f);
    if f.is_absolute() {
        return Some(f.to_path_buf());
    }
    Some(Path::new(v["cwd"].as_str()?).join(f))
}

fn format(file: Option<String>) {
    let budget = env::var("PROJ_FORMAT_BUDGET_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(8000);
    let file = match file {
        Some(f) => Some(PathBuf::from(f)),
        None => hook_file(),
    };
    if let Some(f) = file {
        proj::format_file(&f, Duration::from_millis(budget));
    }
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let code = std::panic::catch_unwind(|| {
        let cli = match Cli::try_parse() {
            Ok(c) => c,
            Err(e) => {
                let _ = e.print();
                return if e.use_stderr() { 2 } else { 0 };
            }
        };
        match cli.cmd {
            Cmd::Detect { path } => detect(path),
            Cmd::Format { file } => {
                format(file);
                0
            }
        }
    })
    .unwrap_or(0);
    std::process::exit(code);
}
