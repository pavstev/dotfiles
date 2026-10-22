---
name: ps:handoff
description: "Writes a self-contained brief for another Claude, Sonnet or OpenCode session, saves it to a file and replies with a short code block that points at it. Use when the user asks for a prompt for another session, a hand-off, a prompt in a markdown code block, to pack this session's work for someone else, or when context passes 300 K."
argument-hint: "[target: claude | sonnet | opencode | cowork] [topic]"
allowed-tools: "Bash(bash ${CLAUDE_SKILL_DIR}/scripts/handoff-path.sh *)"
---

# Handoff

Input: $ARGUMENTS

The brief goes in a file. The reply is a one-line code block that points at the file, plus the folder, model and effort outside the block.
Work alone, no subagents. Finish in 10 minutes.

## Steps

1. **Pick the target.** Take it from the argument or the request. Default is Claude Code. Targets: Claude Code, Sonnet, OpenCode, Cowork. The model is the one the user named, else pick by where the session starts:
   - Desktop app (the default): the picker has fixed models and no `opusplan`. Say `Opus` when the first phase plans or designs, else `Sonnet`.
   - Terminal CLI (only when the user says terminal or CLI): `opusplan` when the first phase plans, else `sonnet`.
2. **Gather facts.** Check them now, do not recall them.
   - `git rev-parse --show-toplevel`, branch, `git status -sb`, `git log -5 --oneline`, unpushed commits.
   - What is done, what is open, what failed or is missing (a tool not installed, a TODO left uncommitted).
   - The 3 to 8 files the next session must read, as absolute paths.
3. **Name the file.** Pick a slug of 2 to 4 words. Run `bash ${CLAUDE_SKILL_DIR}/scripts/handoff-path.sh <slug>`. It prints the path and creates the folder. In a git repo it also keeps the folder out of git. Outside a repo it uses `~/.claude/handoffs/`.
4. **Write the brief** to that path with the Write tool. Use [TEMPLATE.md](TEMPLATE.md). Rules for the brief:
   - Up to 150 lines. Absolute paths. No "as discussed" and no pointer to this chat.
   - Every fact says how it was checked. Date it.
   - `## Read first` lists the exact files (at most 8) the new session opens before anything else. `## Checked facts` lists what you verified, each with the command or file that proved it. The new session trusts both and does not re-explore.
   - `## State` is `Key: Value` lines (Goal, Repo, Branch, Done, Next, Owner, Budget, Blockers). The next session updates them, or the ledger it names, after each phase.
   - The approach sits in `## Why`, `## What`, `## How`. Acceptance lines in `## Done when` start with MUST, SHOULD or MAY.
   - Every phase names a model and an effort. A phase that plans or designs says `Opus` (app) or `opusplan` (CLI), as in step 1. Build and review phases say `Sonnet` (app) or `sonnet` (CLI). Effort is `high`, never `xhigh` or `max`. Never give a build or review phase Opus: a final review phase says Sonnet too, and the guard runs the `code-reviewer` on Opus.
   - A plan says which phases run in parallel (only independent ones, at most 3 at once) and which stop for the user (a secret to paste, a login, a paid or irreversible step), with the exact command or click.
   - Every phase names what it reads and what it writes. A later phase reads what an earlier one wrote, not the whole "Read first" list again. A phase that only reads is not a phase: fold it into the next one.
   - `## Report` caps the answer at 3,000 chars and sends the rest to a file.
5. **Reply** in the shape below. Nothing else.

## Reply shape

The reply starts with a markdown link to the brief (the file name as text, its path as the target), then one fenced block that holds only the paste line, then a short "Start with" section outside the block. Never archive in the same turn as the reply: the user must see it first. A Stop hook (guard G6) checks both. Never put `cd`, `claude --model ...`, `opencode ...` or any launch command in the block: the user starts the session in the app.

Brief: <markdown link: file name, target path>

```
Read <path> and do it.
```

Start with:
- Folder: <repo>
- Model: <Opus | Sonnet in the app; opusplan | sonnet in the CLI; provider/model for OpenCode>
- Effort: high

The paste line points at the file and never holds the brief. Another machine cannot read the file: say so in one line after the section.

## Done when

- The file exists, has every template section, and is at most 150 lines.
- The reply is the file link, the one-line block and the Start with section, nothing longer.
