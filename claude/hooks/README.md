# Hooks

[Hooks](https://docs.claude.com/en/docs/claude-code/hooks) run shell commands on Claude
Code events (e.g. `PostToolUse`, `Stop`, `UserPromptSubmit`). The *scripts* live here;
they're **wired up in `settings.json` under the `hooks` key**.

The hooks directory is symlinked to `~/.claude/hooks` by `./dotfiles`.

## Project contract

"Facts in the project, smarts in the dotfiles." A project states facts in files it already has and
holds no harness logic. The dotfiles read those facts (`proj`, below) and fall back to language
defaults when a project states none. Each side works without the other, and every hook fails open.

A project MAY expose these three commands, as `package.json` scripts, or as `check`, `check-fast`
and `fix` targets in a `justfile` or `Makefile` (a `package.json` script wins over a `justfile`
recipe, which wins over a `Makefile` target, per name):

| Name | What it does |
|------|--------------|
| `check` | the full gate: format check, lint, types, tests |
| `check:fast` (`check-fast`) | the same on the changed files only; `--staged` limits it to the files staged for a commit |
| `fix` | formats and applies the safe autofixes |

Its own git hooks SHOULD run the fast check on commit (`check:fast --staged`) and the full check on
push. Then a change is checked where it is committed, and an agent only runs `check:fast` once
after its last edit. A project that exposes nothing gets the defaults below.

## proj

The `proj` Rust bin (`rust/agent-config/src/proj.rs`, `src/bin/proj.rs`; tests: `cargo test --test proj`).
It runs the project's own installed tools in the project folder (`node_modules/.bin`, `.venv/bin`,
`uv run`, `cargo`), so `mise` picks the pinned runtime and any tool version works.

```bash
~/.claude/hooks/rust-bin proj detect [path]   # JSON: root, package manager, tools, contract, check and fix commands
```

- **Root:** the nearest folder up from the path with `package.json`, `pyproject.toml`, `Cargo.toml`,
  `go.mod`, a `justfile` or `Makefile`, or a Biome or Ruff config. The home folder never counts.
- **Package manager:** the lockfile (`pnpm-lock.yaml`, `bun.lock`, `yarn.lock`, `package-lock.json`),
  looked up to the top of the git repo; else the `packageManager` field; else `npm`.
- **Tools:** a JS tool counts only when it is installed in `node_modules/.bin` *and* configured
  (Prettier, ESLint or Biome config). Python: Ruff (config or a dependency), `ty`, pytest, `uv`.
  Go, Cargo with Clippy and nextest. `rustfmt` and `shfmt` count only when the project says it uses
  them (a `rustfmt.toml`, or a mention in a script, hook config or CI file), so a file is never
  reformatted into a style the project does not use.
- **Check commands:** the contract first. Without `check:fast`, the fast default checks the files
  changed in git (`eslint`, `prettier --check`, `biome check`, `ruff check`, `go vet`, `cargo check`).
  Without `check`, the full default runs the `lint`, `typecheck` or `types`, and `test` scripts, or
  the tools over the whole project, or `ruff`, `ty`, `pytest`, `go vet` and `go test`, or
  `cargo fmt --check`, `clippy` and `nextest`.

### Format on edit

`rust-bin --hook proj format` is a global `PostToolUse` hook on `Edit`, `Write` and `MultiEdit`
(wired in [`claude/settings.json`](../settings.json)). It formats only the edited file with the
project's formatter, and adds a safe lint autofix where the project has one: `eslint --fix` first,
then `prettier --write` or `biome check --write`; `ruff check --fix` (never the unused-import fix,
which would remove an import before the next edit uses it) then `ruff format`; `gofmt`, `rustfmt`,
`shfmt`. It is silent and always exits 0. It skips a file outside a project, a file in
`node_modules`, `.git`, `.venv`, `target`, `dist`, `build` and the like, and a file over 1 MB. The
tool decides what it ignores (`.prettierignore`), and a tool that fails changes nothing. The hook has
a budget of 8 s for all commands (`PROJ_FORMAT_BUDGET_MS`); a slow tool is killed. The Python hooks
never sync an environment (`uv run --no-sync`).

