//! The look changes, each one optional (LOOKS in the settings): the Starship prompt, the
//! VS Code and Cursor settings, and the Nerd Font with the Apple Terminal look. All are off
//! until picked. `dots` asks once at a terminal while LOOKS is not set; `dots settings`
//! changes it. The value is `none` or the names, split by spaces.

use super::{ask, os, settings};
use crate::out::{info, ok, step, warn};

/// Each part: its name in LOOKS and its menu label.
pub const ALL: [(&str, &str); 3] = [
    ("starship", "⭐ Starship prompt (colors and icons in the shell prompt)"),
    ("editor", "📝 VS Code and Cursor settings (theme, Nerd Font, keys)"),
    ("font", "🔤 Nerd Font and the Apple Terminal look (macOS)"),
];

/// The parts this OS can use: Windows has no zsh prompt, only macOS installs the font.
pub fn offered() -> Vec<&'static str> {
    ALL.iter()
        .map(|p| p.0)
        .filter(|n| match *n {
            "starship" => !os::is_windows(),
            "font" => os::is_macos(),
            _ => true,
        })
        .collect()
}

/// The names in a LOOKS value. `all` is every part; anything else unknown is dropped.
pub fn parse(v: &str) -> Vec<String> {
    let w = super::menu::words(&v.to_lowercase());
    if w == "all" {
        return ALL.iter().map(|p| p.0.to_string()).collect();
    }
    w.split(' ').filter(|n| ALL.iter().any(|p| p.0 == *n)).map(String::from).collect()
}

/// Whether a part is on. Not set yet counts as off.
pub fn on(name: &str) -> bool {
    parse(&settings::get("LOOKS")).iter().any(|n| n == name)
}

/// The value for the settings menu.
pub fn summary() -> String {
    let v = settings::get("LOOKS");
    if v.is_empty() {
        return "not set (off)".to_string();
    }
    let p = parse(&v);
    if p.is_empty() { "none".to_string() } else { p.join(" ") }
}

/// Ask which parts are on and save the answer. The parts this OS can't use keep their value.
pub fn edit() {
    let offered = offered();
    let now = parse(&settings::get("LOOKS"));
    let labels: Vec<String> =
        offered.iter().map(|n| ALL.iter().find(|p| p.0 == *n).map(|p| p.1).unwrap_or(n).to_string()).collect();
    let picked: Vec<&str> = if ask::fancy() {
        let marked: Vec<usize> =
            offered.iter().enumerate().filter(|(_, n)| now.iter().any(|x| x == *n)).map(|(i, _)| i).collect();
        match ask::many(
            "🎨 Look changes: Space marks one · Enter saves (none marked: keep your own look)",
            &labels,
            &marked,
        ) {
            Some(v) => offered.iter().zip(&labels).filter(|(_, l)| v.contains(l)).map(|(n, _)| *n).collect(),
            None => {
                info("looks: not changed");
                return;
            }
        }
    } else {
        info(&format!("looks: {} (or none)", offered.join(" ")));
        let Some(v) = ask::value("🎨 Look changes that are on, split by spaces", &now.join(" "), "none") else {
            return;
        };
        let p = parse(&v);
        offered.iter().copied().filter(|n| p.iter().any(|x| x == n)).collect()
    };
    let mut keep: Vec<String> = now.into_iter().filter(|n| !offered.contains(&n.as_str())).collect();
    keep.extend(picked.iter().map(|n| n.to_string()));
    let v = ALL.iter().map(|p| p.0).filter(|n| keep.iter().any(|k| k == n)).collect::<Vec<_>>().join(" ");
    let v = if v.is_empty() { "none".to_string() } else { v };
    if settings::set("LOOKS", &v) {
        ok(&format!("looks: {}", v));
    } else {
        warn("couldn't save LOOKS");
    }
}

/// Ask once, at a terminal, while LOOKS is not set (a new machine or the first run after an update).
pub fn ask_if_unset() {
    if !settings::get("LOOKS").is_empty() || !os::at_terminal() {
        return;
    }
    step("Looks");
    info("The look changes are optional. Pick the ones you want, or none to keep your own look.");
    info("Change it later with: dots settings");
    edit();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        assert_eq!(parse("starship editor"), vec!["starship", "editor"]);
        assert_eq!(parse("none"), Vec::<String>::new());
        assert_eq!(parse(""), Vec::<String>::new());
        assert_eq!(parse("ALL"), vec!["starship", "editor", "font"]);
        assert_eq!(parse("font, bogus"), vec!["font"]);
    }
}
