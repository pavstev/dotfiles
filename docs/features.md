# Features

Why this repo exists, what is inside, and the daily commands and shortcuts.

← [Docs](README.md)

## Why

Plain git is the single source of truth. Every managed file in `$HOME` is a symlink back into
this repo. There is no templating engine and no apply step for content changes.

- **Fast:** lazy zsh loading, cold start about 0.3 s. `eza`, `bat`, `rg`, `fd`, `zoxide`, `fzf`,
  `atuin` and `delta` replace the old tools.
- **Reproducible:** `./dotfiles` is idempotent. A step skips what is installed. A step that fails
  (offline, no permission) warns and steps aside; the run goes on.
- **Override-safe:** a config file that was there before moves to `<file>.bak-<timestamp>`
  before its link goes in.
- **Portable:** macOS, Linux and Windows (Git Bash). Machine-specific parts live in git-ignored
  local files ([Platforms](platforms.md)).
- **AI-ready:** Claude Code skills, agents, hooks and settings live here. Cursor and Opencode read
  the same source ([AI agents](ai-agents.md)).
- **One palette:** warm copper on stone, in Ghostty, Starship, fzf, delta and atuin.

## What's inside

| Area | Tools |
|------|-------|
| **Engine** | plain git + [`./dotfiles`](../dotfiles) · [rage](https://github.com/str4d/rage) (age encryption, for manual secrets) |
| **Shell** | zsh + Oh My Zsh · [Starship](https://starship.rs) (optional, see [Looks](customize.md#looks-starship-editor-settings-and-the-nerd-font)) · [atuin](https://atuin.sh) (Ctrl-R) · [carapace](https://carapace.sh) · autosuggestions · fast-syntax-highlighting · fzf-tab |
| **Navigation** | [fzf](https://github.com/junegunn/fzf) · [zoxide](https://github.com/ajeetdsouza/zoxide) · [yazi](https://github.com/sxyazi/yazi) (`y`) · [eza](https://github.com/eza-community/eza) · [bat](https://github.com/sharkdp/bat) |
| **Search** | [ripgrep](https://github.com/BurntSushi/ripgrep) · [fd](https://github.com/sharkdp/fd) |
| **System** | [dust](https://github.com/bootandy/dust) · [duf](https://github.com/muesli/duf) · [procs](https://github.com/dalance/procs) · [bottom](https://github.com/ClementTsang/bottom) (`btm`, also `top`) |
| **Git** | [delta](https://github.com/dandavison/delta) · [difftastic](https://difftastic.wilfred.me.uk) · [lazygit](https://github.com/jesseduffield/lazygit) · [gh](https://cli.github.com) + [gh-dash](https://github.com/dlvhdr/gh-dash) |
| **Containers** | [oxker](https://github.com/mrjackwills/oxker) · [OrbStack](https://orbstack.dev) |
| **Workflow** | [mise](https://mise.jdx.dev) · [just](https://github.com/casey/just) · [jaq](https://github.com/01mf02/jaq) (a `jq` link) · [gum](https://github.com/charmbracelet/gum) · tealdeer |
| **Python** | [uv](https://docs.astral.sh/uv) · [ruff](https://docs.astral.sh/ruff) · [ty](https://docs.astral.sh/ty) |
| **JS / TS** | [Biome](https://biomejs.dev) · Node + pnpm via mise |
| **Go** | [golangci-lint](https://golangci-lint.run) |
| **Rust** | [rustup](https://rustup.rs) · [cargo-nextest](https://nexte.st) |
| **Apps** | [Ghostty](https://ghostty.org) · VS Code · [Cursor](https://cursor.com) · [Raycast](https://raycast.com) · [AeroSpace](https://nikitabobko.github.io/AeroSpace) · FiraCode Nerd Font |

The modern aliases work in interactive shells only, never in scripts. For the real command in
one call, use `\grep` or `command grep`.

## Daily workflow

The repo (`~/dotfiles/home`) is `$HOME`: most files are symlinks, so an edit on either side
changes the same file.

```sh
$EDITOR ~/dotfiles/home/.config/zsh/init.zsh   # edit: it is live at once, no apply step
dotd                                            # preview source changes (git diff)
dots                                            # pull + relink + reprovision: for new files, packages, machines
dots settings                                   # the settings menu: git name, Java, private rules and more
dots -v                                         # the same run, with every line (the default is one line per step)
dots doctor                                     # check the setup: PASS or FAIL per check, changes nothing
dots uninstall                                  # take it all back (asks first; -y skips the question)
dots update                                     # macOS: update every package manager in one run
dots update --check                             # macOS: list the updates, change nothing
dot                                             # cd into the repo
```

`dots update` (macOS only) checks Homebrew, mise, rustup, cargo crates, uv tools, npm and pnpm
globals, gh extensions, Oh My Zsh, the App Store, Claude Code and macOS. The checks run in
parallel and print one line each. Then it updates what is outdated, one manager at a time, and goes
on after a failure. After each update it checks again and names what is still outdated (a version
pin can hold a package back). macOS and App Store updates are only reported: install macOS updates in System
Settings and App Store updates with `sudo mas upgrade` (it needs root). Pick managers
with `--only brew,mise` or `--skip mas,claude`; `dots -v update` also shows each command's output.
The log of each run is in `~/Library/Logs/dots` (the last 10 are kept). The exit code is 1 when a
check or an update failed, 2 for an unknown manager id. Not covered: system Ruby gems, `go install`
binaries, pipx, editor extensions and apps that update themselves.

`dots doctor` prints one line per check and exits 1 when one fails. A FAIL line names the fix,
usually `Run: dots`. The checks:

| Check | What it looks at |
|-------|------------------|
| `links` | every linked file points into the repo |
| `settings-built` | `~/.claude/settings.json` exists and every hook runs `rust-bin` |
| `bins-fresh` | each Rust program is built and not older than its source |
| `guard-selftest` | the guard denies a bare BSD `sed -i` (macOS rules) and a read of `local.zsh`, asks before a send that holds a private word, and lets `ls` through |
| `presets` | the preset links equal the `PRESETS` setting |
| `generated` | the generated Cursor rule and Opencode files carry the `generated-by: dotfiles` marker |
| `timezone` | `TIMEZONE` is empty or a real zone name (macOS and Linux) |
| `no-leak` | no private word in a tracked file |

To add a tool, see [Customize](customize.md#customizing).

## Cheat sheet

| Key / command | Does |
|---------------|------|
| `Ctrl-R` | Synced fuzzy history (atuin) |
| `Ctrl-T` · `Alt-C` | Fuzzy pick a file · `cd` into a dir (fzf) |
| `Ctrl-Space` | Accept the greyed-out autosuggestion |
| `z <name>` | Jump to a frequent directory (zoxide) |
| `y` | Open yazi; `cd`s to where you quit |
| `ll` · `la` · `lt` | Long · all · tree listing (eza) |
| `lg` · `lzd` · `ghd` | lazygit · oxker (Docker) · gh-dash |
| `git dft` | Structural diff (difftastic) |

## Window management

[AeroSpace](https://nikitabobko.github.io/AeroSpace) tiles windows (macOS). Config:
[`home/.config/aerospace/aerospace.toml`](../home/.config/aerospace/aerospace.toml). All keys use `⌥`:
`⌥ h/j/k/l` focus · `⌥⇧ h/j/k/l` move · `⌥ 1–9` workspaces · `⌥ f` fullscreen · `⌥⇧ ;` service
mode (reload, reset).

## Secrets

This repo is public, so no plaintext secret goes in it:

- **Quick and local:** exports in `~/.config/zsh/local.zsh` (git-ignored).
- **Identity:** `~/.gitconfig.local` (git-ignored), seeded once by `./dotfiles`.
- **Portable:** [rage](https://github.com/str4d/rage) (age) is installed to encrypt and decrypt a
  file by hand (`rage -p`). Nothing wires it into the link step.
