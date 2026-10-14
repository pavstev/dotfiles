# dotfiles guide

The full reference. To set up a Mac or a Linux machine, start with the [README](../README.md).

> [!IMPORTANT]
> Run `./dotfiles` from the Terminal app, not inside a Claude Code session. Claude rewrites
> `~/.claude` during a live session, which can race the link step. If that happens, run
> `dots` again from a plain shell.

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
- 🧳 **Portable**: Apple Silicon + Intel Macs, and Debian 13 and Ubuntu 24.04; machine-specific bits live in gitignored local files.
- 🤖 **AI-ready**: Claude Code skills, agents, hooks, and settings live here and follow you everywhere.
- 🎨 **Coherent**: one warm copper-on-stone palette across Ghostty, Starship, fzf, delta, atuin, and the prompt.

## Daily workflow

The repo (`~/dotfiles/home`) **is** `$HOME`: most files are symlinked, so editing
either side edits the same file:

```sh
$EDITOR ~/dotfiles/home/.config/zsh/init.zsh   # edit: it's live immediately, no apply step
dotd                                            # preview source changes  (git diff)
dots                                            # pull + relink + reprovision: for new files, packages, machines
dots settings                                   # the settings menu: git name, Java, private rules and more
dot                                             # cd into the repo
```

Want a new tool? Drop it in the `Brewfile` and run `dots`: the brew step re-runs
every time and no-ops when nothing's missing.

## What's inside

