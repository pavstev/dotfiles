#!/usr/bin/env bash
# ============================================================================
#  bootstrap.sh: a fresh macOS, Linux or Windows (Git Bash) machine → full environment
#  in one command. Installs Xcode CLT (macOS) or git and curl (Debian, Ubuntu; Git Bash
#  has both), clones this repo, runs ./dotfiles (which installs everything else).
#  The one-liner that runs it is in the README "Set up" sections.
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

IS_WIN=""
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) IS_WIN=1 ;; esac

# 1. macOS gets the Xcode Command Line Tools (git + compilers). Debian and Ubuntu get
#    git and curl from apt; Git Bash on Windows has both. ./dotfiles installs the rest.
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

# 2. Clone the repo, or bring an existing one up to date. Two fixes keep a Windows
#    (Git Bash) run deterministic: the clone never converts line endings (a CRLF
#    shell script cannot run), and a clone with no checkout (an earlier run died at
#    "invalid path" or "unable to checkout working tree") is repaired, not skipped.
die() { printf '✖ %s\n' "$*" >&2; exit 1; }

if [[ ! -d "$REPO_DIR/.git" ]]; then
  existed=0; [[ -e "$REPO_DIR" ]] && existed=1
  info "Cloning $DOTFILES_REPO → $REPO_DIR"
  if ! git -c core.autocrlf=false clone "$DOTFILES_REPO" "$REPO_DIR"; then
    # Leave nothing behind, so the next run clones again instead of finding half a repo.
    [[ "$existed" -eq 0 ]] && rm -rf "$REPO_DIR"
    die "Clone failed. Fix the error above and run this again."
  fi
else
  if [[ ! -f "$REPO_DIR/dotfiles" ]]; then
    info "Repairing the earlier download in $REPO_DIR…"
    git -C "$REPO_DIR" fetch --quiet origin || die "Can't reach $DOTFILES_REPO. Check the network and run this again."
    git -C "$REPO_DIR" -c core.autocrlf=false reset --hard --quiet '@{u}' 2>/dev/null ||
      git -C "$REPO_DIR" -c core.autocrlf=false reset --hard --quiet origin/main ||
      die "Can't repair $REPO_DIR. Delete that folder and run this again."
  else
    info "Repo already at $REPO_DIR — pulling latest defaults…"
    git -C "$REPO_DIR" pull --ff-only --quiet || \
      info "(couldn't fast-forward — local changes? continuing with what's here)"
  fi
fi
[[ -f "$REPO_DIR/dotfiles" ]] || die "$REPO_DIR has no ./dotfiles script. Delete that folder and run this again."
# Same rule for every later pull and checkout in this clone (a Windows git defaults to CRLF).
git -C "$REPO_DIR" config core.autocrlf false
[[ -n "$IS_WIN" ]] || ok "repo at $REPO_DIR"

# 3. Run the one command. First run prompts for git identity, then links every
#    config file and runs the install steps (brew bundle, OMZ, toolchains,
#    Claude links…).
if [[ -n "$IS_WIN" ]]; then info "Setting up (about 5 minutes)…"; else info "./dotfiles (this installs everything; grab a coffee)…"; fi
bash "$REPO_DIR/dotfiles" || exit $?

[[ -n "$IS_WIN" ]] || ok "All done. Open a new shell or run: exec zsh"
