# Hand-off template

Fill every section. Delete none. Write for a reader with no memory of this session.

```markdown
# Task
<one line: what the next session must produce>

## Where
- Repo: <absolute path>
- Branch: <name> at <short sha>, <pushed / not pushed>
- Run commands from: <dir>

## Read first
- <absolute path>: <why, in 6 words>
- (3 to 8 files, most important first)

## Facts (verified <YYYY-MM-DD>)
- <fact you checked this session, with the command or file that proves it>
- Done: <what is finished>
- Open: <what is not>
- Pitfalls: <tool missing, flaky test, secret not to touch>

## State
- Ledger: <path the next session updates after each phase, or "none">
- Status: <one line: where the work stands>
- Last commit: <short sha and subject, or "none">
- Next step: <the one action to take first>
- Open decisions: <each with the default to take, or "none">

## Goal
<2 to 5 lines. The end state, not the steps.>

## Rules (hard)
- <scope limits, files not to touch, no commit / no push unless stated>
- <style and tools this repo uses>

## Phases
1. <phase> | model: <sonnet / opus> | effort: <high> | <minutes>
   reads: <files or State fields this phase needs> | writes: <files, commits or State fields it changes>
2. <phase> | model: <...> | effort: <...> | <minutes>
   reads: <...> | writes: <...>

## Done when
- <check someone can run>: <expected result>

## Report
Hand back at most 3,000 chars: files changed, check output tail, open items.
Write anything longer to a file and cite its path.
```
