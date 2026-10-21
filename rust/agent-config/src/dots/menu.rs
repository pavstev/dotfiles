//! The settings menu (`dots settings`; `dots private` opens it too). The settings are
//! NAME="value" lines in ~/.config/dotfiles/settings; settings.example lists them all.
//! The private rules are free text in ~/.config/dotfiles/owner.md. Each change saves at
//! once. The first run opens it with the cursor on Done.

use super::{ask, git, java, settings};
use crate::out::{info, ok, step, warn};
use std::fs;
use std::path::{Path, PathBuf};

fn s(v: &str) -> String {
    v.to_string()
}

/// Write the git name and email to ~/.gitconfig.local when both are set.
fn sync_git() {
    let (n, e) = (settings::get("GIT_NAME"), settings::get("GIT_EMAIL"));
    if !n.is_empty() && !e.is_empty() {
        git::identity(&n, &e);
    }
}

fn edit_git_name() {
    let Some(v) = ask::value("👤 Your name, for git commits", &settings::get("GIT_NAME"), "Jane Doe") else { return };
    if v.is_empty() {
        warn("a git name can't be empty, so it is not changed");
        return;
    }
    if settings::set("GIT_NAME", &v) {
        sync_git();
    }
}

fn edit_git_email() {
    let Some(v) = ask::value("✉️  Your email, for git commits", &settings::get("GIT_EMAIL"), "jane@example.com")
    else {
        return;
    };
    if !ask::looks_like_email(&v) {
        warn("that is not an email address, so it is not changed");
        return;
    }
    if settings::set("GIT_EMAIL", &v) {
        sync_git();
    }
}

/// A menu of labels, each saving one value. `now` picks the start label.
fn pick_value(header: &str, name: &str, now: &str, opts: &[(&str, &str, &str)]) {
    let labels: Vec<String> = opts.iter().map(|o| s(o.0)).collect();
    let start = opts.iter().find(|o| o.1 == now).map_or(opts[opts.len() - 1].0, |o| o.0);
    let Some(v) = ask::menu(header, start, &labels) else { return };
    if let Some(o) = opts.iter().find(|o| o.0 == v)
        && settings::set(name, o.1)
    {
        ok(o.2);
    }
}

fn edit_profile() {
    pick_value(
        "🧩 Profile",
        "PROFILE",
        &settings::profile(),
        &[
            ("🔒 owner: your private rules load", "owner", "profile: owner"),
            ("🌍 shared: no private rules", "shared", "profile: shared"),
        ],
    );
}

/// The presets that are on: a toggle list (Space marks one). Saved as `all`, `none` or the names.
fn edit_presets() {
    let all: Vec<String> = crate::presets::ALL.iter().map(ToString::to_string).collect();
    let profile = settings::profile();
    let home = super::os::home();
    let on = crate::presets::enabled_quiet(&home, &profile);
    let picked: Vec<String> = if ask::fancy() {
        let marked: Vec<usize> = all.iter().enumerate().filter(|(_, n)| on.contains(n)).map(|(i, _)| i).collect();
        if let Some(v) = ask::many(
            "🧰 Presets (working habits) that are on: Space marks one · Enter saves · Esc keeps",
            &all,
            &marked,
        ) {
            v
        } else {
            info("presets: not changed");
            return;
        }
    } else {
        info(&format!("presets: {} (or all, or none)", all.join(" ")));
        let Some(v) =
            ask::value("🧰 Presets that are on, split by spaces", &crate::presets::summary(&home, &profile), "all")
        else {
            return;
        };
        let w = words(&v);
        if w == "all" {
            all.clone()
        } else if w == "none" || w.is_empty() {
            Vec::new()
        } else {
            w.split(' ').map(String::from).collect()
        }
    };
    let mut out: Vec<String> = Vec::new();
    for n in picked {
        if all.contains(&n) {
            out.push(n);
        } else {
            warn(&format!("no preset named {n}, so it is left out"));
        }
    }
    let v = if out.len() == all.len() {
        s("all")
    } else if out.is_empty() {
        s("none")
    } else {
        out.join(" ")
    };
    if settings::set("PRESETS", &v) {
        ok(&format!("presets: {v}"));
    }
}

