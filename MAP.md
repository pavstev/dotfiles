<!-- ps:map -->
# Map

A plain-git dotfiles repo: shell, terminal and editor config plus the Claude Code harness, set up by one script on
macOS, Linux and Windows.

## Folders
| Path | Holds | Read it when |
|---|---|---|
| `home/` | managed dotfiles, linked into the home folder | changing shell, prompt, terminal or git config |
| `home/.config/` | app configs (zsh parts, terminal, prompt) | changing one app's config |
| `home/Library/` | macOS app settings (editor settings) | changing an editor setting on macOS |
| `claude/` | Claude Code config, linked into the home Claude folder | changing rules, settings or the harness |
| `claude/agents/` | the shared agents for every repo | adding or changing a shared agent |
| `claude/hooks/` | the hook launcher and the guard docs | changing a hook or a guard check |
| `claude/skills/` | the `ps:*` skills, one folder each | adding or changing a skill |
| `claude/presets/` | opt-in rule sets | changing a preset |
| `claude/output-styles/` | output styles | changing how answers read |
| `rust/agent-config/` | the Rust `dots`, the guard and the helper bins | changing setup, a guard check or a tool |
| `rust/agent-config/src/` | the bins and the `dots` install steps | changing code |
| `rust/agent-config/tests/` | unit and process tests with their fixtures | adding a test |
| `rust/agent-config/themes/` | the terminal theme data | changing the theme |
| `scripts/` | shell shims and test scripts | running a check |
| `scripts/git-hooks/` | commit-msg, pre-commit and pre-push hooks | changing what a commit checks |
| `docs/` | user docs per platform and tool | explaining a feature |
| `cursor/` | the Cursor MCP config | changing Cursor MCP servers |
| `assets/` | images for the README | changing the README look |
| `.claude/` | this repo's own agents, agent map and commands | dispatching a helper here |

## Find it fast
- The entry script: `dotfiles` (a shim over the Rust `dots`).
- The install steps, in order: `rust/agent-config/src/dots/install/mod.rs`.
- The guard and its router: `rust/agent-config/src/bin/guard.rs`.
- Hook wiring and Claude settings: `claude/settings.json`.
- The list of command-line tools: `tools.list`.
- The OS check: `scripts/os-check.sh`.
- The global rules: `claude/CLAUDE.md`.
- Every setting with its default: `settings.example`.

## Commands
`cargo test` in `rust/agent-config`; the rest is in `AGENTS.md`, section "Tests".

## Notes
