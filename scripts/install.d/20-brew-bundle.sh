#!/usr/bin/env bash
# Install Brewfile packages. Looks before it leaps: `brew bundle check`
# short-circuits when everything is already present, so satisfied machines
# do zero work — this runs every time but is cheap when there's nothing to do.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Homebrew packages"
brew_env

have brew || { warn "Homebrew missing — skipping brew bundle."; exit 0; }

# bundle_one <file> <label> — install only what's missing, skip if all present.
bundle_one() {
  local file="$1" label="$2"
  [[ -f "$file" ]] || return 0
  if brew bundle check --file="$file" >/dev/null 2>&1; then
    skip "$label (every formula/cask already installed)"
    return 0
  fi
  info "$label — installing only the missing formulae/casks…"
  # Don't abort the whole run on partial failures (e.g. brew trying to "adopt"
  # an app already in /Applications needs sudo). Warn and continue; the rest of
  # the setup (shell, toolchains, Claude links) should still run.
  if ! brew bundle --file="$file" --no-upgrade; then
    warn "Some $label entries failed (often a pre-installed app needing sudo to"
    warn "adopt, or a Homebrew permissions issue). Fix perms and re-run:"
    warn "  $REPO/scripts/fix-brew-permissions.sh  &&  ./dotfiles"
  fi
}

bundle_one "$REPO/Brewfile"        "Brewfile"
bundle_one "$REPO/Brewfile.local"  "Brewfile.local"   # per-machine extras (gitignored)
ok "brew packages satisfied"
