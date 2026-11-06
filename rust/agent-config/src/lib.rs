//! The `agent-config` library: builds `~/.claude/settings.json` from `claude/settings.json` and
//! the private layer, and holds the modules behind the `dots`, `tools` and `port` binaries. The
//! AI harness (rules, skills, hooks, Cursor and Opencode config) is the ps plugin: pavstev/agent-kit.
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const SETTINGS: &str = ".config/dotfiles/settings";
const LIVE_REL: &str = ".claude/settings.json";
const LAYER_REL: &str = ".config/dotfiles/claude-settings.json";
const LAST_REL: &str = ".config/dotfiles/claude-settings.last.json";
static DRY: AtomicBool = AtomicBool::new(false);
/// True when this run is a dry run: writers report what they would do and change nothing.
pub fn dry() -> bool {
    DRY.load(Ordering::Relaxed)
}
/// Turns the dry run on or off for the whole process.
pub fn set_dry(v: bool) {
    DRY.store(v, Ordering::Relaxed);
}

pub mod dots;
pub mod gitutil;
pub mod out;
pub mod paths;
pub mod port;
pub mod presets;
pub mod tools_cli;
pub mod tools_core;
pub use out::{info, ok, skip, summary, warn, wrote};

/// The path with the home folder shown as `~`, for messages.
pub fn tilde(p: &Path) -> String {
    let home = paths::home();
    let s = p.display().to_string();
    let h = home.display().to_string();
    if s == h {
        return "~".to_string();
    }
    if s.starts_with(&format!("{h}/")) {
        return format!("~{}", &s[h.len()..]);
    }
    s
}

/// The dotfiles repo: `DOTFILES_REPO`, else the repo around `scripts/`, else the nearest parent of
/// the current folder that has `claude/` and `scripts/`.
pub fn repo_root() -> PathBuf {
    if let Ok(v) = env::var("DOTFILES_REPO")
        && !v.is_empty()
    {
        return PathBuf::from(v);
    }
    if let Ok(exe) = env::current_exe()
        && let Some(parent) = exe.parent()
        && parent.file_name().and_then(|s| s.to_str()) == Some("scripts")
        && let Some(r) = parent.parent()
    {
        return r.to_path_buf();
    }
    let mut cur = env::current_dir().unwrap_or_default();
    loop {
        if cur.join("claude").is_dir() && cur.join("scripts").is_dir() {
            return cur;
        }
        if !cur.pop() {
            break;
        }
    }
    PathBuf::from(".")
}
/// The `claude/` folder of the repo, the source of all generated config.
pub fn claude_dir() -> PathBuf {
    repo_root().join("claude")
}

/// True when the path itself is a symlink (a link to a missing target counts).
pub fn is_symlink(p: &Path) -> bool {
    p.symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// One value from `~/.config/dotfiles/settings` (the last line of that name wins), or an empty string.
pub fn setting(home: &Path, name: &str) -> String {
    let p = home.join(SETTINGS);
    let Ok(data) = fs::read(&p) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&data);
    let mut value = String::new();
    for line in text.split('\n') {
        if line.starts_with(&format!("{name}=")) {
            let v = &line[name.len() + 1..];
            value = v.trim_end().to_string();
            if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                value = value[1..value.len() - 1].to_string();
            }
        }
    }
    value
}

// Writers
/// Writes the file through a temp file and a rename, so a reader never sees half a file.
/// It creates missing folders and sets `mode` (else keeps the old mode, or 0644 for a new file).
///
/// # Errors
///
/// Returns an error when the folder, the temp file, the write, the mode change or the rename fails.
pub fn atomic_write(path: &Path, text: &str, mode: Option<u32>) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut builder = tempfile::Builder::new();
    let prefix = format!(
        ".{}.",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp")
    );
    builder.prefix(&prefix);
    let mut nt = builder.tempfile_in(parent)?;
    nt.write_all(text.as_bytes())?;
    nt.flush()?;
    #[cfg(not(unix))]
    let _ = mode;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(nt.path())?.permissions();
        if let Some(m) = mode {
            perms.set_mode(m);
        } else if path.exists() {
            if let Ok(md) = fs::metadata(path) {
                perms = md.permissions();
            }
        } else {
            perms.set_mode(0o644);
        }
        fs::set_permissions(nt.path(), perms)?;
    }
    nt.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// The repo base with the settings fragments of the enabled presets merged over it, in order.
