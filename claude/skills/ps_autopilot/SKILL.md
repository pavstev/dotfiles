---
name: ps:autopilot
description: "Runs a saved or pasted brief to completion without asking: keeps a PROGRESS ledger, commits per task, re-arms itself after compaction, app restarts or usage-limit resets, and reports one line per hour. Use when the user says execute it exactly start to finish, do not ask me anything, overnight, do not stop until fully completed, or continue when the limit resets."
argument-hint: "[path to the brief, or paste the brief]"
---

# Autopilot

Input: $ARGUMENTS

Run the brief to the end while the user is away. The ledger is the memory. Commits are the proof. No turn ends with a question.

## Core loop

1. **Resume check first.** If the prompt says "continued from a previous conversation" or "app was quit", or a limit just reset, read `PROGRESS.md`, run `~/.claude/hooks/rust-bin pace status` when `~/.claude/hooks/rust-bin` exists, check the `Budget:` line (step 9) and the Push and deploy log (step 10), then jump to step 5. Do not re-plan. At `critical`: set `Pace: critical` in the `State:` block, log it, and end the turn so the loop waits for the weekly reset. Do this on every loop turn too, without relying on the guard's note.
2. **Write the ledger now.** Copy [the template](PROGRESS.template.md) to `PROGRESS.md` in the repo root. Fill the `State:` block (`Key: Value` lines), one row per task and the resume command. Commit it.
3. **Arm the resume on turn one.** Before any task work, start `/loop 30m` (or one CronCreate every 30 min) with the prompt "Read PROGRESS.md and continue the next open task". Use 30 min, not shorter: a hook denies shorter polls at large context. Do it even if no limit has hit yet.
4. **Decide, never ask.** A blocker becomes a default. Pick the safest option, take it, and log it in the blockers table with the default taken. Stop only for an irreversible or paid action, and name it in the ledger.
5. **Per task:** implement, run the repo's check, tick the row, commit both in one commit per task (even if the brief did not ask), update `Next`. Then start the next task in the same turn.
   Planning goes to a `Plan` helper (the guard runs it on Opus). Never call EnterPlanMode: it stops the run for approval.
   After 2 failed tries on one step, stop retrying: set the row to blocked, log the failure and the default taken, and move to the next task. Never skip or weaken a test to get green.
   A command that asks questions (y/N, a REPL): run it in tmux on its own socket: `tmux -L claude-run new-session -d -s <name> '<cmd>; echo EXIT $?; exec sleep 86400'`, so the pane stays after the command ends. Read the screen with `tmux -L claude-run capture-pane -p -t <name>`. Answer one prompt at a time with `tmux -L claude-run send-keys -t <name> '<answer>' Enter`. Log each prompt and answer in the ledger. When the screen shows `EXIT <n>`, log the code, then run `tmux -L claude-run kill-session -t <name>`. Never send answers you have not read a prompt for. No tmux → use the command's non-interactive flag or env (`--yes`, `CI=1`); none → blocker row, default taken.
6. **Before each fan-out,** run `~/.claude/hooks/rust-bin pace status` when `~/.claude/hooks/rust-bin` exists. Helpers at once: 3 when `ok` or `unknown`, 1 when `ahead`, 0 when `critical`. Send a larger fan-out in waves of that size. Every Agent call carries a deadline and the hand-back cap; the guard sets model and effort and enforces the cap.
   Obey the guard's pace notes. `ahead`: 1 helper at once, no plan mode. `critical`: no new helpers; finish the current task, commit it, log it in `State:`, and end the turn quietly. The resume loop waits for the weekly reset.
7. **Hourly:** append one status line to the ledger and print the same line in chat. Format: `HH:MM · done n/N · now <task> · next <task> · blockers n`.
8. **Limit hit:** write the resume time in the `State:` block and the limit log, end the turn quietly. The loop picks it up after the reset.
9. **Budget.** `Budget:` in the `State:` block holds the stop limit: the brief's own limit (`4h`, `6 tasks`, `3 commits`; write all of them, the first one reached wins), else `pace` (the weekly pace status). Write it in step 2 together with `Used:`. Check it at every resume, before each task and before each fan-out. With `pace`, stop when `pace status` says `critical` or `used` reaches `budget`. With a brief limit, stop when the clock or the count reaches it. Stopping means: finish and commit the task in hand, set `Budget: spent` and the reason in the `State:` block, delete the loop, print the final report, and end the turn. Open rows stay `todo`; do not run `/ps:mic-drop` as if done.
10. **Push and deploy log.** Before each push or deploy, add a row to the Push and deploy table in the ledger: `<task #> · <what> · <sha> · started`, and commit it. Run the push or deploy. Then set the row to `done` with the proof (remote sha, deploy id or URL) and commit. On resume, read that table first. A `done` row is never run again. A `started` row means unknown: check the remote (`git fetch`, `git ls-remote`) or the deploy status, then set the row to `done` with proof, or run it once and set the row to `done`. No row for the task: nothing ran. Add the row, then run it once. Never push or deploy twice for one row.
11. **Finish:** all rows ticked, checks green, everything pushed. Delete the loop or cron. Run `/ps:mic-drop`.

## Red flags

These thoughts mean you are skipping a step.

| Excuse | Do instead |
|---|---|
| "The brief didn't ask for commits" | Commit per task anyway. The ledger row names the commit subject. |
| "Nothing hit a limit yet, so nothing to schedule" | Arm the 30 min resume now. A limit gives no warning. |
| "I'll ask just this once" | Pick the default, log it, keep going. |
| "I'll remember where I was" | You will not after compaction. Update the ledger after every task. |
| "A ledger is overhead for 3 tasks" | Write it. A restart costs more than the file. |
| "I will wrap up when it feels done" | Done means every row ticked and `/ps:mic-drop` clean. |
| "The push probably ran before the restart" | Read the log row. `started` means check the remote, then record `done`. Never guess, never repeat. |
| "Checking pace costs a call" | Check before fan-out and on every resume. Spawning blind is what trips the cap. |
| "I can poll every 5 minutes" | Hooks deny it. Use 30 min or more, or wait for a limit reset. |

## Rules

- First action after any restart or compaction is reading `PROGRESS.md`.
- The last line of every turn is a ledger update or a final report, never a question.
- Plain short sentences in the ledger. No em dashes.
