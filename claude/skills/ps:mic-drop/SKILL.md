---
name: ps:mic-drop
description: Use when the user wants to wrap up, close out, or finish a session — "do everything yourself", "is it all pushed?", "did checks pass?", "clean up background tasks", "archive this session", "are we done?", or before leaving a repo for the day.
argument-hint: "[--squash] [extra repo paths to check — empty = this session's repos]"
allowed-tools: "Bash(bash ${CLAUDE_SKILL_DIR}/scripts/mic-check.sh *)"
---

# Mic Drop

Finish the job. Prove it is finished. Then leave the stage.

Done means: nothing mandatory left, everything committed and pushed, no conflicts, checks green, no stray tasks. Optional ideas go to ONE leftovers file. Then archive when all of that is true and the session is not pinned. No need to ask.

## Steps

1. **Finish the work.** Read back over this session. List every promise, TODO, and failing thing.
   - Mandatory (the user asked for it, or it is broken) → do it now.
   - Optional (nice to have) → save it: `bash ${CLAUDE_SKILL_DIR}/scripts/mic-check.sh --note "<one line>" <repo path>`.
2. **Clean background work.** List everything you started: background Bash shells, agents, Monitors, crons (`CronList`), wakeups. See the table below. Stop what is done, stuck, or obsolete with `TaskStop` / `CronDelete`.
   desktop-commander processes do not show up there. Call desktop-commander `list_sessions` (Claude Code: load with ToolSearch `+desktop-commander list_sessions force_terminate`), one call, no `ps` digging. A PID that this session's own `start_process` call returned (or the ledger names) and that is done, stuck or obsolete: `force_terminate <pid>`. Leave other PIDs alone. No such tool → there are none.
3. **Scan.** Run `bash ${CLAUDE_SKILL_DIR}/scripts/mic-check.sh [--since <first session sha>] <every repo this session touched>`.
   It does `git fetch`, then reads only. Every line is `OK`, `FIX` (mandatory) or `WARN` (decide).
4. **Fix every `FIX`.** See the table. For each `WARN`: fix it, or `--note` it.
5. **Verify.** Run every command under `RUN`. Fix failures. Wait for CI on the pushed commit, up to 15 minutes. Still running after that → report it as pending and do not archive. The user said "archive this" and the scan is CLEAN → skip this step and the CI wait.
6. **Scan again.** It must print `VERDICT: CLEAN`. If not, go back to step 4. Three passes at most; after the third, stop, report what is still red, and keep the session.
7. **Report** in the shape below, then **archive** (Claude desktop app only) when the scan is CLEAN. See "Archive".

## FIX → what I do

| Scan says | I do |
|---|---|
| merge/rebase half done, conflicts, markers | Finish it. Resolve by reading both sides. Re-run checks after. |
| not committed | Commit. Never commit secrets or junk — gitignore junk. |
| not pushed, no upstream | `git push` (`-u origin <branch>` if no upstream). In squash mode, squash first (below). |
| not pulled | `git pull --rebase`, then re-run checks, then push. |
| CI failed | Read the log (`gh run view --log-failed`). Fix. Push. Wait. |
| detached HEAD | Put the commits on a branch first. |

| Scan warns | I do |
|---|---|
| not on main | Work lands on main as one commit (global rule 13). The user or the repo said PR → open a PR instead. Unsure which → ask. |
| local-only branch, stash | Look inside. Ours and needed → push or apply. Old → `--note` it. Never delete it. |
| extra worktree | Agent worktree, branch merged into main, tree clean → `git worktree remove <path>` (or `mcp__ccd_host__clean_up_worktrees`). Not merged → look inside, `--note` it. |
| processes from this repo | Ours and not needed → stop. Not ours → leave. Judge by the scan line and `/bin/ps -o pid=,ppid=,user=,lstart=,ucomm= -p <pid>`. Never print a full command line (`ps` with `args`, `command` or `comm`, `pgrep -fl`): it can hold keys. |
| no check command found | Read README / AGENTS.md / CI config. Run what they run. |

