#!/usr/bin/env bash
# 90-sign-in.sh: the sign-ins the tools need (GitHub, Claude Code, Cursor).
# On the first run at a terminal: a menu, then each picked login runs here.
# Later runs, or no terminal: one line that says what is left.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
brew_env
[[ -d "$HOME/.local/bin" ]] && PATH="$HOME/.local/bin:$PATH"   # claude, agent

step "Sign in"
labels=() cmds=()
if have gh && ! gh auth status >/dev/null 2>&1; then
  labels+=("🐙 GitHub"); cmds+=("gh auth login")
fi
if have claude && ! claude auth status 2>/dev/null | grep -q '"loggedIn": *true'; then
  labels+=("✳️  Claude Code"); cmds+=("claude auth login")
fi
if have agent && ! agent status 2>/dev/null | grep -qi 'logged in as'; then
  labels+=("🖱️  Cursor"); cmds+=("agent login")
fi
if (( ${#labels[@]} == 0 )); then
  skip "sign-ins"
  exit 0
fi
if [[ -z "${DOTFILES_FIRST_RUN:-}" ]] || ! at_terminal; then
  for c in "${cmds[@]}"; do info "not signed in yet, run: $c"; done
  exit 0
fi

yes="✅ Yes, sign in to: $(printf '%s, ' "${labels[@]}" | sed 's/, $//')"
later="⏭️  Later (dots shows the commands)"
answer="$(ask_one "🔑 Sign in now? A browser opens for each one." "$yes" "$later")" || answer=""
if [[ "$answer" != "$yes" ]]; then
  for c in "${cmds[@]}"; do info "later, run: $c"; done
  exit 0
fi
for i in "${!labels[@]}"; do
  info "${labels[$i]}: ${cmds[$i]}"
  ${cmds[$i]} || warn "${cmds[$i]} did not finish. Run it again later."
  if is_macos && [[ "${cmds[$i]}" == "agent login" && -d /Applications/Cursor.app ]]; then
    open -a Cursor && info "sign in inside the Cursor app too"
  fi
done
