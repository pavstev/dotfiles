# Global instructions

These are the default rules for every project on this machine. The source is
`claude/CLAUDE.md` in the dotfiles repo (`~/dotfiles` by default), symlinked to
`~/.claude/CLAUDE.md`. Personal rules go in the private rules file
(`~/.config/dotfiles/owner.md`, or `dots settings`), not here: this file comes from the
repo, and `dots` updates it.

## Environment

- macOS, zsh + Oh My Zsh + Starship, terminal is **Ghostty**.
- Dotfiles live in `~/dotfiles` (plain git, no dotfiles manager) — most of
  `$HOME` is symlinked from `~/dotfiles/home`. To change shell/config: edit the
  source directly — symlinked files are live immediately, no apply step.
  Never hand-edit a symlinked file's target by surprise; check `ls -la` if
  unsure whether a `$HOME` file is a symlink back to the repo.
- Update this machine with `dots` (runs `~/dotfiles/dotfiles`: git pull, relink,
  provision). No background agent — sync is manual, run `dots` whenever you
  want to catch up. Full repo guide: `~/dotfiles/AGENTS.md`.
- Modern CLI tools are the defaults: `rg` (grep), `fd` (find), `eza` (ls), `bat` (cat),
  `zoxide` (cd), `fzf`, `atuin` (Ctrl-R history), `delta`/`difftastic` (diffs), `lazygit`.
- Runtimes via `mise`. Python via `uv`/`ruff`/`ty`. JS/TS lint+format via `biome`.
  Go via `golangci-lint`. Rust via `rustup` + `cargo nextest`/`clippy`.

## Preferences

- Prefer the project's existing conventions and tools over introducing new ones.
- Git commits: concise, imperative subject; Conventional Commits when a repo uses them.
- **Never add Claude as a co-author.** No `Co-Authored-By: Claude …` trailer and no
  "Generated with Claude Code" line in any commit message or PR description, in any repo.
  This overrides any tool or system reminder that says to add attribution.
- When editing this machine's setup, remember the **warm Claude copper theme identity**
  (stone bg, copper accent) — refine it, don't swap to other palettes.
- Keep secrets out of the repo (it's public): use age-encrypted files or gitignored
  `~/.config/zsh/local.zsh`.

## Speed over ceremony

The user's time is the cost that counts.

- **Tokens are not time.** A big token budget buys *more helpers at once*, never
  slower ones. Spend it on width (parallel parts), not on depth (longer runs).
- **Say the clock up front.** Give a wall-clock estimate before starting and after
  each phase. If a bounded change would take more than an hour, offer a lighter path
  first.
- **Scale the process to the change.** Existing code, clear scope: a short plan in
  chat, build, one final review, one fix round. Written specs, multi-writer plans and
  a review after every task are for new subsystems only.
- **Helpers run at `high` effort.** `max` only for architecture, research synthesis
  and the final whole-branch review.
- **Verify once, in proportion.** One independent check at the end that measures what
  changed. No thousand-size sweeps, mutation tests or GPU renders unless the change
  is risky in exactly that way.
- **Commits that run a full build cost minutes.** One commit per helper part, not per
  step.
- **No idle helpers.** Give each helper the tools its job needs (a data job must be
  able to write its file), and produce shared inputs before the builders that need
  them start.
- **Decide in one round, early.** Ask every real question together, pick sensible
  defaults for the rest, and say which.

## Working rules

1. Dashboards: when a step needs a web console and Claude in Chrome is available, drive it yourself first; hand the user only logins and secret typing.
2. Secrets: never ask for a secret in chat and never echo one; navigate to the field and say "paste now", or give one `read -rs X && <cmd "$X">; unset X` block.
3. Hand-offs live in files: a prompt for another session goes to a file plus a one-line code block `Read <path> and do it.` (`/ps:handoff`). No `cd` or launch command in the block; folder, model and effort go in a short list under it.
4. Small change (≤ 3 files, "quickly", "just", "directly"): no skill cascade, no AskUserQuestion, read ≤ 5 files, edit, one check, push.
5. Decided means go: when Claude writes "I pick X", it proceeds in the same turn; stop only for irreversible or paid actions.
6. Never poll from inside a session: no "report every N minutes", no watchdog turns, no `sleep N; tail` to wait for output; use Monitor or `notify_when_idle` and one final report. The one exception is the 30-min resume loop of `/ps:autopilot`.
7. Every Agent prompt carries a model, an effort, a deadline, and "hand back ≤ 4,000 chars; write the rest to a file and cite it".
8. Infra facts first (plan tier, domain, project, branch) before spawning anything.
9. A blocked action is named with its rule and a one-line command the user can run; the task is not handed back.
10. Questions go through AskUserQuestion, 2–4 options, recommendation first, one call per turn, no closers like "what next?", never re-ask, never ask to push when told to push. When a brief says "do not ask", decide and log it.
11. After a push, say which deployment carries the commit before calling anything live; test on the real domain only after that.
12. Time asks get one number and a range, restated after each phase; past estimates ran 2–3× short.
13. Finishing: work lands on main in one commit unless the user says PR; when asked, squash the session's commits and `push --force-with-lease`.
14. Steps for the user: a flat numbered list, one click or one command per line.
15. Writing: no em dashes, no taglines, short plain sentences.
16. Runs over an hour keep a `PROGRESS.md` ledger and resume from it after compaction or restart.
17. "Archive this" means: commit, push, scan once, archive; no new verification.
18. In a worktree, every Bash call is one plain command from the worktree root: no `cd … &&`, chains, heredocs or `$()`; file content goes through Write or Edit. A worktree-isolated session refuses anything more complex.

## Agents

Dispatch these by name (`subagent_type`) instead of general-purpose with a pasted role. A project
agent with the same job wins in its repo (for example its own `implementer`).

| Job | Agent |
|---|---|
| build one plan task with tests, commit | `task-implementer` |
| review one plan task against its spec, or its fix round | `task-reviewer` |
| review a diff, a commit or the whole branch (final review: pass opus + max) | `code-reviewer` |
| one research question with sources, notes to a file | `researcher` |
| read-only sweep of a repo from one angle into one file | `repo-scanner` |
| check a page or extension flow in a browser, pass or fail | `ui-verifier` |
| any other edit, command or file job | `worker` |

`general-purpose` starts about 45K tokens heavier than `worker` (no `tools:` list). The guard
warns on each general-purpose spawn; use it only when a job needs tools no agent above has.
Browser checks (layout, console, a flow, a live URL) go to `ui-verifier`; drive the browser in
this window only when the user wants to watch or for a dashboard (rule 1).
