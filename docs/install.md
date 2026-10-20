# Install

How to set up macOS, Linux or Windows, check that it works, and keep several machines in step.

← [Docs](README.md)

> [!IMPORTANT]
> Run `./dotfiles` from a plain terminal (Git Bash on Windows), not inside a Claude Code session. Claude rewrites
> `~/.claude` during a live session, which can race the link step. If that happens, run
> `dots` again from a plain shell.

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
   - 🎨 the looks: the Starship prompt, the VS Code and Cursor settings, the Nerd Font. All
     are optional. Mark none to keep your own look.
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

More on Linux: [Platforms](platforms.md#linux).

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
  tools (starship, atuin, zoxide and others) are for macOS and Linux. The VS Code and Cursor
  settings link only when you pick `editor` in the looks.
- Your own `~/.gitconfig` stays. Only the git name and email are added to it.
- The macOS-only steps and the zsh setup do not run.
- `scripts/agent-config` generates the Opencode/Claude/Cursor config. It runs the Rust build in
  `rust/agent-config` (built on first use when `cargo` is installed). Python is not needed
  anywhere. With no `cargo` the steps warn and skip, and `dots` installs Rust on its next run.

More on Windows: [Platforms](platforms.md#windows).

## Check that it works

- **Claude Code:** run `claude`, then type `/ps:`. You see the `ps:` skills.
- **Opencode:** run `opencode`, then type `/ps:`. You see the same skills.
- **Cursor:** open Cursor, open the chat, then type `/ps:`. You see the same skills.
- **Git:** run `git config user.name`. You see your name.

## Multi-machine

- **Bootstrap**: the one-liner in the [README](../README.md) turns a blank macOS, Linux or Windows machine into this, end to end.
- **Synced history**: [atuin](https://atuin.sh) owns `Ctrl-R` (the `↑` arrow stays native).
  `atuin register` / `atuin login` once, and your history follows you.
- **Per-machine overrides**: gitignored `~/.config/zsh/local.zsh` & `~/.zshrc.local` for
  shell, `~/.gitconfig.local` for git identity, `Brewfile.local` (see
  [`Brewfile.local.example`](../Brewfile.local.example)) for packages.
- **Manual sync, no daemon**: one machine edits and pushes; every other machine just
  runs `dots` (`./dotfiles`) whenever it wants to catch up: pull, relink, reprovision.
  Nothing runs in the background. If you want automatic pulls, wire your own cron or
  login hook to call `dots`.
- **Agent-aware**: [`AGENTS.md`](../AGENTS.md) (read by Cursor and Opencode) and the repo's
  [`CLAUDE.md`](../CLAUDE.md) (which imports it) teach any AI agent how to operate this repo,
  so Claude, Cursor or Opencode on a fresh machine knows the workflow without being told.

## Updating

Fully manual, by design: no background process. Pull and reprovision on demand:

```sh
dots        # alias for ./dotfiles: git pull + relink + reprovision
```

The tools step and the brew step (macOS) run every time but no-op when nothing's missing; one-time install
steps (Touch ID, git hooks, etc.) detect their own prior state and skip too. Want
automatic pulls on a given machine? Point your own cron or login hook at `dots`.
Nothing in this repo does that for you.
