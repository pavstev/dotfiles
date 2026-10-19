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

`guard.py` and its Rust twin `guard` (`rust/agent-config`; same checks, tested by `rust/agent-config/tools/run_hook_tests.py`) (Python 3, stdlib only, fail-open: any error exits 0 silently) keeps sessions
cheap. Deny = message on stderr + exit 2. Warn (the W rows) = exit 0 with
`hookSpecificOutput.additionalContext` on stdout, so the call runs and the model reads the note.
A deny wins over a warning. Tests: `python3 claude/hooks/test_guard.py`.

| Guard | Event | Rule |
|-------|-------|------|
| G1 | PreToolUse `Agent` | the prompt must name a deadline and a hand-back cap (for example `Deadline: 25 min.` and `Hand back at most 4,000 chars; write the rest to a file.`). `model` and `effort` may be missing: the router fills them |
| G2 | PreToolUse `CronCreate`, `ScheduleWakeup`, `Skill` (`loop`) | no interval under 30 min once context > 100K |
| G3 | UserPromptSubmit; PreToolUse `Agent` | at 300K+ nudge `/ps:handoff` (1st prompt, then every 5th); deny spawning above 350K |
| G4 | PreToolUse `AskUserQuestion` | one call per human turn |
| G5 | PreToolUse `SubagentHandback`; SubagentStop | a subagent's hand-back may not exceed its cap by more than 10 percent; at most 2 denies per agent, then it passes |
| G6 | Stop; PreToolUse `archive_session` | a hand-off file written this turn (`*/handoffs/*.md`) must be in the reply as a file link plus the `Read <path> and do it.` block (one deny); no archive of this session in the same turn |
| G7 | PreToolUse `Agent` | deny `general-purpose`, `claude` and a missing `subagent_type`; the message names `worker`, `researcher`, `Explore`, `task-implementer`, `Plan` |
| G8 | PreToolUse `Agent`; SubagentStop | helpers running at once per session: 3 when pace is `ok` or `unknown`, 1 when `ahead`, 0 when `critical`. Each slot is `{id: tool_use_id, t: time}` in the session state file (plain times from the first version still load). SubagentStop frees the oldest slot (not when its hand-back is denied); `guard.py posttoolfailure` (PostToolUseFailure, matcher `Agent`) frees the slot of the failed call's `tool_use_id`; entries over 90 min old are dropped. `flock` guards parallel calls |
| G9 | PreToolUse `Skill`, `EnterPlanMode` | `superpowers:brainstorming` and `superpowers:writing-plans` in the main thread are allowed in plan mode, when pace is `ahead` or `critical`, and when the main model is Opus or Fable (newest assistant `message.model` in the transcript). With a Sonnet or Haiku main model they are denied: on `entrypoint: cli` use EnterPlanMode (opusplan); on `entrypoint: claude-desktop` (no opusplan) send a `Plan` helper or pick Opus. An unknown model or entrypoint is allowed. `EnterPlanMode` is denied when pace is `ahead` or `critical` (Sonnet plans without plan mode) |
| W3 | PreToolUse browser tools (`mcp__Claude_Browser__*`, `mcp__claude-in-chrome__*`) | warn in a main window: at the 6th browser call of a turn (then every 20th), route checks to `ui-verifier` |
| W4 | PreToolUse `Agent` | warn: past 15 spawns or 1.5M subagent tokens in a session (the `totalTokens` of each Agent result, and of each background task notification once), a budget note on every 5th spawn |

**Router** (PreToolUse `Agent`, main thread, not `fork`): the guard sets `model` and `effort`, and
`xhigh` or `max` never pass. `Plan` and a `code-reviewer` doing a final or whole-branch review
(description or the first 400 chars of the prompt; a panel angle or re-review is not final) get
opus + high (sonnet + high when pace is `ahead` or `critical`). Every other row never raises
cost: the lower of the caller's model and the table's (`Explore`, `claude-code-guide`,
`statusline-setup`: haiku; the rest: sonnet), effort `high` unless the caller gave `low` or
`medium`, no effort for haiku. A changed call is
returned as `permissionDecision: allow` + `updatedInput` (the whole tool input) + a note, on the
same JSON line as the W notes.

**Pace** comes from `pace.py` in this folder (`pace.status()`; `pace.refresh(budget_s=5)` on each
UserPromptSubmit). Without the module, or on any error, the state is `unknown`. On a prompt, the
first time a session's state becomes `ahead`, the guard adds one note (used and budget percent,
what changes); while `critical` it adds the note on every prompt. The last announced state is kept
in the session state file, and `unknown` does not reset it.

Context size comes from the last assistant `usage` in the transcript (read from the end
of the file, so big transcripts are cheap). Session state (nudge counter, spawn times, last pace
note) lives in `${TMPDIR:-/tmp}/claude-guard/<session_id>.json`.

G5 reads the cap from the subagent's own prompt: the smallest `N chars` (N of 200 or more) tied to
the reply by a word in the same sentence, up to 80 characters before the number or 40 after it.
A "hand back" tie wins first, then strong words (final message, reply, response, output), then weak
ones (answer, return, report, summary), so "return at most 500 characters" in a spec does not beat
"hand back 4,000 chars".
Last comes a bare bound right before the number, as in "Stay ≤ 4,000 chars.".
If the counter file cannot be written, G5 allows, so a deny never repeats without end. Without
one, the cap is the default of the agent type (task-implementer, task-reviewer, researcher and
ui-verifier and worker 3,000; repo-scanner 2,000; code-reviewer and anything else 4,000). The prompt is the
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

Not built: a Bash fixer for worktree refusals. Re-measured 2026-10-15 over 7 days: 417 refusals,
86 percent in subagents (59 percent in one repo's own agents). 392 of 404 refusal streaks end on
the next call, so a hook deny in place of the harness refusal saves nothing. None of the 417 has
the one shape a hook could rewrite without changing what runs (`cd <root> && <one command>`): they
are heredoc scripts, pipes and chains. The harness check runs after PreToolUse hooks and is a safety
check, so the hook does not try to get around it. The rule text carries the fix instead:
`claude/CLAUDE.md` rule 18 and the `worker` and `task-implementer` bodies.

### Scratch mode

`~/.claude/hooks/rust-bin --hook guard scratch` is not wired in `settings.json`. An agent file wires it
in its frontmatter as a `PreToolUse` hook on `Write|Edit|NotebookEdit` (the ui-verifier does). It
allows a write only when the real path is under `/tmp`, `/private/tmp`, `/var/folders`,
`/private/var/folders`, `$TMPDIR`, a `scratchpad` folder, or a `.agents/` or `.superpowers/` note
folder (callers name report files there). Anything else is denied.

Wired in [`claude/settings.json`](../settings.json) (`hooks`). Each command starts with
`test -r ~/.claude/hooks/rust-bin || exit 0`, so a missing launcher never blocks a tool.
`rust-bin --hook guard <mode>` runs the Rust `guard` when it is newer than its sources, else
`guard.py`; with neither a binary nor `python3` it exits 0, silent. It never builds (`dots` does,
with `scripts/agent-config --bins`). Force one with `RUST_BIN_IMPL=python|rust`. To go back to
Python for good, revert the commit that switched the commands. Notes: `rust/MIGRATION.md`.
