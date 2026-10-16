# Working in this repo: agent guide

A **plain-git dotfiles repo**: no dotfiles manager, no templating. One script projects it onto the
machine. This is the manual for any AI agent (Claude Code, Cursor) that changes it. Cursor reads
`AGENTS.md` itself; Claude Code reads it through the repo's `CLAUDE.md`.

## The one rule

**Edit the source, never `$HOME` by surprise.** Most of `$HOME` is symlinked from `home/` and
`claude/`, so an edit on either side is the same file and is live at once. New files, packages
and machine setup take effect only after `./dotfiles` (alias `dots`: pull, link, provision).

- A managed dotfile (shell, prompt, terminal, git): edit under `home/`.
- Claude config (settings, skills, agents, hooks): edit under `claude/`.
- Unsure whether a `$HOME` file is a link back here: `ls -la`.
- No background sync. This machine pushes; others run `dots` to catch up. Commit and push yourself.

## Layout

| Path | What |
|------|------|
| `dotfiles` | the one command: git pull, link files, run provisioning steps. |
| `home/` | mirrors `$HOME` 1:1; only paths in the `LINKS` array of `dotfiles` are touched. |
| `scripts/install.d/` | ordered (`NN-name.sh`), skip-aware, fault-tolerant provisioning steps. |
| `scripts/setup-lib.sh` | step helpers (`step`/`info`/`ok`/`warn`/`skip`/`have`/`brew_env`/`link_item`), `dotfiles_profile`, `setting`. |
| `claude/` | Claude config, symlinked per item into `~/.claude`. Skills in `claude/skills/` (rules: `claude/skills/README.md`). |
| `cursor/` | Cursor-only sources: `mcp.json` (merged into `~/.cursor/mcp.json`) and `hooks/guard_cursor.py` (+ test), the hook adapter for guard G3. |
| `scripts/agent-config.py` | stdlib generator: `cursor` writes and merges `~/.cursor`, `opencode` writes `~/.config/opencode/agents/` and merges permissions/MCP into `~/.config/opencode/opencode.jsonc`, `user-rules` prints Cursor's User Rules text, `claude-settings` builds `~/.claude/settings.json`. |
| `docs/guide.md`, `docs/cursor-port.md` | the user guide; the Cursor mapping and gap table. `README.md` is the start page. |
| `Brewfile` | every package (per-machine extras in the ignored `Brewfile.local`). |
| `settings.example`, `scripts/settings.sh` | every setting with a note and default; the `dots settings` menu. |
| `scripts/test-linux.sh` | runs `./dotfiles` twice in a fresh Debian or Ubuntu container. |
| `bootstrap.sh` | fresh-machine one-liner (Mac, Debian, Ubuntu). README "Set up a Mac" has it. |

## Tests

- Unit: `python3 scripts/test_agent_config.py`, `python3 cursor/hooks/test_guard_cursor.py`,
  `python3 claude/hooks/test_guard.py` and every `claude/skills/*/scripts/test_*.py` (pre-commit
  runs those). They must also pass on `/usr/bin/python3` (3.9) and on Linux python (no Mac paths,
  no `pbcopy`).
- `bash scripts/test-dotfiles.sh`: `./dotfiles` in a temp HOME with a temp repo and origin (first
  run, owner profile, owner rules linked and unlinked, a no-change second run, restart after a
  pull). About 2 seconds, nothing installs.
- One step in a temp HOME: `HOME=$(mktemp -d) DOTFILES_STEPS=60-cursor ./dotfiles`.
  `DOTFILES_STEPS` filters install.d steps (names without `.sh`; empty runs all). The pull, git
  identity prompt, links and skill sync still run.

## Skills

`claude/skill-sync.sh` links each `claude/skills/<name>` into `~/.claude/skills`, never touching
skills it does not own (on a name clash the local one wins). Cursor reads `~/.claude/skills`
natively; nothing goes into `~/.cursor/skills`. New skill: copy `claude/skills/_template/`, keep
folder name == `name:`, run `dots`.

