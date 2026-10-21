//! The reverse of the Cursor and Opencode writers: remove what `dots` generated and the entries it
//! merged into the user's own files. A file with no `generated-by` marker, and any entry the user
//! changed, stays. `dots uninstall` calls these.

use crate::out::{info, ok, warn};
use crate::{
    MANAGED, cursor_src_dir, is_guard_hook, is_symlink, load_json, read_text, remove_generated, save_json, setting,
    tilde,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const WHY: &str = "dots uninstall";
const STATUSLINE: &str = "sh ~/.claude/hooks/rust-bin statusline";

/// Load a JSON file, change it, save it. Not valid JSON: warn and leave it.
fn edit_json(path: &Path, private: bool, f: impl FnOnce(&mut Value)) {
    let Some(old) = load_json(path) else {
        warn(&format!("{} is not valid JSON, left untouched", tilde(path)));
        return;
    };
    let mut new = old.clone();
    f(&mut new);
    save_json(path, &old, &new, private);
}

/// Remove one top-level key of a JSON object.
fn drop_key(v: &mut Value, key: &str) {
    if let Some(o) = v.as_object_mut() {
        o.remove(key);
    }
}

/// Remove every generated `*.md` in a folder.
fn remove_generated_md(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")).collect();
    files.sort();
    for f in files {
        remove_generated(&f, WHY);
    }
}

/// Delete a plain file that `dots` wrote without a marker (a sidecar or the theme).
fn remove_plain(path: &Path) {
    if path.is_file() && !is_symlink(path) && fs::remove_file(path).is_ok() {
        ok(&format!("removed {}", tilde(path)));
    }
}

/// The servers of `cursor/mcp.json`, as `(name, config)`.
fn repo_mcp() -> Vec<(String, Value)> {
    let v = load_json(&cursor_src_dir().join("mcp.json")).unwrap_or_else(|| json!({}));
    v.get("mcpServers")
        .and_then(|s| s.as_object())
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// Remove the rules in the sidecar list `kind` from `perms[kind]`.
fn drop_listed(perms: &mut Value, managed: &Value, kind: &str) {
    let was: Vec<&str> = managed
        .get(kind)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    if let Some(list) = perms.get_mut(kind).and_then(|v| v.as_array_mut()) {
        list.retain(|r| !r.as_str().is_some_and(|s| was.contains(&s)));
    }
}

/// Cursor: the generated rules and agents, our hooks, the MCP servers and CLI permissions we added.
pub fn cursor_undo(home: &Path) {
    let cur = home.join(".cursor");
    for f in ["claude-global.mdc", "claude-output-style.mdc"] {
        remove_generated(&cur.join("rules").join(f), WHY);
    }
    remove_generated_md(&cur.join("agents"));
    edit_json(&cur.join("hooks.json"), false, |v| {
        if let Some(h) = v.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            for entries in h.values_mut() {
                if let Some(a) = entries.as_array_mut() {
                    a.retain(|e| !is_guard_hook(e));
                }
            }
            h.retain(|_, e| e.as_array().is_none_or(|a| !a.is_empty()));
        }
    });
    let ours = repo_mcp();
    edit_json(&cur.join("mcp.json"), false, |v| {
        if let Some(s) = v.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
            for (name, cfg) in &ours {
                if s.get(name) == Some(cfg) {
                    s.remove(name);
                }
            }
        }
    });
    let side = cur.join(MANAGED);
    let managed = load_json(&side).unwrap_or_else(|| json!({}));
    edit_json(&cur.join("cli-config.json"), true, |v| {
        if let Some(p) = v.get_mut("permissions") {
            for kind in ["allow", "deny"] {
                drop_listed(p, &managed, kind);
            }
        }
        if v.get("statusLine").and_then(|s| s.get("command")).and_then(|c| c.as_str()) == Some(STATUSLINE) {
            drop_key(v, "statusLine");
        }
    });
    remove_plain(&side);
}

/// Opencode: the generated rules, agents, plugin and theme, plus the permissions and MCP servers we merged.
pub fn opencode_undo(home: &Path) {
    let oc = home.join(".config").join("opencode");
    remove_generated(&oc.join("AGENTS.md"), WHY);
    remove_generated_md(&oc.join("agents"));
    remove_generated(&oc.join("plugins").join("dots-guard.ts"), WHY);
    let theme = oc.join("themes").join("copper.json");
    if read_text(&theme).is_some_and(|t| t == include_str!("../themes/copper.json")) {
        remove_plain(&theme);
    }
    edit_json(&oc.join("tui.json"), false, |v| {
        if v.get("theme").and_then(|t| t.as_str()) == Some("copper") {
            drop_key(v, "theme");
        }
    });
    let side = oc.join(MANAGED);
    let managed = load_json(&side).unwrap_or_else(|| json!({}));
    let ours = repo_mcp();
    edit_json(&oc.join("opencode.jsonc"), false, |v| {
        if let Some(perm) = v.get_mut("permission").and_then(|p| p.as_object_mut()) {
            for (cat, was) in managed.as_object().into_iter().flatten() {
                let (Some(have), Some(was)) = (perm.get_mut(cat).and_then(|h| h.as_object_mut()), was.as_object())
                else {
                    continue;
                };
                for (pat, eff) in was {
                    if have.get(pat) == Some(eff) {
                        have.remove(pat);
                    }
                }
            }
            perm.retain(|_, c| c.as_object().is_none_or(|o| !o.is_empty()));
        }
        if v.get("permission").and_then(|p| p.as_object()).is_some_and(serde_json::Map::is_empty) {
            drop_key(v, "permission");
        }
        if let Some(mcp) = v.get_mut("mcp").and_then(|m| m.as_object_mut()) {
            for (name, cfg) in &ours {
                let ours_entry = match cfg.get("url").and_then(|u| u.as_str()) {
                    Some(url) => json!({"type": "remote", "url": url, "enabled": true}),
                    None => cfg.clone(),
                };
                if mcp.get(name) == Some(&ours_entry) {
                    mcp.remove(name);
                }
            }
        }
        if v.get("mcp").and_then(|m| m.as_object()).is_some_and(serde_json::Map::is_empty) {
            drop_key(v, "mcp");
        }
    });
    remove_plain(&side);
    projects_undo(home);
}

/// The `.opencode` links in each project of `OPENCODE_PROJECTS`. A folder that still holds
/// something of the user's stays.
fn projects_undo(home: &Path) {
    for proj in setting(home, "OPENCODE_PROJECTS").split_whitespace() {
        let proj = match proj.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(proj),
        };
        if !proj.is_absolute() {
            continue;
        }
        let dir = proj.join(".opencode");
        let agents = dir.join("agents");
        let mut links = vec![dir.join("opencode.jsonc")];
        links.extend(fs::read_dir(&agents).into_iter().flatten().flatten().map(|e| e.path()));
        for l in links {
            if is_symlink(&l) && fs::remove_file(&l).is_ok() {
                ok(&format!("removed the link {}", l.display()));
            }
        }
        let _ = fs::remove_dir(&agents);
        if fs::remove_dir(&dir).is_ok() {
            info(&format!("removed {}", dir.display()));
        }
    }
}
