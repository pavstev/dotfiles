# AI agents

How Claude Code, Cursor and Opencode share one source, and what each one gets.

← [Docs](README.md)

`claude/` is the source of truth. Edit it, then run `dots`. Claude Code gets it by symlink.
Cursor and Opencode get it three ways: they read some `~/.claude` files themselves, `dots`
generates files for them, and `dots` merges a few entries into config files they also write.

Rules for generated files: a file with a `generated-by: dotfiles` marker is ours and is
rewritten. A file without it is yours and is never touched. JSON files are merged key by key,
your values win, and each change leaves a `<file>.bak-<timestamp>` copy. `dots uninstall` takes
all of it back ([Customize](customize.md#uninstall)).

## Claude Code

`dots` links each piece into `~/.claude` one by one, never the whole folder, because Claude
writes its session state there:

```
claude/
├── settings.json   built into ~/.claude/settings.json (see below)
├── CLAUDE.md       → ~/.claude/CLAUDE.md          global rules
├── skills/         → ~/.claude/skills (per skill; Cursor and Opencode read it too)
├── agents/         → ~/.claude/agents
├── hooks/          → ~/.claude/hooks
└── output-styles/  → ~/.claude/output-styles
```

**settings.json is built, not linked**, because Claude writes it too. `dots` merges
`claude/settings.json` (the shared base) with your private layer
`~/.config/dotfiles/claude-settings.json` (mode 600, never in git). Objects merge, your values
win, and the `permissions` lists add up. A key set to `null` in the layer removes it.

Before each build, `dots` compares the live file with what it wrote last time. A key that
Claude or you changed moves into the private layer, and `dots` prints its name (never its
value). To share a kept key with everyone, run
`scripts/agent-config claude-settings promote <key>` and commit.

**Skills** are the headline. `claude/skills` holds the `ps:*` workflow skills (sources:
[`SOURCES.md`](../claude/skills/SOURCES.md); start your own from
[`_template`](../claude/skills/_template/SKILL.md)). `dots` links each one into
`~/.claude/skills` without touching your own skills (on a name clash, yours wins). Cursor and
Opencode read that folder themselves, so nothing is linked into `~/.cursor` or
`~/.config/opencode`.

### Presets

The core rules in `claude/CLAUDE.md` are short and neutral. Habits that some people want and
others do not are **presets**: one folder each under `claude/presets/`
([README](../claude/presets/README.md)). A preset that is off costs no tokens, and its guard
check does not run.

| Preset | What it does |
|--------|--------------|
| `workflow` | speed over ceremony, hand-offs in files, briefs, the agent set |
| `pace` | make the weekly budget last the work days, with helper caps |
| `models` | effort and model picks |
| `clock` | say the local time and an estimate with a range |
| `plain-writing` | no em dashes, short plain sentences (sets the `ELI5` output style) |
| `no-ai-attribution` | no assistant co-author or "generated with" line in commits and PRs |
| `ship-to-main` | work lands on main in one commit; the "archive this" routine |

A new install has every preset on except `ship-to-main`. Change them in `dots settings`. `dots`
links the rules to `~/.claude/rules/preset-<name>.md`, merges the settings fragments, and
writes the same rules into the Cursor and Opencode files. Other settings that shape the setup
are `TIMEZONE`, `PLAN`, `WORK_DAYS`, `THEME` and `GUARD_OFF` / `GUARD_SHADOW`
(all in `settings.example`).

## Cursor

Step `60-cursor` runs when Cursor is present. It installs the Cursor CLI if it is missing,
installs the Brewfile `vscode` extensions into Cursor, and runs `scripts/agent-config cursor`.

- **Linked:** `settings.json` and `keybindings.json` (only with the `editor` look).
- **Native:** skills from `~/.claude/skills`, and project `CLAUDE.md` and `AGENTS.md`.
- **Generated:** `~/.cursor/rules/claude-global.mdc` (the rules), `claude-output-style.mdc` (the
  active output style) and `~/.cursor/agents/*.md`.
- **Merged:** `~/.cursor/hooks.json` (the guard), `~/.cursor/mcp.json` (from
  [`cursor/mcp.json`](../cursor/mcp.json); opt-in with Cursor MCP in `dots settings`) and
  `~/.cursor/cli-config.json` (permissions, attribution, status line). Commit attribution to
  the agent is always off.
- **Cloned:** [superlite](https://github.com/iha2/superlite) into `~/.cursor/plugins/local/superlite`
  when Claude Code has `superlite@superlite-dev` on (the built `~/.claude/settings.json`). Each run
  pulls it; off removes our clone. A real folder, because Cursor skips a link to a folder outside
  `plugins/local`.

The full mapping and the gaps: [Cursor port](cursor-port.md).

## Opencode

`opencode` comes from `tools.list` (step `12-tools`). `agent-config opencode` runs inside
`./dotfiles` itself, not as a step, so `dots settings` picks up a changed setting.

- **Native:** skills from `~/.claude/skills`, and project `AGENTS.md` and `CLAUDE.md`.
- **Generated:** `~/.config/opencode/AGENTS.md` (the rules, the owner's private rules and the
  output style), `agents/*.md` (subagents), `themes/copper.json`, and the guard plugin
  `plugins/dots-guard.ts`.
- **Merged:** `opencode.jsonc` (`permission` from `claude/settings.json`; `mcp` from
  `cursor/mcp.json`, opt-in with Opencode MCP in `dots settings`) and `tui.json` (the theme
  name, unless it already names another theme). The superlite plugin spec goes into `plugins`
  (`plugin` when the file uses that V1 key) while Claude Code has `superlite@superlite-dev` on, and
  comes out when it is off.
- **Per project:** each path in `OPENCODE_PROJECTS` gets a `.opencode/` that links the global
  config and agents. What the project already has wins.

Opencode reads `AGENTS.md` as its global rules and falls back to `~/.claude/CLAUDE.md` only
when it is missing. That is why `AGENTS.md` is generated: `CLAUDE.md` alone carries neither the
owner rules nor the output style. The full mapping and the gaps: [Opencode port](opencode-port.md).

## Where each guard check runs

One Rust library (`guard_checks`) holds the shared checks, so the three apps run the same code.
A check also needs its preset on, and `GUARD_OFF` turns one off in every app.

| Check | Claude Code | Cursor | Opencode |
|-------|-------------|--------|----------|
| G14 bare `sed -i` on macOS (a suffix such as `-i ''` or `-i.bak` passes) | deny | deny (`preToolUse`, shell) | deny (plugin) |
| G15 reading `local.zsh`, printing command lines | deny | deny (`preToolUse`: shell, read, grep) | deny (plugin) |
| G16 text with a private word or a secret leaving the machine | ask | ask (`beforeShellExecution`, `beforeMCPExecution`) | blocked, no ask |
| G17 assistant attribution in a commit, PR or issue (preset `no-ai-attribution`) | deny | deny (`preToolUse`, shell) | deny (plugin) |
| G3 context ceiling | yes | yes (`guard_cursor`, other modes) | no |
| H1 hint to use `rust-bin ctx` when Bash runs `sed -n` or `cat` on a code file, not a text or data file such as md, json or yaml (preset `workflow`; never a deny; at most 3 per session) | yes | no | not tested |
| W5 note "skip it" for a check command that already passed with the work tree unchanged (preset `workflow`; never a deny) | yes | no | not tested |
| Format on edit (`rust-bin --hook proj format`; not a guard check) | yes | no (Cursor has its own `afterFileEdit` hook) | no |
| G1, G2, G4 to G13, G18, G19, G20, pace, router | yes | no | not tested |

The Opencode plugin runs the whole `guard pretool` hook, so a Claude check that looks at a shell
command can also fire there. Only G14 to G17 are tested in Opencode.
