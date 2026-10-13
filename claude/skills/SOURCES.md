# Skill sources & provenance

The seeded skills were authored for this repo, informed by patterns from the public
Claude Code / agent-skills ecosystem. They are starting points — adapt freely.

References consulted (June 2026):

- Anthropic official plugins/skills — https://github.com/anthropics/claude-plugins-official
- awesome-claude-skills — https://github.com/travisvn/awesome-claude-skills
- agent-skills open standard — https://agentskills.io
- Claude Code Skills docs — https://docs.claude.com/en/docs/claude-code/skills

The dev-workflow seeds from that sweep (`ps:go-lint`, `ps:readme-writer`) were deleted on
2026-10-12 for lack of use. The rest were written for this setup. Each skill is checked by `check.py` against:

- Claude Code skills reference — https://code.claude.com/docs/en/skills
- Anthropic skill authoring best practices — https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices
- Agent Skills spec — https://agentskills.io/specification
- superpowers `writing-skills` — description = triggers only; recipe over prohibition; test with subagents

## ps:hurry-up (merged into ps:whatsleft on 2026-10-12)

Session speed scanner, now `ps:whatsleft/scripts/hurry-scan.py`. Adapted from these public ideas, rewritten for this setup
(read-only, transcript-based, plain short output, `[ME]`/`[YOU]` split):

- context-budget (ECC) — inventory and flag bloat in agents, skills, rules, MCP — https://claudeskills.info/skills/affaan-m/ecc/context-budget/
- Cozempic diagnose — read the live transcript, show context %, biggest messages — https://openskillindex.com/skills/ruya-ai-cozempic-diagnose
- Token Optimizer — audit setup, find context waste — https://vibeindex.ai/collection/alexgreensh/token-optimizer
- "Making a bloated Claude Code fast again" — measure injected bytes, not file sizes — https://dev.to/bokuwalily/making-a-bloated-claude-code-fast-again-auditing-context-injection-down-from-228kb-to-48kb-1644

## ps:mic-drop

End-of-session wrap-up: finish, prove it with a read-only git/CI scan, park optional
ideas in one leftovers file, stop stray tasks, always archive when clean and unpinned. Built on the same
shape as `ps:hurry-up` and `ps:bag-mode` (script + VERDICT + plain short report), plus ideas from:

- superpowers `verification-before-completion` — no "done" claim without fresh evidence
- superpowers `finishing-a-development-branch` — ask before merging to main

## ps:whatsleft

Mid-session status: what is still open, split into a 🤖 Claude table and a 🧑 user table,
one simple action per row, no ✅ without proof. Read-only. Same shape as `ps:mic-drop`
(script + tagged evidence + VERDICT + fixed report), and its scan reads the live transcript
like `ps:hurry-up`. Ideas from:

- superpowers `verification-before-completion` — no "done" claim without fresh evidence
- superpowers `writing-skills` — fixed output recipe instead of prohibition lists; baseline vs. with-skill subagent runs

## ps:iamrich

Rich mode: the user's word that tokens are no limit, so every phase gets subagents
(`opus`/`fable` at `effort: "max"`) and an independent verifier. Baseline subagent runs
showed the gap it closes: without it, agents spawn nothing ("do not spawn unless asked");
with an ad-hoc "use subagents" line, they never set `effort` (no level named) and leave
`researcher`/`Explore` on their small default model. A real 10-hour session (2026-10-08)
then showed the opposite failure: `max` on every helper plus a review
after every task and "verify until nothing is open" tripled the wall-clock time. So the
default became `high` (with `max` for design, synthesis and the final review), and the skill
now spends tokens on width, not depth. Ideas from:

- superpowers `writing-skills` — required fields over prose reminders; baseline vs. with-skill subagent runs
- superpowers `subagent-driven-development` — fresh agent per task, review between tasks
- `ps:captain-research` — fan out, verify each load-bearing claim with a second agent

## ps:handoff

Hand-off briefs as files (2026-10-12). Test runs, sonnet at high, toy repo with an unpushed branch:
- Baseline: pasted a 40-line brief inline in chat; no file, no pointer.
- With skill: wrote a 49-line brief to `.agents/handoffs/` (git-excluded) and replied with the 3-line block. Gap closed after the run: the launch line now uses the first phase's model.

## ps:autopilot

Run a brief to the end without asking (2026-10-12). Resume interval is 30 min, not 15, because the guard hook denies shorter loops at big context. Test runs, sonnet at high, 3-task brief with "do not ask, continue after the limit":
- Baseline: did the work but made no commits, kept no ledger, armed no resume ("nothing hit a limit").
- With skill: PROGRESS.md first, one commit per task, a 30-min cron armed on turn one, no question, cron deleted and mic-drop at the end. Gap closed after the run: the ledger tick rides in the task commit.

## ps:pick

One artifact of variants with ids and a Copy-my-pick line (2026-10-12). Test runs, sonnet at high, "6 heading fonts I can pick from":
- Baseline: one artifact, but plain numbered cards, no ids, no pick button; "tell me the number".
- With skill: grouped ids A1-B3, knobs for size and weight, Pick this + Copy my pick, reply = link + 3-line legend; the pasted pick was echoed and applied. Gaps closed after the run: a FONTS slot in the template, the label in the pick line, and what to do when the project is not this folder.

## ps:sessions

Find, message, wait for and archive other sessions (2026-10-12). Test runs, sonnet at high, "wait for session X to finish, then rebase and test":
- Baseline: polled the other session with ScheduleWakeup every 270 s.
- With skill: titled the session, one SendMessage with notify_when_idle plus a one-report ask, ended the turn; no wakeup, cron or loop. Gaps closed after the run: how to find your own row, and read the last turn before trusting an idle notice.

## ps:askme light mode (2026-10-12)

