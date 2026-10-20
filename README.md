<div align="center">

<img src="assets/logo.svg" width="96" height="96" alt="dotfiles logo">

# dotfiles

**One command sets up macOS, Linux or Windows for coding with Claude Code, Cursor and Opencode.**

</div>

## Set up macOS

1. Open the **Terminal** app. Do not use a Claude Code window for this.
2. Paste this line, then press Enter:

   ```sh
   sh -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
   ```

3. Type your Mac password when it asks. Homebrew needs it.
4. Answer the questions. Use ↑ ↓ to move and Enter to pick.
   - 👤 your name and ✉️ your email, for git commits
   - ⚙️ the settings menu: Java, private rules, private words and more. Pick ✅ Done to
     keep the defaults.
   - ✨ an optional extra: the Claude desktop app
5. Wait. The first run takes about 20 to 40 minutes.
6. 🔑 Pick what to sign in to: GitHub, Claude Code, Opencode, Cursor. A browser opens for each one.
7. Open a new Terminal window.

## Set up Linux

This works on Debian, Ubuntu, Fedora and Arch, on x86_64 and arm64. You need a normal user
that can run `sudo`. Alpine works too: the tools come from mise and `apk`.

1. Open a terminal.
2. Paste this line, then press Enter:

   ```sh
   bash -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
   ```

   No `curl` yet? Install it with your package manager first (for example `sudo apt-get install -y curl`).
3. Type your password when it asks. The package manager and Homebrew need it.
4. Answer the questions, as in steps 4 to 6 of the macOS setup.
5. Open a new terminal, or run `exec zsh`.

## Set up Windows

This works in Git Bash (MSYS/MINGW). You need `git` and `curl` on PATH. Install them with WinGet if
they are missing.

1. Open a Git Bash terminal.
2. Paste this line, then press Enter:

   ```sh
   bash -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
   ```

3. Answer the two questions (git name and email).
4. Wait about 10 minutes. The run installs mise, the tools, Claude Code and Opencode, and links your config.
   If it says "Windows can't make links yet", turn on Settings > System > For developers >
   Developer Mode, then paste the same line again.
5. Close the window and open a new Git Bash. Run `claude`.

What is different on Windows:

- No Homebrew. The command-line tools in `tools.list` come from mise (installed with WinGet), with
  WinGet and Scoop as fallbacks. Claude Code comes from WinGet (`Anthropic.ClaudeCode`). The shell-only
  tools (starship, atuin, zoxide and others) are for macOS and Linux.
- Your own `~/.gitconfig` stays. Only the git name and email are added to it.
- The macOS-only steps and the zsh setup do not run.
- `scripts/agent-config` generates the Opencode/Claude/Cursor config. It runs the Rust build in
  `rust/agent-config` (built on first use when `cargo` is installed). Python is not needed
  anywhere. With no `cargo` the steps warn and skip, and `dots` installs Rust on its next run.

## Check that it works

- **Claude Code:** run `claude`, then type `/ps:`. You see the `ps:` skills.
- **Opencode:** run `opencode`, then type `/ps:`. You see the same skills.
- **Cursor:** open Cursor, open the chat, then type `/ps:`. You see the same skills.
- **Git:** run `git config user.name`. You see your name.

## Every day

- Run `dots` to get the newest setup. You can run it at any time.
- Edit the files in `~/dotfiles/home/`. Your machine uses the change at once.
- Run `dots settings` to change a setting: git name, Java, private rules and more. The
  settings stay on this machine, never in git. `settings.example` lists them all.
- Run `dots doctor` to check the setup. It prints PASS or FAIL per check and names the fix.
- Working habits are opt-in **presets** (workflow, pace, models, clock, plain-writing,
  no-ai-attribution, ship-to-main). A new install turns on all but `ship-to-main`. Pick them
  with `dots settings`.
- Do not edit `~/.claude/CLAUDE.md`: it comes from this repo, and `dots` updates it.

## More

- [Setting this up for a friend](docs/FRIENDS-SETUP.md): a runbook that Claude follows step by step.
- [Full guide](docs/guide.md): what is inside, shortcuts, how to change things, uninstall, fixes.
- [AGENTS.md](AGENTS.md): how AI agents work on this repo, and the `owner` and `shared` profiles.
- [Cursor details](docs/cursor-port.md) and [Opencode details](docs/opencode-port.md).
- On a fork: change `DOTFILES_REPO` at the top of [`bootstrap.sh`](bootstrap.sh) and the URL in step 2.

## License

[MIT](LICENSE)
