#!/usr/bin/env bash
# Oh My Zsh + custom plugins (cloned, not vendored). Runs after ~/.zshrc exists.
# Each piece is checked first — present ones are skipped, missing ones cloned.
# Tolerant: a failed network step warns and continues.
set -euo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Oh My Zsh + plugins"

if [[ -d "$HOME/.oh-my-zsh" ]]; then
  skip "Oh My Zsh"
else
  info "Installing Oh My Zsh…"
  # REPO= : our runner exports REPO=<repo path>, but the OMZ installer reads
  # $REPO as its GitHub slug (ohmyzsh/ohmyzsh) — shield it or the clone 404s.
  if RUNZSH=no CHSH=no KEEP_ZSHRC=yes REPO= \
       sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)" "" --unattended; then
    ok "Oh My Zsh installed"
  else
    warn "Oh My Zsh install failed (network?) — skipping plugins. Re-run: ./dotfiles"
    exit 0
  fi
fi

ZC="${ZSH_CUSTOM:-$HOME/.oh-my-zsh/custom}"
mkdir -p "$ZC/plugins" 2>/dev/null || true

clone_plugin() { # <repo-url> <name>
  local dir="$ZC/plugins/$2"
  if [[ -d "$dir" ]]; then
    skip "plugin $2"
  else
    info "cloning $2…"
    if git clone --depth=1 "$1" "$dir" >/dev/null 2>&1; then
      ok "$2"
    else
      warn "clone of $2 failed (network?) — re-run: ./dotfiles"
    fi
  fi
}
clone_plugin https://github.com/zsh-users/zsh-autosuggestions             zsh-autosuggestions
clone_plugin https://github.com/zdharma-continuum/fast-syntax-highlighting fast-syntax-highlighting
clone_plugin https://github.com/Aloxaf/fzf-tab                             fzf-tab
clone_plugin https://github.com/zsh-users/zsh-completions                  zsh-completions
ok "Oh My Zsh + plugins ready"
