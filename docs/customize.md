# Customize

How to change things, set the looks, and uninstall.

← [Docs](README.md)

## Customizing

| Want to change… | Edit | Then |
|-----------------|------|------|
| Shell / aliases | `home/.config/zsh/{init,aliases}.zsh` | live immediately |
| Prompt | `home/.config/starship.toml` (only while Looks has `starship`) | live immediately |
| Terminal | `home/.config/ghostty/config` | live immediately |
| Git | `home/.gitconfig` (identity is in `~/.gitconfig.local`, not here) | live immediately |
| VS Code / Cursor | `home/Library/Application Support/{Code,Cursor}/User/settings.json` (only while Looks has `editor`) | live immediately |
| VS Code extensions | `Brewfile`: add `vscode "publisher.ext-id"` lines | `dots` (step `20-brew-bundle` installs them; step `60-cursor` adds them to Cursor) |
| A command-line tool | `tools.list`: add one line. It names the tool, its commands, its OSes and its sources in order (mise first, then `apk`, `winget`, `scoop`, `cargo`, `brew`) | `dots` (step `12-tools` installs it; `scripts/tools --check` lists what is missing) |
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

To turn a look off, change the **Looks** row in `dots settings`. To drop Starship itself, run
`mise uninstall starship`.

## Where things are

The folder map is in [AGENTS.md](../AGENTS.md#layout). The short version: `home/` mirrors
`$HOME`, `claude/` is the Claude config (Cursor and Opencode read it too), `tools.list` lists the
tools, and `rust/agent-config/` holds the Rust programs behind `dots`.

## Uninstall

```sh
dots uninstall
```

It asks once (`-y` skips the question). Then it:

- removes every link into the repo and puts back the file each one replaced
  (`*.bak-<timestamp>`);
- deletes the generated Cursor and Opencode files (the ones with a `generated-by: dotfiles`
  marker);
- takes out the hooks, MCP servers, permissions and theme that `dots` merged into your own
  `~/.cursor` and `~/.config/opencode` files. Your own entries stay.

It leaves these, on purpose: the repo, the tools (mise, Homebrew apps, Oh My Zsh, toolchains),
your settings in `~/.config/dotfiles`, and `~/.claude/settings.json` (a real file that holds your
own keys). Delete them by hand if you want them gone. Running it twice is safe.
