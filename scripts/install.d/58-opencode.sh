#!/usr/bin/env bash
# 58-opencode.sh: Opencode (the `opencode` CLI) from Homebrew, already in the Brewfile.
# Skips when `opencode` is present; warns when brew could not install it.
# Its config comes from claude/ via `agent-config.py opencode` (wired in ./dotfiles).
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

brew_env   # on a fresh Linux the Homebrew from step 10 is not on PATH yet
step "Opencode"
if have opencode; then
  skip "Opencode"
else
  warn "Opencode is missing. The Brewfile installs it (brew \"opencode\"); run dots again."
fi
