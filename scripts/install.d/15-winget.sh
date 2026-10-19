#!/usr/bin/env bash
# 15-winget.sh: Windows only. Installs Opencode (the `opencode` CLI) and yt-dlp (for ps:yt-learn)
# with WinGet, which is built into Windows 10/11. The Brewfile does not apply on Windows.
# Idempotent and tolerant: skips what is present or when winget is missing, and
# warns (never fails) when an install cannot finish. SST.opencode is the Opencode package
# today; Anomaly.OpenCode is the official one that replaces it.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
is_windows || exit 0

winget_install() {
  winget install --id "$1" -e --accept-package-agreements --accept-source-agreements --silent >/dev/null 2>&1
}

install_opencode() {
  step "Opencode"
  if have opencode; then
    skip "Opencode"
    return
  fi
  have winget || { warn "winget not found. Install Opencode yourself: winget install SST.opencode"; return; }
  local id
  for id in SST.opencode Anomaly.OpenCode; do
    if winget_install "$id"; then
      ok "Opencode installed"
      return
    fi
  done
  warn "Opencode did not install. Try in PowerShell: winget install SST.opencode"
}

install_ytdlp() {
  step "yt-dlp"
  if have yt-dlp; then
    skip "yt-dlp"
  elif ! have winget; then
    warn "winget not found. Install yt-dlp yourself: winget install yt-dlp.yt-dlp"
  elif winget_install yt-dlp.yt-dlp; then
    ok "yt-dlp installed"
  else
    warn "yt-dlp did not install. Try in PowerShell: winget install yt-dlp.yt-dlp"
  fi
}

install_opencode
install_ytdlp
