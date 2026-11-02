# Harness map

What every repo on a machine with these dotfiles inherits, and how one repo adds its own agents on top.
Nothing loads this file. Read it when you build or change a repo's agents or its agent map.

## What every repo inherits

| Piece | Source | Job | When it acts |
|---|---|---|---|
| Shared agents | `claude/agents/*.md`, linked to `~/.claude/agents` | build a plan task, review, scan, research, find a session, check a page, any other job | the main thread dispatches them by name (`workflow` preset, "Agents") |
| Skills | `claude/skills/ps_*` | hand-offs, wrap-up, autopilot, research, maps, reviews | their descriptions; the guard's H2 hint maps the user's words to a skill |
| Hub | `claude/skills/ps_hub` | `ps:hub`: one session in `~/dotfiles` splits a task list into one Sonnet session per task, each in its own worktree, and tracks them | `/ps:hub` or "run these in parallel across projects" |
| Guard | `rust/agent-config/src/bin/guard.rs`, wired in `claude/settings.json` | brief shape (G1), hand-back cap (G5), no general-purpose (G7), helper cap (G8), big reads (G20) and more | every tool call; the full table is in `claude/hooks/README.md` |
| Router | the guard's `route_agent` | sets model and effort on each main-thread Agent call | every spawn |
| Format hook | `rust-bin proj format` | formats each edited file with the repo's own formatter | every Edit and Write |
| Presets | `claude/presets/*/rules.md` | workflow, models, pace, clock, plain writing, ship to main, no AI attribution | loaded per the `PRESETS` setting |
| Project contract | `rust-bin proj detect` | the repo's `check`, `check:fast` and `fix` commands | hooks and agents run them |

## How a repo layers on it

1. **Project agents** live in `<repo>/.claude/agents/<name>.md`. A project agent beats a shared one with the same
   name, so a repo can replace `task-implementer` with its own. Follow the house rules in `claude/agents/README.md`.
2. **Pins.** Pin `model:`, `effort:` (not for Haiku) and `maxTurns` in every agent file. The router keeps a pin when
   it is not dearer than its table: a Haiku agent stays on Haiku, an Opus pin becomes Sonnet. Only `Plan` and the
   final `code-reviewer` run on Opus. A critical week spawns no helpers at all.
3. **One agent map:** `<repo>/.claude/AGENT-MAP.md`, under 60 lines, in the shape below. It holds the dispatch order
   for the repo's common jobs and the practice Claude cannot find in the code. No folder tour: repo overviews add
   cost and do not help (arXiv 2602.11988).
4. **One pointer line** in the repo's root `CLAUDE.md`: "**Agents:** before you spawn a helper here, read
   `.claude/AGENT-MAP.md`". Never `@`-import the map: every helper loads the whole `CLAUDE.md`, and only the main
   thread needs the map.
5. **Area rules** go in `.claude/rules/<topic>.md` with `paths:`. They load when a matching file is touched, and
   they drop out at a compaction.
6. **CLAUDE.md** keeps only what Claude cannot find in the repo: owner rulings, gotchas and odd commands (the prune
   test in `ps:harness-optimize`).

## Agent map shape

```markdown
# Agent map: <repo>

<!-- For the main thread, before it spawns a helper here. Under 60 lines. Shared side: the dotfiles repo's `claude/HARNESS-MAP.md` -->

## Jobs
| Job | Dispatch, in order | Check |
|---|---|---|
| <a job this repo repeats> | <agent> -> <agent> | `<command>` |

## Agents here
| Agent | Job and trigger | Tools | Model | Owns |
|---|---|---|---|---|
| <name> | <one line> | <list> | <tier> | <paths it may write, or "none"> |

## Skills, rules and hooks
- <skill>: <when>.
- Rules: <one line, or one per rule that a job row needs>.
- Hooks: <project hooks, one line each>.

## Rules
- <practice Claude cannot find in the code, one line each>.
```

## How to measure

- `rust-bin harness_stats agents --cut <ISO>`: spawns per agent type and their start context. A project agent that
  never runs is a cut candidate.
- `rust-bin harness_stats report --by project`: start and peak context, compactions, questions and stops per project.
- A/B mode of `ps:harness-optimize`: `ab.sh --dirs <with-map> <without-map> --prompt "..." --expect '<regex>'
  --runs 3`. Read the answers too. A map that costs more and wins nothing is cut.
