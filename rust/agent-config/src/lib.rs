//! The `agent-config` library: builds the config other harnesses read from `claude/` (Cursor,
//! Opencode) and the Claude `settings.json`. It also holds the modules behind the `dots`, `guard`
//! and `tools` binaries. The `agent-config` bin and `dots` call it; hooks reach it through `rust-bin`.
use clap::{Parser, Subcommand};
use presets::Profile;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

const MARK: &str = "generated-by: dotfiles";
const OWNER_MD: &str = ".config/dotfiles/owner.md";
const SETTINGS: &str = ".config/dotfiles/settings";
const MANAGED: &str = ".dotfiles-managed.json";
const USER_RULES_MARK: &str = ".config/dotfiles/cursor-user-rules.sha256";
const LIVE_REL: &str = ".claude/settings.json";
const LAYER_REL: &str = ".config/dotfiles/claude-settings.json";
const LAST_REL: &str = ".config/dotfiles/claude-settings.last.json";
/// The note placed above the rules in Cursor: how Claude's tool and agent names map to Cursor's.
pub const CURSOR_NOTE: &str = "> In Cursor, read \"Claude\" as \"the agent\". Claude Code tool names map to Cursor:\n> Agent = Task (subagent), AskUserQuestion = AskQuestion, `/ps:*` skills work the same.\n> Rules about Claude-only features apply where Cursor has an equivalent.\n> Task accepts only these subagent_type values: generalPurpose, explore, cursor-guide,\n> ci-investigator, bugbot, security-review, best-of-n-runner. A named agent from the agents\n> table (code-reviewer, task-implementer, ...) is not one of them. Spawn generalPurpose and put\n> this in the prompt: \"Read ~/.claude/agents/<name>.md first and follow it.\" Plain lookups use explore.\n\n";
const OPENCODE_NOTE: &str = "> In OpenCode, read \"Claude\" as \"the agent\". Claude Code tool names map to OpenCode:\n> Agent = Task (subagent), AskUserQuestion = `question`, `/ps:*` skills work the same.\n> Rules about Claude-only features apply where OpenCode has an equivalent.\n\n";
const WRITE_TOOLS: [&str; 5] = ["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"];

static DRY: AtomicBool = AtomicBool::new(false);
/// True when this run is a dry run: writers report what they would do and change nothing.
pub fn dry() -> bool {
    DRY.load(Ordering::Relaxed)
}
/// Turns the dry run on or off for the whole process.
pub fn set_dry(v: bool) {
    DRY.store(v, Ordering::Relaxed);
}

/// Why a part of a command could not do its job. The command reports it as `skipped a part`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A permission list in `cli-config.json` is not an array.
    #[error("permissions.{0} in cli-config.json is not a list")]
    NotAList(&'static str),
}

/// A Claude permission rule: `Tool(arg)`.
static RULE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+)\((.*)\)$").unwrap());
/// The wildcard end of a Bash rule: `:*` or ` *`.
static WILDCARD_END_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(:\*|\s\*)$").unwrap());
/// The front matter block of a Markdown file.
static FRONTMATTER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^---\n(.*?)\n---\n?(.*)$").unwrap());
/// A `key: value` line of front matter.
static KEY_LINE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z0-9_-]+):(?:\s+(.*)|\s*)$").unwrap());
/// A `- item` line of front matter.
static ITEM_LINE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*-\s+(.*)$").unwrap());

pub mod codeview;
pub mod dots;
pub mod guard_checks;
pub mod out;
pub mod pace_core;
pub mod paths;
pub mod presets;
pub mod proj;
pub mod pycompat;
pub mod quiet_out;
pub mod redact;
pub mod scan;
pub mod stores;
pub mod tools_cli;
pub mod tools_core;
pub mod transcript;
pub mod undo;
#[allow(unused_imports)]
use out::{b, d, g, y, z};
pub use out::{color_enabled, header, info, ok, skip, summary, verbose, warn, wrote};

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
/// The `cursor/` folder of the repo (Cursor-only sources such as `mcp.json`).
pub fn cursor_src_dir() -> PathBuf {
    repo_root().join("cursor")
}
/// The launcher that Cursor's hooks run: `rust-bin --hook guard_cursor <mode>`.
pub fn guard_path() -> PathBuf {
    claude_dir().join("hooks").join("rust-bin")
}

/// The "what is left" file of a session: `<cache dir>/dots/status/<session>.txt`, one line
/// `<n>\t<first open step>`. The guard's Stop pass writes it; the statusline reads it. Only
/// `[A-Za-z0-9_-]` of the session id is kept, so the id cannot leave the folder.
pub fn status_path(session: &str) -> PathBuf {
    let mut name: String = session.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')).collect();
    if name.is_empty() {
        name.push('_');
    }
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache")).join("dots").join("status").join(format!("{name}.txt"))
}

/// The file as text, or `None` when it cannot be read.
pub fn read_text(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok()
}

/// True when the path itself is a symlink (a link to a missing target counts).
pub fn is_symlink(p: &Path) -> bool {
    p.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink())
}

/// The value without one pair of matching quotes, with JSON escapes read for double quotes.
pub fn unquote(v: &str) -> String {
    let bytes = v.as_bytes();
    if v.len() >= 2 && bytes[0] == bytes[v.len() - 1] && (bytes[0] == b'"' || bytes[0] == b'\'') {
        if bytes[0] == b'"'
            && let Ok(s) = serde_json::from_str::<String>(v)
        {
            return s;
        }
        let inner = &v[1..v.len() - 1];
        if bytes[0] == b'\'' {
            return inner.replace("''", "'");
        }
        return inner.to_string();
    }
    v.to_string()
}

/// Splits a Markdown file into its `---` front matter (key to text) and the body.
/// Text with no front matter gives an empty map and the whole text.
pub fn split_frontmatter(text: &str) -> (BTreeMap<String, String>, String) {
    let norm = text.replace("\r\n", "\n");
    let Some(caps) = FRONTMATTER_RE.captures(&norm) else { return (BTreeMap::new(), text.to_string()) };
    let meta_text = caps.get(1).unwrap().as_str();
    let body = caps.get(2).unwrap().as_str().to_string();
    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    let mut key: Option<String> = None;
    let mut style: Option<char> = None;
    let mut items: Vec<String> = Vec::new();
    let mut block: Vec<String> = Vec::new();

    let flush = |key: &mut Option<String>,
                 style: &mut Option<char>,
                 items: &mut Vec<String>,
                 block: &mut Vec<String>,
                 meta: &mut BTreeMap<String, String>| {
        if key.is_none() {
            return;
        }
        let k = key.clone().unwrap();
        if !items.is_empty() {
            meta.insert(k, items.join(", "));
        } else if *style == Some('|') {
            meta.insert(k, block.join("\n").trim_matches('\n').to_string());
        } else if !block.is_empty() {
            let joined: Vec<String> = block.iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            meta.insert(k, joined.join(" "));
        }
    };

    for line in meta_text.split('\n') {
        if line.trim_start().starts_with('#') && style.is_none() {
            continue;
        }
        if let Some(km) = KEY_LINE_RE.captures(line) {
            flush(&mut key, &mut style, &mut items, &mut block, &mut meta);
            key = Some(km.get(1).unwrap().as_str().to_string());
            items = Vec::new();
            block = Vec::new();
            let value = km.get(2).map_or("", |m| m.as_str()).trim().to_string();
            if value.chars().next().is_some_and(|c| c == '|' || c == '>') {
                style = value.chars().next();
            } else {
                style = None;
                let u = unquote(&value);
                meta.insert(key.clone().unwrap(), u.clone());
                if !value.is_empty() {
                    block = vec![u];
                }
            }
            continue;
        }
        if key.is_none() {
            continue;
        }
        if let Some(lm) = ITEM_LINE_RE.captures(line) {
            let k = key.clone().unwrap();
            if style.is_none() && meta.get(&k) == Some(&String::new()) {
                items.push(unquote(lm.get(1).unwrap().as_str().trim()));
                continue;
            }
        }
        if line.starts_with(' ') || line.starts_with('\t') || (style.is_some() && line.trim().is_empty()) {
            if style == Some('|') {
                block.push(line.trim().to_string());
            } else {
                block.push(line.to_string());
            }
        }
    }
    flush(&mut key, &mut style, &mut items, &mut block, &mut meta);
    (meta, body)
}

