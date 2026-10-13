# Hooks

[Hooks](https://docs.claude.com/en/docs/claude-code/hooks) run shell commands on Claude
Code events (e.g. `PostToolUse`, `Stop`, `UserPromptSubmit`). The *scripts* live here;
they're **wired up in `settings.json` under the `hooks` key** (not auto-enabled, so you
opt in deliberately).

Example wiring in `settings.json` to run `format-after-edit.sh` after edits:

```json
{
  "hooks": {
    "PostToolUse": [
      { "matcher": "Edit|Write",
        "hooks": [ { "type": "command", "command": "~/.claude/hooks/format-after-edit.sh" } ] }
    ]
  }
}
```

The hooks directory is symlinked to `~/.claude/hooks` by `./dotfiles`.
See `format-after-edit.sh` for a sample.

## guard.py

`guard.py` (Python 3, stdlib only, fail-open: any error exits 0 silently) keeps sessions
cheap. Deny = message on stderr + exit 2. Tests: `python3 claude/hooks/test_guard.py`.

| Guard | Event | Rule |
|-------|-------|------|
| G1 | PreToolUse `Agent` | needs `model` + `effort`; `effort: max` only for plan / synthesis / architecture / final review |
| G2 | PreToolUse `CronCreate`, `ScheduleWakeup`, `Skill` (`loop`) | no interval under 30 min once context > 100K |
| G3 | UserPromptSubmit; PreToolUse `Agent` | at 300K+ nudge `/ps:handoff` (1st prompt, then every 5th); deny spawning above 500K |
| G4 | PreToolUse `AskUserQuestion` | one call per human turn |

Context size comes from the last assistant `usage` in the transcript (read from the end
of the file, so big transcripts are cheap). Nudge counters live in
`${TMPDIR:-/tmp}/claude-guard/<session_id>.json`.

Wired in [`claude/settings.json`](../settings.json) (`hooks`). Each command starts with
`test -r ~/.claude/hooks/guard.py || exit 0`, so a missing script never blocks a tool.
