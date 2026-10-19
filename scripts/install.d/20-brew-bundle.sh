#!/usr/bin/env bash
# Install Brewfile packages. Looks before it leaps: `brew bundle check`
# short-circuits when everything is already present, so satisfied machines
# do zero work — this runs every time but is cheap when there's nothing to do.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Homebrew packages"
brew_env

have brew || { warn "Homebrew missing — skipping brew bundle."; exit 0; }

# brew_bundle_quiet <file>: brew bundle without its "Using <pkg>" line for each present
# package and its closing count. Installs and warnings print as they come. The exit
# status is brew's. DOTFILES_VERBOSE=1 prints everything.
brew_bundle_quiet() {
  if verbose; then brew bundle --file="$1" --no-upgrade; return; fi
  brew bundle --file="$1" --no-upgrade 2>&1 |
    awk '!/^Using / && !/^`brew bundle` complete/ { print; fflush() }'
}

# bundle_run <file> <label> — install only what's missing, skip if all present.
# --no-upgrade: a package with a newer version still counts as present.
bundle_run() {
  local file="$1" label="$2"
  [[ -f "$file" ]] || return 0
  if brew bundle check --no-upgrade --file="$file" >/dev/null 2>&1; then
    skip "$label (every formula/cask already installed)"
    return 0
  fi
  info "$label — installing only the missing formulae/casks…"
  BUNDLE_CHANGED=1
  # Don't abort the whole run on partial failures (e.g. brew trying to "adopt"
  # an app already in /Applications needs sudo). Warn and continue; the rest of
  # the setup (shell, toolchains, Claude links) should still run.
  if ! brew_bundle_quiet "$file"; then
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

BUNDLE_CHANGED=""
bundle_one "$REPO/Brewfile"        "Brewfile"
bundle_one "$REPO/Brewfile.local"  "Brewfile.local"   # per-machine extras (gitignored)
if [[ -n "$BUNDLE_CHANGED" ]]; then ok "brew packages satisfied"; fi
