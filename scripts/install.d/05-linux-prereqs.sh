#!/usr/bin/env bash
# 05-linux-prereqs.sh: the packages that Homebrew on Linux and the other steps need
# (build tools, curl, git, python3, zsh). Linux only: a Mac prints nothing here.
# apt (Debian, Ubuntu), dnf (Fedora), pacman (Arch) or apk (Alpine); LINUX_PREREQS in
# scripts/setup-lib.sh. Skips when every package is installed. Tolerant: a failure warns.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
is_linux || exit 0

step "Linux prerequisites"
if [[ -z "$(linux_prereqs_missing)" ]]; then
  skip "${LINUX_PKG%-get} packages (${LINUX_PREREQS[*]})"
  exit 0
fi
linux_prereqs || warn "prerequisites missing, so later steps may fail. Fix the error above, then run dots again."
