---
name: ps:harness-optimize
description: Use when the user asks whether their Claude Code harness, skills, hooks or settings changes helped, wants session analytics or a before/after comparison of sessions, asks how to optimize or trim the harness globally or per project, or runs the periodic harness review.
argument-hint: "[since date | project name]"
allowed-tools: "Bash(${CLAUDE_SKILL_DIR}/../../hooks/rust-bin harness_stats *)"
---

# Harness optimize

Measure sessions from local transcripts, split them by harness change and by project, then propose
small changes with evidence. The numbers come from the script, never from eyeballing transcripts.

## Steps

1. **Collect.** `${CLAUDE_SKILL_DIR}/../../hooks/rust-bin harness_stats collect --since <date> --exclude <this session id>`. Every `harness_stats.py <cmd>` below runs the same way: `${CLAUDE_SKILL_DIR}/../../hooks/rust-bin harness_stats <cmd>`.
   Default `--since`: the date of the newest `~/.claude/research/*-harness-review.md`, else 14 days ago.
   It reads main transcripts AND `subagents/*.jsonl` (most tool errors live there) and the desktop
   app's records (title, archived). Default window: since the last review, or 14 days.
2. **Find the change points.** `harness_stats.py cuts --since <date>` lists push groups (commits to
   `claude/` less than 3 h apart) and prints where the repo history starts. If that start is inside the
   window, find the older repo with `gh repo list --limit 50` and run
   `harness_stats.py cuts --gh OWNER/REPO --since <date>`. Use the first time of each group that
   changes settings, hooks, CLAUDE.md or several skills as a cut; skip docs-only groups.
3. **Compare periods.** `harness_stats.py report --cut <ISO> [<ISO> ...]`. Read the metrics in this
   order: peak > 500K, start context, questions/session, stops/100 prompts, tool errors/100 calls,
   subagent effort %, hand-off starts. A period under 20 sessions is "early", say so.
4. **Compare projects.** `harness_stats.py report --by project`, `harness_stats.py errors --top 8`,
   then `harness_stats.py errors --daily` to see when each cluster started or stopped. A cluster that
   starts on a cut date may be that push or a Claude Code update: compare the "Claude Code versions"
   row of the periods. One that already stopped needs only a guard.
   Read 2 or 3 raw examples of each cluster you act on:
   `rg -l -F '<cluster text>' ~/.claude/projects --glob '!*<this session id>*'`. A regex hit is not proof.
5. **Agent dispatch.** `harness_stats.py agents --cut <ISO>` lists subagent spawns per agent type and
   period, with share and median start context. Watch the general-purpose share (it should fall after
   a dispatch change) and any type that starts near 50K or more (it has no `tools:` list).
6. **Read the harness you judge.** `harness_stats.py inventory` gives each project's CLAUDE.md lines,
   rules (path-scoped / all), agents, skills and hooks. Then read the global `~/.claude/CLAUDE.md`,
   `~/.claude/settings.json` and the files behind each proposal.
7. **Skills.** `harness_stats.py skills` lists calls per skill and installed skills never called.
8. **Write the review** to `~/.claude/research/<YYYY-MM-DD>-harness-review.md` in the shape below, and
   keep the previous reviews: the next run compares against them.
9. **Apply nothing yet.** Show the proposals. Apply only the ones the user picks, one commit each,
   in the repo that owns the file. Skill edits follow the skill TDD rule.

## Where a fix belongs

| Evidence is | Put the fix in |
|---|---|
| the same in every project (tool misuse, polling, question count) | global `~/.claude/CLAUDE.md` rule or the guard hook |
| one project's commands, gotchas, error cluster | that repo's `CLAUDE.md` (keep under 200 lines) |
| one folder of a repo | `.claude/rules/<topic>.md` with `paths:` frontmatter |
| a project subagent failing the same way | that agent's `.claude/agents/<name>.md` prompt |
| something that must always happen (format, test, block) | a hook: global for behaviour, project for language tooling |
| a procedure used only sometimes | a skill, not CLAUDE.md |

## Review shape

```markdown
# Harness review <date> (<N> sessions, <window>)
Cuts: <ISO + commit subject per cut>
## Verdict
<2-3 sentences: what got better, what did not, sample size>
## Metrics
<the period table, then the project table>
## Proposals
| # | Scope (global / project:file) | Change | Evidence (count, sessions) | Metric it should move |
## Not measured
<what the data cannot show>
```

## Common mistakes

| Mistake | Fix |
|---|---|
| Skipping `subagents/` | Most worktree, Edit and tool-name errors are in subagent files |
| "I cannot see what changed" | The old history is in an archived repo; step 2 |
| Keyword frustration counts | "do not stop until" is an instruction. Use the script's corrections count and read the hits |
| One big rewrite | One change per proposal, each tied to a metric |
| Judging a 1-day period | The report marks periods under 20 sessions early; re-run next week |
| Per-session rates after hand-offs | Hand-offs make sessions shorter; compare per-100-prompt rates too |

Transcripts older than `cleanupPeriodDays` are deleted. If the setting is under 180, propose raising it.
Built-in `/insights` adds friction themes; `/fewer-permission-prompts` adds allowlist rules.
