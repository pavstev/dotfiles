#!/usr/bin/env bash
# Enable Touch ID for sudo via /etc/pam.d/sudo_local.
# sudo_local is included by /etc/pam.d/sudo and survives macOS upgrades.
# Safe to re-run: exits early if the file already contains pam_tid.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

only_macos "Touch ID for sudo"

step "Touch ID for sudo"

SUDO_LOCAL="/etc/pam.d/sudo_local"

if grep -q "pam_tid" "$SUDO_LOCAL" 2>/dev/null; then
  skip "Touch ID for sudo"
  exit 0
fi

if ! sudo -n true 2>/dev/null; then
  info "Need sudo to write $SUDO_LOCAL — you may be prompted for your password once."
fi

if sudo tee "$SUDO_LOCAL" > /dev/null << 'EOF'
# sudo_local: local config file which survives system update and is included by /etc/pam.d/sudo
auth       sufficient     pam_tid.so
EOF
then
  ok "Touch ID for sudo enabled ($SUDO_LOCAL created)."
else
  warn "Couldn't write $SUDO_LOCAL (no sudo/no TTY for a password prompt) — re-run: ./dotfiles"
fi