/// The private `owner.md` rules file, only for the `owner` profile and only when it exists.
pub fn owner_overlay(home: &Path, profile: Profile) -> Option<PathBuf> {
    let f = home.join(OWNER_MD);
    if profile == Profile::Owner && f.is_file() { Some(f) } else { None }
}

fn has_preset_rules(home: &Path, profile: Profile) -> bool {
    !presets::rules_text(&repo_root(), &presets::enabled_quiet(home, profile)).is_empty()
}

/// The rules for Cursor and Opencode: the core, the enabled presets, then the private overlay.
pub fn rules_text(home: &Path, profile: Profile) -> String {
    let mut text = read_text(&claude_dir().join("CLAUDE.md")).unwrap_or_default();
    let on = presets::enabled(home, profile);
    let extra = presets::rules_text(&repo_root(), &on);
    if !extra.is_empty() {
        text = format!("{}\n\n{}\n", text.trim_end_matches('\n'), extra);
    }
    if let Some(overlay) = owner_overlay(home, profile) {
        format!("{}\n\n{}", text.trim_end_matches('\n'), read_text(&overlay).unwrap_or_default())
    } else {
        text
    }
}

/// The text as a YAML value, double-quoted only when plain YAML would misread it.
pub fn yaml_str(v: &str) -> String {
    if v.contains('\n')
        || v.contains(": ")
        || v.contains(" #")
        || (!v.is_empty() && v.trim() != v)
        || v.chars().next().is_some_and(|c| "!&*?|>%@`'\"[]{},#-".contains(c))
    {
        serde_json::to_string(v).unwrap_or_default()
    } else {
        v.to_string()
    }
}

/// A Cursor `.mdc` rule file that always applies, with the given description and body.
pub fn mdc(description: &str, body: &str) -> String {
    format!("---\ndescription: {description}\nalwaysApply: true\n---\n{body}")
}

/// The `generated-by` comment that opens a generated file and names its sources.
pub fn stamp(srcs: &[&str]) -> String {
    let edit = if srcs.len() == 1 { "that file" } else { "those files" };
    format!("<!-- {} (scripts/agent-config) from {}. Edit {}, then run dots. -->\n\n", MARK, srcs.join(" and "), edit)
}

/// The SHA-256 of the text, in lowercase hex.
pub fn sha(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

/// One value from `~/.config/dotfiles/settings` (the last line of that name wins), or an empty string.
pub fn setting(home: &Path, name: &str) -> String {
    let p = home.join(SETTINGS);
    let Ok(data) = fs::read(&p) else { return String::new() };
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

/// The text to paste into Cursor User Rules: the Cursor note, the rules and the reply style.
pub fn user_rules_text(home: &Path, profile: Profile) -> String {
    let mut text = format!("{}{}", CURSOR_NOTE, rules_text(home, profile));
    if let Some((name, _, body)) = output_style(home) {
        text = format!("{}\n\n# Reply style: {}\n\n{}", text.trim_end_matches('\n'), name, body);
    }
    text
}

/// The settings Claude runs with: the built `~/.claude/settings.json`, else what a build would write
/// (repo base, enabled presets' fragments, private layer), so Cursor and Opencode work without Claude.
pub fn built_settings(home: &Path) -> Value {
    if home.join(LIVE_REL).is_file()
        && let Some(v) = load_json(&home.join(LIVE_REL))
    {
        return v;
    }
    let base = load_json(&claude_dir().join("settings.json")).unwrap_or(serde_json::json!({}));
    let layer = load_json(&home.join(LAYER_REL)).unwrap_or(serde_json::json!({}));
    merge(&with_presets(home, &base), &layer)
}

/// The repo base with the settings fragments of the enabled presets merged over it, in order.
pub fn with_presets(home: &Path, base: &Value) -> Value {
    let on = presets::enabled_quiet(home, presets::current_profile(home));
    presets::fragments(&repo_root(), &on).iter().fold(base.clone(), |acc, f| merge(&acc, f))
}

/// The active output style as (name, file, body), or `None` when the style is `default` or not found.
pub fn output_style(home: &Path) -> Option<(String, PathBuf, String)> {
    let style_name = built_settings(home).get("outputStyle").and_then(|v| v.as_str()).map(ToString::to_string);
    let name = match style_name {
        Some(s) if s.to_lowercase() != "default" => s,
        _ => return None,
    };
    let dir = claude_dir().join("output-styles");
    let entries = fs::read_dir(&dir).ok()?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    files.sort();
    for f in files {
        let (meta, body) = split_frontmatter(&fs::read_to_string(&f).unwrap_or_default());
        let cname = meta.get("name").cloned().unwrap_or_default();
        if f.file_stem().and_then(|s| s.to_str()) == Some(&name) || cname == name {
            return Some((name, f, body.trim_start().to_string()));
        }
    }
    None
}

// Writers
/// Writes the file through a temp file and a rename, so a reader never sees half a file.
/// It creates missing folders and sets `mode` (else keeps the old mode, or 0644 for a new file).
pub fn atomic_write(path: &Path, text: &str, mode: Option<u32>) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut builder = tempfile::Builder::new();
    let prefix = format!(".{}.", path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp"));
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

/// Writes a side file (a sidecar list, the last build) and warns when it cannot.
fn write_or_warn(path: &Path, text: &str, mode: Option<u32>) {
    if let Err(e) = atomic_write(path, text, mode) {
        warn(&format!("could not write {}: {e}", tilde(path)));
    }
}

/// Writes a generated file unless it already matches. A file that is a link or has no
/// `generated-by` marker is the user's and is left alone with a warning. Honors the dry run.
pub fn write_generated(path: &Path, text: &str, mode: Option<u32>) {
    let is_sym = is_symlink(path);
    let has_mark = read_text(path).is_some_and(|t| t.contains(MARK));
    if is_sym || (path.exists() && !has_mark) {
        warn(&format!("{} exists and is not ours, left untouched", tilde(path)));
        return;
    }
    if path.exists()
        && let Some(old) = read_text(path)
        && old == text
    {
        let same_mode = mode.is_none_or(|m| {
            fs::metadata(path).is_ok_and(|md| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    md.permissions().mode() & 0o777 == m
                }
                #[cfg(not(unix))]
                {
                    let _ = (m, md);
                    true
                }
            })
        });
        if mode.is_none() || same_mode {
            skip(&tilde(path));
            return;
        }
    }
    if dry() {
        info(&format!("would write {}", tilde(path)));
        return;
    }
    match atomic_write(path, text, mode) {
        Ok(()) => wrote(&format!("wrote {}", tilde(path))),
        Err(e) => warn(&format!("could not write {}: {e}", tilde(path))),
    }
}

/// Deletes a generated file (one with the `generated-by` marker, not a link) and says why.
pub fn remove_generated(path: &Path, why: &str) {
    let is_sym = is_symlink(path);
    let has_mark = read_text(path).is_some_and(|t| t.contains(MARK));
    if path.is_file() && !is_sym && has_mark {
        if dry() {
            info(&format!("would remove {}", tilde(path)));
            return;
        }
        let _ = fs::remove_file(path);
        ok(&format!("removed {} ({})", tilde(path), why));
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

/// Saves `new` over `old` when they differ, after a timestamped `.bak` copy of the old file.
/// `private` makes the file mode 0600. A link is left alone. Honors the dry run.
pub fn save_json(path: &Path, old: &Value, new: &Value, private: bool) {
    if new == old {
        skip(&tilde(path));
        return;
    }
    if is_symlink(path) {
        warn(&format!("{} is a symlink (yours), left untouched", tilde(path)));
        return;
    }
    if dry() {
        info(&format!("would update {}", tilde(path)));
        return;
    }
    if path.exists() {
        let bak = path.with_file_name(format!("{}.bak-{}", path.file_name().unwrap().to_str().unwrap(), unix_ts()));
        let _ = atomic_write(
            &bak,
            &fs::read_to_string(path).unwrap_or_default(),
            Some(fs::metadata(path).map_or(0o644, |m| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    m.permissions().mode() & 0o777
                }
                #[cfg(not(unix))]
                {
                    let _ = m;
                    0o644
                }
            })),
        );
    }
    let mode = if private { Some(0o600) } else { None };
    match atomic_write(path, &dump(new), mode) {
        Ok(()) => wrote(&format!("updated {}", tilde(path))),
        Err(e) => warn(&format!("could not write {}: {e}", tilde(path))),
    }
}

fn unix_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// MCP
/// True when Cursor MCP servers are on (`DOTFILES_MCP`, else the `CURSOR_MCP` setting).
pub fn mcp_wanted(home: &Path) -> bool {
    let v = env::var("DOTFILES_MCP").unwrap_or_default().trim().to_lowercase();
    let v = if v.is_empty() { setting(home, "CURSOR_MCP").trim().to_lowercase() } else { v };
    matches!(v.as_str(), "1" | "on" | "yes" | "true")
}
/// True when Opencode MCP servers are on (`DOTFILES_MCP`, else the `OPENCODE_MCP` setting).
pub fn opencode_mcp_wanted(home: &Path) -> bool {
    let v = env::var("DOTFILES_MCP").unwrap_or_default().trim().to_lowercase();
    let v = if v.is_empty() { setting(home, "OPENCODE_MCP").trim().to_lowercase() } else { v };
    matches!(v.as_str(), "1" | "on" | "yes" | "true")
}

// Cursor parts
/// Writes the Cursor rule files in `~/.cursor/rules` (core rules and reply style), or only checks
/// them when the rules were pasted into Cursor User Rules.
pub fn cursor_rules(home: &Path, profile: Profile) {
    let rules = home.join(".cursor").join("rules");
    let mark = home.join(USER_RULES_MARK);
    if mark.exists() {
        for f in ["claude-global.mdc", "claude-output-style.mdc"] {
            remove_generated(&rules.join(f), "the rules are in Cursor User Rules");
        }
        if read_text(&mark).unwrap_or_default().trim() == sha(&user_rules_text(home, profile)) {
            skip("Cursor User Rules (pasted in Cursor Settings)");
        } else {
            warn("the rules changed. Run cursor-rules, then paste in Cursor Settings > Rules > User Rules");
        }
        return;
    }
    let overlay = owner_overlay(home, profile);
    let (desc, mode) = if overlay.is_some() {
        ("Global rules from the dotfiles repo (claude/CLAUDE.md) and the owner's private rules", Some(0o600))
    } else {
        if profile == Profile::Owner {
            warn(
                "no ~/.config/dotfiles/owner.md, so the Cursor rule has no owner-only rules. Add them with: dots settings",
            );
        }
        ("Global rules from the dotfiles repo (claude/CLAUDE.md)", None)
    };
    let mut src_labels: Vec<&str> = vec!["claude/CLAUDE.md"];
    if has_preset_rules(home, profile) {
        src_labels.push("claude/presets");
    }
    if overlay.is_some() {
        src_labels.push("~/.config/dotfiles/owner.md");
    }
    let body = format!("{}{}{}", stamp(&src_labels), CURSOR_NOTE, rules_text(home, profile));
    write_generated(&rules.join("claude-global.mdc"), &mdc(desc, &body), mode);

    let target = rules.join("claude-output-style.mdc");
    let Some((name, style_file, sbody)) = output_style(home) else {
        remove_generated(&target, "its source is gone");
        return;
    };
    let from = format!("claude/output-styles/{}", style_file.file_name().unwrap_or_default().to_string_lossy());
    let body = format!("{}# Reply style: {name}\n\n{sbody}", stamp(&[&from]));
    write_generated(&target, &mdc(&format!("Reply style \"{name}\" (Claude outputStyle)"), &body), None);
}

/// The `.md` files directly in `dir`, sorted. A missing folder gives none.
fn md_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> =
        entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")).collect();
    files.sort();
    files
}

