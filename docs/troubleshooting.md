# Troubleshooting

Fixes for the problems people hit most.

← [Docs](README.md)

**Something feels off.** Run `dots doctor`. Each FAIL line names the fix. A `bins-fresh` FAIL
means a Rust program is older than its source or not built: run `dots`.

**`brew bundle` says "Permission denied".** Homebrew is owned by another user. Run
`~/dotfiles/scripts/fix-brew-permissions.sh`, then `dots`.

**`~/.claude` didn't get linked.** You ran `./dotfiles` inside a Claude Code session,
which can race Claude rewriting `~/.claude` live. Re-run `dots` from a plain shell.

**Linux: a step failed at the package manager or Homebrew.** Run `dots` again after fixing the line above it.
The Homebrew installer needs `sudo` and a user that is not root.

**Icons look like boxes.** The Starship prompt needs a Nerd Font. Turn on `font` in
`dots settings` > Looks (macOS), or set your terminal font to "FiraCode Nerd Font Mono". Or
turn `starship` off for the plain prompt. Ghostty uses the font from its config; Apple
Terminal gets it from step `75-terminal-appearance` (needs Automation permission once).

**Starship warns `Unknown key` or `unknown variant`.** Your Starship is older than the
config. Update it (`mise use -g starship@latest`), or turn `starship` off in Looks.

**`dots` seems stuck.** A step that runs longer than 20 seconds prints
`still working on <step> (1m20s)…` every minute. The last line is always
`dotfiles: done in <time>`. No new line for several minutes: press Ctrl-C and run `dots -v`
to see every command.

**`compinit: insecure directories`.** Run `chmod -R go-w "$(brew --prefix)/share"` once.

**`couldn't pull` after the history was rewritten.** When the upstream history is rewritten (a
force push), `dots` and the bootstrap one-liner move a clone to it by themselves, but only when the
clone has no commits of its own. Edits to tracked files (an editor writes through its linked
settings file) are saved as `~/.config/dotfiles/before-rewrite-<time>.patch` first. The old state
stays as a branch `before-rewrite-<time>`. A clone with its own commits keeps them and warns. Save your work, then run
`git -C ~/dotfiles fetch && git -C ~/dotfiles reset --hard origin/main`.
