# Depersonalize and optimize: inventory

Snapshot of `main` at a568658 (2026-10-13): 110 tracked files. This file lists what tied the
tracked tree to its owner, what was duplicated or stale, and what git history still holds.
Personal strings are described here, never quoted.

## 1. Personal data in the tracked tree

| # | What | Where | Action |
|---|------|-------|--------|
| P1 | Owner's GitHub handle in the clone URL and the bootstrap one-liner | `bootstrap.sh:7,11`, `README.md:65,93`, `AGENTS.md:129` | Owner choice: the handle stays as the default. One `DOTFILES_REPO` variable in `bootstrap.sh`, one copy of the one-liner in `README.md`; other docs link to it. |
| P2 | Handle as the copyright holder | `LICENSE:3` | Neutral holder. |
| P3 | Owner's first name in plan headers | `docs/plans/2026-10-12-*.md` | Files deleted. |
| P4 | Owner's personal domain in a skill provenance note | `claude/skills/SOURCES.md:57` | Reworded. |
| P5 | Off-limits rule that names a personal Cloudflare account and a mail address | `claude/CLAUDE.md:34-41` | Moved to the untracked owner overlay. |
| P6 | First-person biography line (a long session anecdote) | `claude/CLAUDE.md:45` | Neutral line in the repo; the anecdote moves to the overlay. |
| P7 | Home paths that hold the account name | `.agents/research/verify-friend.md:3,35`, `.agents/research/verify-harness.md:3` | Files deleted. |
| P8 | Names of the owner's private projects, session stats, a work machine | `docs/plans/2026-10-12-*.md`; example rows in `claude/skills/ps_sessions/SKILL.md:35-36` | Plans deleted. Example rows renamed, with a skill test. |
| P9 | Name of a skill from a private work repo | `claude/skill-sync.sh:74-85` (history comment) | Comment removed. |
| P10 | Lists of the owner's `~/.claude` (with a private tools folder) | `.agents/research/claude-manifest-{before,after}.txt` | Deleted. The manifest check runs from a scratch copy. |
| P11 | Session notes about the owner's Macs, accounts and apps | `.agents/research/*.md` (9 notes) | Deleted. The durable facts are in `docs/cursor-port.md`; its source links stay there. |
| P12 | Profile detection keyed on the owner-only heading | `scripts/setup-lib.sh:73-89`, `scripts/agent-config.py:33,155-162` | Replaced: the owner overlay file marks an owner Mac. |
| P13 | Default clone path `~/dotfiles` in rules and in `Read(~/dotfiles/**)` | `claude/CLAUDE.md`, `claude/settings.json:29`, docs | Kept. It is the bootstrap default for every user, not personal. |
| P14 | Name of a work platform in a shell comment, next to a work-only Java pin | `home/.config/zsh/init.zsh:111-121` | Moved to the owner's ignored `~/.config/zsh/local.zsh`. |

Not personal, kept: `example.com` test values, the `ELI5` output style (written in the first
person, but it is a reply style any user can pick), the profile names `owner` and `shared`.

## 2. Duplicated code

| # | What | Where | Action |
|---|------|-------|--------|
| D1 | Shared-profile CLAUDE.md pipeline: generated copy, owner-section cut, sha256 sidecar, hand-edit backups | `scripts/agent-config.py` (`claude-md`, `OWNER_ONLY`, `drop_sections`), `dotfiles:96-105`, `.generated/` | Removed. With no owner text in the source, every profile links `claude/CLAUDE.md`. |
| D2 | Profile order written twice (bash and Python) | `scripts/setup-lib.sh` `dotfiles_profile`, `scripts/agent-config.py` `default_profile` | Kept as two short functions, same order, both tested. |
| D3 | Homebrew install in two places | `bootstrap.sh:26-34`, `scripts/install.d/10-homebrew.sh` | Removed from `bootstrap.sh`; git comes with Xcode CLT, `./dotfiles` installs Homebrew. |
| D4 | A two-line `setup-lib.sh` guard in 9 install steps | `scripts/install.d/*.sh` | One `source` line, like `60-cursor.sh`; `./dotfiles` already stops without the lib. |
| D5 | `~/.local/bin` added to PATH twice | `home/.zshrc:15-16`, `home/.config/zsh/init.zsh:133` | Removed from `.zshrc`. |
| D6 | The Cursor file table in three docs | `AGENTS.md`, `README.md`, `docs/cursor-port.md` | `AGENTS.md` links to the other two. |

## 3. Dead files and stale docs