/// What a harness needs to turn one `claude/agents/*.md` into its own agent file.
struct AgentSrc<'a> {
    /// The agent name: the front matter `name`, else the file name.
    name: &'a str,
    meta: &'a BTreeMap<String, String>,
    /// The tools the agent lists.
    tools: BTreeSet<&'a str>,
}

/// Writes `<dst>/<name>.md` for each `claude/agents/*.md` (its front matter from `front`) and removes
/// generated files in `dst` whose source is gone.
fn sync_agents(dst: &Path, front: impl Fn(&AgentSrc) -> Vec<String>) {
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for src in md_files(&claude_dir().join("agents")) {
        let file = src.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if file == "README.md" {
            continue;
        }
        let (meta, body) = split_frontmatter(&fs::read_to_string(&src).unwrap_or_default());
        let name =
            meta.get("name").cloned().unwrap_or_else(|| src.file_stem().unwrap_or_default().to_string_lossy().into());
        let tools = meta
            .get("tools")
            .map_or_else(BTreeSet::new, |t| t.split(',').map(str::trim).filter(|t| !t.is_empty()).collect());
        let lines = front(&AgentSrc { name: &name, meta: &meta, tools });
        let text = format!(
            "---\n{}\n---\n{}{}",
            lines.join("\n"),
            stamp(&[&format!("claude/agents/{file}")]),
            body.trim_start()
        );
        wanted.insert(format!("{name}.md"));
        write_generated(&dst.join(format!("{name}.md")), &text, None);
    }
    for f in md_files(dst) {
        if !wanted.contains(f.file_name().unwrap_or_default().to_string_lossy().as_ref()) {
            remove_generated(&f, "its source is gone");
        }
    }
}

/// Writes `~/.cursor/agents/<name>.md` from `claude/agents/*.md` and removes generated ones whose source is gone.
pub fn cursor_agents(home: &Path) {
    sync_agents(&home.join(".cursor").join("agents"), |a| {
        let mut lines = vec![format!("name: {}", a.name)];
        if let Some(desc) = a.meta.get("description") {
            lines.push(format!("description: {}", yaml_str(desc)));
        }
        lines.push("model: inherit".to_string());
        if !a.tools.is_empty() && !a.tools.iter().any(|t| WRITE_TOOLS.contains(t)) {
            lines.push("readonly: true".to_string());
        }
        lines
    });
}

// Not Python's `shlex.quote` byte for byte: the empty string stays empty (Python gives `''`) and a
// `'` becomes `'\''` (Python: `'"'"'`). Both forms are the same shell word, the one input is always
// the non-empty guard path, and the result only lands in hooks.json, which is rebuilt every run.
// The Python tool is gone, so nothing compares against it.
fn shlex_quote(s: &str) -> String {
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || b".,/:=_-@%+".contains(&b)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Puts the guard hook entries into `~/.cursor/hooks.json`, replacing our old ones and keeping the user's.
pub fn cursor_hooks(home: &Path) {
    let path = home.join(".cursor").join("hooks.json");
    let old = load_json(&path);
    if old.is_none() {
        if path.exists() {
            warn(&format!("{} is not valid JSON, left untouched", tilde(&path)));
        }
        return;
    }
    let old = old.unwrap();
    let mut new = old.clone();
    new.as_object_mut().unwrap().entry("version".to_string()).or_insert(serde_json::json!(1));
    let hooks = new.as_object_mut().unwrap().entry("hooks".to_string()).or_insert(serde_json::json!({}));
    if !hooks.is_object() {
        warn(&format!("{} has a non-object `hooks`, left untouched", tilde(&path)));
        return;
    }
    // drop our entries
    let mut slots: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(hobj) = hooks.as_object_mut() {
        let keys: Vec<String> = hobj.keys().cloned().collect();
        for event in keys {
            let entries = hobj.get(&event).and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let ours: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.get("command").and_then(|c| c.as_str()).is_some_and(|s| s.contains("guard_cursor")))
                .map(|(i, _)| i)
                .collect();
            if !ours.is_empty() {
                slots.insert(event.clone(), ours[0]);
            }
            let filtered: Vec<Value> =
                entries.into_iter().enumerate().filter(|(i, _)| !ours.contains(i)).map(|(_, e)| e).collect();
            if filtered.is_empty() {
                hobj.remove(&event);
            } else {
                hobj.insert(event, Value::Array(filtered));
            }
        }
    }
    let q = shlex_quote(guard_path().to_str().unwrap_or(""));
    let guard = guard_path();
    if guard.is_file() {
        // `tool` mode runs the shared checks G14-G17 (src/bin/guard_cursor.rs). preToolUse carries the
        // deny checks for the shell, read and grep tools; beforeShellExecution and beforeMCPExecution
        // carry G16, because Cursor enforces `ask` only on those two (docs: cursor.com/docs/hooks).
        // beforeReadFile carries G15 for reads with the documented `file_path`, so a renamed
        // `tool_input` key on preToolUse cannot skip the secrets check.
        let hook_events = [
            ("beforeSubmitPrompt", "prompt", None::<&str>),
            ("subagentStart", "subagent", None),
            ("preToolUse", "subagent", Some("^(Task|Subagent|mcp_task)$")),
            ("preToolUse", "tool", Some("^(Shell|Read|Grep)$")),
            ("beforeShellExecution", "tool", None),
            ("beforeMCPExecution", "tool", None),
            ("beforeReadFile", "tool", None),
            ("preCompact", "compact", None),
            ("afterAgentResponse", "response", None),
        ];
        let mut placed: BTreeMap<&str, usize> = BTreeMap::new();
        for (event, mode, matcher) in hook_events {
            let mut entry = serde_json::json!({"command": format!("test -r {} || exit 0; sh {} --hook guard_cursor {}", q, q, mode), "timeout": 10});
            if let Some(m) = matcher {
                entry.as_object_mut().unwrap().insert("matcher".to_string(), Value::String(m.to_string()));
            }
            let hooks = new.get_mut("hooks").unwrap().as_object_mut().unwrap();
            let entries = hooks.entry(event.to_string()).or_insert(serde_json::json!([])).as_array_mut().unwrap();
            // an event can hold several of our entries: each goes after the ones placed before it
            let done = placed.entry(event).or_insert(0);
            let idx = slots.get(event).map_or(entries.len(), |s| s + *done);
            *done += 1;
            if idx <= entries.len() {
                entries.insert(idx, entry);
            } else {
                entries.push(entry);
            }
        }
    }
    save_json(&path, &old, &new, false);
}

