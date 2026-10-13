---
name: ps:iamrich
description: Use when the user runs /ps:iamrich, puts "iamrich" before or after a request, or says the session has plenty of tokens and wants many parallel subagents for thorough research, implementation, or verification.
argument-hint: "[task — empty = the message it came with, or your last one]"
---

# ps:iamrich — spend tokens, buy quality, not delay

The user has a large token budget. Cost is not a limit. Their wall-clock time is: tokens buy more helpers at once, never slower ones. The speed rules in CLAUDE.md still apply.

**This skill is the user's explicit request to use subagents** for this session. It stays on for later requests until the user says to save tokens.

## The task

1. The text passed with the command.
2. Empty? The message the command came with (text before or after `/ps:iamrich`).
3. Still empty? The user's previous message.
4. Nothing at all? Reply in one line: rich mode is on.

## Every Agent call

Fill every field. Agent definitions default to a small model; these replace that.

```
subagent_type: researcher | Explore | Plan | general-purpose | code-reviewer
model:         sonnet (implement, review, research) | opus or fable (plan, final review)
effort:        high   (max only for plan, research synthesis, architecture, final review; say which)
description:   3-5 words
prompt:        task + context + deadline ("done in N min") + "hand back ≤ 4,000 chars; write the rest to a file and cite it" (read-only agents: "stay ≤ 4,000 chars")
```

Before each fan-out, call `mcp__ccd_session_mgmt__get_usage`. Under 30% of the 5-hour window left: dispatch at most 3 agents.

## Width, not wait

- Run independent parts at the same time, each in its own worktree. No builder waits on another helper.
- Say a wall-clock estimate before starting and after each phase.
- Size every check to the risk: measure what changed, not everything.

## Each phase the task has

| Phase | What to run |
|---|---|
| Recon | Parallel agents in one message, one question each: `Explore` for code, `researcher` for the web. Read the key files yourself too. |
| Research | `researcher` only, never `general-purpose`. Follow `ps:captain-research`. |
| Plan | A `Plan` agent on opus or fable designs while you draft. Merge the best of both. |
| Implement | `general-purpose` with `isolation: "worktree"`, one per independent part. Batch 3-6 small tasks per implementer. REQUIRED SUB-SKILL: superpowers:subagent-driven-development for multi-step plans. |
| Verify | One reviewer per batch, not per task or per fix. Agents that did not write the change. One proves the behavior (tests, a reproduction). One `code-reviewer` gets the full diff pasted in. One fix round, then the final review (opus or fable, max); anything still open goes to the user. |

The floor, even for a one-line change: one independent verifier before you say done.

## What rich mode does not change

- Irreversible or outward actions (delete, push, publish, install, send, pay) get the usual confirmation.
- Ask the user only what no agent can find: secrets, logins, taste, decisions.
- The final report keeps its usual length. End it with one line:

```
Spent: <agents by role, e.g. 3 research · 1 implement · 2 verify> · Proof: <what an independent agent checked, and the result>
```
