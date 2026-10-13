# Working in this repo — agent guide

This is a **plain-git dotfiles repo** — no dotfiles manager, no templating
engine. Your machine is a *projection* of it, wired up by one script. This
file is the operating manual for any AI agent (Claude Code, Cursor, …)
making changes here. Read it before editing. Cursor picks it up automatically
as `AGENTS.md`; Claude Code reads it via the repo's `CLAUDE.md` (which imports this).

## The one rule

**Edit the source, never `$HOME` by surprise.** Most of `$HOME` is **symlinked**
straight from `home/` and `claude/` — editing either side edits the same file, so
there's no separate "apply" step for content changes. The one thing to watch:
new files, package installs, and machine setup only take effect after running
`./dotfiles`.

- A managed dotfile (e.g. shell, prompt, terminal, git) → edit under `home/` —
  it's live immediately if already linked; run `./dotfiles` once if it's new.
- Claude config (settings, skills, agents, hooks) → edit under `claude/`.
- Never assume a `$HOME` file is a plain copy — check `ls -la` if unsure; if it's
  a symlink back into this repo (the common case), your edit already *is* the source.

## Layout

| Path | What |
|------|------|
| `dotfiles` | the one command: git pull, link files, run provisioning steps. |
| `home/` | mirrors `$HOME` 1:1 (`.zshrc`, `.gitconfig`, `.config/…`) — only the paths listed in `dotfiles`'s link array are ever touched. |
| `scripts/install.d/` | ordered (`NN-name.sh`), skip-aware, fault-tolerant provisioning steps (see `scripts/setup-lib.sh`). |
| `claude/` | Claude config, symlinked **per-item** into `~/.claude`. Skills in `claude/skills/`. |
| `cursor/` | Cursor-only sources: `mcp.json` (MCP servers merged into `~/.cursor/mcp.json`) and `hooks/guard_cursor.py` (+ test), the Cursor hook adapter for guard G3. |
| `scripts/agent-config.py` | Python stdlib generator. `cursor` writes and merges the files under `~/.cursor`; `user-rules` prints the text for Cursor's User Rules box. |
| `docs/guide.md` | the full user guide; `README.md` is the short start page. |
| `docs/cursor-port.md` | Cursor port design: profiles, the Claude to Cursor mapping, and the gap table. |
| `Brewfile` | every package (per-machine extras in gitignored `Brewfile.local`). |
| `settings.example` | every setting with a note and its default. This Mac's copy is `~/.config/dotfiles/settings` (see "Settings"). |
| `scripts/settings.sh` | the settings menu: `dots settings` (also `dots private`) and the first run. |
| `scripts/setup-lib.sh` | shared step helpers (`step`/`info`/`ok`/`warn`/`skip`/`have`/`brew_env`/`link_item`) and `dotfiles_profile`. |
| `bootstrap.sh` | fresh-Mac one-liner. |

## Sync & update

```sh
dots                          # alias for ./dotfiles: pull, link, provision — the ONE command
git -C ~/dotfiles diff        # dotd — preview incoming/local source changes
```