/// Adds the servers of `cursor/mcp.json` that are missing from `~/.cursor/mcp.json`. Opt-in.
pub fn cursor_mcp(home: &Path) {
    if !mcp_wanted(home) {
        info("MCP servers from cursor/mcp.json are opt-in: turn on Cursor MCP in dots settings");
        return;
    }
    let ours = load_json(&cursor_src_dir().join("mcp.json"))
        .unwrap_or(serde_json::json!({}))
        .get("mcpServers")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let path = home.join(".cursor").join("mcp.json");
    let old = load_json(&path);
    if old.is_none() {
        if path.exists() {
            warn(&format!("{} is not valid JSON, left untouched", tilde(&path)));
        }
        return;
    }
    let old = old.unwrap();
    let mut new = old.clone();
    let servers = new.as_object_mut().unwrap().entry("mcpServers".to_string()).or_insert(serde_json::json!({}));
    if let Some(servers) = servers.as_object_mut()
        && let Some(omap) = ours.as_object()
    {
        for (name, cfg) in omap {
            servers.entry(name.clone()).or_insert(cfg.clone());
        }
    }
    save_json(&path, &old, &new, false);
}

/// A Claude permission rule as a Cursor CLI rule (`Shell(cmd)`, `Read(path)`), or `None` when Cursor has no match.
pub fn translate_permission(rule: &str, kind: &str) -> Option<String> {
    let (tool, arg) = split_rule(rule)?;
    match tool {
        "Bash" => Some(match parse_bash(arg)? {
            BashRule::Prefix(cmd) => format!("Shell({cmd})"),
            BashRule::Exact { exe, rest } => format!("Shell({exe}:{rest})"),
        }),
        "Read" => Some(format!("Read({})", rule_path(arg, kind))),
        "Edit" | "Write" => Some(format!("Write({})", rule_path(arg, kind))),
        "WebFetch" => arg.strip_prefix("domain:").map(|d| format!("WebFetch({d})")),
        _ => None,
    }
}

/// A Claude rule `Tool(arg)` as (tool, arg).
fn split_rule(rule: &str) -> Option<(&str, &str)> {
    let caps = RULE_RE.captures(rule.trim())?;
    // The captures borrow `rule.trim()`, which lives as long as `rule`.
    Some((caps.get(1)?.as_str(), caps.get(2)?.as_str()))
}

/// The two forms of a Bash rule that both harnesses can express.
enum BashRule {
    /// `git status:*` or `git status *`: the command and anything after it.
    Prefix(String),
    /// `git push origin main`: the program and its exact arguments.
    Exact { exe: String, rest: String },
}

/// The Bash rule argument as a `BashRule`, or `None` when it uses a wildcard in the middle or is empty.
fn parse_bash(arg: &str) -> Option<BashRule> {
    if WILDCARD_END_RE.is_match(arg) {
        let cmd = WILDCARD_END_RE.replace(arg, "").trim().to_string();
        return (!cmd.is_empty() && !cmd.contains('*')).then_some(BashRule::Prefix(cmd));
    }
    let trimmed = arg.trim();
    let (exe, rest) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
    let (exe, rest) = (exe.trim(), rest.trim());
    (!exe.is_empty() && !rest.is_empty() && !arg.contains('*'))
        .then(|| BashRule::Exact { exe: exe.to_string(), rest: rest.to_string() })
}

/// A path of a Read, Edit or Write rule. A deny rule on a relative path matches it anywhere.
fn rule_path(arg: &str, kind: &str) -> String {
    let path = arg.strip_prefix("./").unwrap_or(arg);
    if kind == "deny" && !path.starts_with('/') && !path.starts_with('~') && !path.starts_with("**") {
        format!("**/{path}")
    } else {
        path.to_string()
    }
}

/// Merges translated permissions, the no-attribution flags and the status line into
/// `~/.cursor/cli-config.json`. Rules it added before are tracked in a sidecar file.
pub fn cursor_cli_config(home: &Path) -> Result<(), ConfigError> {
    let path = home.join(".cursor").join("cli-config.json");
    let Some(old) = load_json(&path) else {
        if path.exists() {
            warn(&format!("{} is not valid JSON, left untouched", tilde(&path)));
        }
        return Ok(());
    };
    let mut new = old.clone();
    new.as_object_mut().unwrap().entry("version".to_string()).or_insert(serde_json::json!(1));
    new.as_object_mut().unwrap().entry("editor".to_string()).or_insert(serde_json::json!({"vimMode": false}));
    // ensure `permissions` exists so the translation below can extend it
    new.as_object_mut().unwrap().entry("permissions".to_string()).or_insert(serde_json::json!({}));
    let mut new_mut = new.clone();
    let src = load_json(&claude_dir().join("settings.json")).unwrap_or(serde_json::json!({}));
    let src_perms = src.get("permissions").cloned().unwrap_or(serde_json::json!({}));
    let side = home.join(".cursor").join(MANAGED);
    let managed = load_json(&side).unwrap_or(serde_json::json!({}));
    let managed = if managed.is_object() { managed } else { serde_json::json!({}) };
    let mut added: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let new_perms = new_mut.get_mut("permissions").unwrap();
    for kind in ["allow", "deny"] {
        let have = new_perms
            .as_object_mut()
            .unwrap()
            .entry(kind.to_string())
            .or_insert(serde_json::json!([]))
            .as_array_mut()
            .ok_or(ConfigError::NotAList(kind))?;
        let rules = src_perms.get(kind).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let want: Vec<String> =
            rules.iter().filter_map(|r| r.as_str()).filter_map(|r| translate_permission(r, kind)).collect();
        let prev: BTreeSet<String> = managed
            .get(kind)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let mut have_vec: Vec<String> = have.iter().filter_map(|x| x.as_str().map(String::from)).collect();
        have_vec.retain(|r| want.contains(r) || !prev.contains(r));
        for m in &want {
            if !have_vec.contains(m) {
                have_vec.push(m.clone());
            }
        }
        *have = have_vec.clone().into_iter().map(Value::String).collect();
        added.insert(kind.to_string(), want);
    }
    // attribution
    let attr = new_mut
        .as_object_mut()
        .unwrap()
        .entry("attribution".to_string())
        .or_insert(serde_json::json!({}))
        .as_object_mut()
        .unwrap();
    attr.insert("attributeCommitsToAgent".to_string(), Value::Bool(false));
    attr.insert("attributePRsToAgent".to_string(), Value::Bool(false));
    // statusLine: the Rust `statusline` bin, through the launcher (same as claude/settings.json)
    let launcher = home.join(".claude").join("hooks").join("rust-bin");
    // Older runs wrote `~/.claude/statusline.sh` (gone) and then ran the launcher through bash (slower
    // to start than sh): replace those two, keep any other command.
    let old_statusline = new_mut.get("statusLine").is_some_and(|s| {
        s.get("command")
            .and_then(|c| c.as_str())
            .is_some_and(|c| c.contains("statusline.sh") || c == "bash ~/.claude/hooks/rust-bin statusline")
    });
    if (new_mut.get("statusLine").is_none() || old_statusline) && launcher.exists() {
        new_mut.as_object_mut().unwrap().insert(
            "statusLine".to_string(),
            serde_json::json!({"type":"command","command": "sh ~/.claude/hooks/rust-bin statusline","padding":0}),
        );
    }
    let old_clone = old;
    save_json(&path, &old_clone, &new_mut, true);
    // sidecar
    if !dry()
        && let Some(after) = load_json(&path)
        && after == new_mut
        && managed != serde_json::to_value(&added).ok().unwrap_or(Value::Null)
    {
        write_or_warn(&side, &(serde_json::to_string_pretty(&added).unwrap_or_default() + "\n"), None);
    }
    Ok(())
}

