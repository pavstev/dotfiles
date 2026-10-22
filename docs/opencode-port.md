# Opencode port

How the Claude setup maps to Opencode (the `opencode` CLI and the app), and what Opencode cannot
do. The overview is in [AI agents](ai-agents.md#opencode).

← [Docs](README.md)

Checked against Opencode v2.0.25 (the plugin against 2.0.30). Sources: the Opencode docs on
[config](https://opencode.ai/docs/config), [agents](https://opencode.ai/docs/agents),
[rules](https://opencode.ai/docs/rules), [themes](https://opencode.ai/docs/themes),
[skills](https://opencode.ai/docs/skills) and the schemas at `https://opencode.ai/config.json`
and `https://opencode.ai/tui.json`.

`scripts/agent-config` (the Rust build in `rust/agent-config`) writes every Opencode file and never
writes under `~/.claude`. When it runs and the marker and merge rules: [AI agents](ai-agents.md).
On Windows `opencode` comes through mise or `winget install SST.opencode`. Step `90-sign-in`
offers `opencode auth login`.

## Mapping

| Claude item | Opencode item | Method |
|-------------|---------------|--------|
| `skills/ps_*` | `~/.claude/skills/ps_*` | native; `dots` links per skill. The colon in `ps:*` is outside the agentskills.io spec and Opencode loads it anyway (`skill: allow` is merged into `opencode.jsonc`). |
| `CLAUDE.md` | `~/.config/opencode/AGENTS.md` | generate: marker, a tool-name note, the rules, the owner overlay (mode 0600) and the active output style as `# Reply style: <name>` |
| `~/.claude/CLAUDE.md` | same file, as fallback | native; `AGENTS.md` wins when both exist |
| project `CLAUDE.md` / `AGENTS.md` | same files | native (`AGENTS.md` first) |
| `agents/*.md` | `~/.config/opencode/agents/<name>.md` | generate: `mode: subagent`, `permission.skill: allow`, `permission.edit: deny` for agents with no write tools. `model:`, `effort:` and `maxTurns:` are dropped: Opencode names models `provider/model-id`, so agents inherit the session model. |
| `permissions.allow/deny` | `opencode.jsonc` `permission.bash/read/edit/tool` | merge, translated: `Bash(x:*)` to `bash "x *"`, `Read` to `read`, `Edit`/`Write` to `edit`, `Suggest*` to `tool deny` |
| `permission.ask` | `permission.ask: {"*": "allow"}` | added so `/ps:askme` asks without a prompt |
| MCP (`cursor/mcp.json`) | `opencode.jsonc` `mcp.<name>` | merge as `type: remote`; only with `OPENCODE_MCP="on"` or `DOTFILES_MCP=on` |
| copper theme | `themes/copper.json` + `tui.json` `"theme": "copper"` | generate the file from the repo palette, then set the name. A `tui.json` that names another theme is yours and only gets reported. |
| per-project config | `project/.opencode/opencode.jsonc` and `agents/*.md`, as links to the global ones | paths in `OPENCODE_PROJECTS` (`~` allowed); existing project files are never replaced |
| guard | `plugins/dots-guard.ts` | generate (see below) |
| `includeCoAuthoredBy`, `attribution` | none | Opencode attributes commits as the user; nothing to force off |
| `statusLine` | none | no status line |

A `~/.config/opencode/AGENTS.md` without the marker is yours. `dots` leaves it and says to put
personal rules in `~/.config/dotfiles/owner.md` instead.

A sidecar file, `~/.config/opencode/.dotfiles-managed.json`, records the permission rules `dots`
added, so a rule dropped from `claude/settings.json` goes on the next run, unless you changed its
effect. `opencode.jsonc` must be strict JSON: a file with comments is left alone with a warning.

Schema facts behind the theme layout: `tui.json` takes `theme` as a name and allows no other keys,
so the palette lives in its own `themes/<name>.json`. The loader refuses the whole theme when one
colour key is missing (`Failed to load theme: copper`). The shipped themes use 49 keys, so
`rust/agent-config/themes/copper.json` follows them, and a test pins that key set. Change the
palette there, not in `~/.config/opencode`. A `permission` object is deprecated in the v2 schema
in favour of a `permissions` array, but it is still accepted and is what this repo writes.

## Guard plugin

`agent-config opencode` writes `~/.config/opencode/plugins/dots-guard.ts` from
`rust/agent-config/src/opencode_plugin.ts.tmpl`. A file of that name without the marker is yours.
The plugin is removed when `claude/hooks/rust-bin` is gone. Which check runs where:
[AI agents](ai-agents.md#where-each-guard-check-runs).

- **Shape:** a default export `{ id, setup(ctx) }` that calls
  `ctx.tool.hook("execute.before", async (input) => ...)`. The older
  `export const X = async () => ({"tool.execute.before": ...})` form is refused. `setup` can run
  twice, so state lives at module level and the verdict is cached per call.
- **Blocking:** a throw blocks the call and the model sees the text. Verified live: a bare
  `sed -i ...` came back as `G14: bare sed -i on macOS ...` and nothing ran.
- **Tool names seen live:** `shell` (arg `command`), `read` (arg `path`), `grep`, `glob`, `execute`,
  and MCP tools as `<server>_<tool>`, which the plugin turns into `mcp__<server>__<tool>`. The
  `execute` tool runs inner calls and each one fires the hook again, so the plugin skips `execute`
  itself and keys its cache by call id, tool name and input.
- **Answer:** exit code 2 (a deny) throws the guard's stderr text. An `ask` throws its reason with
  a note that Opencode cannot ask. A missing launcher, a crash or a non-JSON answer allows the call
  and writes one `console.warn` per session.
- **Settings:** the checks read the settings of the user running Opencode, so the same presets,
  `GUARD_OFF` and `GUARD_SHADOW` apply as in Claude Code.
- **Windows:** the plugin starts `sh` to run `rust-bin`. A bare name there can find a program in
  `System32` first (bash is the WSL launcher), so `dots` writes the full path of Git Bash's
  `sh.exe` (`usr\bin`) into the plugin, never a `System32` path. With no usable `sh` on PATH it
  falls back to `sh` by name, which needs Git Bash on PATH. Run `dots` from Git Bash.

## Gaps

| Claude feature | Why Opencode cannot match it |
|----------------|------------------------------|
| Guard checks other than G14 to G17 | The plugin sees only shell, read, grep and MCP calls, so rules about subagents, questions and hand-backs never see their events. A rule on a shell command may fire; it is not tested. |
| Asking before a call (G16) | A plugin cannot ask. An `ask` blocks the call and the message says so. |
| Output styles | No such feature. The style is appended to `AGENTS.md`; switching means editing `outputStyle` and running `dots`. |
| `claude/` hooks directory | Not read. Opencode loads plugins from `~/.config/opencode/plugins/`. |
| `mode: primary` agents | All ported agents are `subagent`. |
| claude.ai connectors, Claude Code plugins | Tied to the account, or not read (Opencode reads Claude skills and prompts, not plugins). |

## Decisions

- Agents inherit the session model, so a wrong `provider/model-id` can never break a session.
- `AGENTS.md` is generated, not linked, because a hand-written file would hide the owner rules and
  the output style that the `CLAUDE.md` fallback does not carry.
- `opencode.jsonc` is merged, never linked: Opencode rewrites it.
- `DOTFILES_MCP=on` turns MCP on for Cursor and Opencode at once; `CURSOR_MCP` and `OPENCODE_MCP`
  control each app on its own.
- `DOTFILES_STEPS=58-opencode ./dotfiles` runs only the Opencode check, for temp-HOME tests.
