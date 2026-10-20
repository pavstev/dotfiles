---
name: ps:standup
description: "Writes a short standup (Yesterday, Today, Blockers) from git, the gh CLI and any connected tracker, grouped by ticket key. Use when the user asks for a standup, a daily update, what they did yesterday, or what is on their plate today."
argument-hint: "[repo paths or ticket tracker]"
---

# Standup

Reads facts from the user's own work and writes at most 12 lines. A section with no source data
says `no data`. Never fill a gap with a guess.

## Steps

1. Pick the repos: the ones the user named or this session worked in, else the current one.
2. Set the window. Yesterday is the last working day (Friday when today is Monday). Take the
   date from `date`, not from memory.
3. Collect, in this order. One failed source is `unavailable: <why>`, not an empty list.
   - Commits: run `git config user.email` first, then `git log --all --since="<day> 00:00" --until="<today> 00:00" --author=<that email> --format='%h %D %s'` in each repo (`%D` shows the branch).
   - Pull requests: `gh pr list --author @me --state all --limit 20` (merged or opened in the window; open ones are Today).
   - Issues: `gh issue list --assignee @me` (open ones are Today).
   - A connected tracker (Linear, Jira, ...): assigned items and their status, when a tool is loaded.
   - An empty `gh` list counts as empty only when `gh` exited 0 in a repo with a GitHub remote. Otherwise it is `unavailable`.
4. Group by ticket key: the first match of `[A-Z][A-Z0-9]+-\d+` in a commit subject, branch name,
   PR title or issue title. Items with no key go under `No ticket`.
5. Write the standup in the shape below. Join commits of one key into one line. With many
   commits and no key, write one line per theme, using words from the subjects.

## Output shape

```
Yesterday
- ABC-123: <what changed, one line>
- No ticket: <one line>
Today
- ABC-124: <open PR or assigned item>      (or one line: Today: no data (<why>))
Blockers
- <a PR waiting for review, a failing check, a blocked item> | none found in the data read
Sources: <what was read, with counts>; <what was unavailable>
```

## Rules

- Today holds only open PRs, assigned issues and tracker items. With none, write `Today: no data
  (open PRs, issues and tracker empty or unavailable)`. Do not guess it from commit titles.
- Blockers come from the data: a failing check, a PR with requested changes, a blocked status.
  With none, write `none found in the data read`.
- Use the commit text. Do not add outcomes the commits do not show.
- Keep the whole text at 12 lines or fewer. Drop the lowest-signal lines first (formatting,
  merge commits). The Sources line counts.
- Never print an email address or a token. Plain sentences, no em dashes.
