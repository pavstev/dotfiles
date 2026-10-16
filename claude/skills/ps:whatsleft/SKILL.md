---
name: ps:whatsleft
description: Use when the user runs /ps:whatsleft, or asks what is left, what remains, what is still open, "where are we", "what's on me", "what do I still need to do", "who does what next", wants the open work split between the user and Claude, or says the session is "slow", "laggy", "speed up", "hurry up" or "trim this session".
argument-hint: "[kill dead tasks | report only | focus text]"
allowed-tools: "Bash(python3 ${CLAUDE_SKILL_DIR}/scripts/*) Skill(ps:askme) Skill(ps:mic-drop) TaskStop mcp__plugin_desktop-commander_desktop-commander__list_sessions mcp__plugin_desktop-commander_desktop-commander__force_terminate"
---

# What's Left

Two tables, one for the agent and one for the user, plus an ETA and a table of running tasks. Every row is one simple action. Every ✅ has proof.

Then the agent does its own rows, asks the user the rest through [ps:askme](../ps:askme/SKILL.md), and closes out with `ps:mic-drop` when nothing is left to ask. `scripts/route.py` decides who does what, so the answer is the same every time. The same steps hold in Claude Code, in Cursor and in OpenCode.

## Args

- `kill dead tasks`: stop every task marked STOP without asking (route.py `--kill`), and say which.
- `report only`: steps 1-9 only. No work, no questions, no mic-drop.
- Any other text is a focus: keep only rows about it. Args can combine.

## Tools by app

| Job | Claude Code | Cursor | OpenCode |
|---|---|---|---|
| Skill folder (`${CLAUDE_SKILL_DIR}`) | set for you | the folder of this SKILL.md | the folder of this SKILL.md |
| Question with options | `AskUserQuestion` | `AskQuestion` | `question` |
| Run ps:askme, ps:mic-drop | Skill tool | read its SKILL.md (`../ps:askme/`, `../ps:mic-drop/`) and follow it | same: read its SKILL.md and follow it |
| Stop a task | `TaskStop`; desktop-commander PID: `force_terminate` | `kill <pid>` (find it with `lsof -ti :<port>` or `pgrep -f`) | `kill <pid>` (same way to find it) |
| Web dashboard | Claude in Chrome, else the built-in browser | its browser tool | its browser tool, or a `bash` call to the CLI |
| Native app | computer-use | none: 🧑 row | none: 🧑 row |
| Slash command | the user types it | the user types it, or a new chat for `/compact` | the user types it |

## Steps

1. Run `python3 ${CLAUDE_SKILL_DIR}/scripts/whatsleft-scan.py`. It reads this session's transcript, the git state of every repo the session touched, running tasks and slow-session findings. It is read-only except `git fetch`; `--no-fetch` skips it, `--no-ci` skips CI, `--session <id|path>` scans another session.
2. Check the `DELIVERABLE` guess. The reply starts with it: a path, a URL or a commit sha. Nothing → `Deliverable: none yet`.
3. Check every `ASK` line. Find the proof in this session: a tool result that shows it done (test output, pushed sha, file on disk). Proof counts only when it came after the last change it covers, and covers that exact thing. Proof found → ✅ Done. No proof → it is a row.
4. Turn every `ME`, `YOU`, `CHECK`, `LATER` and `SPEED` line into a row. A line that a read-only check shows is fine is dropped.
5. Add what the scan cannot see: promises you made, steps you skipped, work the asks imply (an ask for "tests" needs a test that covers the new code). Confirm cheap facts with read-only commands (`git status`, `gh pr view`).
6. Read the user's rules (CLAUDE.md, AGENTS.md, owner rules). A decision they already settle (main or PR, commit style) is not a question: follow the rule.
7. Write each row once into `rows.json` in your scratchpad (no scratchpad: `mktemp -d`), with its `kind` (see "Who does it"), `risk`, `after` and `done`. Every `TASK` line is a row of kind `task` with `stop` from the scan.
   The scan cannot see desktop-commander processes. Call desktop-commander `list_sessions` (Claude Code: load with ToolSearch `+desktop-commander list_sessions force_terminate`): a PID that this session's own `start_process` call returned is one more task row, named from that call. Not needed any more → `stop`. Any other PID is not yours: leave it out. No such tool → there are none.
