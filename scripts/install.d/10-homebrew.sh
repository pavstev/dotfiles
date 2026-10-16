#!/usr/bin/env bash
# Ensure Homebrew is installed. Idempotent.
# Tolerant: a failed install warns and lets the rest of the run continue — the
# brew-bundle step downstream simply skips itself when brew is absent.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

# Homebrew is macOS and Linux only. On Windows, WinGet installs the tools
# (see scripts/install.d/15-winget.sh); this step skips there.
is_windows && { skip "Homebrew (not on Windows — WinGet installs the tools)"; exit 0; }

brew_env
if have brew; then
  skip "Homebrew"
  exit 0
fi

step "Installing Homebrew"
if ensure_brew; then
  ok "Homebrew installed"
else
  warn "Homebrew install failed (network or permissions). Downstream brew steps will skip;"
  warn "fix the cause and re-run: ./dotfiles"
fi
