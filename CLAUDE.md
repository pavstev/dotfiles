# Working in this repo

This is a plain-git dotfiles repo (no dotfiles manager): **edit the source under
`home/` or `claude/` — symlinked files are live immediately; run `./dotfiles` to
pick up new files or package changes.**

The full operating guide (layout, sync workflow, skills, conventions) is in
@AGENTS.md — read it before making changes. It's shared with Cursor and Opencode
via the `AGENTS.md` standard, so all agents handle this repo the same way.

**Before you commit a change to setup code** (`dotfiles`, `bootstrap.sh`, `scripts/`, `rust/`,
`Brewfile`, `home/`), run `/os-check`. Do not commit on FAIL unless the user says so.
It is manual only: no git hook and no CI runs it.
