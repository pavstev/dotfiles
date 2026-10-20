# dotfiles guide

The full reference. To set up macOS, Linux or Windows, start with the [README](../README.md).

> [!IMPORTANT]
> Run `./dotfiles` from a plain terminal (Git Bash on Windows), not inside a Claude Code session. Claude rewrites
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
- 🧳 **Portable**: macOS (Apple Silicon and Intel), Linux (Debian, Ubuntu, Fedora, Arch, Alpine), and Windows (Git Bash); machine-specific bits live in gitignored local files.
- 🤖 **AI-ready**: Claude Code skills, agents, hooks, and settings live here and follow you everywhere — Cursor and Opencode read the same source.
- 🎨 **Coherent**: one warm copper-on-stone palette across Ghostty, Starship, fzf, delta, atuin, and the prompt.

## Daily workflow

The repo (`~/dotfiles/home`) **is** `$HOME`: most files are symlinked, so editing
either side edits the same file:

```sh
$EDITOR ~/dotfiles/home/.config/zsh/init.zsh   # edit: it's live immediately, no apply step
dotd                                            # preview source changes  (git diff)
dots                                            # pull + relink + reprovision: for new files, packages, machines
dots settings                                   # the settings menu: git name, Java, private rules and more
dots -v                                         # the same run, with every line (the default is one line per step)
dot                                             # cd into the repo
```

Want a new command-line tool? Add one line to [`tools.list`](../tools.list) and run `dots`. Each
line names the tool, its commands, the OSes it is for and its sources in order (mise first, then
`apk`, `winget`, `scoop`, `cargo`, `brew`). `scripts/tools --check` lists what is missing. Mac apps
and fonts go in the `Brewfile`.

## What's inside

