#!/usr/bin/env bash
# ============================================================================
#  bootstrap.sh — fresh Mac or Linux machine → full environment in one command.
#  Installs Xcode CLT (Mac) or git and curl (Debian, Ubuntu), clones this repo,
#  runs ./dotfiles (which installs Homebrew and everything else).
#  The one-liner that runs it is in README.md, "Set up a Mac" and "Set up Linux".
#
#  A fork changes DOTFILES_REPO here (and the URL in that one-liner), or
#  exports DOTFILES_REPO before running.
# ============================================================================
set -euo pipefail

DOTFILES_REPO="${DOTFILES_REPO:-https://github.com/pavstev/dotfiles.git}"
REPO_DIR="${DOTFILES:-$HOME/dotfiles}"

c_b="\033[0;34m"; c_g="\033[0;32m"; c_0="\033[0m"
info() { printf "${c_b}››${c_0} %s\n" "$*"; }
ok()   { printf "${c_g}✓${c_0} %s\n"  "$*"; }

# 1. A Mac gets the Xcode Command Line Tools (git + compilers). Debian and Ubuntu get
#    git and curl from apt; ./dotfiles installs the rest.
if [[ "$(uname -s)" == Darwin ]]; then
  if ! xcode-select -p >/dev/null 2>&1; then
    info "Installing Xcode Command Line Tools — accept the dialog that appears…"
    xcode-select --install || true
    until xcode-select -p >/dev/null 2>&1; do sleep 5; done
  fi
  ok "Xcode Command Line Tools present"
else
  if ! command -v git >/dev/null 2>&1 || ! command -v curl >/dev/null 2>&1; then
    command -v apt-get >/dev/null 2>&1 || { echo "This Linux has no apt-get. Install git and curl, then run this again." >&2; exit 1; }
    [[ "$(id -u)" -eq 0 ]] && SUDO="" || SUDO="sudo"
    info "Installing git and curl with apt (asks for your password once)…"
    $SUDO apt-get update -qq && $SUDO apt-get install -y -qq git curl ca-certificates
  fi
  ok "git and curl present"
fi

# 2. Clone the repo. If it is already here, fast-forward to the latest.
#    defaults so a re-run always applies the current source of truth.
if [[ ! -d "$REPO_DIR/.git" ]]; then
  info "Cloning $DOTFILES_REPO → $REPO_DIR"
  git clone "$DOTFILES_REPO" "$REPO_DIR"
else
  info "Repo already at $REPO_DIR — pulling latest defaults…"
  git -C "$REPO_DIR" pull --ff-only --quiet || \
    info "(couldn't fast-forward — local changes? continuing with what's here)"
fi
ok "repo at $REPO_DIR"

# 3. Run the one command. First run prompts for git identity, then links every
#    config file and runs the install steps (brew bundle, OMZ, toolchains,
#    Claude links…).
info "./dotfiles (this installs everything; grab a coffee)…"
"$REPO_DIR/dotfiles"

ok "All done. Open a new shell or run: exec zsh"
