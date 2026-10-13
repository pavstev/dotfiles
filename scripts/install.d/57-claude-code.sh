#!/usr/bin/env bash
# 57-claude-code.sh: Claude Code (the `claude` CLI) from Anthropic's official
# installer, which keeps it up to date. Skips when `claude` is already there.
# Its config comes from claude/ through the ~/.claude links in ./dotfiles.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

step "Claude Code"
if have claude || [[ -x "$HOME/.local/bin/claude" ]]; then
  skip "Claude Code"
elif curl -fsSL --max-time 300 https://claude.ai/install.sh | bash >/dev/null 2>&1; then
  ok "Claude Code installed (~/.local/bin/claude). Sign in once: claude auth login"
else
  warn "Claude Code install failed. Try later: curl -fsSL https://claude.ai/install.sh | bash"
fi