// Opencode
/// Writes `~/.config/opencode/agents/<name>.md` from `claude/agents/*.md` and removes generated ones whose source is gone.
pub fn opencode_agents(home: &Path) {
    sync_agents(&home.join(".config").join("opencode").join("agents"), |a| {
        let mut lines = Vec::new();
        if let Some(desc) = a.meta.get("description") {
            lines.push(format!("description: {}", yaml_str(desc)));
        }
        lines.push("mode: subagent".to_string());
        lines.push("permission:".to_string());
        // Bash alone does not count here: Opencode's `edit` permission covers file edits only.
        if !["Write", "Edit", "MultiEdit", "NotebookEdit"].iter().any(|t| a.tools.contains(t)) {
            lines.push("  edit: deny".to_string());
        }
        lines.push("  skill: allow".to_string());
        lines
    });
}

/// The guard plugin text (`opencode_plugin.ts.tmpl`) with the launcher path filled in.
const OPENCODE_PLUGIN: &str = include_str!("opencode_plugin.ts.tmpl");

/// Write `~/.config/opencode/plugins/dots-guard.ts`: Opencode's before-tool hook runs the Rust guard
/// (`rust-bin --hook guard pretool`) and a deny or ask throws, which blocks the call. A file with
/// that name and no `generated-by` marker is the user's and stays. Without the launcher the generated
/// plugin is removed.
pub fn opencode_plugin(home: &Path) {
    let path = home.join(".config").join("opencode").join("plugins").join("dots-guard.ts");
    let guard = guard_path();
    if !guard.is_file() {
        remove_generated(&path, "the guard launcher is missing");
        return;
    }
    // a JSON string is a valid TypeScript string literal
    let literal = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    let sh = plugin_sh(dots::os::is_windows(), dots::os::which("sh"));
    let text =
        OPENCODE_PLUGIN.replace("__RUST_BIN__", &literal(&guard.to_string_lossy())).replace("__SH__", &literal(&sh));
    write_generated(&path, &text, None);
}

/// The sh the plugin starts, found when the plugin is written. The launcher is plain POSIX sh, and sh
/// starts faster than Homebrew bash. On Windows a bare name can find a program in System32 first (bash
/// there is the WSL launcher), so the plugin gets the full path of the Git Bash `sh.exe` (`usr\bin`)
/// on the PATH of `dots`, never a System32 path. Empty means the plugin starts `sh` by name: always on
/// macOS and Linux, and on Windows when no usable sh is on PATH.
fn plugin_sh(windows: bool, found: Option<PathBuf>) -> String {
    if !windows {
        return String::new();
    }
    match found {
        Some(p) if !p.to_string_lossy().to_lowercase().replace('\\', "/").contains("/system32/") => {
            p.to_string_lossy().into_owned()
        }
        _ => String::new(),
    }
}

/// A Claude permission rule as an Opencode (tool, pattern, action) triple, or `None` when Opencode has no match.
pub fn opencode_permission(rule: &str, kind: &str) -> Option<(String, String, String)> {
    let (tool, arg) = split_rule(rule)?;
    let (cat, pattern, effect) = match tool {
        "Bash" => match parse_bash(arg)? {
            BashRule::Prefix(cmd) => ("bash", format!("{cmd} *"), kind),
            BashRule::Exact { exe, rest } => ("bash", format!("{exe} {rest}"), kind),
        },
        "Read" => ("read", rule_path(arg, kind), kind),
        "Edit" | "Write" => ("edit", rule_path(arg, kind), kind),
        "SuggestPluginInstall" | "SearchPlugins" | "SuggestSkills" => ("tool", tool.to_string(), "deny"),
        _ => return None,
    };
    Some((cat.to_string(), pattern, effect.to_string()))
}

/// Merges translated permissions and (when on) the MCP servers into `~/.config/opencode/opencode.jsonc`.
/// Rules it added before are tracked in a sidecar file.
pub fn opencode_config(home: &Path) {
    let path = home.join(".config").join("opencode").join("opencode.jsonc");
    let old = load_json(&path);
    if old.is_none() {
        if path.exists() {
            warn(&format!("{} is not valid JSON, left untouched", tilde(&path)));
        }
        return;
    }
    let old = old.unwrap();
    let mut new = old.clone();
    let src = load_json(&claude_dir().join("settings.json")).unwrap_or(serde_json::json!({}));
    let src_perms = src.get("permissions").cloned().unwrap_or(serde_json::json!({}));
    let perms = new.as_object_mut().unwrap().entry("permission".to_string()).or_insert(serde_json::json!({})).clone();
    let mut perms = if perms.is_object() { perms } else { serde_json::json!({}) };
    let mut want: serde_json::Map<String, Value> = serde_json::Map::new();
    for cat in ["bash", "read", "edit", "tool"] {
        want.insert(cat.to_string(), serde_json::json!({}));
    }
    for kind in ["allow", "deny"] {
        let rules = src_perms.get(kind).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for r in rules {
            // Deny rules come second, so a pattern in both lists ends as a deny, as in Claude Code.
            if let Some(rulestr) = r.as_str()
                && let Some((cat, pat, effect)) = opencode_permission(rulestr, kind)
                && let Some(obj) = want.entry(cat).or_insert(serde_json::json!({})).as_object_mut()
            {
                obj.insert(pat, Value::String(effect));
            }
        }
    }
    want.entry("skill".to_string())
        .or_insert(serde_json::json!({}))
        .as_object_mut()
        .unwrap()
        .insert("*".to_string(), Value::String("allow".to_string()));
    want.entry("ask".to_string())
        .or_insert(serde_json::json!({}))
        .as_object_mut()
        .unwrap()
        .insert("*".to_string(), Value::String("allow".to_string()));

    let side = home.join(".config").join("opencode").join(MANAGED);
    let managed = load_json(&side).unwrap_or(serde_json::json!({}));
    let managed = if managed.is_object() { managed } else { serde_json::json!({}) };
    let perms_obj = perms.as_object_mut().unwrap();
    for (cat, ours) in &want {
        let have = perms_obj.entry(cat.clone()).or_insert(serde_json::json!({}));
        if !have.is_object() {
            *have = serde_json::json!({});
        }
        let have_obj = have.as_object_mut().unwrap();
        let ours_obj = ours.as_object().cloned().unwrap_or_default();
        // A rule we wrote before and the user left alone goes when we drop it or change its effect.
        if let Some(was) = managed.get(cat).and_then(|v| v.as_object()) {
            for (pat, eff) in was {
                if ours_obj.get(pat) != Some(eff) && have_obj.get(pat).and_then(|v| v.as_str()) == eff.as_str() {
                    have_obj.remove(pat);
                }
            }
        }
        for (pat, eff) in ours_obj {
            if let Some(es) = eff.as_str()
                && !have_obj.contains_key(&pat)
            {
                have_obj.insert(pat.clone(), Value::String(es.to_string()));
            }
        }
    }
    new.as_object_mut().unwrap().insert("permission".to_string(), perms);

    if opencode_mcp_wanted(home) {
        let mcp = new.as_object_mut().unwrap().entry("mcp".to_string()).or_insert(serde_json::json!({})).clone();
        let mut mcp = if mcp.is_object() { mcp } else { serde_json::json!({}) };
        let ours = load_json(&cursor_src_dir().join("mcp.json"))
            .unwrap_or(serde_json::json!({}))
            .get("mcpServers")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        if let (Some(mcp_obj), Some(ours_obj)) = (mcp.as_object_mut(), ours.as_object()) {
            for (name, cfg) in ours_obj {
                if !mcp_obj.contains_key(name) {
                    if let Some(url) = cfg.get("url").and_then(|v| v.as_str()) {
                        mcp_obj.insert(name.clone(), serde_json::json!({"type":"remote","url":url,"enabled":true}));
                    } else {
                        mcp_obj.insert(name.clone(), cfg.clone());
                    }
                }
            }
        }
        new.as_object_mut().unwrap().insert("mcp".to_string(), mcp);
    }
    save_json(&path, &old, &new, false);
    if !dry()
        && let Some(after) = load_json(&path)
        && after == new
        && managed != serde_json::to_value(&want).unwrap_or(Value::Null)
    {
        write_or_warn(&side, &(serde_json::to_string_pretty(&want).unwrap_or_default() + "\n"), None);
    }
}

