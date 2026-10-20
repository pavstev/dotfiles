//! Prompts. A person at a terminal gets inquire menus in the copper palette. Anything
//! else (a pipe, a test, a terminal Rust cannot drive) gets plain lines and stdin.
//! Every prompt draws on stderr, so stdout stays clean.

use super::os;
use crate::out::info;
use inquire::ui::{Color, RenderConfig, StyleSheet, Styled};
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::Once;

const COPPER: Color = Color::AnsiValue(173);
const STONE: Color = Color::AnsiValue(245);
const DIM: Color = Color::AnsiValue(240);

/// inquire menus: a person can answer, and DOTS_PLAIN_PROMPTS is not set.
pub fn fancy() -> bool { os::at_terminal() && std::env::var_os("DOTS_PLAIN_PROMPTS").is_none() }

fn theme() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let rc = RenderConfig::default_colored()
            .with_prompt_prefix(Styled::new("›").with_fg(COPPER))
            .with_answered_prompt_prefix(Styled::new("✓").with_fg(COPPER))
            .with_highlighted_option_prefix(Styled::new("›").with_fg(COPPER))
            .with_selected_checkbox(Styled::new("[x]").with_fg(COPPER))
            .with_help_message(StyleSheet::new().with_fg(STONE))
            .with_answer(StyleSheet::new().with_fg(COPPER))
            .with_selected_option(Some(StyleSheet::new().with_fg(COPPER)));
        let mut rc = rc;
        rc.placeholder = StyleSheet::new().with_fg(DIM);
        inquire::set_global_render_config(rc);
    });
}

/// One line from stdin, without the new line. None at the end of input. The prompt
/// shows only when stdin is a terminal, as with bash `read -p`.
fn read_line(prompt: &str) -> Option<String> {
    if io::stdin().is_terminal() {
        eprint!("{}", prompt);
        let _ = io::stderr().flush();
    }
    let mut s = String::new();
    match io::stdin().lock().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s.trim_end_matches(['\n', '\r']).to_string()),
    }
}

/// One line of text. Asked again while it is empty, unless `empty_ok`. None: no answer.
pub fn text(label: &str, hint: &str, empty_ok: bool) -> Option<String> {
    loop {
        let v = if fancy() {
            theme();
            match inquire::Text::new(label).with_placeholder(hint).prompt() {
                Ok(v) => v,
                Err(inquire::InquireError::OperationCanceled) | Err(inquire::InquireError::OperationInterrupted) => return None,
                Err(_) => read_line(&format!("{}: ", label))?,
            }
        } else {
            read_line(&format!("{}: ", label))?
        };
        let v = v.trim().to_string();
        if !v.is_empty() || empty_ok { return Some(v); }
    }
}

/// The glob ?*@?*.?* : something, @, something, a dot, something.
pub fn looks_like_email(v: &str) -> bool {
    v.char_indices().any(|(i, ch)| {
        if ch != '@' || i == 0 { return false; }
        let rest = &v[i + 1..];
        rest.char_indices().any(|(j, c2)| c2 == '.' && j > 0 && j + 1 < rest.len())
    })
}

/// Like `text`, but it must look like an email.
pub fn email(label: &str, hint: &str) -> Option<String> {
    loop {
        let v = text(label, hint, false)?;
        if looks_like_email(&v) { return Some(v); }
        info("that is not an email address, try again");
    }
}

/// One line with the value now filled in. Plain: Enter keeps it, a lone - empties it.
/// None (Esc): keep it.
pub fn value(label: &str, now: &str, hint: &str) -> Option<String> {
    let v = if fancy() {
        theme();
        inquire::Text::new(label).with_initial_value(now).with_placeholder(hint).prompt().ok()?
    } else {
        let v = read_line(&format!("{} [{}]: ", label, now))?;
        if v.is_empty() { now.to_string() } else if v == "-" { String::new() } else { v }
    };
    Some(v.trim().to_string())
}

/// Pick one label; the cursor starts on `start` (or the first). None: Esc or no input.
pub fn menu(header: &str, start: &str, labels: &[String]) -> Option<String> {
    let n = labels.iter().position(|l| l == start).unwrap_or(0);
    if fancy() {
        theme();
        match inquire::Select::new(header, labels.to_vec()).with_starting_cursor(n).with_page_size(labels.len().clamp(1, 16)).prompt() {
            Ok(v) => return Some(v),
            Err(inquire::InquireError::OperationCanceled) | Err(inquire::InquireError::OperationInterrupted) => return None,
            Err(_) => {}
        }
    }
    eprintln!("{}", header);
    for (i, l) in labels.iter().enumerate() { eprintln!("  {}) {}", i + 1, l); }
    let a = read_line(&format!("  Number [{}]: ", n + 1))?;
    let i = a.trim().parse::<usize>().ok().filter(|i| *i >= 1 && *i <= labels.len()).unwrap_or(n + 1);
    labels.get(i - 1).cloned()
}

