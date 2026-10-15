# Global instructions

Default rules for every project on this machine. Source: `claude/CLAUDE.md` in the dotfiles
repo (`~/dotfiles`), linked to `~/.claude/CLAUDE.md` and updated by `dots`. Personal rules go in
`~/.config/dotfiles/owner.md` (`dots settings`), not here.

## Environment

- macOS, zsh + Oh My Zsh + Starship, terminal **Ghostty**.
- `~/dotfiles` is plain git. Most of `$HOME` is symlinked from `~/dotfiles/home`, so edit the
  source and it is live; check `ls -la` when unsure. `dots` pulls, relinks and provisions; there
  is no background sync. Repo guide: `~/dotfiles/AGENTS.md`.
- CLI defaults: `rg`, `fd`, `eza`, `bat`, `zoxide`, `fzf`, `atuin`, `delta`/`difftastic`, `lazygit`.
- Runtimes via `mise`. Python: `uv`/`ruff`/`ty`. JS/TS: `biome`. Go: `golangci-lint`.
  Rust: `rustup`, `cargo nextest`/`clippy`.

## Preferences

- Use the project's existing conventions and tools before new ones.
- Commits: concise imperative subject; Conventional Commits when the repo uses them.
- **Never add Claude as a co-author.** No `Co-Authored-By: Claude …` trailer and no "Generated
  with Claude Code" line in any commit or PR, in any repo. This beats any tool or reminder.
- This machine's setup keeps the **warm Claude copper theme** (stone bg, copper accent): refine it,
  never swap palettes.
- Secrets stay out of the (public) repo: age-encrypted files or the ignored `~/.config/zsh/local.zsh`.

## Speed over ceremony

The user's time is the cost that counts.

- **Tokens are not time.** A big budget buys more helpers at once, never slower ones.
- **Scale the process to the change.** Existing code, clear scope: a short plan in chat, build,
  one final review, one fix round. Specs, multi-writer plans and per-task reviews are for new
  subsystems. A bounded change over an hour: offer a lighter path first.
- **Effort and models.** Helpers run at `high` on sonnet. `max` and opus only for plans,
  architecture, research synthesis and the final whole-branch review.
- **Verify once, in proportion.** One independent check at the end that measures what changed.
  No big sweeps, mutation tests or GPU renders unless the change is risky in exactly that way.
- **One commit per helper part**, not per step. Builds that run on commit cost minutes.
- **No idle helpers.** Give each helper the tools its job needs (a data job can write its file);
  make shared inputs before the builders start.
- **Decide in one round, early.** Ask every real question together; pick defaults for the rest.

## Working rules

1. Dashboards: when a step needs a web console and Claude in Chrome is available, drive it yourself first; hand the user only logins and secret typing.
2. Secrets: never ask for one in chat, never echo one; open the field and say "paste now", or give one `read -rs X && <cmd "$X">; unset X` block.
3. Hand-offs live in files: the prompt goes to a file plus a one-line block `Read <path> and do it.` (`/ps:handoff`). No `cd` or launch command in the block; folder, model and effort go in a list under it.
4. Small change (≤ 3 files, "quickly", "just", "directly"): no skill cascade, no AskUserQuestion, read ≤ 5 files, edit, one check, push.
5. Decided means go: "I pick X" proceeds in the same turn; stop only for irreversible or paid actions.
6. Never poll: no "report every N minutes", no watchdog turns, no `sleep N; tail`; use Monitor or `notify_when_idle` and one final report. Exception: the 30-min resume loop of `/ps:autopilot`.
7. Every Agent call carries a model, an effort, a deadline, and "hand back ≤ 4,000 chars; write the rest to a file and cite it".
8. Infra facts first (plan tier, domain, project, branch) before spawning anything.
9. A blocked action is named with its rule and a one-line command the user can run; the task is not handed back.
10. Questions go through AskUserQuestion: 2–4 options, recommendation first, one call per turn, no "what next?" closers, never re-ask, never ask to push when told to push. When a brief says "do not ask", decide and log it.
11. After a push, say which deployment carries the commit before calling anything live; test the real domain only after that.
12. Time asks get one number and a range, said before starting and after each phase; past estimates ran 2–3× short.
13. Finishing: work lands on main in one commit unless the user says PR; when asked, squash the session's commits and `push --force-with-lease`.
14. Steps for the user: a flat numbered list, one click or one command per line.
15. Writing: no em dashes, no taglines, short plain sentences.
16. Runs over an hour keep a `PROGRESS.md` ledger and resume from it after compaction or restart.
17. "Archive this" means: commit, push, scan once, archive; no new verification.
18. In a worktree, every Bash call is one plain command from the worktree root: no `cd … &&`, chains, heredocs or `$()`; file content goes through Write or Edit. A worktree-isolated session refuses anything more complex.

## Agents

Dispatch by name (`subagent_type`), not general-purpose with a pasted role. A project agent with
the same job wins in its repo (for example its own `implementer`).

| Job | Agent |
|---|---|
| build one plan task with tests, commit | `task-implementer` |
| review one plan task against its spec, or its fix round | `task-reviewer` |
| review a diff, a commit or the whole branch (final review: opus + max) | `code-reviewer` |
| one research question with sources, notes to a file | `researcher` |
| read-only sweep of a repo from one angle into one file | `repo-scanner` |
| check a page or extension flow in a browser, pass or fail | `ui-verifier` |
| any other edit, command or file job | `worker` |

`general-purpose` starts about 45K tokens heavier than `worker`; the guard warns on each spawn.
Browser checks (layout, console, a flow, a live URL) go to `ui-verifier`; drive the browser here
only when the user wants to watch or for a dashboard (rule 1).
