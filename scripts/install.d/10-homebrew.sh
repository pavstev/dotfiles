#!/usr/bin/env bash
# Ensure Homebrew is installed. Idempotent.
# Tolerant: a failed install warns and lets the rest of the run continue — the
# brew-bundle step downstream simply skips itself when brew is absent.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

# Homebrew is macOS and Linux only. On Windows, WinGet installs the tools
# (see scripts/install.d/15-winget.sh); this step skips there.
is_windows && { skip "Homebrew (not on Windows — WinGet installs the tools)"; exit 0; }

step "Homebrew"
brew_env
if have brew; then
  if verbose; then skip "Homebrew"; fi   # the Homebrew packages step shows brew works
  exit 0
fi

# Homebrew on Linux needs glibc. On musl (Alpine) say so once; the brew steps skip.
if is_musl; then
  warn "Homebrew needs glibc, and this Linux uses musl (Alpine). The brew steps skip."
  exit 0
fi

info "installing Homebrew"
if ensure_brew; then
  ok "Homebrew installed"
else
  warn "Homebrew install failed. The brew steps skip. Fix the error above, then run dots again."
fi
