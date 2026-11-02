# Working in this repo

This is a plain-git dotfiles repo (no dotfiles manager). The full operating guide (the one rule:
edit the source, layout, sync workflow, skills, conventions) is in
@AGENTS.md. Read it before making changes. It's shared with Cursor and Opencode
via the `AGENTS.md` standard, so all agents handle this repo the same way.

Read MAP.md first to find where things live.

**Agents:** before you spawn a helper here, read `.claude/AGENT-MAP.md` (who does which job, in what order).

**Before you commit a change to setup code** (`dotfiles`, `bootstrap.sh`, `scripts/`, `rust/`,
`Brewfile`, `home/`), run `/os-check` or the `os-checker` agent. Do not commit on FAIL unless the
user says so.
