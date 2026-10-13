---
name: ps:whatsleft
description: Use when the user runs /ps:whatsleft, or asks what is left, what remains, what is still open, "where are we", "what's on me", "what do I still need to do", "who does what next", wants the open work split between the user and Claude, or says the session is "slow", "laggy", "speed up", "hurry up" or "trim this session".
argument-hint: "[kill dead tasks | /askme | focus text]"
allowed-tools: "Bash(python3 ${CLAUDE_SKILL_DIR}/scripts/*) TaskStop mcp__plugin_desktop-commander_desktop-commander__list_sessions mcp__plugin_desktop-commander_desktop-commander__force_terminate"
---

# What's Left

Two tables, one for Claude and one for the user, plus an ETA and a table of running tasks. Every row is one simple action. Every ✅ has proof.

This skill reports. To finish everything and close the session, use `ps:mic-drop`.

## Args

- `kill dead tasks`: call TaskStop (desktop-commander PID: `force_terminate`) on every task marked STOP, in the same turn, and say which. This is the one action that does not wait for **go**.
- `/askme`: after the report, run `ps:askme` light mode on the open 🧑 rows.
- Any other text is a focus: keep only rows about it. Args can combine.

## Steps

1. Run `python3 ${CLAUDE_SKILL_DIR}/scripts/whatsleft-scan.py`. It reads this session's transcript, the git state of every repo the session touched, running tasks and slow-session findings. It is read-only except `git fetch`; `--no-fetch` skips it, `--no-ci` skips CI, `--session <id|path>` scans another session.
2. Check the `DELIVERABLE` guess. The reply starts with it: a path, a URL or a commit sha. Nothing → `Deliverable: none yet`.
3. Check every `ASK` line. Find the proof in this session: a tool result that shows it done (test output, pushed sha, file on disk). Proof counts only when it came after the last change it covers, and covers that exact thing. Proof found → ✅ Done. No proof → it is a row.
4. Turn every `ME`, `YOU`, `CHECK`, `LATER` and `SPEED` line into a row. A line that a read-only check shows is fine is dropped.
5. Add what the scan cannot see: promises you made, steps you skipped, work the asks imply (an ask for "tests" needs a test that covers the new code). Confirm cheap facts with read-only commands (`git status`, `gh pr view`). Running the test suite is 🤖 work and waits for **go**.
6. Put each row in 🤖 Me or 🧑 You. Every `TASK` line is one row of 🏃 Tasks, with its STOP or KEEP advice.
   The scan cannot see desktop-commander processes. Call desktop-commander `list_sessions` (Claude Code: load with ToolSearch `+desktop-commander list_sessions force_terminate`): a PID that this session's own `start_process` call returned is one more task row, kind `dc process`, named from that call. Not needed any more → STOP, else KEEP. Any other PID is not yours: leave it out. No such tool → there are none.
7. Write the report. Then stop and wait.

## Who does it

- 🤖 Me: code, tests, lint, docs, commit and push when asked, logs and CI, stopping dead tasks, `SPEED [ME]` fixes.
- 🧑 You: logins, secrets, payment, decisions (merge or PR, keep or drop, scope), anything refused in a prompt, slash commands (`/compact`, `/mcp`), `SPEED [YOU]` findings, clicks in other apps.

- A decision goes to 🧑 You as a question: 2 options max, what each means, and your pick.
- Anything that deletes or drops work (stash, branch, files) is a decision.
- Work that waits on a 🧑 row stays in 🤖 Me with "after 🧑 1" in the action.
- Optional work (`LATER`, nice-to-have) goes last, marked "(optional)".

## Report shape

Plain markdown, outside any code block, so the tables render.

```markdown
Deliverable: `a1b2c3d` on origin/main

## 🧾 What's left: 🤖 2 for me · 🧑 2 for you
ETA ~35 min (25-50)

### 🤖 Me
| # | What's left | What I do |
|---|---|---|
| 1 | No test for password reset | Add a test for the reset email |
| 2 | Code changed after the last lint | Run `pnpm lint` |

### 🧑 You
| # | What's left | Do this |
|---|---|---|
| 1 | GitHub CLI logged out | Run `gh auth login` |
| 2 | Merge or PR? | Say **merge** (fast) or **PR** (review first). I pick **PR**. |

### 🏃 Tasks
| Task | Kind | Running for | Last output | Recommend |
|---|---|---|---|---|
| dev server | shell | 2h10m | 90m ago | 🛑 Stop: idle |

✅ Done: reset form in `app/reset.tsx` (commit `a1b2c3d`)
👉 Say **go** and I do the 🤖 list.
```

Fill it like this:
- Line 1 is the deliverable. The ETA line is the scan's, copied as is.
- Title counts match the rows. "What's left": 3-8 words, the problem, not the history.
- "What I do" / "Do this": one action. An exact command, a click path, or a question with options. Two steps → two rows.
- Name tasks by what they do. Task IDs stay out of the report (use them for TaskStop).
- Empty table: one row `| – | Nothing | ✨ |`. No tasks → leave out 🏃 Tasks. Both tables empty: title `## 🧾 What's left: ✨ nothing`.
- ✅ line: up to 5 items, each with its proof in brackets. 👉 line: the next move.

## Rules

- No proof, no ✅. Without proof it is a 🤖 row: "Check that X works".
- The user's word is not proof. "The tests passed" → read the output or run the check yourself.
- Never claim a speed gain you did not measure.
- Name secrets. Never write their values.
- The 🤖 work starts after the user says **go**. Only `kill dead tasks` runs at once.
