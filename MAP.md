<!-- ps:map -->
# Map

A plain-git dotfiles repo: shell, terminal and editor config and the Claude Code settings, set up by one script on
macOS, Linux and Windows. The AI harness (skills, agents, rules, guard) is the ps plugin in pavstev/agent-kit.

## Folders
| Path | Holds | Read it when |
|---|---|---|
| `home/` | managed dotfiles, linked into the home folder | changing shell, prompt, terminal or git config |
| `home/.config/` | app configs (zsh parts, terminal, prompt) | changing one app's config |
| `home/Library/` | macOS app settings (editor settings) | changing an editor setting on macOS |
| `claude/` | the base of `~/.claude/settings.json` and the preset settings fragments | changing Claude settings |
| `rust/agent-config/` | the Rust `dots`, `tools`, `port` and the settings builder | changing setup or a tool |
| `rust/agent-config/src/` | the bins and the `dots` install steps | changing code |
| `rust/agent-config/tests/` | unit and process tests with their fixtures | adding a test |
| `rust/agent-config/themes/` | the terminal theme data | changing the theme |
| `scripts/` | shell shims and test scripts | running a check |
| `scripts/git-hooks/` | commit-msg, pre-commit and pre-push hooks | changing what a commit checks |
| `docs/` | user docs per platform and tool | explaining a feature |
| `assets/` | images for the README | changing the README look |
| `.claude/` | this repo's own agents, agent map and commands | dispatching a helper here |

## Find it fast
- The entry script: `dotfiles` (a shim over the Rust `dots`).
- The install steps, in order: `rust/agent-config/src/dots/install/mod.rs`.
- The hand-over to the ps plugin: `rust/agent-config/src/dots/agent_kit.rs`.
- Claude settings: `claude/settings.json`.
- The list of command-line tools: `tools.list`.
- The OS check: `scripts/os-check.sh`.
- Every setting with its default: `settings.example`.

## Commands
`cargo test` in `rust/agent-config`; the rest is in `AGENTS.md`, section "Tests".

## Notes