| # | What | Where | Action |
|---|------|-------|--------|
| S1 | Finished session plans and status | `docs/plans/2026-10-12-*.md`, `docs/plans/2026-10-13-cursor-port-progress.md` | Deleted. Decisions that still explain the Cursor design move to `docs/cursor-port.md`. |
| S2 | Research and verify notes of one session | `.agents/research/*` (11 files) | Deleted. `.agents/` is now ignored, so session notes stay local. |
| S3 | `skill-sync.sh` said to link into `~/.cursor/skills` | `claude/skills/README.md:9-11` | Fixed. |
| S4 | History comment about a removed feature | `claude/skill-sync.sh:74-85` | Removed. |
| S5 | The backup step ran after the links, so it never saved a file; its own copy of the link list also missed 6 paths | `scripts/install.d/05-backup-existing.sh` | Deleted. The real backup is `link_item`'s `<file>.bak-<timestamp>`; README says so now. |
| S6 | `skill status` read the log of a sync agent that no longer exists; `skill publish` was a stub | `home/.config/zsh/skill.zsh` | Both parts removed. |
| S7 | Stale comments and claims | `scripts/git-hooks/pre-commit:4`, `scripts/install.d/70-macos-defaults.sh:41`, `75-terminal-appearance.sh:14`, `claude/hooks/README.md:40-54`, `README.md:176` | Fixed. |

## Kept on purpose

- `claude/skills-parked/`: the owner parked `ps:teach` on 2026-10-12.
- `claude/hooks/format-after-edit.sh`: a documented opt-in sample.
- `legacy_rule` in `agent-config.py` and `unlink_ours` in `skill-sync.sh`: migrations for
  Macs that have not run `dots` since the first Cursor port. Removed later the same day:
  no Mac needs them any more.
- Shared state helpers for the two guards, shared test helpers: small payoff, and the
  guard must stay fail-open.
- About 30 lines shared by `whatsleft-scan.py` and `hurry-scan.py`: each skill change needs
  a subagent test cycle, which costs more than the lines save.
- Shell aliases that overlap git aliases: muscle memory.

New guard: the pre-commit hook blocks staged lines that hold a word from the untracked
`~/.config/dotfiles/private-words`.

## 4. What git history still holds

Nothing below is rewritten. The owner chose a fresh start instead (2026-10-13): the public
repo begins with one clean commit, and the commits named below stay in a private archive
repo.

- 89 commits. Author lines: 78 with the owner's full name and a personal-domain address,
  7 with the owner's name and a work-domain address, 4 with the handle and the GitHub
  noreply address.
- Commit subjects that name private things: the personal Cloudflare account (c826171), the
  owner's project names (aa124b6), a work platform (59cae72), the owner's private config
  layers (24647a0, c020ebd).
- Every file removed here stays readable in old commits: the Off-limits text and the
  anecdote in `claude/CLAUDE.md`, `docs/plans/2026-10-12-*.md`, `.agents/research/*`, the
  handle in `LICENSE`. So do 43 files deleted earlier, for example a personal permission
  layer.

## 5. Design

- Owner overlay: `~/.config/dotfiles/owner.md`, plain Markdown, outside the repo. On the
  `owner` profile, `dots` links it as `~/.claude/rules/owner.md` (Claude Code loads user rules
  in every project, with no approval prompt) and `agent-config.py` adds it to the Cursor rule
  (`claude-global.mdc`, or the `cursor-rules` text). The `shared` profile never reads it, and
  `dots` removes that link when a Mac moves to `shared`. On the `owner` profile without the
  file, `dots` warns.
- `claude/CLAUDE.md` holds no owner text, so every profile links it directly. The generated
  shared copy (`.generated/`), the section cut and the hand-edit backups go away. `dots`
  replaced an old link to the generated copy and kept that folder for a while. That code
  and the ignore line were removed later the same day: no Mac has the old link any more.
- Profile order: `DOTFILES_PROFILE`, then `~/.config/dotfiles/profile`, then `owner` when the
  overlay file exists (saved to the profile file), else `shared`.
- Clone URL: one `DOTFILES_REPO` variable in `bootstrap.sh` (the environment wins). The owner
  chose to keep their handle as its default. The one-liner appears once, in README
  "Set up a Mac"; other docs link to it.
- `.agents/` is ignored: hand-offs, ledgers and session notes stay on the machine.
- Rejected: an age-encrypted overlay in the repo (owner choice: local file); an `@path` import
  in CLAUDE.md (Cursor does not expand imports); a generated owner CLAUDE.md (edits would need
  `dots` before they apply, and the owner's `~/.claude/CLAUDE.md` link would change).

## 6. Added after the first review (2026-10-13)

- Guided first run (asked for during the pass): `./dotfiles` asks for the git name and
  email with gum menus (arrows and Enter), offers the extras as Yes/No menus, installs
  Claude Code (step 57) and offers the sign-ins (step 90). Without a terminal it reads
  the name and email from stdin and offers nothing else.
- The README is one short page. The long reference moved to `docs/guide.md`.
- Review fixes: the pull has no autostash and no password prompt, and restarts the new
  script after new commits. The old `.generated/` folder was kept at first (removed
  later the same day, with the other old-install code). The Cursor rule that holds `owner.md` text is written 0600. The git hooks also
  check file names, commit messages and absolute symlink targets.
- Not changed: the commit types of be1f47a and a773c41 (no history rewrite), and there is
  no shell test suite for `dotfiles` yet (temp-HOME runs cover it by hand).
