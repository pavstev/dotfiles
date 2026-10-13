<div align="center">

<img src="assets/logo.svg" width="96" height="96" alt="dotfiles logo">

# dotfiles

**One command sets up a Mac for coding with Claude Code and Cursor.**

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
   - 🔒 your private rules, 🚫 private words and ☕ which Java to use (all optional,
     they stay on this Mac)
   - ✨ optional extras: the Claude desktop app, Cloudflare docs search in Cursor
5. Wait. The first run takes about 20 to 40 minutes.
6. 🔑 Pick what to sign in to: GitHub, Claude Code, Cursor. A browser opens for each one.
7. Open a new Terminal window.

## Check that it works

- **Claude Code:** run `claude`, then type `/ps:`. You see the `ps:` skills.
- **Cursor:** open Cursor, open the chat, then type `/ps:`. You see the same skills.
- **Git:** run `git config user.name`. You see your name.

## Every day

- Run `dots` to get the newest setup. You can run it at any time.
- Edit the files in `~/dotfiles/home/`. Your Mac uses the change at once.
- Run `dots private` to change your private rules, private words and Java. They stay
  on this Mac, never in git.
- Do not edit `~/.claude/CLAUDE.md`: it comes from this repo, and `dots` updates it.

## More

- [Full guide](docs/guide.md): what is inside, shortcuts, how to change things, uninstall, fixes.
- [AGENTS.md](AGENTS.md): how AI agents work on this repo, and the `owner` and `shared` profiles.
- [Cursor details](docs/cursor-port.md).
- On a fork: change `DOTFILES_REPO` at the top of [`bootstrap.sh`](bootstrap.sh) and the URL in step 2.

## License

[MIT](LICENSE)
