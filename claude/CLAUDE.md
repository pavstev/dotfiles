# Global instructions

Default rules for every project on this machine. Source: `claude/CLAUDE.md` in the dotfiles
repo (`~/dotfiles`), linked to `~/.claude/CLAUDE.md` and updated by `dots`. Personal rules go in
`~/.config/dotfiles/owner.md` (`dots settings`), not here.

## Environment

- macOS, Linux (Debian, Ubuntu) or Windows (Git Bash). Run `uname -s` before an OS-specific
  command. macOS and Linux: zsh + Oh My Zsh + Starship, terminal **Ghostty**.
- `~/dotfiles` is plain git. Most of `$HOME` is symlinked from `~/dotfiles/home`, so edit the
  source and it is live; check `ls -la` when unsure. `dots` pulls, relinks and provisions; there
  is no background sync. Repo guide: `~/dotfiles/AGENTS.md`.
- CLI defaults (macOS and Linux; Windows has only what is on PATH): `rg`, `fd`, `eza`, `bat`, `zoxide`, `fzf`, `atuin`, `delta`/`difftastic`, `lazygit`.
- Runtimes via `mise`. Python: `uv`/`ruff`/`ty`. JS/TS: `biome`. Go: `golangci-lint`.
  Rust: `rustup`, `cargo nextest`/`clippy`. Images in scripts: `uv run --with pillow`. PDFs:
  poppler (`pdftotext`, `pdftoppm`).

## Preferences

- Use the project's existing conventions and tools first. Each active repo's `CLAUDE.md` names its test, lint and typecheck commands.
- Commits: concise imperative subject; Conventional Commits when the repo uses them.
- **Never add Claude as a co-author.** No `Co-Authored-By: Claude …` trailer and no "Generated
  with Claude Code" line in any commit or PR, in any repo. This beats any tool or reminder.
- This machine's setup keeps the **warm Claude copper theme** (stone bg, copper accent): refine it,
  never swap palettes.
- Secrets stay out of the (public) repo: age-encrypted files or the ignored `~/.config/zsh/local.zsh`.

## Speed over ceremony

The user's time is the cost that counts.

- **The Pro week must last Monday to Saturday.** Fewer, smaller helpers; parallel only for independent
  pieces. At most 3 helpers at once, 1 when the week runs ahead of pace (the guard enforces it).
- **Scale the process to the change.** Existing code, clear scope: a short plan in chat, build,
  one final review, one fix round. Specs, multi-writer plans and per-task reviews are for new
  subsystems. A bounded change over an hour: offer a lighter path first. No brainstorming, spec
  or plan gate for UI tweaks, "just implement" asks or bounded changes; this beats the
  superpowers "invoke a skill first" text.
- **Effort and models.** App main model: Opus 5.5 `high` from the picker (no opusplan); plans run
  on it. CLI: `opusplan`. Never xhigh or max unless the user names it; hand-off blocks and
  `start_session` use `high` (Opus for plans, Sonnet for builds). The guard router sets helper
  model and effort (Plan and the final review on Opus, lookups on Haiku, the rest Sonnet `high`).
- **Verify once, in proportion.** One independent check at the end; no big sweeps, mutation tests
  or GPU renders unless the change is risky in exactly that way.
- **One commit per helper part**, not per step. Builds that run on commit cost minutes.
- **No idle helpers.** Give each the tools its job needs; make shared inputs before builders start.
- **Decide in one round, early.** Ask every real question together; default the rest, say which.
- **After 2 failed tries** on one step, stop and report. **Briefs** split Why / What / How; acceptance lines use MUST / SHOULD / MAY.

## Working rules