pub fn with_presets(home: &Path, base: &Value) -> Value {
    let on = presets::enabled_quiet(home, presets::current_profile(home));
    presets::fragments(&repo_root(), &on)
        .iter()
        .fold(base.clone(), |acc, f| merge(&acc, f))
}

/// Writes a side file (a sidecar list, the last build) and warns when it cannot.
fn write_or_warn(path: &Path, text: &str, mode: Option<u32>) {
    if let Err(e) = atomic_write(path, text, mode) {
        warn(&format!("could not write {}: {e}", tilde(path)));
    }
}

/// A JSON object from the file; an empty object when the file is missing, `None` when it is not
/// valid JSON or not an object.
pub fn load_json(path: &Path) -> Option<Value> {
    if !path.exists() {
        return Some(serde_json::json!({}));
    }
    let data = fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&data).ok()?;
    if v.is_object() { Some(v) } else { None }
}

/// The value at a path of keys, if every key is there.
pub fn get_path(obj: &Value, path: &[&str]) -> Option<Value> {
    let mut cur = obj;
    for k in path {
        cur = cur.as_object()?.get(*k)?;
    }
    Some(cur.clone())
}

/// Removes the key at a path, and any parent object that becomes empty.
pub fn drop_path(obj: &mut Value, path: &[&str]) {
    if path.len() == 1 {
        if let Some(o) = obj.as_object_mut() {
            o.remove(path[0]);
        }
        return;
    }
    if let Some(sub) = obj.get_mut(path[0])
        && sub.is_object()
    {
        drop_path(sub, &path[1..]);
        if sub.as_object().is_some_and(serde_json::Map::is_empty)
            && let Some(o) = obj.as_object_mut()
        {
            o.remove(path[0]);
        }
    }
}

fn is_union(path: &[String]) -> bool {
    let joined = path.join(".");
    matches!(
        joined.as_str(),
        "permissions.allow"
            | "permissions.deny"
            | "permissions.ask"
            | "permissions.additionalDirectories"
    )
}

