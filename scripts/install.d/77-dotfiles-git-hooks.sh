#!/usr/bin/env bash
# One-time: wire the git hooks of the dotfiles repo itself.
# Sets core.hooksPath in ~/dotfiles/.git/config (not in ~/.gitconfig).
# The hooks in scripts/git-hooks/ then run on every `git commit` and `git push`.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Dotfiles repo git hooks"

HOOKS_DIR="scripts/git-hooks"

if [[ ! -d "$REPO/$HOOKS_DIR" ]]; then
  warn "scripts/git-hooks/ not found — skipping hook wiring"
  exit 0
fi

current=$(git -C "$REPO" config core.hooksPath 2>/dev/null || true)
if [[ "$current" == "$HOOKS_DIR" ]]; then
  skip "core.hooksPath (already $HOOKS_DIR)"
  exit 0
fi

if git -C "$REPO" config core.hooksPath "$HOOKS_DIR"; then
  ok "core.hooksPath = $HOOKS_DIR (the git hooks are now active)"
else
  warn "Failed to set core.hooksPath — run manually:"
  warn "  git -C ~/dotfiles config core.hooksPath scripts/git-hooks"
fi
