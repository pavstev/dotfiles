#!/usr/bin/env bash
# 60-cursor.sh: Cursor (app + CLI) from the same claude/ source. Runs only when
# Cursor is present. See docs/cursor-port.md.
#   1. the `agent` / `cursor-agent` CLI, via Cursor's official installer, if missing
#   2. the Brewfile `vscode` extensions, into Cursor (brew bundle only feeds VS Code)
#   3. generated rules and agents, merged hooks / MCP / CLI config (agent-config)
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
brew_env

step "Cursor"
app=""
for a in "/Applications/Cursor.app" "$HOME/Applications/Cursor.app"; do
  [[ -d "$a" ]] && { app="$a"; break; }
done
if [[ -z "$app" && ! -d "$HOME/.cursor" ]] && ! have cursor; then
  info "Cursor not installed, nothing to set up"
  exit 0
fi

# 1. CLI. Never probe it with `cursor agent ...`: that runs the installer too.
if [[ -x "$HOME/.local/bin/agent" ]] || have cursor-agent; then
  skip "Cursor CLI"
elif curl -fsS --max-time 60 https://cursor.com/install | bash >/dev/null 2>&1; then
  ok "Cursor CLI installed (~/.local/bin/agent). Sign in once: agent login"
else
  warn "Cursor CLI install failed. Try later: curl https://cursor.com/install -fsS | bash"
fi

# 2. Extensions. Pylance is not on Open VSX; Cursor ships its own Python server.
cursor_bin="$(command -v cursor || true)"
[[ -z "$cursor_bin" && -n "$app" ]] && cursor_bin="$app/Contents/Resources/app/bin/cursor"
if [[ -x "$cursor_bin" && -f "$REPO/Brewfile" ]]; then
  installed="$("$cursor_bin" --list-extensions 2>/dev/null | tr '[:upper:]' '[:lower:]')"
  added=0 failed=0
  while read -r ext; do
    [[ -n "$ext" && "$ext" != ms-python.vscode-pylance ]] || continue
    grep -qxF "$ext" <<<"$installed" && continue
    if "$cursor_bin" --install-extension "$ext" >/dev/null 2>&1; then
      ok "extension $ext"; added=$((added + 1))
    else
      warn "couldn't install extension $ext into Cursor"; failed=$((failed + 1))
    fi
  done < <(sed -nE 's/^vscode "([^"]+)".*/\1/p' "$REPO/Brewfile" | tr '[:upper:]' '[:lower:]')
  [[ $added -eq 0 && $failed -eq 0 ]] && skip "Cursor extensions"
else
  warn "no cursor command found, extensions not installed"
fi

# 3. Rules, agents, hooks, MCP, CLI config.
"$REPO/scripts/agent-config" cursor --profile "$(dotfiles_profile)" \
  || warn "agent-config cursor reported an issue, see above"
