#!/usr/bin/env bash
# Sample PostToolUse hook: auto-format a file after Claude edits it.
# Receives event JSON on stdin. Opt in by wiring it in settings.json (see README).
# Exit 0 always — never block edits on formatter hiccups.
set -uo pipefail

file="$(jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null)"
[[ -n "$file" && -f "$file" ]] || exit 0

case "$file" in
  *.py)                 command -v ruff   >/dev/null && ruff format "$file"        >/dev/null 2>&1 ;;
  *.ts|*.tsx|*.js|*.jsx|*.json|*.css)
                        command -v biome  >/dev/null && biome format --write "$file" >/dev/null 2>&1 ;;
  *.go)                 command -v gofmt  >/dev/null && gofmt -w "$file"            >/dev/null 2>&1 ;;
  *.rs)                 command -v rustfmt>/dev/null && rustfmt "$file"             >/dev/null 2>&1 ;;
esac
exit 0