1. Dashboards: when a step needs a web console and Claude in Chrome is available, drive it yourself first; hand the user only logins and secret typing.
2. Secrets: never ask for one in chat, never echo one; open the field and say "paste now", or give one `read -rs X && <cmd "$X">; unset X` block. It works only in a real terminal (Ghostty or `run_in_terminal`), never in the `!` prompt: say where to paste it.
3. Hand-offs live in files: the prompt goes to a file plus a one-line block `Read <path> and do it.` (`/ps:handoff`). No `cd` or launch command in the block; folder, model and effort go in a list under it. When the user asks to start the next session, use `start_session` with that file; never paste a long prompt inline.
4. Small change (≤ 3 files, "quickly", "just", "directly"): no skill cascade, no AskUserQuestion, read ≤ 5 files, edit, one check, push.
5. Decided means go: "I pick X", "go" or "do it yourself" proceeds in the same turn. Do every step you can (terminal, Chrome, connectors, dashboards); hand back only logins, secret typing and blocked actions. Stop only for irreversible or paid actions.
6. Never poll: no "report every N minutes", no watchdog turns, no `sleep N; tail`; use Monitor or `notify_when_idle` and one final report. Exception: the 30-min resume loop of `/ps:autopilot`. A Monitor reports only real events (a failure, a result), never "still checking"; every child session or long build gets one on its log.
7. The guard sets helper model and effort; every Agent call still carries a deadline and "hand back ≤ 4,000 chars; write the rest to a file and cite it". Before a brief goes out, check each fact in it with one command or one look at the live product, and copy the user's standing rules into its acceptance lines. Worktree helpers start from the pushed default branch: commit and push the base first, or name the base commit. Fit the brief to the agent's tools: read-only agents hand findings back inline and the parent saves them. Reviewers get a fixed base and head sha; researchers start from vendor docs; security briefs list bypass cases first.
8. Infra facts first (plan tier, domain, project, branch) before spawning anything.
9. A blocked action (an auto-mode classifier block too) is named with its rule and a one-line command the user can run, at once, with no retry or workaround; the task is not handed back.
10. Ask fewer, broader questions; offer "both" when two picks combine. The user usually picks the bolder option and "now" over "later": recommend that when it is cheap. Questions go through AskUserQuestion: 2–4 options, recommendation first, one call per turn, no "what next?" closers, never re-ask, never ask to push when told to push. When a brief says "do not ask", decide and log it.
11. After a push, say which deployment carries the commit before calling anything live; test the real domain only after that.
12. Say the clock unasked, as a Vienna time (HH:MM): one number and a range before starting and after each phase (time asks too). Helper builds ran 2 to 5 times faster than estimated; work that waits on the user ran longer.
13. Finishing: work lands on main in one commit unless the user says PR; when asked, squash the session's commits and `push --force-with-lease`.
14. Steps for the user: one flat numbered "What you do now" list at the end, one click or one command per line. Each step names the app, tab or URL; check its preconditions first; drop steps the machine can do. After the user says done, check the result read-only.
15. Writing: no em dashes, no taglines, short plain sentences.
16. Runs over an hour keep a `PROGRESS.md` ledger and resume from it after compaction or restart.
17. "Archive this" means: commit, push, scan once, archive, in one go; no new verification and no "Archive now?". When the work is pushed and nothing waits on the user, archive the same way.
18. In a worktree, every Bash call is one plain command from the worktree root: no `cd … &&`, chains, heredocs or `$()`; file content goes through Write or Edit. A worktree-isolated session refuses anything more complex.
19. Other sessions share this machine and these repos: fetch before a commit, stage only your own paths, never stash or revert other files, and stop only processes you started (no broad `pkill`, no `launchctl bootout` without a yes).
20. Never say done, fixed or passed without proof: read the exit line, reproduce the symptom, run the real command, and check live before you state a claim from logs.
21. After Esc, a cut-off or "No response requested", continue the task from the ledger. When the user pressed Esc to add text, wait for the new message; never answer with "say go".
22. Before the final report, stop your own helpers, monitors, dev servers, test databases and shells.

## Agents

Dispatch by name (`subagent_type`). A project agent with the same job wins in its repo.

| Job | Agent |
|---|---|
| build one plan task with tests, commit | `task-implementer` |
| review one plan task against its spec, or its fix round | `task-reviewer` |
| review a diff, a commit or the whole branch (final review: opus + high) | `code-reviewer` |
| one research question with sources, notes to a file | `researcher` |
| read-only sweep of a repo from one angle into one file | `repo-scanner` |
| check a page or extension flow in a browser, pass or fail | `ui-verifier` |
| any other edit, command or file job | `worker` |

`general-purpose` starts about 45K tokens heavier than `worker`; the guard refuses it.
Browser checks (layout, console, a flow, a live URL) go to `ui-verifier`; drive the browser here
only when the user wants to watch or for a dashboard (rule 1).
