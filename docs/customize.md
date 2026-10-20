# Customize

How to change things, the optional looks, where files live, and how to remove it all.

← [Docs](README.md)

## Customizing

| Want to change… | Edit | Then |
|-----------------|------|------|
| Shell / aliases | `home/.config/zsh/{init,aliases}.zsh` | live immediately |
| Prompt | `home/.config/starship.toml` (only while Looks has `starship`) | live immediately |
| Terminal | `home/.config/ghostty/config` | live immediately |
| Git | `home/.gitconfig` (identity is in `~/.gitconfig.local`, not here) | live immediately |
| VS Code / Cursor | `home/Library/Application Support/{Code,Cursor}/User/settings.json` (only while Looks has `editor`) | live immediately |
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
| Which habits are on (presets), time zone, plan, work days, theme, looks | `dots settings` (`PRESETS`, `TIMEZONE`, `PLAN`, `WORK_DAYS`, `THEME`, `LOOKS`) | `dots` refreshes the links and files; `dots doctor` checks them |
| A new preset | `claude/presets/<name>/rules.md` (steps in its README) | `cargo test --test presets` in `rust/agent-config`, then `dots` |
| Settings: git name, profile, Cursor MCP, Opencode MCP, Opencode projects, Java, steps to skip, private words and rules | `dots settings` (kept in `~/.config/dotfiles`, never in git; all names in `settings.example`) | at once (Java: new shell) |

## Looks: Starship, editor settings and the Nerd Font

The look changes are optional and off until you pick them. `dots` asks once, at a terminal,
on a new machine and on the first run after an update. Change it any time with `dots settings`
and the **Looks** row (the `LOOKS` setting):

| Part | What it changes | OS |
|------|-----------------|----|
| `starship` | the Starship prompt (colors, icons). Off: a plain prompt with the folder name | macOS, Linux |
| `editor` | the VS Code and Cursor settings and keys (theme, Nerd Font, ligatures) | all |
| `font` | installs the FiraCode Nerd Font and sets the Apple Terminal font | macOS |

When you turn a part off, `dots` removes its links and puts back the file it replaced (the
newest `*.bak-<timestamp>`). Open a new terminal or restart the editor to see it.

**Remove them by hand** (no `dots` needed). These are links into the repo, so deleting
them is safe:

```sh
# macOS and Linux: the prompt
rm ~/.config/starship.toml
# macOS: editor settings
rm ~/Library/Application\ Support/{Code,Cursor}/User/{settings,keybindings}.json
# Linux: editor settings
rm ~/.config/{Code,Cursor}/User/{settings,keybindings}.json
# Windows (Git Bash): editor settings
rm "$APPDATA"/{Code,Cursor}/User/{settings,keybindings}.json
```

Then set `LOOKS="none"` with `dots settings`, so the next `dots` run does not link them again.
Without `~/.config/starship.toml` the shell uses the plain prompt even when Starship is
installed. Remove Starship itself with `mise uninstall starship` (or your package manager).

## Repo layout

```
dotfiles/
├── assets/logo.svg           # the logo
├── AGENTS.md  CLAUDE.md      # how AI agents (Claude/Cursor/Opencode) operate this repo
├── bootstrap.sh              # fresh-machine one-liner (macOS, Linux, Windows)
├── dotfiles                  # the one command: pull + link + provision
├── tools.list                # every command-line tool: name, commands, OSes, sources (the one list)
├── Brewfile                  # the macOS apps, fonts + VS Code extensions (Brewfile.local for extras)
├── claude/                   # Claude config, symlinked into ~/.claude
│   ├── settings.json  CLAUDE.md
│   └── skills/ agents/ hooks/ output-styles/ presets/
├── cursor/                   # Cursor-only sources, merged into ~/.cursor
│   └── mcp.json              # MCP servers (cloudflare-docs); your entries win
├── docs/README.md            # the index of the docs
├── docs/install.md           # set up each OS, check it, multi-machine, updating
├── docs/features.md          # why, what is inside, daily workflow, cheat sheet, secrets
├── docs/ai-agents.md         # Claude Code, Cursor, Opencode, presets, guard checks
├── docs/platforms.md         # Linux and Windows details, and how to test them
├── docs/customize.md         # this page
├── docs/troubleshooting.md   # fixes
├── docs/guide.md             # stub: maps the old guide sections to the new pages
├── docs/FRIENDS-SETUP.md     # the runbook Claude follows to set this repo up on a friend's machine
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
│   ├── setup-lib.sh          # shell helpers for the few bash scripts (step/info/ok/warn/have, is_*, setting, ensure_rust_bin)
│   ├── tools                 # shim: builds and runs the Rust `tools` bin (installs the tools of tools.list)
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
