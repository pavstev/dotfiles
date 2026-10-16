# ============================================================================
#  ~/.config/zsh/claude-route.zsh  —  switch Claude Code between OpenRouter's
#  free model and the official Anthropic subscription.
#  Source: $DOTFILES/home/.config/zsh/claude-route.zsh. Sourced by init.zsh.
#
#  The key is never stored here. Put it in the gitignored local.zsh:
#     export OPENROUTER_API_KEY="sk-or-..."
#
#  Claude Code adds /v1/messages to the base URL itself, so the base URL is
#  https://openrouter.ai/api  (with /api/v1 the call goes to /api/v1/v1/messages
#  and OpenRouter answers 404).
# ============================================================================

_CLAUDE_FREE_MODEL="openrouter/free"
_CLAUDE_FREE_URL="https://openrouter.ai/api"
# Cohere rejects the regex patterns in these tools' schemas with
# "400 ... pattern must be a valid regex", so the free route turns them off.
# Add more names here (comma separated) if another tool trips the same error.
_CLAUDE_FREE_DENY="Artifact,ArtifactData,Monitor,RemoteTrigger,SendMessage,Workflow"

# `claude` on the free route gets the deny list; on any other route it is plain.
claude() {
  if [[ "${ANTHROPIC_BASE_URL:-}" == *openrouter.ai* ]]; then
    command claude --disallowedTools="$_CLAUDE_FREE_DENY" "$@"
  else
    command claude "$@"
  fi
}

# Switch this shell to OpenRouter's free model.
claude-free() {
  if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then  # scan-allowlist
    print -u2 "claude-free: OPENROUTER_API_KEY is empty. Set it in ~/.config/zsh/local.zsh."
    return 1
  fi
  export ANTHROPIC_BASE_URL="$_CLAUDE_FREE_URL"
  export ANTHROPIC_AUTH_TOKEN="${OPENROUTER_API_KEY:-}"  # scan-allowlist
  export ANTHROPIC_API_KEY=""  # scan-allowlist
  export CLAUDE_MODEL="$_CLAUDE_FREE_MODEL"
  # Claude Code reads these two for the main model and the small background model.
  export ANTHROPIC_MODEL="$_CLAUDE_FREE_MODEL"
  export ANTHROPIC_SMALL_FAST_MODEL="$_CLAUDE_FREE_MODEL"
  print "Claude route: Free OpenRouter ($_CLAUDE_FREE_MODEL)"
}

# Switch this shell back to the Anthropic subscription.
claude-official() {
  local v
  for v in ${(k)parameters[(I)ANTHROPIC_*]} CLAUDE_MODEL; do
    unset "$v"
  done
  print "Claude route: Official Anthropic subscription"
}

# Show the active route.
claude-status() {
  if [[ "${ANTHROPIC_BASE_URL:-}" == *openrouter.ai* ]]; then
    print "Claude route: Free OpenRouter"
    print "  base url: ${ANTHROPIC_BASE_URL}"
    print "  model:    ${CLAUDE_MODEL:-${ANTHROPIC_MODEL:-<unset>}}"
    if [[ -n "${ANTHROPIC_AUTH_TOKEN:-}" ]]; then print "  key:      set"; else print "  key:      MISSING"; fi  # scan-allowlist
  else
    print "Claude route: Official Anthropic subscription"
  fi
}

# One-shot: run Claude on the free model without changing this shell.
claudef() {
  if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then  # scan-allowlist
    print -u2 "claudef: OPENROUTER_API_KEY is empty. Set it in ~/.config/zsh/local.zsh."
    return 1
  fi
  local -a route_env=(
    "ANTHROPIC_BASE_URL=$_CLAUDE_FREE_URL"
    "ANTHROPIC_AUTH_TOKEN=${OPENROUTER_API_KEY:-}"  # scan-allowlist
    "ANTHROPIC_API_KEY="  # scan-allowlist
    "ANTHROPIC_SMALL_FAST_MODEL=$_CLAUDE_FREE_MODEL"
  )
  command env "${route_env[@]}" claude --disallowedTools="$_CLAUDE_FREE_DENY" --model "$_CLAUDE_FREE_MODEL" "$@"
}
