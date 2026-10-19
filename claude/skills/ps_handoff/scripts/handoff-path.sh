#!/usr/bin/env bash
# handoff-path.sh <slug> — print a fresh hand-off file path and create its folder.
# In a git repo: <toplevel>/.agents/handoffs/<date>-<slug>.md (folder kept out of git
# through .git/info/exclude). Elsewhere: ~/.claude/handoffs/<date>-<slug>.md.
set -euo pipefail

slug="${1:-}"
[ -n "$slug" ] || { echo "usage: handoff-path.sh <slug>" >&2; exit 2; }
slug="$(printf '%s' "$slug" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//')"
[ -n "$slug" ] || { echo "slug has no usable characters" >&2; exit 2; }

date="$(date +%F)"

if top="$(git rev-parse --show-toplevel 2>/dev/null)"; then
  dir="$top/.agents/handoffs"
  mkdir -p "$dir"
  if ! git -C "$top" check-ignore -q "$dir/x.md"; then
    exclude="$(git -C "$top" rev-parse --git-path info/exclude)"
    case "$exclude" in /*) ;; *) exclude="$top/$exclude" ;; esac
    mkdir -p "$(dirname "$exclude")"
    printf '.agents/handoffs/\n' >> "$exclude"
  fi
else
  dir="$HOME/.claude/handoffs"
  mkdir -p "$dir"
fi

path="$dir/$date-$slug.md"
n=2
while [ -e "$path" ]; do
  path="$dir/$date-$slug-$n.md"
  n=$((n + 1))
done
printf '%s\n' "$path"
