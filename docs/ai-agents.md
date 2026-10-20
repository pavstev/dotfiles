# AI agents

How Claude Code, Cursor and Opencode share one source, and what each one gets.

← [Docs](README.md)

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
`statusline` Rust bin, run as `sh ~/.claude/hooks/rust-bin statusline`.

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

### Presets

The core rules in `claude/CLAUDE.md` are short and neutral. The habits that some people want and
others do not are **presets**: one folder each under `claude/presets/` (see its
[README](../claude/presets/README.md)). A preset that is off costs no tokens, and its guard check
does not run.

| Preset | What it does |
|--------|--------------|
| `workflow` | speed over ceremony, hand-offs in files, briefs, the agent set |
| `pace` | make the weekly budget last the work days, with helper caps |
| `models` | effort and model picks (also sets `model` and the helper model) |
| `clock` | say the local time and an estimate with a range |
| `plain-writing` | no em dashes, short plain sentences (sets the `ELI5` output style) |
| `no-ai-attribution` | no assistant co-author or "generated with" line in commits and PRs |
| `ship-to-main` | work lands on main in one commit; the "archive this" routine |

A new install has every preset on except `ship-to-main`. Pick others with `dots settings`
(`PRESETS`: `all`, `none`, or names split by spaces). `dots` then links the rules to
`~/.claude/rules/preset-<name>.md`, merges the settings fragments, and writes the same rules into the
Cursor and Opencode files.

Other settings that shape the setup (all in `settings.example`):

| Setting | Meaning |
|---------|---------|
| `PRESETS` | which presets are on |
| `TIMEZONE` | your time zone as an IANA name (empty: the system zone) |
| `PLAN` | your Claude plan, for the weekly pace budget |
| `WORK_DAYS` | the days you work, for the weekly pace |
| `THEME` | `copper` (default) or `plain` (no colors, no Opencode theme) |
| `GUARD_OFF`, `GUARD_SHADOW` | guard check ids to turn off, or to log without blocking |

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
`opencode` CLI (installed from `tools.list`), and `agent-config opencode`, which is wired into
`./dotfiles` itself and not into a step so that `dots settings` picks up a changed setting,
generates what Opencode needs:

1. **Native:** skills from `~/.claude/skills`. Every `ps:*` skill works in Opencode,
   in every project, because Opencode reads `~/.claude/skills` itself. Project
   `AGENTS.md` and `CLAUDE.md` load there too.
2. **Generated:** `~/.config/opencode/AGENTS.md` (the global rules from `CLAUDE.md`, plus
   the owner's private rules and the active output style) and `~/.config/opencode/agents/*.md`
   (from `claude/agents`), each a `mode: subagent` agent with `permission.skill: allow`
   (read-only agents also get `permission.edit: deny`). These carry a `generated-by: dotfiles`
   marker; a file without it is yours and is left alone. The copper theme goes to
   `~/.config/opencode/themes/copper.json` and `tui.json` names it. A `tui.json` that already
   names another theme is yours, and only gets reported.
3. **Merged key by key:** `~/.config/opencode/opencode.jsonc`: `permission` (translated
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
| G14 bare `sed -i` on macOS (a suffix such as `-i ''` or `-i.bak` passes) | deny | deny (`preToolUse`, shell) | deny (plugin) |
| G15 reading `local.zsh`, printing command lines | deny | deny (`preToolUse`, shell, read, grep) | deny (plugin) |
| G16 text with a private word or a secret leaving the machine | ask | ask (`beforeShellExecution`, `beforeMCPExecution`) | blocked, no ask |
| G17 assistant attribution in a commit, PR or issue (preset `no-ai-attribution`) | deny | deny (`preToolUse`, shell) | deny (plugin) |
| G3 context ceiling | yes | yes (`guard_cursor`, other modes) | no |
| G1, G2, G4 to G13, pace, router | yes | no (see the gaps in `docs/cursor-port.md`) | not tested |

The Opencode plugin runs the whole `guard pretool` hook, so a Claude check that looks at a shell
command (for example G2) can also fire there. Only G14 to G17 are tested in Opencode.
