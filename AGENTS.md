# Working in this repo — agent guide

This is a **plain-git dotfiles repo** — no dotfiles manager, no templating
engine. Your machine is a *projection* of it, wired up by one script. This
file is the operating manual for any AI agent (Claude Code, Cursor, Codex, …)
making changes here. Read it before editing. Cursor/Codex pick it up automatically
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
| `scripts/private.sh` | the private settings questions (private rules, private words, Java); run by the first run and by `dots private`. |
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

- **Claude** → `~/.claude/skills`, **Codex** → `~/.codex/skills` — added **per-skill,
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
2. `~/.config/dotfiles/profile`.
3. `owner` if private rules (`~/.config/dotfiles/owner.md`) exist. The answer is saved to step 2.
4. Else `shared` (the default).

Every profile links `~/.claude/CLAUDE.md` to `claude/CLAUDE.md`. The repo holds nothing
personal; private rules live outside it.

- **owner:** the private rules `~/.config/dotfiles/owner.md` (plain Markdown, never in
  git) are linked as `~/.claude/rules/owner.md` (Claude Code loads user rules in every
  project) and added to the Cursor global rule. Without the file, `dots` warns. Cursor
  MCP servers are on, unless `~/.config/dotfiles/mcp` says `off`.
- **shared:** no overlay. Cursor MCP servers (`cursor/mcp.json`) are opt-in:
  `echo on > ~/.config/dotfiles/mcp` (or `DOTFILES_MCP=on`), then `dots`. Claude plugins
  in `claude/settings.json` are shared as is.
- **Private settings** (`scripts/private.sh`): the first run at a terminal asks for the
  private rules (`owner.md`) and the private words (`private-words`), both mode 600 in
  `~/.config/dotfiles`, and for the Java to use. Java: it looks in the usual places
  (`java_home`, `/Library/Java`, Homebrew, mise, SDKMAN, asdf, jenv, Gradle, jbang,
  Android Studio), offers the current one or else the newest long-term-support one
  first, or takes a typed path. It writes `JAVA_HOME` in a marked block at the end of
  the ignored `home/.config/zsh/local.zsh` and leaves the other lines alone.
  `dots private` asks again later, then refreshes the links and the Cursor rule. The
  first run also asks about Cursor MCP on every profile and keeps the answer.
  `DOTFILES_PROFILE=owner` alone is not saved. Check: `rg -n '^## ' ~/.claude/rules/owner.md`.
- **Personal rules** go in the private rules (`dots private`), not in
  `~/.claude/CLAUDE.md`. That file is the repo file, and a local edit can stop the pull.

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
  Private words: `~/.config/dotfiles/private-words` (one per line; `dots private` edits
  it). The git hooks then block a staged file name, added line or commit message that
  holds one.
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
email, private rules and words, optional extras, then the sign-ins in
`scripts/install.d/90-sign-in.sh`. Without a
terminal (a pipe, a test) it reads plain lines from stdin and skips the extras and sign-ins.
