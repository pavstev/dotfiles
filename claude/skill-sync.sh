#!/usr/bin/env bash
# Sync this repo's skills into every installed agent tool, from ONE source.
#
#   Claude Code   ~/.claude/skills   ┐ ADDED per-skill, non-destructively: your
#   Codex         ~/.codex/skills     ┘ own skills there are left untouched; ours
#                                       appear alongside them. Re-runnable & safe.
#   Cursor        reads ~/.claude/skills itself (see unlink_ours below).
#
# ~/.claude/skills is a REAL directory, not a whole-dir symlink: plugins and other
# tools may install their own skill folders there. A whole-dir link would delete
# them on every run. An existing whole-dir link is migrated by link_each().
#
# Skill discovery skips `_template`, dotfiles, and loose files (README/SOURCES).
# Cursor & Codex read the SKILL.md / agentskills.io format natively. The spec allows
# only a-z, 0-9 and "-" in `name:`. Our `ps:*` names still load in Cursor, because
# its loaders do not check the name (docs/cursor-port.md, "Gaps").
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/skills" && pwd)"

# Names of the real skill folders (has SKILL.md; not _template / dotfile / file).
list_skills() {
  local d name
  for d in "$SRC"/*/; do
    [[ -d "$d" ]] || continue
    name="$(basename "$d")"
    [[ "$name" == _* || "$name" == .* ]] && continue
    [[ -f "$d/SKILL.md" ]] || continue
    printf '%s\n' "$name"
  done
}

# --- Claude / Codex: per-skill symlinks, ADDITIVE and NON-DESTRUCTIVE -------
# prune_stale <root> — remove OUR OWN broken symlinks (target was under $SRC but
# no longer exists, e.g. a skill folder we renamed or deleted). Never touches a
# real directory or a symlink pointing anywhere else — those are the user's.
prune_stale() {
  local root="$1" link target
  for link in "$root"/*; do
    [[ -L "$link" ]] || continue
    target="$(readlink "$link")"
    [[ "$target" == "$SRC"/* ]] || continue
    [[ -e "$target" ]] && continue
    rm -f "$link" && echo "·  removed stale $link (target no longer exists: $target)"
  done
}

link_each() { # <skills-root>
  local root="$1" name link target
  # Migrate an old whole-dir symlink (root → $SRC) to a real dir of per-skill links.
  if [[ -L "$root" ]]; then
    if [[ "$(readlink "$root")" == "$SRC" ]]; then
      rm -f "$root" && echo "!  $root was a whole-dir link — migrating to per-skill links"
    else
      echo "!  $root is a symlink elsewhere — leaving it untouched (skills not added here)" >&2
      return 0
    fi
  fi
  mkdir -p "$root" 2>/dev/null || { echo "!  can't create $root — skipping" >&2; return 0; }
  prune_stale "$root"
  while IFS= read -r name; do
    link="$root/$name"; target="$SRC/$name"
    if [[ -L "$link" && "$(readlink "$link")" == "$target" ]]; then
      continue                                              # already ours — idempotent
    fi
    if [[ -e "$link" || -L "$link" ]]; then
      echo "·  $link exists (not ours) — left untouched"    # NEVER clobber the user's
      continue
    fi
    ln -s "$target" "$link" 2>/dev/null && echo "✓ $link → $target" || echo "!  can't link $link" >&2
  done < <(list_skills)
}

# This repo links only the skills it owns. A private skills repo ships its own
# installer; it never links into this public repo.

# Claude Code (always). Per-skill links so skills from other sources survive.
mkdir -p "$HOME/.claude" 2>/dev/null || true
link_each "$HOME/.claude/skills"

# --- Cursor: no links of ours ------------------------------------------------
# Cursor loads ~/.claude/skills natively ("Include Third-Party Plugins, Skills, and
# Other Configs", on by default; https://cursor.com/docs/context/skills, 2026-10-13).
# A second copy in ~/.cursor/skills adds nothing (3.24.12 drops duplicate paths) and
# older builds listed such skills twice (forum.cursor.com/t/150137). So we only remove
# the links we made there before.
# unlink_ours <root>: remove symlinks that point into $SRC; touch nothing else.
unlink_ours() {
  local root="$1" link
  if [[ -L "$root" ]]; then
    [[ "$(readlink "$root")" == "$SRC" ]] && rm -f "$root" && echo "·  removed $root (whole-dir link of ours)"
    return 0
  fi
  [[ -d "$root" ]] || return 0
  for link in "$root"/*; do
    [[ -L "$link" ]] || continue
    [[ "$(readlink "$link")" == "$SRC"/* ]] || continue
    rm -f "$link" && echo "·  removed $link (Cursor reads ~/.claude/skills)"
  done
}

# Other agents: only if installed.
[[ -d "$HOME/.cursor" ]] && unlink_ours "$HOME/.cursor/skills"
[[ -d "$HOME/.codex"  ]] && { echo "›› Codex detected — adding skills (non-destructive)";   link_each "$HOME/.codex/skills"; }

echo "✓ skill-sync complete"
