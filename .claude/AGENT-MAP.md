# Agent map: dotfiles

<!-- For the main thread, before it spawns a helper here. Under 60 lines. Shared side: claude/HARNESS-MAP.md -->

`claude/agents/` (no dot) is the shared agent source for every repo. `.claude/agents/` (with a dot) holds the
agents of this repo only.

## Jobs

| Job | Dispatch, in order | Check |
|---|---|---|
| Guard or router change | task-implementer (tests first) -> code-reviewer | `cargo nextest run --test guard` in `rust/agent-config` |
| Other Rust change (`dots`, bins) | task-implementer -> code-reviewer | `cargo clippy --all-targets -- -D warnings`, `cargo nextest run` |
| Setup code before a commit (`dotfiles`, `bootstrap.sh`, `scripts/`, `rust/`, `Brewfile`, `home/`) | os-checker | every OS that ran: PASS |
| Skill change | skill-tester (baseline) -> worker (the edit) -> skill-tester (re-run) | `rust-bin skill_check` |
| Harness review | `ps:harness-optimize`, with transcript-miner for the transcript reads | the review file |
| Docs or text across many files | worker | the pre-commit hook (private words) |
| Final review of a branch | code-reviewer | - |

## Agents here

| Agent | Job and trigger | Tools | Model | Owns |
|---|---|---|---|---|
| os-checker | runs `scripts/os-check.sh` before a setup-code commit; hands back each OS's exit line | Bash, Read | sonnet, medium | none |
| skill-tester | the skill TDD runs: a baseline probe, then the same scenario after the edit | Bash, Read, Grep, Glob, scratch Write | sonnet, high | none |
| transcript-miner | counts and quotes from session transcripts for a harness review | Bash, Read, Grep, Glob, Write | sonnet, high | its report |

## Skills, rules and hooks

- `/os-check` (`.claude/commands/os-check.md`): the same OS check by hand.
- Git hooks in `scripts/git-hooks/`: pre-commit (private words, Windows paths, skill checks for staged skills),
  pre-push (private words in every pushed commit, one history only).

## Rules

- `claude/hooks/` and the guard bins are live in every session. Test in a worktree, merge only when the tests pass,
  then rebuild the bins (`scripts/agent-config --bins` or `dots`).
- The tree is public: no person, private project, account or home path in a tracked file. Guard G16 and the hooks
  block them.
- A skill edit runs the full TDD loop with skill-tester.
