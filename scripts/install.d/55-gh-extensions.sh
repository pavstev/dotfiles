#!/usr/bin/env bash
# Install gh CLI extensions. gh-dash (GitHub PR/issue dashboard) is a gh extension,
# invoked as `gh dash` (aliased to `ghd`). Idempotent and tolerant (no set -e).
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

step "gh CLI extensions"
brew_env

have gh || { warn "gh not found — skipping extensions."; exit 0; }

if gh extension list 2>/dev/null | grep -q "dlvhdr/gh-dash"; then
  skip "gh-dash extension"
else
  info "Installing gh-dash extension…"
  gh extension install dlvhdr/gh-dash >/dev/null 2>&1 \
    && ok "gh-dash installed (run: gh dash)" \
    || warn "gh-dash install failed (network or gh auth) — retry: gh extension install dlvhdr/gh-dash"
fi