fn edit_plan() {
    let now = or(settings::get("PLAN"), "unknown");
    pick_value(
        "📊 Claude plan (sets the weekly pace budget)",
        "PLAN",
        &now,
        &[
            ("pro", "pro", "plan: pro"),
            ("max5: Max, 5x", "max5", "plan: max5"),
            ("max20: Max, 20x", "max20", "plan: max20"),
            ("team", "team", "plan: team"),
            ("api: pay per use", "api", "plan: api"),
            ("unknown: no pace budget", "unknown", "plan: unknown"),
        ],
    );
}

/// An IANA zone name such as Europe/Berlin: letters, digits and `_ + - /`, no `..`.
pub fn valid_zone(v: &str) -> bool {
    !v.contains("..") && !v.starts_with('/') && v.chars().all(|c| c.is_ascii_alphanumeric() || "_+-/".contains(c))
}

fn edit_zone() {
    let Some(v) =
        ask::value("🕒 Time zone (an IANA name; empty: the system zone)", &settings::get("TIMEZONE"), "Europe/Berlin")
    else {
        return;
    };
    if !valid_zone(v.trim()) {
        warn("that is not a time zone name, so it is not changed");
        return;
    }
    if settings::set("TIMEZONE", v.trim()) {
        ok(&format!("time zone: {}", or(v.trim().to_string(), "the system zone")));
    }
}

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// Work days: names or ranges split by spaces or commas, e.g. `mon-fri`, `mon-sat`, `sun-thu`.
pub fn valid_work_days(v: &str) -> bool {
    let w = words(&v.to_lowercase());
    !w.is_empty()
        && w.split(' ').all(|part| {
            let mut it = part.split('-');
            let (a, b, rest) = (it.next(), it.next(), it.next());
            rest.is_none() && a.is_some_and(|x| DAYS.contains(&x)) && b.is_none_or(|x| DAYS.contains(&x))
        })
}

fn edit_work_days() {
    let Some(v) =
        ask::value("📅 Work days, e.g. mon-fri or mon-sat (empty: mon-fri)", &settings::get("WORK_DAYS"), "mon-fri")
    else {
        return;
    };
    let v = v.trim().to_lowercase();
    if !v.is_empty() && !valid_work_days(&v) {
        warn("those are not work days (use mon, tue, ... or a range like mon-fri), so it is not changed");
        return;
    }
    if settings::set("WORK_DAYS", &v) {
        ok(&format!("work days: {}", or(v, "mon-fri")));
    }
}

fn edit_theme() {
    pick_value(
        "🎨 Theme for the status line, dots output and Opencode",
        "THEME",
        &or(settings::get("THEME"), "copper"),
        &[("🟠 copper: warm colors", "copper", "theme: copper"), ("⚪ plain: no colors", "plain", "theme: plain")],
    );
}

/// on or off. Like agent-config: 1, on, yes or true is on.
fn on_off(name: &str) -> &'static str {
    if matches!(settings::get(name).to_lowercase().as_str(), "1" | "on" | "yes" | "true") { "on" } else { "off" }
}

fn edit_mcp() {
    pick_value(
        "🔌 Cursor MCP servers (opt-in)",
        "CURSOR_MCP",
        on_off("CURSOR_MCP"),
        &[
            ("🔌 on: Cloudflare docs search in Cursor", "on", "Cursor MCP: on"),
            ("⛔ off: no MCP servers from dots", "off", "Cursor MCP: off"),
        ],
    );
}

fn edit_opencode_mcp() {
    pick_value(
        "🔌 Opencode MCP servers (opt-in)",
        "OPENCODE_MCP",
        on_off("OPENCODE_MCP"),
        &[
            ("🔌 on: Cloudflare docs search in Opencode", "on", "Opencode MCP: on"),
            ("⛔ off: no MCP servers from dots", "off", "Opencode MCP: off"),
        ],
    );
}

fn save_java(h: &Path) {
    if settings::set("JAVA_HOME", &h.to_string_lossy()) {
        ok(&format!("Java set: {}. New terminal windows use it.", java::label(h)));
    }
}