| Area | Tools |
|------|-------|
| **Engine** | plain git + [`./dotfiles`](../dotfiles) · [rage](https://github.com/str4d/rage) (age encryption, for manual secrets) |
| **Shell** | zsh + Oh My Zsh · [Starship](https://starship.rs) · [atuin](https://atuin.sh) (Ctrl-R) · [carapace](https://carapace.sh) · autosuggestions · fast-syntax-highlighting · fzf-tab |
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

## Claude Code

The repo owns your global Claude setup and symlinks each piece into `~/.claude`
**individually** (never the whole dir, since Claude writes its session state there):

```
claude/
├── settings.json   ⇒ ~/.claude/settings.json     built by dots (see below), shared base
├── CLAUDE.md       → ~/.claude/CLAUDE.md          global instructions
├── skills/         → ~/.claude/skills  (per-skill; Cursor and Opencode read it)
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
`scripts/agent-config claude-settings promote <key>` and commit. The copper status line is the
`statusline` Rust bin, run as `bash ~/.claude/hooks/rust-bin statusline`.

**Skills are the headline.** One source (`claude/skills`), linked everywhere by
`dots` (the skills step): write once, use in every agent. Skills use the
portable [SKILL.md](https://cursor.com/docs/skills) format, so the same folder works in
Claude Code, **Cursor** and **Opencode**:

- **Claude** gets skills **added per-item, non-destructively**: your own skills
  there are never moved or overwritten (on a name clash, *yours* wins); `_template` and
  loose docs are skipped. Removing them is just deleting the symlinks.
- **Cursor** and **Opencode** read `~/.claude/skills` natively, so nothing is linked into
  `~/.cursor/skills` (there is no `~/.config/opencode/skills` at all). The script never
  touches either folder.

Ships the `ps:*` workflow skills (provenance and test notes in
[`SOURCES.md`](../claude/skills/SOURCES.md)); grow your own from
[`_template`](../claude/skills/_template/SKILL.md). Machine-local permission grants stay in
`~/.claude/settings.local.json`, untouched.

## Cursor

Cursor (the app and the `agent` / `cursor-agent` CLI) uses the same `claude/` source.
Edit `claude/`, then run `dots`. Step `60-cursor` runs only when
Cursor is present:

1. It installs the Cursor CLI with the official installer
   (`curl https://cursor.com/install -fsS | bash`) if `agent` is missing. The CLI goes to
   `~/.local/bin`.
2. It installs the Brewfile `vscode` extensions into Cursor with
   `cursor --install-extension`. It skips `ms-python.vscode-pylance` (not on Open VSX).
3. It runs [`scripts/agent-config`](../scripts/agent-config) `cursor`.

What Cursor gets:

- **Linked:** `settings.json` and `keybindings.json`, as before. `settings.json` sets
  `cursor.general.globalCursorIgnoreList` to hide `.env` and secrets from AI features.
- **Native:** skills from `~/.claude/skills`, project `CLAUDE.md` and `AGENTS.md`.
- **Generated:** `~/.cursor/rules/claude-global.mdc` (from CLAUDE.md),
  `~/.cursor/rules/claude-output-style.mdc` (the active output style), and
  `~/.cursor/agents/*.md`. These files carry a `generated-by: dotfiles` marker. A file
  without it is yours, and the script leaves it alone.
- **Merged key by key:** `~/.cursor/hooks.json` (guard adapter: the
  `guard_cursor` bin, run through `claude/hooks/rust-bin`), `~/.cursor/mcp.json`
  (from [`cursor/mcp.json`](../cursor/mcp.json), ships `cloudflare-docs`; opt-in on every
  profile: turn on Cursor MCP in `dots settings`), and
  `~/.cursor/cli-config.json` (`permissions`, `attribution`, `statusLine`). Your own
  values win. Commit attribution to the agent is always off.

Some Claude features have no Cursor match (output-style switching, most guards, the IDE
status line, claude.ai connectors). The full mapping and gap table is in
[`docs/cursor-port.md`](cursor-port.md). The first run offers `agent login` and opens
Cursor.app so you can sign in there too.

## Opencode

Opencode (the `opencode` CLI and the Opencode app) uses the same `claude/` source.
Edit `claude/`, then run `dots`. Step `58-opencode` verifies the
`opencode` CLI (installed from `tools.list`), and `agent-config opencode` — wired into
`./dotfiles` itself, not into a step, so `dots settings` picks up a changed setting —
generates what Opencode needs:

1. **Native:** skills from `~/.claude/skills` — every `ps:*` skill works in Opencode,
   in every project, because Opencode reads `~/.claude/skills` itself. Project
   `AGENTS.md` and `CLAUDE.md` load there too.
2. **Generated:** `~/.config/opencode/AGENTS.md` (the global rules from `CLAUDE.md`, plus
   the owner's private rules and the active output style) and `~/.config/opencode/agents/*.md`
   (from `claude/agents`), each a `mode: subagent` agent with `permission.skill: allow`
   (read-only agents also get `permission.edit: deny`). These carry a `generated-by: dotfiles`
   marker; a file without it is yours and is left alone. The copper theme goes to
   `~/.config/opencode/themes/copper.json` and `tui.json` names it — a `tui.json` that already
   names another theme is yours, and only gets reported.
3. **Merged key by key:** `~/.config/opencode/opencode.jsonc` — `permission` (translated
   from `claude/settings.json`; your keys win, and a rule dropped from the repo is dropped
   here too) and `mcp` (from `cursor/mcp.json`, opt-in on every profile: turn on Opencode MCP
   in `dots settings`, or `DOTFILES_MCP=on`).
4. **Per project:** paths in `OPENCODE_PROJECTS` (`~` allowed) each get a `.opencode/` that
   symlinks the global config and agents. What the project already has wins, always.
5. **Guard plugin:** `~/.config/opencode/plugins/dots-guard.ts` (generated, with the marker)
   runs the same Rust guard before every shell, read, grep and MCP call. Opencode cannot ask
   through a plugin, so a deny and an ask both block the call. If the guard cannot run, the
   call goes through and the plugin warns once per session.

`AGENTS.md` is the one worth explaining: Opencode reads it as its global instructions and only
falls back to `~/.claude/CLAUDE.md` when it is missing. Generating it is what carries the owner
rules and the output style into Opencode, since `CLAUDE.md` alone has neither.

What Opencode does *not* get: output-style switching and the claude.ai connectors, and most
of the guard checks (see the table below). The full mapping and gap table is in
[`docs/opencode-port.md`](opencode-port.md). The first run offers `opencode auth login`
(`opencode auth list` says so until one is saved).

### Where each guard check runs

One Rust library (`guard_checks`) holds the shared checks, so the three apps run the same code.
A check also needs its preset on (`dots settings`), and `GUARD_OFF` turns one off in every app.

| Check | Claude Code | Cursor | Opencode |
|-------|-------------|--------|----------|
| G14 BSD `sed -i` on macOS | deny | deny (`preToolUse`, shell) | deny (plugin) |
| G15 reading `local.zsh`, printing command lines | deny | deny (`preToolUse`, shell, read, grep) | deny (plugin) |
| G16 text with a private word or a secret leaving the machine | ask | ask (`beforeShellExecution`, `beforeMCPExecution`) | blocked, no ask |
| G17 assistant attribution in a commit, PR or issue (preset `no-ai-attribution`) | deny | deny (`preToolUse`, shell) | deny (plugin) |
| G3 context ceiling | yes | yes (`guard_cursor`, other modes) | no |
| G1, G2, G4 to G13, pace, router | yes | no (see the gaps in `docs/cursor-port.md`) | not tested |

The Opencode plugin runs the whole `guard pretool` hook, so a Claude check that looks at a shell
command (for example G2) can also fire there. Only G14 to G17 are tested in Opencode.

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

## Linux

`./dotfiles` runs on Debian, Ubuntu, Fedora and Arch (x86_64 and arm64) as a normal user with `sudo`.
The code asks the OS with `is_macos`, `is_linux` and `is_windows` (`scripts/setup-lib.sh`). On Linux:

- **Packages:** step `05-linux-prereqs` installs build tools, `curl`, `file`, `git`, `procps`
  and `zsh` with apt, dnf, pacman or apk (`scripts/linux-prereqs.txt`
  has the names for each). Step `12-tools` then installs the command-line tools of `tools.list`
  with mise (no Homebrew on Linux): it installs mise first, then each tool from the first source
  that works. Same tools and versions as macOS, and no `batcat` or `fdfind`. On Alpine the native
  `apk` package comes first for the tools whose release binaries need glibc.
- **Skipped on Linux:** `70-macos-defaults`, `72-touch-id-sudo` and `75-terminal-appearance`
  print nothing (`dots -v` shows `macOS only, skipping`). The Claude desktop app offer, AeroSpace and the Fira Code Nerd Font
  are macOS-only too.
- **Links:** `~/.config/Code/User/*` and `~/.config/Cursor/User/*` link to the same
  files that macOS gets under `~/Library/Application Support`. Ghostty reads
  `~/.config/ghostty` on both.
- **Shell:** `init.zsh` puts `~/.local/bin` and the mise shims on PATH, falls back to `C.UTF-8` when `en_US.UTF-8` is
  not built, and sets `GIT_EDITOR` to `$EDITOR` in interactive zsh when VS Code is not installed
  (`~/.gitconfig` asks for `code --wait`; git from bash or a hook still uses that).
- **Not changed:** the login shell. `78-linux-extras` prints the `chsh` line when it is not zsh.
- **Without `sudo`:** install the packages in `scripts/linux-prereqs.txt` as root.
  The tools need no root: mise installs them under your home. Only the `apk` source (Alpine) and the
  prerequisites use root.
- **Alpine (musl):** the tools come from `apk` first for the ones that have no musl release (eza, procs,
  delta, difftastic, fastfetch, ffmpeg, poppler), else from mise. whisper-cpp is not installed on Alpine. A tool with no source that
  starts on musl is a warning. Needs root or `sudo` for `apk`.
- **Other distros:** without apt, dnf, pacman or apk, the prerequisites step prints the commands
  to install yourself and carries on.
- **Skills:** `ps:bag-mode` is for a Mac laptop. On Linux and Windows it says so and stops.
  `ps:spawn` needs `tmux`, so it runs on macOS and Linux only. The other skill scripts run on all three.

### Test it

`scripts/test-linux.sh <image>` runs `./dotfiles` twice in a fresh container, as a user with
`sudo`, on a host with podman or docker (rootless podman is enough):

```sh
bash scripts/test-linux.sh debian:13
bash scripts/test-linux.sh ubuntu:24.04
```

It builds a small base image once, copies the working tree in (git-ignored files stay out),
runs `./dotfiles` and again `./dotfiles`, and checks: run 1 exits 0 with no warning and no shell
error, both runs end with the done line, run 2 changes nothing and takes 90 seconds at most,
`dots -v` works, the links, the tools, the Rust tools, `zsh -ic exit` prints nothing, `bash -n`
on every shell script, and that the Rust skill tools run (`skill_check`, `mic_check`,
`whatsleft_scan`, `hurry_scan`, `harness_stats`, `bag_check`, `statusline`). The Rust unit tests run
on the host with `cargo test`, not in the container. `--keep` leaves the container for a look
(`podman exec -it dots-test-debian-13 bash`), `--reuse` syncs your edits into it and runs again.
It runs without a terminal, so the gum menus, the password prompts and the sign-in step are
not covered. A run takes 10 to 25 minutes; most of it is the tool downloads and the Rust build.

To run it on another Linux machine, set `TEST_HOST` (an SSH host name) in `dots settings` and run
`scripts/os-check.sh linux`. It copies the tree there and runs `test-linux.sh` for each image in
`OS_CHECK_IMAGES`. `scripts/os-check.sh` with no OS checks every OS whose files changed: macOS,
Linux and Windows (a throwaway UTM clone of `win11-golden`). It is manual only. In Claude Code, run
`/os-check`.

The Windows check needs the VM `win11-golden` once: install UTM (`brew install --cask utm`), save
the Windows 11 Arm64 ISO from microsoft.com/software-download/windows11arm64 as
`~/VMs/iso/Win11_arm64_en-us.iso`, then run `bash scripts/win-golden.sh`. It installs Windows
unattended (about 30 minutes) and shuts the VM down. Each check then clones it, so the golden VM
stays clean.

## Windows

`./dotfiles` runs on Windows in Git Bash (MSYS/MINGW). Windows is detected with `is_windows`
(`scripts/setup-lib.sh`); WSL counts as Linux and follows the Linux path.

- **Packages:** no Homebrew on Windows. Two steps run there: `12-tools` (mise through WinGet
  `jdx.mise`, then the tools of `tools.list` marked for Windows, from mise first; WinGet and Scoop
  are fallback sources: Opencode `SST.opencode`, yt-dlp, ffmpeg, rustup) and `57-claude-code`
  (`Anthropic.ClaudeCode`, then the PowerShell installer). The shell-only tools (starship, atuin,
  zoxide, eza, yazi, bottom and others) are for macOS and Linux only; zsh does not run on Windows.
  mise and its shims go on PATH for the run; open a new Git Bash afterwards.
- **Links:** real symlinks, so Windows Developer Mode must be on (the script stops with the fix).
  `.claude`, `.config/opencode`, `.config/gh/config.yml` and the VS Code and Cursor settings
  (`%APPDATA%\{Code,Cursor}\User\`) link. `~/.gitconfig` is not linked; the git identity goes to
  `~/.gitconfig.local`, which your own `~/.gitconfig` includes.
- **Skipped on Windows:** `70-macos-defaults`, `72-touch-id-sudo` and `75-terminal-appearance`
  print nothing (`dots -v` shows `macOS only, skipping`). The Claude desktop app offer, AeroSpace and the Nerd Font are macOS-only.
- **Shell:** Git Bash provides `zsh`, `git`, `ln -s` (symlinks) and `clip.exe`. `init.zsh` loads the
  usual tools when they are on PATH.
- **Opencode config:** `scripts/agent-config opencode` generates agents, permissions and MCP under
  `~/.config/opencode` and uses `clip` for the clipboard. The output is the same on all three OSes.
- **Python:** not needed. `scripts/agent-config` builds and runs the Rust version. Without `cargo`
  the generated config steps warn and skip, and `dots` installs Rust on its next run.

## Secrets

This repo is **public**, so plaintext secrets never enter it:

- *Quick + local* → exports in `~/.config/zsh/local.zsh` (gitignored).
- *Identity* → `~/.gitconfig.local` (gitignored), seeded once by `./dotfiles`.
- *Portable, if you ever need it* → [rage](https://github.com/str4d/rage) (age) is installed for
  manual encrypt/decrypt of a file (`rage -p`); there's no automatic wiring into the link step.

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
| `lg` · `lzd` · `ghd` | lazygit · oxker (Docker) · gh-dash |
| `git dft` | Structural diff (difftastic) |
| `dots` · `dotd` | pull + relink + reprovision · preview source changes |
| `dot` | `cd` into this repo |

## Repo layout

```
dotfiles/
├── assets/logo.svg           # the mark above
├── AGENTS.md  CLAUDE.md      # how AI agents (Claude/Cursor/Opencode) operate this repo
├── bootstrap.sh              # fresh-machine one-liner (macOS, Linux, Windows)
├── dotfiles                  # the one command: pull + link + provision
├── tools.list                # every command-line tool: name, commands, OSes, sources (the one list)
├── Brewfile                  # the macOS apps, fonts + VS Code extensions (Brewfile.local for extras)
├── claude/                   # Claude config, symlinked into ~/.claude
│   ├── settings.json  CLAUDE.md
│   └── skills/ agents/ hooks/ output-styles/
├── cursor/                   # Cursor-only sources, merged into ~/.cursor
│   └── mcp.json              # MCP servers (cloudflare-docs); your entries win
├── docs/guide.md            # this guide
├── docs/cursor-port.md       # Cursor port: profiles, mapping, gap table
├── docs/opencode-port.md    # Opencode port: rules, agents, permissions, gaps
├── home/                     # mirrors $HOME 1:1; only paths in `dotfiles`'s link list are touched
│   ├── .zshrc  .gitconfig    # identity lives in ~/.gitconfig.local (gitignored, not here)
│   └── .config/
│       ├── zsh/{init,aliases,skill}.zsh
│       ├── starship.toml  ghostty/config
│       ├── atuin/  aerospace/aerospace.toml
│       ├── gh/config.yml     # gh CLI aliases + protocol (hosts.yml never linked)
│       ├── bat/config        # bat style settings
│       ├── lazygit/config.yml  bottom/bottom.toml  yazi/yazi.toml
│       └── tealdeer/config.toml  fastfetch/config.jsonc
│   Library/Application Support/                     (still under home/, shown separately for width)
│       Code/User/{settings,keybindings}.json         # VS Code: symlinked, edit either side
│       Cursor/User/{settings,keybindings}.json       # Cursor: same
├── scripts/
│   ├── setup-lib.sh          # shell helpers for the few bash scripts (step/info/ok/warn/have, is_*, setting)
│   ├── agent-config          # shim for the Rust build that generates and merges the Cursor and Opencode files
│   ├── test-dotfiles.sh      # ./dotfiles in a temp HOME: profile, owner rules, rerun, pull restart, fake Linux
│   │                         # (the provisioning steps are Rust: rust/agent-config/src/dots/install/)
│   │                         # ordered, skip-aware steps, run by `./dotfiles`:
│   │                         #   05 linux-prereqs (Linux only)
│   │                         #   → 10 homebrew → 12 tools (every OS; mise + tools.list)
│   │                         #   → 20 brew-bundle → 30 omz → 40 rust → 50 node → 55 gh
│   │                         #   → 57 claude-code → 58 opencode → 60 cursor → 70 macos → 72 touch-id
│   │                         #   → 75 terminal → 77 git-hooks → 78 linux-extras → 90 sign-in
│   │                         #   (10, 20, 70, 72, 75: macOS only; 05, 78: Linux only)
│   │                         #   (DOTFILES_STEPS="60-cursor" ./dotfiles runs only the named steps;
│   │                         #    pull, links and skill sync still run)
│   ├── fix-brew-permissions.sh   # macOS only
│   ├── test-linux.sh         # runs ./dotfiles twice in a Linux container
│   └── git-hooks/            # secret scan, private words, push check (step 77)
```

## Customizing

| Want to change… | Edit | Then |
|-----------------|------|------|
| Shell / aliases | `home/.config/zsh/{init,aliases}.zsh` | live immediately |
| Prompt | `home/.config/starship.toml` | live immediately |
| Terminal | `home/.config/ghostty/config` | live immediately |
| Git | `home/.gitconfig` (identity is in `~/.gitconfig.local`, not here) | live immediately |
| VS Code / Cursor | `home/Library/Application Support/{Code,Cursor}/User/settings.json` | live immediately |
| VS Code extensions | `Brewfile`: add `vscode "publisher.ext-id"` lines | `dots` (the brew step re-runs; 60-cursor.sh adds them to Cursor) |
| A command-line tool | `tools.list`: add one line | `dots` (step 12 installs it; `scripts/tools --check` lists what is missing) |
| VS Code/Cursor UI edits | edit in the app; it's a symlink, so the repo file changes too | `git status` in `~/dotfiles` to see it, then commit |
| bat / lazygit / bottom / yazi / tealdeer / fastfetch | `home/.config/<tool>/…` | live immediately |
| Claude | anything under `claude/` | live immediately (or `dots` for a new skill/file) |
| Claude settings (`claude/settings.json`) | the repo base for everyone; your own keys go in `~/.config/dotfiles/claude-settings.json` | `dots` (it builds `~/.claude/settings.json`) |
| Share a key Claude saved for you | `scripts/agent-config claude-settings promote <key>` (e.g. `permissions.allow`) | commit `claude/settings.json` |
| Hide a tool Claude never uses (it then costs no tokens) | add its bare name, or `mcp__<server>__*`, to `permissions.deny` in `claude/settings.json`. A connector has two names: its ID in the app, `claude_ai_<Name>` in the terminal | `dots`, then the next session (a deny beats every allow) |
| Cursor rules, agents, hooks, MCP | `claude/` (shared source) or `cursor/` | `dots` (regenerates `~/.cursor`) |
| Opencode rules, agents, permissions, theme | `claude/` (shared source); the palette lives in `rust/agent-config/themes/copper.json` | `dots` (regenerates `~/.config/opencode`) |
| Machine-local | `~/.config/zsh/local.zsh`, `~/.gitconfig.local`, `Brewfile.local` | new shell |
| Settings: git name, profile, Cursor MCP, Opencode MCP, Opencode projects, Java, steps to skip, private words and rules | `dots settings` (kept in `~/.config/dotfiles`, never in git; all names in `settings.example`) | at once (Java: new shell) |

## Updating

Fully manual, by design: no background process. Pull and reprovision on demand:

```sh
dots        # alias for ./dotfiles: git pull + relink + reprovision
```

The tools step and the brew step (macOS) run every time but no-op when nothing's missing; one-time install
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
from there. The tools (mise), Homebrew apps, Oh My Zsh, and toolchains stay installed.

For Cursor, delete the generated files and our merged entries:

1. `rm ~/.cursor/rules/claude-global.mdc ~/.cursor/rules/claude-output-style.mdc`
2. `rm $(grep -l 'generated-by: dotfiles' ~/.cursor/agents/*.md) ~/.cursor/.dotfiles-managed.json`
3. In `~/.cursor/hooks.json`, delete the 5 entries that run `guard_cursor` (`rust-bin --hook guard_cursor`).
4. In `~/.cursor/mcp.json`, delete `cloudflare-docs` (only there when MCP is on).

For Opencode:

1. `rm ~/.config/opencode/AGENTS.md $(grep -l 'generated-by: dotfiles' ~/.config/opencode/agents/*.md) ~/.config/opencode/.dotfiles-managed.json`
2. `rm ~/.config/opencode/themes/copper.json` and drop `"theme"` from `tui.json`.
3. In `~/.config/opencode/opencode.jsonc`, delete the `permission` keys that came from
   `claude/settings.json` and the `mcp` entries from `cursor/mcp.json`. Each change `dots`
   made there left a `opencode.jsonc.bak-<timestamp>` next to it.
4. In every project listed in `OPENCODE_PROJECTS`: `rm -r .opencode` (symlinks only).

## Troubleshooting

**`brew bundle` says "Permission denied".** Homebrew is owned by another user. Run
`~/dotfiles/scripts/fix-brew-permissions.sh`, then `dots`.

**`~/.claude` didn't get linked.** You ran `./dotfiles` inside a Claude Code session,
which can race Claude rewriting `~/.claude` live. Re-run `dots` from a plain shell.

**Linux: a step failed at the package manager or Homebrew.** Run `dots` again after fixing the line above it.
The Homebrew installer needs `sudo` and a user that is not root.

**Icons look like boxes.** The Nerd Font is wired into **both** terminals automatically:
Ghostty via its config, Apple Terminal via step `75-terminal-appearance`
(needs Automation permission on first run). Other terminals: set the font to "FiraCode
Nerd Font Mono".

**`compinit: insecure directories`.** Run `chmod -R go-w "$(brew --prefix)/share"` once.
