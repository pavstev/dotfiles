<div align="center">

<img src="assets/logo.svg" width="96" height="96" alt="dotfiles logo">

# dotfiles

**One command sets up a Mac, Debian or Ubuntu for coding with Claude Code and Cursor.**

</div>

## Set up a Mac

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
6. 🔑 Pick what to sign in to: GitHub, Claude Code, Cursor. A browser opens for each one.
7. Open a new Terminal window.

## Set up Linux

This works on Debian 13 and Ubuntu 24.04, on x86_64 and arm64. You need a normal user
that can run `sudo`.

1. Open a terminal.
2. Paste this line, then press Enter:

   ```sh
   bash -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
   ```

   No `curl` yet? Run `sudo apt-get install -y curl` first.
3. Type your password when it asks. `apt` and Homebrew need it.
4. Answer the questions, as in steps 4 to 6 of the Mac setup.
5. Open a new terminal, or run `exec zsh`.

What is different from a Mac:

- The tools come from Homebrew on Linux, in `/home/linuxbrew/.linuxbrew`. `apt` only adds the
  build tools and `zsh`.
- Three steps are macOS only and skip with one line: macOS defaults, Touch ID for sudo, Apple
  Terminal appearance.
- The desktop apps are not installed (Ghostty, VS Code, Cursor, Raycast, OrbStack, AeroSpace,
  the font). Install the ones you want. The config for VS Code and Cursor links to
  `~/.config/Code/User` and `~/.config/Cursor/User`, and the extensions install when the
  `code` or `cursor` command exists.
- Your login shell does not change. To use zsh everywhere, run `chsh -s "$(command -v zsh)"`.

## Check that it works

- **Claude Code:** run `claude`, then type `/ps:`. You see the `ps:` skills.
- **Cursor:** open Cursor, open the chat, then type `/ps:`. You see the same skills.
- **Git:** run `git config user.name`. You see your name.

## Every day

- Run `dots` to get the newest setup. You can run it at any time.
- Edit the files in `~/dotfiles/home/`. Your machine uses the change at once.
- Run `dots settings` to change a setting: git name, Java, private rules and more. The
  settings stay on this machine, never in git. `settings.example` lists them all.
- Do not edit `~/.claude/CLAUDE.md`: it comes from this repo, and `dots` updates it.

## More

- [Full guide](docs/guide.md): what is inside, shortcuts, how to change things, uninstall, fixes.
- [AGENTS.md](AGENTS.md): how AI agents work on this repo, and the `owner` and `shared` profiles.
- [Cursor details](docs/cursor-port.md).
- On a fork: change `DOTFILES_REPO` at the top of [`bootstrap.sh`](bootstrap.sh) and the URL in step 2.

## License

[MIT](LICENSE)
