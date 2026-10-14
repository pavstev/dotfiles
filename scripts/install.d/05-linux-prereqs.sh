#!/usr/bin/env bash
# 05-linux-prereqs.sh: the apt packages that Homebrew on Linux and the other steps
# need (build tools, curl, git, python3, zsh). Linux only: a Mac prints nothing here.
# Debian and Ubuntu. Skips when every package is installed. Tolerant: a failure warns.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
is_linux || exit 0

step "Linux prerequisites"
if [[ -z "$(linux_prereqs_missing)" ]]; then
  skip "apt packages (${LINUX_PREREQS[*]})"
  exit 0
fi
linux_prereqs || warn "prerequisites missing, so later steps may fail. Fix the error above, then run dots again."
