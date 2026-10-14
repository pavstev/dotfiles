#!/usr/bin/env bash
# 78-linux-extras.sh: small Linux-only pieces. Linux only: a Mac prints nothing here.
#   1. VS Code extensions from the Brewfile `vscode` lines, when `code` is installed
#      (brew bundle only does this on a Mac)
#   2. a note when the login shell is not zsh (this step never changes it)
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
is_linux || exit 0
brew_env

step "Linux extras"

# 1. VS Code extensions.
if have code && [[ -f "$REPO/Brewfile" ]]; then
  installed="$(code --list-extensions 2>/dev/null | tr '[:upper:]' '[:lower:]')"
  added=0 failed=0
  while read -r ext; do
    [[ -n "$ext" ]] || continue
    grep -qxF "$ext" <<<"$installed" && continue
    if code --install-extension "$ext" >/dev/null 2>&1; then
      ok "extension $ext"; added=$((added + 1))
    else
      warn "couldn't install extension $ext into VS Code"; failed=$((failed + 1))
    fi
  done < <(sed -nE 's/^vscode "([^"]+)".*/\1/p' "$REPO/Brewfile" | tr '[:upper:]' '[:lower:]')
  [[ $added -eq 0 && $failed -eq 0 ]] && skip "VS Code extensions"
else
  info "VS Code not installed, so no extensions to add"
fi

# 2. Login shell.
login_shell="$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)"
if [[ "$login_shell" == */zsh ]]; then
  skip "login shell (zsh)"
elif have zsh; then
  info "your login shell is ${login_shell:-unknown}. To use zsh everywhere: chsh -s $(command -v zsh)"
fi