/// The layer laid over the base: objects merge, `null` removes a key, the permission lists join
/// without repeats, and anything else is replaced.
pub fn merge(base: &Value, layer: &Value) -> Value {
    merge_inner(base, layer, &[])
}
fn merge_inner(base: &Value, layer: &Value, path: &[String]) -> Value {
    let Some(lo) = layer.as_object() else {
        return base.clone();
    };
    let mut out_obj = base.as_object().cloned().unwrap_or_default();
    for (k, v) in lo {
        let mut p = path.to_vec();
        p.push(k.clone());
        if v.is_null() {
            out_obj.remove(k);
        } else if v.is_object() {
            // merge into an empty object when the base has none, so a nested `null` never lands
            let empty = serde_json::json!({});
            let merged = merge_inner(
                out_obj.get(k).filter(|b| b.is_object()).unwrap_or(&empty),
                v,
                &p,
            );
            out_obj.insert(k.clone(), merged);
        } else if is_union(&p)
            && let (Some(have), Some(add)) =
                (out_obj.get(k).and_then(Value::as_array), v.as_array())
        {
            let mut arr = have.clone();
            for x in add {
                if !arr.contains(x) {
                    arr.push(x.clone());
                }
            }
            out_obj.insert(k.clone(), Value::Array(arr));
        } else {
            out_obj.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out_obj)
}

/// What changed in the live settings since the last build, as a patch for the private layer plus
/// notes (kind, key path, item count) for the user.
pub fn drift(last: &Value, live: &Value) -> (Value, Vec<(String, Vec<String>, usize)>) {
    drift_inner(last, live, &[])
}
fn drift_inner(
    last: &Value,
    live: &Value,
    path: &[String],
) -> (Value, Vec<(String, Vec<String>, usize)>) {
    let mut patch = serde_json::Map::new();
    let mut notes = Vec::new();
    if let Some(lo) = live.as_object() {
        for (k, v) in lo {
            let mut p = path.to_vec();
            p.push(k.clone());
            let old = last.get(k);
            match old {
                None => {
                    patch.insert(k.clone(), v.clone());
                    notes.push(("set".to_string(), p, 0));
                }
                Some(oldv) if oldv.is_object() && v.is_object() => {
                    let (sub, n) = drift_inner(oldv, v, &p);
                    if !sub.as_object().is_none_or(serde_json::Map::is_empty) {
                        patch.insert(k.clone(), sub);
                    }
                    notes.extend(n);
                }
                Some(oldv) if is_union(&p) && oldv.is_array() && v.is_array() => {
                    let old_items: &[Value] = oldv.as_array().map_or(&[], Vec::as_slice);
                    let new_items: &[Value] = v.as_array().map_or(&[], Vec::as_slice);
                    let added: Vec<Value> = new_items
                        .iter()
                        .filter(|x| !old_items.contains(x))
                        .cloned()
                        .collect();
                    let removed_n = old_items.iter().filter(|x| !new_items.contains(x)).count();
                    if !added.is_empty() {
                        patch.insert(k.clone(), Value::Array(added.clone()));
                        notes.push(("add".to_string(), p.clone(), added.len()));
                    }
                    if removed_n > 0 {
                        notes.push(("removed".to_string(), p.clone(), removed_n));
                    }
                }
                Some(oldv) => {
                    if oldv != v {
                        patch.insert(k.clone(), v.clone());
                        notes.push(("set".to_string(), p, 0));
                    }
                }
            }
        }
    }
    if let Some(lo) = last.as_object() {
        for (k, _v) in lo {
            if live.get(k).is_none() {
                let mut p = path.to_vec();
                p.push(k.clone());
                patch.insert(k.clone(), Value::Null);
                notes.push(("unset".to_string(), p, 0));
            }
        }
    }
    (Value::Object(patch), notes)
}

/// The layer with a `drift` patch applied to it.
pub fn apply_patch(layer: &Value, patch: &Value) -> Value {
    apply_patch_inner(layer, patch, &[])
}
fn apply_patch_inner(layer: &Value, patch: &Value, path: &[String]) -> Value {
    let Some(po) = patch.as_object() else {
        return layer.clone();
    };
    let mut out_obj = layer.as_object().cloned().unwrap_or_default();
    for (k, v) in po {
        let mut p = path.to_vec();
        p.push(k.clone());
        if let Some(ov) = out_obj.get(k).cloned()
            && ov.is_object()
            && v.is_object()
        {
            out_obj.insert(k.clone(), apply_patch_inner(&ov, v, &p));
            continue;
        }
        if is_union(&p)
            && let (Some(add), Some(have)) =
                (v.as_array(), out_obj.get(k).and_then(Value::as_array))
        {
            let mut arr = have.clone();
            for x in add {
                if !arr.contains(x) {
                    arr.push(x.clone());
                }
            }
            out_obj.insert(k.clone(), Value::Array(arr));
        } else {
            out_obj.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out_obj)
}

/// The JSON as pretty text with a closing newline.
pub fn dump(obj: &Value) -> String {
    serde_json::to_string_pretty(obj).unwrap_or_default() + "\n"
}

/// Builds `~/.claude/settings.json` from the repo base, the enabled presets and the private layer.
/// Changes Claude made to the live file move into the layer first. Returns 1 on invalid JSON.
pub fn claude_settings_build(home: &Path) -> i32 {
    let base_f = claude_dir().join("settings.json");
    let layer_f = home.join(LAYER_REL);
    let last_f = home.join(LAST_REL);
    let live_f = home.join(LIVE_REL);
    let (Some(base), Some(mut layer)) = (load_json(&base_f), load_json(&layer_f)) else {
        let bad = if load_json(&base_f).is_none() {
            &base_f
        } else {
            &layer_f
        };
        warn(&format!(
            "{} is not valid JSON. Fix it; ~/.claude/settings.json left as it is.",
            tilde(bad)
        ));
        return 1;
    };
    let base = with_presets(home, &base);
    let was_link = is_symlink(&live_f);
    let mut notes = Vec::new();
    if live_f.exists() && !was_link {
        let Some(live) = load_json(&live_f) else {
            warn(&format!(
                "{} is not valid JSON (Claude may be writing it). Run dots again.",
                tilde(&live_f)
            ));
            return 1;
        };
        let last = if last_f.exists() {
            load_json(&last_f).unwrap_or_else(|| merge(&with_presets(home, &base), &layer))
        } else {
            merge(&with_presets(home, &base), &layer)
        };
        let (patch, n) = drift(&last, &live);
        notes = n;
        if !patch.as_object().is_none_or(serde_json::Map::is_empty) {
            layer = apply_patch(&layer, &patch);
            if dry() {
                info(&format!(
                    "would keep {} change(s) in {}",
                    notes.len(),
                    tilde(&layer_f)
                ));
            } else if let Err(e) = atomic_write(&layer_f, &dump(&layer), Some(0o600)) {
                warn(&format!("could not write {}: {e}", tilde(&layer_f)));
                return 1;
            }
        }
    }
    for (kind, p, n) in &notes {
        let key = p.join(".");
        if kind == "removed" {
            warn(&format!(
                "Claude removed {n} item(s) from {key}, but the repo or your private layer has them. Remove them there to make it stick."
            ));
        } else {
            let what = match kind.as_str() {
                "set" => "changed".to_string(),
                "add" => format!("+{n} item(s)"),
                "unset" => "removed".to_string(),
                _ => kind.clone(),
            };
            info(&format!("kept in your private layer: {key} ({what})"));
        }
    }
    let text = dump(&merge(&with_presets(home, &base), &layer));
    if !was_link
        && live_f.exists()
        && let Ok(oldtext) = fs::read_to_string(&live_f)
        && oldtext == text
    {
        skip(&tilde(&live_f));
        // still maybe update last
        if !dry() && (!last_f.exists() || fs::read_to_string(&last_f).unwrap_or_default() != text) {
            write_or_warn(&last_f, &text, Some(0o600));
        }
        return 0;
    }
    if dry() {
        info(&format!("would write {}", tilde(&live_f)));
        return 0;
    }
    if was_link {
        info(&format!(
            "{} was a link into the repo; it is now a built file",
            tilde(&live_f)
        ));
        let _ = fs::remove_file(&live_f);
    }
    if let Err(e) = atomic_write(&live_f, &text, Some(0o644)) {
        warn(&format!("could not write {}: {e}", tilde(&live_f)));
        return 1;
    }
    wrote(&format!(
        "built {} (repo base + private layer)",
        tilde(&live_f)
    ));
    if !dry() && (!last_f.exists() || fs::read_to_string(&last_f).unwrap_or_default() != text) {
        write_or_warn(&last_f, &text, Some(0o600));
    }
    0
}

/// Moves a dotted key from the private layer into the repo base, then rebuilds the live file.
pub fn claude_settings_promote(home: &Path, dotted: &str) -> i32 {
    let path: Vec<&str> = dotted.split('.').collect();
    let base_f = claude_dir().join("settings.json");
    let layer_f = home.join(LAYER_REL);
    let (Some(base), Some(mut layer)) = (load_json(&base_f), load_json(&layer_f)) else {
        warn("the repo base or the private layer is not valid JSON; nothing moved");
        return 1;
    };
    let Some(value) = get_path(&layer, &path) else {
        warn(&format!("{} is not in {}", dotted, tilde(&layer_f)));
        return 1;
    };
    let mut patch = value;
    for k in path.iter().rev() {
        let key = *k;
        patch = serde_json::json!({ key: patch});
    }
    let new_base = merge(&base, &patch);
    drop_path(&mut layer, &path.clone());
    if dry() {
        info(&format!("would move {} to {}", dotted, tilde(&base_f)));
        return 0;
    }
    // The base first: when it fails, the key stays in the layer and nothing is lost.
    if let Err(e) = atomic_write(&base_f, &dump(&new_base), None) {
        warn(&format!(
            "could not write {}: {e}; nothing moved",
            tilde(&base_f)
        ));
        return 1;
    }
    if let Err(e) = atomic_write(&layer_f, &dump(&layer), Some(0o600)) {
        warn(&format!(
            "could not write {}: {e}; the key is now in both files",
            tilde(&layer_f)
        ));
        return 1;
    }
    ok(&format!(
        "moved {} to {}. Commit it to share it.",
        dotted,
        tilde(&base_f)
    ));
    claude_settings_build(home)
}

/// The reverse of promote: copy a key's value from the live `~/.claude/settings.json` into the
/// private layer, so it stays when the repo base drops it. A note and no change when the live file
/// lacks the key.
pub fn claude_settings_demote(home: &Path, dotted: &str) -> i32 {
    let path: Vec<&str> = dotted.split('.').collect();
    let live_f = home.join(LIVE_REL);
    let layer_f = home.join(LAYER_REL);
    let (Some(live), Some(layer)) = (load_json(&live_f), load_json(&layer_f)) else {
        warn("the live settings or the private layer is not valid JSON; nothing moved");
        return 1;
    };
    let Some(value) = get_path(&live, &path) else {
        info(&format!(
            "{} is not in {}; nothing to keep",
            dotted,
            tilde(&live_f)
        ));
        return 0;
    };
    if get_path(&layer, &path).as_ref() == Some(&value) {
        skip(&format!("{dotted} (already in your private layer)"));
        return 0;
    }
    let mut patch = value;
    for k in path.iter().rev() {
        let key = *k;
        patch = serde_json::json!({ key: patch });
    }
    let new_layer = merge(&layer, &patch);
    if dry() {
        info(&format!("would copy {} to {}", dotted, tilde(&layer_f)));
        return 0;
    }
    if let Err(e) = atomic_write(&layer_f, &dump(&new_layer), Some(0o600)) {
        warn(&format!("could not write {}: {}", tilde(&layer_f), e));
        return 1;
    }
    ok(&format!("copied {} to {}", dotted, tilde(&layer_f)));
    0
}

// CLI args structs
/// The `agent-config` command line.
#[derive(Parser, Debug)]
#[command(
    name = "agent-config",
    about = "Build ~/.claude/settings.json from claude/settings.json and the private layer.",
    long_about = "Build ~/.claude/settings.json from claude/settings.json and the private layer.\n\n\
        Cursor and Opencode config, the rules, skills and hooks come from the ps plugin\n\
        (github.com/pavstev/agent-kit)."
)]
pub struct Cli {
    /// Override the dotfiles repo root (defaults to the repo this binary ships in).
    #[arg(long, global = true)]
    pub repo: Option<PathBuf>,
    /// The command to run.
    #[command(subcommand)]
    pub cmd: Commands,
}