/// Writes the copper theme file and sets it in `tui.json`, unless the user already names another theme.
pub fn opencode_theme(home: &Path) {
    if setting(home, "THEME") == "plain" {
        skip("Opencode theme (THEME=plain)");
        return;
    }
    let themes = home.join(".config").join("opencode").join("themes");
    let text = include_str!("../themes/copper.json");
    let path = themes.join("copper.json");
    if read_text(&path).as_deref() == Some(text) {
        skip(&tilde(&path));
    } else if dry() {
        info(&format!("would write {}", tilde(&path)));
    } else {
        match atomic_write(&path, text, None) {
            Ok(()) => wrote(&format!("wrote {}", tilde(&path))),
            Err(e) => warn(&format!("could not write {}: {e}", tilde(&path))),
        }
    }

    let path = home.join(".config").join("opencode").join("tui.json");
    let old = if path.exists() {
        if let Some(v) = load_json(&path) {
            v
        } else {
            warn(&format!("{} is not valid JSON, left untouched", tilde(&path)));
            return;
        }
    } else {
        serde_json::json!({})
    };
    // our own file from an older layout: the palette inline plus a `_dotfiles` marker
    let legacy = old.get("_dotfiles").and_then(|d| d.get("generated")) == Some(&Value::Bool(true));
    if let Some(theme) = old.get("theme")
        && !theme.is_null()
        && theme.as_str() != Some("copper")
        && !legacy
    {
        let copperfile = home.join(".config").join("opencode").join("themes").join("copper.json");
        let shown = match theme {
            Value::String(s) => s.clone(),
            other => pycompat::py_repr(other),
        };
        info(&format!(
            "{} uses your Opencode theme ({}); the copper one is in {}",
            tilde(&path),
            shown,
            tilde(&copperfile)
        ));
        return;
    }
    // keep every other key of the file; drop the markers of the older layouts
    let mut new = old.clone();
    let obj = new.as_object_mut().unwrap();
    obj.retain(|k, _| !["_dotfiles", "theme", "generated-by", "source"].contains(&k.as_str()));
    obj.entry("$schema".to_string()).or_insert(serde_json::json!("https://opencode.ai/tui.json"));
    obj.insert("theme".to_string(), Value::String("copper".to_string()));
    save_json(&path, &old, &new, false);
}

/// Mirrors the global Opencode config and agents into each listed project's `.opencode/` as links.
/// With no list it reads `DOTFILES_PROJECTS`, then the `OPENCODE_PROJECTS` setting.
pub fn opencode_projects(home: &Path, projects: Option<Vec<String>>) {
    let projects = projects.unwrap_or_else(|| {
        let v = env::var("DOTFILES_PROJECTS").unwrap_or_default().trim().to_string();
        let v = if v.is_empty() { setting(home, "OPENCODE_PROJECTS") } else { v };
        v.split_whitespace().map(String::from).filter(|s| !s.is_empty()).collect()
    });
    if projects.is_empty() {
        return;
    }
    let global_cfg = home.join(".config").join("opencode").join("opencode.jsonc");
    let global_agents = home.join(".config").join("opencode").join("agents");
    for proj in projects {
        // a leading ~ means the home folder (Path.expanduser)
        let proj = if proj == "~" {
            home.to_path_buf()
        } else if let Some(rest) = proj.strip_prefix("~/") {
            home.join(rest)
        } else {
            PathBuf::from(&proj)
        };
        if !proj.is_absolute() {
            warn(&format!("skipping non-absolute Opencode project path: {}", proj.display()));
            continue;
        }
        let popencode = proj.join(".opencode");
        let _ = fs::create_dir_all(&popencode);
        let pcfg = popencode.join("opencode.jsonc");
        if !pcfg.exists() {
            #[cfg(unix)]
            {
                if let Err(e) = std::os::unix::fs::symlink(&global_cfg, &pcfg) {
                    warn(&format!("couldn't link {}: {}", pcfg.display(), e));
                } else {
                    ok(&format!("linked {} → {}", pcfg.display(), global_cfg.display()));
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (&global_cfg, &pcfg);
            }
        }
        if global_agents.is_dir() {
            let pagents = popencode.join("agents");
            let _ = fs::create_dir_all(&pagents);
            if let Ok(entries) = fs::read_dir(&global_agents) {
                let mut agent_files: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
                    .collect();
                agent_files.sort();
                for src in agent_files {
                    let dst = pagents.join(src.file_name().unwrap());
                    if dst.exists() {
                        continue;
                    }
                    #[cfg(unix)]
                    {
                        if let Err(e) = std::os::unix::fs::symlink(&src, &dst) {
                            warn(&format!("couldn't link {}: {}", dst.display(), e));
                        } else {
                            ok(&format!("linked {} → {}", dst.display(), src.display()));
                        }
                    }
                }
            }
        }
    }
}

/// Serializes the swap of the panic hook in `run_part`.
static PART_HOOK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A part of a command: it reports its own problems with `warn`, and returns an error only when it
/// cannot go on.
type Part<'a> = Box<dyn FnOnce() -> Result<(), ConfigError> + 'a>;

/// A part that cannot fail: it only warns.
fn part<'a>(f: impl FnOnce() + 'a) -> Part<'a> {
    Box::new(move || {
        f();
        Ok(())
    })
}

/// Runs the parts of a command in order. One bad file must not stop the other parts: a failing part
/// is reported as `skipped a part: <why>` and counted. A panic is caught as a last safety net and
/// reported the same way. Returns how many parts failed.
fn run_parts(parts: Vec<Part<'_>>) -> usize {
    let _g = PART_HOOK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // the warning below is the report, not a backtrace
    let mut failed = 0;
    for p in parts {
        let why = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(p)) {
            Ok(Ok(())) => continue,
            Ok(Err(e)) => e.to_string(),
            Err(p) => p
                .downcast_ref::<&str>()
                .map(ToString::to_string)
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown error".to_string()),
        };
        warn(&format!("skipped a part: {why}"));
        failed += 1;
    }
    std::panic::set_hook(prev);
    failed
}

/// The `cursor` command: runs each Cursor part. Returns 1 when a part failed, else 0.
pub fn cmd_cursor(args: &CursorArgs) -> i32 {
    let home = paths::home();
    header(&format!("Cursor config from {} (profile: {})", tilde(&claude_dir()), args.profile));
    let parts = vec![
        part(|| cursor_rules(&home, args.profile)),
        part(|| cursor_agents(&home)),
        part(|| cursor_hooks(&home)),
        part(|| cursor_mcp(&home)),
        Box::new(|| cursor_cli_config(&home)) as Part,
    ];
    i32::from(run_parts(parts) > 0)
}

/// Arguments of the `cursor` command.
#[derive(Parser, Debug)]
pub struct CursorArgs {
    /// owner pulls in the private owner.md rules
    #[arg(long, default_value_t = Profile::Shared, value_parser = str::parse::<Profile>)]
    pub profile: Profile,
    /// report what would change without writing
    #[arg(long)]
    pub dry_run: bool,
}

/// The profile of this machine (`owner` or `shared`), from the settings.
pub fn default_profile() -> Profile {
    presets::current_profile(&paths::home())
}

/// The command that copies stdin to the clipboard on this OS, or `None` when none is installed.
pub fn clipboard_command() -> Option<Vec<String>> {
    clipboard_command_in(env::var("PATH").ok().as_deref())
}

/// `clipboard_command` with the search path given, so a test does not change the process PATH.
pub fn clipboard_command_in(path: Option<&str>) -> Option<Vec<String>> {
    if cfg!(target_os = "macos") {
        return Some(vec!["pbcopy".to_string()]);
    }
    if cfg!(target_os = "windows") {
        return Some(vec!["clip".to_string()]);
    }
    linux_clipboard_in(path)
}