8. Run `python3 ${CLAUDE_SKILL_DIR}/scripts/route.py <scratch>/rows.json` (add `--kill` for `kill dead tasks`). It prints a route per row and three NEXT lines. Exit 2 → fix the row it names, run it again. Do not route by hand.
9. Write the report from the routes: DO rows and waiting work go in 🤖 Me, YOU rows in 🧑 You, ASK rows in 🧑 You as "Question below", a waiting question in 🧑 You as "Question next round", task rows in 🏃 Tasks.
10. **NEXT 1:** do the DO rows now, in the same turn, in order.
11. **NEXT 2:** run `ps:askme` with the `whatsleft:` line route.py printed. askme asks in one call, then does the work the answers unlock. Name the DEFAULT rows and your pick in one line.
12. **NEXT 3 says `rerun`:** mark rows `done`, `skip` (the user said skip, or you took a DEFAULT) or `asked: true` (the user said yes), then go to step 8.
13. **NEXT 3 says `mic-drop`:** first one short update: only what changed since the report (new ✅, skipped rows, rows still on the user). Then run `ps:mic-drop` as the last step. It checks, pushes, and archives only when clean.

## Who does it

Try every row yourself first. A row goes to the user only when no tool you have can do it. `kind` in rows.json:

| kind | When | Route |
|---|---|---|
| `work` | code, tests, lint, docs, commits, logs and CI, `SPEED [ME]` fixes, a CLI that is logged in, a dashboard through a browser tool that is logged in | DO |
| `work` + `risk` | `push`, `deploy`, `delete` (stash, branch, files), `paid`, `prod-config` (env vars, project settings). Set `asked: true` when the user asked for exactly this | ASK, or DO when asked |
| `decision` | keep or drop, scope, anything the user's rules do not settle | ASK |
| `value` | a non-secret value no CLI or browser tool can fetch (a bucket name behind a login) | ASK, free text |
| `optional` | `LATER`, nice-to-have; `who: you` when only the user can do it | ASK, or YOU |
| `task` | a running task; `stop: true` when the scan says STOP | ASK, DO with `--kill`, KEEP |
| `login`, `secret`, `payment`, `refused` | the user must type or click it, or a prompt refused it | YOU |
| `no-tool` | you tried and no tool reaches it; `tried` names the tool | YOU |

- `after: [ids]` when a row needs another row first. A push waits on the tests and on the production config it needs.
- A value from another console (a bucket name, a project ID): fetch it with a CLI or a browser tool first.
- For a secret, open the field (browser tool) and say "paste now", or give one `read -rs X && <cmd "$X">; unset X` line. Never ask for the value.
- `SPEED [YOU]` lines are `optional` with `who: you`.

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
| 2 | Merge or PR? | Question below. I pick **PR**. |

### 🏃 Tasks
| Task | Kind | Running for | Last output | Recommend |
|---|---|---|---|---|
| dev server | shell | 2h10m | 90m ago | 🛑 Stop: idle |

✅ Done: reset form in `app/reset.tsx` (commit `a1b2c3d`)
👉 Doing the 🤖 list now. Then one question round.
```

Fill it like this:
- Line 1 is the deliverable. The ETA line is the scan's, copied as is.
- Title counts match the rows. "What's left": 3-8 words, the problem, not the history.
- "What I do" / "Do this": one action. An exact command, a click path, or "Question below". Two steps → two rows. A WAIT row says what it waits on: "after 🧑 1".
- Name tasks by what they do. Task IDs stay out of the report (use them for TaskStop).
- Empty table: one row `| – | Nothing | ✨ |`. No tasks → leave out 🏃 Tasks. Both tables empty: title `## 🧾 What's left: ✨ nothing`, and go straight to `ps:mic-drop`.
- ✅ line: up to 5 items, each with its proof in brackets. DEFAULT and SKIP rows: one `⏭ Skipped:` line. 👉 line: the next move.

## Rules

- No proof, no ✅. Without proof it is a 🤖 row: "Check that X works".
- The user's word is not proof. "The tests passed" → read the output or run the check yourself.
- Never claim a speed gain you did not measure.
- Name secrets. Never write their values.
- "I can't" needs a reason: name the tool you tried, or the rule (login, secret, payment, decision).
