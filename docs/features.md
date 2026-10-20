# Features

Why this repo exists, what is inside, and the daily commands and shortcuts.

← [Docs](README.md)

## Why

Most dotfiles rot the moment you touch a second machine. This one doesn't. Plain
git is the single source of truth; every managed file in `$HOME` is a **symlink** back into
this repo, so there's no templating engine and no separate "apply" step for content
changes. The old tools get eaten by faster, prettier ones. The terminal goes **brrr**.

- ⚡️ **Fast**: lazy, well-ordered zsh loading; cold start ≈ 0.3 s.
- 🦴 **Modern**: `eza`, `bat`, `rg`, `fd`, `zoxide`, `fzf`, `atuin`, `delta` replace the fossils.
- ♻️ **Reproducible**: `./dotfiles` is idempotent; every step *looks before it leaps*:
  already-installed tools are detected and **skipped**, never reinstalled. Steps are
  **fault-tolerant**: one that fails (offline, missing permission) warns and steps aside
  instead of derailing the whole run.
- 🛟 **Override-safe**: a pre-existing config file is **moved aside** to
  `<file>.bak-<timestamp>` before its link goes in, so adopting these defaults on a
  machine that's already set up never costs you your old setup.
- 🧳 **Portable**: macOS (Apple Silicon and Intel), Linux (Debian, Ubuntu, Fedora, Arch, Alpine), and Windows (Git Bash); machine-specific bits live in gitignored local files.
- 🤖 **AI-ready**: Claude Code skills, agents, hooks, and settings live here and follow you everywhere. Cursor and Opencode read the same source.
- 🎨 **Coherent**: one warm copper-on-stone palette across Ghostty, Starship, fzf, delta, atuin, and the prompt.

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

> Modern aliases are **interactive-only**: they never leak into scripts. Need the real
> thing for a one-off? `\grep` or `command grep`.

## Daily workflow

The repo (`~/dotfiles/home`) **is** `$HOME`: most files are symlinked, so editing
either side edits the same file:

```sh
$EDITOR ~/dotfiles/home/.config/zsh/init.zsh   # edit: it's live immediately, no apply step
dotd                                            # preview source changes  (git diff)
dots                                            # pull + relink + reprovision: for new files, packages, machines
dots settings                                   # the settings menu: git name, Java, private rules and more
dots -v                                         # the same run, with every line (the default is one line per step)
dots doctor                                     # check the setup: PASS or FAIL per check, changes nothing
dot                                             # cd into the repo
```

`dots doctor` prints one line per check and exits 1 when one fails. Each FAIL line says what is wrong
and the fix, usually `Run: dots`. The checks:

| Check | What it looks at |
|-------|------------------|
| `links` | every linked file points into the repo |
| `settings-built` | `~/.claude/settings.json` exists and every hook runs `rust-bin` |
| `bins-fresh` | each Rust program is built and not older than its source |
| `guard-selftest` | the guard denies BSD `sed -i` (macOS rules) and a read of `local.zsh`, asks before a send that holds a private word, and lets `ls` through |
| `presets` | the preset links equal the `PRESETS` setting |
| `generated` | the generated Cursor rule and Opencode files carry the `generated-by: dotfiles` marker |
| `timezone` | `TIMEZONE` is empty or a real zone name (macOS and Linux) |
| `no-leak` | no private word in a tracked file |

Want a new command-line tool? Add one line to [`tools.list`](../tools.list) and run `dots`. Each
line names the tool, its commands, the OSes it is for and its sources in order (mise first, then
`apk`, `winget`, `scoop`, `cargo`, `brew`). `scripts/tools --check` lists what is missing. Mac apps
and fonts go in the `Brewfile`.

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
| `dots` · `dotd` | pull + relink + reprovision · preview source changes |
| `dot` | `cd` into this repo |

## Window management

[AeroSpace](https://nikitabobko.github.io/AeroSpace) tiling, configured in
[`home/.config/aerospace/aerospace.toml`](../home/.config/aerospace/aerospace.toml).
All `⌥`-driven: `⌥ h/j/k/l` focus · `⌥⇧ h/j/k/l` move · `⌥ 1–9` workspaces · `⌥ f`
fullscreen · `⌥⇧ ;` service mode (reload / reset).

## Secrets

This repo is **public**, so plaintext secrets never enter it:

- *Quick + local* → exports in `~/.config/zsh/local.zsh` (gitignored).
- *Identity* → `~/.gitconfig.local` (gitignored), seeded once by `./dotfiles`.
- *Portable, if you ever need it* → [rage](https://github.com/str4d/rage) (age) is installed for
  manual encrypt/decrypt of a file (`rage -p`); there's no automatic wiring into the link step.
