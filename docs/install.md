# Install

How to set up macOS, Linux or Windows, check that it works, and keep several machines in step.

← [Docs](README.md)

> [!IMPORTANT]
> Run the line in a plain terminal (Git Bash on Windows), not inside a Claude Code session.
> Claude rewrites `~/.claude` during a live session, which can race the link step. If that
> happens, run `dots` again from a plain shell.

## Set up macOS

1. Open the **Terminal** app and paste this line:

   ```sh
   sh -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
   ```

2. Type your Mac password when it asks. Homebrew needs it.
3. Answer the questions. Use ↑ ↓ to move and Enter to pick.
   - 👤 your name and ✉️ your email, for git commits
   - ⚙️ the settings menu. Pick ✅ Done to keep the defaults.
   - 🎨 the looks: the Starship prompt, the VS Code and Cursor settings, the Nerd Font. All are
     optional. Mark none to keep your own look.
   - ✨ an optional extra: the Claude desktop app
4. Wait. The first run takes about 20 to 40 minutes.
5. 🔑 Pick what to sign in to: GitHub, Claude Code, Opencode, Cursor. A browser opens for each.
6. Open a new Terminal window.

## Set up Linux

Debian, Ubuntu, Fedora, Arch and Alpine, on x86_64 and arm64. You need a normal user that can run
`sudo`.

1. Open a terminal and paste this line:

   ```sh
   bash -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
   ```

   No `curl` yet? Install it first (for example `sudo apt-get install -y curl`).
2. Type your password when it asks, then answer the questions as in step 3 of the macOS setup.
3. Open a new terminal, or run `exec zsh`.

## Set up Windows

Git Bash only. You need `git` and `curl` (install them with WinGet if they are missing).

1. Open Git Bash and paste this line:

   ```sh
   bash -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
   ```

2. Answer the two questions (git name and email).
3. Wait about 5 to 10 minutes. If it says "Windows can't make links yet", turn on
   Settings > System > For developers > Developer Mode, then paste the same line again.
4. Close the window, open a new Git Bash and run `claude`.

What is different on Linux and Windows: [Platforms](platforms.md).

## Check that it works

- **Claude Code, Opencode or Cursor:** open it and type `/ps:`. You see the `ps:` skills.
- **Git:** `git config user.name` prints your name.
- **Everything:** `dots doctor` prints PASS or FAIL for each check.

## Update and several machines

One machine edits and pushes. Every other machine runs `dots` when it wants to catch up: pull,
relink, install what is missing. Nothing runs in the background. For automatic pulls, point your
own cron or login hook at `dots`.

Machine-only settings stay out of git: `~/.config/zsh/local.zsh` and `~/.zshrc.local` for the
shell, `~/.gitconfig.local` for git, `Brewfile.local` for Mac apps (see
[`Brewfile.local.example`](../Brewfile.local.example)). [atuin](https://atuin.sh) syncs shell
history (`atuin register` once).

## Uninstall

`dots uninstall`. Details: [Customize](customize.md#uninstall).
