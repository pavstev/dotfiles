# Agent map: dotfiles

<!-- For the main thread, before it spawns a helper here. Under 60 lines. -->

`.claude/agents/` holds the agents of this repo only. The shared agents, skills and the guard are
the ps plugin (pavstev/agent-kit).

## Jobs

| Job | Dispatch, in order | Check |
|---|---|---|
| Rust change (`dots`, `tools`, `port`, the settings builder) | task-implementer -> code-reviewer | `cargo clippy --all-targets`, `cargo test` in `rust/agent-config` |
| Setup code before a commit (`dotfiles`, `bootstrap.sh`, `scripts/`, `rust/`, `Brewfile`, `home/`) | os-checker | every OS that ran: PASS |
| Docs or text across many files | worker | the pre-commit hook (private words) |
| Final review of a branch | code-reviewer | - |

## Agents here

| Agent | Job and trigger | Tools | Model | Owns |
|---|---|---|---|---|
| os-checker | runs `scripts/os-check.sh` before a setup-code commit; hands back each OS's exit line | Bash, Read | sonnet, medium | none |
| skill-tester | the skill TDD runs (skills now live in pavstev/agent-kit; this agent waits to move there) | Bash, Read, Grep, Glob, scratch Write | sonnet, high | none |
| transcript-miner | counts and quotes from session transcripts for a harness review (waits to move to pavstev/agent-kit) | Bash, Read, Grep, Glob, Write | sonnet, high | its report |

## Skills, rules and hooks

- `/os-check` (`.claude/commands/os-check.md`): the same OS check by hand.
- Git hooks in `scripts/git-hooks/`: pre-commit (private words, Windows paths, symlinks, credentials),
  pre-push (private words in every pushed commit, one history only).

## Rules

- The tree is public: no person, private project, account or home path in a tracked file. The git
  hooks and `dots doctor` block them.
- After a Rust change, rebuild the bins (`scripts/agent-config --bins` or `dots`).
