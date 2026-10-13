#!/usr/bin/env bash
# Install a Rust toolchain via rustup (Homebrew installs the manager, not a toolchain).
# Tolerant: a failed network step warns and continues.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Rust toolchain"
brew_env

if have rustup; then
  if rustup show active-toolchain >/dev/null 2>&1; then
    skip "Rust toolchain"
  else
    info "Installing Rust stable toolchain…"
    rustup default stable || { warn "rustup default stable failed (network?) — re-run: ./dotfiles"; exit 0; }
  fi
elif have rustup-init; then
  info "Initialising rustup…"
  rustup-init -y --no-modify-path --default-toolchain stable \
    || { warn "rustup-init failed (network?) — re-run: ./dotfiles"; exit 0; }
else
  warn "rustup not found — skipping Rust toolchain."
  exit 0
fi

rustup component add clippy rust-analyzer >/dev/null 2>&1 || true
ok "Rust ready ($(rustc --version 2>/dev/null || echo 'rustc pending'))"
