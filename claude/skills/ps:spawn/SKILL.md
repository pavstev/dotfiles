---
name: ps:spawn
description: "Starts a new Claude Code session on this Mac with Remote Control on and permissions bypassed, working on a given prompt, so the user can watch it from the phone. Use when the user runs /ps:spawn, or asks (often from the phone through Remote Control) to start, open, launch or spawn a new session, a fresh session or a second session with a prompt."
argument-hint: "[prompt for the new session] [in <folder>] [as <name>]"
allowed-tools: "Bash(python3 ${CLAUDE_SKILL_DIR}/scripts/*)"
---

# Spawn a Remote Control session

One script call starts a new session in a detached tmux, with `claude --remote-control`, in permission mode `bypassPermissions`. Every check runs before the start, so it starts once and never retries.

## Steps

1. **The prompt.** Take the text after `/ps:spawn`, or the prompt the user just gave you for the new session. "The prompt above" or "the same prompt" means the user's earlier message: copy it word for word. Write it with the Write tool to `prompt.md` in your scratchpad (no scratchpad: a new `mktemp -d` folder). Do not reword it. Add nothing.
2. **The folder.** A folder the user named, else this session's working directory. The prompt names files (`tests/auth.spec.ts`) that are not in that folder and the user named none → ask one question: the folder you think fits (Recommended) or this one.
   **The name.** A name the user gave, else 2-4 words for the job (`fix-login-test`). The script adds nothing to it.
3. **Say it starts.** One line before the call: "Starting <name> in <folder>, up to 60 s."
4. **Run once:** `python3 ${CLAUDE_SKILL_DIR}/scripts/spawn.py --prompt-file <scratch>/prompt.md [--dir <folder>] [--name <name>]`
   `--dry-run` runs only the checks and prints the command.
5. **Read the exit code.** Never run it a second time for the same prompt.

| Exit | Meaning | Do |
|---|---|---|
| 0 | started, Remote Control link seen | Reply in the shape below |
| 3 | a check failed before the start (no tmux, not logged in, folder not trusted, name taken, bypass not accepted) | Give the FIX line as the user's one step. Name taken → pick another name, run once more. Bypass not accepted → ask one question (below) |
| 4 | the session stopped, or showed a dialog | Give the FAIL line and the screen lines. The script already stopped it |
| 5 | no link in time, the session still runs | Give the WATCH and STOP lines. Do not start another |

Bypass warning not accepted (exit 3): ask with `AskUserQuestion` (Cursor: `AskQuestion`), one question:
- "Start now in auto mode (Recommended)": run step 4 again with `--mode auto`. A safety check approves most commands; it asks the phone only for risky ones. This is the one allowed second run.
- "I accept bypass first": the user runs `claude --permission-mode bypassPermissions` once in a terminal, accepts, types `/exit`, then says go.

## Reply shape

The script prints `NAME`, `MODE`, `URL`, `WATCH` and `STOP` lines. Use the mode it really ran in.

```markdown
Started **fix-login-test** in `~/proj` (bypassPermissions, Remote Control on).
- Phone: Claude app > Code > fix-login-test, or https://claude.ai/code/session_…
- Mac: `tmux -L claude-spawn attach -t fix-login-test`
- Stop: `tmux -L claude-spawn kill-session -t fix-login-test`
```

To watch it or message it from here later, use `ps:sessions`.

## Rules

- Never turn off the bypass warning or trust a folder for the user. Accepting it once is the user's step: Claude then writes `skipDangerousModePermissionPrompt: true` to the user settings itself.
- Never edit `~/.claude.json` or the settings to make a check pass.
- One start per prompt. A failed start gets its FIX line, not a retry loop.
- The new session runs on its own. This session does not wait for it or poll it.