## leftovers

Each project keeps its own optional work in `<main checkout>/.agents/leftovers.jsonl`. A call from a
worktree writes to the main checkout. The first `add` puts `.agents/` into the project's `.gitignore`
(the user commits that line). Only the `leftovers` bin (`rust/agent-config/src/leftovers.rs`,
`src/bin/leftovers.rs`; tests: `cargo test --test leftovers`) writes the file. It is an append-only
log of JSON lines, `{"v":1,"op":"add"|"done"|"drop","id":"<8 hex>","at":"<RFC 3339 UTC>",...}`, and
the state is the fold of the events in order. Never edit the file by hand. Nothing is loaded at session
start: ask for the list.

```bash
~/.claude/hooks/rust-bin leftovers add "text" [--tag T] [--repo PATH]
~/.claude/hooks/rust-bin leftovers list [--all] [--json]   # open items; also: done ID, drop ID, count, path, compact
~/.claude/hooks/rust-bin leftovers import                  # split the old ~/.claude/leftovers.md into the projects
```

`mic_check --note` and `--leftovers` use the same store.

## guard

The `guard` Rust bin (`rust/agent-config/src/bin/guard.rs`; fail-open: any error exits 0 silently)
keeps sessions cheap. Deny = message on stderr + exit 2. Warn (the W rows) = exit 0 with
`hookSpecificOutput.additionalContext` on stdout, so the call runs and the model reads the note.
A deny wins over a warning. Tests: `cargo test --test guard` in `rust/agent-config`.