/// The likely answer comes first: the setting now, else the Java in use now, else the
/// newest long-term-support Java. A path can be typed too.
fn edit_java(repo: &Path) {
    let mut now = settings::get("JAVA_HOME");
    let mut gone = String::new();
    let mut used: Option<PathBuf> = None;
    if !now.is_empty() && java::home_of(Path::new(&now)).is_none() {
        info(&format!("JAVA_HOME points to a folder that is gone: {now}"));
        gone = std::mem::take(&mut now);
    }
    if now.is_empty() {
        info("JAVA_HOME tells Java tools (Maven, Gradle, editors) which Java to use.");
        if let Ok(j) = std::env::var("JAVA_HOME")
            && !j.is_empty()
        {
            used = java::home_of(Path::new(&j));
        }
    }
    let mut labels: Vec<String> = Vec::new();
    let mut homes: Vec<PathBuf> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut add = |label: String, h: PathBuf, labels: &mut Vec<String>, homes: &mut Vec<PathBuf>| {
        let r = java::real(&h);
        if seen.contains(&r) {
            return;
        }
        seen.push(r);
        labels.push(label);
        homes.push(h);
    };
    if !now.is_empty() {
        let h = PathBuf::from(&now);
        add(format!("⏭️  Keep it: Java {}", java::label(&h)), h, &mut labels, &mut homes);
    }
    if let Some(u) = used {
        add(format!("☕ Java {} (in use now)", java::label(&u)), u, &mut labels, &mut homes);
    }
    for h in java::ranked() {
        add(format!("☕ Java {}", java::label(&h)), h, &mut labels, &mut homes);
    }
    let typed = s("✏️  Type a path");
    let none = if now.is_empty() && gone.is_empty() {
        s("⏭️  No, skip it")
    } else {
        s("🗑️  Remove the Java setting")
    };
    if homes.is_empty() {
        info("No Java found in the usual places.");
        labels = vec![none.clone(), typed.clone()];
    } else {
        labels.push(typed.clone());
        labels.push(none.clone());
    }
    let local = repo.join("home/.config/zsh/local.zsh");
    if fs::read_to_string(&local)
        .is_ok_and(|t| t.lines().any(|l| l.split('#').next().unwrap_or("").contains("JAVA_HOME=")))
    {
        info("Your own lines in local.zsh also set JAVA_HOME. The setting from here runs after them, so it wins.");
    }
    let pick = ask::one("☕ Which Java should this machine use? (sets JAVA_HOME)", &labels).unwrap_or_default();
    let chosen = labels.iter().position(|l| *l == pick).and_then(|i| homes.get(i).cloned());
    let had = !now.is_empty() || !gone.is_empty();
    if let Some(j) = chosen {
        if j == Path::new(&now) {
            info(&format!("Java: kept ({})", java::label(&j)));
        } else {
            save_java(&j);
        }
    } else if pick == typed {
        let p = java::clean_path(
            &ask::text(
                "☕ Path to a Java home (the folder with bin/java). Empty: no change.",
                "/Library/Java/JavaVirtualMachines/<name>.jdk/Contents/Home",
                true,
            )
            .unwrap_or_default(),
        );
        if p.is_empty() {
            info("Java: not changed");
        } else if let Some(h) = java::home_of(Path::new(&p)) {
            save_java(&h);
        } else {
            warn(&format!("no bin/java in {p}, so the Java setting is not changed"));
        }
    } else if pick == none && had {
        if settings::set("JAVA_HOME", "") {
            ok("Java setting removed");
        }
    } else if had {
        info("Java: not changed");
    } else {
        info("Java: none set");
    }
}