## Squash mode

On when the repo CLAUDE.md or AGENTS.md says one commit on main, the user asks for it, or `--squash` is passed. Squash only commits THIS session made.

1. Find them: scan with `--since <sha of the first commit this session made>`. It lists them and warns about other authors and merges.
2. Any listed commit this session did not make, or a merge → leave it out. Squash only the unbroken run of ours at the tip. None → skip squash.
3. `git reset --soft <base>`, one commit with a plain subject, then `git push --force-with-lease`. A rejected lease means someone pushed. Fetch, look, never retry with `--force`. A squash not done (denied, rejected) means NOT CLEAN: keep the session and put the one-line command in "You do".

## Background tasks

Finished → read its result once. Running and still needed → wait. Obsolete (task changed, duplicate, result known) → stop it. Stuck (no output for 10+ min) → read its last output, stop it, redo the work if needed. Dev server or watcher you started → stop it.

## Report, exactly this shape

Short words. Short sentences. No jargon. One idea per line.
Send these lines as plain text, outside any code block. The first line is the deliverable: the path, URL or short sha. A surprise (a bug you found) goes on line 2.

```
Done: <path, URL or sha>: <what I did, max 5 lines>
Checks: <which ran>: passed / failed
Git: <repo>: committed, pushed, <short sha> on main, CI green / no CI / skipped
Tasks: <n> stopped, <n> still running (why)
Leftovers: <n> saved in ~/.claude/leftovers.md
You do: <exact steps for anything only the user can do, or "nothing">
Session: archived / kept (pinned) / kept (not clean)
```

"You do" lists what I cannot do: logins, secrets, approvals, paid things, decisions. Give the exact command or click.

## Archive

1. `mcp__ccd_session_mgmt__get_session` with `"self"`. Read `pinned`. Pinned → do NOT archive. Say "pinned, so I kept it".
2. Sweep, right before the archive call. The app refuses to archive while any agent run, queued message or background task is live. Stop all of these:
   - `TaskStop` every background task, background agent, monitor and dev server this session started.
   - `CronList`, then `CronDelete` every cron. A pending wakeup: `ScheduleWakeup` with `stop: true`.
   - `force_terminate` every desktop-commander PID it started.
   - Remove merged agent worktrees.
3. Archive when ALL are true. Do not wait for the user to ask.
   - The last scan printed `VERDICT: CLEAN`.
   - Every check under `RUN` passed in this session and CI is green, unless the user said "archive this".
   - No task of mine is left after the sweep.
   - "You do" says "nothing".
   Any of them false → do NOT archive. Say "kept (not clean)" and why.
4. Remote Control on (`get_session` shows it) → `mcp__ccd_session_mgmt__set_remote_control` off, before the next step.
5. Send the report text FIRST, in the same message. Then call `mcp__ccd_session_mgmt__archive_session` with `"self"`. The conversation ends at that call.
6. Refused with "still has live work" → the report already said "archived", so fix it in one line. Run the sweep once more and call archive once more. Refused again → say "Session: kept (not clean)" and put "archive it from the sidebar" under "You do". Never loop.
7. No `ccd_session_mgmt` tools (terminal CLI) → say "archive it by hand".

## Rules

- **CLEAN is the only proof.** Not memory. Not "I pushed earlier". Run the scan last.
- Never say "checks pass" for a check you did not run in this session.
- Never plain `--force`, `reset --hard`, delete branches, drop stashes, or skip hooks (`--no-verify`) to make the scan green. The only force is `--force-with-lease` in squash mode.
- Optional work goes to the leftovers file, never mandatory work. When a fix needs the user, put it in "You do". Do not archive.

## Options

`--no-fetch` skip the network. `--since <sha|time>` list this session's commits. `--note "text" [repo]` save a leftover for that repo. `--leftovers` show the file. `MIC_LEFTOVERS=<path>` use another file.