## Cursor

`claude/` is the source for Cursor too: edit it, then run `dots`. Step `60-cursor` runs only when
Cursor is present: it installs the CLI, the Brewfile `vscode` extensions, and runs
`scripts/agent-config.py cursor`. What Cursor gets: `docs/guide.md` "Cursor" and
`docs/cursor-port.md`.

- Generated Markdown carries a `generated-by: dotfiles` marker. A file without it is the user's.
- Never edit the generated files under `~/.cursor`; the next `dots` overwrites them.
- Never symlink `~/.cursor/cli-config.json` or `~/.cursor/mcp.json`. Cursor rewrites them.
- Rules in every folder: run `cursor-rules`, paste in Cursor Settings > Rules > User Rules.
  `dots` then skips the file rule and warns when to paste again.

## Opencode

Opencode (the `opencode` CLI and the Opencode app) uses the same `claude/` source.
Edit `claude/`, then run `dots`. Step `scripts/install.d/58-opencode.sh` verifies the
`opencode` CLI (installed by the Brewfile). `agent-config.py opencode` generates what
Opencode needs:

- **Skills (shared, no extra step):** Opencode reads `~/.claude/skills` natively, and
  `claude/skill-sync.sh` already links every `claude/skills/<name>` there — all `ps:*`
  skills work in Opencode, in every project.
- **Agents:** `~/.config/opencode/agents/<name>.md` generated from `claude/agents/*.md`
  (`mode: subagent`, `permission.skill: allow`; read-only agents also get
  `permission.edit: deny`). The Claude `model:` pin is dropped because Opencode uses
  `provider/model-id` — ported agents inherit the session model, so nothing breaks.
- **Permissions + MCP:** merged into `~/.config/opencode/opencode.jsonc` — translated
  `claude/settings.json` permissions (your keys win) and `cursor/mcp.json` servers,
  opt-in on every profile (`OPENCODE_MCP="on"` or `DOTFILES_MCP=on`).

Opencode does not get Claude's guard hooks, output styles, `CLAUDE.md`, or connectors
(Opencode has no hook system or global-instructions file). Full mapping and gap table:
[`docs/opencode-port.md`](docs/opencode-port.md).

## Linux

`./dotfiles` also runs on Debian 13 and Ubuntu 24.04 (a normal user with `sudo`). The Mac is the
reference: a Mac run must behave as before.

- Ask the OS with `is_macos`, `is_linux` and `only_macos <what>` from `scripts/setup-lib.sh`, never
  inline `uname`. Python uses `sys.platform`. A Mac-only step starts with `only_macos "<what>"`; a
  Linux-only step with `is_linux || exit 0`.
- Tools come from Homebrew on Linux (`/home/linuxbrew/.linuxbrew`), same Brewfile:
  `brewfile_for_os` drops `cask`, `mas`, `tap`, `vscode` and `brew "mas"` lines. apt installs only
  `LINUX_PREREQS` (step `05-linux-prereqs`); never add `apt install` of tools.
- Paths: `brew_env` knows the Linux prefixes. Editor settings keep their Mac path under
  `home/Library/Application Support/` and link to `~/.config/{Code,Cursor}/User` on Linux.
- No GNU-only or BSD-only flags in shared scripts (`sed -i`, `stat`, `date -j`, `readlink -f`).
- Test: `bash scripts/test-linux.sh debian:13` and `ubuntu:24.04` exit 0 and the second run changes
  nothing. Needs podman or docker. `--keep` leaves the container, `--reuse` reruns your edits.

## Profiles

`dotfiles_profile` picks one per machine: `DOTFILES_PROFILE`, else `PROFILE` in the settings, else
`owner` when `~/.config/dotfiles/owner.md` exists (saved to the settings), else `shared`.
`DOTFILES_PROFILE=owner` alone is not saved.

