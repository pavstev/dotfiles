# Troubleshooting

Fixes for the problems people hit most.

← [Docs](README.md)

**Something feels off.** Run `dots doctor`. Each FAIL line names the fix. A `bins-fresh` FAIL
means a Rust program is older than its source or not built: run `dots`.

**`dots` seems stuck.** A step that runs longer than 20 seconds prints
`still working on <step> (1m20s)…` every minute. The last line is always `dotfiles: done in <time>`.
No new line for several minutes: press Ctrl-C and run `dots -v` to see every command.

**`~/.claude` did not get linked.** You ran `./dotfiles` inside a Claude Code session, which can
race Claude rewriting `~/.claude`. Run `dots` again from a plain shell.

**`brew bundle` says "Permission denied".** Another user owns Homebrew. Run
`~/dotfiles/scripts/fix-brew-permissions.sh`, then `dots`.

**Linux: a step failed at the package manager.** Fix the line above it, then run `dots` again.
No `sudo`: install the packages of `scripts/linux-prereqs.txt` as root
([Platforms](platforms.md#linux)).

**Icons look like boxes.** The Starship prompt needs a Nerd Font. Turn on `font` in
`dots settings` > Looks (macOS), or set your terminal font to "FiraCode Nerd Font Mono". Or turn
`starship` off for the plain prompt. Ghostty uses the font from its config. Apple Terminal gets
it from step `75-terminal-appearance`, which needs the Automation permission once.

**Starship warns `Unknown key` or `unknown variant`.** Your Starship is older than the config.
Run `mise use -g starship@latest`, or turn `starship` off in Looks.

**`compinit: insecure directories`.** Run `chmod -R go-w "$(brew --prefix)/share"` once.

**`couldn't pull` after a force push upstream.** `dots` and the bootstrap line move a clone to
the new history by themselves, but only when the clone has no commits of its own. Edits to tracked
files are saved to `~/.config/dotfiles/before-rewrite-<time>.patch` first, and the old state stays
as the branch `before-rewrite-<time>`. A clone with its own commits keeps them and warns. Save
your work, then run `git -C ~/dotfiles fetch && git -C ~/dotfiles reset --hard origin/main`.

**Start over.** `dots uninstall` removes the links and generated files and puts your old files
back ([Customize](customize.md#uninstall)). Then run the bootstrap line again
([Install](install.md)).
