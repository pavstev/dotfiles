//! Private words: the leak check of `dots doctor` (no private word in a tracked file of this
//! public repo). The guard of the ps plugin has its own copy for prompts and commands.

use std::path::Path;
use std::process::{Command, Stdio};

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The `remote.origin.url`, `user.email` and `user.name` of the git config in effect (HOME set to
/// `home`, in `dir` when given). Missing ones are "".
struct GitIdentity {
    url: String,
    email: String,
    name: String,
}

fn git_identity(home: &Path, dir: Option<&Path>) -> GitIdentity {
    let mut found = GitIdentity {
        url: String::new(),
        email: String::new(),
        name: String::new(),
    };
    let mut c = Command::new("git");
    c.args([
        "config",
        "-z",
        "--get-regexp",
        r"^(remote\.origin\.url|user\.(email|name))$",
    ])
    .env("HOME", home)
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let Ok(done) = c.output() else {
        return found;
    };
    // entries are `key\nvalue` split by NUL; the first url and the last identity win, as in `git config`
    for entry in String::from_utf8_lossy(&done.stdout).split('\0') {
        let Some((key, value)) = entry.split_once('\n') else {
            continue;
        };
        let value = value.trim().to_string();
        match key {
            "remote.origin.url" if found.url.is_empty() => found.url = value,
            "user.email" => found.email = value,
            "user.name" => found.name = value,
            _ => {}
        }
    }
    found
}

/// A word of 4 letters or fewer matches whole words only; a longer one matches anywhere.
/// Both sides are lower case.
pub fn word_hit(text: &str, word: &str) -> bool {
    !word_spans(text, word).is_empty()
}

/// The byte spans of `word` in `text` that count as a hit by the rule of `word_hit`. Both sides
/// are lower case.
pub fn word_spans(text: &str, word: &str) -> Vec<(usize, usize)> {
    if word.is_empty() {
        return Vec::new();
    }
    let long = word.chars().count() > 4;
    text.match_indices(word)
        .filter(|(i, _)| {
            long || {
                let before = text[..*i].chars().next_back();
                let after = text[*i + word.len()..].chars().next();
                !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
            }
        })
        .map(|(i, _)| (i, i + word.len()))
        .collect()
}

/// The words the repo must not leak, lower case and without repeats: `PRIVATE_PROJECTS` and
/// `PRIVATE_WORDS` from the settings, and the git email, the full git name and that name as one
/// word (from the settings and from `git config`, run in `dir` when given). A word of the identity
/// that is part of the origin URL (the GitHub handle in the clone URL) is left out, as
/// `scripts/git-hooks/private-words.sh` does.
pub fn private_words_at(home: &Path, dir: Option<&Path>) -> Vec<String> {
    fn push(out: &mut Vec<String>, w: &str) {
        let w = w.trim().to_lowercase();
        if !w.is_empty() && !out.contains(&w) {
            out.push(w);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for name in ["PRIVATE_PROJECTS", "PRIVATE_WORDS"] {
        for w in crate::setting(home, name).split(|c: char| c.is_whitespace() || c == ',') {
            push(&mut out, w);
        }
    }
    let git = git_identity(home, dir);
    let url = git.url.to_lowercase();
    let idents = [
        crate::setting(home, "GIT_EMAIL"),
        crate::setting(home, "GIT_NAME"),
        git.email,
        git.name,
    ];
    for raw in idents {
        let name = raw.trim().to_lowercase();
        let joined: String = name.split_whitespace().collect();
        for w in [name.clone(), joined] {
            if w.chars().count() >= 3 && (url.is_empty() || !url.contains(&w)) {
                push(&mut out, &w);
            }
        }
    }
    out
}

/// The host names of the `PUBLIC_HOSTS` setting (a space or comma list), lower case. The word checks
/// skip them, so a link to the owner's own site may hold a private word. An entry with a character
/// outside `[A-Za-z0-9.-]` is ignored.
pub fn public_hosts(home: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in crate::setting(home, "PUBLIC_HOSTS").split(|c: char| c.is_whitespace() || c == ',') {
        let h = h.to_lowercase();
        if !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && !out.contains(&h)
        {
            out.push(h);
        }
    }
    out
}

/// `text` (lower case) with every host in `hosts` removed. Whole host names only, never paths.
pub fn strip_hosts(text: &str, hosts: &[String]) -> String {
    let mut out = text.to_string();
    for h in hosts.iter().filter(|h| !h.is_empty()) {
        out = out.replace(h.as_str(), "");
    }
    out
}
