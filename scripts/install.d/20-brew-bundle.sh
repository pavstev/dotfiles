#!/usr/bin/env bash
# Install Brewfile packages. Looks before it leaps: `brew bundle check`
# short-circuits when everything is already present, so satisfied machines
# do zero work — this runs every time but is cheap when there's nothing to do.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Homebrew packages"
brew_env

have brew || { warn "Homebrew missing — skipping brew bundle."; exit 0; }

# bundle_run <file> <label> — install only what's missing, skip if all present.
bundle_run() {
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
    if is_macos; then
      warn "Some $label entries failed (often a pre-installed app needing sudo to"
      warn "adopt, or a Homebrew permissions issue). Fix perms and re-run:"
      warn "  $REPO/scripts/fix-brew-permissions.sh  &&  ./dotfiles"
    else
      warn "Some $label entries failed (see above, often the network). Re-run: ./dotfiles"
    fi
  fi
}

# bundle_one <file> <label> — bundle_run on the Brewfile for this OS (on Linux, a temp
# copy without cask, mas, tap and vscode lines, which only work on a Mac).
bundle_one() {
  local file="$1" label="$2" use rc=0
  [[ -f "$file" ]] || return 0
  use="$(brewfile_for_os "$file")" || use="$file"
  if [[ "$use" != "$file" ]]; then trap 'rm -f "$use"; exit 130' INT TERM; fi
  bundle_run "$use" "$label" || rc=$?
  if [[ "$use" != "$file" ]]; then rm -f "$use"; fi
  return "$rc"
}

bundle_one "$REPO/Brewfile"        "Brewfile"
bundle_one "$REPO/Brewfile.local"  "Brewfile.local"   # per-machine extras (gitignored)
ok "brew packages satisfied"
