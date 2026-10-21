//! `dots`: the dotfiles setup (pull, link, provision). `./dotfiles` is a small bash shim that
//! builds and runs it.
//!
//! A run prints one short line per step. `dots -v` (or DOTFILES_VERBOSE=1) prints
//! everything. `dots settings` (or `dots private`) opens the settings menu, then
//! refreshes the links, the Cursor rule and the Opencode config. `dots doctor` checks the
//! result and changes nothing.

pub mod ask;
pub mod doctor;
pub mod git;
pub mod install;
pub mod java;
pub mod link;
pub mod looks;
pub mod menu;
pub mod os;
pub mod settings;
pub mod skills;
pub mod steps;
pub mod uninstall;

use crate::out::{self, info, ok, step, step_end, uptodate, verbose, warn};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repo: DOTFILES_REPO (the shim sets it), else four folders up from this binary
/// (rust/agent-config/target/release/dots).
pub fn repo() -> PathBuf {
    if let Ok(v) = env::var("DOTFILES_REPO") && !v.is_empty() { return PathBuf::from(v); }
    // `rust-bin` starts the exe as `~/.claude/hooks/../../rust/...`: resolve it, or the root is `~/.claude/hooks/../..`
    env::current_exe().ok()
        .map(|e| if os::is_windows() { e } else { std::fs::canonicalize(&e).unwrap_or(e) })
        .and_then(|e| e.ancestors().nth(5).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Run one agent-config command in this process. A panic counts as a failure.
fn agent_config(args: &[&str]) -> bool { crate::run_args(args) == 0 }

/// The `dots` command line: `dots [-v] [settings | doctor | uninstall]`. Returns the exit code.
/// It pulls the repo (and restarts on new commits), links files, builds the Claude config and runs the steps.
pub fn main() -> i32 {
    let started = std::time::Instant::now();
    let repo = repo();
    let r = repo.to_string_lossy().into_owned();
    os::set_env("DOTFILES_REPO", &r);
    os::set_env("REPO", &r);
    os::set_env("DOTFILES", &r);
    if os::is_windows() { os::set_env("MSYS", "winsymlinks:nativestrict"); }
    os::brew_env();
    os::tool_paths(); // mise, cargo and WinGet tools on PATH for every step

    // `dots uninstall` takes the links and generated files back: no pull, no steps.
    if env::args().nth(1).as_deref() == Some("uninstall") {
        let rest: Vec<String> = env::args().skip(2).collect();
        return uninstall::run(&repo, &rest);
    }

    // `dots doctor` only reads: no pull, no links, no steps.
    if env::args().nth(1).as_deref() == Some("doctor") {
        if env::args().len() > 2 { eprintln!("usage: dots doctor"); return 2; }
        return doctor::run();
    }

    let mut ask_settings = false;
    let mut own_steps = false;
    for a in env::args().skip(1) {
        match a.as_str() {
            "-v" | "--verbose" => os::set_env("DOTFILES_VERBOSE", "1"),
            "settings" | "private" => {
                ask_settings = true;
                // Only the Cursor step runs then, to pick up the new settings. Opencode needs
                // no step: its config is written below on every run.
                if env::var_os("DOTFILES_STEPS").is_none() { os::set_env("DOTFILES_STEPS", "60-cursor"); own_steps = true; }
            }
            _ => { eprintln!("usage: dots [-v] [settings | doctor | uninstall]"); return 2; }
        }
    }

    // Every warning also goes to this file (bash steps are child processes), so the last
    // line can count them. The run after a pull keeps the same file.
    let pulled = env::var("DOTFILES_PULLED").map(|v| !v.is_empty()).unwrap_or(false);
    let mut warn_file = env::var("DOTFILES_WARN_FILE").unwrap_or_default();
    if !pulled || warn_file.is_empty() {
        warn_file = tempfile::Builder::new().prefix("dots-warn.").tempfile()
            .ok().and_then(|f| f.keep().ok()).map(|(_, p)| p.to_string_lossy().into_owned()).unwrap_or_default();
    }
    os::set_env("DOTFILES_WARN_FILE", &warn_file);

    if pulled {
        ok("pulled the latest version"); // the run before this one printed the step
    } else {
        step("Pulling latest");
        let before = git::head(&repo);
        if git::pull(&repo) {
            // New commits can change the setup. Run the new one for the rest.
            if git::head(&repo) != before { return restart(&repo); }
            uptodate("up to date");
        } else {
            warn("couldn't pull (no network, local commits, or a changed file), so this run uses what is here");
            let changed = git::changed(&repo);
            if !changed.is_empty() { info(&format!("changed here: {}", changed)); }
            if format!(" {} ", changed).contains(" claude/CLAUDE.md ") {
                info("keep your own rules in your private rules (dots settings), not in claude/CLAUDE.md");
            }
        }
    }

    // Windows needs real symlinks (Developer Mode). Stop first, once, with the fix.
    if os::is_windows() && !os::windows_links_ready() {
        warn("Windows can't make links yet. Turn on Developer Mode:");
        info("Settings > System > For developers > Developer Mode (On). Then run the same command again.");
        return 1;
    }

    // The first run is the one without a settings file (AGENTS.md "Settings").
    step("Git identity");
    let first_run = !settings::settings_path().is_file();
    if first_run {
        let machine = if os::is_macos() { "Mac" } else if os::is_windows() { "Windows machine" } else { "Linux machine" };
        if os::is_windows() {
            ask::banner("🦴 dotfiles", &[&format!("This sets up your {} for Claude Code and Opencode.", machine),
                "It asks for your name and email, then takes about 5 minutes."]);
        } else {
            ask::banner("🦴 dotfiles", &[&format!("This sets up your {} for Claude Code, Cursor and Opencode.", machine),
                "A few questions first, then about 20 to 40 minutes of installs.", "Use ↑ ↓ to move and Enter to pick."]);
        }
    }
    let mut name = settings::get("GIT_NAME");
    let mut email = settings::get("GIT_EMAIL");
    // --includes: also read ~/.gitconfig.local, which ~/.gitconfig includes.
    if name.is_empty() { name = git::out(&["config", "--global", "--includes", "user.name"]).unwrap_or_default(); }
    if email.is_empty() { email = git::out(&["config", "--global", "--includes", "user.email"]).unwrap_or_default(); }
    if name.is_empty() { name = ask::text("👤 Your name, for git commits", "Jane Doe", false).unwrap_or_default(); }
    if email.is_empty() { email = ask::email("✉️  Your email, for git commits", "jane@example.com").unwrap_or_default(); }
    if !name.is_empty() && !email.is_empty() {
        if settings::get("GIT_NAME") != name && !settings::set("GIT_NAME", &name) { warn("couldn't save GIT_NAME"); }
        if settings::get("GIT_EMAIL") != email && !settings::set("GIT_EMAIL", &email) { warn("couldn't save GIT_EMAIL"); }
        git::identity(&name, &email);
    } else {
        warn("no git name or email given. Run dots settings in a terminal to set them.");
    }
    os::set_env("DOTFILES_FIRST_RUN", if first_run { "1" } else { "" });

    // The settings menu: on the first run at a terminal, or any time with: dots settings
    if first_run && !ask_settings && os::is_windows() {
        info("Change settings later with: bash ~/dotfiles/dotfiles settings");
    } else if (first_run || ask_settings) && os::at_terminal() {
        menu::run(&repo);
    } else if ask_settings {
        warn("dots settings asks questions, so run it in a terminal");
    }

    // The look changes are optional: ask once while LOOKS is not set (also after an update).
    looks::ask_if_unset();

    let profile = settings::profile();
    os::set_env("DOTFILES_PROFILE", &profile);

    // An optional extra, offered once: on the first run, at a terminal. "No" comes first.
    if os::is_macos() && first_run && os::at_terminal() && !Path::new("/Applications/Claude.app").is_dir() {
        step("Extras");
        let no = "⏭️  No, skip it".to_string();
        let app = "🖥️  Yes, install the Claude desktop app".to_string();
        if ask::one("✨ Install the Claude desktop app too?", &[no, app.clone()]).as_deref() == Some(app.as_str()) {
            info("Installing the Claude app");
            if quiet(Command::new("brew").args(["install", "--cask", "--quiet", "claude"])) {
                ok("Claude desktop app installed");
            } else {
                warn("couldn't install the Claude app. Try later: brew install --cask claude");
            }
        }
    }

    link_files(&repo);
    claude_config(&repo, &profile);

    step("Skills");
    let (good, lines) = skills::sync(&repo);
    if !good { warn("the skill sync reported an issue — skills may be unlinked"); }
    if lines.is_empty() { uptodate("skills linked"); } else { out::replay(&lines); }

    if verbose() { step("Opencode config -> ~/.config/opencode"); } else { step_end(); }
    // Opencode reads ~/.claude/skills natively. What remains: the global rules file, the
    // agents mirrored from claude/agents, the copper theme, and permissions and opt-in MCP
    // servers merged into opencode.jsonc. Generated or merged, never linked.
    let oc = ["opencode", "--profile", profile.as_str()];
    if os::is_windows() {
        // One line instead of eleven; the whole output only when something fails.
        let (good, lines) = out::capture(|| agent_config(&oc));
        let n = lines.iter().filter(|(l, _)| l.starts_with('✓') || l.contains("✓\x1b[0m")).count();
        if good && n > 0 && !verbose() { ok(&format!("Opencode config written ({} files)", n)); } else { out::replay(&lines); }
        if !good { warn("agent-config opencode reported an issue, see above"); }
    } else if !agent_config(&oc) {
        warn("agent-config opencode reported an issue, see above");
    }

    steps::run_all(&repo, own_steps);

    step_end();
    os::brew_env(); // step 10 can install it during this run
    // Step 40 can bring the Rust toolchain during this run, so build the Rust tools now. A
    // no-op when they are fresh. A HOME with no toolchain (a temp HOME, a bare box) skips it.
    if env::var("DOTFILES_STEPS").unwrap_or_default().is_empty() {
        if os::have("cargo") && quiet(Command::new("cargo").arg("--version").env("RUSTUP_AUTO_INSTALL", "0")) {
            let _ = Command::new(os::bash()).arg(repo.join("scripts").join("agent-config")).arg("--bins").status();
        }
        // again: the cargo source works now if step 40 just brought cargo
        crate::tools_cli::run(vec!["--quiet".into()]);
    }
    if os::is_windows() {
        if first_run {
            ask::banner("✅ All set", &["Close this window and open a new Git Bash.",
                "Claude Code: run claude (it asks you to sign in the first time).",
                "Opencode: run opencode auth login once, then opencode.",
                "Skills: type /ps: in Claude Code to see them.",
                "Update later: bash ~/dotfiles/dotfiles"]);
        }
    } else if os::is_macos() && !os::have("brew") {
        warn("Homebrew is missing, so the apps did not install. Fix the error above, then run dots again.");
    } else if first_run {
        let new_shell = if os::is_macos() { "Open a new Terminal window." } else { "Open a new terminal, or run: exec zsh" };
        ask::banner("✅ All set", &[new_shell,
            "Claude Code: run claude, then type /ps: to see the skills.",
            "Opencode: run opencode, then type /ps: to see the same skills.",
            "Cursor: open the chat, then type /ps: to see the same skills."]);
    }
    let nwarn = if warn_file.is_empty() { out::warn_count() } else {
        let n = fs::read_to_string(&warn_file).map(|t| t.lines().count()).unwrap_or(0);
        let _ = fs::remove_file(&warn_file);
        n
    };
    os::unset_env("DOTFILES_WARN_FILE");
    let took = out::duration(started.elapsed().as_secs());
    if nwarn > 0 {
        let w = if nwarn == 1 { "warning" } else { "warnings" };
        warn(&format!("dotfiles: done in {} with {} {} (profile: {}). Read the ! lines above.", took, nwarn, w, profile));
    } else {
        ok(&format!("dotfiles: done in {} (profile: {})", took, profile));
    }
    0
}

/// Run a command with its output hidden. True when it worked.
fn quiet(cmd: &mut Command) -> bool {
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// New commits came in: run the new `./dotfiles` for the rest, with the same warn file.
/// Windows has no exec, so the new run is a child. It can rebuild dots.exe because the
/// shim runs a copy of it there (Windows locks a running .exe).
fn restart(repo: &Path) -> i32 {
    let mut cmd = Command::new(os::bash());
    cmd.arg(repo.join("dotfiles")).args(env::args().skip(1)).env("DOTFILES_PULLED", "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = cmd.exec();
        eprintln!("dots: couldn't restart: {}", e);
        1
    }
    #[cfg(not(unix))]
    { cmd.status().ok().and_then(|s| s.code()).unwrap_or(1) }
}

/// The files under home/ that link 1:1 into $HOME. Anything not listed is never touched:
/// secrets and per-machine state live outside this list.
pub fn links() -> Vec<&'static str> {
    let mut v = Vec::new();
    if os::is_macos() { v.push(".config/aerospace"); } // a Mac window manager
    if os::is_windows() {
        // No zsh or terminal tools here: link only what Windows uses.
        v.push(".config/gh/config.yml");
    } else {
        v.extend([".config/atuin", ".config/bat", ".config/bottom", ".config/fastfetch", ".config/gh/config.yml",
            ".config/ghostty", ".config/lazygit", ".config/tealdeer", ".config/yazi",
            ".config/zsh", ".gitconfig", ".zshrc"]);
        if looks::on("starship") { v.push(".config/starship.toml"); }
    }
    v
}

/// The VS Code and Cursor settings: (repo file, link in $HOME). The repo keeps them under
/// their Mac path in home/; each OS looks for them in its own place.
pub fn editor_links(repo: &Path, home: &Path) -> Vec<(PathBuf, PathBuf)> {
    let editor_cfg = if os::is_macos() { "Library/Application Support" } else if os::is_windows() { "AppData/Roaming" } else { ".config" };
    let mut v = Vec::new();
    for app in ["Code", "Cursor"] {
        for f in ["settings.json", "keybindings.json"] {
            v.push((repo.join("home/Library/Application Support").join(app).join("User").join(f),
                home.join(editor_cfg).join(app).join("User").join(f)));
        }
    }
    v
}

fn link_files(repo: &Path) {
    step("Linking files");
    let home = os::home();
    link::take_count();
    if os::is_windows() { git::windows_gitconfig(); }
    for item in links() { link::link_item(&repo.join("home").join(item), &home.join(item), true); }
    // The optional look changes (LOOKS): an item that is off loses our link, and the
    // file it replaced comes back.
    let starship = ".config/starship.toml";
    if !os::is_windows() && !looks::on("starship") { link::unlink_item(&repo.join("home").join(starship), &home.join(starship)); }
    for (src, dst) in editor_links(repo, &home) {
        if looks::on("editor") { link::link_item(&src, &dst, true); } else { link::unlink_item(&src, &dst); }
    }
    // The Claude and Opencode config below needs no cargo here: it runs in this
    // process. Step 05 installs the Linux prerequisites.
    let n = link::take_count();
    if n > 0 { ok(&format!("linked {} files", n)); }
}

fn claude_config(repo: &Path, profile: &str) {
    step("Claude config");
    if verbose() { info(&format!("profile: {} (AGENTS.md \"Profiles\")", profile)); }
    let src = repo.join("claude");
    let dst = os::home().join(".claude");
    if !src.is_dir() {
        warn(&format!("{} missing — skipping Claude config link", src.display()));
        return;
    }
    if fs::create_dir_all(&dst).is_err() { warn(&format!("couldn't create {}", dst.display())); }
    // Per item, never the whole folder: Claude Code writes session state in ~/.claude.
    for d in ["agents", "hooks", "output-styles", "CLAUDE.md"] {
        link::link_item(&src.join(d), &dst.join(d), true);
    }
    // The old status line script is gone (`rust-bin statusline` replaced it): drop our link to it.
    let old_sl = dst.join("statusline.sh");
    if os::is_symlink(&old_sl) && os::readlink(&old_sl).is_some_and(|t| t.starts_with(&src)) && fs::remove_file(&old_sl).is_ok() {
        ok("removed the old statusline.sh link");
    }
    // The enabled presets load as user rules; the ones that went off lose their link.
    let on = crate::presets::enabled(&os::home(), profile);
    crate::presets::link_all(repo, &os::home(), &on);
    if verbose() { info(&format!("presets: {}", crate::presets::summary(&os::home(), profile))); }
    let n = link::take_count();
    if n > 0 { ok(&format!("linked {} Claude files", n)); }
    step_end(); // the agent-config lines below print their own count
    // settings.json is built, not linked: the repo base plus the private layer. Claude writes
    // this file itself; what it changed since the last build moves into the layer first.
    if !agent_config(&["claude-settings", "build"]) {
        warn("couldn't build ~/.claude/settings.json, so it stays as it was");
    }
    // The owner's private rules (never in the repo) load as a Claude user rule.
    let rule = dst.join("rules").join("owner.md");
    let owner = settings::owner_md();
    if profile == "owner" && owner.is_file() {
        link::link_item(&owner, &rule, false);
    } else {
        if profile == "owner" { warn("owner profile, but no private rules on this machine. Add them with: dots settings"); }
        if os::is_symlink(&rule) && os::readlink(&rule).as_deref() == Some(owner.as_path()) && fs::remove_file(&rule).is_ok() {
            ok("removed the owner rules link");
        }
    }
}
