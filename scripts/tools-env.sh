#!/usr/bin/env bash
# scripts/tools-env.sh: puts the places that tools from mise, cargo, WinGet and the mise
# installer land on PATH. Sourced by ./dotfiles (so every install step sees them) and by
# scripts/tools. They go on the END of PATH: a tool that is already on PATH keeps its place.
# Needs is_windows from scripts/setup-lib.sh.

tools_add_paths() {
  local d
  # Not tested with -d: a step that runs later in the same ./dotfiles run makes these folders.
  for d in "$HOME/.local/bin" "$HOME/.local/share/mise/shims" "$HOME/.cargo/bin"; do
    [[ ":$PATH:" != *":$d:"* ]] && PATH="$PATH:$d"
  done
  if is_windows && [[ -n "${LOCALAPPDATA:-}" ]]; then
    for d in "$LOCALAPPDATA/mise/shims" "$LOCALAPPDATA/Microsoft/WinGet/Links"; do
      [[ -d "$d" ]] || continue
      d="$(cygpath -u "$d" 2>/dev/null || printf '%s' "$d")"
      [[ ":$PATH:" != *":$d:"* ]] && PATH="$PATH:$d"
    done
  fi
  export PATH
}
