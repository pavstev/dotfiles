//! skill_check: lint every skill folder against the rules we rely on. Replaces
//! claude/skills/check.py (same checks, same messages, same exit codes).
//!
//! Rules come from the Claude Code skills docs, Anthropic's skill-authoring best practices and
//! the agentskills.io spec. Exit 1 on any FAIL.
//!
//! ```text
//! skill_check              all skills in claude/skills of this repo
//! skill_check ps:askme     one or more skill names or folders
//! skill_check <dir>        every skill folder inside <dir>
//! skill_check -q           failures only, nothing when all pass
//! ```

use agent_config::pycompat::py_repr_str;
use agent_config::quiet_println as println;
use clap::Parser;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::LazyLock;

/// Frontmatter keys Claude Code documents. Anything else is ignored by Claude Code, so it is
/// almost always a typo.
const KNOWN_KEYS: [&str; 20] = [
    "name",
    "description",
    "when_to_use",
    "argument-hint",
    "arguments",
    "disable-model-invocation",
    "user-invocable",
    "allowed-tools",
    "disallowed-tools",
    "model",
    "effort",
    "context",
    "agent",
    "background",
    "hooks",
    "paths",
    "shell",
    "metadata",
    "license",
    "compatibility",
];
const NAME_PATTERN: &str = r"^(ps:)?[a-z0-9]+(-[a-z0-9]+)*$"; // spec regex + our ps: prefix
const MAX_NAME: usize = 64;
const MAX_DESC: usize = 1024; // spec / Claude Code hard cap
const MAX_DESC_COMBINED: usize = 1536; // description + when_to_use, truncated past this in the listing
const MAX_BODY_LINES: usize = 500; // Anthropic: keep SKILL.md body under 500 lines
const MAX_BODY_TOKENS: usize = 5000; // agentskills.io: < 5000 tokens recommended (4 chars/token)
const WARN_BODY_TOKENS: usize = 2500; // our own bar: a skill this size is doing too much
const JUNK: [&str; 5] = [".ruff_cache", "__pycache__", ".pytest_cache", ".DS_Store", ".mypy_cache"];
const INTERPRETERS: [&str; 8] = ["bash", "sh", "zsh", "python3", "python", "node", "uv", "bun"];
const NOT_LOCAL: [&str; 4] = ["http://", "https://", "#", "mailto:"];

static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(NAME_PATTERN).unwrap());
static HARDCODED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(~|\$HOME|/(?:Users|home)/[^/\s]+)/\.claude/skills/").unwrap());
static XML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[a-zA-Z/][^>]*>").unwrap());
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]\(([^)\s]+)\)").unwrap());
static SKILL_DIR_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\$\{CLAUDE_SKILL_DIR\}/([^\s`'")*]+)"#).unwrap());
static FORCE_LOAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\s)@(~/|/|\./|skills/)").unwrap());

struct Report {
    skill: String,
    fails: Vec<String>,
    warns: Vec<String>,
}

type Fields = Vec<(String, String)>;

fn field<'a>(fields: &'a Fields, key: &str) -> &'a str {
    fields.iter().find(|(k, _)| k == key).map_or("", |(_, v)| v.as_str())
}

/// Return (fields, body, errors). Strict on purpose: it mimics the parsers that silently drop a
/// skill, instead of forgiving what they would not.
fn parse_frontmatter(text: &str) -> (Fields, String, Vec<String>) {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines[0].trim() != "---" {
        return (vec![], text.to_string(), vec!["frontmatter must start on line 1 with ---".into()]);
    }
    let Some(end) = (1..lines.len()).find(|&i| lines[i].trim() == "---") else {
        return (vec![], text.to_string(), vec!["frontmatter never closes (missing second ---)".into()]);
    };
    let mut errors = Vec::new();
    let mut fields: Fields = Vec::new();
    for raw in &lines[1..end] {
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }
        if raw.starts_with([' ', '\t']) {
            continue; // nested value (hooks:, metadata:) — not validated here
        }
        let Some((key, value)) = raw.split_once(':') else {
            errors.push(format!("frontmatter line is not key: value → {}", py_repr_str(raw)));
            continue;
        };
        let (key, value) = (key.trim().to_string(), value.trim().to_string());
        match fields.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value.clone(),
            None => fields.push((key.clone(), value.clone())),
        }
        let Some(q) = value.chars().next() else { continue };
        if q == '"' || q == '\'' {
            if value.chars().count() < 2 || !value.ends_with(q) {
                errors.push(format!("{key}: quoted value does not close"));
            }
            continue;
        }
        if value.contains(": ") || value.ends_with(':') {
            errors.push(format!("{key}: unquoted value contains ': ' — quote the whole value"));
        }
        if "[]{}&*!|>%@`".contains(q) {
            errors.push(format!("{key}: unquoted value starts with {} — quote it", py_repr_str(&q.to_string())));
        }
        if value.contains(" #") {
            errors.push(format!("{key}: unquoted ' #' starts a YAML comment — quote the value"));
        }
        if value.contains(" ... ") {
            errors.push(format!("{key}: ' ... ' breaks Claude Code's parser — rephrase or quote"));
        }
    }
    (fields, lines[end + 1..].join("\n"), errors)
}

