#!/usr/bin/env bash
# ============================================================================
#  scripts/fix-brew-permissions.sh
#
#  Fix "Permission denied" / "directory not writable" errors from Homebrew
#  when the install is owned by a DIFFERENT macOS user than the one running
#  brew (common on shared machines / after a user migration).
#
#  It makes the whole Homebrew tree group-writable by the `admin` group and
#  sets the setgid bit so future installs by any admin user stay shared.
#
#  Requires sudo (you will be prompted for your password). Safe to re-run.
# ============================================================================
set -euo pipefail

# Resolve the Homebrew prefix (Apple Silicon or Intel).
if command -v brew >/dev/null 2>&1; then
  PREFIX="$(brew --prefix)"
elif [[ -d /opt/homebrew ]]; then
  PREFIX="/opt/homebrew"
elif [[ -d /usr/local/Homebrew ]]; then
  PREFIX="/usr/local"
else
  echo "Homebrew not found — nothing to fix." >&2
  exit 1
fi

echo "Fixing permissions on: $PREFIX"
echo "(You will be asked for your macOS password.)"

sudo chgrp -R admin "$PREFIX"
sudo chmod -R g+rwX "$PREFIX"
sudo find "$PREFIX" -type d -exec chmod g+s {} +

echo "Done. Homebrew is now writable by the 'admin' group."
