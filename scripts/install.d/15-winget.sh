#!/usr/bin/env bash
# 15-winget.sh: Windows only. Installs Opencode (the `opencode` CLI) via WinGet,
# which is built into Windows 10/11. The Brewfile's `brew "opencode"` does not
# apply on Windows (no Homebrew there), so this step covers it.
# Idempotent and tolerant: skips when `opencode` is present or winget is missing,
# and warns (never fails) when the install cannot complete.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
is_windows || exit 0

step "Opencode via WinGet"

if have opencode; then
  skip "Opencode"
  exit 0
fi

have winget || { warn "winget not found — skipping WinGet tools. Install Opencode manually, e.g. winget install Anomaly.OpenCode"; exit 0; }

info "Installing Opencode via WinGet…"
if winget install --id Anomaly.OpenCode -e --accept-package-agreements --accept-source-agreements --silent >/dev/null 2>&1; then
  ok "Opencode installed (sign in once: opencode auth login)"
else
  warn "WinGet could not install Opencode (package not in the store yet, or no network)."
  warn "Install manually: winget install Anomaly.OpenCode  (or see the Opencode docs for other Windows options)"
fi
