//! A dev server port for each worktree of a repo.
//!
//! A main checkout uses the base port its `launch.json` names. A worktree gets a port above the
//! base (`base + 1 ..= base + SPAN`), picked from a hash of the server name and the worktree path,
//! so the same worktree and server get the same port every time. The pick skips ports that a
//! `launch.json` of a sibling repo claims, and (when asked) ports that something listens on now.
//!
//! The checkout kind comes from the files only: a `.git` folder is a main checkout, a `.git` file
//! (`gitdir: <main>/.git/worktrees/<name>`) is a worktree. No `git` process runs.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

/// How many ports above the base a worktree may use.
pub const SPAN: u32 = 99;

/// What kind of checkout a folder is in.
#[derive(Debug, PartialEq, Eq)]
pub enum Checkout {
    /// The folder with the `.git` directory.
    Main,
    /// A linked worktree: its own root, and the root of the main checkout when the `.git` file names it.
    Worktree {
        root: PathBuf,
        main: Option<PathBuf>,
    },
}

/// The checkout that `start` lives in, or `None` outside any repo.
pub fn find_checkout(start: &Path) -> Option<Checkout> {
    for dir in start.ancestors() {
        let git = dir.join(".git");
        if git.is_dir() {
            return Some(Checkout::Main);
        }
        if git.is_file() {
            let main = fs::read_to_string(&git).ok().and_then(|t| main_of(&t));
            return Some(Checkout::Worktree {
                root: dir.to_path_buf(),
                main,
            });
        }
    }
    None
}

/// The main checkout root named by the text of a worktree's `.git` file.
fn main_of(text: &str) -> Option<PathBuf> {
    let gitdir = PathBuf::from(text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim());
    // <main>/.git/worktrees/<name>
    let main_git = gitdir.parent()?.parent()?;
    if main_git.file_name()? != ".git" {
        return None;
    }
    main_git.parent().map(Path::to_path_buf)
}

/// Every `port` that a `launch.json` claims in `<dir>/.claude/` and in `<dir>/*/.claude/`.
pub fn claimed_ports(dir: &Path) -> BTreeSet<u16> {
    let mut files = vec![dir.join(".claude").join("launch.json")];
    if let Ok(rd) = fs::read_dir(dir) {
        files.extend(
            rd.flatten()
                .map(|e| e.path().join(".claude").join("launch.json")),
        );
    }
    let mut ports = BTreeSet::new();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(entries) = json["configurations"].as_array() else {
            continue;
        };
        ports.extend(
            entries
                .iter()
                .filter_map(|e| e["port"].as_u64())
                .filter_map(|p| u16::try_from(p).ok()),
        );
    }
    ports
}

/// True when something listens on the port at 127.0.0.1 or at `::1` (Vite under modern Node binds
/// `localhost`, which is `::1`). A host with no IPv6 loopback checks 127.0.0.1 alone.
pub fn in_use(port: u16) -> bool {
    let v6_taken = matches!(TcpListener::bind(("::1", port)),
        Err(e) if e.kind() != std::io::ErrorKind::AddrNotAvailable);
    TcpListener::bind(("127.0.0.1", port)).is_err() || v6_taken
}

/// The port for server `name` in the worktree at `root`: the first free one, walking from a hash
/// of both. `None` when no port is left in `base + 1 ..= base + SPAN` below 65536.
pub fn pick(
    name: &str,
    root: &Path,
    base: u16,
    claimed: &BTreeSet<u16>,
    probe: bool,
) -> Option<u16> {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update([0]);
    hasher.update(root.to_string_lossy().as_bytes());
    let d = hasher.finalize();
    let start = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % SPAN;
    (0..SPAN)
        .map(|i| u32::from(base) + 1 + (start + i) % SPAN)
        .filter_map(|p| u16::try_from(p).ok())
        .find(|p| !(claimed.contains(p) || probe && in_use(*p)))
}

/// The port to print for `name` with `base`, as seen from `cwd`.
pub fn resolve(cwd: &Path, name: &str, base: u16, probe: bool) -> Option<u16> {
    match find_checkout(cwd) {
        None | Some(Checkout::Main) => Some(base),
        Some(Checkout::Worktree { root, main }) => {
            // The folder that holds the repos: its own launch.json and every repo's are claims.
            let scan = main
                .as_deref()
                .and_then(Path::parent)
                .or_else(|| root.parent());
            let claimed = scan.map(claimed_ports).unwrap_or_default();
            pick(name, &root, base, &claimed, probe)
        }
    }
}
