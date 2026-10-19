#!/usr/bin/env bash
# Node LTS + corepack + pnpm via mise. Installs only what's missing.
# Tolerant: a failed network step warns and continues.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Node + pnpm (via mise)"
brew_env

have mise || { warn "mise not found — skipping node/pnpm."; exit 0; }

if mise ls node 2>/dev/null | grep -q .; then
  skip "Node (mise)"
else
  info "Installing Node LTS via mise…"
  mise use -g node@lts || { warn "mise node install failed (network?) — re-run: ./dotfiles"; exit 0; }
  ok "Node LTS installed"
fi

mise exec -- corepack enable >/dev/null 2>&1 || true
if mise exec -- pnpm --version >/dev/null 2>&1; then
  skip "pnpm"
else
  info "Activating pnpm via corepack…"
  mise exec -- corepack prepare pnpm@latest --activate >/dev/null 2>&1 \
    || warn "pnpm activation failed — re-run: ./dotfiles"
fi
ok_if_changed "node + pnpm ready"