- Every profile links `~/.claude/CLAUDE.md` to `claude/CLAUDE.md`. The repo holds nothing personal.
- **owner:** `owner.md` (plain Markdown, never in git) is linked as `~/.claude/rules/owner.md` and
  added to the Cursor global rule. Without it, `dots` warns. Check: `rg -n '^## ' ~/.claude/rules/owner.md`.
- **shared:** no overlay; plugins in `claude/settings.json` are shared as is.
- **Claude settings**, every profile: `dots` builds `~/.claude/settings.json` (a real file) from
  `claude/settings.json` plus the private layer `~/.config/dotfiles/claude-settings.json`; live
  changes move into the layer first (`scripts/agent-config.py claude-settings build`).
  `claude-settings promote <key>` moves a key to the repo base. Owner-only keys stay in the layer.
- **Cursor MCP servers** (`cursor/mcp.json`) are opt-in: `CURSOR_MCP="on"`, or `DOTFILES_MCP=on` once.
- Personal rules go in `owner.md` (`dots settings`), never in `~/.claude/CLAUDE.md`: that is the
  repo file, and a local edit can stop the pull.

## Settings

`~/.config/dotfiles/settings` (one `NAME="value"` per line, mode 600, never in git) holds this
Mac's settings; `settings.example` lists each with a note and default. `dots settings` (also
`dots private`) shows a menu that saves each change at once; Done ends it and `dots` refreshes
links and the Cursor rule. The first run (no file) asks git name and email, then shows the menu.

- Names: `GIT_NAME`, `GIT_EMAIL` (written to `~/.gitconfig.local`), `PROFILE`, `CURSOR_MCP`,
  `OPENCODE_MCP`, `JAVA_HOME` (picked from the Javas found, or typed), `SKIP_STEPS`,
  `PRIVATE_PROJECTS`, `PRIVATE_WORDS` (see "Conventions").
- Readers: `setting` in `scripts/setup-lib.sh`, `setting()` in `scripts/agent-config.py`,
  `home/.config/zsh/settings.zsh` (sourced after `local.zsh`, so its `JAVA_HOME` wins). The last
  line of a name wins. Env vars win for one run: `DOTFILES_PROFILE`, `DOTFILES_MCP`, `DOTFILES_STEPS`.
- Secrets never go in the settings; they stay in the ignored `~/.config/zsh/local.zsh`.

## Conventions

- **Public and impersonal.** Never commit secrets (use `~/.config/zsh/local.zsh`,
  `~/.gitconfig.local`, `Brewfile.local`). The tracked tree names no person: no names, emails,
  accounts, private project names or home-folder paths. One exception: the GitHub handle in the
  default clone URL (`DOTFILES_REPO` in `bootstrap.sh`, the README one-liner). Session notes,
  hand-offs and ledgers stay in the ignored `.agents/`.
- **Private words:** `PRIVATE_PROJECTS`, `PRIVATE_WORDS`, the git email, the full git name and that
  name as one word (a word in the origin URL is left out). The git hooks block a staged file name,
  added line or commit message that holds one. The pre-push hook checks every pushed commit the
  same way (cherry-pick, rebase and `--no-verify` skip the commit hooks) and stops a push that
  brings a second history.
- **Theme:** warm Claude copper-on-stone. Refine it; never swap palettes.
- **Mac and Linux:** one code path where possible; OS-only branches behind `is_macos`/`is_linux`.
- **Idempotent and safe:** steps detect and skip what is installed, back up config before
  overriding, and warn and continue on failure. Source `scripts/setup-lib.sh`.
- **Explicit linking:** `./dotfiles` links only the paths in its `LINKS` array. A new managed file
  means a new entry there.
- **New machine:** `bootstrap.sh` installs Xcode CLT, clones `DOTFILES_REPO` and runs `./dotfiles`.
  At a terminal the first run is guided (gum menus: git identity, settings, an optional extra, the
  sign-ins in `90-sign-in.sh`); without one it reads name and email from stdin and skips the rest.
