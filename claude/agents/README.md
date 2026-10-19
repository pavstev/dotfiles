# Agents (subagents)

Custom [subagents](https://code.claude.com/docs/en/sub-agents): focused assistants Claude
delegates to. Each is a Markdown file with YAML frontmatter followed by the system prompt.

Symlinked to `~/.claude/agents` by `./dotfiles`, so a new file here is live on this machine at once.
Run `dots` on other machines. `scripts/agent-config.py` copies them to `~/.cursor/agents`
and to `~/.config/opencode/agents`.

## The agents

| Agent | Job | Tools | Model / effort |
|---|---|---|---|
| `task-implementer` | build one plan task: failing test, code, checks, commit | Read, Edit, Write, Bash, Grep, Glob | sonnet / high |
| `task-reviewer` | review one plan task against its spec, then quality; re-review a fix round | Read, Grep, Glob, Bash | sonnet / high |
| `code-reviewer` | review a diff, a commit or a whole branch; the final review gets opus + max per call | Read, Grep, Glob, Bash | sonnet / high |
| `researcher` | one research question with sources; full notes to one named file | web, Read, Grep, Glob, Write | sonnet / high |
| `repo-scanner` | read-only sweep of a repo from one angle into one file | Read, Grep, Glob, Bash, Write | sonnet / high |
| `ui-verifier` | check a page or extension flow in a browser, pass or fail with evidence | Read, Write, Bash, Grep, Glob, ToolSearch, both browser MCP servers (`mcp__<server>__*`); Write only to scratch (frontmatter hook) | sonnet / high |
| `worker` | any other edit, command or file job; the lean stand-in for general-purpose | Read, Edit, Write, Bash, Grep, Glob | sonnet / high |

`claude/CLAUDE.md` "Agents" holds the routing table. Opus 5.x rarely picks an agent from its
description alone (anthropics/claude-code#82456), so the table names them. A project agent with the
same name wins in its repo, which is why these use `task-` names.

## House rules for an agent file

- Always `name:`. Quote the `description` (a bare `: ` in it drops the file silently). Start it with
  "Use to" or "Use for", say when, and say what it is not for.
- Pin `model:` and `effort:`. An unpinned agent inherits the caller's effort. Set `maxTurns`.
- Give only the tools the job needs, as a `tools:` list. An agent without one loads every tool and
  skill: a fresh spawn measured 14.6K start tokens for `worker` against 46K for general-purpose
  (2026-10-15, `claude -p`; interactive sessions add MCP tools, 59-72K over 7 days). A read-only agent also gets `disallowedTools: Write, Edit,
  NotebookEdit, Agent`; prose like "never modify files" is not a tool limit. Workers never get Agent.
- Bash in a read-only agent is for reading: `git diff`, tests, `rg`. Say so in the body.
- The body ends with a hand-back shape: a status word on line 1 (`DONE`, `APPROVED`, `PASS`,
  `BLOCKED` ...), then a "Denied or skipped" line, real command output, and a character cap.
  The status word comes first even when the caller sends its own report template.
- The caller's cap wins when it names one. Guard G5 (`claude/hooks/guard.py`) sends an over-cap
  hand-back back to the agent, so the body says "count before you hand back".
- File content goes through Write or Edit, never a heredoc, `echo >` or `sed -i`. Bash `cat` is not
  a Read. In a worktree, every Bash call is one plain command, not only git.
- An agent that needs throwaway files but must not touch code gets Write plus a frontmatter
  `PreToolUse` hook running `guard.py scratch` (see `ui-verifier.md`). Never write `hooks: []`.
- Git rule for anything that may run in a worktree: one plain git command per Bash call, from the
  worktree root. No `cd … &&`, heredoc or `$()`.
- No private project names: name the repo kind.
- A new or edited file reaches a running session only after a short delay, and a new agent name
  can fail with "Agent type not found" until a new session starts. To test at once, run a fresh
  `claude -p` that dispatches the agent.

## Test runs (2026-10-14)

One real past task per agent, the same caller prompt twice: general-purpose (how it was done before)
and the agent. Sonnet 5.5 at high. Evidence for the picks: 14 days of transcripts, 1,438 Agent calls,
in the ignored `.agents/reports/2026-10-14-agent-evidence.md`.

| Agent | Task | Without (general-purpose or old file) | With the agent |
|---|---|---|---|
| task-implementer | replace a deprecated macOS disk-image tool in a Python CLI clone | done, 19 calls, tests pass; no failing-test step, no lint, no status line | `DONE` first, 17 calls, 0 errors; showed 5 tests failing first, then 325 pass, ruff check and format |
| task-reviewer | review a browser-extension fix commit against its plan task | "ship it", about 4,000 chars, 4 soft points | `APPROVED` first, 1,991 chars, 0 errors; found the later follow-up commit, so no stale point |
| code-reviewer | review a skill commit with a router script | old file: no shell, "did not run the tests"; general-purpose: good, no verdict line | `CHANGES NEEDED` first, ran the tests on python3 and 3.9, scored findings, found one new real issue |
| researcher | structured-data question, notes to a file | old file: could not write the file; general-purpose: wrote it | wrote the file, `Notes:` line, 3,474 chars, 0 errors |
| repo-scanner | SEO inventory of the static-site repo into a file | file written, 4,000-char hand-back, few line numbers | `DONE <path>` first, 2,992 chars, line ranges and counts, found a wrong path in the repo's own docs |
| ui-verifier | live site at 375 and 1440 px, heading, overflow, console | 23 calls, screenshots | `PASS` first, 13 calls, no screenshots (page text first), table with evidence, found a doubled heading text |

The ui-verifier run used the agent body through general-purpose: a headless `claude -p` has no
built-in browser. Gaps closed after the runs: none needed. The test parent once mis-copied a long
path and dropped line 1 when it echoed a hand-back; both were the parent, not the agents.

## Deep pass (2026-10-14, every transcript)

Every session was read, not sampled: 211 main and 1,432 subagent transcripts on this Mac, a Linux
host, and the Cursor chats. The findings behind each change, with counts:

- Hand-back over the caller's cap: 222 of 449 capped runs since 2026-10-12 (researcher 45 of 60,
  implementer 32 of 52, code-reviewer 8 of 11). Fix: guard G5 plus "count before you hand back".
- Worktree refusals: 478. The top shape is a heredoc file write (136), not git. Fix: file content
  through Write or Edit, one plain command per Bash call in a worktree.
- Edit before Read: 152, mostly after `cat` in Bash. Fix: "Bash `cat` is not a Read".
- Reviewers had no verdict on line 1 when the caller sent a template (0 of 57 in one repo). Fix:
  verdict first, template after.
- `echo =====` fails under zsh (22 to 61 errors). Fix: `echo '---'`.
- ui-verifier needed throwaway scripts (19 refusals in 6 runs) and once wrote a repo file through a
  Bash heredoc. Fix: Write with the `guard.py scratch` frontmatter hook; no file writes in Bash.

Same caller prompt twice in a fresh `claude -p` (auto mode, Sonnet 5.5): old files as project
agents, then the new files with G5 wired. Task kinds are real past tasks.

| Agent | Task | Old | New |
|---|---|---|---|
| task-implementer | fix a CLI option check, with tests, in a worktree | `DONE`, 12 calls, 1,229 chars | `DONE`, 12 calls, 1,195 chars, no heredoc |
| task-reviewer | review a commit against its task, caller template | verdict first, 1,614 chars | verdict first, 1,498 chars |
| code-reviewer | review a commit, cap 2,500 | 2,911 chars, over cap | 3,059 denied by G5, resent at 2,166 |
| researcher | tool support question, notes file, cap 2,000 | no status line, 2,194 chars | `DONE` first, 2,122 chars, 0 errors |
| repo-scanner | GNU or BSD only flags into a file, cap 1,200 | 1,445 chars, over cap | 1,490 denied by G5, resent at 910 |
| ui-verifier | curl check, script to `$TMPDIR`, notes file | Write refused, wrote both via Bash heredoc | Write used; a write to the home folder was denied by the frontmatter hook |
