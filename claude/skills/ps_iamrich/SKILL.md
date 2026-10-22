---
name: ps:iamrich
description: Use when the user runs /ps:iamrich, puts "iamrich" before or after a request, or says the session has plenty of tokens and wants parallel subagents for thorough research, implementation, or verification.
argument-hint: "[task — empty = the message it came with, or your last one]"
---

# ps:iamrich — spend tokens on quality, not delay

The user wants thorough work from helpers. Their wall-clock time and the weekly pace both count. The speed rules in CLAUDE.md still apply.

**This skill is the user's explicit request to use subagents** for this session. It stays on for later requests until the user says to save tokens.

## The task

1. The text passed with the command: `$ARGUMENTS`.
2. Empty? The message the command came with (text before or after `/ps:iamrich`).
3. Still empty? The user's previous message.
4. Nothing at all? Reply in one line: rich mode is on.

## Fan-out gate

A multi-agent run costs about 15 times a plain chat. Before each fan-out, answer four questions:

1. Are there 2 or more parts that need no other part's output?
2. Must the run pause for a person between steps?
3. Must it recover mid-run (long, may die, resume from a checkpoint)?
4. Must each step leave a record (a ledger, a per-part commit)?

All four no: do the work in this window, then send the one verifier. Only a yes to 1 buys
parallel builders, one per independent part. Two parts that touch the same files are one part.

## Every Agent call

```
subagent_type: worker | task-implementer | task-reviewer | code-reviewer | researcher | Explore | Plan
description:   3-5 words
prompt:        task + context + deadline ("done in N min") + "hand back ≤ 4,000 chars; write the rest to a file and cite it" (read-only agents: "stay ≤ 4,000 chars")
```

The `superlite:*` agents are outside this section and the worktree rule: superlite's own skill writes
their calls, and the guard leaves them alone.

Do not pass `model` or `effort`: the guard router sets them (Plan and the final review on
Opus, lookups on Haiku, the rest on Sonnet `high`). Never `general-purpose`: the guard refuses it.
An agent that edits files also gets these two lines in its prompt: "Read each file with Read
before you Edit it; `cat` in Bash is not a Read." and "The shell tool is `Bash`, with a capital B."

## Pace

Before each fan-out, when `~/.claude/hooks/rust-bin` exists, run `~/.claude/hooks/rust-bin pace status`.
Helpers at once: 3 when `ok` or `unknown`, 1 when `ahead`, 0 when `critical` (do the work
yourself). Send a larger fan-out in waves of that size. The guard enforces the cap.

## Width, not wait

- Run independent parts at the same time (up to the Pace cap), each in its own worktree.
- Say a wall-clock estimate before starting and after each phase.
- Size every check to the risk: measure what changed, not everything.

## Each phase the task has

| Phase | What to run |
|---|---|
| Recon | Parallel agents in one message, one question each: `Explore` for code, `researcher` for the web. Read the key files yourself too. |
| Research | `researcher` only, never `general-purpose`. Follow `ps:captain-research`. |
| Plan | A `Plan` agent designs while you draft. Merge the best of both. |
| Implement | `task-implementer` (a plan task with tests) or `worker`, with `isolation: "worktree"`, one per independent part. Batch 3-6 small tasks per implementer. A multi-step plan with tests: write it with superlite:writing-plans and run it with superlite:subagent-driven-development instead. That skill spawns its own `superlite:*` agents (no worktree flag, no batching rule from this row); pass its `sl dispatch` tool `--cap 10` (3 when the week runs ahead); these agents have their own guard pool, so up to 3 normal helpers (a verifier) can run beside them. superlite gives each task its own files. |
| Verify | Agents that did not write the change. One proves the behavior (tests, a reproduction). Per-batch reviews run on sonnet; with one batch, skip them. One fix round, then the final review: one `code-reviewer` on the full diff (the router runs it on Opus). Anything still open goes to the user. |

The floor, even for a one-line change: one independent verifier before you say done.

## What rich mode does not change

- Irreversible or outward actions (delete, push, publish, install, send, pay) get the usual confirmation.
- Ask the user only what no agent can find: secrets, logins, taste, decisions.
- The final report keeps its usual length. End it with one line:

```
Spent: <agents by role, e.g. 3 research · 1 implement · 2 verify> · Proof: <what an independent agent checked, and the result>
```
