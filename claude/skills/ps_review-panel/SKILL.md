---
name: ps:review-panel
description: "Use when a branch or a large diff needs its final review before merge and the user wants it thorough, from several angles, a review panel, or one merged report; also when a plan or ps:iamrich reaches the final review of a big change. Not for one plan task (task-reviewer) or a small diff (one code-reviewer)."
argument-hint: "[base..head or branch] [repo path]"
---

# Review panel

Several review angles run at once, then one merged report. Each angle is an agent with one lens.
The merge waits for all of them (a barrier). This buys quality, not tokens: a panel is 4 agent runs.
The guard router sets model and effort: `panel: <angle>` calls run on Sonnet, a `final review:` call runs on Opus.

## Gate

- Under about 300 changed lines and 5 files, and no risky area (auth, payments, data migration,
  security): send ONE `code-reviewer`, description `final review: <slug>`. Stop here.
- Else run the panel.

## Pace

When `~/.claude/hooks/pace.py` exists, run `python3 ~/.claude/hooks/pace.py status`. Agents at once:
3 when `ok` or `unknown`, 1 when `ahead`, 0 when `critical` (send nothing: run the repo's checks
yourself, review the diff yourself, and say the week is spent). The guard enforces the cap.

## 1. Pin the change (you)

- Find the range (`<base>..<head>`), `git diff --stat <range>`, and the repo's test, typecheck and
  lint commands from its `CLAUDE.md`, `package.json` or `Makefile`.
- Do not paste the diff into the briefs. Each agent runs `git diff <range>` itself.

## 2. Fan out in waves

Wave 1 is the three Sonnet angles, in one message (one at a time when pace is `ahead`). The last
agent is the judge: the `correctness` final review (Opus by the router). Send it after the angles
return, with their findings in its brief. Do not merge until every agent has handed back.
Never merge partial results.

| Angle | subagent_type | Description | Lens |
|---|---|---|---|
| verify | `worker` | `panel: verify` | run the tests, typecheck, lint and build; reproduce the main behaviour once. Change nothing. |
| simplify | `code-reviewer` | `panel: simplify` | duplicate or dead code, an existing helper not reused, parts built bigger than the task needs. |
| design | `code-reviewer` | `panel: design` | fit with the repo's rules and conventions, docs and tests updated, names, rollout and migration safety. |
| correctness | `code-reviewer` | `final review: <slug>` | bugs, security, broken contracts, a rule one commit adds and another breaks. Gets the other angles' findings and confirms or drops them. |

One risky area that dominates the change (a payment webhook, an auth flow) may get a fifth agent:
`code-reviewer`, description `panel: <area>`, that area only. Never more than 5.

Each brief holds: the repo path, the range, the angle and its lens, the check commands, and:

```
Report each finding as: [blocking|minor] file:line: <defect>. Fix: <concrete fix>. (score 0-100)
Drop findings under 80. Change nothing. Deadline: 15 min. Hand back at most 3,000 chars.
```

Reviewers have no Write tool. Never ask them for a notes file. The verify brief adds: "Line 1
`PASS` or `FAIL`. Quote each command with its real last line."

## 3. Merge (you)

- One finding is one `file:line` plus one defect. Two angles that name the same defect within 5
  lines of each other give one finding with both angle names.
- Found by two angles: keep it. Found by one angle and blocking: Read the cited lines yourself,
  then confirm it or drop it with the reason. Found by one angle and minor: keep it as minor.
- A failing check from verify is a blocking finding.
- Two angles that disagree (simplify says remove, design says keep) go under Conflicts with both
  sides. Never pick a winner silently.
- Write the full merged report to `.agents/reviews/<YYYY-MM-DD>-<slug>.md` when `.agents/` is
  git-ignored, else to the scratchpad.

## 4. Report

```
Verdict: APPROVED | CHANGES NEEDED | BLOCKED: <why>
Checks: <command> -> <real last line>   (from verify, one line each)
Blocking (<n>):
1. file:line: <defect>. Fix: <fix>. [angles]
Minor (<n>, top 5):
Conflicts: <or "none">
Not checked: <what no angle covered>
Full report: <path>
Spent: <n> agents (<n> opus)
```

## Fix round

One fix round, then a re-review of the blocking findings only: one `code-reviewer`,
description `re-review: <slug>` (Sonnet), with the findings and the fix range. Not the full panel again. Anything still open goes to the user.

## Mistakes

| Mistake | Do instead |
|---|---|
| Angles split by area (UI, API, DB) | Split by lens. Area splits miss bugs that cross areas. |
| Nobody runs the tests | The verify angle runs them and quotes real output. |
| Merging as results arrive | Wait for every angle, then merge once. |
| Pasting a 900-line diff into 4 briefs | Give the range; each agent reads the diff itself. |
| A panel for a 40-line change | The gate: one `final review:` `code-reviewer`. |
| More than 3 agents at once | The Pace cap. Send the judge after the angles. |
| A second full panel after the fixes | Re-review the blocking findings only. |