fn unquote(v: &str) -> String {
    match (v.chars().next(), v.chars().last()) {
        (Some(a), Some(b)) if v.chars().count() >= 2 && (a == '"' || a == '\'') && a == b => {
            v[a.len_utf8()..v.len() - b.len_utf8()].to_string()
        }
        _ => v.to_string(),
    }
}

/// Junk that must never ship, top-down, each folder's hits sorted (like `os.walk` + `sorted`).
fn find_junk(root: &Path, dir: &Path, r: &mut Report) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(String, bool)> = rd
        .filter_map(Result::ok)
        .map(|e| {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            (e.file_name().to_string_lossy().into_owned(), is_dir)
        })
        .collect();
    entries.sort();
    for (name, _) in &entries {
        if JUNK.contains(&name.as_str()) {
            let rel = dir.join(name);
            let rel = rel.strip_prefix(root).unwrap_or(&rel).to_string_lossy().replace('\\', "/");
            r.fails.push(format!("junk in skill folder: {rel}"));
        }
    }
    for (name, is_dir) in &entries {
        if *is_dir {
            find_junk(root, &dir.join(name), r);
        }
    }
}

fn read_text(path: &Path) -> Result<String, std::io::Error> {
    // Python's text mode folds \r\n and a lone \r into \n.
    Ok(std::fs::read_to_string(path)?.replace("\r\n", "\n").replace('\r', "\n"))
}

fn check_skill(path: &Path) -> Report {
    let shown = path.to_string_lossy();
    let name =
        Path::new(shown.trim_end_matches('/')).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let name = name.as_str();
    let mut r = Report { skill: name.to_string(), fails: vec![], warns: vec![] };
    let skill_md = path.join("SKILL.md");
    if !skill_md.is_file() {
        r.fails.push("no SKILL.md".into());
        return r;
    }
    find_junk(path, path, &mut r);

    let text = match read_text(&skill_md) {
        Ok(t) => t,
        Err(e) => {
            r.fails.push(format!("cannot read SKILL.md: {e}"));
            return r;
        }
    };
    let (fields, body, errors) = parse_frontmatter(&text);
    r.fails.extend(errors);

    for (k, _) in &fields {
        if !KNOWN_KEYS.contains(&k.as_str()) {
            r.warns.push(format!("unknown frontmatter key '{k}' (ignored by Claude Code)"));
        }
    }

    // name
    let fname = unquote(field(&fields, "name"));
    if fname.is_empty() {
        r.fails.push("name missing".into());
    } else {
        let want = fname.replace(':', "_");
        if want != name {
            r.fails.push(format!("name '{fname}' needs folder '{want}', found '{name}'"));
        }
        if fname.chars().count() > MAX_NAME {
            r.fails.push(format!("name longer than {MAX_NAME} chars"));
        }
        if !NAME_RE.is_match(&fname) {
            r.fails.push(format!("name must match {NAME_PATTERN}"));
        }
    }

    // description
    let desc = unquote(field(&fields, "description"));
    if desc.is_empty() {
        r.fails.push("description missing or empty".into());
    } else {
        let len = desc.chars().count();
        if len > MAX_DESC {
            r.fails.push(format!("description is {len} chars (max {MAX_DESC})"));
        }
        if XML_TAG.is_match(&desc) {
            r.fails.push("description contains an XML tag".into());
        }
        let low = desc.to_lowercase();
        if low.starts_with("i ") || low.contains(" i can ") || low.starts_with("you can use") {
            r.fails.push("description must be third person".into());
        }
        if !low.contains("when") {
            r.warns.push("description never says WHEN to use the skill".into());
        }
        let combined = len + unquote(field(&fields, "when_to_use")).chars().count();
        if combined > MAX_DESC_COMBINED {
            r.fails.push(format!(
                "description + when_to_use is {combined} chars (listing truncates at {MAX_DESC_COMBINED})"
            ));
        }
    }

    // allowed-tools
    let tools = unquote(field(&fields, "allowed-tools"));
    if tools.split(|c: char| c == ',' || c.is_whitespace()).any(|t| t == "Bash") {
        r.warns.push("allowed-tools grants bare 'Bash' — scope it, e.g. Bash(go *)".into());
    }

    // body size
    let body_chars = body.chars().count();
    let body_lines = body.matches('\n').count() + usize::from(!body.is_empty() && !body.ends_with('\n'));
    let tokens = body_chars / 4;
    if body_lines > MAX_BODY_LINES {
        r.fails.push(format!("body is {body_lines} lines (max {MAX_BODY_LINES}) — move detail to a reference file"));
    }
    if tokens > MAX_BODY_TOKENS {
        r.fails.push(format!("body is ~{tokens} tokens (max {MAX_BODY_TOKENS})"));
    } else if tokens > WARN_BODY_TOKENS {
        r.warns.push(format!("body is ~{tokens} tokens — consider moving detail to a reference file"));
    }

    // paths and links
    for m in HARDCODED.find_iter(&body) {
        r.fails.push(format!("hardcoded skill path '{}…' — use ${{CLAUDE_SKILL_DIR}}", m.as_str()));
    }
    if FORCE_LOAD.is_match(&body) {
        r.warns.push("'@path' force-loads a file into context — use a plain link instead".into());
    }
    for caps in SKILL_DIR_REF.captures_iter(&body) {
        let (whole, rel) = (caps.get(0).unwrap(), caps.get(1).unwrap().as_str());
        if !path.join(rel).exists() {
            r.fails.push(format!("${{CLAUDE_SKILL_DIR}}/{rel} does not exist"));
        }
        let before: String =
            body[..whole.start()].chars().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
        let prev = before.split_whitespace().last().map_or("", |w| w.trim_matches('`'));
        if rel.starts_with("scripts/") && !INTERPRETERS.contains(&prev) {
            r.warns.push(format!("run scripts through an interpreter (bash/python3 …): {rel}"));
        }
    }
    for caps in MD_LINK.captures_iter(&body) {
        let target = caps.get(1).unwrap().as_str();
        if NOT_LOCAL.iter().any(|p| target.starts_with(p)) {
            continue;
        }
        let target = target.replace("${CLAUDE_SKILL_DIR}/", "");
        let target = target.split('#').next().unwrap_or("").to_string();
        let full = path.join(&target);
        if !full.exists() {
            r.fails.push(format!("link target missing: {target}"));
        } else if full.to_string_lossy().ends_with(".md") {
            let sub = read_text(&full).unwrap_or_default();
            for n in MD_LINK.captures_iter(&sub) {
                let t = n.get(1).unwrap().as_str();
                if !NOT_LOCAL.iter().any(|p| t.starts_with(p)) && t.ends_with(".md") {
                    r.warns.push(format!("{target} links on to {t} — keep references one level deep"));
                    break;
                }
            }
        }
    }
    r
}