Test runs, sonnet at high, "also add a CSV export button /ps:askme" mid-task:
- Baseline: a 7-row decision map and a risk essay before one question.
- With skill: light mode, quick recon, one AskUserQuestion with the 2 real blockers, defaults named in one list.

## ps:iamrich, ps:captain-research and the agents (2026-10-12)

Every Agent call now names model and effort (a hook enforces it); researcher and code-reviewer default to sonnet with a hard hand-back cap. Test runs, sonnet at high:
- iamrich baseline: every agent on opus, no deadline, no hand-back cap, a re-review after each fix. With skill: sonnet high for recon, research, build and review; opus max only for the plan and the final review; deadlines, a get_usage check, one review per batch.
- captain-research baseline: researcher at high but no model, no deadline. With skill: every researcher call has model sonnet, effort high and a 5-minute deadline.

## ps:mic-drop squash mode (2026-10-12)

Test runs, sonnet at high, "archive this" in a repo whose CLAUDE.md wants one commit per session, 3 session commits already pushed:
- Baseline: refused to squash ("I do not force-push"), asked, kept Remote Control on, did not archive.
- With skill: found the 3 session commits, tried reset --soft + one commit + push --force-with-lease, stopped the dev server, skipped checks for "archive this". The force push was denied by the auto-mode classifier, so it kept the session and gave the one-line command. Gaps closed after the run: `--since <sha>` now includes that commit, and a squash not done counts as NOT CLEAN.

## ps:whatsleft + ps:hurry-up merge, ps:bag-mode shared ETA (2026-10-12)

whatsleft now leads with the deliverable, prints an ETA line and a Tasks table with stop advice, and takes inline args (`kill dead tasks`, `/askme`). hurry-up's scan lives on as `scripts/hurry-scan.py` (SPEED rows). bag-mode reads its ETA from `whatsleft-scan.py --eta`. Test runs, sonnet at high:
- whatsleft baseline on "/ps:whatsleft kill dead tasks": killed nothing ("a focus, not a go"), no ETA, no task table, header first.
- whatsleft with skill: `Deliverable:` first, ETA line, Tasks table, the silent watcher stopped in the same turn, the finished build left alone.
- bag-mode baseline: own deadline arithmetic. With skill: ran the shared scan, saw it could not see the stated deadlines, added them. Gap closed after the run: add known phases and deadlines to the scan's ETA and say so.

## desktop-commander in the ps: skills (2026-10-12)

Fit check: desktop-commander wins in 3 skills. autopilot gets `start_process` + `interact_with_process` for commands that ask questions (a prompt hangs the Bash tool). whatsleft and mic-drop get `list_sessions` + `force_terminate`, because its processes are invisible to the scan and to TaskStop. No change elsewhere: `list_processes` returns 266 K chars with no filter (bag-mode keeps `top`); `read_file`, `edit_block`, `start_search` add nothing over Read, Edit and Grep; sessions, handoff, pick, askme, iamrich and captain-research have no process or terminal job. Every use names a fallback, since Cursor and Codex read the same skills. Test runs, sonnet at high:
- autopilot baseline, brief with an interactive `migrate.sh`: Bash runs hung on the prompt, about 8 min lost, one wrong answer before a home-made pty driver. With skill: `start_process`, read each prompt, answered with `interact_with_process`, done in 1 min, right answers first time. Gap closed after the run: the ToolSearch query now names the tools, `+desktop-commander` alone did not load `start_process`.
- whatsleft baseline, "/ps:whatsleft kill dead tasks" with two desktop-commander servers running: the scan did not see them, nothing stopped. With skill: `list_sessions`, both rows in Tasks, both stopped with `force_terminate`. Gap closed after the run: only PIDs this session's own `start_process` returned count as ours.
- mic-drop baseline, "clean up background tasks" with the same servers: found and killed them, but after 7 `ps` and `lsof` calls. With skill: one `list_sessions`, one `force_terminate`, the background `sleep` stopped with TaskStop. Gap closed: the archive sweep is scoped to PIDs this session started.
- whatsleft scan fix (same day): hurry-scan guessed a 200K window for any session under 200K, so a 1M Opus session at 197K read "99% full". It now picks 1M for Opus, Sonnet and Fable 5.x. Checked by calling `guess_window` per model and on this session (203K of 1M, 20%).

## ps:sessions example rows (2026-10-13)

The hub-report example rows now use neutral names. Test runs, sonnet at high, "give me a hub view" with three fake sessions (one waits on the user, one idle, one busy):
- Baseline: right table, own row left out, the idle session listed. It flagged that "one row per session" is unclear about idle ones and that the example wording pulls.
- After the rename: same table, no example names copied, but the idle session dropped to a footnote. Gap closed: "one row per open session, busy or idle, except your own" and "the rows below show the format only". Re-run: all three sessions as rows, own row out, only the waiting one says "Yes".

## ps:mic-drop process lines (2026-10-13)

The scan's process list printed whole command lines, and one held a real API key. Now each process gets one short line: PID, program name (`ucomm`) and at most 100 masked characters (flag values, secret-named KEY=VALUE pairs, URL passwords, known token prefixes, long opaque strings). Test runs, sonnet at high, "run the scan and report" in a test repo with two fake processes whose command lines held fake keys:
- Baseline: the scan printed all 4 fake secrets. The agent left them out of its report, then ran `ps` with `comm` itself, which prints them again (on macOS `comm` is argv[0], which a process can set).
- With the fix: the scan showed no secret, but the agent ran `ps -o command` to judge ownership. Gap closed: the skill row now names a safe `/bin/ps -o pid=,ppid=,user=,lstart=,ucomm=` call and forbids printing full command lines. Re-run: only that call, no command line printed, same decision (not ours, leave).