/// The install steps that dots always skips on this machine.
fn edit_steps() {
    let all = super::steps::names();
    let now = settings::get("SKIP_STEPS");
    let picked: Vec<String> = if ask::fancy() {
        let marked: Vec<usize> = all
            .iter()
            .enumerate()
            .filter(|(_, n)| now.split_whitespace().any(|w| w == n.as_str()))
            .map(|(i, _)| i)
            .collect();
        if let Some(v) = ask::many("⏩ Steps to skip: Space marks one · Enter saves · Esc keeps", &all, &marked) {
            v
        } else {
            info("steps to skip: not changed");
            return;
        }
    } else {
        info(&format!("steps: {}", all.join(" ")));
        let Some(v) = ask::value("⏩ Steps to skip, split by spaces", &now, "") else { return };
        v.split_whitespace().map(String::from).collect()
    };
    let mut out: Vec<String> = Vec::new();
    for w in picked {
        if all.contains(&w) {
            out.push(w);
        } else {
            warn(&format!("no step named {w}, so it is left out"));
        }
    }
    let v = out.join(" ");
    if settings::set("SKIP_STEPS", &v) {
        ok(&format!("steps to skip: {}", if v.is_empty() { "none" } else { &v }));
    }
}

/// Words, split by spaces (commas count as spaces).
pub fn words(v: &str) -> String {
    v.replace(',', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn edit_words(name: &str, header: &str, example: &str, what: &str) {
    let Some(v) = ask::value(header, &settings::get(name), example) else { return };
    let v = words(&v);
    if settings::set(name, &v) {
        ok(&format!("{}: {}", what, if v.is_empty() { "none" } else { &v }));
    }
}

/// "1 line" or "N lines" (new lines counted, as wc -l does).
fn lines_of(p: &Path) -> String {
    let n = fs::read_to_string(p).map_or(0, |t| t.matches('\n').count());
    if n == 1 { s("1 line") } else { format!("{n} lines") }
}

/// The private rules, free text for Claude, Cursor and Opencode. Empty removes them.
fn edit_rules() {
    let md = settings::owner_md();
    let now = fs::read_to_string(&md).unwrap_or_default();
    match ask::long(
        "🔒 Private rules for Claude, Cursor and Opencode (Markdown). They stay on this machine.",
        now.trim_end_matches('\n'),
    ) {
        None => info("private rules: not changed"),
        Some(t) if t.trim().is_empty() => {
            if fs::remove_file(&md).is_ok() || !md.exists() {
                info("private rules: none on this machine");
            }
        }
        Some(t) => {
            let conf = settings::conf_dir();
            let saved = fs::create_dir_all(&conf).is_ok() && {
                settings::set_mode(&conf, 0o700);
                let text = format!("{}\n", t.trim_end_matches('\n'));
                write_private(&md, &text)
            };
            if saved {
                ok(&format!("private rules saved ({})", lines_of(&md)));
            }
        }
    }
}

/// Write a file that only this user can read (mode 600 from the start).
fn write_private(p: &Path, text: &str) -> bool {
    use std::io::Write;
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let good = o.open(p).and_then(|mut f| f.write_all(text.as_bytes())).is_ok();
    settings::set_mode(p, 0o600);
    good
}

/// One menu line: emoji, name padded to 18, the value (no commas, at most 48 characters).
pub fn row(emoji: &str, name: &str, value: &str) -> String {
    let v = value.replace(", ", " ").replace(',', " ");
    let v = if v.chars().count() > 48 { format!("{}…", v.chars().take(47).collect::<String>()) } else { v };
    format!("{emoji}{name:<18} {v}")
}

fn or(v: String, text: &str) -> String {
    if v.is_empty() { s(text) } else { v }
}

fn java_value() -> String {
    let j = settings::get("JAVA_HOME");
    if j.is_empty() {
        s("not set")
    } else if java::home_of(Path::new(&j)).is_none() {
        format!("gone: {j}")
    } else {
        format!("Java {}", java::label(Path::new(&j)))
    }
}

fn rules_value() -> String {
    let md = settings::owner_md();
    if fs::metadata(&md).is_ok_and(|m| m.len() > 0) { lines_of(&md) } else { s("none") }
}

/// The `dots settings` menu: shows each setting, asks for changes and saves each one at once.
/// Needs a terminal. Ends when the user picks Done.
pub fn run(repo: &Path) {
    step("Settings (they stay on this machine, never in git)");
    info("File: ~/.config/dotfiles/settings. Each change saves at once.");
    let done = s("✅ Done");
    let first = std::env::var("DOTFILES_FIRST_RUN").is_ok_and(|v| !v.is_empty());
    let mut start = if first { done.clone() } else { String::new() };
    let mut last: Option<usize> = None;
    loop {
        let rows = vec![
            row("👤 ", "Git name", &or(settings::get("GIT_NAME"), "not set")),
            row("📧 ", "Git email", &or(settings::get("GIT_EMAIL"), "not set")),
            row("🧩 ", "Profile", &settings::profile()),
            row("🧰 ", "Presets", &crate::presets::summary(&super::os::home(), &settings::profile())),
            row("📊 ", "Plan", &or(settings::get("PLAN"), "unknown")),
            row("🕒 ", "Time zone", &or(settings::get("TIMEZONE"), "system")),
            row("📅 ", "Work days", &or(settings::get("WORK_DAYS"), "mon-fri")),
            row("🎨 ", "Theme", &or(settings::get("THEME"), "copper")),
            row("🖌️ ", "Looks", &super::looks::summary()),
            row("🔌 ", "Cursor MCP", on_off("CURSOR_MCP")),
            row("🔌 ", "Opencode MCP", on_off("OPENCODE_MCP")),
            row("☕ ", "Java", &java_value()),
            row("⏩ ", "Steps to skip", &or(settings::get("SKIP_STEPS"), "none")),
            row("📁 ", "Private projects", &or(settings::get("PRIVATE_PROJECTS"), "none")),
            row("📂 ", "Opencode projects", &or(settings::get("OPENCODE_PROJECTS"), "none")),
            row("🧪 ", "Test host", &or(settings::get("TEST_HOST"), "this machine")),
            row("🚫 ", "Private words", &or(settings::get("PRIVATE_WORDS"), "none")),
            row("🔒 ", "Private rules", &rules_value()),
        ];
        // The cursor goes back to the row just changed (its value may be new).
        if let Some(l) = last {
            start = rows[l].clone();
        }
        let mut all = rows.clone();
        all.push(done.clone());
        let Some(pick) = ask::menu("⚙️  Pick a setting to change, or Done (↑ ↓, Enter)", &start, &all) else {
            break;
        };
        let Some(i) = rows.iter().position(|r| *r == pick) else { break };
        match i {
            0 => edit_git_name(),
            1 => edit_git_email(),
            2 => edit_profile(),
            3 => edit_presets(),
            4 => edit_plan(),
            5 => edit_zone(),
            6 => edit_work_days(),
            7 => edit_theme(),
            8 => super::looks::edit(),
            9 => edit_mcp(),
            10 => edit_opencode_mcp(),
            11 => edit_java(repo),
            12 => edit_steps(),
            13 => edit_words(
                "PRIVATE_PROJECTS",
                "📁 Private project names, split by spaces. The git hooks keep them out of commits.",
                "acme-app side-project",
                "private projects",
            ),
            14 => edit_words(
                "OPENCODE_PROJECTS",
                "📂 Opencode project paths, split by spaces (folders whose .opencode/ mirrors dots; ~ works)",
                "~/code/my-app ~/code/another-one",
                "Opencode projects",
            ),
            15 => edit_words(
                "TEST_HOST",
                "🧪 SSH host for the Linux check of scripts/os-check.sh (empty: this machine)",
                "my-linux-box",
                "test host",
            ),
            16 => edit_words(
                "PRIVATE_WORDS",
                "🚫 More private words, split by spaces. Your git name and email count already.",
                "client-name nickname",
                "private words",
            ),
            _ => edit_rules(),
        }
        last = Some(i);
    }
    ok("settings: done");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zones_and_days() {
        assert!(valid_zone("Europe/Berlin") && valid_zone("") && valid_zone("Etc/GMT+1"));
        assert!(!valid_zone("../etc") && !valid_zone("a b") && !valid_zone("/x"));
        assert!(valid_work_days("mon-fri") && valid_work_days("Mon, tue wed") && valid_work_days("sun-thu"));
        assert!(!valid_work_days("") && !valid_work_days("monday") && !valid_work_days("mon-fri-sat"));
    }
    #[test]
    fn word_lists() {
        assert_eq!(words(" a, b,,c  d "), "a b c d");
        assert_eq!(words(""), "");
    }
}
