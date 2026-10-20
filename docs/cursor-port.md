# Cursor port

One source, two harnesses. `claude/` stays the source of truth. Claude Code gets it by
symlink, exactly as before. Cursor (Cursor.app agent and the `agent` / `cursor-agent` CLI)
gets it three ways: native compat (Cursor reads some `~/.claude` files itself), generated
files under `~/.cursor`, and key-level JSON merges into files Cursor also writes.

Checked against Cursor.app 3.24.12 and CLI 2026.10.01 (2026-10-13). Sources: the Cursor
docs on [rules](https://cursor.com/docs/rules), [skills](https://cursor.com/docs/context/skills),
[subagents](https://cursor.com/docs/context/subagents), [hooks](https://cursor.com/docs/agent/hooks)
and [Claude hooks](https://cursor.com/docs/reference/third-party-hooks),
[MCP](https://cursor.com/docs/context/mcp),
[CLI config](https://cursor.com/docs/cli/reference/configuration) and
[permissions](https://cursor.com/docs/reference/permissions), plus read-only greps of the
shipped app and CLI bundles.

## How it runs

- `./dotfiles` picks a profile (`dots`, `settings::profile`), links `~/.claude`,
  links the skills, then runs the install steps. Step `60-cursor`
  installs the Cursor CLI if missing, installs the Brewfile editor extensions into Cursor, and
  runs `scripts/agent-config cursor`.
- `scripts/agent-config` (the Rust build in `rust/agent-config`) generates every Cursor file. It never writes
  under `~/.claude`.
- Generated Markdown files carry a `generated-by: dotfiles` marker. A file without the marker
  is the user's: it is left alone and a warning is printed. JSON files are merged key by key;
  the user's existing values win, except the commit attribution switches (see below). Each
  changed JSON file is backed up to `<file>.bak-<timestamp>` first.

## Profiles

| Profile | Who | Claude | Cursor global rule | Cursor MCP |
|---------|-----|--------|--------------------|------------|
| `owner` | a machine with private rules | `CLAUDE.md` link, plus `~/.claude/rules/owner.md` linked to `~/.config/dotfiles/owner.md` | `claude/CLAUDE.md` + `owner.md` | opt-in |
| `shared` | anyone else (default) | `CLAUDE.md` link | `claude/CLAUDE.md` | opt-in |

`owner.md` holds the private rules (`dots settings`). It never enters the repo. Order:
`DOTFILES_PROFILE` env, then `PROFILE` in `~/.config/dotfiles/settings`, then `owner` when `owner.md`
exists (saved as `PROFILE` in the settings), else `shared`.

## Mapping

| Claude item | Cursor item | Method |
|-------------|-------------|--------|
| `~/.claude/CLAUDE.md` | `~/.cursor/rules/claude-global.mdc` (`alwaysApply: true`) | generate (adds frontmatter + a short tool-name note, which also lists the 7 `subagent_type` values Cursor accepts and tells the agent to run a named agent as `generalPurpose` with "Read ~/.claude/agents/<name>.md first and follow it"; the owner profile adds `owner.md` and writes the file 0600) |
| `outputStyle` + `output-styles/<name>.md` | `~/.cursor/rules/claude-output-style.mdc` (`alwaysApply: true`) | generate from the active style |
| project `CLAUDE.md` / `AGENTS.md` | same files | native (Cursor reads both) |
| `skills/ps_*` | `~/.claude/skills/ps_*` | native compat; `dots` no longer links them into `~/.cursor/skills` (two roots can list them twice) |
| `agents/*.md` | `~/.cursor/agents/*.md` | generate (`model: inherit`, `readonly: true` when tools are read-only); wins over the native `~/.claude/agents` import |
| `guard` (G3 context ceiling) | `~/.cursor/hooks.json` -> `sh <repo>/claude/hooks/rust-bin --hook guard_cursor <mode>` on `subagentStart` (IDE) and `preToolUse` for `Task` (the CLI fires no `subagentStart`) | merge entries; the `guard_cursor` bin keeps the `guard` limits |
| `guard` G14 to G17 (sed, secrets, outbound, attribution) | `~/.cursor/hooks.json` -> `... --hook guard_cursor tool` on `preToolUse` (matcher `^(Shell\|Read\|Grep)$`: G14, G15, G17 deny), `beforeShellExecution` and `beforeMCPExecution` (G16 asks), `beforeReadFile` (G15 on the documented `file_path`) | merge entries; the `tool` mode runs `guard_checks`, the same library as the Claude guard, and uses the same gate (preset on, not in `GUARD_OFF`, `GUARD_SHADOW` logs and allows). Cursor enforces `ask` only on the two `before*Execution` hooks, so G16 sits there. Hook docs: cursor.com/docs/hooks. Not yet run in a live Cursor session |
| `permissions.allow/deny` | `~/.cursor/cli-config.json` `permissions` | merge (translated: `Bash(x:*)` -> `Shell(x)`, exact `Bash(x y)` -> `Shell(x:y)`, deny `Read(./p)` -> `Read(**/p)`) |
| `includeCoAuthoredBy: false`, `attribution` | `cli-config.json` `attribution.attributeCommitsToAgent/PRsToAgent = false` | merge (forced false) |
| `statusLine` -> `rust-bin statusline` | `cli-config.json` `statusLine` (CLI only) | merge, same command |
| `enabledPlugins` (cloudflare) | Cursor imports Claude plugins | native compat |
| MCP | `~/.cursor/mcp.json` from `cursor/mcp.json` (`cloudflare-docs`) | merge; on every profile only with `CURSOR_MCP="on"` (`dots settings`) or `DOTFILES_MCP=on` |
| editor settings, keybindings | `~/Library/Application Support/Cursor/User/*` (macOS), `~/.config/Cursor/User/*` (Linux), `%APPDATA%\Cursor\User\*` (Windows) | link (unchanged) |
| Brewfile `vscode` extensions | Cursor extensions | `cursor --install-extension` loop |

## Gaps

| Claude feature | Why Cursor cannot match it |
|----------------|----------------------------|
| Output style switching (`/output-style`, `outputStyle`) | No output-style feature. Old Custom Modes were removed in 2.1. The active style is shipped as an always-on rule; switching means editing `outputStyle` and running `dots`. |
| Statusline in the IDE | Only the CLI has `statusLine`. Its payload has no `rate_limits`, so the 5h/7d part stays empty. |
| Guard G1 (subagent needs model + effort) | Cursor subagents have no effort field, and many plans ignore the model pin. |
| Guard G2 (no polling at big context) | Cursor has no cron, wakeup or loop tools. |
| Guard G4 (no new question card while an earlier one is unanswered) | Cursor's AskQuestion tool fires no hooks (open bug). |
| Guards other than G3 and G14 to G17 | Cursor has hooks, but no mirror of the other checks is built. The table of which check runs where is in [`docs/ai-agents.md`](ai-agents.md#where-each-guard-check-runs). |
| Guard G3 exact context size | Cursor hooks carry no context size. In Cursor.app the adapter reads the app's own count (`contextTokensUsed` in `globalStorage/state.vscdb`, read-only, about 1 ms), so the ceiling is exact there (replayed with real app payloads, 2026-10-13). Hook `input_tokens` are per-turn totals (138,002 for a 32,539-token chat), so they stay ignored. The CLI store is encrypted and `-p` mode skips `beforeSubmitPrompt` and `subagentStart`, so the CLI falls back to an explicit `context_tokens` or a transcript, and may stay silent. Models with a 256K window never reach the 300K/350K limits. A live app probe (2026-10-13) showed the app does not display a `beforeSubmitPrompt` `user_message`, but does pass `additional_context` to the agent, so the nudge asks the agent to tell the user. |
| IDE permission allowlist | `~/.cursor/permissions.json` replaces the in-app list and can disable Auto-review, so it is not shipped. The IDE has no read deny; `cursor.general.globalCursorIgnoreList` hides `.env` and `secrets/` from AI features only. |
| Rules in every folder | The file rule in `~/.cursor/rules` only loads for folders under the home folder (tested: yes in `~/x`, no in `/tmp/x`). Cursor's User Rules apply everywhere, but they live on the Cursor account, so no file can fill them. `cursor-rules` copies the text to the clipboard for a one-time paste and stops the file copy (no duplicates); `dots` warns when the text changes and must be pasted again. |
| `@AGENTS.md` imports inside CLAUDE.md | Cursor does not inline `@file` lines. It loads `AGENTS.md` on its own anyway. |
| claude.ai connectors (Linear, Sentry, Gmail, ...) | Tied to the claude.ai account. Not portable. |
| Skill names with `:` | The agentskills.io spec allows only `a-z0-9-`. Cursor's loaders do not check, so `ps:*` load; the CLI uses the folder name as the slash command. Opencode loads them too (`docs/opencode-port.md`). |
| Settings Sync | Cursor has no VS Code Settings Sync. The repo links are the sync. |
| Pylance | Not on Open VSX. Cursor ships its own Python language server. |
| Custom agents (`~/.cursor/agents`) | Neither the CLI nor the app (3.24.12) offers them to the Task tool. The app watches the folder and caches 2 agents, but a live app chat (2026-10-13) got `Invalid enum value. Expected 'generalPurpose' \| 'explore' \| 'cursor-guide' \| 'ci-investigator' \| 'bugbot' \| 'security-review' \| 'best-of-n-runner', received 'code-reviewer'`. The model then ran `generalPurpose` with the agent's prompt. So the generated rule now says that up front (see the first mapping row), and the stand-in reads `~/.claude/agents/<name>.md`. The generated `~/.cursor/agents` files stay: they do no harm and a project can copy them. Agents in the open project's own `.cursor/agents` do work: a live CLI run with `code-reviewer.md` there got `subagent_type: code-reviewer` accepted (2026-10-13). So copy an agent into a project's `.cursor/agents` to use it there. |
| Read deny vs the shell | Deny rules stop the Read tool (`Read(**/.env)` denied live), but `cat .env` through the shell still works. Claude Code has the same gap. |
| Agent frontmatter hooks | `scripts/agent-config` copies only `name`, `description` and `readonly` to `~/.cursor/agents`, so the ui-verifier's `guard scratch` write hook does not reach Cursor. Guard G5 (hand-back cap) has no Cursor mirror either: Cursor showed no over-cap hand-back yet (2026-10-14). |
| Cursor login | Cursor has no login file to ship. The first guided run offers `agent login` and opens Cursor.app to sign in there. |

## Decisions

- Skills are not linked into `~/.cursor/skills`: Cursor reads `~/.claude/skills`, and two
  roots listed them twice in older builds. The `ps:` colon names stay.
- The CLI comes from the official installer, not the `cursor-cli` cask (no auto-update).
  Never probe it with `cursor agent ...`: the launcher runs the installer when the CLI is
  missing.
- `cli-config.json` and `mcp.json` are merged, never linked: Cursor rewrites them by rename.
  The commit and PR attribution switches are forced off.
- Generated agents get `model: inherit` (`sonnet` is not a Cursor model ID) and
  `readonly: true` when their tools are read-only.
- Only guard G3 is ported. Hook commands start with `test -r <script> || exit 0`: Cursor
  runs them with `/bin/sh -c`, and exit 2 means deny. The CLI fires no `subagentStart`, so
  G3 also gates `preToolUse` for the subagent tool. The app does not show a hook
  `user_message`, so the nudge goes through `additional_context`.
- Exact `Bash(x y)` rules map to `Shell(x:y)`, prefix rules to `Shell(x)`. The sidecar
  `~/.cursor/.dotfiles-managed.json` lets a later run drop rules removed from `claude/`.
- `DOTFILES_STEPS=60-cursor` runs only the Cursor step, for temp-HOME tests.
- A live CLI pass (2026-10-13) confirmed the rules, skills, MCP, a built-in subagent,
  the `.env` read deny, the G3 block at a seeded 600K context, and a project-level agent.
