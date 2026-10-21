# Cursor port

How the Claude setup maps to Cursor (the app and the `agent` / `cursor-agent` CLI), and what
Cursor cannot do. The overview is in [AI agents](ai-agents.md#cursor).

← [Docs](README.md)

Checked against Cursor.app 3.24.12 and CLI 2026.10.01 (2026-10-13). Sources: the Cursor docs on
[rules](https://cursor.com/docs/rules), [skills](https://cursor.com/docs/context/skills),
[subagents](https://cursor.com/docs/context/subagents), [hooks](https://cursor.com/docs/agent/hooks),
[MCP](https://cursor.com/docs/context/mcp), [CLI config](https://cursor.com/docs/cli/reference/configuration)
and [permissions](https://cursor.com/docs/reference/permissions), plus read-only greps of the
shipped app and CLI.

`scripts/agent-config` (the Rust build in `rust/agent-config`) writes every Cursor file and never
writes under `~/.claude`. Generated Markdown carries a `generated-by: dotfiles` marker; a file
without it is yours and is left alone with a warning. JSON files are merged key by key and your
values win (except the commit attribution switches, which are forced off).

## Mapping

| Claude item | Cursor item | Method |
|-------------|-------------|--------|
| `~/.claude/CLAUDE.md` | `~/.cursor/rules/claude-global.mdc` (`alwaysApply: true`) | generate: frontmatter, a tool-name note and the rules. The note lists the `subagent_type` values Cursor accepts and tells the agent to run a named agent as `generalPurpose` after reading `~/.claude/agents/<name>.md`. The owner profile adds `owner.md` and writes the file 0600. |
| `outputStyle` + `output-styles/<name>.md` | `~/.cursor/rules/claude-output-style.mdc` (`alwaysApply: true`) | generate from the active style |
| project `CLAUDE.md` / `AGENTS.md` | same files | native (Cursor reads both) |
| `skills/ps_*` | `~/.claude/skills/ps_*` | native; `dots` does not link them into `~/.cursor/skills` (two roots can list them twice) |
| `agents/*.md` | `~/.cursor/agents/*.md` | generate (`model: inherit`, `readonly: true` when the tools are read-only) |
| guard G3 (context ceiling) | `~/.cursor/hooks.json`: `rust-bin --hook guard_cursor <mode>` on `subagentStart` and on `preToolUse` for `Task` (the CLI fires no `subagentStart`) | merge entries |
| guard G14 to G17 | `hooks.json`: `guard_cursor tool` on `preToolUse` (matcher `^(Shell\|Read\|Grep)$`: G14, G15, G17 deny), `beforeShellExecution` and `beforeMCPExecution` (G16 asks), `beforeReadFile` (G15) | merge entries. The `tool` mode runs the same `guard_checks` library as the Claude guard, with the same gate (preset on, not in `GUARD_OFF`, `GUARD_SHADOW` logs and allows). Cursor enforces `ask` only on the two `before*Execution` hooks, so G16 sits there. Not yet run in a live Cursor session. |
| `permissions.allow/deny` | `~/.cursor/cli-config.json` `permissions` | merge, translated: `Bash(x:*)` to `Shell(x)`, exact `Bash(x y)` to `Shell(x:y)`, deny `Read(./p)` to `Read(**/p)` |
| `attribution` | `cli-config.json` `attribution.*ToAgent = false` | merge (forced) |
| `statusLine` | `cli-config.json` `statusLine` (CLI only) | merge, same command |
| `enabledPlugins` | Cursor imports Claude plugins | native |
| MCP | `~/.cursor/mcp.json` from `cursor/mcp.json` | merge; only with `CURSOR_MCP="on"` or `DOTFILES_MCP=on` |
| editor settings, keybindings | `Cursor/User/*` (macOS `~/Library/Application Support`, Linux `~/.config`, Windows `%APPDATA%`) | link |
| Brewfile `vscode` extensions | Cursor extensions | `cursor --install-extension` loop |

A sidecar file, `~/.cursor/.dotfiles-managed.json`, records the permission rules `dots` added, so
a later run drops the ones removed from `claude/`.

## Gaps

| Claude feature | Why Cursor cannot match it |
|----------------|----------------------------|
| Output style switching | No such feature. The active style is an always-on rule; switching means editing `outputStyle` and running `dots`. |
| Status line in the IDE | Only the CLI has `statusLine`, and its payload has no `rate_limits`. |
| Guards G1, G2, G4 | Cursor subagents have no effort field, there is no cron or loop tool, and the AskQuestion tool fires no hooks. |
| Other guards (except G3, G14 to G17) | No mirror is built. Which check runs where: [AI agents](ai-agents.md#where-each-guard-check-runs). |
| Guard G3 exact context size | Hooks carry no context size. In the app the adapter reads the app's own count (`contextTokensUsed` in `globalStorage/state.vscdb`, read-only). The CLI store is encrypted, so the CLI uses an explicit `context_tokens` or a transcript and may stay silent. The app does not show a `beforeSubmitPrompt` `user_message` but passes `additional_context`, so the nudge asks the agent to tell the user. |
| IDE permission allowlist | `~/.cursor/permissions.json` replaces the in-app list, so it is not shipped. `cursor.general.globalCursorIgnoreList` hides `.env` and `secrets/` from AI features only. |
| Rules in every folder | The file rule loads only for folders under the home folder. Cursor's User Rules apply everywhere but live on the account, so no file can fill them. Run `cursor-rules` to copy the text for a one-time paste; `dots` then stops the file copy and warns when you must paste again. |
| `@AGENTS.md` imports in CLAUDE.md | Cursor does not inline `@file` lines. It loads `AGENTS.md` on its own. |
| claude.ai connectors | Tied to the claude.ai account. |
| Skill names with `:` | The agentskills.io spec allows only `a-z0-9-`, but Cursor's loaders do not check, so `ps:*` load. |
| Settings Sync, Pylance | No VS Code Settings Sync (the repo links are the sync). Pylance is not on Open VSX. |
| Custom agents in `~/.cursor/agents` | Neither the CLI nor the app offers them to the Task tool (a live chat got `Invalid enum value ... received 'code-reviewer'`). The generated rule tells the agent to run `generalPurpose` after reading the agent file. An agent copied into a project's own `.cursor/agents` does work. |
| Read deny vs the shell | A deny rule stops the Read tool, but `cat .env` through the shell still works. Claude Code has the same gap. |
| Agent frontmatter hooks | Only `name`, `description` and `readonly` are copied, so the ui-verifier's write hook and guard G5 do not reach Cursor. |
| Login | There is no login file to ship. The first guided run offers `agent login`. |

## Decisions

- The CLI comes from the official installer, not the `cursor-cli` cask (no auto-update). Never
  probe it with `cursor agent ...`: the launcher runs the installer when the CLI is missing.
- `cli-config.json` and `mcp.json` are merged, never linked: Cursor rewrites them by rename.
- Hook commands start with `test -r <script> || exit 0`. Cursor runs them with `/bin/sh -c`, and
  exit 2 means deny.
- `DOTFILES_STEPS=60-cursor` runs only the Cursor step, for temp-HOME tests.
