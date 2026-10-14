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
| G1 | PreToolUse `Agent` | needs `model` + `effort`; `effort: max` only for plan / synthesis / architecture / final review; the prompt must name a deadline and a hand-back cap (for example `Deadline: 25 min.` and `Hand back at most 4,000 chars; write the rest to a file.`) |
| G2 | PreToolUse `CronCreate`, `ScheduleWakeup`, `Skill` (`loop`) | no interval under 30 min once context > 100K |
| G3 | UserPromptSubmit; PreToolUse `Agent` | at 300K+ nudge `/ps:handoff` (1st prompt, then every 5th); deny spawning above 500K |
| G4 | PreToolUse `AskUserQuestion` | one call per human turn |
| G5 | PreToolUse `SubagentHandback`; SubagentStop | a subagent's hand-back may not exceed its cap by more than 10 percent; at most 2 denies per agent, then it passes |
| G6 | Stop; PreToolUse `archive_session` | a hand-off file written this turn (`*/handoffs/*.md`) must be in the reply as a file link plus the `Read <path> and do it.` block (one deny); no archive of this session in the same turn |

Context size comes from the last assistant `usage` in the transcript (read from the end
of the file, so big transcripts are cheap). Nudge counters live in
`${TMPDIR:-/tmp}/claude-guard/<session_id>.json`.

G5 reads the cap from the subagent's own prompt: the smallest `N chars` (N of 200 or more) tied to
the reply by a word in the same sentence, up to 80 characters before the number or 40 after it.
A "hand back" tie wins first, then strong words (final message, reply, response, output), then weak
ones (answer, return, report, summary), so "return at most 500 characters" in a spec does not beat
"hand back 4,000 chars".
Last comes a bare bound right before the number, as in "Stay ≤ 4,000 chars.".
If the counter file cannot be written, G5 allows, so a deny never repeats without end. Without
one, the cap is the default of the agent type (task-implementer, task-reviewer, researcher and
ui-verifier 3,000; repo-scanner 2,000; code-reviewer and anything else 4,000). The prompt is the
first user record of the subagent transcript (`agent_transcript_path`, or
`<session>/subagents/agent-<agent_id>.jsonl`). A deny makes the subagent write its text to a file and
hand back a shorter one. SubagentStop skips the check when `stop_hook_active` is true. Deny counters
live in `${TMPDIR:-/tmp}/claude-guard/handback-<agent_id>.json`.

In auto mode a subagent hands back through the `SubagentHandback` tool, and SubagentStop then has no
`last_assistant_message`. In other modes SubagentStop carries the text. So G5 hooks both (tested
live with Claude Code 2.1.296).

Replay on real past calls (2026-10-14, every transcript on this Mac):

- G1 brief check: 1,472 Agent calls. After the deadline-and-cap rule began (2026-10-12, about
  11:00 UTC), 2 of 474 are denied, both in the first hour and both "report per the brief" prompts
  with no cap of their own. Before the rule, 922 of 998 lacked a deadline, a cap or both.
- G5: 1,423 hand-backs replayed through the script. Since the rule, 162 of 449 are denied. A hand
  check of all 162: 155 are over every character number in the prompt, 4 are over the default cap
  of a prompt with no character cap, and 3 are over the hand-back cap but under a second cap the
  caller gave for text about something else or for "if the notes file is refused". The 2-deny limit
  bounds the cost of those 3.
- Scratch mode: 148 real writes by browser-check subagents. 133 scratch or report-folder writes
  pass; the 15 denied ones are code and docs edits (`src/lib/*.ts`, `README.md`, a test file).

Not built: a Bash rewrite of `cd <worktree> && git ...`. Of 478 worktree refusals only 1 had that
shape; the harness check runs after PreToolUse hooks and cannot be bypassed, and agents recover on
the next call. The agent prompts carry the fix instead.

### Scratch mode

`python3 ~/.claude/hooks/guard.py scratch` is not wired in `settings.json`. An agent file wires it
in its frontmatter as a `PreToolUse` hook on `Write|Edit|NotebookEdit` (the ui-verifier does). It
allows a write only when the real path is under `/tmp`, `/private/tmp`, `/var/folders`,
`/private/var/folders`, `$TMPDIR`, a `scratchpad` folder, or a `.agents/` or `.superpowers/` note
folder (callers name report files there). Anything else is denied.

Wired in [`claude/settings.json`](../settings.json) (`hooks`). Each command starts with
`test -r ~/.claude/hooks/guard.py || exit 0`, so a missing script never blocks a tool.