| Area | Tools |
|------|-------|
| **Engine** | plain git + [`./dotfiles`](../dotfiles) · [age](https://age-encryption.org) (available for manual secrets) |
| **Shell** | zsh + Oh My Zsh · [Starship](https://starship.rs) · [atuin](https://atuin.sh) (Ctrl-R) · [carapace](https://carapace.sh) · autosuggestions · fast-syntax-highlighting · fzf-tab |
| **Navigation** | [fzf](https://github.com/junegunn/fzf) · [zoxide](https://github.com/ajeetdsouza/zoxide) · [yazi](https://github.com/sxyazi/yazi) (`y`) · [eza](https://github.com/eza-community/eza) · [bat](https://github.com/sharkdp/bat) |
| **Search** | [ripgrep](https://github.com/BurntSushi/ripgrep) · [fd](https://github.com/sharkdp/fd) · [ast-grep](https://ast-grep.github.io) |
| **System** | [dust](https://github.com/bootandy/dust) · [duf](https://github.com/muesli/duf) · [procs](https://github.com/dalance/procs) · [btop](https://github.com/aristocratos/btop) · [glow](https://github.com/charmbracelet/glow) |
| **Git** | [delta](https://github.com/dandavison/delta) · [difftastic](https://difftastic.wilfred.me.uk) · [lazygit](https://github.com/jesseduffield/lazygit) · [gh](https://cli.github.com) + [gh-dash](https://github.com/dlvhdr/gh-dash) |
| **Containers** | [lazydocker](https://github.com/jesseduffield/lazydocker) · [OrbStack](https://orbstack.dev) |
| **Workflow** | [mise](https://mise.jdx.dev) · [just](https://github.com/casey/just) · [direnv](https://direnv.net) · jq · yq · [xh](https://github.com/ducaale/xh) · [gum](https://github.com/charmbracelet/gum) · hyperfine · tealdeer |
| **Python** | [uv](https://docs.astral.sh/uv) · [ruff](https://docs.astral.sh/ruff) · [ty](https://docs.astral.sh/ty) |
| **JS / TS** | [Biome](https://biomejs.dev) · Node + pnpm via mise |
| **Go** | [golangci-lint](https://golangci-lint.run) · [air](https://github.com/air-verse/air) |
| **Rust** | [rustup](https://rustup.rs) · cargo-binstall · [cargo-nextest](https://nexte.st) · [bacon](https://dystroy.org/bacon) |
| **Apps** | [Ghostty](https://ghostty.org) · VS Code · [Cursor](https://cursor.com) · [Raycast](https://raycast.com) · [AeroSpace](https://nikitabobko.github.io/AeroSpace) · FiraCode Nerd Font |

> Modern aliases are **interactive-only**: they never leak into scripts. Need the real
> thing for a one-off? `\grep` or `command grep`.

## Claude Code

The repo owns your global Claude setup and symlinks each piece into `~/.claude`
**individually** (never the whole dir, since Claude writes its session state there):

```
claude/
├── settings.json   ⇒ ~/.claude/settings.json     built by dots (see below), shared base
├── CLAUDE.md       → ~/.claude/CLAUDE.md          global instructions
├── statusline.sh   → ~/.claude/statusline.sh      copper-themed status line
├── skills/         → ~/.claude/skills  (per-skill; Cursor reads it)
├── agents/         → ~/.claude/agents
├── hooks/          → ~/.claude/hooks
└── output-styles/  → ~/.claude/output-styles
```

`settings.json` is the one piece that is built, not linked, because Claude writes it
itself. `dots` writes `~/.claude/settings.json` from `claude/settings.json` (the shared
base) with your private layer `~/.config/dotfiles/claude-settings.json` on top (mode 600,
never in git, on any profile). Objects merge, your values win, and `permissions.allow`,
`deny` and `ask` add up. A key set to `null` in the layer removes it.

Before it builds, `dots` compares the live file with what it wrote last time
(`~/.config/dotfiles/claude-settings.last.json`). Each key Claude or you changed since
then moves into the private layer, and `dots` prints its name (never its value). So
nothing is lost, the repo stays clean, and the next run prints nothing new. A rewrite that
only changes the layout counts as no change. To share a kept key with everyone, run
`python3 scripts/agent-config.py claude-settings promote <key>` and commit.

**Skills are the headline.** One source (`claude/skills`), synced everywhere by
[`skill-sync.sh`](../claude/skill-sync.sh): write once, use in every agent. Skills use the
portable [SKILL.md](https://cursor.com/docs/skills) format, so the same folder works in
Claude Code **and Cursor**:

- **Claude** gets skills **added per-item, non-destructively**: your own skills
  there are never moved or overwritten (on a name clash, *yours* wins); `_template` and
  loose docs are skipped. Removing them is just deleting the symlinks.
- **Cursor** reads `~/.claude/skills` natively, so nothing is linked into
  `~/.cursor/skills`. The script never touches that folder.

Ships the `ps:*` workflow skills (provenance and test notes in
[`SOURCES.md`](../claude/skills/SOURCES.md)); grow your own from
[`_template`](../claude/skills/_template/SKILL.md). Machine-local permission grants stay in
`~/.claude/settings.local.json`, untouched.

## Cursor

Cursor (the app and the `agent` / `cursor-agent` CLI) uses the same `claude/` source.
Edit `claude/`, then run `dots`. Step `scripts/install.d/60-cursor.sh` runs only when
Cursor is present:

1. It installs the Cursor CLI with the official installer
   (`curl https://cursor.com/install -fsS | bash`) if `agent` is missing. The CLI goes to
   `~/.local/bin`.
2. It installs the Brewfile `vscode` extensions into Cursor with
   `cursor --install-extension`. It skips `ms-python.vscode-pylance` (not on Open VSX).
3. It runs [`scripts/agent-config.py`](../scripts/agent-config.py) `cursor`.

What Cursor gets:

- **Linked:** `settings.json` and `keybindings.json`, as before. `settings.json` sets
  `cursor.general.globalCursorIgnoreList` to hide `.env` and secrets from AI features.
- **Native:** skills from `~/.claude/skills`, project `CLAUDE.md` and `AGENTS.md`.
- **Generated:** `~/.cursor/rules/claude-global.mdc` (from CLAUDE.md),
  `~/.cursor/rules/claude-output-style.mdc` (the active output style), and
  `~/.cursor/agents/*.md`. These files carry a `generated-by: dotfiles` marker. A file
  without it is yours, and the script leaves it alone.
- **Merged key by key:** `~/.cursor/hooks.json` (guard adapter
  [`cursor/hooks/guard_cursor.py`](../cursor/hooks/guard_cursor.py)), `~/.cursor/mcp.json`
  (from [`cursor/mcp.json`](../cursor/mcp.json), ships `cloudflare-docs`; opt-in on every
  profile: turn on Cursor MCP in `dots settings`), and
  `~/.cursor/cli-config.json` (`permissions`, `attribution`, `statusLine`). Your own
  values win. Commit attribution to the agent is always off.

Some Claude features have no Cursor match (output-style switching, most guards, the IDE
status line, claude.ai connectors). The full mapping and gap table is in
[`docs/cursor-port.md`](cursor-port.md). The first run offers `agent login` and opens
Cursor.app so you can sign in there too.

## Multi-machine

- **Bootstrap**: the one-liner in the [README](../README.md) turns a blank Mac into this, end to end.
- **Synced history**: [atuin](https://atuin.sh) owns `Ctrl-R` (the `↑` arrow stays native).
  `atuin register` / `atuin login` once, and your history follows you.
- **Per-machine overrides**: gitignored `~/.config/zsh/local.zsh` & `~/.zshrc.local` for
  shell, `~/.gitconfig.local` for git identity, `Brewfile.local` (see
  [`Brewfile.local.example`](../Brewfile.local.example)) for packages.
- **Manual sync, no daemon**: one machine edits and pushes; every other machine just
  runs `dots` (`./dotfiles`) whenever it wants to catch up: pull, relink, reprovision.
  Nothing runs in the background. If you want automatic pulls, wire your own cron or
  login hook to call `dots`.
- **Agent-aware**: [`AGENTS.md`](../AGENTS.md) (read by Cursor) and the repo's
  [`CLAUDE.md`](../CLAUDE.md) (which imports it) teach any AI agent how to operate this repo,
  so Claude or Cursor on a fresh machine knows the workflow without being told.

## Linux

`./dotfiles` runs on Debian 13 and Ubuntu 24.04 (x86_64 and arm64) as a normal user with `sudo`.
The code asks the OS with `is_macos` and `is_linux` (`scripts/setup-lib.sh`); a Mac behaves as
before. On Linux:

- **Packages:** step `05-linux-prereqs` installs `build-essential`, `curl`, `file`, `git`,
  `procps`, `python3` and `zsh` with `apt`. Step `10-homebrew` then installs Homebrew on Linux
  (`/home/linuxbrew/.linuxbrew`), and `20-brew-bundle` installs the Brewfile from it. A temp
  copy of the Brewfile drops the lines that only work on a Mac (`cask`, `mas`, `tap`,
  `vscode`). One Brewfile and one tool version on both systems, and no `batcat` or `fdfind`.
- **Skipped on Linux:** `70-macos-defaults`, `72-touch-id-sudo` and `75-terminal-appearance`
  print `macOS only, skipping`. The Claude desktop app offer, AeroSpace and the Fira Code Nerd Font
  are Mac-only too.
- **Links:** `~/.config/Code/User/*` and `~/.config/Cursor/User/*` link to the same
  files that a Mac gets under `~/Library/Application Support`. Ghostty reads
  `~/.config/ghostty` on both.
- **Shell:** `init.zsh` finds Homebrew on Linux, falls back to `C.UTF-8` when `en_US.UTF-8` is
  not built, and sets `GIT_EDITOR` to `$EDITOR` in interactive zsh when VS Code is not installed
  (`~/.gitconfig` asks for `code --wait`; git from bash or a hook still uses that).
- **Not changed:** the login shell. `78-linux-extras` prints the `chsh` line when it is not zsh.
- **Without `sudo`:** install the packages in `LINUX_PREREQS` (`scripts/setup-lib.sh`) as root,
  and install Homebrew yourself under `~/.linuxbrew`; `dots` finds it there. Homebrew on Linux
  needs a user that is not root.
- **Other distros:** the prerequisites step needs `apt-get`. Without it, it prints the package
  list and carries on; the Homebrew installer may then complain about missing tools.
- **Skills:** `ps:bag-mode` is for a Mac laptop. On Linux its script says so and exits 0.
  The other skill scripts run on both.

### Test it

`scripts/test-linux.sh <image>` runs `./dotfiles` twice in a fresh container, as a user with
`sudo`, on a host with podman or docker (rootless podman is enough):

```sh
bash scripts/test-linux.sh debian:13
bash scripts/test-linux.sh ubuntu:24.04
```

It builds a small base image once, copies the working tree in (git-ignored files stay out),
runs `./dotfiles` and again `./dotfiles`, and checks: run 1 exits 0 with no warning, run 2
changes nothing, the links, the tools, `zsh -ic exit` prints nothing, `bash -n` on every shell
script, and the four Python test files. `--keep` leaves the container for a look
(`podman exec -it dots-test-debian-13 bash`), `--reuse` syncs your edits into it and runs again.
It runs without a terminal, so the gum menus, the password prompts and the sign-in step are
not covered. A run takes about 10 minutes.

## Secrets

This repo is **public**, so plaintext secrets never enter it:

- *Quick + local* → exports in `~/.config/zsh/local.zsh` (gitignored).
- *Identity* → `~/.gitconfig.local` (gitignored), seeded once by `./dotfiles`.
- *Portable, if you ever need it* → [age](https://age-encryption.org) is installed for
  manual encrypt/decrypt of a file; there's no automatic wiring into the link step.

## Window management

[AeroSpace](https://nikitabobko.github.io/AeroSpace) tiling, configured in
[`home/.config/aerospace/aerospace.toml`](../home/.config/aerospace/aerospace.toml).
All `⌥`-driven: `⌥ h/j/k/l` focus · `⌥⇧ h/j/k/l` move · `⌥ 1–9` workspaces · `⌥ f`
fullscreen · `⌥⇧ ;` service mode (reload / reset).

## Cheat sheet

| Key / command | Does |
|---------------|------|
| `Ctrl-R` | Synced fuzzy history (atuin) |
| `Ctrl-T` · `Alt-C` | Fuzzy pick a file · `cd` into a dir (fzf) |
| `Ctrl-Space` | Accept the greyed-out autosuggestion |
| `z <name>` | Jump to a frequent directory (zoxide) |
| `y` | Open yazi; `cd`s to where you quit |
| `ll` · `la` · `lt` | Long · all · tree listing (eza) |
| `lg` · `lzd` · `ghd` | lazygit · lazydocker · gh-dash |
| `git dft` | Structural diff (difftastic) |
| `dots` · `dotd` | pull + relink + reprovision · preview source changes |
| `dot` | `cd` into this repo |

## Repo layout

```
dotfiles/
├── assets/logo.svg           # the mark above
├── AGENTS.md  CLAUDE.md      # how AI agents (Claude/Cursor) operate this repo
├── bootstrap.sh              # fresh-machine one-liner (Mac, Debian, Ubuntu)
├── dotfiles                  # the one command: pull + link + provision
├── Brewfile                  # every package + VS Code extensions (Brewfile.local for extras)
├── claude/                   # Claude config, symlinked into ~/.claude
│   ├── settings.json  CLAUDE.md  statusline.sh  skill-sync.sh
│   └── skills/ agents/ hooks/ output-styles/
├── cursor/                   # Cursor-only sources, merged into ~/.cursor
│   ├── mcp.json              # MCP servers (cloudflare-docs); your entries win
│   └── hooks/guard_cursor.py # Cursor hook adapter for guard G3 (+ test)
├── docs/guide.md            # this guide
├── docs/cursor-port.md       # Cursor port: profiles, mapping, gap table
├── home/                     # mirrors $HOME 1:1; only paths in `dotfiles`'s link list are touched
│   ├── .zshrc  .gitconfig    # identity lives in ~/.gitconfig.local (gitignored, not here)
│   └── .config/
│       ├── zsh/{init,aliases,skill}.zsh
│       ├── starship.toml  ghostty/config
│       ├── atuin/  aerospace/aerospace.toml
│       ├── gh/config.yml     # gh CLI aliases + protocol (hosts.yml never linked)
│       ├── bat/config        # bat style settings
│       ├── lazygit/config.yml  btop/btop.conf  yazi/yazi.toml
│       └── tealdeer/config.toml  fastfetch/config.jsonc
│   Library/Application Support/                     (still under home/, shown separately for width)
│       Code/User/{settings,keybindings}.json         # VS Code: symlinked, edit either side
│       Cursor/User/{settings,keybindings}.json       # Cursor: same
├── scripts/
│   ├── setup-lib.sh          # shared step helpers (step/info/ok/warn/skip/have/brew_env/link_item),
│   │                         #   gum prompts (banner/ask_text/ask_one/ask_long) and dotfiles_profile
│   ├── settings.sh           # the settings menu: first run and `dots settings`
│   ├── agent-config.py       # generates and merges the Cursor files
│   ├── test_agent_config.py  # its unit tests (python3 scripts/test_agent_config.py)
│   ├── test-dotfiles.sh      # ./dotfiles in a temp HOME: profile, owner rules, rerun, pull restart
│   ├── install.d/            # ordered, skip-aware provisioning steps, run by `./dotfiles`:
│   │                         #   05 linux-prereqs (Linux only)
│   │                         #   → 10 homebrew
│   │                         #   → 20 brew-bundle → 30 omz → 40 rust → 50 node → 55 gh
│   │                         #   → 57 claude-code → 60 cursor → 70 macos → 72 touch-id
│   │                         #   → 75 terminal → 77 git-hooks → 78 linux-extras → 90 sign-in
│   │                         #   (70, 72, 75: macOS only; 05, 78: Linux only)
│   │                         #   (DOTFILES_STEPS="60-cursor" ./dotfiles runs only the named steps;
│   │                         #    pull, links and skill sync still run)
│   ├── fix-brew-permissions.sh   # macOS only
│   ├── test-linux.sh         # runs ./dotfiles twice in a Debian or Ubuntu container
│   └── git-hooks/            # secret scan, private words, push check (install.d/77)
```

## Customizing

| Want to change… | Edit | Then |
|-----------------|------|------|
| Shell / aliases | `home/.config/zsh/{init,aliases}.zsh` | live immediately |
| Prompt | `home/.config/starship.toml` | live immediately |
| Terminal | `home/.config/ghostty/config` | live immediately |
| Git | `home/.gitconfig` (identity is in `~/.gitconfig.local`, not here) | live immediately |
| VS Code / Cursor | `home/Library/Application Support/{Code,Cursor}/User/settings.json` | live immediately |
| VS Code extensions | `Brewfile`: add `vscode "publisher.ext-id"` lines | `dots` (brew step re-runs; 60-cursor.sh adds them to Cursor) |
| VS Code/Cursor UI edits | edit in the app; it's a symlink, so the repo file changes too | `git status` in `~/dotfiles` to see it, then commit |
| bat / lazygit / btop / yazi / tealdeer / fastfetch | `home/.config/<tool>/…` | live immediately |
| Claude | anything under `claude/` | live immediately (or `dots` for a new skill/file) |
| Claude settings (`claude/settings.json`) | the repo base for everyone; your own keys go in `~/.config/dotfiles/claude-settings.json` | `dots` (it builds `~/.claude/settings.json`) |
| Share a key Claude saved for you | `python3 scripts/agent-config.py claude-settings promote <key>` (e.g. `permissions.allow`) | commit `claude/settings.json` |
| Hide a tool Claude never uses (it then costs no tokens) | add its bare name, or `mcp__<server>__*`, to `permissions.deny` in `claude/settings.json`. A connector has two names: its ID in the app, `claude_ai_<Name>` in the terminal | `dots`, then the next session (a deny beats every allow) |
| Cursor rules, agents, hooks, MCP | `claude/` (shared source) or `cursor/` | `dots` (regenerates `~/.cursor`) |
| Machine-local | `~/.config/zsh/local.zsh`, `~/.gitconfig.local`, `Brewfile.local` | new shell |
| Settings: git name, profile, Cursor MCP, Java, steps to skip, private words and rules | `dots settings` (kept in `~/.config/dotfiles`, never in git; all names in `settings.example`) | at once (Java: new shell) |

## Updating

Fully manual, by design: no background process. Pull and reprovision on demand:

```sh
dots        # alias for ./dotfiles: git pull + relink + reprovision
```

The brew step runs every time but no-ops when nothing's missing; one-time install
steps (Touch ID, git hooks, etc.) detect their own prior state and skip too. Want
automatic pulls on a given machine? Point your own cron or login hook at `dots`.
Nothing in this repo does that for you.

## Uninstall

Nothing here holds your machine hostage:

```sh
rm ~/.zshrc ~/.gitconfig ~/.config/starship.toml   # etc. These are symlinks; removing them is enough
```

For `~/.claude`, remove the symlinks the same way. Any pre-existing config the link
step overwrote is backed up as `*.bak-<timestamp>` next to the original path. Restore
from there. Homebrew packages, Oh My Zsh, and toolchains stay installed.

For Cursor, delete the generated files and our merged entries:

1. `rm ~/.cursor/rules/claude-global.mdc ~/.cursor/rules/claude-output-style.mdc`
2. `rm ~/.cursor/agents/code-reviewer.md ~/.cursor/agents/researcher.md ~/.cursor/.dotfiles-managed.json`
3. In `~/.cursor/hooks.json`, delete the 5 entries that run `cursor/hooks/guard_cursor.py`.
4. In `~/.cursor/mcp.json`, delete `cloudflare-docs` (only there when MCP is on).

## Troubleshooting

**`brew bundle` says "Permission denied".** Homebrew is owned by another user. Run
`~/dotfiles/scripts/fix-brew-permissions.sh`, then `dots`.

**`~/.claude` didn't get linked.** You ran `./dotfiles` inside a Claude Code session,
which can race Claude rewriting `~/.claude` live. Re-run `dots` from a plain shell.

**Linux: a step failed at `apt` or Homebrew.** Run `dots` again after fixing the line above it.
The Homebrew installer needs `sudo` and a user that is not root.

**Icons look like boxes.** The Nerd Font is wired into **both** terminals automatically:
Ghostty via its config, Apple Terminal via `scripts/install.d/75-terminal-appearance.sh`
(needs Automation permission on first run). Other terminals: set the font to "FiraCode
Nerd Font Mono".

**`compinit: insecure directories`.** Run `chmod -R go-w "$(brew --prefix)/share"` once.
