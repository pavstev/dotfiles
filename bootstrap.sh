#!/usr/bin/env bash
# ============================================================================
#  bootstrap.sh: a fresh macOS, Linux or Windows (Git Bash) machine → full environment
#  in one command. Installs Xcode CLT (macOS) or git and curl (Linux; Git Bash
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

# 1. macOS gets the Xcode Command Line Tools (git + compilers). Linux gets git and curl
#    from apt, dnf, pacman or apk; Git Bash on Windows has both. ./dotfiles installs the rest.
if [[ "$(uname -s)" == Darwin ]]; then
  if ! xcode-select -p >/dev/null 2>&1; then
    info "Installing Xcode Command Line Tools — accept the dialog that appears…"
    xcode-select --install || true
    until xcode-select -p >/dev/null 2>&1; do sleep 5; done
  fi
  ok "Xcode Command Line Tools present"
else
  if ! command -v git >/dev/null 2>&1 || ! command -v curl >/dev/null 2>&1; then
    [[ "$(id -u)" -eq 0 ]] && SUDO="" || SUDO="sudo"
    info "Installing git and curl (asks for your password once)…"
    if command -v apt-get >/dev/null 2>&1; then
      $SUDO apt-get update -qq && $SUDO apt-get install -y -qq git curl ca-certificates
    elif command -v dnf >/dev/null 2>&1; then $SUDO dnf install -y -q git curl ca-certificates
    elif command -v pacman >/dev/null 2>&1; then $SUDO pacman -S --needed --noconfirm git curl ca-certificates
    elif command -v apk >/dev/null 2>&1; then $SUDO apk add -q git curl ca-certificates bash
    else echo "This Linux has no apt, dnf, pacman or apk. Install git and curl, then run this again." >&2; exit 1
    fi
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
    before="$(git -C "$REPO_DIR" rev-parse --verify -q '@{u}' 2>/dev/null || true)"
    if ! git -C "$REPO_DIR" pull --ff-only --quiet 2>/dev/null; then
      # A rewritten upstream (force push): move a clone with no work of its own to it.
      # The same rule as `dots` (heal_rewrite): upstream lost its old tip, HEAD was inside
      # it, and no tracked file changed. The old state stays as a branch.
      # An old tip: the one before this pull, or one in the upstream reflog (a clone that
      # fetched earlier).
      after="$(git -C "$REPO_DIR" rev-parse --verify -q '@{u}' 2>/dev/null || true)"
      up="$(git -C "$REPO_DIR" rev-parse --symbolic-full-name '@{u}' 2>/dev/null || true)"
      old_tip=""
      log=""; [[ -n "$up" ]] && log="$(git -C "$REPO_DIR" rev-parse --git-path "logs/$up" 2>/dev/null)"
      [[ -z "$log" || "$log" == /* ]] || log="$REPO_DIR/$log"
      # raw reflog lines: "<old> <new> ...", both are tips the upstream had
      for t in $before $( [[ -f "$log" ]] && cut -d' ' -f1,2 "$log"); do
        if [[ "$t" != "$after" ]] && ! git -C "$REPO_DIR" merge-base --is-ancestor "$t" "$after" 2>/dev/null &&
          git -C "$REPO_DIR" merge-base --is-ancestor HEAD "$t" 2>/dev/null; then old_tip="$t"; break; fi
      done
      if [[ -n "$after" && -n "$old_tip" ]] &&
        [[ -z "$(git -C "$REPO_DIR" status --porcelain --untracked-files=no)" ]] &&
        git -C "$REPO_DIR" branch "before-rewrite-$(date +%Y%m%d-%H%M%S)" HEAD &&
        git -C "$REPO_DIR" -c core.autocrlf=false reset --hard --quiet '@{u}'; then
        info "The history was rewritten upstream. This clone had no work of its own, so it moved to the new one."
      else
        info "(couldn't fast-forward — local changes? continuing with what's here)"
      fi
    fi
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