/// The first Linux clipboard tool found on `path`.
fn linux_clipboard_in(path: Option<&str>) -> Option<Vec<String>> {
    for cand in [vec!["wl-copy"], vec!["xclip", "-selection", "clipboard"], vec!["xsel", "--clipboard", "--input"]] {
        if crate_which(cand[0], path) {
            return Some(cand.into_iter().map(String::from).collect());
        }
    }
    None
}
fn crate_which(cmd: &str, path: Option<&str>) -> bool {
    if let Some(path) = path {
        for dir in path.split(':') {
            let p = PathBuf::from(dir).join(cmd);
            if p.is_file() {
                return true;
            }
        }
    }
    false
}

/// The `user-rules` command: prints the Cursor User Rules text, or with `copy` puts it on the
/// clipboard and removes the file copies in `~/.cursor/rules`. Returns 1 when there is no clipboard tool.
pub fn cmd_user_rules(profile: Profile, copy: bool) -> i32 {
    let home = paths::home();
    let text = user_rules_text(&home, profile);
    if !copy {
        print!("{text}");
        return 0;
    }
    let Some(c) = clipboard_command() else {
        warn("no clipboard tool found (install wl-clipboard or xclip). Printing the rules instead.");
        print!("{text}");
        return 1;
    };
    let Ok(mut child) = process::Command::new(&c[0]).args(&c[1..]).stdin(process::Stdio::piped()).spawn() else {
        warn("no clipboard tool found. Printing the rules instead.");
        print!("{text}");
        return 1;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
    write_or_warn(&home.join(USER_RULES_MARK), &(sha(&text) + "\n"), None);
    for f in ["claude-global.mdc", "claude-output-style.mdc"] {
        remove_generated(&home.join(".cursor").join("rules").join(f), "the rules move to Cursor User Rules");
    }
    ok("copied the rules to the clipboard");
    info("Now: Cursor > Cursor Settings > Rules > User Rules > paste > save.");
    info(&format!("To go back to the file copy: rm {} && dots", tilde(&home.join(USER_RULES_MARK))));
    0
}

// Claude settings
/// The value at a key path in a JSON object, or `None` when any step is missing.
pub fn get_path(obj: &Value, path: &[&str]) -> Option<Value> {
    let mut cur = obj;
    for k in path {
        if !cur.is_object() || cur.get(*k).is_none() {
            return None;
        }
        cur = cur.get(*k).unwrap();
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
        "permissions.allow" | "permissions.deny" | "permissions.ask" | "permissions.additionalDirectories"
    )
}

/// The layer laid over the base: objects merge, `null` removes a key, the permission lists join
/// without repeats, and anything else is replaced.
pub fn merge(base: &Value, layer: &Value) -> Value {
    merge_inner(base, layer, &[])
}
fn merge_inner(base: &Value, layer: &Value, path: &[String]) -> Value {
    let mut out = base.clone();
    if let Some(lo) = layer.as_object() {
        if !out.is_object() {
            out = serde_json::json!({});
        }
        let out_obj = out.as_object_mut().unwrap();
        for (k, v) in lo {
            let mut p = path.to_vec();
            p.push(k.clone());
            if v.is_null() {
                out_obj.remove(k);
            } else if out_obj.get(k).is_some_and(Value::is_object) && v.is_object() {
                let merged = merge_inner(out_obj.get(k).unwrap(), v, &p);
                out_obj.insert(k.clone(), merged);
            } else if is_union(&p) && out_obj.get(k).is_some_and(Value::is_array) && v.is_array() {
                let mut arr = out_obj.get(k).unwrap().as_array().cloned().unwrap_or_default();
                for x in v.as_array().unwrap() {
                    if !arr.contains(x) {
                        arr.push(x.clone());
                    }
                }
                out_obj.insert(k.clone(), Value::Array(arr));
            } else {
                out_obj.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// What changed in the live settings since the last build, as a patch for the private layer plus
/// notes (kind, key path, item count) for the user.
pub fn drift(last: &Value, live: &Value) -> (Value, Vec<(String, Vec<String>, usize)>) {
    drift_inner(last, live, &[])
}
fn drift_inner(last: &Value, live: &Value, path: &[String]) -> (Value, Vec<(String, Vec<String>, usize)>) {
    let mut patch = serde_json::json!({});
    let mut notes = Vec::new();
    if let Some(lo) = live.as_object() {
        for (k, v) in lo {
            let mut p = path.to_vec();
            p.push(k.clone());
            let old = last.get(k);
            match old {
                None => {
                    patch.as_object_mut().unwrap().insert(k.clone(), v.clone());
                    notes.push(("set".to_string(), p, 0));
                }
                Some(oldv) if oldv.is_object() && v.is_object() => {
                    let (sub, n) = drift_inner(oldv, v, &p);
                    if !sub.as_object().is_none_or(serde_json::Map::is_empty) {
                        patch.as_object_mut().unwrap().insert(k.clone(), sub);
                    }
                    notes.extend(n);
                }
                Some(oldv) if is_union(&p) && oldv.is_array() && v.is_array() => {
                    let added: Vec<Value> = v
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|x| !oldv.as_array().unwrap().contains(x))
                        .cloned()
                        .collect();
                    let removed_n =
                        oldv.as_array().unwrap().iter().filter(|x| !v.as_array().unwrap().contains(x)).count();
                    if !added.is_empty() {
                        patch.as_object_mut().unwrap().insert(k.clone(), Value::Array(added.clone()));
                        notes.push(("add".to_string(), p.clone(), added.len()));
                    }
                    if removed_n > 0 {
                        notes.push(("removed".to_string(), p.clone(), removed_n));
                    }
                }
                Some(oldv) => {
                    if oldv != v {
                        patch.as_object_mut().unwrap().insert(k.clone(), v.clone());
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
                patch.as_object_mut().unwrap().insert(k.clone(), Value::Null);
                notes.push(("unset".to_string(), p, 0));
            }
        }
    }
    (patch, notes)
}

/// The layer with a `drift` patch applied to it.
pub fn apply_patch(layer: &Value, patch: &Value) -> Value {
    apply_patch_inner(layer, patch, &[])
}
fn apply_patch_inner(layer: &Value, patch: &Value, path: &[String]) -> Value {
    let mut out = layer.clone();
    if let Some(po) = patch.as_object() {
        if !out.is_object() {
            out = serde_json::json!({});
        }
        let out_obj = out.as_object_mut().unwrap();
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
            if is_union(&p) && v.is_array() && out_obj.get(k).is_some_and(Value::is_array) {
                let mut arr = out_obj.get(k).unwrap().as_array().cloned().unwrap_or_default();
                for x in v.as_array().unwrap() {
                    if !arr.contains(x) {
                        arr.push(x.clone());
                    }
                }
                out_obj.insert(k.clone(), Value::Array(arr));
            } else {
                out_obj.insert(k.clone(), v.clone());
            }
        }
    }
    out
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
    let base = load_json(&base_f);
    let layer = load_json(&layer_f);
    if base.is_none() || layer.is_none() {
        let bad = if base.is_none() { &base_f } else { &layer_f };
        warn(&format!("{} is not valid JSON. Fix it; ~/.claude/settings.json left as it is.", tilde(bad)));
        return 1;
    }
    let base = with_presets(home, &base.unwrap());
    let mut layer = layer.unwrap();
    let was_link = is_symlink(&live_f);
    let mut notes = Vec::new();
    if live_f.exists() && !was_link {
        let live = load_json(&live_f);
        if live.is_none() {
            warn(&format!("{} is not valid JSON (Claude may be writing it). Run dots again.", tilde(&live_f)));
            return 1;
        }
        let live = live.unwrap();
        let last = if last_f.exists() {
            load_json(&last_f).unwrap_or_else(|| merge(&base, &layer))
        } else {
            merge(&base, &layer)
        };
        let (patch, n) = drift(&last, &live);
        notes = n;
        if !patch.as_object().is_none_or(serde_json::Map::is_empty) {
            layer = apply_patch(&layer, &patch);
            if dry() {
                info(&format!("would keep {} change(s) in {}", notes.len(), tilde(&layer_f)));
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
    let text = dump(&merge(&base, &layer));
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
        info(&format!("{} was a link into the repo; it is now a built file", tilde(&live_f)));
        let _ = fs::remove_file(&live_f);
    }
    if let Err(e) = atomic_write(&live_f, &text, Some(0o644)) {
        warn(&format!("could not write {}: {e}", tilde(&live_f)));
        return 1;
    }
    wrote(&format!("built {} (repo base + private layer)", tilde(&live_f)));
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
    let base = load_json(&base_f);
    let layer = load_json(&layer_f);
    if base.is_none() || layer.is_none() {
        warn("the repo base or the private layer is not valid JSON; nothing moved");
        return 1;
    }
    let base = base.unwrap();
    let mut layer = layer.unwrap();
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
        warn(&format!("could not write {}: {e}; nothing moved", tilde(&base_f)));
        return 1;
    }
    if let Err(e) = atomic_write(&layer_f, &dump(&layer), Some(0o600)) {
        warn(&format!("could not write {}: {e}; the key is now in both files", tilde(&layer_f)));
        return 1;
    }
    ok(&format!("moved {} to {}. Commit it to share it.", dotted, tilde(&base_f)));
    claude_settings_build(home)
}

/// The reverse of promote: copy a key's value from the live `~/.claude/settings.json` into the
/// private layer, so it stays when the repo base drops it. A note and no change when the live file
/// lacks the key.
pub fn claude_settings_demote(home: &Path, dotted: &str) -> i32 {
    let path: Vec<&str> = dotted.split('.').collect();
    let live_f = home.join(LIVE_REL);
    let layer_f = home.join(LAYER_REL);
    let live = load_json(&live_f);
    let layer = load_json(&layer_f);
    if live.is_none() || layer.is_none() {
        warn("the live settings or the private layer is not valid JSON; nothing moved");
        return 1;
    }
    let (live, layer) = (live.unwrap(), layer.unwrap());
    let Some(value) = get_path(&live, &path) else {
        info(&format!("{} is not in {}; nothing to keep", dotted, tilde(&live_f)));
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
    about = "Build agent config for other harnesses from claude/.",
    long_about = "Build agent config for other harnesses from claude/.\n\n\
        cursor writes generated files under ~/.cursor (rules, agents) and merges keys\n\
        into the JSON files Cursor also writes itself. opencode writes generated files\n\
        under ~/.config/opencode and merges keys into opencode.jsonc and tui.json.\n\
        Neither ever writes under ~/.claude."
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
    /// generate and merge Cursor config under ~/.cursor
    Cursor(CursorArgs),
    /// text for Cursor Settings > Rules > User Rules
    #[command(name = "user-rules")]
    UserRules(UserRulesArgs),
    /// build ~/.claude/settings.json from the repo base and the private layer
    #[command(name = "claude-settings")]
    ClaudeSettings(ClaudeSettingsArgs),
    /// generate and merge Opencode config under ~/.config/opencode
    Opencode(OpencodeArgs),
}

/// Arguments of the `user-rules` command.
#[derive(Parser, Debug)]
pub struct UserRulesArgs {
    /// owner pulls in the private ~/.config/dotfiles/owner.md rules
    #[arg(long, default_value_t = default_profile(), value_parser = str::parse::<Profile>)]
    pub profile: Profile,
    /// copy to the clipboard and drop the file copy
    #[arg(long)]
    pub copy: bool,
    /// report what would change without writing
    #[arg(long)]
    pub dry_run: bool,
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

/// Arguments of the `opencode` command.
#[derive(Parser, Debug)]
pub struct OpencodeArgs {
    /// owner pulls in the private ~/.config/dotfiles/owner.md rules
    #[arg(long, default_value_t = Profile::Shared, value_parser = str::parse::<Profile>)]
    pub profile: Profile,
    /// report what would change without writing
    #[arg(long)]
    pub dry_run: bool,
    /// comma/space-separated absolute project paths to mirror the global config into
    #[arg(long)]
    pub projects: Option<String>,
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
        Commands::Cursor(a) => {
            set_dry(a.dry_run);
            (cmd_cursor_dispatch(&a), "Cursor config")
        }
        Commands::UserRules(a) => {
            set_dry(a.dry_run);
            (cmd_user_rules(a.profile, a.copy), "user-rules")
        }
        Commands::ClaudeSettings(a) => {
            set_dry(a.dry_run);
            (cmd_claude_settings_dispatch(&a), "Claude settings")
        }
        Commands::Opencode(a) => {
            set_dry(a.dry_run);
            (cmd_opencode_dispatch(&a), "Opencode config")
        }
    };
    summary(label);
    rc
}

fn cmd_cursor_dispatch(a: &CursorArgs) -> i32 {
    set_dry(a.dry_run);
    cmd_cursor(a)
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

fn cmd_opencode_dispatch(a: &OpencodeArgs) -> i32 {
    set_dry(a.dry_run);
    let home = paths::home();
    header(&format!("Opencode config from {} (profile: {})", tilde(&claude_dir()), a.profile));
    let projects = a
        .projects
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>());
    let parts = vec![
        part(|| opencode_agents(&home)),
        part(|| opencode_plugin(&home)),
        part(|| opencode_config(&home)),
        part(|| opencode_rules(&home, a.profile)),
        part(|| opencode_theme(&home)),
        part(|| opencode_projects(&home, projects)),
    ];
    i32::from(run_parts(parts) > 0)
}

/// Writes `~/.config/opencode/AGENTS.md`: the rules, enabled presets, owner overlay and reply style.
pub fn opencode_rules(home: &Path, profile: Profile) {
    let path = home.join(".config").join("opencode").join("AGENTS.md");
    if is_symlink(&path) || (path.exists() && !read_text(&path).unwrap_or_default().contains(MARK)) {
        warn(&format!(
            "{} exists and is not ours ({} has no {} marker), left untouched. Move your own text into ~/.config/dotfiles/owner.md (dots settings) and it lands here too.",
            tilde(&path),
            tilde(&path),
            MARK
        ));
        return;
    }
    let overlay = owner_overlay(home, profile);
    let mut srcs: Vec<&str> = vec!["claude/CLAUDE.md"];
    if has_preset_rules(home, profile) {
        srcs.push("claude/presets");
    }
    let mut mode: Option<u32> = None;
    if overlay.is_some() {
        srcs.push("~/.config/dotfiles/owner.md");
        mode = Some(0o600);
    } else if profile == Profile::Owner {
        warn(
            "no ~/.config/dotfiles/owner.md, so the OpenCode rules have no owner-only rules. Add them with: dots settings",
        );
    }
    let stamp_line = stamp(&srcs);
    let mut text = format!("{}{}{}", stamp_line, OPENCODE_NOTE, rules_text(home, profile));
    if let Some((name, _, sbody)) = output_style(home) {
        text = format!("{}\n\n# Reply style: {}\n\n{}", text.trim_end_matches('\n'), name, sbody);
    }
    write_generated(&path, &text, mode);
}

#[cfg(test)]
mod agent_config_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plugin_sh_is_a_full_path_on_windows_only() {
        let git = PathBuf::from(r"C:\Program Files\Git\usr\bin\sh.exe");
        assert_eq!(plugin_sh(true, Some(git.clone())), git.to_string_lossy());
        // a System32 path is never written, in any case or slash style; nothing found means `sh` by name
        assert_eq!(plugin_sh(true, Some(PathBuf::from(r"C:\Windows\System32\sh.exe"))), "");
        assert_eq!(plugin_sh(true, Some(PathBuf::from("C:/WINDOWS/SYSTEM32/sh.exe"))), "");
        assert_eq!(plugin_sh(true, None), "");
        // macOS and Linux keep `sh` by name
        assert_eq!(plugin_sh(false, Some(PathBuf::from("/bin/sh"))), "");
    }
    #[test]
    fn opencode_permission_keeps_the_effect() {
        let t = |cat: &str, pat: &str, eff: &str| Some((cat.to_string(), pat.to_string(), eff.to_string()));
        assert_eq!(opencode_permission("Read(./foo)", "deny"), t("read", "**/foo", "deny"));
        assert_eq!(opencode_permission("Bash(git status:*)", "allow"), t("bash", "git status *", "allow"));
        assert_eq!(
            opencode_permission("Bash(git push origin main)", "allow"),
            t("bash", "git push origin main", "allow")
        );
        // A deny stays a deny, for Bash too: it once came out as an allow.
        assert_eq!(
            opencode_permission("Bash(git push origin main)", "deny"),
            t("bash", "git push origin main", "deny")
        );
        assert_eq!(opencode_permission("Bash(rm:*)", "deny"), t("bash", "rm *", "deny"));
    }
}