/// The skill folders inside `root`: sorted, no `_*` or `.*` folders, each with a SKILL.md.
fn list_skills(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    let mut names: Vec<String> =
        rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
        .into_iter()
        .filter(|d| !d.starts_with(['_', '.']))
        .map(|d| root.join(d))
        .filter(|p| p.is_dir() && p.join("SKILL.md").is_file())
        .collect()
}

fn run(paths: &[PathBuf], quiet: bool) -> i32 {
    let mut bad = 0;
    for p in paths {
        let r = check_skill(p);
        let status = if !r.fails.is_empty() {
            "FAIL"
        } else if !r.warns.is_empty() {
            "WARN"
        } else {
            "OK"
        };
        if !r.fails.is_empty() {
            bad += 1;
        }
        if quiet && status != "FAIL" {
            continue; // -q prints failures only; warnings show without -q
        }
        println!("{:<4} {}", status, r.skill);
        for f in &r.fails {
            println!("       ✖ {f}");
        }
        for w in &r.warns {
            println!("       · {w}");
        }
    }
    if bad > 0 || !quiet {
        println!("{} skills checked, {} failing", paths.len(), bad);
    }
    i32::from(bad > 0)
}

const USAGE: &str = "usage: skill_check [-q] [skill name or folder | skills dir]...";

#[derive(Parser)]
#[command(name = "skill_check", disable_help_flag = true, disable_version_flag = true)]
struct Cli {
    /// Print the usage line.
    #[arg(short = 'h', long = "help")]
    help: bool,
    /// Failures only.
    #[arg(short = 'q')]
    quiet: bool,
    /// Skill names, skill folders or folders of skills.
    targets: Vec<String>,
}

/// What one command-line target means: a skill folder, a folder of skills, or a skill name under
/// the repo's claude/skills.
fn resolve(arg: &str, skills_root: &Path) -> Vec<PathBuf> {
    let p = Path::new(arg);
    if p.is_dir() {
        if p.join("SKILL.md").is_file() {
            return vec![p.to_path_buf()];
        }
        let inside = list_skills(p);
        if !inside.is_empty() {
            return inside;
        }
        return vec![p.to_path_buf()];
    }
    if p.is_absolute() {
        return vec![p.to_path_buf()];
    }
    vec![skills_root.join(arg.replace(':', "_"))]
}

fn main() -> ExitCode {
    let Cli { help, quiet, targets } = Cli::parse();
    if help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let root = agent_config::claude_dir().join("skills");
    let paths: Vec<PathBuf> = if targets.is_empty() {
        let found = list_skills(&root);
        if found.is_empty() {
            // an empty list would pass silently: that is a miss, not a clean tree
            eprintln!(
                "skill_check: no skills found in {}. Run it inside the dotfiles repo or set DOTFILES_REPO.",
                root.display()
            );
            return ExitCode::from(1);
        }
        found
    } else {
        targets.iter().flat_map(|t| resolve(t, &root)).collect()
    };
    ExitCode::from(run(&paths, quiet) as u8)
}