| Guard | Event | Rule |
|-------|-------|------|
| G1 | PreToolUse `Agent` | the prompt must name a deadline and a hand-back cap (for example `Deadline: 25 min.` and `Hand back at most 4,000 chars; write the rest to a file.`). `model` and `effort` may be missing: the router fills them. A `superlite:*` agent is exempt: superlite's `sl` writes its brief |
| G2 | PreToolUse `CronCreate`, `ScheduleWakeup`, `Skill` (`loop`) | no interval under 30 min once context > 100K |
| G3 | UserPromptSubmit; PreToolUse `Agent` | at 300K+ nudge `/ps:handoff` (1st prompt, then every 5th); deny spawning above 350K |
| G4 | PreToolUse `AskUserQuestion` | one call per human turn |
| G5 | PreToolUse `SubagentHandback`; SubagentStop | a subagent's hand-back may not exceed its cap by more than 10 percent; at most 2 denies per agent, then it passes |
| G6 | Stop; PreToolUse `archive_session` | a hand-off file written this turn (`*/handoffs/*.md`) must be in the reply as a file link plus the `Read <path> and do it.` block (one deny); no archive of this session in the same turn |
| G7 | PreToolUse `Agent` | deny `general-purpose`, `claude` and a missing `subagent_type`; the message names `worker`, `researcher`, `Explore`, `task-implementer`, `Plan` |
| G8 | PreToolUse `Agent`; SubagentStop | helpers running at once per session: 3 when pace is `ok` or `unknown`, 1 when `ahead`, 0 when `critical`. `superlite:*` agents have a pool of their own (`sl_spawns`): 10, 3 when `ahead`, 0 when `critical`; a SubagentStop frees a slot in the pool of its `agent_type`. Each slot is `{id: tool_use_id, t: time}` in the session state file (plain times from the first version still load). SubagentStop frees the oldest slot (not when its hand-back is denied); `guard posttoolfailure` (PostToolUseFailure, matcher `Agent`) frees the slot of the failed call's `tool_use_id`; entries over 90 min old are dropped. `flock` guards parallel calls |
| G9 | PreToolUse `Skill`, `EnterPlanMode` | `superlite:brainstorming` and `superlite:writing-plans` in the main thread are allowed in plan mode, when pace is `ahead` or `critical`, and when the main model is Opus or Fable (newest assistant `message.model` in the transcript). With a Sonnet or Haiku main model they are denied: on `entrypoint: cli` use EnterPlanMode (opusplan); on `entrypoint: claude-desktop` (no opusplan) send a `Plan` helper or pick Opus. An unknown model or entrypoint is allowed. `EnterPlanMode` is denied when pace is `ahead` or `critical` (Sonnet plans without plan mode) |
| W5 | PreToolUse `Bash` (the record is PostToolUse) | warn, never deny: a plain check command (`<pm> run check\|test\|lint\|typecheck\|types\|build`, `cargo test\|nextest\|clippy\|check`, `go test\|vet`, `pytest`, `uv run ruff check\|ty check\|pytest`, `ruff check`, `ty check`, `just\|make check\|test\|lint`) that already passed in this session, with the git work tree unchanged since, gets the note "skip it". A pass is recorded by PostToolUse, which Claude Code runs only on success (a failed Bash goes to PostToolUseFailure; an interrupted one is not recorded). Only one plain command counts, with optional `cd x &&` steps and `VAR=value` prefixes: a pipe, `;`, `\|\|` or `&` hides the exit code, so those are never recorded. The key is the folder plus the command in one line; the table keeps the latest 40 per session. The fingerprint is the HEAD commit plus `git status` plus the size and time of each listed file (two git calls); it changes on a commit, an edit, a new file or a stage. No git or any error says nothing |
| G18 | PreToolUse `Bash`, `Edit`, `Write`, `MultiEdit` (the record is PostToolUseFailure, the clear is PostToolUse) | the workflow rule "after 2 failed tries on one step, stop and report". A step is the same Bash command in the same folder, or the same file and edit target (`Write`: the file). The second failure adds the note "stop and report" (warn). The third try is denied with "stop and report: 2 failed tries on this step". A Bash step counts only while the git work tree is unchanged since its last failure, so edit, rerun, fail is a new try. A pass of the step, a new user message, or `is_interrupt` resets or skips the count. A helper has its own count. The table keeps the latest 40 steps per session |
| G19 | Stop (autopilot sessions only); SubagentStop (every helper) | holds the stop back once with "run the fast check before you stop: `<fast check>`" when the caller edited a file in a project that has a fast check (`proj detect`) and no plain check command (the W5 list) passed after the last edit. PostToolUse counts the edits (`n`) and the pass (`ok = n`) per agent in the session state file. The second stop goes through, by `stop_hook_active` and by a `blocked` mark in the state, so it never loops. A normal chat is never held back: the main thread needs `ps:autopilot` in its transcript (a Skill call or the typed command). A held-back helper keeps its G8 slot until its second stop. A `superlite:*` agent is never held back: superlite runs its own test gate (`sl run_tests`) |
| W3 | PreToolUse browser tools (`mcp__Claude_Browser__*`, `mcp__claude-in-chrome__*`) | warn in a main window: at the 6th browser call of a turn (then every 20th), route checks to `ui-verifier` |
| W4 | PreToolUse `Agent` | warn: past 15 spawns or 1.5M subagent tokens in a session (the `totalTokens` of each Agent result, and of each background task notification once), a budget note on every 5th spawn |

**Router** (PreToolUse `Agent`, main thread, not `fork`): the guard sets `model` and `effort`, and
`xhigh` or `max` never pass. `Plan` and a `code-reviewer` doing a final or whole-branch review
(description or the first 400 chars of the prompt; a panel angle or re-review is not final) get
opus + high (sonnet + high when pace is `ahead` or `critical`). Every other row never raises
cost: the lower of the caller's model and the table's (`Explore`, `claude-code-guide`,
`statusline-setup`: haiku; the rest: sonnet), effort `high` unless the caller gave `low` or
`medium`, no effort for haiku. A changed call is
returned as `permissionDecision: allow` + `updatedInput` (the whole tool input) + a note, on the
same JSON line as the W notes. A `superlite:*` agent is left alone, because its agent file pins model and
effort and superlite says a `model` overrides it. The one change: `superlite:final-reviewer` and
`superlite:fixer-opus` get `model: sonnet` when pace is `ahead` or `critical`.

**Pace** comes from the `pace` module (`rust/agent-config/src/pace_core.rs`: status, and a refresh with a
5 second budget on each UserPromptSubmit). On any error the state is `unknown`. On a prompt, the
first time a session's state becomes `ahead`, the guard adds one note (used and budget percent,
what changes); while `critical` it adds the note on every prompt. The last announced state is kept
in the session state file, and `unknown` does not reset it.

