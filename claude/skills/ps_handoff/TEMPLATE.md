# Hand-off template

Fill every section. Delete none. Write for a reader with no memory of this session.

```markdown
# Task
<one line: what the next session must produce>

## State
Goal: <one line, the end state>
Repo: <absolute path; commands run from here>
Branch: <name> at <short sha>, <pushed / not pushed>
Done: <what is finished, with the last commit>
Next: <the one action to take first>
Owner: <what only the user can do, or "none">
Budget: <time and helper cap, for example "2 h, 3 helpers at once, 1 when pace is ahead">
Blockers: <each with the default to take, or "none">

The next session updates these lines after each phase, or the ledger it names.

## Read first
- <absolute path>: <why, in 6 words>
- (3 to 8 files, most important first)

## Facts (verified <YYYY-MM-DD>)
- <fact you checked this session, with the command or file that proves it>
- Pitfalls: <tool missing, flaky test, secret not to touch>

## Why
<2 to 4 lines. The problem and who it is for.>

## What
<2 to 5 lines. The end state, not the steps.>

## How
<the approach in a few lines: the design chosen, the tools and commands to use>

## Rules (hard)
- <scope limits, files not to touch, no commit / no push unless stated>
- <style and tools this repo uses>

## Phases
1. <phase> | model: <plan phase: Opus (app) or opusplan (CLI) / build or review: Sonnet (app) or sonnet (CLI)> | effort: high | <minutes>
   reads: <files or State lines this phase needs> | writes: <files, commits or State lines it changes>
2. <phase> | model: <...> | effort: high | <minutes> | parallel with: <phase numbers, or "none">
   reads: <...> | writes: <...>
3. <phase> | stops for the user: <the step only they can do, with the exact command or click>

## Done when
- MUST: <check someone can run>: <expected result>
- SHOULD: <check that is worth doing, not blocking>
- MAY: <optional extra>

## Report
Hand back at most 3,000 chars: files changed, check output tail, open items.
Write anything longer to a file and cite its path.
```