/// The `agent-config` subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// build ~/.claude/settings.json from the repo base and the private layer
    #[command(name = "claude-settings")]
    ClaudeSettings(ClaudeSettingsArgs),
}

/// Arguments of the `claude-settings` command.
#[derive(Parser, Debug)]
pub struct ClaudeSettingsArgs {
    /// build, promote a dotted key from the private layer into the repo base, or demote one from the live file into the layer
    #[arg(value_parser = ["build", "promote", "demote"])]
    pub action: String,
    /// promote or demote: the dotted key to move, e.g. permissions.allow
    pub key: Option<String>,
    /// report what would change without writing
    #[arg(long)]
    pub dry_run: bool,
}

/// Runs the `agent-config` command line and returns the exit code.
pub fn run() -> i32 {
    run_cli(Cli::parse())
}

/// Run one agent-config command in this process (`dots` does). A panic counts as a failure.
pub fn run_args(args: &[&str]) -> i32 {
    let argv = std::iter::once("agent-config").chain(args.iter().copied());
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => {
            warn(e.to_string().trim_end());
            return 2;
        }
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_cli(cli))).unwrap_or(1)
}

fn run_cli(cli: Cli) -> i32 {
    if let Some(r) = cli.repo {
        unsafe {
            env::set_var("DOTFILES_REPO", r);
        }
    }
    let (rc, label) = match cli.cmd {
        Commands::ClaudeSettings(a) => {
            set_dry(a.dry_run);
            (cmd_claude_settings_dispatch(&a), "Claude settings")
        }
    };
    summary(label);
    rc
}

fn cmd_claude_settings_dispatch(a: &ClaudeSettingsArgs) -> i32 {
    set_dry(a.dry_run);
    let home = paths::home();
    if a.action == "promote" {
        let Some(key) = a.key.as_deref().filter(|k| !k.is_empty()) else {
            warn("promote needs a key, e.g. permissions.allow");
            return 2;
        };
        claude_settings_promote(&home, key)
    } else if a.action == "demote" {
        let Some(key) = a.key.as_deref().filter(|k| !k.is_empty()) else {
            warn("demote needs a key, e.g. enabledPlugins");
            return 2;
        };
        claude_settings_demote(&home, key)
    } else {
        claude_settings_build(&home)
    }
}