**No background agent.** This machine (wherever you're editing from) is the
only one that pushes; other machines just run `dots` on demand to catch up.
Commit and push your own changes when you're ready — nothing does it for you.

## Skills (work in Claude **and** Cursor)

Skills are `SKILL.md` folders in `claude/skills/`, synced by `claude/skill-sync.sh`:

- **Claude** → `~/.claude/skills` — added **per-skill,
  non-destructively**: existing skills there are never touched (on a name clash, the
  local one wins). `_template/` and loose docs are skipped.
- **Cursor** reads `~/.claude/skills` natively ("Include Third-Party Plugins, Skills,
  and Other Configs", on by default). The script does not link skills into
  `~/.cursor/skills` and never touches that folder.

To add a skill: copy `claude/skills/_template/` to `claude/skills/<name>/`, keep the
folder name == the `name:` in `SKILL.md`, then run `dots` (or `claude/skill-sync.sh` directly).

## Cursor

`claude/` is the source for Cursor too. **Edit `claude/`, then run `dots`.** Step
`scripts/install.d/60-cursor.sh` runs only when Cursor is present. It installs the CLI
if missing, installs the Brewfile `vscode` extensions into Cursor, and runs
`scripts/agent-config.py cursor`. Full mapping and gaps: `docs/cursor-port.md`.

What Cursor gets (linked, native, generated, merged): `docs/guide.md` "Cursor" and
the mapping table in `docs/cursor-port.md`.

- Generated Markdown carries a `generated-by: dotfiles` marker. A file without it is
  the user's, and the script leaves it alone.
- Never symlink `~/.cursor/cli-config.json` or `~/.cursor/mcp.json`. Cursor rewrites them.
- Rules in every folder (also outside `$HOME`): run `cursor-rules`, then paste in Cursor
  Settings > Rules > User Rules. `dots` then skips the file rule and warns when to paste again.
- Never edit the generated files under `~/.cursor`. The next `dots` overwrites them.
- Unit tests: `python3 scripts/test_agent_config.py`, `python3 cursor/hooks/test_guard_cursor.py`
  and `python3 claude/hooks/test_guard.py`. They must also pass on `/usr/bin/python3` (3.9).
- Test in a temp HOME: `HOME=$(mktemp -d) DOTFILES_STEPS=60-cursor ./dotfiles`.
  `DOTFILES_STEPS` takes install.d step names without `.sh`; empty runs all steps.
  It filters only the install.d steps: the pull, git identity prompt, links and
  skill sync still run.

## Profiles

`dotfiles_profile` in `scripts/setup-lib.sh` picks one profile per machine:

1. `DOTFILES_PROFILE` env var.
2. `PROFILE` in the settings (`~/.config/dotfiles/settings`).
3. `owner` if private rules (`~/.config/dotfiles/owner.md`) exist. The answer is saved to step 2.
4. Else `shared` (the default).

Every profile links `~/.claude/CLAUDE.md` to `claude/CLAUDE.md`. The repo holds nothing
personal; private rules live outside it.

- **owner:** the private rules `~/.config/dotfiles/owner.md` (plain Markdown, never in
  git) are linked as `~/.claude/rules/owner.md` (Claude Code loads user rules in every
  project) and added to the Cursor global rule. Without the file, `dots` warns.
- **shared:** no overlay. Claude plugins in `claude/settings.json` are shared as is.
- **Cursor MCP servers** (`cursor/mcp.json`) are opt-in on every profile: turn them on
  in `dots settings` (`CURSOR_MCP="on"`), or `DOTFILES_MCP=on` for one run.
- `DOTFILES_PROFILE=owner` alone is not saved. Check the owner rules:
  `rg -n '^## ' ~/.claude/rules/owner.md`.
- **Personal rules** go in the private rules (`dots settings`), not in
  `~/.claude/CLAUDE.md`. That file is the repo file, and a local edit can stop the pull.

## Settings

One private file holds the settings of this Mac: `~/.config/dotfiles/settings`, one
`NAME="value"` line each, mode 600, never in git. `settings.example` lists every name
with a note and its default. `dots settings` (also `dots private`) opens a menu that
shows each setting with its value: pick one, change it, and it saves at once. Done
ends the menu, then `dots` refreshes the links and the Cursor rule. The first run is
the run without the file: it asks for the git name and email, then shows the menu once,
with the cursor on Done.

| Setting | What it does |
|---|---|
| `GIT_NAME`, `GIT_EMAIL` | git identity; `dots` writes it to `~/.gitconfig.local` |
| `PROFILE` | `owner` or `shared` (see "Profiles") |
| `CURSOR_MCP` | `on` or `off` (empty: off). MCP servers are opt-in on every profile |
| `JAVA_HOME` | the Java folder new shells use, picked from the Javas found (`java_home`, `/Library/Java`, Homebrew, mise, SDKMAN, asdf, jenv, Gradle, jbang, Android Studio) or typed |
| `SKIP_STEPS` | `scripts/install.d` steps that never run on this Mac |
| `PRIVATE_PROJECTS`, `PRIVATE_WORDS` | words the git hooks keep out of commits (see "Conventions") |

- The private rules stay free text in `~/.config/dotfiles/owner.md`. The menu edits them.
- Secrets never go in the settings. They stay in the ignored `~/.config/zsh/local.zsh`.
- Readers: `setting` in `scripts/setup-lib.sh`, `setting()` in `scripts/agent-config.py`,
  and `home/.config/zsh/settings.zsh`. `~/.zshrc` sources that file after `local.zsh`,
  so its `JAVA_HOME` wins. The last line of a name wins.
- Env vars still win for one run: `DOTFILES_PROFILE`, `DOTFILES_MCP`, `DOTFILES_STEPS`.

## Conventions

- **Theme:** warm Claude **copper-on-stone** palette (stone bg, copper accent). Refine
  it; do not swap to other palettes.
- **Secrets:** this repo is **public**. Never commit secrets — use the gitignored
  `~/.config/zsh/local.zsh` / `~/.gitconfig.local` / `Brewfile.local`.
- **Impersonal:** the tracked tree names no person: no names, emails, accounts, private
  project names or home-folder paths. One exception, chosen by the owner: the GitHub handle
  in the default clone URL (`DOTFILES_REPO` in `bootstrap.sh`, and the README one-liner).
  Private rules go in `~/.config/dotfiles/owner.md`.
  Session notes, hand-offs and ledgers stay in the ignored `.agents/`.
  Private words: `PRIVATE_PROJECTS` and `PRIVATE_WORDS` in the settings, plus the git
  email, the full git name and that name as one word (a word in the origin URL is left
  out). The git hooks block a staged file name, added line or commit message that
  holds one. The pre-push hook checks every commit a push adds the same way (cherry-pick,
  rebase and `--no-verify` skip the commit hooks). It also stops a push that brings a
  second history (another first commit), such as an old private history.
- **Idempotent & safe:** setup steps detect what's installed and skip it, back up
  pre-existing config before override, and warn-and-continue on failure. Keep new steps
  that way (source `scripts/setup-lib.sh`).
- **Explicit linking:** `./dotfiles` only symlinks paths explicitly listed in its
  `LINKS` array — nothing else in `$HOME` is ever touched. Adding a new managed
  file means adding it to that list.
- **Modern CLI defaults:** `rg`, `fd`, `eza`, `bat`, `zoxide`, `fzf`, `delta`. Commits:
  concise, imperative subject.

## New machine

Run the one-liner in README.md "Set up a Mac". `bootstrap.sh` installs Xcode CLT, clones
this repo (`DOTFILES_REPO`, set at its top), and runs `./dotfiles` end to end. At a
terminal the first run is guided (gum menus, `scripts/setup-lib.sh`): git name and
email, the settings menu, an optional extra, then the sign-ins in
`scripts/install.d/90-sign-in.sh`. Without a terminal (a pipe, a test) it reads the
name and email as plain lines from stdin and skips the menu, the extra and the sign-ins.
