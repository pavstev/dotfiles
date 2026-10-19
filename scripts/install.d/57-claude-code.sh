#!/usr/bin/env bash
# 57-claude-code.sh: Claude Code (the `claude` CLI). Mac and Linux: Anthropic's installer,
# which keeps it up to date. Windows: WinGet (Anthropic.ClaudeCode), else Anthropic's
# PowerShell installer. Skips when `claude` is already there.
# Its config comes from claude/ through the ~/.claude links in ./dotfiles.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Claude Code"
if have claude || [[ -x "$HOME/.local/bin/claude" ]]; then
  skip "Claude Code"
elif is_windows; then
  if have winget && winget install --id Anthropic.ClaudeCode -e --accept-package-agreements --accept-source-agreements --silent >/dev/null 2>&1; then
    ok "Claude Code installed"
  elif powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "irm https://claude.ai/install.ps1 | iex" >/dev/null 2>&1; then
    ok "Claude Code installed"
  else
    warn "Claude Code did not install. In PowerShell run: irm https://claude.ai/install.ps1 | iex"
  fi
elif curl -fsSL --max-time 300 https://claude.ai/install.sh | bash >/dev/null 2>&1; then
  ok "Claude Code installed (~/.local/bin/claude). Sign in once: claude auth login"
else
  warn "Claude Code install failed. Try later: curl -fsSL https://claude.ai/install.sh | bash"
fi
