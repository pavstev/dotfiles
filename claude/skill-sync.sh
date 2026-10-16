#!/usr/bin/env bash
# Sync this repo's skills into every installed agent tool, from ONE source.
#
#   Claude Code   ~/.claude/skills   ADDED per-skill, non-destructively: your own
#                                    skills there are left untouched; ours appear
#                                    alongside them. Re-runnable & safe.
#   Cursor        reads ~/.claude/skills itself, so nothing is linked for it.
#   Opencode      reads ~/.claude/skills itself, so nothing is linked for it either.
#
# ~/.claude/skills is a REAL directory, not a whole-dir symlink: plugins and other
# tools may install their own skill folders there. A whole-dir link would delete
# them on every run. If it is a symlink, link_each() leaves it alone.
#
# Skill discovery skips `_template`, dotfiles, and loose files (README/SOURCES).
# Cursor and Opencode read the SKILL.md / agentskills.io format natively. The spec
# allows only a-z, 0-9 and "-" in `name:`. Our `ps:*` names still load in both,
# because their loaders do not check the name (docs/cursor-port.md, "Gaps";
# docs/opencode-port.md, "Skills").
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

# --- Claude: per-skill symlinks, ADDITIVE and NON-DESTRUCTIVE ---------------
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
  if [[ -L "$root" ]]; then
    echo "!  $root is a symlink, left untouched (no skills added there)" >&2
    return 0
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

# --- Cursor and Opencode: no links --------------------------------------------
# Both load ~/.claude/skills natively (Cursor: "Include Third-Party Plugins, Skills,
# and Other Configs", on by default, https://cursor.com/docs/context/skills,
# 2026-10-13; Opencode: Claude Code compatibility, https://opencode.ai/docs/rules/).
# A second copy in ~/.cursor/skills adds nothing (3.24.12 drops duplicate paths) and
# older builds listed such skills twice (forum.cursor.com/t/150137); ~/.config/opencode
# has no skills dir at all. So this script never touches either folder.

echo "✓ skill-sync complete"
