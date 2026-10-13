#!/usr/bin/env bash
# Claude Code status line — copper-themed. Receives session JSON on stdin.
# Shows:   <dir>   <git branch>   <model>   ctx <K> <%>   5h <%> · 7d <%>
# Context turns copper at 300 K: time to /ps:handoff. Rate limits are absent
# until the first API reply; then nothing is printed for them.
set -euo pipefail

input="$(cat)"
get() { printf '%s' "$input" | jq -r "$1 // empty" 2>/dev/null; }

cwd="$(get '.workspace.current_dir')"; [ -n "$cwd" ] || cwd="$(get '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
model="$(get '.model.display_name')"
dir="$(basename "$cwd")"
branch="$(git -C "$cwd" branch --show-current 2>/dev/null || true)"
# Current context = the last call's input + cache tokens (total_input_tokens is a session running total).
ctx_tok="$(get '.context_window.current_usage | (.input_tokens + .cache_creation_input_tokens + .cache_read_input_tokens)')"
[ -n "$ctx_tok" ] && [ "$ctx_tok" != "0" ] || ctx_tok="$(get '.context_window.total_input_tokens')"
ctx_pct="$(get '.context_window.used_percentage')"
five="$(get '.rate_limits.five_hour.used_percentage')"
week="$(get '.rate_limits.seven_day.used_percentage')"

# Refined Claude palette (truecolor).
copper=$'\033[38;2;224;130;95m'   # #E0825F
sage=$'\033[38;2;135;176;131m'    # #87B083
stone=$'\033[38;2;168;162;158m'   # #A8A29E
rst=$'\033[0m'

int() { printf '%.0f' "$1" 2>/dev/null || true; }

line="${copper} ${dir}${rst}"
[ -n "$branch" ] && line+="  ${sage} ${branch}${rst}"
[ -n "$model" ]  && line+="  ${stone}${model}${rst}"

if [ -n "$ctx_tok" ]; then
  k=$(( $(int "$ctx_tok") / 1000 ))
  col="$stone"; [ "$k" -ge 300 ] && col="$copper"
  ctx="ctx ${k}K"; [ -n "$ctx_pct" ] && ctx+=" $(int "$ctx_pct")%"
  line+="  ${col}${ctx}${rst}"
fi

caps=""
[ -n "$five" ] && caps="5h $(int "$five")%"
[ -n "$week" ] && caps+="${caps:+ · }7d $(int "$week")%"
[ -n "$caps" ] && line+="  ${stone}${caps}${rst}"
printf '%s' "$line"