Context size comes from the last assistant `usage` in the transcript (read from the end
of the file, so big transcripts are cheap). Session state (nudge counter, spawn times, last pace
note) lives in `${TMPDIR:-/tmp}/claude-guard/<session_id>.json`.

G5 reads the cap from the subagent's own prompt: the smallest `N chars` (N of 200 or more) tied to
the reply by a word in the same sentence, up to 80 characters before the number or 40 after it.
A "hand back" tie wins first, then strong words (final message, reply, response, output), then weak
ones (answer, return, report, summary), so "return at most 500 characters" in a spec does not beat
"hand back 4,000 chars".
Last comes a bare bound right before the number, as in "Stay ≤ 4,000 chars.".
If the counter file cannot be written, G5 allows, so a deny never repeats without end. Without
one, the cap is the default of the agent type (task-implementer, task-reviewer, researcher and
ui-verifier and worker 3,000; repo-scanner 2,000; code-reviewer and anything else 4,000). The prompt is the
first user record of the subagent transcript (`agent_transcript_path`, or
`<session>/subagents/agent-<agent_id>.jsonl`). A deny makes the subagent write its text to a file and
hand back a shorter one. SubagentStop skips the check when `stop_hook_active` is true. Deny counters
live in `${TMPDIR:-/tmp}/claude-guard/handback-<agent_id>.json`.

In auto mode a subagent hands back through the `SubagentHandback` tool, and SubagentStop then has no
`last_assistant_message`. In other modes SubagentStop carries the text. So G5 hooks both (tested
live with Claude Code 2.1.296).

Each check that fires (a deny, an ask, a Stop reminder, a warning, a hint, a note, a route, or a
shadow-mode hit) appends `<unix secs> <id> <kind>` to `${XDG_CACHE_HOME:-~/.cache}/dots-guard-fires.log`.
The line holds no message text. Over 1 MB the log moves to `.old`. `harness_stats guard [--days 30]`
reads both files: fires per check, the last fire, and the checks that are on but never fired in the
window. A check that stays quiet for weeks may be a guardrail the models outgrew, so it is a
candidate to remove. The Cursor adapter (`guard_cursor`) logs its denies, asks and shadow hits
too. Opencode runs this guard, so its fires count, but its plugin drops warnings, hints and router
notes: under Opencode those fires reach the log and not the model.

Not built: a Bash fixer for worktree refusals. A hook deny in place of the harness refusal saves
nothing, because a refusal streak ends on the next call anyway. Almost no refusal has the one shape
a hook could rewrite without changing what runs (`cd <root> && <one command>`): they are heredoc
scripts, pipes and chains. The harness check runs after PreToolUse hooks and is a safety check, so
the hook does not try to get around it. The rule text carries the fix instead: the
plain-worktree-commands rule in `claude/CLAUDE.md` and the `worker` and `task-implementer` bodies.

### Scratch mode

`~/.claude/hooks/rust-bin --hook guard scratch` is not wired in `settings.json`. An agent file wires it
in its frontmatter as a `PreToolUse` hook on `Write|Edit|NotebookEdit` (the ui-verifier does). It
allows a write only when the real path is under `/tmp`, `/private/tmp`, `/var/folders`,
`/private/var/folders`, `$TMPDIR`, a `scratchpad` folder, or a `.agents/` or `.superlite/` note
folder (callers name report files there). Anything else is denied.

Wired in [`claude/settings.json`](../settings.json) (`hooks`). Each command starts with
`test -r ~/.claude/hooks/rust-bin || exit 0`, so a missing launcher never blocks a tool.
`rust-bin --hook guard <mode>` runs the Rust `guard` binary built by `dots`; with no binary it
exits 0, silent. A skill call (no `--hook`) with no binary says `Run: dots`
and exits 127. It never builds (`dots` does, with `scripts/agent-config --bins`).
Hooks start it with `sh`, not `bash`, and it starts no subshell or other program: Homebrew bash
alone takes about 6 ms to start, and a hook runs on every tool call.