/// `menu` with the cursor on the first label.
pub fn one(header: &str, labels: &[String]) -> Option<String> { menu(header, "", labels) }

/// Pick any of the labels; `now` are marked first. None: Esc.
pub fn many(header: &str, labels: &[String], now: &[usize]) -> Option<Vec<String>> {
    theme();
    inquire::MultiSelect::new(header, labels.to_vec()).with_default(now).with_page_size(labels.len().clamp(1, 16)).prompt().ok()
}

/// Several lines of text. At a terminal, e opens the editor ($VISUAL, $EDITOR, else nano,
/// vi or notepad) with the text now, and Enter saves. Plain: lines until one that holds only a dot; an
/// empty answer keeps the text. None: Esc.
pub fn long(header: &str, now: &str) -> Option<String> {
    if fancy() {
        theme();
        // "code --wait" is a program and its arguments.
        let ed = editor_command();
        let mut parts = ed.split_whitespace();
        let prog = parts.next().unwrap_or("vi").to_string();
        let args: Vec<&std::ffi::OsStr> = parts.map(std::ffi::OsStr::new).collect();
        return match inquire::Editor::new(header)
            .with_predefined_text(now)
            .with_editor_command(std::ffi::OsStr::new(&prog))
            .with_args(&args)
            .with_file_extension(".md")
            .with_help_message("e opens your editor (save and close it when done) · Enter saves · Esc keeps it as it is")
            .prompt() {
            Ok(t) => Some(t),
            Err(inquire::InquireError::OperationCanceled) | Err(inquire::InquireError::OperationInterrupted) => None,
            Err(e) => { crate::out::warn(&format!("the editor did not run ({}): {}", prog, e)); None }
        };
    }
    eprintln!("{}\nPaste or type. End with a line that holds only a dot.", header);
    let mut out = String::new();
    while let Some(l) = read_line("") {
        if l == "." { break; }
        out.push_str(&l);
        out.push('\n');
    }
    Some(if out.is_empty() { now.to_string() } else { out })
}

fn editor_command() -> String {
    for k in ["VISUAL", "EDITOR"] {
        if let Ok(v) = std::env::var(k) && !v.trim().is_empty() { return v; }
    }
    for c in ["nano", "vi", "notepad"] { if os::have(c) { return c.to_string(); } }
    "vi".into()
}

/// A rounded copper box at a terminal, plain lines otherwise.
pub fn banner(title: &str, lines: &[&str]) {
    if !fancy() {
        println!("\n{}", title);
        for l in lines { println!("  {}", l); }
        return;
    }
    let c = "\x1b[38;5;173m";
    let bold = "\x1b[1;38;5;173m";
    let z = "\x1b[0m";
    let width = std::iter::once(title).chain(lines.iter().copied()).map(display_width).max().unwrap_or(0) + 4;
    let row = |text: &str, style: &str| {
        let pad = width - 4 - display_width(text);
        println!("{}│{}  {}{}{}{}  {}│{}", c, z, style, text, z, " ".repeat(pad), c, z);
    };
    println!();
    println!("{}╭{}╮{}", c, "─".repeat(width - 2), z);
    row(title, bold);
    for l in lines { row(l, ""); }
    println!("{}╰{}╯{}", c, "─".repeat(width - 2), z);
    println!();
}

/// Terminal columns of a string: wide emoji count two, joiners and variation marks none.
pub fn display_width(s: &str) -> usize {
    s.chars().map(|ch| {
        let u = ch as u32;
        if u == 0xFE0F || u == 0x200D || (0x0300..0x0370).contains(&u) { 0 }
        else if (0x1F300..0x1FAFF).contains(&u) || (0x2600..0x27C0).contains(&u) && u != 0x2192 || (0x2B00..0x2C00).contains(&u) { 2 }
        else { 1 }
    }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email_glob() {
        assert!(looks_like_email("jane@example.com"));
        assert!(looks_like_email("a@b.c"));
        assert!(!looks_like_email("@b.c"));
        assert!(!looks_like_email("a@.c"));
        assert!(!looks_like_email("a@b."));
        assert!(!looks_like_email("jane"));
        assert!(looks_like_email("a@b.c@d"));
    }
    #[test]
    fn widths() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("✅ All set"), 10);
    }
}
